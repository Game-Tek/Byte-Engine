/// What a root bake is charged before its arenas grow; growth past it is charged as it happens.
///
/// It only bounds how many roots start before their growth shows, so it stays near a typical root's peak rather than the
/// largest's: Sponza, the largest model viewer root, peaks at about 240 MiB, while shaders and audio stay under 1 MiB.
/// A larger reservation fills the budget with roots that never use it and serializes the bake.
const BAKE_MEMORY_RESERVATION_BYTES: usize = 64 * 1024 * 1024;

/// The `BakeMemoryBudget` struct keeps concurrent bake arenas below a shared soft limit.
///
/// The asset manager owns it. Each root bake holds a [`MemoryLease`] on one tree of the table, which its dependencies
/// share, so a tree's charge returns once the root and every dependency it started are done.
pub(crate) struct BakeMemoryBudget {
	byte_limit: usize,
	state: Mutex<BudgetState>,
	/// The release that paused roots wait for, replaced each time one is announced.
	release: Mutex<(Announcer<()>, Announcement<()>)>,
}

/// The `BudgetState` struct is what the budget charged, in total and per bake tree.
struct BudgetState {
	retained_bytes: usize,
	/// One entry per bake tree; a tree no lease holds is free for the next root.
	trees: Vec<BakeTree>,
	/// The indices of the free entries in `trees`.
	free: Vec<u32>,
}

/// The `BakeTree` struct tracks the live arena bytes of a root bake and its dependencies, and what they are charged.
#[derive(Default)]
struct BakeTree {
	/// How many leases hold this tree: the root's, and one per dependency bake or queued request for one.
	leases: usize,
	observed_bytes: usize,
	charged_bytes: usize,
}

impl BakeMemoryBudget {
	pub(super) fn new(byte_limit: usize) -> Self {
		Self {
			byte_limit,
			state: Mutex::new(BudgetState {
				retained_bytes: 0,
				trees: Vec::with_capacity(16),
				free: Vec::with_capacity(16),
			}),
			release: Mutex::new(Announcement::new()),
		}
	}

	/// The charge every tree keeps while any lease holds it.
	fn reservation(&self) -> usize {
		self.byte_limit.min(BAKE_MEMORY_RESERVATION_BYTES)
	}

	/// Returns `bytes` to the budget and wakes every paused root, so the first to retry is admitted as soon as memory
	/// returns rather than at a polling interval.
	fn return_bytes(&self, state: &mut BudgetState, bytes: usize) {
		state.retained_bytes = state.retained_bytes.saturating_sub(bytes);
		let (released, _) = std::mem::replace(&mut *self.release.lock(), Announcement::new());
		let _ = released.announce(());
	}

	/// Reserves admission capacity when the soft budget has room for another independent bake tree.
	fn try_acquire(&self) -> Result<MemoryLease<'_>, usize> {
		let reservation = self.reservation();
		let mut state = self.state.lock();
		// Always admit one root. It may exceed the soft budget, but it must run so the bake can make progress.
		if state.retained_bytes != 0 && state.retained_bytes.saturating_add(reservation) > self.byte_limit {
			return Err(state.retained_bytes);
		}

		state.retained_bytes = state.retained_bytes.saturating_add(reservation);
		let tree = BakeTree {
			leases: 1,
			observed_bytes: 0,
			charged_bytes: reservation,
		};
		let index = match state.free.pop() {
			Some(index) => {
				state.trees[index as usize] = tree;
				index
			}
			None => {
				state.trees.push(tree);
				(state.trees.len() - 1) as u32
			}
		};
		Ok(MemoryLease { budget: self, tree: index })
	}

	/// Waits until a new bake tree can reserve memory without blocking work that already owns memory.
	pub(super) async fn acquire(&self) -> MemoryLease<'_> {
		let mut reported_pause = false;

		loop {
			// Listen before trying, so a release between a failed try and the wait still wakes this root.
			let release = self.release.lock().1.listener();
			match self.try_acquire() {
				Ok(lease) => {
					if reported_pause {
						log::trace!("Resuming asset baking after retained bake memory dropped below the configured budget.");
					}
					return lease;
				}
				Err(retained_bytes) if !reported_pause => {
					log::debug!(
						"Pausing new asset bakes because reserved or retained bake memory is {} and the configured budget is {}. Active bakes will continue so they can release memory.",
						utils::ByteSize::from(retained_bytes),
						utils::ByteSize::from(self.byte_limit)
					);
					reported_pause = true;
				}
				Err(_) => {}
			}
			let _ = release.listen().await;
		}
	}

	#[cfg(test)]
	fn retained_bytes(&self) -> usize {
		self.state.lock().retained_bytes
	}
}

