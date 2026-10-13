/// The `Alley` struct provides reusable parallel execution across a fixed number of owned lanes.
pub struct Alley {
	threadpool: LanePool,
}

/// The `AlleyFunction` trait defines value-producing work that every lane runs during an [`Alley`] dispatch.
pub trait AlleyFunction<R> = for<'dispatch> Fn(&mut ConcreteLane<'dispatch>) -> R + Clone + Send;

/// The `Lane` trait identifies a logical parallel path through an [`Alley`].
pub trait Lane {
	fn idx(&self) -> usize;
}

/// The `LaneMut` struct provides one stable lane with exclusive access to a mutable resource.
pub struct LaneMut<'value, T> {
	owner_lane: usize,
	value: UnsafeCell<&'value mut T>,
}

impl<'value, T> LaneMut<'value, T> {
	fn new(owner_lane: usize, value: &'value mut T) -> Self {
		Self {
			owner_lane,
			value: UnsafeCell::new(value),
		}
	}
}

// SAFETY: `only_one_runs_mut` dereferences the resource only on its stable owner lane.
// The blocking dispatch prevents that access from outliving the original `&mut T`.
#[allow(unsafe_code, reason = "A stable owner lane provides exclusive mutable resource access.")]
unsafe impl<T: Send> Sync for LaneMut<'_, T> {}

/// The `LanePhased` struct gives every lane access to one resource in alternating exclusive and shared phases.
///
/// Use it when one lane function runs a whole algorithm: serial stages mutate the resource through
/// [`ConcreteLane::broadcast_mut`] while the other lanes wait, and parallel stages read it from every lane through
/// [`ConcreteLane::each_ref`]. Create one by dispatching with [`Alley::execute_phased`].
pub struct LanePhased<'value, T> {
	value: UnsafeCell<&'value mut T>,
}

// SAFETY: Lanes only reach the resource inside collective sections. A broadcast section admits one elected lane
// while the others wait, and an `each` section admits only shared references and ends for every lane together.
// The blocking dispatch prevents either access from outliving the original `&mut T`.
#[allow(unsafe_code, reason = "Collective sections separate exclusive and shared resource access.")]
unsafe impl<T: Send + Sync> Sync for LanePhased<'_, T> {}

mod lane_resources_private {
	// This token keeps wrapper construction inside the Alley dispatch API.
	pub struct BindToken;
}

/// The `IntoLaneResources` trait binds mutable resources to stable owner lanes for an [`Alley`] dispatch.
///
/// Pass one mutable reference or a tuple containing 1 through 8 distinct mutable references.
pub trait IntoLaneResources {
	/// The wrappers shared with every lane during the blocking dispatch.
	type Bound: Sync;

	#[doc(hidden)]
	fn bind(self, parallelism: usize, next_index: &mut usize, _token: lane_resources_private::BindToken) -> Self::Bound;
}

impl<'value, T: Send> IntoLaneResources for &'value mut T {
	type Bound = LaneMut<'value, T>;

	fn bind(self, parallelism: usize, next_index: &mut usize, _token: lane_resources_private::BindToken) -> Self::Bound {
		let owner_lane = *next_index % parallelism;
		*next_index += 1;
		LaneMut::new(owner_lane, self)
	}
}

macro_rules! impl_into_lane_resources_for_tuple {
	($(($resource_type:ident, $resource:ident)),+) => {
		impl<'value, $($resource_type: Send),+> IntoLaneResources for ($(&'value mut $resource_type,)+) {
			type Bound = ($(LaneMut<'value, $resource_type>,)+);

			fn bind(
				self,
				parallelism: usize,
				next_index: &mut usize,
				_token: lane_resources_private::BindToken,
			) -> Self::Bound {
				let ($($resource,)+) = self;
				($($resource.bind(parallelism, next_index, lane_resources_private::BindToken),)+)
			}
		}
	};
}

impl_into_lane_resources_for_tuple!((T0, resource0));
impl_into_lane_resources_for_tuple!((T0, resource0), (T1, resource1));
impl_into_lane_resources_for_tuple!((T0, resource0), (T1, resource1), (T2, resource2));
impl_into_lane_resources_for_tuple!((T0, resource0), (T1, resource1), (T2, resource2), (T3, resource3));
impl_into_lane_resources_for_tuple!(
	(T0, resource0),
	(T1, resource1),
	(T2, resource2),
	(T3, resource3),
	(T4, resource4)
);
impl_into_lane_resources_for_tuple!(
	(T0, resource0),
	(T1, resource1),
	(T2, resource2),
	(T3, resource3),
	(T4, resource4),
	(T5, resource5)
);
impl_into_lane_resources_for_tuple!(
	(T0, resource0),
	(T1, resource1),
	(T2, resource2),
	(T3, resource3),
	(T4, resource4),
	(T5, resource5),
	(T6, resource6)
);
impl_into_lane_resources_for_tuple!(
	(T0, resource0),
	(T1, resource1),
	(T2, resource2),
	(T3, resource3),
	(T4, resource4),
	(T5, resource5),
	(T6, resource6),
	(T7, resource7)
);

/// The `LaneValues` struct provides lane-ordered iteration over completed collective values.
pub struct LaneValues<'dispatch, T> {
	storage: *const u8,
	next_lane: usize,
	lane_count: usize,
	marker: std::marker::PhantomData<&'dispatch T>,
}

impl<'dispatch, T> LaneValues<'dispatch, T> {
	/// Makes the values of `lane_count` lanes at `storage`, which is only read for values with a size.
	fn new(storage: *const u8, lane_count: usize) -> Self {
		Self {
			storage,
			next_lane: 0,
			lane_count,
			marker: std::marker::PhantomData,
		}
	}

	#[allow(unsafe_code, reason = "Lane values are copied from synchronized strided storage.")]
	fn read_lane(&self, lane_idx: usize) -> T
	where
		T: Copy,
	{
		if size_of::<T>() == 0 {
			// SAFETY: `Section::values` makes these values only after the lane's work returned a `T`, which this
			// copies. A read of no bytes is valid at any aligned address.
			return unsafe { std::ptr::NonNull::<T>::dangling().as_ptr().read() };
		}
		// SAFETY: `Section::values` constructs this iterator only after each of its lanes initialized its aligned
		// slot with a `T` and the section ended. `T: Copy` leaves storage valid.
		let lane_offset = lane_idx * COLLECTIVE_VALUE_SIZE;
		// SAFETY: The validated lane index and fixed stride keep this address inside collective storage.
		let lane = unsafe { self.storage.add(lane_offset) };
		// SAFETY: The section checked that a `T` fits a slot and that the lane wrote one before it made these values.
		unsafe { lane.cast::<T>().read() }
	}
}

impl<T> Clone for LaneValues<'_, T> {
	fn clone(&self) -> Self {
		Self {
			storage: self.storage,
			next_lane: self.next_lane,
			lane_count: self.lane_count,
			marker: std::marker::PhantomData,
		}
	}
}

impl<T: Copy> Iterator for LaneValues<'_, T> {
	type Item = T;

	fn next(&mut self) -> Option<Self::Item> {
		if self.next_lane == self.lane_count {
			return None;
		}

		let lane_idx = self.next_lane;
		self.next_lane += 1;
		Some(self.read_lane(lane_idx))
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		let remaining = self.lane_count - self.next_lane;
		(remaining, Some(remaining))
	}
}

impl<T: Copy> ExactSizeIterator for LaneValues<'_, T> {}
impl<T: Copy> std::iter::FusedIterator for LaneValues<'_, T> {}

/// The `ConcreteLane` struct provides lane-local operations during an [`Alley`] dispatch.
pub struct ConcreteLane<'dispatch> {
	lane_idx: usize,
	state: &'dispatch DispatchState,
	next_limited_parallelism_section: usize,
	/// The collective section this lane runs next.
	next_section: usize,
	/// The slot that the next collective section with values keeps them in.
	next_slot: usize,
}

