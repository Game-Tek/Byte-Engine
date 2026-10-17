//! Storage that the lanes of an [`Alley`](super::Alley) dispatch write side by side without locks.
//!
//! A stage that runs on every lane reaches its data through a shared reference, so the data has to say which lane
//! may write which part of it. The types here hold that rule and check it: [`LaneOwned`] gives every lane a slot of
//! its own, and [`LaneRanges`] gives every lane a range of one array, or the items that the lane's share of a batch
//! of [`PairBatches`](super::PairBatches) claims. Both are lent to one alley with `lend`, and both switch between
//! lanes writing their own part and everyone reading everything, which takes exclusive access and so happens in a
//! stage that runs on one lane, such as [`ConcreteLane::broadcast_mut`].

use std::{
	cell::{Cell, UnsafeCell},
	num::NonZeroU64,
	ops::{Deref, DerefMut, Range},
	sync::atomic::{AtomicU64, Ordering},
};

use utils::{StableVec, stable_vec::SlotsMut};

use super::{ConcreteLane, Lane as _, shared_range};

/// The `AlleyId` struct names one [`Alley`](super::Alley), so that storage lent to its lanes can tell them from the
/// lanes of any other alley.
///
/// Get it from [`Alley::id`](super::Alley::id) and pass it to [`LaneOwned::lend`] or [`LaneRanges::lend`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlleyId(NonZeroU64);

impl AlleyId {
	/// Returns an identifier that no other alley has.
	pub(super) fn next() -> Self {
		static NEXT: AtomicU64 = AtomicU64::new(1);
		let id = NEXT.fetch_add(1, Ordering::Relaxed);
		Self(NonZeroU64::new(id).expect("Alley identifiers ran out. The most likely cause is a counter that wrapped."))
	}
}

/// The `Padded` struct keeps a value on cache lines of its own, so that a lane that writes it does not slow down the
/// lane that writes the next one.
#[repr(align(128))]
pub(super) struct Padded<T>(pub(super) T);

/// Panics unless `lane` belongs to the alley that storage was lent to.
///
/// Two lanes of one alley never have the same index while both run, because an alley runs one dispatch at a time.
/// Lanes of different alleys do, so only one alley's lanes may reach storage that is split by lane index.
pub(super) fn check_lent(lent_to: Option<AlleyId>, lane: &ConcreteLane<'_>) {
	assert!(
		lent_to == Some(lane.alley()),
		"Lane storage is not lent to this alley. The most likely cause is a dispatch that started without a call to `lend` with the identifier of the alley it runs on."
	);
}

/// Marks a lane's part of some storage as taken, and panics if it already was.
pub(super) fn take(taken: &Cell<bool>) {
	assert!(
		!taken.replace(true),
		"Lane storage is already taken. The most likely cause is a lane that takes its part a second time while it still holds the first."
	);
}

/// The `LaneOwned` struct gives every lane of a dispatch a value of its own to write without a lock, which one lane
/// can then collect, or every lane read.
///
/// Use it for what lanes produce side by side, such as each lane's share of a result. Before a dispatch, call
/// [`Self::lend`] and [`Self::grow`]. In a stage that runs on every lane, each lane reaches its value with
/// [`Self::own`]. A stage that runs on one lane has exclusive access, and reads every value with
/// [`Self::iter_mut`]. To let every lane read every value, call [`Self::join`] in such a stage, and then
/// [`Self::all`] from the lanes, until [`Self::split`].
pub struct LaneOwned<S> {
	slots: Vec<Padded<Slot<S>>>,
	/// The alley whose lanes may take the slots.
	lent_to: Option<AlleyId>,
	/// Whether every lane may read every slot, and none may write one.
	joined: bool,
}

/// The `Slot` struct holds one lane's value.
struct Slot<S> {
	/// Whether the lane holds an [`Owned`] to the value. Only that lane reads or writes this.
	taken: Cell<bool>,
	value: UnsafeCell<S>,
}

