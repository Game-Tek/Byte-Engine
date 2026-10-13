//! Batches of items that lanes work through side by side, because no two items of a batch write the same entry.
//!
//! Some work reads and writes a few entries of an array for each of many items, where items share entries: every
//! contact between two bodies changes both, and a body is in several contacts. Lanes cannot split such items by
//! range. [`PairBatches`] sorts them into batches in which every entry is written by at most one item, so that
//! lanes can share a batch, and hands each group of a batch's items a [`Claim`] on its entries.
//! [`LaneRanges::batch`] then lets a lane reach exactly the entries its claims name.

use std::{
	cell::Cell,
	sync::atomic::{AtomicU64, Ordering},
};

use super::{
	AlleyId, Lane as _, Stage, shared_range,
	split::{LaneCells, LaneRanges, RangeSplit, check_lent, take},
};

/// The `PairBatches` struct sorts items that each write up to two entries of an array into batches in which no
/// entry is written twice, so that the items of one batch can be worked through in any order, or by several lanes
/// at the same time, with the same result.
///
/// Call [`Self::lend`] before a dispatch. In a stage that runs on one lane, call [`Self::rebuild`] with the entries
/// of every item, and split the arrays the items write with [`LaneRanges::split_by`]. Then run one stage for each
/// batch, in order, with [`ConcreteLane::each_ref_keyed`](super::ConcreteLane::each_ref_keyed) and the batch as the
/// key: each lane takes its groups of the batch with [`Self::share`], and reaches their entries through
/// [`LaneRanges::batch`]. Work through [`Self::leftover_groups`] on one lane after every batch.
#[derive(Default)]
pub struct PairBatches {
	/// Item indices, grouped by batch.
	order: Vec<u32>,
	/// Where each batch ends in `order`.
	ends: Vec<usize>,
	/// The items that fit in no batch, in the order they were given.
	leftover: Vec<u32>,
	/// The batch each item was given, kept between rebuilds so batching does not allocate.
	assigned: Vec<u8>,
	/// For each entry, the batches that already hold an item that writes it, one bit per batch.
	taken: Vec<u64>,
	/// The two entries of each item, as they were given.
	pairs: Vec<[u32; 2]>,
	/// The entries that items only read, as they were given, each at least once. Working storage of
	/// [`Self::rebuild`].
	fixed: Vec<u32>,
	/// Tells the batches of one rebuild from those of any other, here or in other batches.
	stamp: u64,
	/// The alley whose lanes may take groups.
	lent_to: Option<AlleyId>,
}

/// Returns a stamp that no rebuild has had.
fn next_stamp() -> u64 {
	static NEXT: AtomicU64 = AtomicU64::new(1);
	NEXT.fetch_add(1, Ordering::Relaxed)
}

impl PairBatches {
	/// Marks an entry of an item as one the item only reads. Any number of a batch's items can read an entry that
	/// none writes, such as a body that contacts never move.
	pub const FIXED: u32 = 1 << 31;

	/// The most batches there can be, one for each bit of an entry's record of the batches that write it.
	pub const LIMIT: usize = u64::BITS as usize;

	/// Lets the lanes of `alley`, and no others, take groups with [`Self::share`]. Call it before a dispatch on
	/// that alley.
	pub fn lend(&mut self, alley: AlleyId) {
		self.lent_to = Some(alley);
	}

