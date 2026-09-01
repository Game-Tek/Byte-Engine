const BAKE_MEMORY_RESERVATION_BYTES: usize = 512 * 1024 * 1024;
const BAKE_MEMORY_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The `BakeMemoryBudget` struct keeps concurrent bake arenas below a shared soft limit.
pub(super) struct BakeMemoryBudget {
	byte_limit: usize,
	retained_bytes: Mutex<usize>,
}

impl BakeMemoryBudget {
	pub(super) fn new(byte_limit: usize) -> Self {
		Self {
			byte_limit,
			retained_bytes: Mutex::new(0),
		}
	}

	/// Reserves admission capacity when the soft budget has room for another independent bake tree.
	fn try_acquire(self: &Arc<Self>) -> Result<Arc<BakeMemoryScope>, usize> {
		let reservation = self.byte_limit.min(BAKE_MEMORY_RESERVATION_BYTES);
		let mut retained_bytes = self.retained_bytes.lock();
		// Always admit one root. It may exceed the soft budget, but it must run so the bake can make progress.
		if *retained_bytes != 0 && retained_bytes.saturating_add(reservation) > self.byte_limit {
			return Err(*retained_bytes);
		}

		*retained_bytes = retained_bytes.saturating_add(reservation);
		Ok(Arc::new(BakeMemoryScope {
			budget: Arc::clone(self),
			state: Mutex::new(BakeMemoryScopeState {
				observed_bytes: 0,
				charged_bytes: reservation,
			}),
		}))
	}

	/// Waits until a new bake tree can reserve memory without blocking work that already owns memory.
	pub(super) async fn acquire(self: &Arc<Self>) -> Arc<BakeMemoryScope> {
		let mut reported_pause = false;

		loop {
			match self.try_acquire() {
				Ok(scope) => {
					if reported_pause {
						log::info!("Resuming asset baking after retained bake memory dropped below the configured budget.");
					}
					return scope;
				}
				Err(retained_bytes) if !reported_pause => {
					log::info!(
						"Pausing new asset bakes because reserved or retained bake memory is {} MiB and the configured budget is {} MiB. Active bakes will continue so they can release memory.",
						retained_bytes / (1024 * 1024),
						self.byte_limit / (1024 * 1024)
					);
					reported_pause = true;
				}
				Err(_) => {}
			}
			compio::time::sleep(BAKE_MEMORY_POLL_INTERVAL).await;
		}
	}
}

/// The `BakeMemoryScope` struct groups a root bake and its dependencies under one deadlock-free admission charge.
pub(super) struct BakeMemoryScope {
	budget: Arc<BakeMemoryBudget>,
	state: Mutex<BakeMemoryScopeState>,
}

/// The `BakeMemoryScopeState` struct tracks live arena bytes and the amount charged to the shared budget.
struct BakeMemoryScopeState {
	observed_bytes: usize,
	charged_bytes: usize,
}

impl BakeMemoryScope {
	/// Adds arena growth from one request and raises the shared charge above the initial reservation when needed.
	fn record_growth(&self, additional_bytes: usize) {
		let mut state = self.state.lock();
		state.observed_bytes = state.observed_bytes.saturating_add(additional_bytes);
		if state.observed_bytes <= state.charged_bytes {
			return;
		}

		let additional_charge = state.observed_bytes - state.charged_bytes;
		let mut retained_bytes = self.budget.retained_bytes.lock();
		*retained_bytes = retained_bytes.saturating_add(additional_charge);
		state.charged_bytes = state.observed_bytes;
	}

	/// Returns a completed dependency arena's bytes while retaining this bake tree's admission reservation.
	fn release(&self, released_bytes: usize) {
		let mut state = self.state.lock();
		state.observed_bytes = state.observed_bytes.saturating_sub(released_bytes);
		let reservation = self.budget.byte_limit.min(BAKE_MEMORY_RESERVATION_BYTES);
		let new_charge = reservation.max(state.observed_bytes);
		if new_charge >= state.charged_bytes {
			return;
		}

		let released_charge = state.charged_bytes - new_charge;
		let mut retained_bytes = self.budget.retained_bytes.lock();
		*retained_bytes = retained_bytes.saturating_sub(released_charge);
		state.charged_bytes = new_charge;
	}
}

impl Drop for BakeMemoryScope {
	fn drop(&mut self) {
		let charged_bytes = self.state.lock().charged_bytes;
		let mut retained_bytes = self.budget.retained_bytes.lock();
		*retained_bytes = retained_bytes.saturating_sub(charged_bytes);
	}
}