// SAFETY: Through a shared reference, a slot is only reached by `own` and by `all`. `own` only reaches the slot of a
// lane of the one alley the storage is lent to, and no two running lanes of an alley share an index, so no two
// threads reach one slot. It refuses while the storage is joined, and `all` refuses unless it is, and changing that
// takes exclusive access, so no slot is read while a lane writes it. A lane's value moves to that lane's thread, so
// values have to be `Send`; `all` asks for `Sync` itself.
#[allow(unsafe_code, reason = "Lanes each reach their own slot.")]
unsafe impl<S: Send> Sync for LaneOwned<S> {}

impl<S> Default for LaneOwned<S> {
	fn default() -> Self {
		Self {
			slots: Vec::new(),
			lent_to: None,
			joined: false,
		}
	}
}

impl<S> LaneOwned<S> {
	/// Lets the lanes of `alley`, and no others, take the slots. Call it before a dispatch on that alley.
	pub fn lend(&mut self, alley: AlleyId) {
		self.lent_to = Some(alley);
	}

	/// Makes sure there is a slot for each of `lanes` lanes. Slots are only added, so each lane keeps the value it
	/// had.
	pub fn grow(&mut self, lanes: usize)
	where
		S: Default,
	{
		if self.slots.len() < lanes {
			self.slots.resize_with(lanes, || {
				Padded(Slot {
					taken: Cell::new(false),
					value: UnsafeCell::new(S::default()),
				})
			});
		}
	}

	/// Lets every lane read every slot with [`Self::all`], and keeps lanes from taking their own until
	/// [`Self::split`].
	pub fn join(&mut self) {
		self.joined = true;
	}

	/// Lets each lane take its own slot with [`Self::own`] again, after [`Self::join`].
	pub fn split(&mut self) {
		self.joined = false;
	}

	/// Returns every slot's value, in lane order. Use it in a stage that runs on one lane, to collect what the lanes
	/// left.
	pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = &mut S> {
		self.slots.iter_mut().map(|slot| slot.0.value.get_mut())
	}

	/// Returns this lane's value, for the lane to read and write until the returned guard is dropped.
	///
	/// # Panics
	///
	/// Panics if the slots are not lent to the lane's alley, if they are joined, if the lane has no slot, or if the
	/// lane still holds its value from an earlier call.
	pub fn own<'a>(&'a self, lane: &ConcreteLane<'_>) -> Owned<'a, S> {
		check_lent(self.lent_to, lane);
		assert!(
			!self.joined,
			"Lane storage is joined. The most likely cause is a lane that takes its own slot after `join`, without a `split` in a one-lane stage in between."
		);
		let Some(slot) = self.slots.get(lane.idx()) else {
			panic!(
				"Lane storage has no slot for lane {}. The most likely cause is a dispatch with more lanes than `grow` made slots for.",
				lane.idx()
			);
		};
		take(&slot.0.taken);
		Owned { slot: &slot.0 }
	}

	/// Returns every slot's value, in lane order, once [`Self::join`] let every lane read them.
	///
	/// # Panics
	///
	/// Panics unless the slots are joined.
	#[allow(unsafe_code, reason = "Joined slots are only read.")]
	pub fn all(&self) -> impl ExactSizeIterator<Item = &S> + Clone
	where
		S: Sync,
	{
		assert!(
			self.joined,
			"Lane storage is not joined. The most likely cause is reading every lane's slot before a one-lane stage called `join`."
		);
		// SAFETY: While the slots are joined, `own` hands out no value, and nothing else writes one through a shared
		// reference. Leaving that state takes exclusive access, which ends these references first.
		self.slots.iter().map(|slot| unsafe { &*slot.0.value.get() })
	}
}

/// The `Owned` struct lends a lane its value in a [`LaneOwned`] until it is dropped.
pub struct Owned<'a, S> {
	slot: &'a Slot<S>,
}

impl<S> Deref for Owned<'_, S> {
	type Target = S;

	#[allow(unsafe_code, reason = "The guard is the only access to its lane's slot.")]
	fn deref(&self) -> &S {
		// SAFETY: `LaneOwned::own` made this guard for the one lane that may reach the slot, and marked the slot
		// taken so that the lane gets no second guard.
		unsafe { &*self.slot.value.get() }
	}
}

