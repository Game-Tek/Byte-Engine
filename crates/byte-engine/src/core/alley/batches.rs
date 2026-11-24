//! Batches of items that lanes work through side by side, because no two items of a batch write the same entry.
//!
//! Some work reads and writes a few entries of an array for each of many items, where items share entries: every
//! contact between two bodies changes both, and a body is in several contacts. Lanes cannot split such items by
//! range. [`PairBatches`] sorts them into batches in which every entry is written by at most one item, so that
//! lanes can share a batch, and names each group of a batch's items with a [`Claim`]. The [`LaneRanges`] whose
//! entries the items write keeps the batches, and [`LaneRanges::batch`] lets a lane reach exactly the entries of the
//! groups that are its share of the stage's batch.

#![expect(
	clippy::disallowed_types,
	reason = "A synchronization primitive: lanes write disjoint parts of this storage through shared references, which these cells implement, and its checks keep each lane to its own part"
)]

use std::cell::Cell;

use super::{
	Lane as _, Stage, shared_range,
	split::{LaneCells, LaneRanges, RangeSplit, check_lent, take},
};

/// The `PairBatches` struct sorts items that each write up to two entries of an array into batches in which no
/// entry is written twice, so that the items of one batch can be worked through in any order, or by several lanes
/// at the same time, with the same result.
///
/// The [`LaneRanges`] of the array the items write keeps it, so that a lane can only reach entries through the
/// batches of that array. In a stage that runs on one lane, call [`Self::rebuild`] through
/// [`LaneRanges::batches_mut`] with the entries of every item, and then [`LaneRanges::split_by`]. Then run one stage
/// for each batch, in order, with [`ConcreteLane::each_ref_keyed`](super::ConcreteLane::each_ref_keyed) and the batch
/// as the key: each lane takes its groups of the batch with [`LaneRanges::share`], and reaches their entries through
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
}

impl PairBatches {
	/// Marks an entry of an item as one the item only reads. Any number of a batch's items can read an entry that
	/// none writes, such as a body that contacts never move.
	pub const FIXED: u32 = 1 << 31;

	/// The most batches there can be, one for each bit of an entry's record of the batches that write it.
	pub const LIMIT: usize = u64::BITS as usize;

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

	/// Returns every group of `batch`, each as a claim on the entries of up to `W` items and the items themselves,
	/// for one thread with exclusive access to the array to work through. Use the claims with
	/// [`LaneRanges::batch_mut`].
	///
	/// # Panics
	///
	/// Panics if there is no such batch.
	pub fn batch_groups<const W: usize>(&self, batch: usize) -> impl Iterator<Item = (Claim<W>, &[u32])> + '_ {
		self.groups(batch as u32, self.batch(batch), 0)
	}

	/// Returns each item that fit in no batch as a group of its own, like [`Self::batch_groups`].
	pub fn leftover_groups<const W: usize>(&self) -> impl Iterator<Item = (Claim<W>, &[u32])> + '_ {
		self.leftover.chunks(1).enumerate().map(|(group, item)| {
			let claim = Claim {
				batch: Claim::<W>::LEFTOVER,
				group: group as u32,
			};
			(claim, item)
		})
	}

	/// Returns the entry on `side` of the item at `place` of the group that `claim` names.
	///
	/// # Panics
	///
	/// Panics if the group does not exist in the batches as they are now.
	pub fn entry<const W: usize>(&self, claim: &Claim<W>, side: usize, place: usize) -> usize {
		let group = self.group(claim);
		(self.pairs[group[place.min(group.len() - 1)] as usize][side] & !Self::FIXED) as usize
	}

	/// Returns `items`, the groups of `batch` from group `first` on, in groups of up to `W`, each with its claim.
	fn groups<'a, const W: usize>(
		&'a self,
		batch: u32,
		items: &'a [u32],
		first: usize,
	) -> impl Iterator<Item = (Claim<W>, &'a [u32])> + 'a {
		items.chunks(W).enumerate().map(move |(group, items)| {
			let claim = Claim {
				batch,
				group: (first + group) as u32,
			};
			(claim, items)
		})
	}

	/// Returns the number of groups of up to `W` items that `batch` holds.
	fn group_count<const W: usize>(&self, batch: u32) -> usize {
		if batch == Claim::<W>::LEFTOVER {
			self.leftover.len()
		} else {
			self.ends
				.get(batch as usize)
				.map_or(0, |_| self.batch(batch as usize).len().div_ceil(W))
		}
	}

	/// Returns the items of the group that `claim` names.
	///
	/// # Panics
	///
	/// Panics if the group does not exist in the batches as they are now.
	fn group<const W: usize>(&self, claim: &Claim<W>) -> &[u32] {
		let group = claim.group as usize;
		assert!(
			group < self.group_count::<W>(claim.batch),
			"Claimed group does not exist. The most likely cause is a claim kept from before the batches were rebuilt with fewer items."
		);
		if claim.batch == Claim::<W>::LEFTOVER {
			&self.leftover[group..group + 1]
		} else {
			let items = self.batch(claim.batch as usize);
			&items[group * W..(group * W + W).min(items.len())]
		}
	}

	/// Returns the entries of `group`, which holds up to `W` items, for each of an item's two sides, and the places
	/// whose entry the group writes, one bit per place.
	fn entries<const W: usize>(&self, group: &[u32]) -> ([[u32; W]; 2], [u8; 2]) {
		let (mut entries, mut writable) = ([[0; W]; 2], [0; 2]);
		for place in 0..W {
			// The places past the last item repeat it, so that reading a whole group reads real entries. Nothing
			// is written for them.
			let held = place < group.len();
			let pair = self.pairs[group[place.min(group.len() - 1)] as usize];
			for side in 0..2 {
				entries[side][place] = pair[side] & !Self::FIXED;
				writable[side] |= u8::from(held && pair[side] & Self::FIXED == 0) << place;
			}
		}
		(entries, writable)
	}
}

