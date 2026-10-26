/// The `Alley` struct provides reusable parallel execution across a fixed number of owned lanes.
///
/// The execution model follows Ryan Fleury's [Multi-Core By Default](https://www.dgtlgrove.com/p/multi-core-by-default):
/// lanes run the same function, narrow to serial work, and share results before continuing in parallel.
///
/// A dispatch runs one function on every lane, and the lanes meet in collective sections such as
/// [`ConcreteLane::each`] and [`ConcreteLane::broadcast`]. Sections cost far less than dispatches, which wake
/// threads, and a dispatch allocates nothing for its sections, so prefer one dispatch that runs a whole algorithm
/// over many small dispatches. Call [`Self::execute`] or [`Self::execute_phased`] to dispatch work.
pub struct Alley {
	pool: LanePool,
	/// Tells this alley's lanes from those of any other, for storage that is split between lanes.
	id: AlleyId,
	/// Where the lanes of a dispatch meet. It lives as long as the alley, so a dispatch allocates nothing for it.
	sections: Sections,
	/// The number of the next dispatch's first section. Sections are numbered across dispatches, so nothing that
	/// lanes posted in an earlier dispatch reads as part of a later one.
	next_section: u64,
}

/// The `AlleyFunction` trait defines value-producing work that every lane runs during an [`Alley`] dispatch.
///
/// Every lane calls the same function, so the lanes share it instead of each taking a copy.
pub trait AlleyFunction<R> = for<'dispatch> Fn(&mut ConcreteLane<'dispatch>) -> R + Sync;

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

// SAFETY: Lanes only reach the resource inside collective sections. A broadcast section admits only the lane that took
// it, once no other lane reaches the resource, while the others wait. An `each` section that reaches the resource
// admits only shared references, once no broadcast runs, and no lane leaves it before every lane's work there
// returned. `Sections` explains why a lane never enters one while another runs the other. The blocking dispatch
// prevents either access from outliving the original `&mut T`.
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
	/// The first lane's value. Every other lane keeps its value one lane's storage further on. Only read for values
	/// with a size.
	first: *const u8,
	next_lane: usize,
	lane_count: usize,
	marker: PhantomData<&'dispatch T>,
}

impl<'dispatch, T> LaneValues<'dispatch, T> {
	/// Makes the values of `lane_count` lanes, the first one's at `first`, which is only read for values with a size.
	fn new(first: *const u8, lane_count: usize) -> Self {
		Self {
			first,
			next_lane: 0,
			lane_count,
			marker: PhantomData,
		}
	}

	#[allow(unsafe_code, reason = "Lane values are copied from synchronized strided storage.")]
	fn read_lane(&self, lane_idx: usize) -> T
	where
		T: Copy,
	{
		if size_of::<T>() == 0 {
			// SAFETY: A section makes these values only after the lanes' work returned a `T`, which this copies. A
			// read of no bytes is valid at any aligned address.
			return unsafe { NonNull::<T>::dangling().as_ptr().read() };
		}
		// SAFETY: The lane index is below the lane count, and each lane's storage follows the one before it, so the
		// address stays within the alley's sections.
		let lane = unsafe { self.first.add(lane_idx * size_of::<LaneBlock>()) };
		// SAFETY: The section checked that a `T` fits a slot, that every lane wrote one there before the section
		// ended, and that every lane wrote the same type. No lane writes the slot again in this dispatch.
		unsafe { lane.cast::<T>().read() }
	}
}