impl<'dispatch> ConcreteLane<'dispatch> {
	fn new(lane_idx: usize, state: &'dispatch DispatchState) -> Self {
		Self {
			lane_idx,
			state,
			next_limited_parallelism_section: 0,
			next_section: 0,
			next_slot: 0,
		}
	}

	/// Runs `f` on the first lane to reach this single-runner section.
	///
	/// Every lane must encounter limited-parallelism sections in the same order.
	pub fn only_one_runs<F>(&mut self, f: F)
	where
		F: FnOnce(),
	{
		self.with_limited_parallelism(1, f);
	}

	/// Runs `f` with the mutable resource only when this lane is its stable owner.
	///
	/// This operation performs one lane-index branch without synchronization or allocation.
	#[allow(
		unsafe_code,
		reason = "The resource owner lane exclusively dereferences its bound mutable value."
	)]
	pub fn only_one_runs_mut<T, R>(&mut self, resource: &LaneMut<'_, T>, f: impl FnOnce(&mut T) -> R) -> Option<R> {
		if self.lane_idx != resource.owner_lane {
			return None;
		}

		// SAFETY: `LaneMut` has one private owner index, and only that lane enters
		// this branch. One lane executes its calls in source order before dispatch returns.
		let value = unsafe { &mut **resource.value.get() };
		Some(f(value))
	}

	/// Runs `f` on the first `limit` lanes to reach this section.
	///
	/// Lanes beyond `limit` continue immediately. Every lane must encounter these
	/// sections in the same order and use the same limit for each section.
	pub fn with_limited_parallelism<F>(&mut self, limit: usize, f: F)
	where
		F: FnOnce(),
	{
		let section = self.next_limited_parallelism_section;
		self.next_limited_parallelism_section += 1;

		let claims = self.state.limited_parallelism.get(section).unwrap_or_else(|| {
			panic!(
				"Limited-parallelism capacity exceeded. A dispatch supports at most {LIMITED_PARALLELISM_SECTION_COUNT} limited sections."
			)
		});

		if claims.fetch_add(1, Ordering::Relaxed) < limit {
			f();
		}
	}

	/// Runs `f` with exclusive access to the resource on the first lane to reach this section and returns its
	/// value to every lane.
	///
	/// Every other lane waits until `f` finishes, so a following [`Self::each_ref`] sees everything `f` wrote.
	/// Every lane must encounter this section in the same order and with the same result type.
	///
	/// Return `()` from stages that run in a loop. A dispatch runs any number of sections that return a value without
	/// size, but it keeps other values until it ends and has room for those of 32 sections.
	#[allow(unsafe_code, reason = "The elected lane exclusively dereferences the phased resource.")]
	pub fn broadcast_mut<T, R, F>(&mut self, resource: &LanePhased<'_, T>, f: F) -> R
	where
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&mut T) -> R,
	{
		self.next_section::<R>().broadcast(|| {
			// SAFETY: Only the first lane to enter this broadcast section runs this closure. Every earlier section
			// has ended on all lanes, and no lane passes this section before the closure returned.
			let value = unsafe { &mut **resource.value.get() };
			f(value)
		})
	}

	/// Runs `f` on every lane with shared access to the resource and returns lane-ordered values after every
	/// lane finished.
	///
	/// `f` also receives this lane, for [`Lane::idx`] and [`Self::share`]. Every lane must encounter this section
	/// in the same order and with the same result type.
	///
	/// Return `()` from stages that run in a loop, as with [`Self::broadcast_mut`]: only sections that return a
	/// value with a size count toward the 32 a dispatch has room for.
	#[allow(unsafe_code, reason = "Every lane shares the phased resource for one collective section.")]
	pub fn each_ref<T, R, F>(&mut self, resource: &LanePhased<'_, T>, f: F) -> LaneValues<'dispatch, R>
	where
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&T, &Self) -> R,
	{
		let section = self.next_section::<R>();
		let lane = &*self;
		section.each(|| {
			// SAFETY: This closure runs once this lane entered the section as `each`. Every earlier section has
			// ended, and a lane that enters this one as a broadcast fails before it runs anything, so no lane holds
			// exclusive access. The section ends for all lanes only after every closure returned, so no shared
			// reference reaches a later exclusive section.
			let value = unsafe { &**resource.value.get() };
			f(value, lane)
		})
	}

	/// Returns this lane's balanced partition of `values`, the one [`Self::each_shared`] passes to its closure.
	///
	/// Every lane must pass the same slice so the partitions cover it exactly once.
	pub fn share<'values, T>(&self, values: &'values [T]) -> &'values [T] {
		&values[self.share_range(values.len())]
	}

	/// Returns this lane's balanced range of `len` items, the indices [`Self::share`] covers in a slice of that length.
	///
	/// Use it to partition data that is not one slice, such as parallel arrays. Every lane must pass the same length
	/// so the ranges cover it exactly once.
	pub fn share_range(&self, len: usize) -> Range<usize> {
		shared_range(len, self.lane_idx, self.state.lane_count)
	}

	/// Returns the value produced by the first lane to reach this broadcast section.
	///
	/// Every lane must encounter broadcast sections in the same order and with the same types. A dispatch keeps the
	/// values of at most 32 collective sections, and runs any number of sections whose value has no size, such as
	/// `()`.
	pub fn broadcast<T, F>(&mut self, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		self.next_section::<T>().broadcast(f)
	}

	/// Runs `f` on every lane and returns lane-ordered values after every result is published.
	///
	/// Every lane must encounter `each` sections in the same order and with the same types. A dispatch keeps the
	/// values of at most 32 collective sections, and runs any number of sections whose value has no size, such as
	/// `()`.
	pub fn each<T, F>(&mut self, f: F) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		self.next_section::<T>().each(f)
	}

	/// Runs `f` on each lane's balanced partition and returns all lane results.
	///
	/// Every lane must provide the same shared slice and encounter this collective in the same order.
	pub fn each_shared<T, R, F>(&mut self, values: &[T], f: F) -> LaneValues<'dispatch, R>
	where
		T: Sync,
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&[T]) -> R,
	{
		let partition = self.share(values);
		self.each(move || f(partition))
	}

	/// Takes this lane's turn at the next collective section, which returns a `T` to the lane.
	fn next_section<T>(&mut self) -> Section<'dispatch> {
		let index = self.next_section;
		self.next_section += 1;
		let slot_holder = self.state.slot_holders.get(self.next_slot);
		// A value without size needs no storage, so only the other sections take one of the dispatch's slots.
		let slot = (size_of::<T>() != 0).then(|| {
			if size_of::<T>() > COLLECTIVE_VALUE_SIZE || align_of::<T>() > COLLECTIVE_VALUE_ALIGNMENT {
				self.state.gate.poison();
				panic!(
					"Collective value does not fit. Section {index} returns a value of more than {COLLECTIVE_VALUE_SIZE} bytes or aligned to more than {COLLECTIVE_VALUE_ALIGNMENT}. The most likely cause is returning the data itself where a count or an index would do."
				);
			}
			let slot = self.next_slot;
			self.next_slot += 1;
			self.state.slots.get(slot).unwrap_or_else(|| {
				self.state.gate.poison();
				panic!(
					"Collective value capacity exceeded. A dispatch keeps the values of at most {COLLECTIVE_SECTION_COUNT} collective sections. The most likely cause is a loop of sections that return a value where `()` would do."
				)
			})
		});
		Section {
			index,
			lane_idx: self.lane_idx,
			lane_count: self.state.lane_count,
			gate: &self.state.gate,
			slot,
			slot_holder,
		}
	}
}

/// The `SharedLane` struct provides lane-local access to one exclusive partition of shared mutable data.
pub struct SharedLane<'dispatch, 'values, T> {
	lane: ConcreteLane<'dispatch>,
	values: &'values mut [T],
}

