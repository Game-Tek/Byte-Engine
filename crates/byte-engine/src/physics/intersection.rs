//! The broad phase finds the world bounds that overlap, so the narrow phase only tests those pairs.

use std::{
	ops::Range,
	simd::{
		cmp::{SimdOrd as _, SimdPartialOrd as _},
		f32x4,
		num::SimdFloat as _,
		u32x4,
	},
};

use math::AABB;
use utils::sync::RwLock;

/// The number of bounds one sweep step tests at once.
const WIDTH: usize = 4;

/// The edge of a grid cell along an axis, in extents of a typical box along that axis.
///
/// Smaller cells leave each box fewer neighbors to scan, and larger ones leave fewer boxes straddling several
/// cells. On an Apple M4, piles of 1,000 to 16,000 one-meter bodies stepped fastest at about this size.
const CELL_EXTENTS: f32 = 4.0;

/// The cells a box may reach across along one axis before it is scanned on its own instead of through the grid.
///
/// A box as wide as a floor would otherwise be listed in every cell it covers.
const WIDE_BOX_CELLS: f32 = 2.0;

/// The most cells the grid has along one axis.
const MAX_AXIS_CELLS: u32 = 1024;

/// The number of boxes from which they are split into cells.
///
/// Listing the boxes in cells takes time that only a long scan gives back. On an Apple M4, cells made steps of
/// 2,000 one-meter bodies about 4% slower, left 4,000 about even, and made 8,000 12% to 26% faster and 16,000 25%
/// to 33% faster, depending on how far the pile had spread.
const GRID_BOXES: usize = 4096;

/// The `SweepBounds` struct keeps a step's world bounds sorted along one axis and split into the cells of a grid
/// over the other two, so the pairs that overlap are found by scanning each box's neighbors in its cells instead of
/// testing every pair.
///
/// Call [`Self::rebuild`] with the step's bounds, then [`Self::overlaps`] to read the overlapping pairs. To spread
/// the scan over lanes, give each lane its own range of `0..`[`Self::count`]. To spread the rebuild over lanes too,
/// call [`Self::stage`] instead and give each lane one part to [`Self::fill`].
#[derive(Default)]
pub struct SweepBounds {
	/// Every box, sorted along the sweep axis.
	sorted: BoundsArrays,
	/// Whether each sorted box is too wide for the grid.
	is_wide: Vec<bool>,
	/// The sorted indices of the boxes that are too wide for the grid. Each is scanned against every box.
	wide: Vec<u32>,
	/// The grid over the two axes that are not swept.
	grid: Grid,
	/// The boxes of the grid's cells, split into parts that each hold a run of neighboring cells, so that lanes can
	/// each fill one. Every part has its own lock, which only its lane takes to fill it.
	parts: Vec<RwLock<CellPart>>,
	/// One past the last cell of each part that the last [`Self::stage`] laid out. A part starts where the one
	/// before it ends, and no part is laid out while the boxes are not split into cells.
	part_ends: Vec<usize>,
	/// The boxes as they were received, kept between rebuilds so staging them does not allocate.
	staged: Vec<StagedBounds>,
	/// The order the last rebuild sorted `staged` into. Boxes move little between steps, so the next rebuild
	/// usually only has to repair it.
	order: Vec<SweepKey>,
	/// The axis the last rebuild sorted along.
	axis: usize,
}

/// The `BoundsArrays` struct holds boxes as one array per corner coordinate, so a scan reads the same coordinate
/// of several boxes at once.
#[derive(Default)]
struct BoundsArrays {
	/// The identifier each box was staged with.
	ids: Vec<u32>,
	/// The smallest corner of each box as one array per axis. Index 0 is the sweep axis.
	min: [Vec<f32>; 3],
	/// The largest corner of each box, laid out like `min`.
	max: [Vec<f32>; 3],
}

impl BoundsArrays {
	/// Makes room for `count` entries. The ones that were there keep what they held, so every entry has to be
	/// written again, with [`Self::set`] or [`Self::set_empty`].
	fn resize(&mut self, count: usize) {
		self.ids.resize(count, 0);
		for corner in self.min.iter_mut().chain(&mut self.max) {
			corner.resize(count, f32::NAN);
		}
	}

	/// Makes entry `index` overlap nothing.
	fn set_empty(&mut self, index: usize) {
		for corner in self.min.iter_mut().chain(&mut self.max) {
			// A comparison with NaN is never true, so the entry overlaps nothing, not even an unbounded box.
			corner[index] = f32::NAN;
		}
	}

	/// Writes a box to entry `index`.
	fn set(&mut self, index: usize, id: u32, min: [f32; 3], max: [f32; 3]) {
		self.ids[index] = id;
		for axis in 0..3 {
			self.min[axis][index] = min[axis];
			self.max[axis][index] = max[axis];
		}
	}

	/// Returns the corners of the box at entry `index`.
	fn get(&self, index: usize) -> ([f32; 3], [f32; 3]) {
		(
			[0, 1, 2].map(|axis| self.min[axis][index]),
			[0, 1, 2].map(|axis| self.max[axis][index]),
		)
	}
}