impl<T> Clone for LaneValues<'_, T> {
	fn clone(&self) -> Self {
		Self {
			first: self.first,
			next_lane: self.next_lane,
			lane_count: self.lane_count,
			marker: PhantomData,
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
///
/// No lane leads the dispatch's collective sections. The first lane to reach a broadcast or a claim runs it, so a
/// busy lane holds the others up only where they need it: at the end of an `each` section, and at a broadcast that
/// it runs. No section takes a lock or allocates, so a lane function can run any number of sections without values.
pub struct ConcreteLane<'dispatch> {
	lane_idx: usize,
	lane_count: usize,
	/// The alley that runs the dispatch.
	alley: AlleyId,
	sections: &'dispatch Sections,
	/// The number of the dispatch's first section. It tells what lanes post in this dispatch from what they posted
	/// in earlier ones.
	first_section: u64,
	/// The collective section this lane runs next.
	next_section: u64,
	/// The slot that the next collective section with values keeps them in.
	next_slot: usize,
	/// Keeps a shared reference to a lane on the lane's own thread. Storage that is split between lanes, such as
	/// [`LaneOwned`], takes one as proof that the lane itself is asking.
	on_its_thread: PhantomData<Cell<()>>,
}

impl<'dispatch> ConcreteLane<'dispatch> {
	fn new(lane_idx: usize, lane_count: usize, alley: AlleyId, sections: &'dispatch Sections, first_section: u64) -> Self {
		Self {
			lane_idx,
			lane_count,
			alley,
			sections,
			first_section,
			next_section: first_section,
			next_slot: 0,
			on_its_thread: PhantomData,
		}
	}

	/// Returns what tells the alley that runs this lane from any other.
	///
	/// [`LaneOwned`] and [`LaneRanges`] compare it with the alley they were lent to.
	pub fn alley(&self) -> AlleyId {
		self.alley
	}

	/// Runs `f` on the first lane to reach this claim section, while the other lanes go on at once.
	///
	/// Every lane must reach claim sections such as this one in the same order. Lanes do not wait for each other
	/// here, so a dispatch runs any number of them.
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

	/// Runs `f` on the first `limit` lanes to reach this claim section, while the other lanes go on at once.
	///
	/// Every lane must reach claim sections in the same order and with the same limit. A limit of zero runs `f` on no
	/// lane, and one of at least the lane count runs it on every lane. Lanes do not wait for each other here, so a
	/// dispatch runs any number of these sections. A lane that reaches one only after a lane ahead of it took a later
	/// broadcast or claim does not run `f`, so lanes that drift apart may leave it to fewer than `limit` lanes, but
	/// always to one at least.
	pub fn with_limited_parallelism<F>(&mut self, limit: usize, f: F)
	where
		F: FnOnce(),
	{
		if limit >= self.lane_count {
			// Every lane runs it, so there is nothing to claim. A lane that caught the panic of a failed dispatch runs no
			// further section.
			self.check_failure();
			f();
		} else if limit > 0 && self.claim(limit) {
			f();
		}
	}

	/// Runs `f` with exclusive access to the resource on the first lane to reach this section, and returns its value
	/// to every lane.
	///
	/// Every other lane waits until `f` finishes, so a following [`Self::each_ref`] sees everything `f` wrote.
	/// Every lane must encounter this section in the same order and with the same result type.
	///
	/// Return `()` from stages that run in a loop. A dispatch runs any number of sections that return a value without
	/// size, but it keeps other values until it ends and has room for those of 32 sections.
	#[allow(
		unsafe_code,
		reason = "The lane that runs the broadcast exclusively dereferences the phased resource."
	)]
	pub fn broadcast_mut<T, R, F>(&mut self, resource: &LanePhased<'_, T>, f: F) -> R
	where
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&mut T) -> R,
	{
		let section = self.next_section::<R>();
		self.run_broadcast(section, true, || {
			// SAFETY: Only the lane that runs this broadcast runs this closure, and only once it saw that no other lane
			// reaches the resource. A lane that enters a section to reach the resource from now on sees this broadcast
			// and fails before it does, as `Sections` explains, and no lane passes this section before the closure
			// returned.
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
		lane.run_each(section, Some(0), || {
			// SAFETY: This closure runs once this lane posted that it entered this section to reach the resource, and
			// saw no broadcast in it or running before it, so no lane holds exclusive access. A lane that takes a
			// broadcast from now on sees the post and fails before it reaches the resource, as `Sections` explains. This
			// lane leaves the section only once every lane's closure returned, so no shared reference reaches a later
			// exclusive section.
			let value = unsafe { &**resource.value.get() };
			f(value, lane)
		})
	}

	/// Runs `f` on every lane like [`Self::each_ref`], as a stage that every lane says works on the same `key`, such
	/// as one batch of a list of batches.
	///
	/// `f` receives a [`Stage`], which proves to storage that every lane is in this stage with this key. Storage
	/// that is only safe to write while all lanes work on the same batch asks for one, such as
	/// [`LaneRanges::batch`]. Lanes that pass different keys fail the dispatch, and never run `f` side by side.
	#[allow(unsafe_code, reason = "Every lane shares the phased resource for one collective section.")]
	pub fn each_ref_keyed<T, R, F>(&mut self, resource: &LanePhased<'_, T>, key: u32, f: F) -> LaneValues<'dispatch, R>
	where
		R: Copy + Send + Sync + 'static,
		F: FnOnce(&T, &Stage<'_, 'dispatch>) -> R,
	{
		let section = self.next_section::<R>();
		let stage = Stage { lane: &*self, key };
		stage.lane.run_each(section, Some(key), || {
			// SAFETY: As in `each_ref`: this closure runs once this lane posted its entry with this key and saw no
			// broadcast in this section or running before it, and no lane holds exclusive access until this section
			// ended for every lane.
			let value = unsafe { &**resource.value.get() };
			f(value, &stage)
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
		shared_range(len, self.lane_idx, self.lane_count)
	}

	/// Runs `f` on the first lane to reach this section and returns its value to every lane once it finished.
	///
	/// Every lane must encounter broadcast sections in the same order and with the same types. A dispatch keeps the
	/// values of at most 32 collective sections, and runs any number of sections whose value has no size, such as
	/// `()`.
	pub fn broadcast<T, F>(&mut self, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		let section = self.next_section::<T>();
		self.run_broadcast(section, false, f)
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
		let section = self.next_section::<T>();
		self.run_each(section, None, f)
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
	fn next_section<T>(&mut self) -> Section {
		// A lane that caught the panic of a failed dispatch runs no further section.
		self.check_failure();
		let index = self.next_section;
		self.next_section += 1;
		let valued = size_of::<T>() != 0;
		let slot = self.next_slot;
		// A value without size needs no storage, so only the other sections take one of the dispatch's slots.
		if valued {
			if size_of::<T>() > COLLECTIVE_VALUE_SIZE || align_of::<T>() > COLLECTIVE_VALUE_ALIGNMENT {
				self.fail();
				panic!(
					"Collective value does not fit. Section {} returns a value of more than {COLLECTIVE_VALUE_SIZE} bytes or aligned to more than {COLLECTIVE_VALUE_ALIGNMENT}. The most likely cause is returning the data itself where a count or an index would do.",
					index - self.first_section
				);
			}
			if slot == COLLECTIVE_SECTION_COUNT {
				self.fail();
				panic!(
					"Collective value capacity exceeded. A dispatch keeps the values of at most {COLLECTIVE_SECTION_COUNT} collective sections. The most likely cause is a loop of sections that return a value where `()` would do."
				);
			}
			self.next_slot += 1;
		}
		Section { index, valued, slot }
	}

	/// Returns the number of `section` within the dispatch, counted from one, as a [`Turn`] names it.
	fn number(&self, section: &Section) -> u64 {
		section.index - self.first_section + 1
	}

	/// Runs `f` as the broadcast `section` on the first lane to reach it, and returns its value on every lane once it
	/// finished. `f` has exclusive access to the dispatch's resource when `exclusive`.
	fn run_broadcast<T, F>(&self, section: Section, exclusive: bool, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if self.lane_count == 1 {
			// One lane has no other lane to meet.
			return self.guarded(f);
		}
		self.meet_at_broadcast(section, exclusive, f)
	}

	/// Runs the broadcast `section` as [`Self::run_broadcast`] does, on one of several lanes.
	///
	/// It stays out of line, like [`Self::meet_at_each`], so that broadcasts on one lane keep the code around them
	/// small.
	#[inline(never)]
	#[allow(
		unsafe_code,
		reason = "The lane that runs a broadcast hands its value to the other lanes through the section's slot."
	)]
	fn meet_at_broadcast<T, F>(&self, section: Section, exclusive: bool, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if !self.take_broadcast(&section, exclusive) {
			return self.receive_broadcast(&section);
		}
		if exclusive {
			self.check_exclusive(&section);
		}
		let value = self.guarded(f);
		if section.valued {
			// SAFETY: `Self::next_section` checked that a `T` fits a slot. Only the lane that takes a broadcast writes its
			// slot, which no other section of the dispatch takes, and the other lanes read it once the turn below says that
			// the section ended.
			unsafe { self.sections.values[section.slot].keep(section.index, value) };
		}
		let ended = Turn::broadcast(self.number(&section), section.valued).ended();
		self.sections.turn.0.store(ended.0, Ordering::Release);
		value
	}

	/// Returns whether this lane is the first to reach the broadcast `section`, which makes it the lane that runs it,
	/// with exclusive access to the resource when `exclusive`.
	fn take_broadcast(&self, section: &Section, exclusive: bool) -> bool {
		let number = self.number(section);
		let taken = Turn::broadcast(number, section.valued);
		let turns = &self.sections.turn.0;
		let mut turn = Turn(turns.load(Ordering::Acquire));
		while turn.section() < number {
			// Lanes pass a broadcast only once it ended, so one that has not is in a section this lane ran differently.
			if !turn.has_ended() {
				self.mismatch(section);
			}
			// Sequentially consistent for exclusive access, so that this lane and one that enters a section to reach the
			// resource cannot both miss each other, as `Sections` explains. Other broadcasts publish nothing with it.
			let order = if exclusive { Ordering::SeqCst } else { Ordering::Acquire };
			match turns.compare_exchange_weak(turn.0, taken.0, order, Ordering::Acquire) {
				Ok(_) => return true,
				Err(current) => turn = Turn(current),
			}
		}
		false
	}

	/// Fails the dispatch if another lane reaches the resource in an `each` section, other than one that it left before
	/// the broadcast `section`, which this lane took to run with exclusive access to the resource.
	///
	/// A lane that enters such a section after this check sees that this lane took `section`, and fails instead.
	fn check_exclusive(&self, section: &Section) {
		// Orders the turn this lane took before the posts it reads, as `Sections` explains. The reads need no order
		// among themselves, so they overlap.
		fence(Ordering::SeqCst);
		for (lane_idx, block) in self.sections.lanes[..self.lane_count].iter().enumerate() {
			if lane_idx != self.lane_idx
				&& LaneBlock::reaches(block.post.load(Ordering::Relaxed), self.first_section, section.index)
			{
				self.mismatch(section);
			}
		}
	}

	/// Returns the value that the lane that took the broadcast `section` gave it, once that lane finished, and fails
	/// unless that lane ran the section as this lane does: as a broadcast, with a value of the same type or without
	/// one.
	#[allow(unsafe_code, reason = "Reads the value the lane that ran the broadcast kept in its slot.")]
	fn receive_broadcast<T: Copy + 'static>(&self, section: &Section) -> T {
		let number = self.number(section);
		let slot = self.sections.values.get(section.slot);
		let mut waits = 0;
		let kept = loop {
			let turn = Turn(self.sections.turn.0.load(Ordering::Acquire));
			if turn.section() > number {
				// Lanes pass a broadcast only once it ended. Whether its lane kept a value shows in the slot that a
				// broadcast with a value takes next: only one of this section leaves this section's number there.
				break slot.is_some_and(|slot| slot.holds(section.index));
			}
			// A lane that took this section took it before this lane looked, and the turn never goes back.
			if turn.is_claim() || turn.valued() != section.valued {
				self.mismatch(section);
			}
			if turn.has_ended() {
				break section.valued;
			}
			self.pause(&mut waits);
		};
		if kept != section.valued {
			self.mismatch(section);
		}
		let Some(slot) = slot.filter(|_| section.valued) else {
			// SAFETY: The lane that ran this section returned a `T` from it, which this copies. A read of no bytes is
			// valid at any aligned address.
			return unsafe { NonNull::<T>::dangling().as_ptr().read() };
		};
		// SAFETY: The lane that ran this section kept its value with its type in the slot before the turn said that the
		// section ended, which this lane saw, and no lane writes the slot again in this dispatch.
		if unsafe { slot.value_type() } != TypeId::of::<T>() {
			self.type_mismatch(section);
		}
		// SAFETY: As above, the slot holds a `T`, which the check above confirmed.
		unsafe { slot.read() }
	}

	/// Takes the next section as a claim that the first `limit` lanes to reach it run, and returns whether this lane
	/// is one of them.
	#[inline(never)]
	fn claim(&mut self, limit: usize) -> bool {
		let section = self.next_section::<()>();
		let number = self.number(&section);
		let limit = limit.min(Turn::MOST_CLAIMS);
		let turns = &self.sections.turn.0;
		let mut turn = Turn(turns.load(Ordering::Acquire));
		loop {
			let taken = if turn.section() > number {
				// A lane ahead took a later section, which leaves this one to the lanes that reached it before.
				return false;
			} else if turn.section() == number {
				if !turn.is_claim() {
					self.mismatch(&section);
				}
				if turn.count() >= limit {
					return false;
				}
				turn.taken_again()
			} else {
				// Lanes pass a broadcast only once it ended, so one that has not is in a section this lane ran differently.
				if !turn.has_ended() {
					self.mismatch(&section);
				}
				Turn::claim(number)
			};
			match turns.compare_exchange_weak(turn.0, taken.0, Ordering::Acquire, Ordering::Acquire) {
				Ok(_) => return true,
				Err(current) => turn = Turn(current),
			}
		}
	}

	/// Runs `f` on every lane as the `each` section `section`, and returns the lanes' values in lane order once every
	/// lane finished.
	///
	/// A section that reaches the dispatch's resource passes its key as `entry`: a lane posts that it entered it before
	/// it runs `f`, and checks that no lane entered it another way, with another key, or runs a broadcast that has not
	/// ended. No lane waits for another to enter, and each lane leaves the section as soon as it saw every other lane
	/// arrive.
	#[allow(
		unsafe_code,
		reason = "Each lane writes its value to its own slot, which the other lanes read once it arrived."
	)]
	fn run_each<T, F>(&self, section: Section, entry: Option<u32>, f: F) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if self.lane_count == 1 {
			// One lane has no other lane to meet.
			let value = self.guarded(f);
			if section.valued {
				// SAFETY: `Self::next_section` checked that a `T` fits a slot, and only this lane reaches it.
				unsafe { self.sections.lanes[0].slots[section.slot].write(value) };
			}
			return self.values(&section);
		}
		self.meet_at_each(section, entry, f)
	}

	/// Runs the `each` section `section` as [`Self::run_each`] does, on one of several lanes.
	///
	/// It stays out of line, so that sections on one lane, which skip it, keep the code around them small.
	#[inline(never)]
	#[allow(
		unsafe_code,
		reason = "Each lane writes its value to its own slot, which the other lanes read once it arrived."
	)]
	fn meet_at_each<T, F>(&self, section: Section, entry: Option<u32>, f: F) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		let block = &self.sections.lanes[self.lane_idx];
		if let Some(key) = entry {
			self.enter_shared(&section, key);
		}
		let value = self.guarded(f);
		if section.valued {
			// SAFETY: `Self::next_section` checked that a `T` fits a slot. Only this lane writes its slots, each one once
			// in a dispatch, and the other lanes read this one once this lane posted its arrival.
			unsafe { block.slots[section.slot].put(value) };
		}
		let shared = entry.is_some();
		block.arrive(section.index, shared, section.valued);
		self.await_others::<T>(&section, shared);
		// A lane that has not seen this lane arrive yet finds out from this that the section ended.
		block.complete(section.index);
		self.values(&section)
	}

	/// Posts that this lane enters the `each` section `section` to reach the resource with `key`, and fails unless no
	/// lane entered it another way or with another key, and no lane took a broadcast in it or one that has not ended.
	///
	/// This lane reads the other lanes' posts after its own, and a lane that takes a broadcast reads every post after it
	/// took it, so of two lanes that disagree, at least one sees the other and fails before it reaches the resource. A
	/// lane that has not posted yet checks this lane's post once it does.
	fn enter_shared(&self, section: &Section, key: u32) {
		let entered = LaneBlock::entered(section.index, section.valued);
		self.sections.lanes[self.lane_idx].enter(entered, key);
		let turn = Turn(self.sections.turn.0.load(Ordering::SeqCst));
		// Lanes pass a broadcast only once it ended, so one that has not is in a section this lane ran differently.
		if turn.section() >= self.number(section) || !turn.has_ended() {
			self.mismatch(section);
		}
		let arrived = LaneBlock::arrived_at(section.index, true, section.valued);
		for (lane_idx, block) in self.sections.lanes[..self.lane_count].iter().enumerate() {
			if lane_idx == self.lane_idx {
				continue;
			}
			let posted = block.post.load(Ordering::SeqCst);
			if posted == entered || posted == arrived {
				// The lane wrote its key before its post, which this load acquired, and keeps it until this lane arrives.
				if block.key.load(Ordering::Relaxed) != key {
					self.mismatch(section);
				}
			} else if LaneBlock::posted(posted).0 >= section.index {
				// A lane leaves this section only once this lane arrived, so one that posted anything else here or later
				// runs something else in it.
				self.mismatch(section);
			}
		}
	}

	/// Waits until every other lane arrived at the `each` section `section`, which reaches the resource when `shared`,
	/// and fails unless each ran it as this lane did, with a value of the same type or without one.
	///
	/// A lane that already left the section saw this lane's arrival and checked it, so this lane takes its word that
	/// the section ended.
	#[allow(unsafe_code, reason = "Reads the types the other lanes kept with their values.")]
	fn await_others<T: 'static>(&self, section: &Section, shared: bool) {
		let arrived = LaneBlock::arrived_at(section.index, shared, section.valued);
		// A lane that reaches the resource here is still at work while its post says that it entered.
		let working = if shared {
			LaneBlock::entered(section.index, section.valued)
		} else {
			arrived
		};
		for (lane_idx, block) in self.sections.lanes[..self.lane_count].iter().enumerate() {
			if lane_idx == self.lane_idx {
				continue;
			}
			let mut waits = 0;
			loop {
				let posted = block.post.load(Ordering::Acquire);
				if posted == arrived {
					// SAFETY: The lane kept its value with its type before it posted its arrival, and changes the slot no
					// more in this dispatch.
					if section.valued && unsafe { block.slots[section.slot].value_type() } != TypeId::of::<T>() {
						self.type_mismatch(section);
					}
					break;
				}
				let (posted_section, finished) = LaneBlock::posted(posted);
				// What lanes posted in an earlier dispatch numbers sections before this one's.
				if posted_section >= self.first_section {
					if posted_section > section.index || finished {
						// The lane left this section, or ran out of sections. It left only if it saw every lane arrive
						// here, which it recorded before it posted anything later.
						if block.completed.load(Ordering::Acquire) != section.index {
							self.mismatch(section);
						}
						break;
					}
					if posted_section == section.index && posted != working {
						// The lane is in this section, but not as this lane is.
						self.mismatch(section);
					}
				}
				self.pause(&mut waits);
			}
		}
	}

	/// Returns the values every lane gave `section`, which has ended.
	fn values<T>(&self, section: &Section) -> LaneValues<'dispatch, T> {
		let first = if section.valued {
			self.sections.lanes[0].slots[section.slot].value_ptr()
		} else {
			NonNull::<u8>::dangling().as_ptr().cast_const()
		};
		LaneValues::new(first, self.lane_count)
	}

	/// Waits briefly for other lanes, and fails if a lane failed the dispatch.
	///
	/// Sections of one dispatch follow each other within microseconds, so a lane spins at first, and only yields its
	/// thread once it has waited long.
	fn pause(&self, waits: &mut u32) {
		self.check_failure();
		if *waits < SECTION_SPINS {
			*waits += 1;
			spin_loop();
		} else {
			yield_now();
		}
	}

	/// Runs `f`, and fails the dispatch for every lane if it panics, before the panic leaves the section.
	fn guarded<T>(&self, f: impl FnOnce() -> T) -> T {
		let guard = FailOnUnwind(self);
		let value = f();
		std::mem::forget(guard);
		value
	}

	/// Panics if a lane failed the dispatch.
	fn check_failure(&self) {
		if self.sections.failed(self.first_section) {
			panic!(
				"Collective failed. The most likely cause is a lane of this dispatch that panicked or ran a different collective operation."
			);
		}
	}

	/// Fails the dispatch for every lane, as this lane's doing.
	fn fail(&self) {
		self.sections.fail(self.first_section, self.lane_idx);
	}

	/// Fails the dispatch for every lane with what this lane panicked with.
	#[allow(unsafe_code, reason = "A lane writes its own panic while it runs.")]
	fn fail_with(&self, panic: Box<dyn Any + Send>) {
		// SAFETY: Only this lane writes its panic, and the caller reads it once every lane finished.
		unsafe { *self.sections.lanes[self.lane_idx].panic.get() = Some(panic) };
		self.fail();
	}

	/// Posts that this lane ran its last section, so that a lane that waits for it in a later one fails instead of
	/// waiting forever.
	fn finish(&self) {
		self.sections.lanes[self.lane_idx].finish(self.next_section);
	}

	#[cold]
	#[inline(never)]
	fn mismatch(&self, section: &Section) -> ! {
		self.fail();
		panic!(
			"Collective operation mismatch at section {}. The most likely cause is that lanes took different branches, so they did not all run the same operation, with or without a value and with the same key, in the same order.",
			section.index - self.first_section
		)
	}

	#[cold]
	#[inline(never)]
	fn type_mismatch(&self, section: &Section) -> ! {
		self.fail();
		panic!(
			"Collective type mismatch at section {}. The most likely cause is lanes that return values of different types from the same section.",
			section.index - self.first_section
		)
	}
}