/// The `BakeAllocator` struct gives one request an arena charged to its root bake's memory scope.
pub(super) struct BakeAllocator {
	arena: bumpalo::Bump,
	memory_scope: Option<Arc<BakeMemoryScope>>,
	recorded_retained_bytes: Cell<usize>,
}

impl BakeAllocator {
	/// Acquires budget before creating the arena used by a new root bake request.
	pub(super) async fn new(memory_budget: Option<&Arc<BakeMemoryBudget>>) -> Self {
		let memory_scope = match memory_budget {
			Some(memory_budget) => Some(memory_budget.acquire().await),
			None => None,
		};
		Self::in_scope(memory_scope)
	}

	/// Creates a dependency arena in its parent's scope so waiting children cannot deadlock their parent.
	pub(super) fn in_scope(memory_scope: Option<Arc<BakeMemoryScope>>) -> Self {
		Self {
			arena: bumpalo::Bump::new(),
			memory_scope,
			recorded_retained_bytes: Cell::new(0),
		}
	}

	pub(super) fn memory_scope(&self) -> Option<&Arc<BakeMemoryScope>> {
		self.memory_scope.as_ref()
	}

	fn record_retained_bytes(&self) {
		let retained_bytes = self.arena.allocated_bytes_including_metadata();
		let previous_bytes = self.recorded_retained_bytes.get();
		if retained_bytes <= previous_bytes {
			return;
		}
		self.recorded_retained_bytes.set(retained_bytes);
		if let Some(memory_scope) = &self.memory_scope {
			memory_scope.record_growth(retained_bytes - previous_bytes);
		}
	}
}

impl Drop for BakeAllocator {
	fn drop(&mut self) {
		let retained_bytes = self.recorded_retained_bytes.get();
		// Release the arena before returning its charge so newly admitted work does not overlap its physical allocation.
		drop(std::mem::replace(&mut self.arena, bumpalo::Bump::new()));
		if let Some(memory_scope) = &self.memory_scope {
			memory_scope.release(retained_bytes);
		}
	}
}

#[allow(unsafe_code)]
// SAFETY: Every allocation operation delegates to the same arena, and the wrapper preserves each layout and pointer contract.
unsafe impl Allocator for BakeAllocator {
	fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
		let allocation = Allocator::allocate(&&self.arena, layout)?;
		self.record_retained_bytes();
		Ok(allocation)
	}

	unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
		// SAFETY: The caller guarantees that ptr and layout identify a live allocation from this allocator.
		unsafe { Allocator::deallocate(&&self.arena, ptr, layout) };
	}

	unsafe fn grow(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<[u8]>, AllocError> {
		// SAFETY: The caller guarantees the old allocation contract, and the arena owns the pointer.
		let allocation = unsafe { Allocator::grow(&&self.arena, ptr, old, new) }?;
		self.record_retained_bytes();
		Ok(allocation)
	}

	unsafe fn shrink(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<[u8]>, AllocError> {
		// SAFETY: The caller guarantees the old allocation contract, and the arena owns the pointer.
		let allocation = unsafe { Allocator::shrink(&&self.arena, ptr, old, new) }?;
		self.record_retained_bytes();
		Ok(allocation)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn budget_admits_reserved_work_only_after_capacity_returns() {
		let budget = Arc::new(BakeMemoryBudget::new(BAKE_MEMORY_RESERVATION_BYTES * 2));
		let first = budget.try_acquire().unwrap();
		let second = budget.try_acquire().unwrap();

		assert!(budget.try_acquire().is_err());
		drop(first);
		let replacement = budget.try_acquire().unwrap();
		drop((second, replacement));

		assert_eq!(*budget.retained_bytes.lock(), 0);
	}

	#[test]
	fn allocator_charges_retained_arena_growth_and_releases_it_on_drop() {
		let budget = Arc::new(BakeMemoryBudget::new(1024));
		let memory_scope = budget.try_acquire().unwrap();
		let allocator = BakeAllocator::in_scope(Some(Arc::clone(&memory_scope)));
		let mut data = Vec::<u8, _>::with_capacity_in(1, &allocator);
		data.resize(4096, 0);

		assert!(*budget.retained_bytes.lock() > 1024);
		drop(data);
		drop(allocator);

		assert_eq!(*budget.retained_bytes.lock(), 1024);
		drop(memory_scope);

		assert_eq!(*budget.retained_bytes.lock(), 0);
	}
}

use std::{
	alloc::{AllocError, Allocator, Layout},
	cell::Cell,
	ptr::NonNull,
	sync::Arc,
	time::Duration,
};

use utils::sync::Mutex;