/// The `CellPart` struct holds the boxes of a run of neighboring grid cells, so that the cells of a grid can be
/// filled as several parts at the same time.
#[derive(Default)]
struct CellPart {
	/// The cells the part holds.
	cells: Range<usize>,
	/// Where each cell's boxes start in `boxes`, followed by where the last cell's end.
	starts: Vec<u32>,
	/// The boxes of every cell, cell after cell, in sweep order within a cell. A box is listed in every cell it
	/// reaches into, and the boxes of a cell are followed by `WIDTH` entries that overlap nothing.
	boxes: BoundsArrays,
	/// The number of boxes placed in each cell so far, kept between rebuilds so filling the cells does not allocate.
	fills: Vec<u32>,
	/// The sorted indices of the boxes that may reach into the part's cells, kept between rebuilds like `fills`.
	members: Vec<u32>,
}

impl CellPart {
	/// Replaces the part's boxes with the ones `bounds` has in `cells`.
	fn fill(&mut self, bounds: &SweepBounds, cells: Range<usize>) {
		if cells.len() == bounds.grid.count() {
			// A part that holds every cell holds every box of the grid, so there are no members to pick.
			let every_box = (0..bounds.boxes()).filter(|index| !bounds.is_wide[*index]);
			self.list(bounds, cells, every_box);
		} else {
			let mut members = std::mem::take(&mut self.members);
			bounds.cell_members(&cells, &mut members);
			self.list(bounds, cells, members.iter().map(|index| *index as usize));
			self.members = members;
		}
	}

	/// Replaces the part's boxes with the ones that `members`, sorted indices in sweep order, have in `cells`.
	///
	/// It counts the boxes of each cell, gives each cell its stretch of entries, then writes the boxes in sweep
	/// order, which leaves every cell sorted like the whole.
	fn list(&mut self, bounds: &SweepBounds, cells: Range<usize>, members: impl Iterator<Item = usize> + Clone) {
		/// Calls `entry` with each of `cells` that a member reaches into, counted from the first of them, and the
		/// member's sorted index.
		fn for_each_entry(
			bounds: &SweepBounds,
			cells: &Range<usize>,
			members: impl Iterator<Item = usize>,
			mut entry: impl FnMut(usize, usize),
		) {
			for index in members {
				let (min, max) = bounds.sorted.get(index);
				for cell in bounds.grid.cells(min, max).filter(|cell| cells.contains(cell)) {
					entry(cell - cells.start, index);
				}
			}
		}

		self.fills.clear();
		self.fills.resize(cells.len(), 0);
		let fills = &mut self.fills;
		for_each_entry(bounds, &cells, members.clone(), |cell, _| fills[cell] += 1);

		self.starts.clear();
		let mut start = 0;
		for fill in &mut self.fills {
			self.starts.push(start);
			// A cell with boxes ends in entries that overlap nothing, so a scan can read whole groups past its end.
			let entries = if *fill == 0 { 0 } else { *fill + WIDTH as u32 };
			*fill = start;
			start += entries;
		}
		self.starts.push(start);

		self.boxes.resize(start as usize);
		let (fills, boxes) = (&mut self.fills, &mut self.boxes);
		for_each_entry(bounds, &cells, members, |cell, index| {
			let fill = &mut fills[cell];
			let (min, max) = bounds.sorted.get(index);
			boxes.set(*fill as usize, bounds.sorted.ids[index], min, max);
			*fill += 1;
		});
		// Every fill now points past its cell's last box, at the entries that have to overlap nothing.
		for (fill, end) in self.fills.iter().zip(&self.starts[1..]) {
			for entry in *fill..*end {
				self.boxes.set_empty(entry as usize);
			}
		}
		self.cells = cells;
	}
}

/// The `Grid` struct maps positions on the two axes that are not swept to the cells boxes are split into.
#[derive(Clone, Copy)]
struct Grid {
	/// Where the first cell starts on each axis.
	origin: [f32; 2],
	/// The number of cells in one unit of length on each axis.
	cells_per_unit: [f32; 2],
	/// The number of cells along each axis, at least one.
	cells: [u32; 2],
}

impl Default for Grid {
	fn default() -> Self {
		Self {
			origin: [0.0; 2],
			cells_per_unit: [0.0; 2],
			cells: [1; 2],
		}
	}
}

impl Grid {
	/// Returns the cell that holds `position` along one of the grid's axes. Positions outside the grid belong to
	/// its first or last cell.
	fn coordinate(&self, axis: usize, position: f32) -> u32 {
		// The conversion turns a negative or NaN offset into the first cell and saturates large ones.
		(((position - self.origin[axis]) * self.cells_per_unit[axis]) as u32).min(self.cells[axis] - 1)
	}