/// The `MemoryLease` struct holds one bake tree of a [`BakeMemoryBudget`], so a root and its dependencies are admitted
/// once and charged together.
///
/// A root gets one from [`BakeMemoryBudget::acquire`]; each dependency it asks for clones it. The tree's charge returns
/// to the budget when the last lease is dropped.
pub(crate) struct MemoryLease<'b> {
	budget: &'b BakeMemoryBudget,
	tree: u32,
}

impl Clone for MemoryLease<'_> {
	fn clone(&self) -> Self {
		self.budget.state.lock().trees[self.tree as usize].leases += 1;
		Self {
			budget: self.budget,
			tree: self.tree,
		}
	}
}

impl<'b> MemoryLease<'b> {
	/// Returns this lease borrowing `budget` for its whole life, which must be the budget it was taken from.
	///
	/// A dependency queued on the workers outlives the borrow its parent's lease has; the manager reborrows its own
	/// budget for the workers' lifetime, and the tree's lease count carries over unchanged.
	pub(crate) fn rebind<'c>(self, budget: &'c BakeMemoryBudget) -> MemoryLease<'c> {
		debug_assert!(
			std::ptr::eq(self.budget, budget),
			"A memory lease was moved to another budget. The most likely cause is a lease passed between two asset managers."
		);
		let tree = self.tree;
		// The count this lease holds moves to the returned one, so this one must not give it back.
		std::mem::forget(self);
		MemoryLease { budget, tree }
	}

	/// Adds arena growth from one request and raises the shared charge above the initial reservation when needed.
	fn record_growth(&self, additional_bytes: usize) {
		let mut state = self.budget.state.lock();
		let tree = &mut state.trees[self.tree as usize];
		tree.observed_bytes = tree.observed_bytes.saturating_add(additional_bytes);
		if tree.observed_bytes <= tree.charged_bytes {
			return;
		}

		let additional_charge = tree.observed_bytes - tree.charged_bytes;
		tree.charged_bytes = tree.observed_bytes;
		state.retained_bytes = state.retained_bytes.saturating_add(additional_charge);
	}

	/// Returns a completed dependency arena's bytes while retaining this bake tree's admission reservation.
	fn release(&self, released_bytes: usize) {
		let reservation = self.budget.reservation();
		let mut state = self.budget.state.lock();
		let tree = &mut state.trees[self.tree as usize];
		tree.observed_bytes = tree.observed_bytes.saturating_sub(released_bytes);
		let new_charge = reservation.max(tree.observed_bytes);
		if new_charge >= tree.charged_bytes {
			return;
		}

		let released_charge = tree.charged_bytes - new_charge;
		tree.charged_bytes = new_charge;
		self.budget.return_bytes(&mut state, released_charge);
	}
}

impl Drop for MemoryLease<'_> {
	fn drop(&mut self) {
		let mut state = self.budget.state.lock();
		let tree = &mut state.trees[self.tree as usize];
		tree.leases -= 1;
		if tree.leases != 0 {
			return;
		}

		let charged_bytes = std::mem::take(tree).charged_bytes;
		state.free.push(self.tree);
		self.budget.return_bytes(&mut state, charged_bytes);
	}
}

/// The `BakeAllocator` struct gives one request an arena charged to its root bake's memory lease.
pub(super) struct BakeAllocator<'b> {
	arena: bumpalo::Bump,
	lease: Option<MemoryLease<'b>>,
	/// The arena size last charged to the lease. Atomic only so the allocator can be shared by reference; one bake
	/// allocates from it at a time.
	recorded_retained_bytes: AtomicUsize,
}

impl<'b> BakeAllocator<'b> {
	/// Creates a dependency arena in its parent's tree so waiting children cannot deadlock their parent.
	pub(super) fn in_tree(lease: Option<MemoryLease<'b>>) -> Self {
		Self {
			arena: bumpalo::Bump::new(),
			lease,
			recorded_retained_bytes: AtomicUsize::new(0),
		}
	}