impl<'dispatch, 'values, T> SharedLane<'dispatch, 'values, T> {
	fn new(lane: ConcreteLane<'dispatch>, values: &'values mut [T]) -> Self {
		Self { lane, values }
	}

	/// Mutates this lane's partition and returns all lane results after publication.
	///
	/// Every lane must encounter this collective in the same order and with the same result type.
	pub fn each_shared_mut<R, F>(&mut self, f: F) -> LaneValues<'dispatch, R>
	where
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&mut [T]) -> R,
	{
		let values = &mut *self.values;
		self.lane.each(move || f(values))
	}
}

impl<'dispatch, T> Deref for SharedLane<'dispatch, '_, T> {
	type Target = ConcreteLane<'dispatch>;

	fn deref(&self) -> &Self::Target {
		&self.lane
	}
}

impl<'dispatch, T> DerefMut for SharedLane<'dispatch, '_, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.lane
	}
}

impl Lane for ConcreteLane<'_> {
	fn idx(&self) -> usize {
		self.lane_idx
	}
}

impl<T> Lane for SharedLane<'_, '_, T> {
	fn idx(&self) -> usize {
		self.lane.idx()
	}
}

/// Returns the balanced index range assigned to one lane out of `value_count` values.
fn shared_range(value_count: usize, lane_idx: usize, lane_count: usize) -> Range<usize> {
	let minimum_len = value_count / lane_count;
	let larger_partition_count = value_count % lane_count;
	let start = lane_idx * minimum_len + lane_idx.min(larger_partition_count);
	start..start + balanced_partition_len(value_count, lane_idx, lane_count)
}

fn balanced_partition_len(value_count: usize, lane_idx: usize, lane_count: usize) -> usize {
	value_count / lane_count + usize::from(lane_idx < value_count % lane_count)
}

const LIMITED_PARALLELISM_SECTION_COUNT: usize = 32;
/// The most collective sections of one dispatch that can return a value with a size.
///
/// A dispatch keeps those values until it ends, so it has storage for a fixed number of them. Sections that return
/// a value without size, such as `()`, are not counted.
const COLLECTIVE_SECTION_COUNT: usize = 32;
const COLLECTIVE_LANE_CAPACITY: usize = 64;
const COLLECTIVE_VALUE_SIZE: usize = 64;
const COLLECTIVE_VALUE_ALIGNMENT: usize = 64;

struct DispatchState {
	lane_count: usize,
	limited_parallelism: [AtomicUsize; LIMITED_PARALLELISM_SECTION_COUNT],
	gate: SectionGate,
	/// For each slot, the broadcast section whose value it holds, plus one. Zero until a broadcast writes the slot.
	///
	/// These are apart from the slots so that making a dispatch's state writes no slot.
	slot_holders: [AtomicUsize; COLLECTIVE_SECTION_COUNT],
	slots: [CollectiveSlot; COLLECTIVE_SECTION_COUNT],
}

impl DispatchState {
	fn new(lane_count: usize) -> Self {
		Self {
			lane_count,
			limited_parallelism: std::array::from_fn(|_| AtomicUsize::new(0)),
			gate: SectionGate::default(),
			slot_holders: std::array::from_fn(|_| AtomicUsize::new(0)),
			slots: std::array::from_fn(|_| CollectiveSlot::new()),
		}
	}
}

/// The `SectionGate` struct keeps the lanes of a dispatch together through its collective sections.
///
/// Every lane enters a section, the lanes it admits run their work, and the section ends for all of them at once.
/// One set of counters serves every section in turn, so a dispatch can run any number of sections.
///
/// A broadcast section ends when the lane that runs it finishes, so that lane may be sections ahead of the others
/// until the next `each` section, which ends when every lane finished.
#[derive(Default)]
#[repr(align(64))]
struct SectionGate {
	/// The section that lanes entered last and what they run there, as [`Self::enter`] packs them.
	entered: AtomicUsize,
	/// How many lanes are done with the `each` section that is running.
	done: AtomicUsize,
	/// How many sections have ended, or [`Self::POISONED`] once a lane failed in one.
	ended: AtomicUsize,
}

impl SectionGate {
	const POISONED: usize = usize::MAX;

	/// Enters `section` as a lane that runs a broadcast or an `each` there, and returns whether it is the first lane
	/// to enter.
	///
	/// `valued` tells whether the section keeps values in a [`CollectiveSlot`]. A lane that enters to run something
	/// else than the lanes before it fails the section for every lane.
	///
	/// The lanes ahead may have left a broadcast section behind and entered a later one. A lane that then enters the
	/// earlier one cannot compare itself with them here, so [`Section::broadcast`] does it once the section ended.
	fn enter(&self, section: usize, broadcast: bool, valued: bool) -> bool {
		let entry = (section + 1) << 2 | usize::from(valued) << 1 | usize::from(broadcast);
		// A later section packs to a larger number, so the first lane to enter a section is the one that raises it.
		let before = self.entered.fetch_max(entry, Ordering::Relaxed);
		if before >> 2 <= section {
			return true;
		}
		// Lanes only leave a section behind without this lane when it is a broadcast.
		let left_behind = before >> 2 > section + 1 && broadcast;
		if before != entry && !left_behind {
			self.fail_mismatch(section);
		}
		false
	}

	/// Counts this lane as done with the `each` section `section`, and returns once every lane is.
	///
	/// What every lane wrote before it was done is visible to all of them afterwards.
	fn finish(&self, section: usize, lane_count: usize) {
		if self.done.fetch_add(1, Ordering::AcqRel) + 1 != lane_count {
			return self.wait_until_ended(section);
		}
		// Every other lane waits for the section to end, so none counts again before it does.
		self.done.store(0, Ordering::Relaxed);
		self.end(section);
	}

	/// Ends `section` for every lane. What this lane wrote before is visible to all of them afterwards.
	fn end(&self, section: usize) {
		let ended = self
			.ended
			.compare_exchange(section, section + 1, Ordering::Release, Ordering::Relaxed);
		if ended.is_err() {
			self.panic_poisoned(section);
		}
	}

	/// Fails `section` for every lane because this lane runs something else in it than another lane.
	fn fail_mismatch(&self, section: usize) -> ! {
		self.poison();
		panic!(
			"Collective operation mismatch at section {section}. The most likely cause is that lanes took different branches, so they did not all run the same operation, with or without a value, in the same order."
		)
	}

	fn wait_until_ended(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			let ended = self.ended.load(Ordering::Acquire);
			if ended == Self::POISONED {
				self.panic_poisoned(section);
			}
			if ended > section {
				return;
			}
			wait_briefly(&mut spins);
		}
	}

	/// Fails the running section, and with it the dispatch, for every lane that waits on it or reaches it.
	fn poison(&self) {
		self.ended.store(Self::POISONED, Ordering::Release);
	}

	fn panic_poisoned(&self, section: usize) -> ! {
		panic!("Collective failed. A lane in section {section} panicked or used a different operation or type.")
	}
}

/// The `Section` struct lets one lane run a collective section in step with the other lanes of its dispatch.
struct Section<'dispatch> {
	index: usize,
	lane_idx: usize,
	lane_count: usize,
	gate: &'dispatch SectionGate,
	/// Where the section keeps its values. `None` when they have no size, so there is nothing to keep.
	slot: Option<&'dispatch CollectiveSlot>,
	/// Which broadcast section wrote the dispatch's next free slot, the one this section takes when it has values.
	/// `None` when no slot is free. See [`DispatchState::slot_holders`].
	slot_holder: Option<&'dispatch AtomicUsize>,
}