	/// Returns the indices of the cells a box reaches into.
	fn cells(self, min: [f32; 3], max: [f32; 3]) -> impl Iterator<Item = usize> {
		let [first, second] = [0, 1].map(|axis| self.coordinate(axis, min[axis + 1])..=self.coordinate(axis, max[axis + 1]));
		second.flat_map(move |second| first.clone().map(move |first| self.cell(first, second)))
	}

	/// Returns the index of the cell at the given coordinates.
	fn cell(&self, first: u32, second: u32) -> usize {
		(second * self.cells[0] + first) as usize
	}

	/// Returns the number of cells.
	fn count(&self) -> usize {
		(self.cells[0] * self.cells[1]) as usize
	}
}

/// The `StagedBounds` struct holds one box between being received and being sorted along the sweep axis.
struct StagedBounds {
	id: u32,
	min: [f32; 3],
	max: [f32; 3],
}

/// The `SweepKey` struct places one staged box in the sweep order.
#[derive(Clone, Copy)]
struct SweepKey {
	/// Where the box starts on the sweep axis.
	start: f32,
	id: u32,
	/// The box's index in the staged boxes.
	staged: u32,
}

impl SweepKey {
	/// Orders keys by where their boxes start. Identifiers break ties, so equal boxes come out in the same order
	/// whatever order they were staged in and whatever order an earlier rebuild left.
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.start.total_cmp(&other.start).then_with(|| self.id.cmp(&other.id))
	}
}

impl SweepBounds {
	/// Returns the number of scan positions [`Self::overlaps`] takes ranges of.
	///
	/// A box that reaches into several cells has a position in each, so this is not the number of boxes.
	pub fn count(&self) -> usize {
		if self.in_cells() {
			let entries = |part: &RwLock<CellPart>| part.read().boxes.ids.len();
			self.wide.len() + self.laid_out_parts().iter().map(entries).sum::<usize>()
		} else {
			self.boxes()
		}
	}

	/// Returns the parts the last [`Self::stage`] laid out.
	fn laid_out_parts(&self) -> &[RwLock<CellPart>] {
		&self.parts[..self.part_ends.len()]
	}

	/// Returns the number of boxes.
	fn boxes(&self) -> usize {
		self.order.len()
	}

	/// Returns whether the boxes are split into cells. Few boxes are scanned as one sorted list instead.
	fn in_cells(&self) -> bool {
		self.boxes() >= GRID_BOXES
	}

	/// Replaces the stored boxes with `bounds`, each identified by the distinct `usize` it comes with.
	///
	/// The boxes are sorted along the axis on which the fewest of them share a stretch and split into cells along
	/// the other two, which keeps each box's scan in [`Self::overlaps`] short. When `bounds` yields the boxes of
	/// the last rebuild again, in the same order, the sort starts from the order they had then, which is quick
	/// when they have moved little.
	pub fn rebuild(&mut self, bounds: impl Iterator<Item = (usize, AABB)>) {
		if self.stage(bounds, 1) {
			self.fill(0);
		}
	}

	/// Replaces the stored boxes like [`Self::rebuild`], but leaves the grid's cells to be filled as `parts` parts.
	///
	/// Returns whether the boxes are split into cells. If they are, call [`Self::fill`] with every part from `0` up
	/// to `parts` before reading [`Self::count`] or [`Self::overlaps`]; each call can come from another lane. The
	/// pairs come out in the same sequence whatever the number of parts.
	pub fn stage(&mut self, bounds: impl Iterator<Item = (usize, AABB)>, parts: usize) -> bool {
		debug_assert!(
			parts != 0,
			"Broad-phase part count is zero. The most likely cause is staging the bounds for a step that runs on no lane."
		);
		// Staging over the last rebuild's boxes shows whether these are the same boxes.
		let mut count = 0;
		let mut same_boxes = true;
		for (id, bounds) in bounds {
			debug_assert!(
				u32::try_from(id).is_ok(),
				"Broad-phase identifier is too large. The most likely cause is a body index above the 32-bit slot range."
			);
			let staged = StagedBounds {
				id: id as u32,
				min: bounds.min().to_array(),
				max: bounds.max().to_array(),
			};
			match self.staged.get_mut(count) {
				Some(previous) => {
					same_boxes &= previous.id == staged.id;
					*previous = staged;
				}
				None => {
					same_boxes = false;
					self.staged.push(staged);
				}
			}
			count += 1;
		}
		same_boxes &= count == self.staged.len();
		self.staged.truncate(count);

		let sweep_axis = least_crowded_axis(&self.staged);
		let axes = [sweep_axis, (sweep_axis + 1) % 3, (sweep_axis + 2) % 3];

		// The last order only helps for the same boxes along the same axis.
		let repairable = same_boxes && sweep_axis == self.axis;
		if repairable {
			for key in &mut self.order {
				key.start = self.staged[key.staged as usize].min[sweep_axis];
			}
		} else {
			self.order.clear();
			self.order
				.extend(self.staged.iter().enumerate().map(|(index, staged)| SweepKey {
					start: staged.min[sweep_axis],
					id: staged.id,
					staged: index as u32,
				}));
		}
		if !(repairable && repair(&mut self.order)) {
			self.order.sort_unstable_by(SweepKey::cmp);
		}
		self.axis = sweep_axis;

		self.sorted.ids.clear();
		self.sorted.ids.extend(self.order.iter().map(|key| key.id));
		let staged = |key: &SweepKey| &self.staged[key.staged as usize];
		for (slot, axis) in axes.into_iter().enumerate() {
			// The padding lets a scan read whole groups past the last box. A comparison with NaN is never true, so
			// it overlaps nothing, not even an unbounded box.
			let padding = std::iter::repeat_n(f32::NAN, WIDTH);
			let (min, max) = (&mut self.sorted.min[slot], &mut self.sorted.max[slot]);
			min.clear();
			min.extend(self.order.iter().map(|key| staged(key).min[axis]).chain(padding.clone()));
			max.clear();
			max.extend(self.order.iter().map(|key| staged(key).max[axis]).chain(padding));
		}
		self.part_ends.clear();
		if self.in_cells() {
			self.lay_out_cells(count, parts);
		}
		self.in_cells()
	}