	pub(super) fn lease(&self) -> Option<&MemoryLease<'b>> {
		self.lease.as_ref()
	}

	fn record_retained_bytes(&self) {
		let retained_bytes = self.arena.allocated_bytes_including_metadata();
		let previous_bytes = self.recorded_retained_bytes.load(Ordering::Relaxed);
		if retained_bytes <= previous_bytes {
			return;
		}
		self.recorded_retained_bytes.store(retained_bytes, Ordering::Relaxed);
		if let Some(lease) = &self.lease {
			lease.record_growth(retained_bytes - previous_bytes);
		}
	}
}

impl Drop for BakeAllocator<'_> {
	fn drop(&mut self) {
		let retained_bytes = *self.recorded_retained_bytes.get_mut();
		// Release the arena before returning its charge so newly admitted work does not overlap its physical allocation.
		drop(std::mem::replace(&mut self.arena, bumpalo::Bump::new()));
		if let Some(lease) = &self.lease {
			lease.release(retained_bytes);
		}
	}
}

#[allow(unsafe_code)]
// SAFETY: Every allocation is owned by `arena`, and the forwarded operations preserve the
// caller-provided `Allocator` layouts and pointer provenance. The scope only observes retained
// byte counts; it never reads, moves, or frees individual allocations.
unsafe impl Allocator for BakeAllocator<'_> {
	fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
		let allocation = Allocator::allocate(&&self.arena, layout)?;
		self.record_retained_bytes();
		Ok(allocation)
	}

	unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
		// SAFETY: The caller guarantees that `ptr` and `layout` identify a live allocation from this allocator.
		unsafe { Allocator::deallocate(&&self.arena, ptr, layout) };
	}

	unsafe fn grow(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<[u8]>, AllocError> {
		// SAFETY: The caller guarantees that `ptr` and `old` identify a live allocation from this allocator,
		// and the arena allocator enforces the requirements of `new`.
		let allocation = unsafe { Allocator::grow(&&self.arena, ptr, old, new) }?;
		self.record_retained_bytes();
		Ok(allocation)
	}

	unsafe fn shrink(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<[u8]>, AllocError> {
		// SAFETY: The caller guarantees that `ptr` and `old` identify a live allocation from this allocator,
		// and the arena allocator enforces the requirements of `new`.
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
		let budget = BakeMemoryBudget::new(BAKE_MEMORY_RESERVATION_BYTES * 2);
		let first = budget.try_acquire().unwrap();
		let second = budget.try_acquire().unwrap();

		assert!(budget.try_acquire().is_err());
		drop(first);
		let replacement = budget.try_acquire().unwrap();
		drop((second, replacement));

		assert_eq!(budget.retained_bytes(), 0);
	}

	#[test]
	fn allocator_charges_retained_arena_growth_and_releases_it_on_drop() {
		let budget = BakeMemoryBudget::new(1024);
		let lease = budget.try_acquire().unwrap();
		let allocator = BakeAllocator::in_tree(Some(lease.clone()));
		let mut data = Vec::<u8, _>::with_capacity_in(1, &allocator);
		data.resize(4096, 0);

		assert!(budget.retained_bytes() > 1024);
		drop(data);
		drop(allocator);

		assert_eq!(budget.retained_bytes(), 1024);
		drop(lease);

		assert_eq!(budget.retained_bytes(), 0);
	}

	#[test]
	fn a_tree_returns_its_charge_once_its_last_lease_is_dropped() {
		let budget = BakeMemoryBudget::new(BAKE_MEMORY_RESERVATION_BYTES);
		let root = budget.try_acquire().unwrap();
		let dependency = root.clone();

		drop(root);
		assert!(budget.try_acquire().is_err(), "the dependency still holds the tree");

		drop(dependency);
		assert_eq!(budget.retained_bytes(), 0);
		drop(budget.try_acquire().unwrap());
	}
}

use std::{
	alloc::{AllocError, Allocator, Layout},
	ptr::NonNull,
	sync::atomic::{AtomicUsize, Ordering},
};

use announcement::{Announcement, Announcer};
use utils::sync::Mutex;