	/// Replaces the batches with ones for the items in `runs`, taken one run after the other. Each item is given
	/// as its two entries of an array of `entries` entries, with [`Self::FIXED`] added to an entry it only reads.
	///
	/// Each item joins the first batch that holds no item that writes one of its entries, so the batches depend
	/// only on the order of the items.
	///
	/// # Panics
	///
	/// Panics if the array is longer than [`Self::FIXED`], if an item writes an entry past its end, or if one item
	/// reads an entry as fixed that another one writes. No lane can take groups of the batches after that until they
	/// are rebuilt.
	pub fn rebuild<'a>(&mut self, entries: usize, runs: impl Iterator<Item = &'a [[u32; 2]]>) {
		assert!(
			entries <= Self::FIXED as usize,
			"Batched array is too long. The most likely cause is an array with more entries than the 31 bits an item names one with."
		);
		// Groups are only taken of batches that are complete.
		self.ends.clear();
		self.stamp = next_stamp();
		// One more entry stands in for every entry that items only read. It stays empty, so those never fill a
		// batch.
		self.taken.clear();
		self.taken.resize(entries + 1, 0);
		self.assigned.clear();
		self.leftover.clear();
		self.pairs.clear();
		self.fixed.clear();
		let mut sizes = [0usize; Self::LIMIT];
		// Whether an item writes an entry past the array's end.
		let mut past_end = false;
		// The fixed entry that was noted last. An entry that many items read, such as a floor, comes many times in a
		// row, and is noted once.
		let mut noted = 0;
		for run in runs {
			self.assigned.reserve(run.len());
			self.pairs.extend_from_slice(run);
			// The items of one entry with the entries after it follow each other. Its batches are kept at hand
			// while they do, because reading them back for every item makes each wait for the one before it.
			let (mut held, mut held_taken) = (entries, 0);
			for pair in run {
				for entry in *pair {
					if entry & Self::FIXED == 0 {
						past_end |= entry as usize >= entries;
					} else if entry != noted {
						self.fixed.push(entry);
						noted = entry;
					}
				}
				// A fixed entry is above every entry of the array.
				let [first, second] = pair.map(|entry| (entry as usize).min(entries));
				if first != held {
					self.taken[held] = held_taken;
					(held, held_taken) = (first, self.taken[first]);
				}
				let taken = held_taken | self.taken[second];
				let batch = (!taken).trailing_zeros() as usize;
				if batch == Self::LIMIT {
					// One of the entries is already written in every batch.
					self.leftover.push(self.assigned.len() as u32);
				} else {
					if first != entries {
						held_taken |= 1 << batch;
					}
					if second != entries {
						self.taken[second] |= 1 << batch;
					}
					sizes[batch] += 1;
				}
				self.assigned.push(batch as u8);
			}
			self.taken[held] = held_taken;
		}

		// An item only opens a batch when its entries fill every earlier one, so the used batches are the first ones.
		let mut starts = [0usize; Self::LIMIT];
		let mut ends = [0usize; Self::LIMIT];
		let (mut batches, mut end) = (0, 0);
		for (batch, size) in sizes.into_iter().enumerate().take_while(|(_, size)| *size != 0) {
			starts[batch] = end;
			end += size;
			ends[batch] = end;
			batches = batch + 1;
		}
		self.order.clear();
		self.order.resize(end, 0);
		for (item, batch) in self.assigned.iter().map(|batch| usize::from(*batch)).enumerate() {
			if batch != Self::LIMIT {
				self.order[starts[batch]] = item as u32;
				starts[batch] += 1;
			}
		}

		// Lanes write what an item writes while others read what theirs only read, so no entry may be both, and the
		// batches only hold when every written entry is one of the array's.
		assert!(
			!past_end,
			"Batched item writes an entry past the end. The most likely cause is an item that names an entry of a longer array than the one the batches were rebuilt for."
		);
		let written = |entry: &u32| {
			let taken = self.taken.get((entry & !Self::FIXED) as usize);
			taken.is_some_and(|taken| *taken != 0)
		};
		assert!(
			!self.fixed.iter().any(written),
			"Batched item reads an entry as fixed that another item writes. The most likely cause is an entry that only some of its items mark with `PairBatches::FIXED`."
		);
		self.ends.extend_from_slice(&ends[..batches]);
	}

	/// Returns the number of batches. Batches fill from the first one, so the large ones lead.
	pub fn len(&self) -> usize {
		self.ends.len()
	}

	/// Returns whether there are no batches.
	pub fn is_empty(&self) -> bool {
		self.ends.is_empty()
	}

	/// Returns the items of one batch, in the order they were given to [`Self::rebuild`].
	///
	/// # Panics
	///
	/// Panics if there is no such batch.
	pub fn batch(&self, batch: usize) -> &[u32] {
		let Some(end) = self.ends.get(batch).copied() else {
			panic!(
				"Batch {batch} does not exist. The most likely cause is a stage whose key is not a batch of the last rebuild."
			);
		};
		let start = if batch == 0 { 0 } else { self.ends[batch - 1] };
		&self.order[start..end]
	}

	/// Returns the items that fit in no batch, in the order they were given to [`Self::rebuild`].
	///
	/// Work through them one at a time after every batch, because they share entries with each other.
	pub fn leftover(&self) -> &[u32] {
		&self.leftover
	}

	/// Returns this lane's share of the groups of the batch that the stage's key names, each as a claim on the
	/// entries of up to `W` items and the items themselves.
	///
	/// The claims let this lane, in a stage with this key, read and write those entries through
	/// [`LaneRanges::batch`], for as long as the batches are not rebuilt.
	///
	/// # Panics
	///
	/// Panics if the batches are not lent to the lane's alley, or if the stage's key is not a batch.
	pub fn share<'a, const W: usize>(&'a self, stage: &Stage<'_, '_>) -> impl Iterator<Item = (Claim<W>, &'a [u32])> + 'a {
		let lane = stage.lane();
		check_lent(self.lent_to, lane);
		let items = self.batch(stage.key() as usize);
		let (index, lanes) = (lane.idx(), lane.state.lane_count);
		let owner = ClaimOwner {
			stamp: self.stamp,
			batch: stage.key(),
			lane: narrow(index),
			lanes: narrow(lanes),
		};
		let groups = shared_range(items.len().div_ceil(W), index, lanes);
		self.groups(
			&items[(groups.start * W).min(items.len())..(groups.end * W).min(items.len())],
			owner,
		)
	}

	/// Returns every group of `batch`, like [`Self::share`], for one thread with exclusive access to the arrays
	/// to work through. Use the claims with [`LaneRanges::batch_mut`].
	///
	/// # Panics
	///
	/// Panics if there is no such batch.
	pub fn batch_groups<const W: usize>(&self, batch: usize) -> impl Iterator<Item = (Claim<W>, &[u32])> + '_ {
		self.groups(self.batch(batch), ClaimOwner::NO_LANE)
	}

	/// Returns each item that fit in no batch as a group of its own, like [`Self::batch_groups`].
	pub fn leftover_groups<const W: usize>(&self) -> impl Iterator<Item = (Claim<W>, &[u32])> + '_ {
		self.leftover
			.chunks(1)
			.map(|item| (self.claim(item, ClaimOwner::NO_LANE), item))
	}

	/// Returns `items` in groups of up to `W`, each with a claim on its entries for `owner`.
	fn groups<'a, const W: usize>(
		&'a self,
		items: &'a [u32],
		owner: ClaimOwner,
	) -> impl Iterator<Item = (Claim<W>, &'a [u32])> + 'a {
		items.chunks(W).map(move |group| (self.claim(group, owner), group))
	}

	/// Returns a claim on the entries of `group`, which holds up to `W` items, for `owner`.
	fn claim<const W: usize>(&self, group: &[u32], owner: ClaimOwner) -> Claim<W> {
		debug_assert!(
			!group.is_empty() && group.len() <= W,
			"Claimed group size is invalid. The most likely cause is a batch that was not split into groups of {W}."
		);
		let mut claim = Claim {
			entries: [[0; W]; 2],
			writable: [0; 2],
			owner,
		};
		for place in 0..W {
			// The places past the last item repeat it, so that reading a whole group reads real entries. Nothing
			// is written for them.
			let held = place < group.len();
			let pair = self.pairs[group[place.min(group.len() - 1)] as usize];
			for side in 0..2 {
				claim.entries[side][place] = pair[side] & !Self::FIXED;
				claim.writable[side] |= u8::from(held && pair[side] & Self::FIXED == 0) << place;
			}
		}
		claim
	}
}