	/// Lists in part `part` the boxes of the cells that [`Self::stage`] gave it.
	///
	/// This only reads what [`Self::stage`] left and writes the one part, so every part can be filled from another
	/// lane at the same time. A part that was not laid out is left as it is.
	pub fn fill(&self, part: usize) {
		let Some(end) = self.part_ends.get(part).copied() else {
			return;
		};
		let start = part.checked_sub(1).map_or(0, |previous| self.part_ends[previous]);
		self.parts[part].write().fill(self, start..end);
	}

	/// Lays a grid over the two axes that are not swept, and splits its cells into `parts` runs that hold about as
	/// many of the `count` sorted boxes each.
	///
	/// Cells are a few typical boxes wide, so a box shares its cells with few others. Boxes much wider than a
	/// cell stay out of the grid.
	fn lay_out_cells(&mut self, count: usize, parts: usize) {
		let extent = |index: usize, axis: usize| self.sorted.max[axis + 1][index] - self.sorted.min[axis + 1][index];
		// A cell edge that is not a positive length, as when the boxes are flat along an axis, means one cell
		// along that axis that no box is too wide for.
		let edges = [0, 1].map(|axis| {
			let edge = CELL_EXTENTS * typical(count, |index| extent(index, axis));
			if edge > 0.0 && edge.is_finite() { edge } else { f32::INFINITY }
		});

		self.is_wide.clear();
		self.wide.clear();
		let mut low = [f32::INFINITY; 2];
		let mut high = [f32::NEG_INFINITY; 2];
		for index in 0..count {
			// Written so that a box with a NaN or unbounded extent is wide: the grid cannot place it.
			let fits = (0..2).all(|axis| extent(index, axis) <= WIDE_BOX_CELLS * edges[axis]);
			self.is_wide.push(!fits);
			if fits {
				for axis in 0..2 {
					low[axis] = low[axis].min(self.sorted.min[axis + 1][index]);
					high[axis] = high[axis].max(self.sorted.max[axis + 1][index]);
				}
			} else {
				self.wide.push(index as u32);
			}
		}

		let mut cells = [0, 1].map(|axis| {
			let span = high[axis] - low[axis];
			if span > 0.0 && edges[axis].is_finite() {
				((span / edges[axis]).ceil() as u32).clamp(1, MAX_AXIS_CELLS)
			} else {
				1
			}
		});
		// Boxes spread thinly over a large area would leave most cells empty, so there are never more cells than
		// boxes.
		let most = count.max(1) as f32;
		let total = (cells[0] * cells[1]) as f32;
		if total > most {
			let shrink = (total / most).sqrt();
			cells = cells.map(|axis_cells| ((axis_cells as f32 / shrink) as u32).max(1));
		}
		self.grid = Grid {
			origin: low,
			cells_per_unit: [0, 1].map(|axis| {
				let span = high[axis] - low[axis];
				if span > 0.0 && span.is_finite() {
					cells[axis] as f32 / span
				} else {
					0.0
				}
			}),
			cells,
		};

		if self.parts.len() < parts {
			self.parts.resize_with(parts, Default::default);
		}
		if parts > 1 {
			self.split_cells(count, parts);
		}
		self.part_ends.push(self.grid.count());
	}

	/// Chooses where each part but the last ends, so that the `parts` parts hold about as many of the `count`
	/// sorted boxes each, wherever the boxes crowd.
	fn split_cells(&mut self, count: usize, parts: usize) {
		/// Enough boxes to place the ends within a few percent of an even split, and few enough to sort on every
		/// rebuild.
		const SAMPLES: usize = 1024;
		// Each part ends at the cell where the next share of the boxes starts, judged from the cells that an evenly
		// spaced sample of the boxes start in.
		let samples = count.min(SAMPLES);
		let mut sample = [0u32; SAMPLES];
		let grid = self.grid;
		for (slot, cell) in sample.iter_mut().enumerate().take(samples) {
			let (min, _) = self.sorted.get(slot * count / samples);
			*cell = grid.cell(grid.coordinate(0, min[1]), grid.coordinate(1, min[2])) as u32;
		}
		let sample = &mut sample[..samples];
		sample.sort_unstable();
		self.part_ends
			.extend((1..parts).map(|part| sample[part * samples / parts] as usize));
	}

