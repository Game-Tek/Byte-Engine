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
	next_broadcast_section: usize,
}

impl<'dispatch> ConcreteLane<'dispatch> {
	fn new(lane_idx: usize, state: &'dispatch DispatchState) -> Self {
		Self {
			lane_idx,
			state,
			next_limited_parallelism_section: 0,
			next_broadcast_section: 0,
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
		let section = self.next_broadcast_section;
		self.next_broadcast_section += 1;

		self.state
			.broadcasts
			.get(section)
			.unwrap_or_else(|| {
				panic!("Broadcast capacity exceeded. A dispatch supports at most {BROADCAST_SLOT_COUNT} broadcast sections.")
			})
			.get_or_init(section, f)
	}
}

impl Lane for ConcreteLane<'_> {
	fn idx(&self) -> usize {
		self.lane_idx
	}
}

const LIMITED_PARALLELISM_SECTION_COUNT: usize = 32;
const BROADCAST_SLOT_COUNT: usize = 32;
const BROADCAST_SLOT_SIZE: usize = 64;
const BROADCAST_SLOT_ALIGNMENT: usize = 64;

const BROADCAST_EMPTY: u8 = 0;
const BROADCAST_WRITING: u8 = 1;
const BROADCAST_READY: u8 = 2;
const BROADCAST_POISONED: u8 = 3;

struct DispatchState {
	limited_parallelism: [AtomicUsize; LIMITED_PARALLELISM_SECTION_COUNT],
	broadcasts: [BroadcastSlot; BROADCAST_SLOT_COUNT],
}

impl DispatchState {
	fn new() -> Self {
		Self {
			limited_parallelism: std::array::from_fn(|_| AtomicUsize::new(0)),
			broadcasts: std::array::from_fn(|_| BroadcastSlot::new()),
		}
	}
}

struct BroadcastSlot {
	state: AtomicU8,
	type_id: UnsafeCell<MaybeUninit<TypeId>>,
	storage: UnsafeCell<BroadcastStorage>,
}

impl BroadcastSlot {
	fn new() -> Self {
		Self {
			state: AtomicU8::new(BROADCAST_EMPTY),
			type_id: UnsafeCell::new(MaybeUninit::uninit()),
			storage: UnsafeCell::new(BroadcastStorage::new()),
		}
	}

	/// Initializes the slot once and returns a copy of its shared value.
	#[allow(unsafe_code, reason = "Broadcast values use synchronized, fixed-size inline storage.")]
	fn get_or_init<T, F>(&self, section: usize, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		assert!(
			size_of::<T>() <= BROADCAST_SLOT_SIZE,
			"Broadcast value is too large. Section {section} exceeds the {BROADCAST_SLOT_SIZE}-byte slot capacity."
		);
		assert!(
			align_of::<T>() <= BROADCAST_SLOT_ALIGNMENT,
			"Broadcast value alignment is unsupported. Section {section} requires more than {BROADCAST_SLOT_ALIGNMENT}-byte alignment."
		);

		if self
			.state
			.compare_exchange(BROADCAST_EMPTY, BROADCAST_WRITING, Ordering::Relaxed, Ordering::Relaxed)
			.is_ok()
		{
			match catch_unwind(AssertUnwindSafe(f)) {
				Ok(value) => {
					// SAFETY: The size and alignment checks above prove that `T` fits in
					// the slot, and the successful state transition grants exclusive write access.
					unsafe {
						self.storage.get().cast::<T>().write(value);
						self.type_id.get().write(MaybeUninit::new(TypeId::of::<T>()));
					}
					self.state.store(BROADCAST_READY, Ordering::Release);
				}
				Err(payload) => {
					self.state.store(BROADCAST_POISONED, Ordering::Release);
					resume_unwind(payload);
				}
			}
		} else {
			self.wait_until_readable(section);
		}

		// SAFETY: A ready state is observed with `Acquire`, so the winning lane's
		// type and value writes happen before these reads.
		let stored_type = unsafe { self.type_id.get().read().assume_init() };
		assert!(
			stored_type == TypeId::of::<T>(),
			"Broadcast type mismatch at section {section}. Every lane must use the same result type for that section."
		);

		// SAFETY: The published type matches `T`, and `T: Copy` means reading does
		// not move the shared value out of the slot.
		unsafe { self.storage.get().cast::<T>().read() }
	}

	/// Waits until the winning lane publishes a value or reports an initialization panic.
	fn wait_until_readable(&self, section: usize) {
		let mut spins = 0usize;
		loop {
			match self.state.load(Ordering::Acquire) {
				BROADCAST_READY => return,
				BROADCAST_POISONED => {
					panic!("Broadcast initialization failed. The lane that claimed section {section} panicked.")
				}
				BROADCAST_EMPTY | BROADCAST_WRITING => {
					if spins < 64 {
						spin_loop();
						spins += 1;
					} else {
						yield_now();
					}
				}
				state => unreachable!("unknown broadcast state {state}"),
			}
		}
	}
}

// SAFETY: The atomic state grants exclusive write access and publishes writes before
// reads. The slot only accepts `Send + Sync` values and is not accessed outside that protocol.
#[allow(unsafe_code, reason = "Atomic state synchronizes access to the slot's inline storage.")]
unsafe impl Sync for BroadcastSlot {}

#[repr(C, align(64))]
struct BroadcastStorage {
	bytes: [MaybeUninit<u8>; BROADCAST_SLOT_SIZE],
}

impl BroadcastStorage {
	fn new() -> Self {
		Self {
			bytes: [MaybeUninit::uninit(); BROADCAST_SLOT_SIZE],
		}
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
		let state = DispatchState::new();
		let state = &state;
		self.threadpool.execute_on_all(move |lane_idx| {
			f(&mut ConcreteLane::new(lane_idx, state));
		});
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
		let state = DispatchState::new();
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
}

use std::{
	any::TypeId,
	cell::UnsafeCell,
	hint::spin_loop,
	mem::{align_of, size_of, MaybeUninit},
	panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
	sync::atomic::{AtomicU8, AtomicUsize, Ordering},
	thread::yield_now,
};

use crate::core::threadpool::{Scope, ScopedThreadPool};