/// The `FailOnUnwind` struct fails a dispatch when lane work that it guards panics, while the panic leaves the
/// section, so that no lane waits for the panicking one and no lane goes on if the panic is caught.
struct FailOnUnwind<'lane, 'dispatch>(&'lane ConcreteLane<'dispatch>);

impl Drop for FailOnUnwind<'_, '_> {
	fn drop(&mut self) {
		self.0.fail();
	}
}

/// The `Stage` struct proves that a lane is in a stage that every lane of its dispatch runs with the same key.
///
/// [`ConcreteLane::each_ref_keyed`] makes one for the work it runs. It cannot be kept past that work, so whatever
/// borrows it ends with the stage.
pub struct Stage<'lane, 'dispatch> {
	lane: &'lane ConcreteLane<'dispatch>,
	key: u32,
}

impl<'lane, 'dispatch> Stage<'lane, 'dispatch> {
	/// Returns the lane that runs the stage.
	pub fn lane(&self) -> &'lane ConcreteLane<'dispatch> {
		self.lane
	}

	/// Returns the key every lane runs the stage with.
	pub fn key(&self) -> u32 {
		self.key
	}
}

/// The `SharedLane` struct provides lane-local access to one exclusive partition of shared mutable data.
pub struct SharedLane<'dispatch, 'values, T> {
	lane: &'values mut ConcreteLane<'dispatch>,
	values: &'values mut [T],
}