	/// Replaces `members` with the sorted indices of the grid's boxes that may reach into `cells`, in sweep order.
	///
	/// Every box that reaches into one of `cells` is a member. A box that reaches around them, into cells before
	/// and after, is one too.
	fn cell_members(&self, cells: &Range<usize>, members: &mut Vec<u32>) {
		members.clear();
		if cells.is_empty() {
			return;
		}
		let (grid, count) = (self.grid, self.boxes());
		// The same arithmetic as `Grid::coordinate` and `Grid::cell`, four boxes at a time, so that a box has the
		// cells here that it is listed in.
		let coordinate = |axis: usize, positions: &[f32]| {
			let offset = f32x4::from_slice(positions) - f32x4::splat(grid.origin[axis]);
			(offset * f32x4::splat(grid.cells_per_unit[axis]))
				.cast::<u32>()
				.simd_min(u32x4::splat(grid.cells[axis] - 1))
		};
		let cell = |corner: &[Vec<f32>; 3], group: Range<usize>| {
			coordinate(1, &corner[2][group.clone()]) * u32x4::splat(grid.cells[0]) + coordinate(0, &corner[1][group])
		};
		let (start, end) = (u32x4::splat(cells.start as u32), u32x4::splat(cells.end as u32));
		for group in (0..count).step_by(WIDTH) {
			// The sorted boxes end in entries that are not boxes, so the last group can be read whole. A box's
			// cells lie between the cell of its smallest corner and the cell of its largest.
			let group_range = group..group + WIDTH;
			let reaches =
				cell(&self.sorted.max, group_range.clone()).simd_ge(start) & cell(&self.sorted.min, group_range).simd_lt(end);
			let mut lanes = reaches.to_bitmask();
			while lanes != 0 {
				let index = group + lanes.trailing_zeros() as usize;
				lanes &= lanes - 1;
				if index < count && !self.is_wide[index] {
					members.push(index as u32);
				}
			}
		}
	}

	/// Calls `found` with every pair of overlapping boxes whose first box has its scan position in `range`.
	///
	/// Boxes that only touch count as overlapping. Ranges that together cover `0..`[`Self::count`] report every pair
	/// exactly once, and reading them in ascending order gives the same sequence as one scan of the whole range.
	pub fn overlaps(&self, range: Range<usize>, mut found: impl FnMut([usize; 2])) {
		if !self.in_cells() {
			// Every box is a neighbor of every other one, as if they shared one cell.
			return self.scan_cell(&self.sorted, None, range, self.boxes(), &mut found);
		}
		// The wide boxes take the first positions, and the entries of each part in turn follow.
		let wide = self.wide.len();
		for position in range.start.min(wide)..range.end.min(wide) {
			self.scan_wide(self.wide[position] as usize, &mut found);
		}
		let mut first = wide;
		for part in self.laid_out_parts() {
			let part = part.read();
			let entries = part.boxes.ids.len();
			let range = range.start.saturating_sub(first).min(entries)..range.end.saturating_sub(first).min(entries);
			if !range.is_empty() {
				self.scan_part(&part, range, &mut found);
			}
			first += entries;
		}
	}

	/// Calls `found` with the pairs each box in `entries` of `part` forms with the boxes after it in its cell.
	fn scan_part(&self, part: &CellPart, entries: Range<usize>, found: &mut impl FnMut([usize; 2])) {
		let first_cell = part
			.starts
			.partition_point(|start| *start as usize <= entries.start)
			.saturating_sub(1);
		for cell in first_cell..part.starts.len().saturating_sub(1) {
			let (start, end) = (part.starts[cell] as usize, part.starts[cell + 1] as usize);
			if start >= entries.end {
				break;
			}
			if start < end {
				let boxes_end = end - WIDTH;
				let range = entries.start.max(start)..entries.end.min(boxes_end);
				self.scan_cell(&part.boxes, Some(part.cells.start + cell), range, boxes_end, found);
			}
		}
	}