impl<S> DerefMut for Owned<'_, S> {
	#[allow(unsafe_code, reason = "The guard is the only access to its lane's slot.")]
	fn deref_mut(&mut self) -> &mut S {
		// SAFETY: As for `deref`, and this borrows the guard exclusively.
		unsafe { &mut *self.slot.value.get() }
	}
}

impl<S> Drop for Owned<'_, S> {
	fn drop(&mut self) {
		self.slot.taken.set(false);
	}
}

/// The `SplitByRange` trait describes storage whose items different threads can change at the same time, as long as
/// each keeps to its own range of them. Implement it to let [`LaneRanges`] split that storage between lanes.
///
/// # Safety
///
/// Pieces of ranges that do not overlap must reach different memory, and none may reach what the storage's methods
/// that take a shared reference write. A piece of a range that reaches past [`Self::count`] must be cut short or
/// panic. What [`Self::reborrow`] returns must reach no more than the piece it was made from. If the storage is
/// `Sync`, its pieces must be sound to use from another thread than the one that owns it.
#[allow(unsafe_code, reason = "Implementors promise that pieces of different ranges are apart.")]
pub unsafe trait SplitByRange {
	/// What one thread gets of its range, such as a slice of it.
	type Piece<'a>
	where
		Self: 'a;

	/// Returns the number of items that ranges are taken of.
	fn count(&self) -> usize;

	/// Returns the items in `range`, for one thread to change.
	///
	/// # Safety
	///
	/// Until the last use of the piece, nothing else may reach the items in `range`: not another piece of a range
	/// that overlaps, and not any other method of the storage, on any thread.
	#[allow(unsafe_code, reason = "The caller keeps everything else from the range.")]
	unsafe fn piece(&self, range: Range<usize>) -> Self::Piece<'_>;

	/// Returns `piece` again, for as long as it is borrowed here. [`RangeOwned`] hands a lane its piece this way, so
	/// that the lane cannot keep it past the guard.
	fn reborrow<'piece>(piece: &'piece mut Self::Piece<'_>) -> Self::Piece<'piece>;
}

/// The `LaneCells` struct holds an array that lanes write side by side, each its own items. Put it in a
/// [`LaneRanges`] to give each lane a range of it.
///
/// It reads like a slice. Size it with [`Self::resize`].
pub struct LaneCells<T> {
	pub(super) items: Vec<UnsafeCell<T>>,
}

// SAFETY: A shared reference only reads the items, except through `SplitByRange::piece` and through what
// `LaneRanges` hands out of the items it wraps. A piece is unsafe to make, and its maker keeps every other access
// from the items it writes, as `LaneRanges` does for its own. Items move to other threads that way, so they have to
// be `Send` as well.
#[allow(unsafe_code, reason = "Writers through a shared reference keep to their own items.")]
unsafe impl<T: Send + Sync> Sync for LaneCells<T> {}

impl<T> Default for LaneCells<T> {
	fn default() -> Self {
		Self { items: Vec::new() }
	}
}

impl<T> LaneCells<T> {
	/// Makes the array `len` items long. Items are added as copies of `value` and removed from the end.
	pub fn resize(&mut self, len: usize, value: T)
	where
		T: Clone,
	{
		self.items.resize_with(len, || UnsafeCell::new(value.clone()));
	}
}

impl<T> Deref for LaneCells<T> {
	type Target = [T];

	#[allow(unsafe_code, reason = "Cells have the layout of their items.")]
	fn deref(&self) -> &[T] {
		// SAFETY: An `UnsafeCell<T>` is laid out like a `T`. Whoever writes an item through a shared reference
		// promised that nothing reads it meanwhile.
		unsafe { std::slice::from_raw_parts(self.items.as_ptr().cast::<T>(), self.items.len()) }
	}
}

impl<T> DerefMut for LaneCells<T> {
	#[allow(unsafe_code, reason = "Cells have the layout of their items.")]
	fn deref_mut(&mut self) -> &mut [T] {
		// SAFETY: An `UnsafeCell<T>` is laid out like a `T`, and this has the only reference to the items.
		unsafe { std::slice::from_raw_parts_mut(self.items.as_mut_ptr().cast::<T>(), self.items.len()) }
	}
}