/// The `Claim` struct names one group of up to `W` batched items, whose entries a lane may reach while its batch is
/// worked through.
///
/// [`LaneRanges::share`] gives each lane its own. Keep it with what is worked out for the group, and pass it to
/// [`BatchOwned::get`] and [`BatchOwned::write`] on every pass over the batch. It is only a number: the
/// [`LaneRanges`] looks its entries up in the batches it keeps, and checks that the group is the lane's share of the
/// stage's batch, so a claim kept past a rebuild or taken to another array can't reach another lane's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim<const W: usize> {
	/// The batch, or [`Self::LEFTOVER`] for an item that fit in no batch.
	batch: u32,
	/// The group's position among the groups of its batch.
	group: u32,
}

impl<const W: usize> Claim<W> {
	/// The batch of the groups that [`PairBatches::leftover_groups`] names.
	const LEFTOVER: u32 = u32::MAX;
}

impl<T> LaneRanges<LaneCells<T>> {
	/// Returns the batches that the items are split by with [`Self::split_by`].
	pub fn batches(&self) -> &PairBatches {
		&self.batches
	}

	/// Returns the batches, to rebuild them for the items with [`PairBatches::rebuild`] before [`Self::split_by`].
	pub fn batches_mut(&mut self) -> &mut PairBatches {
		&mut self.batches
	}

	/// Gives each of `lanes` lanes the items that its share of the groups of a batch name, to reach with
	/// [`Self::batch`] in a stage for that batch, and keeps lanes from every other access until the items are split
	/// again or joined.
	pub fn split_by(&mut self, lanes: usize) {
		self.split = RangeSplit::Batches;
		self.track(lanes);
	}