	/// Calls `found` with the pairs each box in `range` of `boxes` forms with the boxes after it, up to `boxes_end`.
	///
	/// `boxes` are sorted along the sweep axis. When they are the entries of a grid cell, `cell` names it.
	// Inlined so the scan without cells is compiled without the checks that only cells need.
	#[inline(always)]
	fn scan_cell(
		&self,
		boxes: &BoundsArrays,
		cell: Option<usize>,
		range: Range<usize>,
		boxes_end: usize,
		found: &mut impl FnMut([usize; 2]),
	) {
		let [min_sweep, min_second, min_third] = &boxes.min;
		let [max_sweep, max_second, max_third] = &boxes.max;
		for first in range {
			let reach = f32x4::splat(max_sweep[first]);
			let (low_second, high_second) = (f32x4::splat(min_second[first]), f32x4::splat(max_second[first]));
			let (low_third, high_third) = (f32x4::splat(min_third[first]), f32x4::splat(max_third[first]));
			let mut next = first + 1;
			while next < boxes_end {
				let group = next..next + WIDTH;
				let within_reach = f32x4::from_slice(&min_sweep[group.clone()]).simd_le(reach);
				let overlapping = within_reach
					& f32x4::from_slice(&min_second[group.clone()]).simd_le(high_second)
					& f32x4::from_slice(&max_second[group.clone()]).simd_ge(low_second)
					& f32x4::from_slice(&min_third[group.clone()]).simd_le(high_third)
					& f32x4::from_slice(&max_third[group]).simd_ge(low_third);
				let mut lanes = overlapping.to_bitmask();
				while lanes != 0 {
					let second = next + lanes.trailing_zeros() as usize;
					lanes &= lanes - 1;
					// Two boxes that both reach into several cells meet in each cell they share. Only the cell
					// that holds the lowest corner of their overlap reports them.
					let reports = cell.is_none_or(|cell| {
						let corner = [
							min_second[first].max(min_second[second]),
							min_third[first].max(min_third[second]),
						];
						self.grid
							.cell(self.grid.coordinate(0, corner[0]), self.grid.coordinate(1, corner[1]))
							== cell
					});
					if reports {
						found([boxes.ids[first] as usize, boxes.ids[second] as usize]);
					}
				}
				// Boxes are sorted by where they start on the sweep axis, so the first one that starts past this
				// box's end is followed only by boxes that do too.
				if !within_reach.all() {
					break;
				}
				next += WIDTH;
			}
		}
	}

	/// Calls `found` with the pairs the wide box at `index` of the sorted boxes forms with every other box.
	///
	/// A pair of wide boxes is reported by the one that comes first in the sweep order.
	fn scan_wide(&self, index: usize, found: &mut impl FnMut([usize; 2])) {
		let count = self.boxes();
		let (min, max) = self.sorted.get(index);
		let (low, high) = (min.map(f32x4::splat), max.map(f32x4::splat));
		for next in (0..count).step_by(WIDTH) {
			let group = next..next + WIDTH;
			let mut overlapping = f32x4::from_slice(&self.sorted.min[0][group.clone()]).simd_le(high[0])
				& f32x4::from_slice(&self.sorted.max[0][group.clone()]).simd_ge(low[0]);
			for axis in 1..3 {
				overlapping &= f32x4::from_slice(&self.sorted.min[axis][group.clone()]).simd_le(high[axis])
					& f32x4::from_slice(&self.sorted.max[axis][group.clone()]).simd_ge(low[axis]);
			}
			let mut lanes = overlapping.to_bitmask();
			while lanes != 0 {
				let other = next + lanes.trailing_zeros() as usize;
				lanes &= lanes - 1;
				if other != index && !(self.is_wide[other] && other < index) {
					found([self.sorted.ids[index] as usize, self.sorted.ids[other] as usize]);
				}
			}
		}
	}
}

/// Returns a typical value among the `count` extents that `extent` returns by index: the median of an evenly spaced
/// sample of them, or zero when there are none.
///
/// A median ignores the few boxes that are far larger than the rest, such as a floor under a pile.
fn typical(count: usize, extent: impl Fn(usize) -> f32) -> f32 {
	/// Enough boxes to find the usual size, and few enough to sort on every rebuild.
	const SAMPLES: usize = 64;
	let samples = count.min(SAMPLES);
	if samples == 0 {
		return 0.0;
	}
	let mut sample = [0.0f32; SAMPLES];
	for (slot, value) in sample.iter_mut().enumerate().take(samples) {
		*value = extent(slot * count / samples);
	}
	let sample = &mut sample[..samples];
	sample.sort_unstable_by(f32::total_cmp);
	sample[samples / 2]
}

/// Sorts `keys` that are already nearly in order and returns whether it finished.
///
/// It moves each key back past the keys that belong after it, which is quick while keys are close to their places.
/// It gives up, leaving the keys in some other order, once that has taken more moves than a full sort is worth.
fn repair(keys: &mut [SweepKey]) -> bool {
	/// The moves allowed for each key on average before a full sort is the better choice.
	const MOVES_PER_KEY: usize = 8;
	let mut moves_left = keys.len() * MOVES_PER_KEY;
	for sorted in 1..keys.len() {
		let key = keys[sorted];
		let mut place = sorted;
		while place > 0 && key.cmp(&keys[place - 1]).is_lt() {
			keys[place] = keys[place - 1];
			place -= 1;
		}
		keys[place] = key;
		let moves = sorted - place;
		if moves > moves_left {
			return false;
		}
		moves_left -= moves;
	}
	true
}