/// Returns a lane index or count as a claim keeps it.
fn narrow(lanes: usize) -> u16 {
	u16::try_from(lanes).expect("Lane count is too large. The most likely cause is an alley with more than 65,535 lanes.")
}

/// The `ClaimOwner` struct names the lane and stage a [`Claim`] was made for: which rebuild's batch, and which lane
/// of how many.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ClaimOwner {
	stamp: u64,
	batch: u32,
	lane: u16,
	lanes: u16,
}

impl ClaimOwner {
	/// The owner of claims that no lane stage may use, only a thread with exclusive access.
	const NO_LANE: Self = Self {
		stamp: 0,
		batch: u32::MAX,
		lane: u16::MAX,
		lanes: 0,
	};
}

/// The `Claim` struct names the entries that a group of up to `W` batched items reads and writes, two for each item,
/// and proves that the lane it was made for may reach them while its batch is worked through.
///
/// [`PairBatches::share`] makes one for each group. Keep it with what is worked out for the group, and pass it to
/// [`BatchOwned::get`] and [`BatchOwned::write`] on every pass over the batch.
pub struct Claim<const W: usize> {
	/// The entries of each item, for each of an item's two sides.
	entries: [[u32; W]; 2],
	/// The places whose entry the group writes, one bit per place, for each side.
	writable: [u8; 2],
	owner: ClaimOwner,
}