// SAFETY: A piece is a slice of the range it was asked for, and slices of ranges that do not overlap are apart.
// Slicing past the end panics. No method writes through a shared reference.
#[allow(unsafe_code, reason = "Slices of ranges that do not overlap are apart.")]
unsafe impl<T> SplitByRange for LaneCells<T> {
	type Piece<'a>
		= &'a mut [T]
	where
		Self: 'a;

	fn count(&self) -> usize {
		self.items.len()
	}

	#[allow(unsafe_code, reason = "The caller keeps everything else from the range.")]
	unsafe fn piece(&self, range: Range<usize>) -> &mut [T] {
		let cells = &self.items[range];
		// SAFETY: The cells allow writing through a shared reference, and the caller keeps everything else from
		// them, so this is the only access to these items.
		unsafe { std::slice::from_raw_parts_mut(UnsafeCell::raw_get(cells.as_ptr()), cells.len()) }
	}

	fn reborrow<'piece>(piece: &'piece mut &mut [T]) -> &'piece mut [T] {
		piece
	}
}

// SAFETY: A piece yields the values of the slots in its range, which are apart from those of other slots, and
// leaves out the slots past the last one. `StableVec` writes nothing through a shared reference.
#[allow(unsafe_code, reason = "The values of different slots are apart.")]
unsafe impl<T> SplitByRange for StableVec<T> {
	type Piece<'a>
		= SlotsMut<'a, T>
	where
		Self: 'a;

	fn count(&self) -> usize {
		self.slots_len()
	}

	#[allow(unsafe_code, reason = "The caller keeps everything else from the range.")]
	unsafe fn piece(&self, range: Range<usize>) -> SlotsMut<'_, T> {
		// SAFETY: The caller keeps everything else from the values in these slots.
		unsafe { self.slots_mut_unchecked(range) }
	}

	fn reborrow<'piece>(piece: &'piece mut SlotsMut<'_, T>) -> SlotsMut<'piece, T> {
		piece.reborrow()
	}
}

/// How a [`LaneRanges`] is split between lanes.
pub(super) enum RangeSplit {
	/// Every lane may read every item, and none may write one.
	Joined,
	/// Each of this many lanes has a balanced range, as [`ConcreteLane::share_range`] gives.
	Even(usize),
	/// Each lane's range starts at its entry of `starts` and ends at the next one.
	At,
	/// Each lane has the items that its groups of one batch claim, in the batches that had this stamp when the items
	/// were split by them. See [`PairBatches`](super::PairBatches).
	Batches(u64),
}

/// The `LaneRanges` struct gives every lane of a dispatch a range of one array to write without a lock, while it
/// keeps the ranges apart.
///
/// Use it for work that every lane does to its own stretch of some items. Before a dispatch, call [`Self::lend`].
/// With exclusive access, as a stage that runs on one lane has, call [`Self::split`] to give lanes balanced ranges
/// or [`Self::split_at`] to choose where each starts. In a stage that runs on every lane, each lane reaches its
/// range with [`Self::own`]. Call [`Self::join`] to let every lane read every item with [`Self::all`] instead,
/// which is also how new storage starts.
pub struct LaneRanges<C> {
	pub(super) items: C,
	/// The alley whose lanes may take ranges.
	pub(super) lent_to: Option<AlleyId>,
	pub(super) split: RangeSplit,
	/// Where each lane's range starts, followed by where the last one ends, for [`RangeSplit::At`].
	starts: Vec<usize>,
	/// For each lane, whether it holds its part of the items. Only that lane reads or writes its entry.
	pub(super) taken: Vec<Padded<Cell<bool>>>,
}

// SAFETY: Through a shared reference, the items are only reached by `own` and by `all`. `own` only gives a lane of
// the one alley the storage is lent to its own range, no two running lanes of an alley share an index, and the
// ranges of different lanes do not overlap, so no two threads reach one item. It refuses while the storage is
// joined, and `all` refuses unless it is, and changing that takes exclusive access, so no item is read while a lane
// writes it. A lane's entry of `taken` is only reached by that lane. `SplitByRange` makes pieces of `Sync` storage
// sound to use from other threads.
#[allow(unsafe_code, reason = "Lanes each reach their own range.")]
unsafe impl<C: SplitByRange + Sync> Sync for LaneRanges<C> {}