impl<'dispatch, 'values, T> SharedLane<'dispatch, 'values, T> {
	fn new(lane: &'values mut ConcreteLane<'dispatch>, values: &'values mut [T]) -> Self {
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
		self.lane
	}
}

impl<T> DerefMut for SharedLane<'_, '_, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.lane
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
	start..start + minimum_len + usize::from(lane_idx < larger_partition_count)
}

/// The most collective sections of one dispatch that can return a value with a size.
///
/// A dispatch keeps those values until it ends, so it has storage for a fixed number of them. Sections that return
/// a value without size, such as `()`, are not counted.
const COLLECTIVE_SECTION_COUNT: usize = 32;
const COLLECTIVE_VALUE_SIZE: usize = 64;
const COLLECTIVE_VALUE_ALIGNMENT: usize = 64;

/// How many times a lane checks for the lanes it waits on in a section before it starts to yield its thread, about
/// ten microseconds. Lanes of one dispatch run side by side, so the lane they wait for usually arrives within that,
/// and a lane that waits longer lets other threads use its core.
const SECTION_SPINS: u32 = 1 << 10;

/// The `Section` struct names one lane's turn at a collective section.
struct Section {
	/// The section's number, counted across the alley's dispatches.
	index: u64,
	/// Whether the section's value has a size, so that lanes keep it in a slot.
	valued: bool,
	/// The slot the section keeps its values in. For a section without values, the slot that one with values would
	/// take, which is [`COLLECTIVE_SECTION_COUNT`] when none is left.
	slot: usize,
}

/// The `Sections` struct holds where the lanes of an [`Alley`] dispatch meet in collective sections.
///
/// No lane leads, so a lane waits only for what a section needs: every lane at the end of an `each` section, and
/// the lane that runs a broadcast. Every lane posts its arrival at an `each` section to its own [`LaneBlock`] and
/// reads every other lane's, so no two lanes write one cache line there. The first lane to reach a broadcast or a
/// claim takes it by raising [`Self::turn`], the only read-modify-write of shared memory a section makes, and a
/// lane that runs a broadcast with a value keeps it in [`Self::values`].
///
/// A broadcast with exclusive access to the dispatch's resource must never run while a lane reaches the resource in
/// an `each` section. A lane that enters such a section posts it and only then reads the turn, and a lane that takes
/// a broadcast with exclusive access raises the turn and only then reads every lane's post, each side with
/// sequentially consistent order between its write and its reads. So of two such lanes, at least one sees the other
/// and fails the dispatch before it reaches the resource. Lanes that enter such a section with different keys find
/// each other the same way, through each other's posts. A lane never waits for another lane to enter a section.
struct Sections {
	/// The broadcast or claim that a lane took last. It numbers sections within the dispatch, which clears it before
	/// it starts.
	turn: TurnLine,
	failure: FailureLine,
	/// The values of the dispatch's broadcasts, one slot for each section with values.
	values: Box<[Slot; COLLECTIVE_SECTION_COUNT]>,
	/// One block for each lane of the alley, each lane's after the one before it.
	lanes: Box<[LaneBlock]>,
}

impl Sections {
	fn new(lanes: usize) -> Self {
		Self {
			turn: TurnLine(AtomicU64::new(Turn::NONE.0)),
			failure: FailureLine {
				dispatch: AtomicU64::new(0),
				lane: AtomicUsize::new(0),
			},
			values: Box::new(std::array::from_fn(|_| Slot::new())),
			lanes: (0..lanes).map(|_| LaneBlock::new()).collect(),
		}
	}

	/// Clears the turn for a dispatch whose lanes have not started.
	fn start(&self) {
		self.turn.0.store(Turn::NONE.0, Ordering::Relaxed);
	}

	/// Fails the dispatch whose first section is `first_section`, as `lane`'s doing unless another lane failed it
	/// first.
	fn fail(&self, first_section: u64, lane: usize) {
		if self.failure.dispatch.fetch_max(first_section, Ordering::AcqRel) < first_section {
			self.failure.lane.store(lane, Ordering::Relaxed);
		}
	}

	/// Returns whether a lane failed the dispatch whose first section is `first_section`.
	fn failed(&self, first_section: u64) -> bool {
		self.failure.dispatch.load(Ordering::Relaxed) == first_section
	}

	/// Returns the lane that failed the dispatch whose first section is `first_section` first. Call it once every
	/// lane of the dispatch finished.
	fn failed_lane(&self, first_section: u64) -> Option<usize> {
		self.failed(first_section).then(|| self.failure.lane.load(Ordering::Relaxed))
	}

	/// Returns a section number past every one that the first `lane_count` lanes of the dispatch whose first section is
	/// `first_section` posted or kept, so that the next dispatch starts after them. Call it once every lane finished.
	fn after(&self, first_section: u64, lane_count: usize) -> u64 {
		// Every lane finishes with a post of the section after its last one.
		let finished = |block: &LaneBlock| LaneBlock::posted(block.post.load(Ordering::Relaxed)).0;
		// The lanes of a dispatch that did not fail all reached every `each` section, so the first lane's post is past
		// those. Lanes pass broadcasts and claims without each other, and the turn names the last of them.
		let last = if self.failed(first_section) {
			self.lanes[..lane_count].iter().map(finished).max().unwrap_or(first_section)
		} else {
			finished(&self.lanes[0])
		};
		let turn = Turn(self.turn.0.load(Ordering::Relaxed));
		// A finish post names the section after the lane's last, so the next dispatch starts one later still.
		last.max(first_section + turn.section()) + 1
	}
}

/// The `TurnLine` struct holds the [`Turn`] of a dispatch on cache lines of its own.
#[repr(align(128))]
struct TurnLine(AtomicU64);

/// The `Turn` struct packs the broadcast or claim that a lane of a dispatch took last: the section's number within
/// the dispatch, counted from one, whether it is a claim, whether it ended, whether it keeps a value, and how many
/// lanes took a claim. A lane takes a section by raising the turn to it, so the turn's section never goes back
/// within a dispatch.
#[derive(Clone, Copy)]
struct Turn(u64);

impl Turn {
	const CLAIM: u64 = 1;
	const ENDED: u64 = 1 << 1;
	const VALUED: u64 = 1 << 2;
	const COUNT_SHIFT: u32 = 4;
	const SECTION_SHIFT: u32 = 20;
	/// The most lanes that take one claim.
	const MOST_CLAIMS: usize = (1 << (Self::SECTION_SHIFT - Self::COUNT_SHIFT)) - 1;
	/// What a dispatch starts with: no section, and none to wait for.
	const NONE: Self = Self(Self::ENDED);

	/// The broadcast `section`, which a lane runs.
	fn broadcast(section: u64, valued: bool) -> Self {
		Self(section << Self::SECTION_SHIFT | if valued { Self::VALUED } else { 0 })
	}

	/// The claim `section`, which one lane took. A claim has nothing to wait for, so it has ended.
	fn claim(section: u64) -> Self {
		Self(section << Self::SECTION_SHIFT | 1 << Self::COUNT_SHIFT | Self::ENDED | Self::CLAIM)
	}

	/// The same broadcast, once its lane ran it.
	fn ended(self) -> Self {
		Self(self.0 | Self::ENDED)
	}

	/// The same claim, which one more lane took.
	fn taken_again(self) -> Self {
		Self(self.0 + (1 << Self::COUNT_SHIFT))
	}

	fn section(self) -> u64 {
		self.0 >> Self::SECTION_SHIFT
	}

	fn is_claim(self) -> bool {
		self.0 & Self::CLAIM != 0
	}

	fn has_ended(self) -> bool {
		self.0 & Self::ENDED != 0
	}

	fn valued(self) -> bool {
		self.0 & Self::VALUED != 0
	}

	fn count(self) -> usize {
		(self.0 >> Self::COUNT_SHIFT) as usize & Self::MOST_CLAIMS
	}
}

/// The `FailureLine` struct records the dispatch that a lane failed last, on cache lines of its own.
#[repr(align(128))]
struct FailureLine {
	/// The first section of the dispatch that failed last.
	dispatch: AtomicU64,
	/// The lane that failed it first, whose panic the dispatch returns.
	lane: AtomicUsize,
}