impl<const W: usize> Claim<W> {
	/// Returns the entry on `side` of the item at `place`.
	pub fn entry(&self, side: usize, place: usize) -> usize {
		self.entries[side][place] as usize
	}
}

impl<T> LaneRanges<LaneCells<T>> {
	/// Gives each of `lanes` lanes the items that its groups of a batch of `batches` claim, to reach with
	/// [`Self::batch`] in a stage for that batch, and keeps lanes from every other access until the items are split
	/// again or joined.
	///
	/// The split holds for the batches as they are now: claims of a later rebuild are refused.
	pub fn split_by(&mut self, batches: &PairBatches, lanes: usize) {
		self.split = RangeSplit::Batches(batches.stamp);
		self.track(lanes);
	}

	/// Returns access to the items that this lane's claims of the stage's batch name, until the returned guard is
	/// dropped. The guard cannot be kept past the stage.
	///
	/// # Panics
	///
	/// Panics if the items are not lent to the lane's alley, if they are not split by batch, or if the lane still
	/// holds a part of the items from an earlier call.
	pub fn batch<'a>(&'a self, stage: &'a Stage<'_, '_>) -> BatchOwned<'a, T> {
		let lane = stage.lane();
		check_lent(self.lent_to, lane);
		let RangeSplit::Batches(stamp) = self.split else {
			panic!(
				"Lane ranges are not split by batch. The most likely cause is a batch stage that runs before a one-lane stage called `split_by`."
			);
		};
		let Some(taken) = self.taken.get(lane.idx()) else {
			panic!(
				"Lane ranges have no entry for lane {}. The most likely cause is a dispatch with more lanes than the items were split between.",
				lane.idx()
			);
		};
		take(&taken.0);
		BatchOwned {
			cells: &self.items,
			owner: Some(ClaimOwner {
				stamp,
				batch: stage.key(),
				lane: narrow(lane.idx()),
				lanes: narrow(lane.state.lane_count),
			}),
			taken: Some(&taken.0),
		}
	}

	/// Returns access to the items that any claim names, for one thread to work through groups that lanes cannot
	/// share, such as [`PairBatches::leftover_groups`].
	pub fn batch_mut(&mut self) -> BatchOwned<'_, T> {
		BatchOwned {
			cells: &self.items,
			owner: None,
			taken: None,
		}
	}
}

/// The `BatchOwned` struct lets one lane reach the items that its claims name, while every lane works through the
/// same batch. [`LaneRanges::batch`] returns it.
pub struct BatchOwned<'a, T> {
	cells: &'a LaneCells<T>,
	/// Whose claims this access takes. `None` takes every claim, because the access is exclusive.
	owner: Option<ClaimOwner>,
	/// Whether the lane holds this access. `None` for exclusive access, which no lane can hold at the same time.
	taken: Option<&'a Cell<bool>>,
}