	/// Returns this lane's share of the groups of the batch that the stage's key names, each as a claim on the
	/// entries of up to `W` items and the items themselves.
	///
	/// The claims let this lane, in a stage with this key, read and write those entries through [`Self::batch`].
	///
	/// # Panics
	///
	/// Panics if the items are not lent to the lane's alley, or if the stage's key is not a batch.
	pub fn share<'a, const W: usize>(&'a self, stage: &Stage<'_, '_>) -> impl Iterator<Item = (Claim<W>, &'a [u32])> + 'a {
		let lane = stage.lane();
		check_lent(self.lent_to, lane);
		let items = self.batches.batch(stage.key() as usize);
		let groups = shared_range(items.len().div_ceil(W), lane.idx(), lane.lane_count);
		self.batches.groups(
			stage.key(),
			&items[(groups.start * W).min(items.len())..(groups.end * W).min(items.len())],
			groups.start,
		)
	}

	/// Returns access to the items that this lane's share of the stage's batch names, until the returned guard is
	/// dropped. The guard cannot be kept past the stage.
	///
	/// # Panics
	///
	/// Panics if the items are not lent to the lane's alley, if they are not split by batch, or if the lane still
	/// holds a part of the items from an earlier call.
	pub fn batch<'a, const W: usize>(&'a self, stage: &'a Stage<'_, '_>) -> BatchOwned<'a, T, W> {
		let lane = stage.lane();
		check_lent(self.lent_to, lane);
		assert!(
			matches!(self.split, RangeSplit::Batches),
			"Lane ranges are not split by batch. The most likely cause is a batch stage that runs before a one-lane stage called `split_by`."
		);
		let Some(taken) = self.taken.get(lane.idx()) else {
			panic!(
				"Lane ranges have no entry for lane {}. The most likely cause is a dispatch with more lanes than the items were split between.",
				lane.idx()
			);
		};
		take(&taken.0);
		// The lane's share is worked out once here, so that each access only checks a claim against it.
		let items = self.batches.batch(stage.key() as usize);
		BatchOwned {
			cells: &self.items,
			batches: &self.batches,
			share: Some(LaneShare {
				batch: stage.key(),
				items,
				groups: shared_range(items.len().div_ceil(W), lane.idx(), lane.lane_count),
			}),
			taken: Some(&taken.0),
		}
	}

	/// Returns access to the items of any group, for one thread to work through groups that lanes cannot share,
	/// such as [`PairBatches::leftover_groups`].
	pub fn batch_mut<const W: usize>(&mut self) -> BatchOwned<'_, T, W> {
		BatchOwned {
			cells: &self.items,
			batches: &self.batches,
			share: None,
			taken: None,
		}
	}
}

/// The `LaneShare` struct names the groups one lane may reach in a stage: its share of the groups of one batch.
struct LaneShare<'a> {
	batch: u32,
	/// The items of the batch.
	items: &'a [u32],
	/// The lane's groups of the batch.
	groups: std::ops::Range<usize>,
}

/// The `BatchOwned` struct lets one lane reach the items that its share of the stage's batch names, while every
/// lane works through the same batch, by claims on groups of up to `W` items. [`LaneRanges::batch`] returns it.
pub struct BatchOwned<'a, T, const W: usize> {
	cells: &'a LaneCells<T>,
	/// The batches the items are split by, which claims are looked up in.
	batches: &'a PairBatches,
	/// Which groups this access reaches. `None` reaches every group, because the access is exclusive.
	share: Option<LaneShare<'a>>,
	/// Whether the lane holds this access. `None` for exclusive access, which no lane can hold at the same time.
	taken: Option<&'a Cell<bool>>,
}

impl<'a, T, const W: usize> BatchOwned<'a, T, W> {
	/// Returns the batches the items are split by, to take groups of them with exclusive access.
	pub fn batches(&self) -> &'a PairBatches {
		self.batches
	}

	/// Returns the entries of the group that `claim` names and the places whose entry it writes.
	///
	/// # Panics
	///
	/// Panics unless the group is in this lane's share of this stage's batch, or this access is exclusive.
	#[inline]
	fn entries(&self, claim: &Claim<W>) -> ([[u32; W]; 2], [u8; 2]) {
		let group = match &self.share {
			Some(share) => {
				let group = claim.group as usize;
				assert!(
					claim.batch == share.batch && share.groups.contains(&group),
					"Claim is not this lane's for this batch. The most likely cause is a group that is worked through in the stage of another batch, or on another lane."
				);
				&share.items[group * W..(group * W + W).min(share.items.len())]
			}
			None => self.batches.group(claim),
		};
		self.batches.entries(group)
	}

	/// Returns the items that `claim` names, for each of the two sides of its group's items.
	///
	/// # Panics
	///
	/// Panics if the group is not this lane's share of this stage's batch, or doesn't exist.
	#[allow(
		unsafe_code,
		reason = "A group's items are only written by the lane whose share holds the group."
	)]
	#[inline]
	pub fn get(&self, claim: &Claim<W>) -> [[&T; W]; 2] {
		let (entries, _) = self.entries(claim);
		// SAFETY: Indexing keeps within the items. The group is this lane's share of the batch that every lane works
		// through in this stage, looked up in the batches the items were split by, or this access is exclusive. No
		// other group of a batch writes an item that this group names, and a batch is only worked through in its own
		// stage, so only this lane writes these items now. It does so through `write`, which borrows this access
		// exclusively, so not while these references live.
		entries.map(|entries| entries.map(|entry| unsafe { &*self.cells.items[entry as usize].get() }))
	}

	/// Calls `change` with each item that the group of `claim` writes, along with the side and the place of the
	/// group's item that it belongs to.
	///
	/// # Panics
	///
	/// Panics if the group is not this lane's share of this stage's batch, or doesn't exist.
	#[allow(
		unsafe_code,
		reason = "A group's written items are only reached by the lane whose share holds the group."
	)]
	#[inline]
	pub fn write(&mut self, claim: &Claim<W>, mut change: impl FnMut(usize, usize, &mut T)) {
		let (entries, writable) = self.entries(claim);
		for side in 0..2 {
			let mut places = writable[side];
			while places != 0 {
				let place = places.trailing_zeros() as usize;
				places &= places - 1;
				let cell = &self.cells.items[entries[side][place] as usize];
				// SAFETY: As for `get`, only this lane reaches the items its group writes, and no item that a group
				// writes is read as fixed by another group. This borrows the access exclusively, the lane has no
				// other access, and each reference ends before the next one is made, so it is the only one.
				change(side, place, unsafe { &mut *cell.get() });
			}
		}
	}
}