/// The `LaneBlock` struct holds what one lane posts and keeps during a dispatch, on cache lines that only that lane
/// writes.
#[repr(C, align(128))]
struct LaneBlock {
	/// The lane's last post: the `each` section it entered or arrived at and how it runs it, or that it finished.
	post: AtomicU64,
	/// The key of the last section the lane entered to reach the resource. Other lanes read it once they saw that
	/// entry posted.
	key: AtomicU32,
	/// The last `each` section the lane left, which it left once it saw every other lane arrive there.
	completed: AtomicU64,
	/// What the lane panicked with in the last dispatch.
	panic: UnsafeCell<Option<Box<dyn Any + Send>>>,
	/// The values the lane gave the dispatch's `each` sections, one slot for each section with values.
	slots: [Slot; COLLECTIVE_SECTION_COUNT],
}

impl LaneBlock {
	const FINISHED: u64 = 1;
	const VALUED: u64 = 1 << 1;
	const SHARED: u64 = 1 << 2;
	const ARRIVED: u64 = 1 << 3;
	const SECTION_SHIFT: u32 = 4;

	fn new() -> Self {
		Self {
			post: AtomicU64::new(0),
			key: AtomicU32::new(0),
			completed: AtomicU64::new(0),
			panic: UnsafeCell::new(None),
			slots: std::array::from_fn(|_| Slot::new()),
		}
	}

	/// Returns what the lane posts when it enters `section` to reach the resource, a section that keeps values when
	/// `valued`.
	fn entered(section: u64, valued: bool) -> u64 {
		section << Self::SECTION_SHIFT | Self::SHARED | if valued { Self::VALUED } else { 0 }
	}

	/// Returns what the lane posts when it arrives at `section`, a section that reaches the resource when `shared`
	/// and keeps values when `valued`.
	fn arrived_at(section: u64, shared: bool, valued: bool) -> u64 {
		let shared = if shared { Self::SHARED } else { 0 };
		section << Self::SECTION_SHIFT | Self::ARRIVED | shared | if valued { Self::VALUED } else { 0 }
	}

	/// Returns the section a lane's post names, and whether the post says that the lane finished, before that section.
	fn posted(posted: u64) -> (u64, bool) {
		(posted >> Self::SECTION_SHIFT, posted & Self::FINISHED != 0)
	}

	/// Returns whether a lane whose post is `posted` reaches the resource in an `each` section of the dispatch whose
	/// first section is `first_section`, other than one that it left before `section`.
	fn reaches(posted: u64, first_section: u64, section: u64) -> bool {
		let (posted_section, _) = Self::posted(posted);
		posted_section >= first_section
			&& posted & Self::SHARED != 0
			&& (posted & Self::ARRIVED == 0 || posted_section >= section)
	}

	/// Posts `entered`, that the lane enters a section to reach the resource with `key`, before any other lane can
	/// read anything the lane reads afterwards.
	fn enter(&self, entered: u64, key: u32) {
		// Lanes read the key only after they saw the post below, which publishes it.
		self.key.store(key, Ordering::Relaxed);
		self.post.store(entered, Ordering::SeqCst);
	}

	/// Posts that the lane arrived at `section`, which publishes its value.
	fn arrive(&self, section: u64, shared: bool, valued: bool) {
		self.post.store(Self::arrived_at(section, shared, valued), Ordering::Release);
	}

	/// Records that the lane left the `each` section `section`, once it saw every other lane arrive there.
	fn complete(&self, section: u64) {
		self.completed.store(section, Ordering::Release);
	}

	/// Posts that the lane finished, before section `next_section`.
	fn finish(&self, next_section: u64) {
		self.post
			.store(next_section << Self::SECTION_SHIFT | Self::FINISHED, Ordering::Release);
	}

	/// Returns whether the lane panicked in the last dispatch.
	///
	/// # Safety
	///
	/// The lane must have finished.
	#[allow(unsafe_code, reason = "Reads a lane's panic once it finished.")]
	unsafe fn panicked(&self) -> bool {
		// SAFETY: The caller guarantees that the lane finished, so it writes its panic no more.
		unsafe { (*self.panic.get()).is_some() }
	}

	/// Takes what the lane panicked with in the last dispatch.
	///
	/// # Safety
	///
	/// The lane must have finished.
	#[allow(unsafe_code, reason = "Takes a lane's panic once it finished.")]
	unsafe fn take_panic(&self) -> Option<Box<dyn Any + Send>> {
		// SAFETY: The caller guarantees that the lane finished, so it writes its panic no more.
		unsafe { (*self.panic.get()).take() }
	}
}

// SAFETY: A lane writes the slots and the panic of its own block, and of no other, while it runs. Other lanes read a
// slot only after the lane posted it with release ordering, which they observe with acquire ordering, and a lane
// writes each of its slots once in a dispatch. The caller reads panics once every lane finished.
#[allow(unsafe_code, reason = "Section posts order every access to a lane's block.")]
unsafe impl Sync for LaneBlock {}

/// The `Slot` struct keeps the value of one collective section until the dispatch ends: a lane's value for an
/// `each` section, or the value of a broadcast.
#[repr(C, align(128))]
struct Slot {
	/// The broadcast section whose value the slot holds. Only broadcasts record it.
	section: AtomicU64,
	/// The type of the value.
	value_type: UnsafeCell<MaybeUninit<TypeId>>,
	value: UnsafeCell<CollectiveStorage>,
}

// SAFETY: One lane writes a slot once in a dispatch, and other lanes read it only after a post with release ordering
// that they observe with acquire ordering: a lane's arrival for its own slots, and the turn or the slot's section
// for a broadcast's.
#[allow(unsafe_code, reason = "Section posts order every access to a slot.")]
unsafe impl Sync for Slot {}

impl Slot {
	fn new() -> Self {
		Self {
			section: AtomicU64::new(0),
			value_type: UnsafeCell::new(MaybeUninit::uninit()),
			value: UnsafeCell::new(CollectiveStorage::new()),
		}
	}

	/// Keeps `value` as the lane's value for its section, without its type.
	///
	/// # Safety
	///
	/// `T` must fit the slot in size and alignment, and no other lane may read or write the slot during the call.
	#[allow(unsafe_code, reason = "Collective values occupy cache-line-sized inline slots.")]
	unsafe fn write<T>(&self, value: T) {
		// SAFETY: The caller checked that a `T` fits, and keeps other lanes away.
		unsafe { self.value.get().cast::<T>().write(value) };
	}

	/// Keeps `value` with its type, as a lane's value for an `each` section.
	///
	/// # Safety
	///
	/// As for [`Self::write`].
	#[allow(unsafe_code, reason = "Collective values occupy cache-line-sized inline slots.")]
	unsafe fn put<T: 'static>(&self, value: T) {
		// SAFETY: The caller keeps other lanes away.
		unsafe { self.value_type.get().write(MaybeUninit::new(TypeId::of::<T>())) };
		// SAFETY: The caller checked that a `T` fits, and keeps other lanes away.
		unsafe { self.write(value) };
	}

	/// Keeps `value` with its type as the value of the broadcast `section`.
	///
	/// # Safety
	///
	/// As for [`Self::write`].
	#[allow(unsafe_code, reason = "Collective values occupy cache-line-sized inline slots.")]
	unsafe fn keep<T: 'static>(&self, section: u64, value: T) {
		// SAFETY: The caller checked that a `T` fits, and keeps other lanes away.
		unsafe { self.put(value) };
		// Publishes the type and the value to a lane that finds the section here.
		self.section.store(section, Ordering::Release);
	}

	/// Returns whether the slot holds the value of the broadcast `section`.
	fn holds(&self, section: u64) -> bool {
		self.section.load(Ordering::Acquire) == section
	}

	/// Returns the type of the value the slot holds.
	///
	/// # Safety
	///
	/// A lane must have kept a value in the slot with its type, which the caller saw, and must not be keeping another.
	#[allow(unsafe_code, reason = "Reads the type a lane kept with its value.")]
	unsafe fn value_type(&self) -> TypeId {
		// SAFETY: The caller guarantees that the type was written and that nothing writes it now.
		unsafe { self.value_type.get().read().assume_init() }
	}

	/// Returns a copy of the value the slot holds.
	///
	/// # Safety
	///
	/// A lane must have written a `T` to the slot, which the caller saw, and must not be writing it.
	#[allow(unsafe_code, reason = "Reads a value a lane published.")]
	unsafe fn read<T: Copy>(&self) -> T {
		// SAFETY: The caller guarantees that a `T` was written and that nothing writes it now.
		unsafe { self.value.get().cast::<T>().read() }
	}

	fn value_ptr(&self) -> *const u8 {
		self.value.get().cast_const().cast()
	}
}