impl<C: Default> Default for LaneRanges<C> {
	fn default() -> Self {
		Self::new(C::default())
	}
}

impl<C> LaneRanges<C> {
	/// Wraps `items`, which every lane may read until they are split. Call [`Self::lend`] next.
	pub fn new(items: C) -> Self {
		Self {
			items,
			lent_to: None,
			split: RangeSplit::Joined,
			starts: Vec::new(),
			taken: Vec::new(),
		}
	}

	/// Lets the lanes of `alley`, and no others, take ranges. Call it before a dispatch on that alley.
	pub fn lend(&mut self, alley: AlleyId) {
		self.lent_to = Some(alley);
	}

	/// Gives each of `lanes` lanes a balanced range of the items to take with [`Self::own`], and keeps lanes from
	/// reading the others' until [`Self::join`].
	///
	/// The ranges follow the number of items as it is when a lane takes its range, so items can be added or removed
	/// in between.
	pub fn split(&mut self, lanes: usize) {
		self.split = RangeSplit::Even(lanes);
		self.track(lanes);
	}

	/// Gives each lane the range from its entry of `starts` to the next one, to take with [`Self::own`], and keeps
	/// lanes from reading the others' until [`Self::join`]. `starts` has one entry for each lane and then the end of
	/// the last lane's range.
	///
	/// # Panics
	///
	/// Panics if `starts` do not ascend.
	pub fn split_at(&mut self, starts: impl IntoIterator<Item = usize>) {
		self.starts.clear();
		self.starts.extend(starts);
		assert!(
			self.starts.is_sorted(),
			"Lane ranges do not ascend. The most likely cause is split points that were worked out for other items than these."
		);
		self.split = RangeSplit::At;
		self.track(self.starts.len().saturating_sub(1));
	}

	/// Lets every lane read every item with [`Self::all`], and keeps lanes from taking ranges until the items are
	/// split again.
	pub fn join(&mut self) {
		self.split = RangeSplit::Joined;
	}

	/// Returns the items, to change them however they are split.
	pub fn get_mut(&mut self) -> &mut C {
		&mut self.items
	}

	/// Returns the items, once [`Self::join`] let every lane read them.
	///
	/// # Panics
	///
	/// Panics while the items are split.
	pub fn all(&self) -> &C {
		assert!(
			matches!(self.split, RangeSplit::Joined),
			"Lane ranges are not joined. The most likely cause is reading every item after `split`, before a one-lane stage called `join`."
		);
		&self.items
	}

	/// Makes sure each of `lanes` lanes has an entry that tells whether it holds its part of the items.
	pub(super) fn track(&mut self, lanes: usize) {
		if self.taken.len() < lanes {
			self.taken.resize_with(lanes, || Padded(Cell::new(false)));
		}
	}
}

impl<C: SplitByRange> LaneRanges<C> {
	/// Returns this lane's range of the items, for the lane to read and write until the returned guard is dropped.
	///
	/// # Panics
	///
	/// Panics if the items are not lent to the lane's alley, if they are joined, if they are split between fewer
	/// lanes than this one's index, or if the lane still holds its range from an earlier call.
	#[allow(unsafe_code, reason = "Lanes of one alley take ranges that do not overlap.")]
	pub fn own<'a>(&'a self, lane: &ConcreteLane<'_>) -> RangeOwned<'a, C> {
		check_lent(self.lent_to, lane);
		let index = lane.idx();
		let range = match self.split {
			RangeSplit::Joined => panic!(
				"Lane ranges are joined. The most likely cause is a lane that takes its range after `join`, without a `split` in a one-lane stage in between."
			),
			RangeSplit::Even(lanes) if index < lanes => shared_range(self.items.count(), index, lanes),
			RangeSplit::At if index + 1 < self.starts.len() => self.starts[index]..self.starts[index + 1],
			RangeSplit::Batches(_) => panic!(
				"Lane ranges are split by batch. The most likely cause is a lane that takes a range of items that a one-lane stage split with `split_by`."
			),
			_ => panic!(
				"Lane ranges have no range for lane {index}. The most likely cause is a dispatch with more lanes than the items were split between."
			),
		};
		let taken = &self.taken[index].0;
		take(taken);
		// SAFETY: The lane belongs to the one alley these items are lent to, and no two running lanes of an alley
		// share an index. The ranges of different indices do not overlap: balanced ranges never do, and the other
		// ones follow starts that ascend. `taken` keeps this lane from a second piece of its range until the guard
		// that holds this one is dropped, and nothing reads the items while they are split.
		let piece = unsafe { self.items.piece(range.clone()) };
		RangeOwned { piece, range, taken }
	}
}