/// Returns the axis on which the fewest boxes overlap each other, judged from an evenly spaced sample of `boxes`.
///
/// Counting real overlaps, instead of measuring how far the boxes spread, keeps a few distant boxes from picking an
/// axis that the rest are stacked along. Ties go to the lower axis.
fn least_crowded_axis(boxes: &[StagedBounds]) -> usize {
	/// Enough boxes to tell a crowded axis from a sparse one, and few enough to compare every pair of them.
	const SAMPLES: usize = 64;
	let samples = boxes.len().min(SAMPLES);
	let sample = |index: usize| &boxes[index * boxes.len() / samples];
	let mut overlaps = [0u32; 3];
	for first in 0..samples {
		for second in first + 1..samples {
			let (first, second) = (sample(first), sample(second));
			for axis in 0..3 {
				overlaps[axis] += u32::from(first.min[axis] <= second.max[axis] && second.min[axis] <= first.max[axis]);
			}
		}
	}
	let mut least = 0;
	for axis in 1..3 {
		if overlaps[axis] < overlaps[least] {
			least = axis;
		}
	}
	least
}

#[cfg(test)]
mod tests {
	use math::{Point, Vector};

	use super::*;

	/// Returns a box from its center and half extents.
	fn bounds(center: [f32; 3], half_extents: [f32; 3]) -> AABB {
		AABB::from_center_and_half_extents(Point::from_array(center), Vector::from_array(half_extents))
	}

