/// The `Alley` struct provides scoped parallel execution across a fixed number of lanes.
pub struct Alley<'scope> {
	threadpool: ScopedThreadPool<'scope>,
}

/// The `AlleyFunction` trait defines work that every lane runs during an [`Alley`] dispatch.
pub trait AlleyFunction = for<'dispatch> Fn(&mut ConcreteLane<'dispatch>) + Clone + Send;

/// The `Lane` trait identifies a logical parallel path through an [`Alley`].
pub trait Lane {
	fn idx(&self) -> usize;
}

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

	/// Runs `f` on every lane and returns all lane results after every value is published.
	///
	/// Every lane must encounter `each` sections in the same order and with the same types.
	pub fn each<T, F>(&mut self, f: F) -> &'dispatch [T]
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
	pub fn each_shared<T, R, F>(&mut self, values: &[T], f: F) -> &'dispatch [R]
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
	pub fn each_shared_mut<R, F>(&mut self, f: F) -> &'dispatch [R]
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

const COLLECTIVE_KIND_UNSET: u8 = 0;
const COLLECTIVE_KIND_BROADCAST: u8 = 1;
const COLLECTIVE_KIND_EACH: u8 = 2;

const COLLECTIVE_EMPTY: u8 = 0;
const COLLECTIVE_REGISTERING: u8 = 1;
const COLLECTIVE_ACTIVE: u8 = 2;
const COLLECTIVE_READY: u8 = 3;
const COLLECTIVE_POISONED: u8 = 4;

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
			kind: AtomicU8::new(COLLECTIVE_KIND_UNSET),
			state: AtomicU8::new(COLLECTIVE_EMPTY),
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
		self.register::<T>(COLLECTIVE_KIND_BROADCAST, section);

		if self
			.producer_claimed
			.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
			.is_ok()
		{
			let value = self.compute(f);
			// SAFETY: Registration validates the type layout, and producer election
			// grants this lane exclusive write access to element zero.
			unsafe { self.value_ptr::<T>(0).write(value) };
			self.published.store(1, Ordering::Release);
			self.publish_ready(section);
		} else {
			self.wait_until_ready(section);
		}

		// SAFETY: Ready was published after element zero was initialized, and `T: Copy`.
		unsafe { self.value_ptr::<T>(0).read() }
	}

	/// Publishes one value per lane and returns the completed lane-ordered result slice.
	#[allow(unsafe_code, reason = "Each reads synchronized lane values from inline collective storage.")]
	fn each<T, F>(&self, lane_idx: usize, lane_count: usize, section: usize, f: F) -> &[T]
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
		self.register::<T>(COLLECTIVE_KIND_EACH, section);

		let value = self.compute(f);
		// SAFETY: Each lane writes to its unique index after layout validation.
		unsafe { self.value_ptr::<T>(lane_idx).write(value) };
		let published = self.published.fetch_add(1, Ordering::AcqRel) + 1;
		if published == lane_count {
			self.publish_ready(section);
		} else {
			self.wait_until_ready(section);
		}

		// SAFETY: Ready was published after all lane-indexed values were initialized.
		unsafe { std::slice::from_raw_parts(self.value_ptr::<T>(0), lane_count) }
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
	fn register<T>(&self, kind: u8, section: usize)
	where
		T: 'static,
	{
		match self
			.kind
			.compare_exchange(COLLECTIVE_KIND_UNSET, kind, Ordering::Relaxed, Ordering::Relaxed)
		{
			Ok(_) => {}
			Err(registered) if registered == kind => {}
			Err(_) => {
				self.poison();
				panic!(
					"Collective operation mismatch at section {section}. Every lane must call broadcast or each consistently."
				);
			}
		}

		if self
			.state
			.compare_exchange(COLLECTIVE_EMPTY, COLLECTIVE_REGISTERING, Ordering::Relaxed, Ordering::Relaxed)
			.is_ok()
		{
			// SAFETY: The successful state transition grants exclusive type write access.
			unsafe { self.type_id.get().write(MaybeUninit::new(TypeId::of::<T>())) };
			if self
				.state
				.compare_exchange(
					COLLECTIVE_REGISTERING,
					COLLECTIVE_ACTIVE,
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
		match self
			.state
			.compare_exchange(COLLECTIVE_ACTIVE, COLLECTIVE_READY, Ordering::Release, Ordering::Acquire)
		{
			Ok(_) | Err(COLLECTIVE_READY) => {}
			Err(COLLECTIVE_POISONED) => self.panic_poisoned(section),
			Err(state) => unreachable!("cannot publish collective from state {state}"),
		}
	}

	fn wait_until_registered(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			match self.state.load(Ordering::Acquire) {
				COLLECTIVE_ACTIVE | COLLECTIVE_READY => return,
				COLLECTIVE_POISONED => self.panic_poisoned(section),
				COLLECTIVE_EMPTY | COLLECTIVE_REGISTERING => wait_briefly(&mut spins),
				state => unreachable!("unknown collective state {state}"),
			}
		}
	}

	fn wait_until_ready(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			match self.state.load(Ordering::Acquire) {
				COLLECTIVE_READY => return,
				COLLECTIVE_POISONED => self.panic_poisoned(section),
				COLLECTIVE_EMPTY | COLLECTIVE_REGISTERING | COLLECTIVE_ACTIVE => wait_briefly(&mut spins),
				state => unreachable!("unknown collective state {state}"),
			}
		}
	}

	fn poison(&self) {
		self.state.store(COLLECTIVE_POISONED, Ordering::Release);
	}

	fn panic_poisoned(&self, section: usize) -> ! {
		panic!("Collective failed. A lane in section {section} panicked or used a different operation or type.")
	}

	#[allow(unsafe_code, reason = "Collective values occupy lane-indexed offsets in inline storage.")]
	unsafe fn value_ptr<T>(&self, lane_idx: usize) -> *mut T {
		// SAFETY: Callers validate lane capacity, value size, and alignment.
		unsafe { self.storage.get().cast::<u8>().add(lane_idx * size_of::<T>()).cast::<T>() }
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

impl<'scope> Alley<'scope> {
	pub fn new(scope: &'scope Scope<'scope, '_>) -> Self {
		Self {
			threadpool: ScopedThreadPool::with_parallelism(scope, 8),
		}
	}

	pub fn with_parallelism(scope: &'scope Scope<'scope, '_>, count: usize) -> Self {
		Self {
			threadpool: ScopedThreadPool::with_parallelism(scope, count),
		}
	}

	pub fn execute(&self, f: impl AlleyFunction) {
		let state = DispatchState::new(self.threadpool.parallelism());
		let state = &state;
		self.threadpool.execute_on_all(move |lane_idx| {
			f(&mut ConcreteLane::new(lane_idx, state));
		});
	}

	/// Executes every lane with exclusive access to a balanced mutable partition.
	pub fn execute_shared_mut<T, F>(&self, values: &mut [T], f: F)
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

		self.threadpool.execute_many(jobs);
	}

	/// Distributes `items` evenly and gives each lane exclusive access to one partition.
	pub fn for_each_mut<T, F>(&self, items: &mut [T], f: F)
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

		self.threadpool.execute_many(jobs);
	}
}