/// The `RangeOwned` struct lends a lane its range of a [`LaneRanges`] until it is dropped.
///
/// [`Self::items`] returns what the storage gives of a range: an iterator over slots and values for a
/// [`StableVec`], and a slice for [`LaneCells`], which this also reads like. [`Self::range`] tells which items
/// those are.
pub struct RangeOwned<'a, C: SplitByRange + 'a> {
	/// The lane's items. They are only handed out for as long as this guard is borrowed, so that none outlives it.
	piece: C::Piece<'a>,
	range: Range<usize>,
	taken: &'a Cell<bool>,
}

impl<C: SplitByRange> RangeOwned<'_, C> {
	/// Returns the range of items this lane has.
	pub fn range(&self) -> Range<usize> {
		self.range.clone()
	}

	/// Returns this lane's items, to read and write while this guard is borrowed.
	pub fn items(&mut self) -> C::Piece<'_> {
		C::reborrow(&mut self.piece)
	}
}

impl<T> Deref for RangeOwned<'_, LaneCells<T>> {
	type Target = [T];

	fn deref(&self) -> &[T] {
		self.piece
	}
}

impl<T> DerefMut for RangeOwned<'_, LaneCells<T>> {
	fn deref_mut(&mut self) -> &mut [T] {
		self.piece
	}
}

impl<'a, C: SplitByRange + 'a> Drop for RangeOwned<'a, C> {
	fn drop(&mut self) {
		self.taken.set(false);
	}
}

#[cfg(test)]
pub(super) mod tests {
	use std::panic::resume_unwind;

	use utils::StableVec;

	use super::{LaneCells, LaneOwned, LaneRanges};
	use crate::core::alley::{Alley, ConcreteLane, Lane as _, LanePhased};

	/// Returns what a panic said.
	pub(in crate::core::alley) fn message(panic: Box<dyn std::any::Any + Send>) -> String {
		match panic.downcast::<String>() {
			Ok(message) => *message,
			Err(panic) => panic
				.downcast::<&str>()
				.map_or_else(|_| String::new(), |message| message.to_string()),
		}
	}

	/// Runs `stages` on every lane of `alley` with `storage`, and returns what it panicked with, if it did.
	fn dispatch<T: Send + Sync>(
		alley: &mut Alley,
		storage: &mut T,
		stages: impl Fn(&mut ConcreteLane<'_>, &LanePhased<'_, T>) + Sync,
	) -> Option<String> {
		let lanes = alley.parallelism();
		alley.execute_phased(storage, lanes, stages).err().map(message)
	}

	#[test]
	fn lanes_write_their_own_slots_and_one_lane_collects_them() {
		let mut alley = Alley::with_parallelism(4);
		let mut slots = LaneOwned::<Vec<usize>>::default();
		slots.lend(alley.id());
		slots.grow(4);

		let failed = dispatch(&mut alley, &mut slots, |lane, slots| {
			for round in 0..3 {
				let _ = lane.each_ref(slots, |slots, lane| slots.own(lane).push(lane.idx() * 10 + round));
			}
		});

		assert_eq!(failed, None);
		let collected: Vec<Vec<usize>> = slots.iter_mut().map(|slot| slot.clone()).collect();
		assert_eq!(collected, [[0, 1, 2], [10, 11, 12], [20, 21, 22], [30, 31, 32]]);
	}

