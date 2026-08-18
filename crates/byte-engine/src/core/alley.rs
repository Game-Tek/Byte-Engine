/// The `Alley` struct provides scoped parallel execution across a fixed number of lanes.
pub struct Alley<'scope> {
	threadpool: ScopedThreadPool<'scope>,
}

/// The `AlleyFunction` trait defines work that every lane runs during an [`Alley`] dispatch.
pub trait AlleyFunction = for<'dispatch> Fn(&ConcreteLane<'dispatch>) + Clone + Send;

/// The `Lane` trait identifies a logical parallel path through an [`Alley`].
pub trait Lane {
	fn idx(&self) -> usize;
}

/// The `ConcreteLane` struct provides lane-local operations during an [`Alley`] dispatch.
pub struct ConcreteLane<'dispatch> {
	lane_idx: usize,
	state: &'dispatch DispatchState,
	next_single_runner_section: Cell<u32>,
	next_broadcast_section: Cell<usize>,
}

impl<'dispatch> ConcreteLane<'dispatch> {
	fn new(lane_idx: usize, state: &'dispatch DispatchState) -> Self {
		Self {
			lane_idx,
			state,
			next_single_runner_section: Cell::new(0),
			next_broadcast_section: Cell::new(0),
		}
	}

	/// Runs `f` on the first lane to reach this single-runner section.
	///
	/// Every lane must encounter single-runner sections in the same order.
	pub fn only_one_runs<F>(&self, f: F)
	where
		F: FnOnce(),
	{
		let section = self.next_single_runner_section.get();
		assert!(
			section < usize::BITS,
			"Single-runner section limit exceeded. A dispatch supports at most usize::BITS sections."
		);
		self.next_single_runner_section.set(section + 1);

		let claim = 1usize << section;
		if self.state.claimed.fetch_or(claim, Ordering::Relaxed) & claim == 0 {
			f();
		}
	}

	/// Returns the value produced by the first lane to reach this broadcast section.
	///
	/// Every lane must encounter broadcast sections in the same order and with the same types.
	pub fn broadcast<T, F>(&self, f: F) -> T
	where
		T: Copy + Send + Sync + 'static,
		F: FnOnce() -> T,
	{
		let section = self.next_broadcast_section.get();
		self.next_broadcast_section.set(section + 1);

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

const BROADCAST_SLOT_COUNT: usize = 32;
const BROADCAST_SLOT_SIZE: usize = 64;
const BROADCAST_SLOT_ALIGNMENT: usize = 64;

struct DispatchState {
	claimed: AtomicUsize,
	broadcasts: [BroadcastSlot; BROADCAST_SLOT_COUNT],
}

impl DispatchState {
	fn new() -> Self {
		Self {
			claimed: AtomicUsize::new(0),
			broadcasts: std::array::from_fn(|_| BroadcastSlot::new()),
		}
	}
}

struct BroadcastSlot {
	initialized: Once,
	type_id: OnceLock<TypeId>,
	storage: UnsafeCell<BroadcastStorage>,
}

impl BroadcastSlot {
	fn new() -> Self {
		Self {
			initialized: Once::new(),
			type_id: OnceLock::new(),
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

		self.initialized.call_once(|| {
			let value = f();
			// SAFETY: The size and alignment checks above prove that `T` fits in the
			// slot. `Once` grants this closure exclusive initialization access.
			unsafe { self.storage.get().cast::<T>().write(value) };
			self.type_id
				.set(TypeId::of::<T>())
				.expect("Broadcast type registration failed. The slot was initialized more than once.");
		});

		assert!(
			self.type_id.get() == Some(&TypeId::of::<T>()),
			"Broadcast type mismatch at section {section}. Every lane must use the same result type for that section."
		);

		// SAFETY: `Once` establishes that initialization completed, the type ID
		// matches `T`, and `T: Copy` means reading does not move out of the slot.
		unsafe { self.storage.get().cast::<T>().read() }
	}
}

// SAFETY: `Once` serializes writes and publishes them before reads. The slot only
// accepts `Send + Sync` values, and its storage is not accessed outside that protocol.
#[allow(unsafe_code, reason = "Once synchronizes access to the slot's UnsafeCell storage.")]
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
			f(&ConcreteLane::new(lane_idx, state));
		});
	}

	/// Distributes `items` evenly and gives each lane exclusive access to one partition.
	pub fn for_each_mut<T, F>(&self, items: &mut [T], f: F)
	where
		T: Send,
		F: for<'dispatch> Fn(&ConcreteLane<'dispatch>, &mut [T]) + Clone + Send,
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
				f(&ConcreteLane::new(current_lane_idx, state), chunk);
			})
		});

		self.threadpool.execute_many(jobs);
	}
}

#[cfg(test)]
mod tests {
	use std::{
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
}

use std::{
	any::TypeId,
	cell::{Cell, UnsafeCell},
	mem::{align_of, size_of, MaybeUninit},
	sync::{
		atomic::{AtomicUsize, Ordering},
		Once, OnceLock,
	},
};

use crate::core::threadpool::{Scope, ScopedThreadPool};