#[repr(C, align(64))]
struct CollectiveStorage {
	bytes: [MaybeUninit<u8>; COLLECTIVE_VALUE_SIZE],
}

impl CollectiveStorage {
	fn new() -> Self {
		Self {
			bytes: [MaybeUninit::uninit(); COLLECTIVE_VALUE_SIZE],
		}
	}
}

/// The `LaneParts` struct lets every lane of a dispatch reach its own part of one array, such as its entry of the
/// results or its partition of the values.
struct LaneParts<T>(*mut T);

impl<T> LaneParts<T> {
	/// Writes `value` as entry `index`.
	///
	/// # Safety
	///
	/// The array must have room for the entry, and no other lane may reach it.
	#[allow(unsafe_code, reason = "Each lane writes only its own entry.")]
	unsafe fn write(&self, index: usize, value: T) {
		// SAFETY: The caller guarantees that the entry is within the array and that only this lane reaches it.
		unsafe { self.0.add(index).write(value) };
	}

	/// Drops entry `index`.
	///
	/// # Safety
	///
	/// The entry must hold a value that nothing else owns.
	#[allow(unsafe_code, reason = "Drops a result that a lane wrote.")]
	unsafe fn drop_entry(&self, index: usize) {
		// SAFETY: The caller guarantees that the entry holds a value that nothing else owns.
		unsafe { self.0.add(index).drop_in_place() };
	}

	/// Returns the entries in `range`.
	///
	/// # Safety
	///
	/// The range must lie within the array, which must be borrowed mutably for the returned lifetime, and no other
	/// lane may reach the entries.
	#[allow(unsafe_code, reason = "Each lane reaches only its own range.")]
	unsafe fn slice<'a>(&self, range: Range<usize>) -> &'a mut [T] {
		// SAFETY: The caller guarantees that the range is within the array and that only this lane reaches it.
		unsafe { std::slice::from_raw_parts_mut(self.0.add(range.start), range.len()) }
	}
}

// SAFETY: Each lane reaches only its own entries, and the dispatch hands them back only once every lane finished.
// Entries move to other threads that way, so they have to be `Send`.
#[allow(unsafe_code, reason = "Each lane reaches only its own entries.")]
unsafe impl<T: Send> Sync for LaneParts<T> {}

/// The `TakeOnce` struct hands a value between the caller and the one worker of a dispatch.
struct TakeOnce<T>(UnsafeCell<Option<T>>);

impl<T> TakeOnce<T> {
	fn new(value: Option<T>) -> Self {
		Self(UnsafeCell::new(value))
	}

	/// Takes the value.
	///
	/// # Safety
	///
	/// No other thread may reach the value during the call.
	#[allow(unsafe_code, reason = "One worker takes the value.")]
	unsafe fn take(&self) -> Option<T> {
		// SAFETY: The caller keeps other threads away.
		unsafe { (*self.0.get()).take() }
	}

	/// Puts `value` in place of the value.
	///
	/// # Safety
	///
	/// No other thread may reach the value during the call.
	#[allow(unsafe_code, reason = "One worker puts the value.")]
	unsafe fn put(&self, value: T) {
		// SAFETY: The caller keeps other threads away.
		unsafe { *self.0.get() = Some(value) };
	}

	fn into_inner(self) -> Option<T> {
		self.0.into_inner()
	}
}

// SAFETY: Only one worker reaches the value while the dispatch runs, and the caller only before and after it.
#[allow(unsafe_code, reason = "One worker at a time reaches the value.")]
unsafe impl<T: Send> Sync for TakeOnce<T> {}

impl Alley {
	/// Creates an alley with one lane for each available hardware thread.
	///
	/// Call [`Self::execute`] or [`Self::execute_shared_mut`] to dispatch work.
	pub fn new() -> Self {
		Self::with_pool(LanePool::new())
	}

	/// Creates an alley with `count` persistent worker lanes.
	///
	/// Call [`Self::execute`] or [`Self::execute_shared_mut`] to dispatch work.
	pub fn with_parallelism(count: usize) -> Self {
		Self::with_pool(LanePool::with_parallelism(count))
	}

	fn with_pool(pool: LanePool) -> Self {
		Self {
			sections: Sections::new(pool.parallelism()),
			pool,
			id: AlleyId::next(),
			next_section: 1,
		}
	}

	/// Returns the number of lanes every full dispatch runs.
	///
	/// Use it to size per-lane storage before a dispatch indexes it by [`Lane::idx`].
	pub fn parallelism(&self) -> usize {
		self.pool.parallelism()
	}

	/// Returns what tells this alley's lanes from those of any other.
	///
	/// Pass it to [`LaneOwned::lend`] or [`LaneRanges::lend`] before a dispatch, so that this alley's lanes can write
	/// their own parts of that storage.
	pub fn id(&self) -> AlleyId {
		self.id
	}