	#[test]
	fn joined_slots_are_read_by_every_lane_until_they_are_split() {
		let mut alley = Alley::with_parallelism(3);
		let mut slots = LaneOwned::<usize>::default();
		slots.lend(alley.id());
		slots.grow(3);

		let sums = alley.execute_phased(&mut slots, 3, |lane, slots| {
			let _ = lane.each_ref(slots, |slots, lane| *slots.own(lane) = lane.idx() + 1);
			lane.broadcast_mut(slots, |slots| slots.join());
			let sums = lane.each_ref(slots, |slots, _| slots.all().sum::<usize>());
			lane.broadcast_mut(slots, |slots| slots.split());
			let _ = lane.each_ref(slots, |slots, lane| *slots.own(lane) += 10);
			sums.sum::<usize>()
		});

		assert_eq!(sums.unwrap_or_else(|panic| resume_unwind(panic)), [18, 18, 18]);
		assert_eq!(slots.iter_mut().map(|slot| *slot).collect::<Vec<_>>(), [11, 12, 13]);
	}

	/// Runs `stages` on one lane of `alley` with `storage`, and returns what it panicked with, if it did. One lane
	/// runs on the calling thread, so what comes back is what that lane panicked with.
	fn refusal<T: Send + Sync>(
		alley: &mut Alley,
		storage: &mut T,
		stages: impl Fn(&mut ConcreteLane<'_>, &LanePhased<'_, T>) + Sync,
	) -> Option<String> {
		alley.execute_phased(storage, 1, stages).err().map(message)
	}

	#[test]
	fn slots_refuse_lanes_they_are_not_split_for() {
		let mut alley = Alley::with_parallelism(2);
		let mut other_alley = Alley::with_parallelism(2);
		let mut slots = LaneOwned::<usize>::default();
		let take = |lane: &mut ConcreteLane<'_>, slots: &LanePhased<'_, _>| {
			let _ = lane.each_ref(slots, |slots: &LaneOwned<usize>, lane| *slots.own(lane) += 1);
		};
		let refused = |refusal: Option<String>, with: &str| refusal.is_some_and(|message| message.starts_with(with));

		// Not lent at all, and then lent to another alley.
		assert!(refused(
			refusal(&mut alley, &mut slots, take),
			"Lane storage is not lent to this alley."
		));
		slots.lend(other_alley.id());
		assert!(refused(
			refusal(&mut alley, &mut slots, take),
			"Lane storage is not lent to this alley."
		));

		// A lane has no slot before there is one for it.
		assert!(refused(
			refusal(&mut other_alley, &mut slots, take),
			"Lane storage has no slot for lane 0."
		));
		slots.grow(2);
		assert_eq!(dispatch(&mut other_alley, &mut slots, take), None);

		// Joined slots are only read, and split ones are only taken.
		slots.join();
		assert!(refused(
			refusal(&mut other_alley, &mut slots, take),
			"Lane storage is joined."
		));
		slots.split();
		let read = refusal(&mut other_alley, &mut slots, |lane, slots| {
			let _ = lane.each_ref(slots, |slots, _| slots.all().count());
		});
		assert!(refused(read, "Lane storage is not joined."));

		// A lane holds its slot once.
		let twice = refusal(&mut other_alley, &mut slots, |lane, slots| {
			let _ = lane.each_ref(slots, |slots, lane| {
				let held = slots.own(lane);
				*held + *slots.own(lane)
			});
		});
		assert!(refused(twice, "Lane storage is already taken."));

		// Only the one dispatch that was let through changed the slots.
		assert_eq!(slots.iter_mut().map(|slot| *slot).collect::<Vec<_>>(), [1, 1]);
	}

	#[test]
	fn lanes_write_balanced_ranges_that_cover_every_item_once() {
		let mut alley = Alley::with_parallelism(4);
		let mut items = LaneRanges::<LaneCells<usize>>::default();
		items.get_mut().resize(10, 0);
		items.lend(alley.id());
		items.split(4);

		let failed = dispatch(&mut alley, &mut items, |lane, items| {
			let _ = lane.each_ref(items, |items, lane| {
				let mut own = items.own(lane);
				let first = own.range().start;
				for (place, item) in own.iter_mut().enumerate() {
					*item += (lane.idx() + 1) * 100 + first + place;
				}
			});
			lane.broadcast_mut(items, |items| items.join());
			// Every lane reads every item, also the ones other lanes wrote.
			let _ = lane.each_ref(items, |items, _| assert_eq!(items.all().len(), 10));
		});

		assert_eq!(failed, None);
		assert_eq!(items.all()[..], [100, 101, 102, 203, 204, 205, 306, 307, 408, 409]);
	}