	/// Returns the pairs a sweep of `boxes` reports, each ordered by identifier and sorted.
	fn swept_pairs(boxes: &[AABB]) -> Vec<(usize, usize)> {
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());
		let mut pairs = Vec::new();
		sweep.overlaps(0..sweep.count(), |[first, second]| {
			pairs.push((first.min(second), first.max(second)))
		});
		pairs.sort_unstable();
		pairs
	}

	/// Returns boxes scattered through a volume by a fixed integer hash, densely enough that many overlap.
	fn scattered_boxes(count: usize) -> Vec<AABB> {
		let unit = |seed: u32| (seed.wrapping_mul(0x9E37_79B9) >> 8) as f32 / (1u32 << 24) as f32;
		(0..count as u32)
			.map(|index| {
				let center = [unit(index * 3) * 12.0, unit(index * 3 + 1) * 4.0, unit(index * 3 + 2) * 9.0];
				let half_extents = [0.2 + unit(index + 500), 0.2 + unit(index + 900), 0.2 + unit(index + 1300)];
				bounds(center, half_extents)
			})
			.collect()
	}

	#[test]
	fn boxes_that_overlap_on_every_axis_are_paired() {
		let boxes = [
			bounds([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
			// Overlaps the first box on every axis.
			bounds([1.5, 0.5, -0.5], [1.0, 1.0, 1.0]),
			// Overlaps the first box on two axes only.
			bounds([0.0, 5.0, 0.0], [1.0, 1.0, 1.0]),
			// Lies inside the first box.
			bounds([0.2, 0.2, 0.2], [0.1, 0.1, 0.1]),
		];

		assert_eq!(swept_pairs(&boxes), [(0, 1), (0, 3)]);
	}

	#[test]
	fn boxes_that_only_touch_are_paired() {
		for axis in 0..3 {
			let mut center = [0.0; 3];
			center[axis] = 2.0;
			let boxes = [bounds([0.0; 3], [1.0; 3]), bounds(center, [1.0; 3])];

			assert_eq!(swept_pairs(&boxes), [(0, 1)]);
		}
	}

	#[test]
	fn sweep_reports_the_pairs_an_exhaustive_test_finds() {
		// More boxes than one group holds, in an order unrelated to their positions.
		let boxes = scattered_boxes(67);
		let expected = exhaustive_pairs(&boxes);

		assert!(expected.len() > boxes.len());
		assert_eq!(swept_pairs(&boxes), expected);
	}

	#[test]
	fn split_ranges_report_the_same_sequence_as_one_scan() {
		let boxes = scattered_boxes(67);
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());
		let mut whole = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| whole.push(pair));

		let mut split = Vec::new();
		for range in [0..9, 9..9, 9..40, 40..sweep.count()] {
			sweep.overlaps(range, |pair| split.push(pair));
		}

		assert_eq!(split, whole);
	}

	/// Returns every pair of `boxes` that overlap, found by testing each pair.
	fn exhaustive_pairs(boxes: &[AABB]) -> Vec<(usize, usize)> {
		let mut pairs = Vec::new();
		for first in 0..boxes.len() {
			for second in first + 1..boxes.len() {
				let (a_min, a_max) = (boxes[first].min().to_array(), boxes[first].max().to_array());
				let (b_min, b_max) = (boxes[second].min().to_array(), boxes[second].max().to_array());
				if (0..3).all(|axis| a_min[axis] <= b_max[axis] && b_min[axis] <= a_max[axis]) {
					pairs.push((first, second));
				}
			}
		}
		pairs
	}

	/// The small boxes [`spread_boxes`] returns, enough for them to be split into cells.
	const SPREAD_BOXES: usize = GRID_BOXES + 100;

	/// Returns small boxes spread over a wide, flat area by a fixed integer hash, with two floors under them: boxes
	/// in many cells, boxes that straddle cells, and boxes too wide for any cell.
	fn spread_boxes() -> Vec<AABB> {
		let unit = |seed: u32| (seed.wrapping_mul(0x9E37_79B9) >> 8) as f32 / (1u32 << 24) as f32;
		let mut boxes = (0..SPREAD_BOXES as u32)
			.map(|index| {
				let center = [
					unit(index * 3) * 200.0,
					unit(index * 3 + 1) * 6.0,
					unit(index * 3 + 2) * 200.0,
				];
				let half_extents = [0.3 + unit(index + 500), 0.3 + unit(index + 900), 0.3 + unit(index + 1300)];
				bounds(center, half_extents)
			})
			.collect::<Vec<_>>();
		boxes.push(bounds([100.0, -0.5, 100.0], [250.0, 0.6, 250.0]));
		boxes.push(bounds([150.0, 0.0, 100.0], [75.0, 0.6, 250.0]));
		boxes
	}

	#[test]
	fn boxes_across_many_cells_pair_once_with_each_other_and_with_wide_boxes() {
		let boxes = spread_boxes();
		let expected = exhaustive_pairs(&boxes);

		// The floors overlap each other and many of the small boxes.
		assert!(expected.contains(&(SPREAD_BOXES, SPREAD_BOXES + 1)));
		assert!(expected.len() > boxes.len());
		assert_eq!(swept_pairs(&boxes), expected);
	}

	#[test]
	fn split_ranges_cover_cells_and_wide_boxes_like_one_scan() {
		let boxes = spread_boxes();
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());
		let mut whole = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| whole.push(pair));

		let mut split = Vec::new();
		let count = sweep.count();
		for range in [
			0..1,
			1..3,
			3..count / 3,
			count / 3..count / 3,
			count / 3..count - 5,
			count - 5..count,
		] {
			sweep.overlaps(range, |pair| split.push(pair));
		}

		assert_eq!(split, whole);
	}

	#[test]
	fn cells_filled_as_several_parts_report_the_same_sequence_as_one_part() {
		let boxes = spread_boxes();
		let mut whole = SweepBounds::default();
		whole.rebuild(boxes.iter().copied().enumerate());
		let mut expected = Vec::new();
		whole.overlaps(0..whole.count(), |pair| expected.push(pair));

		for parts in [2, 3, 7] {
			let mut sweep = SweepBounds::default();
			assert!(sweep.stage(boxes.iter().copied().enumerate(), parts));
			// Parts are filled in any order, as lanes would.
			for part in (0..parts).rev() {
				sweep.fill(part);
			}
			let mut pairs = Vec::new();
			let count = sweep.count();
			for range in [0..count / 2, count / 2..count] {
				sweep.overlaps(range, |pair| pairs.push(pair));
			}

			assert_eq!(count, whole.count());
			assert_eq!(pairs, expected, "with {parts} parts");
		}
	}

	#[test]
	fn few_boxes_have_no_cells_to_fill() {
		let mut sweep = SweepBounds::default();

		assert!(!sweep.stage(scattered_boxes(67).into_iter().enumerate(), 4));
		assert_eq!(sweep.count(), 67);
	}

	#[test]
	fn rebuilding_moved_boxes_reports_the_same_sequence_as_a_first_sweep() {
		let boxes = scattered_boxes(67);
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());

		// The same boxes again, some moved a little and some moved far past their neighbors.
		let moved = boxes
			.iter()
			.enumerate()
			.map(|(index, bounds)| {
				let shift = match index % 5 {
					0 => 0.0,
					4 => 7.5,
					step => step as f32 * -0.15,
				};
				AABB::new(
					bounds.min() + Vector::new(shift, shift * 0.5, 0.0),
					bounds.max() + Vector::new(shift, shift * 0.5, 0.0),
				)
			})
			.collect::<Vec<_>>();
		sweep.rebuild(moved.iter().copied().enumerate());
		let mut repaired = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| repaired.push(pair));

		let mut first = SweepBounds::default();
		first.rebuild(moved.iter().copied().enumerate());
		let mut expected = Vec::new();
		first.overlaps(0..first.count(), |pair| expected.push(pair));

		assert!(!expected.is_empty());
		assert_eq!(repaired, expected);
	}

	#[test]
	fn rebuilding_replaces_the_previous_boxes() {
		let mut sweep = SweepBounds::default();
		sweep.rebuild(scattered_boxes(67).into_iter().enumerate());
		sweep.rebuild(
			[
				bounds([0.0; 3], [1.0; 3]),
				bounds([1.5, 0.0, 0.0], [1.0; 3]),
				bounds([9.0, 0.0, 0.0], [1.0; 3]),
			]
			.into_iter()
			.enumerate(),
		);
		let mut pairs = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| pairs.push(pair));

		assert_eq!(pairs, [[0, 1]]);
	}
}