impl<T, const W: usize> Drop for BatchOwned<'_, T, W> {
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

	/// The `Counted` struct holds what a test's lanes count: how many items wrote each entry, with the batches of the
	/// items, and the claims each lane keeps between its stages.
	#[derive(Default)]
	struct Counted {
		counts: LaneRanges<LaneCells<u32>>,
		claims: LaneOwned<Vec<(u32, Claim<4>)>>,
	}

	/// Returns counts for `entries` entries and batches of `items`, lent to `alley` and split between `lanes` lanes.
	fn counted(alley: &Alley, entries: usize, items: &[[u32; 2]], lanes: usize) -> Counted {
		let mut counted = Counted::default();
		counted.counts.get_mut().resize(entries, 0);
		counted.counts.lend(alley.id());
		counted.counts.batches_mut().rebuild(entries, [items].into_iter());
		counted.counts.split_by(lanes);
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
		assert!(counted.counts.batches().leftover().is_empty());
		let batches = counted.counts.batches().len() as u32;

		let run = alley.execute_phased(&mut counted, 4, |lane, counted| {
			for batch in 0..batches {
				let _ = lane.each_ref_keyed(counted, batch, |counted, stage| {
					let mut counts = counted.counts.batch(stage);
					let mut kept = counted.claims.own(stage.lane());
					for (claim, _) in counted.counts.share::<4>(stage) {
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
			let claims = counted.counts.share::<4>(stage).map(|(claim, _)| (0, claim));
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

		// Nothing above wrote an entry, and exclusive access takes every group of a batch.
		counted.counts.join();
		assert_eq!(counted.counts.all()[..], [0; 10]);
		let written = (counted.counts.batches().batch(0).iter())
			.flat_map(|item| items[*item as usize])
			.filter(|entry| *entry != FLOOR)
			.count();
		let mut counts = counted.counts.batch_mut();
		let batches = counts.batches();
		for (claim, _) in batches.batch_groups::<4>(0) {
			counts.write(&claim, |_, _, count| *count += 1);
		}
		drop(counts);
		assert_eq!(counted.counts.all().iter().sum::<u32>(), written as u32);
	}

	#[test]
	#[should_panic(expected = "Claimed group does not exist.")]
	fn a_claim_past_the_rebuilt_groups_is_refused() {
		let mut counts = LaneRanges::new(LaneCells::<u32>::default());
		counts.get_mut().resize(10, 0);
		counts
			.batches_mut()
			.rebuild(10, [chained_items(9).repeat(8).as_slice()].into_iter());
		let kept: Vec<Claim<4>> = counts.batches().batch_groups(0).map(|(claim, _)| claim).collect();
		// The same entries, now written by one item, which is one group.
		counts.batches_mut().rebuild(10, [[[0, 1]].as_slice()].into_iter());
		let mut access = counts.batch_mut();
		access.write(&kept[1], |_, _, count| *count += 1);
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