	#[test]
	fn lanes_write_the_ranges_they_are_given() {
		let mut alley = Alley::with_parallelism(3);
		let mut items = LaneRanges::<LaneCells<usize>>::default();
		items.get_mut().resize(8, 0);
		items.lend(alley.id());
		// The last lane has nothing, and the last two items belong to no lane.
		items.split_at([0, 1, 6, 6]);

		let failed = dispatch(&mut alley, &mut items, |lane, items| {
			let _ = lane.each_ref(items, |items, lane| items.own(lane).fill(lane.idx() + 1));
		});

		assert_eq!(failed, None);
		items.join();
		assert_eq!(items.all()[..], [1, 2, 2, 2, 2, 2, 0, 0]);
	}

	#[test]
	fn ranges_refuse_lanes_they_are_not_split_for() {
		let mut alley = Alley::with_parallelism(2);
		let mut items = LaneRanges::<LaneCells<usize>>::default();
		items.get_mut().resize(4, 0);
		let take = |lane: &mut ConcreteLane<'_>, items: &LanePhased<'_, _>| {
			let _ = lane.each_ref(items, |items: &LaneRanges<LaneCells<usize>>, lane| items.own(lane).fill(1));
		};
		let refused = |refusal: Option<String>, with: &str| refusal.is_some_and(|message| message.starts_with(with));

		items.split(2);
		assert!(refused(
			refusal(&mut alley, &mut items, take),
			"Lane storage is not lent to this alley."
		));
		items.lend(alley.id());
		items.join();
		assert!(refused(refusal(&mut alley, &mut items, take), "Lane ranges are joined."));
		items.split(0);
		assert!(refused(
			refusal(&mut alley, &mut items, take),
			"Lane ranges have no range for lane 0."
		));
		let read = refusal(&mut alley, &mut items, |lane, items| {
			let _ = lane.each_ref(items, |items, _| items.all().len());
		});
		assert!(refused(read, "Lane ranges are not joined."));
		items.split(2);
		let twice = refusal(&mut alley, &mut items, |lane, items| {
			let _ = lane.each_ref(items, |items, lane| {
				let held = items.own(lane);
				held.len() + items.own(lane).len()
			});
		});
		assert!(refused(twice, "Lane storage is already taken."));

		// No refused lane wrote an item.
		items.join();
		assert_eq!(items.all()[..], [0; 4]);
	}

	#[test]
	#[should_panic(expected = "Lane ranges do not ascend.")]
	fn ranges_that_do_not_ascend_are_refused() {
		let mut items = LaneRanges::<LaneCells<usize>>::default();
		items.get_mut().resize(4, 0);
		items.split_at([0, 3, 2, 4]);
	}

	#[test]
	fn lanes_change_the_values_in_their_ranges_of_a_stable_vec() {
		let mut alley = Alley::with_parallelism(3);
		let mut values = LaneRanges::new((0..7usize).collect::<StableVec<usize>>());
		let removed = values.get_mut().handled_iter().nth(4).map(|(handle, _)| handle).unwrap();
		values.get_mut().remove(removed);
		values.lend(alley.id());
		values.split(3);

		let failed = dispatch(&mut alley, &mut values, |lane, values| {
			let _ = lane.each_ref(values, |values, lane| {
				for (slot, value) in values.own(lane).items() {
					*value = slot * 10 + lane.idx();
				}
			});
		});

		assert_eq!(failed, None);
		values.join();
		// Seven slots split as three, two, and two. The removed one is left out.
		let changed: Vec<(usize, usize)> = values.all().indexed_iter().map(|(slot, value)| (slot, *value)).collect();
		assert_eq!(changed, [(0, 0), (1, 10), (2, 20), (3, 31), (5, 52), (6, 62)]);
	}
}
