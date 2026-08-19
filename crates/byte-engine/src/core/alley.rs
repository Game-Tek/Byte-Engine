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
	fn new(collective: &'dispatch CollectiveSlot, lane_count: usize) -> Self {
		Self {
			storage: collective.storage.get().cast::<u8>(),
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
		// SAFETY: `CollectiveSlot::each` constructs this iterator only after every lane
		// initialized its aligned slot and the collective reached ready. `T: Copy` leaves storage valid.
		unsafe { self.storage.add(lane_idx * COLLECTIVE_VALUE_SIZE).cast::<T>().read() }
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
	next_collective_section: usize,
}

impl<'dispatch> ConcreteLane<'dispatch> {
	fn new(lane_idx: usize, state: &'dispatch DispatchState) -> Self {
		Self {
			lane_idx,
			state,
			next_limited_parallelism_section: 0,
			next_collective_section: 0,
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

	/// Returns the value produced by the first lane to reach this broadcast section.
	///
	/// Every lane must encounter broadcast sections in the same order and with the same types.
	pub fn broadcast<T, F>(&mut self, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		let (section, collective) = self.next_collective();
		collective.broadcast(section, f)
	}

	/// Runs `f` on every lane and returns lane-ordered values after every result is published.
	///
	/// Every lane must encounter `each` sections in the same order and with the same types.
	pub fn each<T, F>(&mut self, f: F) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		let lane_idx = self.lane_idx;
		let lane_count = self.state.lane_count;
		let (section, collective) = self.next_collective();
		collective.each(lane_idx, lane_count, section, f)
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
		let partition = shared_partition(values, self.lane_idx, self.state.lane_count);
		self.each(move || f(partition))
	}

	fn next_collective(&mut self) -> (usize, &'dispatch CollectiveSlot) {
		let section = self.next_collective_section;
		self.next_collective_section += 1;
		let collective = self.state.collectives.get(section).unwrap_or_else(|| {
			panic!("Collective capacity exceeded. A dispatch supports at most {COLLECTIVE_SECTION_COUNT} collective sections.")
		});
		(section, collective)
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

/// Returns the balanced read-only partition assigned to one lane.
fn shared_partition<T>(values: &[T], lane_idx: usize, lane_count: usize) -> &[T] {
	let minimum_len = values.len() / lane_count;
	let larger_partition_count = values.len() % lane_count;
	let start = lane_idx * minimum_len + lane_idx.min(larger_partition_count);
	let len = balanced_partition_len(values.len(), lane_idx, lane_count);
	&values[start..start + len]
}

fn balanced_partition_len(value_count: usize, lane_idx: usize, lane_count: usize) -> usize {
	value_count / lane_count + usize::from(lane_idx < value_count % lane_count)
}

const LIMITED_PARALLELISM_SECTION_COUNT: usize = 32;
const COLLECTIVE_SECTION_COUNT: usize = 32;
const COLLECTIVE_LANE_CAPACITY: usize = 64;
const COLLECTIVE_VALUE_SIZE: usize = 64;
const COLLECTIVE_VALUE_ALIGNMENT: usize = 64;

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum CollectiveKind {
	Unset,
	Broadcast,
	Each,
}

impl CollectiveKind {
	const fn as_raw(self) -> u8 {
		self as u8
	}

	/// Converts atomic storage into a collective operation kind.
	fn from_raw(raw: u8) -> Self {
		match raw {
			0 => Self::Unset,
			1 => Self::Broadcast,
			2 => Self::Each,
			_ => {
				unreachable!("Unknown collective kind raw value {raw}. Atomic collective metadata contained an invalid value.")
			}
		}
	}
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum CollectiveState {
	Empty,
	Registering,
	Active,
	Ready,
	Poisoned,
}

impl CollectiveState {
	const fn as_raw(self) -> u8 {
		self as u8
	}

	/// Converts atomic storage into a collective lifecycle state.
	fn from_raw(raw: u8) -> Self {
		match raw {
			0 => Self::Empty,
			1 => Self::Registering,
			2 => Self::Active,
			3 => Self::Ready,
			4 => Self::Poisoned,
			_ => {
				unreachable!("Unknown collective state raw value {raw}. Atomic collective metadata contained an invalid value.")
			}
		}
	}
}

struct DispatchState {
	lane_count: usize,
	limited_parallelism: [AtomicUsize; LIMITED_PARALLELISM_SECTION_COUNT],
	collectives: [CollectiveSlot; COLLECTIVE_SECTION_COUNT],
}

impl DispatchState {
	fn new(lane_count: usize) -> Self {
		Self {
			lane_count,
			limited_parallelism: std::array::from_fn(|_| AtomicUsize::new(0)),
			collectives: std::array::from_fn(|_| CollectiveSlot::new()),
		}
	}
}

struct CollectiveSlot {
	kind: AtomicU8,
	state: AtomicU8,
	producer_claimed: AtomicBool,
	published: AtomicUsize,
	type_id: UnsafeCell<MaybeUninit<TypeId>>,
	storage: UnsafeCell<CollectiveStorage>,
}

impl CollectiveSlot {
	fn new() -> Self {
		Self {
			kind: AtomicU8::new(CollectiveKind::Unset.as_raw()),
			state: AtomicU8::new(CollectiveState::Empty.as_raw()),
			producer_claimed: AtomicBool::new(false),
			published: AtomicUsize::new(0),
			type_id: UnsafeCell::new(MaybeUninit::uninit()),
			storage: UnsafeCell::new(CollectiveStorage::new()),
		}
	}

	/// Publishes one elected lane value and returns it to every lane.
	#[allow(
		unsafe_code,
		reason = "Broadcast reads a synchronized value from inline collective storage."
	)]
	fn broadcast<T, F>(&self, section: usize, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		self.validate_value::<T>(section);
		self.register::<T>(CollectiveKind::Broadcast, section);

		if self
			.producer_claimed
			.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
			.is_ok()
		{
			let value = self.compute(f);
			// SAFETY: Registration validates the type layout, and producer election
			// grants this lane exclusive write access to lane slot zero.
			unsafe { self.value_ptr::<T>(0).write(value) };
			self.published.store(1, Ordering::Release);
			self.publish_ready(section);
		} else {
			self.wait_until_ready(section);
		}

		// SAFETY: Ready was published after lane slot zero was initialized, and `T: Copy`.
		unsafe { self.value_ptr::<T>(0).read() }
	}

	/// Publishes one value per lane and returns completed values in lane order.
	#[allow(unsafe_code, reason = "Each writes synchronized lane values to strided inline storage.")]
	fn each<'dispatch, T, F>(
		&'dispatch self,
		lane_idx: usize,
		lane_count: usize,
		section: usize,
		f: F,
	) -> LaneValues<'dispatch, T>
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		if lane_count > COLLECTIVE_LANE_CAPACITY {
			self.poison();
			panic!(
				"Collective lane capacity exceeded. Section {section} has {lane_count} lanes but supports at most {COLLECTIVE_LANE_CAPACITY}."
			);
		}
		self.validate_value::<T>(section);
		self.register::<T>(CollectiveKind::Each, section);

		let value = self.compute(f);
		// SAFETY: Each lane writes to its unique cache-line-sized slot after layout validation.
		unsafe { self.value_ptr::<T>(lane_idx).write(value) };
		let published = self.published.fetch_add(1, Ordering::AcqRel) + 1;
		if published == lane_count {
			self.publish_ready(section);
		} else {
			self.wait_until_ready(section);
		}

		LaneValues::new(self, lane_count)
	}

	fn compute<T, F>(&self, f: F) -> T
	where
		F: FnOnce() -> T,
	{
		match catch_unwind(AssertUnwindSafe(f)) {
			Ok(value) => value,
			Err(payload) => {
				self.poison();
				resume_unwind(payload);
			}
		}
	}

	fn validate_value<T>(&self, section: usize) {
		if size_of::<T>() > COLLECTIVE_VALUE_SIZE {
			self.poison();
			panic!("Collective value is too large. Section {section} exceeds the {COLLECTIVE_VALUE_SIZE}-byte value capacity.");
		}
		if align_of::<T>() > COLLECTIVE_VALUE_ALIGNMENT {
			self.poison();
			panic!(
				"Collective value alignment is unsupported. Section {section} requires more than {COLLECTIVE_VALUE_ALIGNMENT}-byte alignment."
			);
		}
	}

	/// Registers and validates the operation kind and value type shared by every lane.
	#[allow(unsafe_code, reason = "Collective type metadata is published through atomic state.")]
	fn register<T>(&self, kind: CollectiveKind, section: usize)
	where
		T: 'static,
	{
		match self.kind.compare_exchange(
			CollectiveKind::Unset.as_raw(),
			kind.as_raw(),
			Ordering::Relaxed,
			Ordering::Relaxed,
		) {
			Ok(_) => {}
			Err(registered) if CollectiveKind::from_raw(registered) == kind => {}
			Err(_) => {
				self.poison();
				panic!(
					"Collective operation mismatch at section {section}. Every lane must call broadcast or each consistently."
				);
			}
		}

		if self
			.state
			.compare_exchange(
				CollectiveState::Empty.as_raw(),
				CollectiveState::Registering.as_raw(),
				Ordering::Relaxed,
				Ordering::Relaxed,
			)
			.is_ok()
		{
			// SAFETY: The successful state transition grants exclusive type write access.
			unsafe { self.type_id.get().write(MaybeUninit::new(TypeId::of::<T>())) };
			if self
				.state
				.compare_exchange(
					CollectiveState::Registering.as_raw(),
					CollectiveState::Active.as_raw(),
					Ordering::Release,
					Ordering::Acquire,
				)
				.is_err()
			{
				self.panic_poisoned(section);
			}
		} else {
			self.wait_until_registered(section);
		}

		// SAFETY: Active or ready was observed with `Acquire` after type publication.
		let stored_type = unsafe { self.type_id.get().read().assume_init() };
		if stored_type != TypeId::of::<T>() {
			self.poison();
			panic!("Collective type mismatch at section {section}. Every lane must use the same result type for that section.");
		}
	}

	fn publish_ready(&self, section: usize) {
		match self.state.compare_exchange(
			CollectiveState::Active.as_raw(),
			CollectiveState::Ready.as_raw(),
			Ordering::Release,
			Ordering::Acquire,
		) {
			Ok(_) => {}
			Err(state) => match CollectiveState::from_raw(state) {
				CollectiveState::Ready => {}
				CollectiveState::Poisoned => self.panic_poisoned(section),
				state => {
					unreachable!("Cannot publish a collective from state {state:?}. The collective lifecycle is inconsistent.")
				}
			},
		}
	}

	fn wait_until_registered(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			match CollectiveState::from_raw(self.state.load(Ordering::Acquire)) {
				CollectiveState::Active | CollectiveState::Ready => return,
				CollectiveState::Poisoned => self.panic_poisoned(section),
				CollectiveState::Empty | CollectiveState::Registering => wait_briefly(&mut spins),
			}
		}
	}

	fn wait_until_ready(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			match CollectiveState::from_raw(self.state.load(Ordering::Acquire)) {
				CollectiveState::Ready => return,
				CollectiveState::Poisoned => self.panic_poisoned(section),
				CollectiveState::Empty | CollectiveState::Registering | CollectiveState::Active => {
					wait_briefly(&mut spins);
				}
			}
		}
	}

	fn poison(&self) {
		self.state.store(CollectiveState::Poisoned.as_raw(), Ordering::Release);
	}

	fn panic_poisoned(&self, section: usize) -> ! {
		panic!("Collective failed. A lane in section {section} panicked or used a different operation or type.")
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

// SAFETY: Producers write exclusive elements, then publish with release ordering.
// Readers wait for an acquire observation of ready before reading inline storage.
#[allow(unsafe_code, reason = "Atomic publication synchronizes CollectiveSlot inline storage.")]
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
			atomic::{AtomicUsize, Ordering},
			Barrier,
		},
		thread::sleep,
		time::Duration,
	};

	use crate::core::alley::{Alley, Lane};

	fn unwrap_dispatch<R>(result: std::thread::Result<Vec<R>>) -> Vec<R> {
		result.unwrap_or_else(|payload| resume_unwind(payload))
	}

	#[test]
	fn runs_on_all_lanes() {
		let mut alley = Alley::with_parallelism(8);
		let counter = AtomicUsize::new(0);

		unwrap_dispatch(alley.execute(|_| {
			counter.fetch_add(1, Ordering::Relaxed);
		}));

		assert_eq!(counter.load(Ordering::Relaxed), 8);
	}

	#[test]
	fn execute_returns_one_ordered_value_per_lane() {
		let mut alley = Alley::with_parallelism(4);

		let values = unwrap_dispatch(alley.execute(|lane| lane.idx() * 10));

		assert_eq!(values, [0, 10, 20, 30]);
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
	fn broadcast_and_each_share_collective_sections() {
		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			let lane_idx = lane.idx();
			let first = lane.broadcast(|| 7u32);
			let all = lane.each(|| lane_idx);
			let second = lane.broadcast(|| 11u16);

			assert_eq!(first, 7);
			assert_eq!(all.collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 7]);
			assert_eq!(second, 11);
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
	fn each_panic_releases_waiting_lanes() {
		let mut alley = Alley::with_parallelism(8);
		let result = alley.execute(|lane| {
			let _ = lane.each(|| -> usize { panic!("expected each panic") });
		});

		assert!(result.is_err());

		let completed = AtomicUsize::new(0);
		unwrap_dispatch(alley.execute(|_| {
			completed.fetch_add(1, Ordering::Relaxed);
		}));

		assert_eq!(completed.load(Ordering::Relaxed), 8);
	}

	#[test]
	fn each_shared() {
		let value = AtomicUsize::new(0);

		let values = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute(|lane| {
			let all = lane.each_shared(&values, |partition| partition.iter().sum::<usize>());

			lane.only_one_runs(|| {
				value.fetch_add(all.sum::<usize>(), Ordering::Relaxed);
			});
		}));

		assert_eq!(value.load(Ordering::Relaxed), 136);
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
	fn mut_access() {
		let mut a = 0;
		let mut b = 0;

		let mut alley = Alley::with_parallelism(8);

		unwrap_dispatch(alley.execute_with_mut((&mut a, &mut b), |lane, (a, b)| {
			let _ = lane.only_one_runs_mut(a, |a| *a += 1);
			let _ = lane.only_one_runs_mut(b, |b| *b += 1);
		}));

		assert_eq!(a, 1);
		assert_eq!(b, 1);
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
	mem::{align_of, size_of, MaybeUninit},
	ops::{Deref, DerefMut},
	panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
	sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
	thread::yield_now,
};

use crate::core::threadpool::LanePool;