impl<T> BatchOwned<'_, T> {
	/// Panics unless `claim` was made for this lane, for this stage's batch.
	#[inline]
	fn check<const W: usize>(&self, claim: &Claim<W>) {
		assert!(
			self.owner.is_none_or(|owner| owner == claim.owner),
			"Claim is not this lane's for this batch. The most likely cause is a group that is worked through in the stage of another batch, on another lane, or after the batches were rebuilt."
		);
	}

	/// Returns the items that `claim` names, for each of the two sides of its group's items.
	///
	/// # Panics
	///
	/// Panics if `claim` was not made for this lane and this stage's batch, or names an item past the last one.
	#[allow(unsafe_code, reason = "A claim's items are only written by the lane that holds the claim.")]
	#[inline]
	pub fn get<const W: usize>(&self, claim: &Claim<W>) -> [[&T; W]; 2] {
		self.check(claim);
		// SAFETY: Indexing keeps within the items. The claim is this lane's for the batch that every lane works
		// through in this stage, or this access is exclusive. No other group of a batch writes an item that this
		// group names, and a batch is only worked through in its own stage, so only this lane writes these items
		// now. It does so through `write`, which borrows this access exclusively, so not while these references
		// live.
		claim
			.entries
			.map(|entries| entries.map(|entry| unsafe { &*self.cells.items[entry as usize].get() }))
	}

	/// Calls `change` with each item that the group of `claim` writes, along with the side and the place of the
	/// group's item that it belongs to.
	///
	/// # Panics
	///
	/// Panics if `claim` was not made for this lane and this stage's batch, or names an item past the last one.
	#[allow(
		unsafe_code,
		reason = "A claim's written items are only reached by the lane that holds the claim."
	)]
	#[inline]
	pub fn write<const W: usize>(&mut self, claim: &Claim<W>, mut change: impl FnMut(usize, usize, &mut T)) {
		self.check(claim);
		for side in 0..2 {
			let mut places = claim.writable[side];
			while places != 0 {
				let place = places.trailing_zeros() as usize;
				places &= places - 1;
				let cell = &self.cells.items[claim.entries[side][place] as usize];
				// SAFETY: As for `get`, only this lane reaches the items its claim writes, and no item that a claim
				// writes is read as fixed by another group. This borrows the access exclusively, the lane has no
				// other access, and each reference ends before the next one is made, so it is the only one.
				change(side, place, unsafe { &mut *cell.get() });
			}
		}
	}
}