	/// Runs worker work alongside work confined to the calling thread, then returns both results.
	///
	/// Neither caller work nor its result needs to be `Send`. Both jobs finish before a captured panic
	/// is returned, so their borrowed inputs remain valid. The alley can then run another [`Self::join`]
	/// or a collective [`Self::execute`] dispatch.
	#[allow(unsafe_code, reason = "The one worker of the dispatch takes the job and puts its result.")]
	pub fn join<R: Send, C>(
		&mut self,
		worker: impl FnOnce() -> R + Send,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<(R, C)> {
		let job = TakeOnce::new(Some(worker));
		let result = TakeOnce::new(None);
		let caller = self.pool.run(
			0..1,
			&|_| {
				// SAFETY: Only the first worker runs this job, once.
				let worker = unsafe { job.take() }.expect("Alley join failed. The worker job ran twice.");
				let value = worker();
				// SAFETY: Only this worker reaches the result while it runs, and the caller reads it once it finished.
				unsafe { result.put(value) };
			},
			caller,
		)?;
		let worker = result
			.into_inner()
			.expect("Alley join failed. The worker finished without a result.");
		Ok((worker, caller))
	}

	/// Executes `f` once on every lane and returns after all lanes finish.
	///
	/// The calling thread runs the first lane, so a dispatch wakes one worker fewer than it has lanes. Mutable access
	/// prevents overlapping or nested dispatches on this `Alley`. Gang-scheduled lane collectives require exclusive
	/// access to its worker set and could otherwise deadlock. On success, the lane-ordered vector contains exactly
	/// one value per alley lane, and a vector of values without size allocates nothing. Returns the first captured
	/// panic if any lane panics. The alley remains available for later dispatches.
	pub fn execute<R>(&mut self, f: impl AlleyFunction<R>) -> std::thread::Result<Vec<R>>
	where
		R: Send,
	{
		let lanes = self.parallelism();
		self.dispatch(lanes, true, f, || ()).map(|(values, ())| values)
	}

	/// Executes `f` once on every lane alongside work confined to the calling thread, then returns both results.
	///
	/// Lanes follow the rules of [`Self::execute`], except that every lane runs on a worker. The caller and its result
	/// stay on this thread and need not be `Send`, as with [`Self::join`]. Every lane and the caller finish before a
	/// captured panic is returned, so their borrowed inputs remain valid. Use this to overlap lane work with a caller
	/// that owns thread-bound state; without one, [`Self::execute`] lets the calling thread run a lane instead.
	pub fn execute_with_caller<R, C>(
		&mut self,
		f: impl AlleyFunction<R>,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<(Vec<R>, C)>
	where
		R: Send,
	{
		let lanes = self.parallelism();
		self.dispatch(lanes, false, f, caller)
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
		F: for<'dispatch, 'lane, 'resources> Fn(&'lane mut ConcreteLane<'dispatch>, &'resources Resources::Bound) -> R + Sync,
	{
		let parallelism = self.parallelism();
		let mut next_index = 0;
		let resources = resources.bind(parallelism, &mut next_index, lane_resources_private::BindToken);
		let resources = &resources;
		self.execute(move |lane| f(lane, resources))
	}

	/// Executes up to `lanes` lanes with one resource that they access in alternating exclusive and shared phases.
	///
	/// Use this to run a whole algorithm as one lane function: [`ConcreteLane::broadcast_mut`] runs a serial stage
	/// on the first lane to reach it while the others wait, and [`ConcreteLane::each_ref`] runs a parallel stage on
	/// every running lane. The blocking dispatch keeps the mutable borrow bound until all lanes finish.
	///
	/// Pass [`Self::parallelism`] to run every lane. Pass fewer when the work is too small to pay for all of them:
	/// every stage ends only once each running lane has reached it, so lanes with little to do slow a small job
	/// down. The lanes left out stay asleep, [`ConcreteLane::share`] splits work among the running lanes only, and
	/// the result holds one value for each of them. The calling thread runs the first lane, so a count of one runs
	/// `f` on the calling thread alone. A count above [`Self::parallelism`] runs every lane, and a count of zero runs
	/// none, so `f` is never called.
	pub fn execute_phased<T, R, F>(&mut self, resource: &mut T, lanes: usize, f: F) -> std::thread::Result<Vec<R>>
	where
		T: Send + Sync,
		R: Send,
		F: for<'dispatch, 'lane, 'resource, 'value> Fn(
				&'lane mut ConcreteLane<'dispatch>,
				&'resource LanePhased<'value, T>,
			) -> R
			+ Sync,
	{
		let resource = LanePhased {
			value: UnsafeCell::new(resource),
		};
		let resource = &resource;
		let lanes = lanes.min(self.parallelism());
		self.dispatch(lanes, true, move |lane| f(lane, resource), || ())
			.map(|(values, ())| values)
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
		F: for<'dispatch> Fn(&mut ConcreteLane<'dispatch>, &Resources::Bound) + Sync,
	{
		let parallelism = self.parallelism();
		let mut resource_count = 0;
		let resources = resources.bind(parallelism, &mut resource_count, lane_resources_private::BindToken);
		let resources = &resources;
		// Lanes without resources have no work in this dispatch. Keep them asleep.
		let active_lanes = resource_count.min(parallelism);
		self.dispatch(active_lanes, false, move |lane| f(lane, resources), caller)
			.map(|(_, caller)| caller)
	}

	/// Executes every lane with exclusive access to a balanced mutable partition.
	///
	/// Mutable access prevents overlapping or nested dispatches on this `Alley`. Gang-scheduled
	/// lane collectives require exclusive access to its worker set and could otherwise deadlock.
	#[allow(unsafe_code, reason = "Each lane borrows its own balanced range of the values.")]
	pub fn execute_shared_mut<T, F>(&mut self, values: &mut [T], f: F)
	where
		T: Send,
		F: for<'dispatch, 'values> Fn(&mut SharedLane<'dispatch, 'values, T>) + Sync,
	{
		let lane_count = self.parallelism();
		let len = values.len();
		let parts = LaneParts(values.as_mut_ptr());
		// Every lane runs, including lanes whose partition is empty when there are fewer values than lanes, so that
		// every lane reaches the collectives.
		let executed = self.dispatch(
			lane_count,
			true,
			|lane| {
				// SAFETY: Balanced ranges of different lanes do not overlap and stay within `values`, which this
				// dispatch borrows mutably until every lane finished.
				let partition = unsafe { parts.slice(shared_range(len, lane.idx(), lane_count)) };
				f(&mut SharedLane::new(lane, partition));
			},
			|| (),
		);
		if let Err(panic) = executed {
			resume_unwind(panic);
		}
	}

	/// Distributes `items` evenly over up to `lanes` lanes and gives each lane exclusive access to one partition.
	///
	/// Pass [`Self::parallelism`] to run every lane, or fewer when the items are too few to pay for waking them all.
	/// At most one lane runs for each item. The calling thread runs the first lane, so a count of one runs `f` on
	/// the calling thread alone, and a count of zero runs none, so `f` is never called. Mutable access prevents
	/// overlapping or nested dispatches on this `Alley`. Gang-scheduled lane collectives require exclusive access to
	/// its worker set and could otherwise deadlock.
	#[allow(unsafe_code, reason = "Each lane borrows its own balanced range of the items.")]
	pub fn for_each_mut<T, F>(&mut self, items: &mut [T], lanes: usize, f: F)
	where
		T: Send,
		F: for<'dispatch> Fn(&mut ConcreteLane<'dispatch>, &mut [T]) + Sync,
	{
		// Lanes without items have no work, so only as many lanes run as there are items.
		let lane_count = self.parallelism().min(items.len()).min(lanes);
		let len = items.len();
		let parts = LaneParts(items.as_mut_ptr());
		let executed = self.dispatch(
			lane_count,
			true,
			|lane| {
				// SAFETY: Balanced ranges of different lanes do not overlap and stay within `items`, which this
				// dispatch borrows mutably until every lane finished.
				let chunk = unsafe { parts.slice(shared_range(len, lane.idx(), lane_count)) };
				f(lane, chunk);
			},
			|| (),
		);
		if let Err(panic) = executed {
			resume_unwind(panic);
		}
	}

	/// Runs `f` on lanes `0..lane_count` alongside `caller` on the calling thread, and returns their results once all
	/// of them finished.
	///
	/// With `caller_leads`, the calling thread runs the first lane before `caller`, so the dispatch wakes one worker
	/// fewer. Otherwise every lane runs on a worker, so that `caller` runs alongside all of them. The lane that failed
	/// the dispatch first says why it failed, so its panic is returned before the others and before the caller's.
	#[allow(unsafe_code, reason = "Lanes write their results to the vector's spare capacity.")]
	fn dispatch<R, C>(
		&mut self,
		lane_count: usize,
		caller_leads: bool,
		f: impl AlleyFunction<R>,
		caller: impl FnOnce() -> C,
	) -> std::thread::Result<(Vec<R>, C)>
	where
		R: Send,
	{
		if lane_count == 0 {
			return catch_unwind(AssertUnwindSafe(caller)).map(|caller| (Vec::new(), caller));
		}
		let Self {
			pool,
			id,
			sections,
			next_section,
		} = self;
		let (alley, sections, first_section) = (*id, &*sections, *next_section);
		if lane_count == 1 && caller_leads {
			// One lane meets no other, so it runs here without touching the workers or posting anything.
			let mut lane = ConcreteLane::new(0, 1, alley, sections, first_section);
			let value = catch_unwind(AssertUnwindSafe(|| f(&mut lane)));
			// The next dispatch numbers its sections after this one's, so that a failure of this one stays here.
			*next_section = lane.next_section + 1;
			let caller = catch_unwind(AssertUnwindSafe(caller));
			return value.and_then(|value| caller.map(|caller| (vec![value], caller)));
		}
		// Results without size need no storage, so their vector allocates nothing.
		let mut values = Vec::<R>::with_capacity(lane_count);
		let results = LaneParts(values.as_mut_ptr());
		let lane = |lane_idx: usize| {
			let mut lane = ConcreteLane::new(lane_idx, lane_count, alley, sections, first_section);
			match catch_unwind(AssertUnwindSafe(|| f(&mut lane))) {
				// SAFETY: The vector has room for every lane, and each lane writes only its own entry.
				Ok(value) => unsafe { results.write(lane_idx, value) },
				Err(panic) => lane.fail_with(panic),
			}
			lane.finish();
		};
		// The turn of the last dispatch numbered its sections within it, so a lane would take it for one of this one's.
		sections.start();
		let caller = pool.run(usize::from(caller_leads)..lane_count, &lane, || {
			if caller_leads {
				lane(0);
			}
			caller()
		});
		*next_section = sections.after(first_section, lane_count);

		// SAFETY: Every lane finished.
		let panicked = sections.lanes[..lane_count].iter().any(|block| unsafe { block.panicked() });
		if !panicked {
			// SAFETY: Every lane finished without a panic, so each wrote its result.
			unsafe { values.set_len(lane_count) };
			return caller.map(|caller| (values, caller));
		}
		let failed = sections.failed_lane(first_section);
		let mut panic = None;
		for (lane_idx, block) in sections.lanes[..lane_count].iter().enumerate() {
			// SAFETY: Every lane finished.
			match unsafe { block.take_panic() } {
				Some(payload) if panic.is_none() || failed == Some(lane_idx) => panic = Some(payload),
				Some(_) => {}
				// SAFETY: A lane that finished without a panic wrote its result, which the vector does not own yet.
				None => unsafe { results.drop_entry(lane_idx) },
			}
		}
		Err(panic.expect("Alley dispatch failed. A lane panicked without leaving its panic."))
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
		panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
		sync::{
			Barrier,
			atomic::{AtomicBool, AtomicUsize, Ordering},
		},
		thread::sleep,
		time::{Duration, Instant},
	};

	use crate::core::alley::{Alley, Lane};

	fn unwrap_dispatch<R>(result: std::thread::Result<Vec<R>>) -> Vec<R> {
		result.unwrap_or_else(|payload| resume_unwind(payload))
	}

	/// Waits until `done` holds, for five seconds at most, so that a test of lanes that must not wait for each other
	/// fails instead of hanging when they do.
	fn wait_until(done: impl Fn() -> bool) {
		let deadline = Instant::now() + Duration::from_secs(5);
		while !done() && Instant::now() < deadline {
			std::thread::yield_now();
		}
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
	fn execute_runs_the_first_lane_on_the_calling_thread() {
		let mut alley = Alley::with_parallelism(3);
		let caller = std::thread::current().id();

		let threads = unwrap_dispatch(alley.execute(|_| std::thread::current().id()));

		assert_eq!(threads[0], caller);
		assert!(threads[1..].iter().all(|thread| *thread != caller));
		assert_ne!(threads[1], threads[2]);
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
	fn a_failed_dispatch_returns_the_panic_of_the_lane_that_failed_it() {
		let mut alley = Alley::with_parallelism(4);

		let result = alley.execute(|lane| {
			if lane.idx() == 2 {
				panic!("expected lane panic");
			}
			// The other lanes wait for the panicking one here, and stop once it failed the dispatch.
			let _ = lane.each(|| ());
		});

		let message = crate::core::alley::split::tests::message(result.unwrap_err());
		assert_eq!(message, "expected lane panic");
	}

	#[test]
	fn each_partition_receives_its_lane_index() {
		let mut alley = Alley::with_parallelism(3);
		let mut counters = [0; 10];

		alley.for_each_mut(&mut counters, 3, |lane, partition| {
			partition.fill(lane.idx());
		});

		assert_eq!(counters, [0, 0, 0, 0, 1, 1, 1, 2, 2, 2]);
	}

	#[test]
	fn a_lane_count_splits_the_items_among_that_many_lanes() {
		let mut alley = Alley::with_parallelism(3);
		let mut counters = [usize::MAX; 10];

		alley.for_each_mut(&mut counters, 2, |lane, partition| {
			partition.fill(lane.idx());
		});

		assert_eq!(counters, [0, 0, 0, 0, 0, 1, 1, 1, 1, 1]);
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
	fn limited_parallelism_runs_on_as_many_lanes_as_its_limit() {
		let mut alley = Alley::with_parallelism(8);
		let limits = [3, 5, 0, usize::MAX];
		let runs = limits.map(|_| AtomicUsize::new(0));

		unwrap_dispatch(alley.execute(|lane| {
			for (limit, runs) in limits.into_iter().zip(&runs) {
				lane.with_limited_parallelism(limit, || {
					runs.fetch_add(1, Ordering::Relaxed);
				});
				// A lane that falls a whole claim behind leaves it to fewer lanes, so the lanes meet between claims.
				let _ = lane.each(|| ());
			}
		}));

		assert_eq!(runs.map(AtomicUsize::into_inner), [3, 5, 0, 8]);
	}

	#[test]
	fn a_claim_goes_to_the_first_lane_to_reach_it() {
		let mut alley = Alley::with_parallelism(4);
		let ran_on = AtomicUsize::new(usize::MAX);

		unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			if lane_idx != 2 {
				// Every other lane is busy until the claim ran, so the third lane reaches it first.
				wait_until(|| ran_on.load(Ordering::Acquire) != usize::MAX);
			}
			lane.only_one_runs(|| ran_on.store(lane_idx, Ordering::Release));
		}));

		assert_eq!(ran_on.load(Ordering::Relaxed), 2);
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
	fn a_busy_lane_holds_up_no_broadcast() {
		let mut alley = Alley::with_parallelism(4);
		let ran_on = AtomicUsize::new(usize::MAX);

		let values = unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			if lane_idx == 0 {
				// The first lane is busy until another lane ran the broadcast, which none could if they waited for it.
				wait_until(|| ran_on.load(Ordering::Acquire) != usize::MAX);
			}
			lane.broadcast(|| {
				ran_on.store(lane_idx, Ordering::Release);
				lane_idx
			})
		}));