impl<'dispatch> Section<'dispatch> {
	/// Runs `f` on the first lane to enter the section, and returns its value on every lane once it finished.
	#[allow(
		unsafe_code,
		reason = "Broadcast copies one value between lanes through the section's slot."
	)]
	fn broadcast<T, F>(self, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if self.gate.enter(self.index, true, self.slot.is_some()) {
			let value = self.compute(f);
			if let (Some(slot), Some(holder)) = (self.slot, self.slot_holder) {
				// SAFETY: Only the first lane to enter writes lane slot zero, which `ConcreteLane::next_section`
				// checked that a `T` fits. The other lanes read it after the section ended.
				unsafe { slot.write(0, value) };
				// The section ends after this, which is when other lanes read it.
				holder.store(self.index + 1, Ordering::Relaxed);
			}
			self.gate.end(self.index);
			return value;
		}

		self.gate.wait_until_ended(self.index);
		// The first lane may have been sections ahead when this lane entered, so the gate could not tell whether it
		// kept a value. The slot can: every lane takes the same slot next, and only a broadcast in this section
		// leaves this section's number in it.
		let kept = self
			.slot_holder
			.is_some_and(|holder| holder.load(Ordering::Relaxed) == self.index + 1);
		if kept != self.slot.is_some() {
			self.gate.fail_mismatch(self.index);
		}
		// SAFETY: The section ended, and when it keeps a value the first lane wrote it to lane slot zero.
		unsafe { self.values(1) }.read_lane(0)
	}

	/// Runs `f` on every lane, and returns the values in lane order once every lane finished.
	#[allow(unsafe_code, reason = "Each lane writes its value to its own part of the section's slot.")]
	fn each<T, F>(self, f: F) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if self.slot.is_some() && self.lane_count > COLLECTIVE_LANE_CAPACITY {
			self.gate.poison();
			panic!(
				"Collective lane capacity exceeded. Section {} has {} lanes but keeps values for at most {COLLECTIVE_LANE_CAPACITY}.",
				self.index, self.lane_count
			);
		}
		self.gate.enter(self.index, false, self.slot.is_some());

		let value = self.compute(f);
		if let Some(slot) = self.slot {
			// SAFETY: The capacity check keeps this lane's slot within storage, `ConcreteLane::next_section`
			// checked that a `T` fits it, and only this lane writes it. The other lanes read it after the
			// section ended.
			unsafe { slot.write(self.lane_idx, value) };
		}
		self.gate.finish(self.index, self.lane_count);
		// SAFETY: The section ended, which it does once every lane wrote its value.
		unsafe { self.values(self.lane_count) }
	}

	/// Runs `f`, and fails the section for every lane when it panics.
	fn compute<T, F>(&self, f: F) -> T
	where
		F: FnOnce() -> T,
	{
		match catch_unwind(AssertUnwindSafe(f)) {
			Ok(value) => value,
			Err(payload) => {
				self.gate.poison();
				resume_unwind(payload);
			}
		}
	}

	/// Returns the values that the first `lanes` lanes gave the section, and fails it unless each is a `T`.
	///
	/// Every lane checks every value it can read, so lanes that disagree on the type never read each other's values.
	///
	/// # Safety
	///
	/// The section must have ended. If it keeps values, each of the first `lanes` lanes must have written one.
	#[allow(unsafe_code, reason = "Reads the types lanes recorded next to their values.")]
	unsafe fn values<T: 'static>(&self, lanes: usize) -> LaneValues<'dispatch, T> {
		let Some(slot) = self.slot else {
			return LaneValues::new(std::ptr::NonNull::<T>::dangling().as_ptr().cast(), lanes);
		};
		// SAFETY: The caller guarantees that each of these lanes wrote a value, and with it the value's type.
		if (0..lanes).any(|lane_idx| unsafe { slot.value_type(lane_idx) } != TypeId::of::<T>()) {
			self.gate.poison();
			panic!(
				"Collective type mismatch at section {}. Every lane must use the same result type for that section.",
				self.index
			);
		}
		LaneValues::new(slot.storage.get().cast(), lanes)
	}
}

/// The `CollectiveSlot` struct keeps the values lanes give one collective section until the dispatch ends.
///
/// The [`SectionGate`] decides when a lane may write its value and when all lanes may read them.
struct CollectiveSlot {
	/// The type of the value each lane wrote, so that lanes that disagree on it never read each other's values.
	types: UnsafeCell<[MaybeUninit<TypeId>; COLLECTIVE_LANE_CAPACITY]>,
	storage: UnsafeCell<CollectiveStorage>,
}

impl CollectiveSlot {
	fn new() -> Self {
		Self {
			types: UnsafeCell::new([MaybeUninit::uninit(); COLLECTIVE_LANE_CAPACITY]),
			storage: UnsafeCell::new(CollectiveStorage::new()),
		}
	}

	/// Keeps `value` as the one lane `lane_idx` gives the section.
	///
	/// # Safety
	///
	/// `lane_idx` must be below [`COLLECTIVE_LANE_CAPACITY`], `T` must fit a lane's storage in size and alignment, and
	/// no other lane may read or write this lane's value during the call.
	#[allow(unsafe_code, reason = "Collective values occupy cache-line-sized inline slots.")]
	unsafe fn write<T: 'static>(&self, lane_idx: usize, value: T) {
		// SAFETY: The caller keeps the lane within capacity and other lanes away from its value.
		unsafe {
			self.types.get().cast::<TypeId>().add(lane_idx).write(TypeId::of::<T>());
			self.value_ptr::<T>(lane_idx).write(value);
		}
	}

	/// Returns the type of the value lane `lane_idx` wrote.
	///
	/// # Safety
	///
	/// The lane must have written a value with [`Self::write`], and no lane may be writing it.
	#[allow(unsafe_code, reason = "Reads a type a lane wrote before the section ended.")]
	unsafe fn value_type(&self, lane_idx: usize) -> TypeId {
		// SAFETY: The caller guarantees the lane initialized this entry and that nothing writes it now.
		unsafe { self.types.get().cast::<TypeId>().add(lane_idx).read() }
	}

	#[allow(unsafe_code, reason = "Collective values occupy cache-line-sized inline slots.")]
	unsafe fn value_ptr<T>(&self, lane_idx: usize) -> *mut T {
		// SAFETY: Callers validate lane capacity, value size, and alignment. The storage
		// and every lane stride are aligned to `COLLECTIVE_VALUE_ALIGNMENT`.
		unsafe {
			self.storage
				.get()
				.cast::<u8>()
				.add(lane_idx * COLLECTIVE_VALUE_SIZE)
				.cast::<T>()
		}
	}
}

// SAFETY: A lane writes only its own value and type, while the section runs. Lanes read values and types after
// the section ended, which the gate publishes with release ordering and lanes observe with acquire ordering.
#[allow(unsafe_code, reason = "The section gate synchronizes CollectiveSlot inline storage.")]
unsafe impl Sync for CollectiveSlot {}

#[repr(C, align(64))]
struct CollectiveStorage {
	bytes: [MaybeUninit<u8>; COLLECTIVE_LANE_CAPACITY * COLLECTIVE_VALUE_SIZE],
}

impl CollectiveStorage {
	fn new() -> Self {
		Self {
			bytes: [MaybeUninit::uninit(); COLLECTIVE_LANE_CAPACITY * COLLECTIVE_VALUE_SIZE],
		}
	}
}

fn wait_briefly(spins: &mut usize) {
	if *spins < 64 {
		spin_loop();
		*spins += 1;
	} else {
		yield_now();
	}
}

impl Alley {
	/// Creates an alley with one lane for each available hardware thread.
	///
	/// Call [`Self::execute`] or [`Self::execute_shared_mut`] to dispatch work.
	pub fn new() -> Self {
		Self {
			threadpool: LanePool::new(),
		}
	}

	/// Creates an alley with `count` persistent worker lanes.
	///
	/// Call [`Self::execute`] or [`Self::execute_shared_mut`] to dispatch work.
	pub fn with_parallelism(count: usize) -> Self {
		Self {
			threadpool: LanePool::with_parallelism(count),
		}
	}

	/// Returns the number of lanes every full dispatch runs.
	///
	/// Use it to size per-lane storage before a dispatch indexes it by [`Lane::idx`].
	pub fn parallelism(&self) -> usize {
		self.threadpool.parallelism()
	}