#[cfg(test)]
mod tests {
	use std::{
		panic::{catch_unwind, AssertUnwindSafe},
		sync::atomic::{AtomicUsize, Ordering},
		thread::scope,
	};

	use crate::core::alley::{Alley, Lane};

	#[test]
	fn runs_on_all_lanes() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);
			let counter = AtomicUsize::new(0);

			alley.execute(|_| {
				counter.fetch_add(1, Ordering::Relaxed);
			});

			assert_eq!(counter.load(Ordering::Relaxed), 8);
		});
	}

	#[test]
	fn each_partition_receives_its_lane_index() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 3);
			let mut counters = [0; 10];

			alley.for_each_mut(&mut counters, |lane, partition| {
				partition.fill(lane.idx());
			});

			assert_eq!(counters, [0, 0, 0, 0, 1, 1, 1, 2, 2, 2]);
		});
	}

	#[test]
	fn only_one_runs() {
		let first_counter = AtomicUsize::new(0);
		let second_counter = AtomicUsize::new(0);

		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute(|lane| {
				lane.only_one_runs(|| {
					first_counter.fetch_add(1, Ordering::Relaxed);
				});
				lane.only_one_runs(|| {
					second_counter.fetch_add(1, Ordering::Relaxed);
				});
			});
		});

		assert_eq!(first_counter.load(Ordering::Relaxed), 1);
		assert_eq!(second_counter.load(Ordering::Relaxed), 1);
	}

	#[test]
	fn limited_parallelism_runs_only_the_first_lanes() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);
			let first_section = AtomicUsize::new(0);
			let second_section = AtomicUsize::new(0);
			let zero_section = AtomicUsize::new(0);

			alley.execute(|lane| {
				lane.with_limited_parallelism(3, || {
					first_section.fetch_add(1, Ordering::Relaxed);
				});
				lane.with_limited_parallelism(5, || {
					second_section.fetch_add(1, Ordering::Relaxed);
				});
				lane.with_limited_parallelism(0, || {
					zero_section.fetch_add(1, Ordering::Relaxed);
				});
			});

			assert_eq!(first_section.load(Ordering::Relaxed), 3);
			assert_eq!(second_section.load(Ordering::Relaxed), 5);
			assert_eq!(zero_section.load(Ordering::Relaxed), 0);
		});
	}

	#[test]
	fn broadcast() {
		let counter = AtomicUsize::new(0);
		let initializer_count = AtomicUsize::new(0);

		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute(|lane| {
				let first = lane.broadcast(|| {
					initializer_count.fetch_add(1, Ordering::Relaxed);
					1usize
				});
				let second = lane.broadcast(|| {
					initializer_count.fetch_add(1, Ordering::Relaxed);
					2u16
				});

				counter.fetch_add(first + usize::from(second), Ordering::Relaxed);
			});
		});

		assert_eq!(initializer_count.load(Ordering::Relaxed), 2);
		assert_eq!(counter.load(Ordering::Relaxed), 24);
	}

	#[test]
	fn broadcast_panic_releases_waiting_lanes() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);
			let panic = catch_unwind(AssertUnwindSafe(|| {
				alley.execute(|lane| {
					let _: usize = lane.broadcast(|| panic!("expected broadcast panic"));
				});
			}));
			assert!(panic.is_err());

			let completed = AtomicUsize::new(0);
			alley.execute(|_| {
				completed.fetch_add(1, Ordering::Relaxed);
			});
			assert_eq!(completed.load(Ordering::Relaxed), 8);
		});
	}

	#[test]
	fn each() {
		let counter = AtomicUsize::new(0);
		let initializer_count = AtomicUsize::new(0);

		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute(|lane| {
				let lane_idx = lane.idx();
				let all = lane.each(|| {
					initializer_count.fetch_add(1, Ordering::Relaxed);
					1usize
				});
				let lane_indices = lane.each(|| lane_idx);

				assert_eq!(lane_indices, &[0, 1, 2, 3, 4, 5, 6, 7]);
				counter.fetch_add(all.iter().sum::<usize>(), Ordering::Relaxed);
			});
		});

		assert_eq!(initializer_count.load(Ordering::Relaxed), 8);
		assert_eq!(counter.load(Ordering::Relaxed), 64);
	}

	#[test]
	fn broadcast_and_each_share_collective_sections() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute(|lane| {
				let lane_idx = lane.idx();
				let first = lane.broadcast(|| 7u32);
				let all = lane.each(|| lane_idx);
				let second = lane.broadcast(|| 11u16);

				assert_eq!(first, 7);
				assert_eq!(all, &[0, 1, 2, 3, 4, 5, 6, 7]);
				assert_eq!(second, 11);
			});
		});
	}

	#[test]
	fn collective_operation_mismatch_releases_waiting_lanes() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);
			let panic = catch_unwind(AssertUnwindSafe(|| {
				alley.execute(|lane| {
					if lane.idx() == 0 {
						let _ = lane.broadcast(|| 1usize);
					} else {
						let _ = lane.each(|| 1usize);
					}
				});
			}));
			assert!(panic.is_err());
		});
	}

	#[test]
	fn each_panic_releases_waiting_lanes() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);
			let panic = catch_unwind(AssertUnwindSafe(|| {
				alley.execute(|lane| {
					let _: &[usize] = lane.each(|| panic!("expected each panic"));
				});
			}));
			assert!(panic.is_err());

			let completed = AtomicUsize::new(0);
			alley.execute(|_| {
				completed.fetch_add(1, Ordering::Relaxed);
			});
			assert_eq!(completed.load(Ordering::Relaxed), 8);
		});
	}

	#[test]
	fn each_shared() {
		let value = AtomicUsize::new(0);

		let values = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute(|lane| {
				let all = lane.each_shared(&values, |partition| partition.iter().sum());

				lane.only_one_runs(|| {
					value.fetch_add(all.iter().sum::<usize>(), Ordering::Relaxed);
				});
			});
		});

		assert_eq!(value.load(Ordering::Relaxed), 136);
	}

	#[test]
	fn each_shared_balances_uneven_and_empty_partitions() {
		scope(|s| {
			let alley = Alley::with_parallelism(s, 5);
			let values = [1usize, 2, 3];

			alley.execute(|lane| {
				let lengths = lane.each_shared(&values, <[usize]>::len);
				let sums = lane.each_shared(&values, |partition| partition.iter().sum::<usize>());

				assert_eq!(lengths, &[1, 1, 1, 0, 0]);
				assert_eq!(sums, &[1, 2, 3, 0, 0]);
			});
		});
	}

	#[test]
	fn each_shared_mut() {
		let value = AtomicUsize::new(0);
		let mut values = vec![0; 16];

		scope(|s| {
			let alley = Alley::with_parallelism(s, 8);

			alley.execute_shared_mut(&mut values, |lane| {
				let lane_idx = lane.idx();
				let all = lane.each_shared_mut(|partition| {
					for (offset, value) in partition.iter_mut().enumerate() {
						*value = lane_idx * 2 + offset + 1;
					}
					partition.iter().sum::<usize>()
				});

				lane.only_one_runs(|| {
					value.fetch_add(all.iter().sum::<usize>(), Ordering::Relaxed);
				});
			});
		});

		assert_eq!(values, (1..=16).collect::<Vec<_>>());
		assert_eq!(value.load(Ordering::Relaxed), 136);
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

use crate::core::threadpool::{Scope, ScopedThreadPool};