		let ran_on = ran_on.load(Ordering::Relaxed);
		assert_ne!(ran_on, 0);
		assert_eq!(values, [ran_on; 4]);
	}

	#[test]
	fn a_busy_lane_holds_up_no_lane_that_enters_a_shared_stage() {
		let mut alley = Alley::with_parallelism(4);
		let mut value = 7usize;
		let entered = AtomicUsize::new(0);

		let others_went_first = unwrap_dispatch(alley.execute_phased(&mut value, 4, |lane, value| {
			if lane.idx() == 0 {
				// The first lane is busy until every other lane ran its share, which none could if they waited for it to
				// enter.
				wait_until(|| entered.load(Ordering::Acquire) == 3);
			}
			let others_went_first = entered.load(Ordering::Acquire) == 3;
			let _ = lane.each_ref(value, |value, _| {
				assert_eq!(*value, 7);
				entered.fetch_add(1, Ordering::AcqRel);
			});
			others_went_first
		}));

		assert!(others_went_first[0]);
	}

	#[test]
	fn no_lane_reaches_the_resource_while_another_runs_an_exclusive_stage() {
		let mut alley = Alley::with_parallelism(4);
		let writing = AtomicBool::new(false);
		let overlapped = AtomicBool::new(false);
		let mut value = 0usize;

		for writer in 0..4 {
			let result = alley.execute_phased(&mut value, 4, |lane, value| {
				if lane.idx() == writer {
					lane.broadcast_mut(value, |value| {
						writing.store(true, Ordering::SeqCst);
						sleep(Duration::from_millis(1));
						*value += 1;
						writing.store(false, Ordering::SeqCst);
					});
				} else {
					// These lanes run a shared stage where the writer runs an exclusive one, which fails the dispatch.
					let _ = lane.each_ref(value, |_, _| {
						for _ in 0..2 {
							if writing.load(Ordering::SeqCst) {
								overlapped.store(true, Ordering::SeqCst);
							}
							sleep(Duration::from_millis(1));
						}
					});
				}
			});

			assert!(result.is_err());
		}

		assert!(!overlapped.load(Ordering::SeqCst));
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
	fn each_keeps_values_for_any_number_of_lanes() {
		let mut alley = Alley::with_parallelism(66);

		let counts = unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let indices = lane.each(|| lane_idx);
			assert!(indices.eq(0..66));
			lane.broadcast(|| 66usize)
		}));

		assert_eq!(counts, [66; 66]);
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
	fn lanes_that_mix_each_with_each_ref_fail_the_dispatch() {
		let mut alley = Alley::with_parallelism(4);
		let mut value = 0usize;

		for plain_lane in [0, 2] {
			let result = alley.execute_phased(&mut value, 4, |lane, value| {
				if lane.idx() == plain_lane {
					let _ = lane.each(|| ());
				} else {
					let _ = lane.each_ref(value, |_, _| ());
				}
			});

			assert!(result.is_err());
		}
	}

	#[test]
	fn a_lane_that_leaves_a_section_early_keeps_its_values_for_the_lanes_behind_it() {
		let mut alley = Alley::with_parallelism(3);

		unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			for round in 0..30 {
				// The middle lane is slow, so the last lane often leaves each section and arrives at the next before
				// the first lane looks for it.
				let values = lane.each(|| {
					if lane_idx == 1 {
						sleep(Duration::from_micros(50));
					}
					lane_idx * 100 + round
				});
				assert!(values.eq([round, 100 + round, 200 + round]));
			}
		}));
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
					// The first lane runs both sections before the other one enters the first.
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
	fn a_lane_that_panics_between_sections_fails_the_dispatch() {
		let mut alley = Alley::with_parallelism(4);

		let result = alley.execute(|lane| {
			if lane.idx() == 3 {
				panic!("expected lane panic");
			}
			// The other lanes would wait here for the lane that panicked.
			let _ = lane.each(|| ());
		});

		assert!(result.is_err());
		assert_eq!(unwrap_dispatch(alley.execute(|lane| lane.idx())), [0, 1, 2, 3]);
	}

	#[test]
	fn lanes_that_run_different_numbers_of_sections_fail_the_dispatch() {
		let mut alley = Alley::with_parallelism(4);

		// The first lane runs out of sections, and then another lane does.
		for short_lane in [0, 2] {
			let result = alley.execute(|lane| {
				let _ = lane.each(|| ());
				if lane.idx() != short_lane {
					let _ = lane.each(|| ());
				}
			});

			assert!(result.is_err());
		}
	}

	#[test]
	fn a_lane_that_catches_the_failure_of_its_dispatch_runs_no_further_section() {
		let mut alley = Alley::with_parallelism(2);
		let ran = AtomicUsize::new(0);

		let result = alley.execute(|lane| {
			let caught = catch_unwind(AssertUnwindSafe(|| {
				if lane.idx() == 0 {
					let _ = lane.each(|| 1usize);
				} else {
					let _ = lane.broadcast(|| 1usize);
				}
			}));
			if caught.is_err() {
				// The dispatch failed, so this broadcast panics before any lane runs it.
				lane.broadcast(|| {
					ran.fetch_add(1, Ordering::Relaxed);
				});
			}
		});

		assert!(result.is_err());
		assert_eq!(ran.load(Ordering::Relaxed), 0);
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

mod batches;
mod split;

use std::{
	any::{Any, TypeId},
	cell::{Cell, UnsafeCell},
	hint::spin_loop,
	marker::PhantomData,
	mem::{MaybeUninit, align_of, size_of},
	ops::{Deref, DerefMut, Range},
	panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
	ptr::NonNull,
	sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering, fence},
	thread::yield_now,
};

pub use batches::{BatchOwned, Claim, PairBatches};
pub use split::{AlleyId, LaneCells, LaneOwned, LaneRanges, Owned, RangeOwned, SplitByRange};

use crate::core::threadpool::LanePool;