	/// Runs worker work alongside work confined to the calling thread, then returns both results.
	///
	/// Neither caller work nor its result needs to be `Send`. Both jobs finish before a captured panic
	/// is returned, so their borrowed inputs remain valid. The alley can then run another [`Self::join`]
	/// or a collective [`Self::execute`] dispatch.
	pub fn join<R: Send, C>(
		&mut self,
		worker: impl FnOnce() -> R + Send,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<(R, C)> {
		self.threadpool.try_join(worker, caller)
	}

	/// Executes `f` once on every lane and returns after all lanes finish.
	///
	/// Mutable access prevents overlapping or nested dispatches on this `Alley`. Gang-scheduled
	/// lane collectives require exclusive access to its worker set and could otherwise deadlock.
	/// On success, the lane-ordered vector contains exactly one value per alley lane.
	/// Returns the first captured panic if any lane panics. The alley remains available for later dispatches.
	pub fn execute<R>(&mut self, f: impl AlleyFunction<R>) -> std::thread::Result<Vec<R>>
	where
		R: Send,
	{
		let state = DispatchState::new(self.threadpool.parallelism());
		let state = &state;
		self.threadpool
			.try_dispatch_all(move |lane_idx| f(&mut ConcreteLane::new(lane_idx, state)))
	}

	/// Executes `f` once on each of the first `lane_count` lanes and returns after they finish.
	///
	/// The other lanes stay asleep and take no part in the dispatch's collectives. One lane alone runs on the
	/// calling thread.
	fn execute_lanes<R>(&mut self, lane_count: usize, f: impl AlleyFunction<R>) -> std::thread::Result<Vec<R>>
	where
		R: Send,
	{
		let state = DispatchState::new(lane_count);
		let state = &state;
		if lane_count == 1 {
			// One lane has no other lane to meet, so waking a worker for it would only add the time that takes.
			return catch_unwind(AssertUnwindSafe(|| vec![f(&mut ConcreteLane::new(0, state))]));
		}
		let jobs = (0..lane_count).map(|lane_idx| {
			let f = f.clone();
			move || f(&mut ConcreteLane::new(lane_idx, state))
		});
		self.threadpool.try_dispatch_many(jobs)
	}

	/// Executes `f` once on every lane alongside work confined to the calling thread, then returns both results.
	///
	/// Lanes follow the rules of [`Self::execute`]. The caller and its result stay on this thread and need not be
	/// `Send`, as with [`Self::join`]. Every lane and the caller finish before a captured panic is returned, so
	/// their borrowed inputs remain valid. Use this to overlap lane work with a caller that owns thread-bound state.
	pub fn execute_with_caller<R, C>(
		&mut self,
		f: impl AlleyFunction<R>,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<(Vec<R>, C)>
	where
		R: Send,
	{
		let lane_count = self.threadpool.parallelism();
		let state = DispatchState::new(lane_count);
		let state = &state;
		let jobs = (0..lane_count).map(|lane_idx| {
			let f = f.clone();
			move || f(&mut ConcreteLane::new(lane_idx, state))
		});
		self.threadpool.try_dispatch_many_with_caller(jobs, caller)
	}

	/// Executes every lane with mutable resources assigned to stable owner lanes.
	///
	/// Resources are assigned by tuple position in round-robin order across this alley's
	/// lanes. The blocking dispatch keeps every mutable borrow bound until all lanes finish.
	/// Mutable access to the `Alley` prevents another dispatch from overlapping those borrows.
	/// Use [`ConcreteLane::only_one_runs_mut`] to access each resource without synchronization.
	pub fn execute_with_mut<Resources, R, F>(&mut self, resources: Resources, f: F) -> std::thread::Result<Vec<R>>
	where
		Resources: IntoLaneResources,
		R: Send,
		F: for<'dispatch, 'lane, 'resources> Fn(&'lane mut ConcreteLane<'dispatch>, &'resources Resources::Bound) -> R
			+ Clone
			+ Send,
	{
		let parallelism = self.threadpool.parallelism();
		let mut next_index = 0;
		let resources = resources.bind(parallelism, &mut next_index, lane_resources_private::BindToken);
		let resources = &resources;
		self.execute(move |lane| f(lane, resources))
	}

	/// Executes up to `lanes` lanes with one resource that they access in alternating exclusive and shared phases.
	///
	/// Use this to run a whole algorithm as one lane function: [`ConcreteLane::broadcast_mut`] runs a serial stage
	/// on one lane while the others wait, and [`ConcreteLane::each_ref`] runs a parallel stage on every running
	/// lane. The blocking dispatch keeps the mutable borrow bound until all lanes finish.
	///
	/// Pass [`Self::parallelism`] to run every lane. Pass fewer when the work is too small to pay for all of them:
	/// every stage ends only once each running lane has reached it, so lanes with little to do slow a small job
	/// down. The lanes left out stay asleep, [`ConcreteLane::share`] splits work among the running lanes only, and
	/// the result holds one value for each of them. A count above [`Self::parallelism`] runs every lane, a count of
	/// one runs `f` on the calling thread, and a count of zero runs none, so `f` is never called.
	pub fn execute_phased<T, R, F>(&mut self, resource: &mut T, lanes: usize, f: F) -> std::thread::Result<Vec<R>>
	where
		T: Send + Sync,
		R: Send,
		F: for<'dispatch, 'lane, 'resource, 'value> Fn(
				&'lane mut ConcreteLane<'dispatch>,
				&'resource LanePhased<'value, T>,
			) -> R
			+ Clone
			+ Send,
	{
		let resource = LanePhased {
			value: UnsafeCell::new(resource),
		};
		let resource = &resource;
		self.execute_lanes(lanes.min(self.threadpool.parallelism()), move |lane| f(lane, resource))
	}

	/// Runs resource-owning lanes alongside caller-bound work, then joins every lane.
	///
	/// Resources follow the ownership rules of [`Self::execute_with_mut`]. The caller and its result
	/// stay on this thread; all jobs finish before a panic is returned. Use this to overlap CPU setup
	/// with graphics work that must remain on the caller.
	pub fn join_with_mut<Resources, F, C>(
		&mut self,
		resources: Resources,
		f: F,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<C>
	where
		Resources: IntoLaneResources,
		F: for<'dispatch> Fn(&mut ConcreteLane<'dispatch>, &Resources::Bound) + Clone + Send,
	{
		let parallelism = self.threadpool.parallelism();
		let mut resource_count = 0;
		let resources = resources.bind(parallelism, &mut resource_count, lane_resources_private::BindToken);
		// Lanes without resources have no work in this dispatch. Keep them asleep.
		let active_lanes = resource_count.min(parallelism);
		let state = DispatchState::new(active_lanes);
		let resources = &resources;
		let state = &state;
		let jobs = (0..active_lanes).map(|index| {
			let f = f.clone();
			move || f(&mut ConcreteLane::new(index, state), resources)
		});
		self.threadpool
			.try_dispatch_many_with_caller(jobs, caller)
			.map(|(_, caller)| caller)
	}

	/// Executes every lane with exclusive access to a balanced mutable partition.
	///
	/// Mutable access prevents overlapping or nested dispatches on this `Alley`. Gang-scheduled
	/// lane collectives require exclusive access to its worker set and could otherwise deadlock.
	pub fn execute_shared_mut<T, F>(&mut self, values: &mut [T], f: F)
	where
		T: Send,
		F: for<'dispatch, 'values> Fn(&mut SharedLane<'dispatch, 'values, T>) + Clone + Send,
	{
		let lane_count = self.threadpool.parallelism();
		let state = DispatchState::new(lane_count);
		let state = &state;
		let value_count = values.len();
		let mut remaining = Some(values);
		let mut lane_idx = 0;

		// Build one job per lane so every lane reaches collectives, including lanes
		// whose partition is empty when there are fewer values than lanes.
		let jobs = std::iter::from_fn(move || {
			if lane_idx == lane_count {
				return None;
			}

			let values = remaining
				.take()
				.expect("Shared Alley partitioning failed. The previous lane consumed the remaining slice.");
			let partition_len = balanced_partition_len(value_count, lane_idx, lane_count);
			let (partition, rest) = values.split_at_mut(partition_len);
			remaining = Some(rest);

			let current_lane_idx = lane_idx;
			lane_idx += 1;
			let f = f.clone();
			Some(move || {
				let lane = ConcreteLane::new(current_lane_idx, state);
				f(&mut SharedLane::new(lane, partition));
			})
		});

		self.threadpool.dispatch_many(jobs);
	}

	/// Distributes `items` evenly and gives each lane exclusive access to one partition.
	///
	/// Mutable access prevents overlapping or nested dispatches on this `Alley`. Gang-scheduled
	/// lane collectives require exclusive access to its worker set and could otherwise deadlock.
	pub fn for_each_mut<T, F>(&mut self, items: &mut [T], f: F)
	where
		T: Send,
		F: for<'dispatch> Fn(&mut ConcreteLane<'dispatch>, &mut [T]) + Clone + Send,
	{
		let job_count = self.threadpool.parallelism().min(items.len());

		if job_count == 0 {
			return;
		}

		let minimum_chunk_len = items.len() / job_count;
		let larger_chunk_count = items.len() % job_count;
		let state = DispatchState::new(job_count);
		let state = &state;
		let mut remaining = Some(items);
		let mut lane_idx = 0;

		// Produce disjoint lane jobs lazily so the pool can submit all work before waiting.
		let jobs = std::iter::from_fn(move || {
			if lane_idx == job_count {
				return None;
			}

			let chunk_len = minimum_chunk_len + usize::from(lane_idx < larger_chunk_count);
			let (chunk, rest) = remaining
				.take()
				.expect("Alley partitioning failed. The previous lane consumed the remaining slice.")
				.split_at_mut(chunk_len);
			remaining = Some(rest);

			let current_lane_idx = lane_idx;
			lane_idx += 1;
			let f = f.clone();
			Some(move || {
				f(&mut ConcreteLane::new(current_lane_idx, state), chunk);
			})
		});

		self.threadpool.dispatch_many(jobs);
	}
}