impl<T> Drop for BatchOwned<'_, T> {
	fn drop(&mut self) {
		if let Some(taken) = self.taken {
			taken.set(false);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Claim, PairBatches};
	use crate::core::alley::split::tests::message;
	use crate::core::alley::{Alley, Lane as _, LaneCells, LaneOwned, LaneRanges, Stage};

	/// The entry that stands for something no item writes, among nine that items do.
	const FLOOR: u32 = floor(9);

	/// Returns the entry after `count` others, as one that items only read.
	const fn floor(count: u32) -> u32 {
		count | PairBatches::FIXED
	}

	/// Returns the entries of items that chain `count` entries to their neighbors and to one fixed entry, the one
	/// after them.
	fn chained_items(count: u32) -> Vec<[u32; 2]> {
		let mut items = Vec::new();
		for entry in 0..count {
			items.push([entry, floor(count)]);
			if entry + 1 < count {
				items.push([entry, entry + 1]);
			}
		}
		items
	}

	#[test]
	fn a_batch_writes_each_entry_once() {
		let items = chained_items(9);
		let mut batches = PairBatches::default();
		batches.rebuild(10, [items.as_slice()].into_iter());

		for batch in 0..batches.len() {
			let mut seen = [false; 9];
			let written = batches.batch(batch).iter().flat_map(|item| items[*item as usize]);
			for entry in written.filter(|entry| *entry != FLOOR) {
				assert!(!seen[entry as usize]);
				seen[entry as usize] = true;
			}
		}
	}

	#[test]
	fn every_item_is_placed_once() {
		let items = chained_items(9);
		let mut batches = PairBatches::default();
		batches.rebuild(10, [items.as_slice()].into_iter());

		let mut placed = (0..batches.len())
			.flat_map(|batch| batches.batch(batch).iter().copied())
			.chain(batches.leftover().iter().copied())
			.collect::<Vec<_>>();
		placed.sort_unstable();

		assert_eq!(placed, (0..items.len() as u32).collect::<Vec<_>>());
	}

	#[test]
	fn items_that_only_share_fixed_entries_fill_one_batch() {
		// The items come as two runs, as two lanes would have found them.
		let items = (0..5).map(|entry| [entry, FLOOR]).collect::<Vec<_>>();
		let mut batches = PairBatches::default();
		batches.rebuild(10, [&items[..2], &items[2..]].into_iter());

		assert_eq!(batches.len(), 1);
		assert_eq!(batches.batch(0), [0, 1, 2, 3, 4]);
	}

	#[test]
	fn items_beyond_the_batch_limit_are_left_over_in_order() {
		// One entry in more items than there are batches.
		let limit = PairBatches::LIMIT as u32;
		let items = (0..limit + 3).map(|other| [0, other + 1]).collect::<Vec<_>>();
		let mut batches = PairBatches::default();
		batches.rebuild(PairBatches::LIMIT + 4, [items.as_slice()].into_iter());

		assert_eq!(batches.len(), PairBatches::LIMIT);
		assert_eq!(batches.leftover(), [limit, limit + 1, limit + 2]);
		// Each is a group of its own, because they share an entry.
		let groups: Vec<Vec<u32>> = batches.leftover_groups::<4>().map(|(_, group)| group.to_vec()).collect();
		assert_eq!(groups, [[limit], [limit + 1], [limit + 2]]);
	}

	#[test]
	fn rebuilding_replaces_the_previous_batches() {
		let mut batches = PairBatches::default();
		batches.rebuild(10, [chained_items(9).as_slice()].into_iter());
		batches.rebuild(2, [[[0, 1]].as_slice()].into_iter());

		assert_eq!(batches.len(), 1);
		assert_eq!(batches.batch(0), [0]);
		assert!(batches.leftover().is_empty());
	}

	#[test]
	#[should_panic(expected = "Batched item writes an entry past the end.")]
	fn an_item_that_writes_past_the_array_is_refused() {
		PairBatches::default().rebuild(4, [[[0, 4]].as_slice()].into_iter());
	}

	#[test]
	#[should_panic(expected = "Batched item reads an entry as fixed that another item writes.")]
	fn an_entry_that_is_both_fixed_and_written_is_refused() {
		PairBatches::default().rebuild(4, [[[0, 1], [2, 1 | PairBatches::FIXED]].as_slice()].into_iter());
	}

	/// The `Counted` struct holds what a test's lanes count: how many items wrote each entry, the batches, and the
	/// claims each lane keeps between its stages.
	#[derive(Default)]
	struct Counted {
		counts: LaneRanges<LaneCells<u32>>,
		batches: PairBatches,
		claims: LaneOwned<Vec<(u32, Claim<4>)>>,
	}

	/// Returns counts for `entries` entries and batches of `items`, lent to `alley` and split between `lanes` lanes.
	fn counted(alley: &Alley, entries: usize, items: &[[u32; 2]], lanes: usize) -> Counted {
		let mut counted = Counted::default();
		counted.counts.get_mut().resize(entries, 0);
		counted.counts.lend(alley.id());
		counted.batches.lend(alley.id());
		counted.batches.rebuild(entries, [items].into_iter());
		counted.counts.split_by(&counted.batches, lanes);
		counted.claims.lend(alley.id());
		counted.claims.grow(lanes);
		counted
	}

	#[test]
	fn lanes_write_the_entries_of_their_groups_batch_after_batch() {
		let mut alley = Alley::with_parallelism(4);
		// Enough entries for every lane to have groups of a batch, each in few enough items for all to fit in batches.
		let items = chained_items(64).repeat(8);
		let mut counted = counted(&alley, 65, &items, 4);
		assert!(counted.batches.leftover().is_empty());
		let batches = counted.batches.len() as u32;

		let run = alley.execute_phased(&mut counted, 4, |lane, counted| {
			for batch in 0..batches {
				let _ = lane.each_ref_keyed(counted, batch, |counted, stage| {
					let mut counts = counted.counts.batch(stage);
					let mut kept = counted.claims.own(stage.lane());
					for (claim, _) in counted.batches.share::<4>(stage) {
						// A group reads its entries, the fixed one among them, and adds one to each it writes.
						let before = counts.get(&claim).map(|side| side.map(|count| *count));
						counts.write(&claim, |side, place, count| *count = before[side][place] + 1);
						kept.push((batch, claim));
					}
				});
			}
			// A second pass over the same groups, with the claims the lane kept.
			for batch in 0..batches {
				let _ = lane.each_ref_keyed(counted, batch, |counted, stage| {
					let mut counts = counted.counts.batch(stage);
					let kept = counted.claims.own(stage.lane());
					for (_, claim) in kept.iter().filter(|(kept, _)| *kept == batch) {
						counts.write(claim, |_, _, count| *count += 1);
					}
				});
			}
		});

		assert!(run.is_ok());
		counted.counts.join();
		// Each entry is written by every item that names it, once in each pass, and nothing wrote the fixed one.
		let mut expected = [0u32; 65];
		for entry in items.iter().flatten().filter(|entry| **entry != floor(64)) {
			expected[*entry as usize] += 2;
		}
		assert_eq!(counted.counts.all()[..], expected);
	}

	#[test]
	fn a_claim_is_refused_outside_its_lane_and_batch() {
		const REFUSED: &str = "Claim is not this lane's for this batch.";
		let mut alley = Alley::with_parallelism(2);
		let items = chained_items(9).repeat(8);
		let mut counted = counted(&alley, 10, &items, 2);
		// Keeps the claims a lane takes of the first batch, and then adds one to every entry the kept claims write.
		let keep = |counted: &Counted, stage: &Stage<'_, '_>| {
			let claims = counted.batches.share::<4>(stage).map(|(claim, _)| (0, claim));
			counted.claims.own(stage.lane()).extend(claims);
		};
		let count = |counted: &Counted, stage: &Stage<'_, '_>| {
			let mut counts = counted.counts.batch(stage);
			for (_, claim) in counted.claims.own(stage.lane()).iter() {
				counts.write(claim, |_, _, count| *count += 1);
			}
		};

		// In the stage of the next batch. One lane runs these, so that what it panics with is what comes back.
		let late = alley.execute_phased(&mut counted, 1, |lane, counted| {
			let _ = lane.each_ref_keyed(counted, 0, keep);
			let _ = lane.each_ref_keyed(counted, 1, count);
		});
		assert!(message(late.unwrap_err()).starts_with(REFUSED));

		// After a rebuild, even of the same items.
		counted.claims.iter_mut().for_each(Vec::clear);
		let rebuilt = alley.execute_phased(&mut counted, 1, |lane, counted| {
			let _ = lane.each_ref_keyed(counted, 0, keep);
			lane.broadcast_mut(counted, |counted| {
				counted
					.batches
					.rebuild(10, [chained_items(9).repeat(8).as_slice()].into_iter());
				counted.counts.split_by(&counted.batches, 2);
			});
			let _ = lane.each_ref_keyed(counted, 0, count);
		});
		assert!(message(rebuilt.unwrap_err()).starts_with(REFUSED));

		// By a lane of a dispatch that splits the batch between another number of lanes.
		counted.claims.iter_mut().for_each(Vec::clear);
		let kept = alley.execute_phased(&mut counted, 2, |lane, counted| {
			let _ = lane.each_ref_keyed(counted, 0, keep);
		});
		assert!(kept.is_ok());
		let resplit = alley.execute_phased(&mut counted, 1, |lane, counted| {
			let _ = lane.each_ref_keyed(counted, 0, count);
		});
		assert!(message(resplit.unwrap_err()).starts_with(REFUSED));

		// On another lane: the lanes trade the claims they kept.
		counted.claims.iter_mut().for_each(Vec::clear);
		let traded = alley.execute_phased(&mut counted, 2, |lane, counted| {
			let _ = lane.each_ref_keyed(counted, 0, keep);
			lane.broadcast_mut(counted, |counted| {
				let mut kept: Vec<_> = counted.claims.iter_mut().map(std::mem::take).collect();
				kept.reverse();
				counted.claims.iter_mut().zip(kept).for_each(|(slot, kept)| *slot = kept);
			});
			let _ = lane.each_ref_keyed(counted, 0, count);
		});
		assert!(traded.is_err());

		// Nothing above wrote an entry, and exclusive access takes any claim.
		counted.counts.join();
		assert_eq!(counted.counts.all()[..], [0; 10]);
		let stale: Vec<Claim<4>> = counted.batches.batch_groups(0).map(|(claim, _)| claim).collect();
		let written = (counted.batches.batch(0).iter())
			.flat_map(|item| items[*item as usize])
			.filter(|entry| *entry != FLOOR)
			.count();
		counted.batches.rebuild(10, [[[0, 1]].as_slice()].into_iter());
		let mut counts = counted.counts.batch_mut();
		for claim in &stale {
			counts.write(claim, |_, _, count| *count += 1);
		}
		drop(counts);
		assert_eq!(counted.counts.all().iter().sum::<u32>(), written as u32);
	}

	#[test]
	fn lanes_that_name_different_batches_fail_the_stage() {
		let mut alley = Alley::with_parallelism(2);
		let mut value = 0usize;

		let run = alley.execute_phased(&mut value, 2, |lane, value| {
			let key = lane.idx() as u32;
			let _ = lane.each_ref_keyed(value, key, |_, stage| stage.key());
		});

		assert!(run.is_err());
	}
}