impl Default for Alley {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use std::{
		panic::resume_unwind,
		sync::{
			Barrier,
			atomic::{AtomicUsize, Ordering},
		},
		thread::sleep,
		time::Duration,
	};

	use crate::core::alley::{Alley, Lane};

	fn unwrap_dispatch<R>(result: std::thread::Result<Vec<R>>) -> Vec<R> {
		result.unwrap_or_else(|payload| resume_unwind(payload))
	}

	#[test]
	fn join_overlaps_borrowed_work_and_keeps_caller_state_on_its_thread() {
		let mut alley = Alley::with_parallelism(1);
		let caller_thread = std::thread::current().id();
		let local = std::rc::Rc::new(std::cell::Cell::new(0));
		let mut value = 3;
		let (started, waiting) = std::sync::mpsc::channel();
		let (release, released) = std::sync::mpsc::channel();
		let value_ref = &mut value;
		let (worker_thread, returned) = alley
			.join(
				move || {
					started.send(()).unwrap();
					released.recv_timeout(Duration::from_secs(5)).unwrap();
					*value_ref *= 2;
					std::thread::current().id()
				},
				|| {
					waiting.recv_timeout(Duration::from_secs(5)).unwrap();
					assert_eq!(std::thread::current().id(), caller_thread);
					local.set(7);
					release.send(()).unwrap();
					local.clone()
				},
			)
			.unwrap();
		assert_ne!(worker_thread, caller_thread);
		assert_eq!(value, 6);
		assert_eq!(returned.get(), 7);
		assert_eq!(alley.join(std::thread::current, || ()).unwrap().0.id(), worker_thread);
		assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0]);
	}

	#[test]
	fn join_waits_for_borrowed_worker_after_caller_panics() {
		let mut alley = Alley::with_parallelism(1);
		let mut finished = false;
		let (release, released) = std::sync::mpsc::channel();
		let finished_ref = &mut finished;
		let result = alley.join(
			move || {
				released.recv_timeout(Duration::from_secs(5)).unwrap();
				sleep(Duration::from_millis(10));
				*finished_ref = true;
			},
			|| {
				release.send(()).unwrap();
				panic!("caller failed");
			},
		);
		assert!(result.is_err());
		assert!(finished);
		assert_eq!(alley.join(|| 2, || 3).unwrap(), (2, 3));
	}

	#[test]
	fn resource_lanes_overlap_caller_and_remain_reusable_after_panics() {
		for panic_lane in [None, Some(0), Some(1), Some(2)] {
			let mut alley = Alley::with_parallelism(2);
			let mut first = None;
			let mut second = None;
			let barrier = Barrier::new(3);
			let caller_thread = std::thread::current().id();
			let local = std::rc::Rc::new(7);
			let result = alley.join_with_mut(
				(&mut first, &mut second),
				|lane, (first, second)| {
					lane.only_one_runs_mut(first, |value| {
						*value = Some(std::thread::current().id());
						barrier.wait();
						assert_ne!(panic_lane, Some(0), "first worker failed");
					});
					lane.only_one_runs_mut(second, |value| {
						*value = Some(std::thread::current().id());
						barrier.wait();
						assert_ne!(panic_lane, Some(1), "second worker failed");
					});
				},
				|| {
					barrier.wait();
					assert_eq!(std::thread::current().id(), caller_thread);
					assert_ne!(panic_lane, Some(2), "caller failed");
					local.clone()
				},
			);
			assert_ne!(first, second);
			assert_ne!(first, Some(caller_thread));
			assert_ne!(second, Some(caller_thread));
			assert_eq!(result.is_err(), panic_lane.is_some());
			if let Ok(value) = result {
				assert_eq!(*value, 7);
			}
			assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0, 1]);
		}
	}

	#[test]
	fn resource_join_runs_all_initializers_on_a_single_available_lane() {
		let mut alley = Alley::with_parallelism(1);
		let (mut first, mut second) = (0, 0);
		alley
			.join_with_mut(
				(&mut first, &mut second),
				|lane, (first, second)| {
					lane.only_one_runs_mut(first, |value| *value = 4);
					lane.only_one_runs_mut(second, |value| *value = 9);
				},
				|| (),
			)
			.unwrap();
		assert_eq!((first, second), (4, 9));
	}

	#[test]
	fn execute_returns_one_ordered_value_per_lane() {
		let mut alley = Alley::with_parallelism(4);

		let values = unwrap_dispatch(alley.execute(|lane| lane.idx() * 10));

		assert_eq!(values, [0, 10, 20, 30]);
	}

	#[test]
	fn execute_with_caller_overlaps_every_lane_with_caller_bound_work() {
		let mut alley = Alley::with_parallelism(3);
		// Every lane and the caller must wait together, so the barrier only opens when all of them overlap.
		let barrier = Barrier::new(4);
		let caller_thread = std::thread::current().id();
		let local = std::rc::Rc::new(7);

		let (lanes, returned) = alley
			.execute_with_caller(
				|lane| {
					barrier.wait();
					assert_ne!(std::thread::current().id(), caller_thread);
					lane.idx()
				},
				|| {
					barrier.wait();
					assert_eq!(std::thread::current().id(), caller_thread);
					local.clone()
				},
			)
			.unwrap_or_else(|payload| resume_unwind(payload));

		assert_eq!(lanes, [0, 1, 2]);
		assert_eq!(*returned, 7);
	}

	#[test]
	fn execute_with_caller_waits_for_the_caller_after_a_lane_panics() {
		let mut alley = Alley::with_parallelism(2);
		let mut finished = false;

		let result = alley.execute_with_caller(
			|lane| assert_ne!(lane.idx(), 0, "expected lane panic"),
			|| {
				sleep(Duration::from_millis(10));
				finished = true;
			},
		);

		assert!(result.is_err());
		assert!(finished);
		assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0, 1]);
	}

	#[test]
	fn execute_waits_for_all_lanes_after_a_panic_and_remains_reusable() {
		let mut alley = Alley::with_parallelism(4);
		let started = Barrier::new(4);
		let completed = AtomicUsize::new(0);

		let result = alley.execute(|lane| {
			// Start every lane before the panic so completion proves the whole batch was joined.
			started.wait();
			if lane.idx() == 0 {
				panic!("expected lane panic");
			}

			sleep(Duration::from_millis(10));
			completed.fetch_add(1, Ordering::Relaxed);
			lane.idx()
		});

		assert!(result.is_err());
		assert_eq!(completed.load(Ordering::Relaxed), 3);

		let values = unwrap_dispatch(alley.execute(|lane| lane.idx()));

		assert_eq!(values, [0, 1, 2, 3]);
	}

	#[test]
	fn each_partition_receives_its_lane_index() {
		let mut alley = Alley::with_parallelism(3);
		let mut counters = [0; 10];

		alley.for_each_mut(&mut counters, |lane, partition| {
			partition.fill(lane.idx());
		});

		assert_eq!(counters, [0, 0, 0, 0, 1, 1, 1, 2, 2, 2]);
	}

	#[test]
	fn only_one_runs() {
		let first_counter = AtomicUsize::new(0);
		let second_counter = AtomicUsize::new(0);

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			lane.only_one_runs(|| {
				first_counter.fetch_add(1, Ordering::Relaxed);
			});
			lane.only_one_runs(|| {
				second_counter.fetch_add(1, Ordering::Relaxed);
			});
		}));

		assert_eq!(first_counter.load(Ordering::Relaxed), 1);
		assert_eq!(second_counter.load(Ordering::Relaxed), 1);
	}

	#[test]
	fn limited_parallelism_runs_only_the_first_lanes() {
		let mut alley = Alley::with_parallelism(8);
		let first_section = AtomicUsize::new(0);
		let second_section = AtomicUsize::new(0);
		let zero_section = AtomicUsize::new(0);

		unwrap_dispatch(alley.execute(|lane| {
			lane.with_limited_parallelism(3, || {
				first_section.fetch_add(1, Ordering::Relaxed);
			});
			lane.with_limited_parallelism(5, || {
				second_section.fetch_add(1, Ordering::Relaxed);
			});
			lane.with_limited_parallelism(0, || {
				zero_section.fetch_add(1, Ordering::Relaxed);
			});
		}));

		assert_eq!(first_section.load(Ordering::Relaxed), 3);
		assert_eq!(second_section.load(Ordering::Relaxed), 5);
		assert_eq!(zero_section.load(Ordering::Relaxed), 0);
	}

	#[test]
	fn broadcast() {
		let counter = AtomicUsize::new(0);
		let initializer_count = AtomicUsize::new(0);

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			let first = lane.broadcast(|| {
				initializer_count.fetch_add(1, Ordering::Relaxed);
				1usize
			});
			let second = lane.broadcast(|| {
				initializer_count.fetch_add(1, Ordering::Relaxed);
				2u16
			});

			counter.fetch_add(first + usize::from(second), Ordering::Relaxed);
		}));

		assert_eq!(initializer_count.load(Ordering::Relaxed), 2);
		assert_eq!(counter.load(Ordering::Relaxed), 24);
	}

	#[test]
	fn broadcast_panic_releases_waiting_lanes() {
		let mut alley = Alley::with_parallelism(8);
		let result = alley.execute(|lane| {
			let _: usize = lane.broadcast(|| panic!("expected broadcast panic"));
		});

		assert!(result.is_err());

		let completed = AtomicUsize::new(0);
		unwrap_dispatch(alley.execute(|_| {
			completed.fetch_add(1, Ordering::Relaxed);
		}));

		assert_eq!(completed.load(Ordering::Relaxed), 8);
	}

	#[test]
	fn each() {
		let counter = AtomicUsize::new(0);
		let initializer_count = AtomicUsize::new(0);

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let all = lane.each(|| {
				initializer_count.fetch_add(1, Ordering::Relaxed);
				1usize
			});
			let lane_indices = lane.each(|| lane_idx);

			assert_eq!(lane_indices.collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
			counter.fetch_add(all.sum::<usize>(), Ordering::Relaxed);
		}));

		assert_eq!(initializer_count.load(Ordering::Relaxed), 8);
		assert_eq!(counter.load(Ordering::Relaxed), 64);
	}

	#[test]
	fn each_iterates_small_and_full_padded_values_in_lane_order() {
		#[derive(Copy, Clone, Debug, Eq, PartialEq)]
		struct Value64([u8; 64]);

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let mut small = lane.each(|| lane_idx as u8);
			let full = lane.each(|| Value64([lane_idx as u8; 64]));

			assert_eq!(small.size_hint(), (8, Some(8)));
			assert_eq!(small.clone().collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
			assert_eq!(small.nth(2), Some(2));
			assert_eq!(small.len(), 5);
			for (expected_lane, value) in full.enumerate() {
				assert_eq!(value, Value64([expected_lane as u8; 64]));
			}
		}));
	}

	#[test]
	fn collective_operation_mismatch_releases_waiting_lanes() {
		let mut alley = Alley::with_parallelism(8);
		let result = alley.execute(|lane| {
			if lane.idx() == 0 {
				let _ = lane.broadcast(|| 1usize);
			} else {
				let _ = lane.each(|| 1usize);
			}
		});

		assert!(result.is_err());
	}

	#[test]
	fn lanes_that_disagree_on_a_value_type_fail_the_dispatch() {
		let mut alley = Alley::with_parallelism(8);
		let result = alley.execute(|lane| {
			if lane.idx() == 0 {
				let _ = lane.each(|| 1u64);
			} else {
				let _ = lane.each(|| 1usize);
			}
		});

		assert!(result.is_err());
	}

	#[test]
	fn lanes_that_disagree_on_whether_a_section_has_a_value_fail_the_dispatch() {
		let mut alley = Alley::with_parallelism(8);
		let result = alley.execute(|lane| {
			if lane.idx() == 0 {
				lane.broadcast(|| ());
			} else {
				let _ = lane.broadcast(|| 1usize);
			}
		});

		assert!(result.is_err());
	}

	#[test]
	fn a_lane_that_enters_a_broadcast_late_with_another_kind_of_value_fails_the_dispatch() {
		let mut alley = Alley::with_parallelism(2);

		for late_has_value in [false, true] {
			let ahead = AtomicUsize::new(0);
			let result = alley.execute(|lane| {
				if lane.idx() == 0 {
					// The first lane is two sections ahead before the other one enters the first section.
					for _ in 0..2 {
						if late_has_value {
							lane.broadcast(|| ());
						} else {
							let _ = lane.broadcast(|| 1usize);
						}
					}
					ahead.store(1, Ordering::Release);
					return;
				}
				while ahead.load(Ordering::Acquire) == 0 {
					std::thread::yield_now();
				}
				if late_has_value {
					let _ = lane.broadcast(|| 1usize);
				} else {
					lane.broadcast(|| ());
				}
			});

			assert!(result.is_err());
		}
	}

	#[test]
	fn a_dispatch_runs_any_number_of_sections_without_values() {
		let mut alley = Alley::with_parallelism(4);
		let broadcasts = AtomicUsize::new(0);
		let eaches = AtomicUsize::new(0);

		unwrap_dispatch(alley.execute(|lane| {
			for _ in 0..1_000 {
				lane.broadcast(|| {
					broadcasts.fetch_add(1, Ordering::Relaxed);
				});
				let all = lane.each(|| {
					eaches.fetch_add(1, Ordering::Relaxed);
				});
				assert_eq!(all.len(), 4);
			}
			// Sections with values still work after them.
			assert_eq!(lane.broadcast(|| 7usize), 7);
		}));

		assert_eq!(broadcasts.load(Ordering::Relaxed), 1_000);
		assert_eq!(eaches.load(Ordering::Relaxed), 4_000);
	}

	#[test]
	fn a_dispatch_fails_when_it_runs_out_of_room_for_values() {
		let mut alley = Alley::with_parallelism(4);
		let result = alley.execute(|lane| {
			for value in 0..33usize {
				let _ = lane.broadcast(|| value);
			}
		});

		assert!(result.is_err());
	}

	#[test]
	fn each_shared_balances_uneven_and_empty_partitions() {
		let mut alley = Alley::with_parallelism(5);
		let values = [1usize, 2, 3];

		unwrap_dispatch(alley.execute(|lane| {
			let lengths = lane.each_shared(&values, <[usize]>::len);
			let sums = lane.each_shared(&values, |partition| partition.iter().sum::<usize>());

			assert_eq!(lengths.collect::<Vec<_>>(), [1, 1, 1, 0, 0]);
			assert_eq!(sums.collect::<Vec<_>>(), [1, 2, 3, 0, 0]);
		}));
	}

	#[test]
	fn each_shared_mut() {
		let value = AtomicUsize::new(0);
		let mut values = vec![0; 16];

		let mut alley = Alley::with_parallelism(8);

		alley.execute_shared_mut(&mut values, |lane| {
			let lane_idx = lane.idx();
			let all = lane.each_shared_mut(|partition| {
				for (offset, value) in partition.iter_mut().enumerate() {
					*value = lane_idx * 2 + offset + 1;
				}
				partition.iter().sum::<usize>()
			});

			lane.only_one_runs(|| {
				value.fetch_add(all.sum::<usize>(), Ordering::Relaxed);
			});
		});

		assert_eq!(values, (1..=16).collect::<Vec<_>>());
		assert_eq!(value.load(Ordering::Relaxed), 136);
	}

	#[test]
	fn phased_resource_alternates_exclusive_and_shared_stages() {
		let mut alley = Alley::with_parallelism(4);
		let mut values = Vec::new();
		let writers = AtomicUsize::new(0);

		let totals = unwrap_dispatch(alley.execute_phased(&mut values, 4, |lane, values| {
			// One lane fills the resource while the others wait for it.
			let count = lane.broadcast_mut(values, |values| {
				writers.fetch_add(1, Ordering::Relaxed);
				values.extend(1..=10usize);
				values.len()
			});
			// Every lane then reads the finished resource and sums its own share.
			let sums = lane.each_ref(values, |values, lane| {
				assert_eq!(values.len(), count);
				lane.share(values).iter().sum::<usize>()
			});
			let total = sums.sum::<usize>();
			// A later exclusive stage sees the resource again after every reader left.
			lane.broadcast_mut(values, |values| {
				values.push(total);
				total
			})
		}));

		assert_eq!(writers.load(Ordering::Relaxed), 1);
		assert_eq!(totals, [55; 4]);
		assert_eq!(values.len(), 11);
		assert_eq!(values[10], 55);
	}

	#[test]
	fn phased_stages_without_values_alternate_any_number_of_times() {
		let mut alley = Alley::with_parallelism(4);
		let mut values = vec![0usize; 64];
		let seen = AtomicUsize::new(0);

		unwrap_dispatch(alley.execute_phased(&mut values, 4, |lane, values| {
			for round in 0..500 {
				// One lane writes every value, then every lane checks its share of them.
				lane.broadcast_mut(values, |values| values.fill(round));
				let _ = lane.each_ref(values, |values, lane| {
					assert!(lane.share(values).iter().all(|value| *value == round));
					seen.fetch_add(lane.share(values).len(), Ordering::Relaxed);
				});
			}
		}));

		assert_eq!(seen.load(Ordering::Relaxed), 500 * 64);
		assert_eq!(values, [499; 64]);
	}

	#[test]
	fn phased_exclusive_stage_panic_releases_waiting_lanes() {
		let mut alley = Alley::with_parallelism(4);
		let mut value = 0usize;

		let result = alley.execute_phased(&mut value, 4, |lane, value| {
			let _: usize = lane.broadcast_mut(value, |_| panic!("expected exclusive stage panic"));
		});

		assert!(result.is_err());
		assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0, 1, 2, 3]);
	}

	#[test]
	fn a_phased_dispatch_of_one_lane_runs_on_the_calling_thread() {
		let mut alley = Alley::with_parallelism(4);
		let caller = std::thread::current().id();
		let mut value = 0usize;

		let threads = unwrap_dispatch(alley.execute_phased(&mut value, 1, |lane, value| {
			lane.broadcast_mut(value, |value| *value += 1);
			let _ = lane.each_ref(value, |value, _| assert_eq!(*value, 1));
			std::thread::current().id()
		}));

		assert_eq!(threads, [caller]);
		assert_eq!(value, 1);

		// A panic on the calling thread is returned like one on a lane, and the alley stays usable.
		let panicked = alley.execute_phased(&mut value, 1, |_, _| panic!("expected lane panic"));
		assert!(panicked.is_err());
		assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0, 1, 2, 3]);
	}

	#[test]
	fn phased_dispatch_runs_only_the_lanes_it_is_given() {
		let mut alley = Alley::with_parallelism(4);
		let values = (1..=10usize).collect::<Vec<_>>();

		for (lanes, running) in [(1, 1), (2, 2), (4, 4), (9, 4)] {
			let mut shared = values.clone();
			let sums = unwrap_dispatch(alley.execute_phased(&mut shared, lanes, |lane, values| {
				// Each running lane sums its share, and every lane reads all the sums once the stage ends.
				let sums = lane.each_ref(values, |values, lane| lane.share(values).iter().sum::<usize>());
				let (count, total) = sums.fold((0, 0), |(count, total), sum| (count + 1, total + sum));
				(lane.idx(), count, total)
			}));

			// The running lanes are the first ones, they meet without the others, and their shares cover the values.
			let expected = (0..running).map(|lane| (lane, running, 55)).collect::<Vec<_>>();
			assert_eq!(sums, expected);
		}
	}

	#[test]
	fn mutable_resources_use_round_robin_stable_owner_lanes() {
		let mut first_owner = usize::MAX;
		let mut second_owner = usize::MAX;
		let mut third_owner = usize::MAX;
		let mut fourth_owner = usize::MAX;
		let mut history = Vec::new();
		let mut alley = Alley::with_parallelism(3);

		unwrap_dispatch(alley.execute_with_mut(
			(
				&mut first_owner,
				&mut second_owner,
				&mut third_owner,
				&mut fourth_owner,
				&mut history,
			),
			|lane, (first, second, third, fourth, history)| {
				let lane_idx = lane.idx();
				let _ = lane.only_one_runs_mut(first, |owner| *owner = lane_idx);
				let _ = lane.only_one_runs_mut(second, |owner| *owner = lane_idx);
				let _ = lane.only_one_runs_mut(third, |owner| *owner = lane_idx);
				let _ = lane.only_one_runs_mut(fourth, |owner| *owner = lane_idx);
				let _ = lane.only_one_runs_mut(history, |values| values.push((lane_idx, 1)));
				let _ = lane.only_one_runs_mut(history, |values| values.push((lane_idx, 2)));
			},
		));

		assert_eq!([first_owner, second_owner, third_owner, fourth_owner], [0, 1, 2, 0]);
		assert_eq!(history, [(1, 1), (1, 2)]);
	}
}

use std::{
	any::TypeId,
	cell::UnsafeCell,
	hint::spin_loop,
	mem::{MaybeUninit, align_of, size_of},
	ops::{Deref, DerefMut, Range},
	panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
	sync::atomic::{AtomicUsize, Ordering},
	thread::yield_now,
};

use crate::core::threadpool::LanePool;
