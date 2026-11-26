use super::{flow::Location, layout::LayoutElement};
use crate::ui::{
	components::container::Sector,
	flow::{Location3, Size},
};

#[derive(Clone, Copy, PartialEq, Debug)]
struct QueryElement {
	id: u64,
	position: Location3,
	size: Size,
	/// Index of the polyline a curve is hit along, within its bounds.
	curve: Option<u32>,
	/// The sector a container is shaped as, hit within its bounds.
	sector: Option<Sector>,
}

/// The `HitCurve` struct describes a hit-testable curve as a polyline in layout units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct HitCurve {
	pub(crate) id: u64,
	pub(crate) half_width: f32,
	/// The polyline's points, as a range of the shared point list.
	pub(crate) first: u32,
	pub(crate) count: u32,
}

/// The `MouseClickAcceleration` struct provides a uniform-grid index for pointer
/// hit testing.
#[derive(Default)]
pub(crate) struct MouseClickAcceleration {
	cell_size: f32,
	columns: usize,
	rows: usize,
	bounds: (f32, f32),
	elements: Vec<QueryElement>,
	curves: Vec<HitCurve>,
	points: Vec<Location>,
	buckets: Vec<Vec<usize>>,
	/// Capacity kept for the next update's comparison so unchanged frames allocate nothing.
	scratch: Vec<QueryElement>,
	/// Entries moved by [`Self::patch`] with their previous bounds, until [`Self::commit`] re-indexes them.
	moved: Vec<(usize, Location3, Size)>,
}

impl MouseClickAcceleration {
	/// Reuses the pointer index while clipped hit geometry stays unchanged.
	///
	/// `sectors` holds the sector each element of `layout` is shaped as, by position, one entry per element.
	/// `curves` follow the order of the curve entries in `layout`; each names the
	/// polyline, in `points`, that its element is hit along within its bounds.
	pub(crate) fn update(
		&mut self,
		layout: &[LayoutElement],
		sectors: &[Option<Sector>],
		curves: &[HitCurve],
		points: &[Location],
	) {
		let mut next_curve = 0u32;
		let mut elements = std::mem::take(&mut self.scratch);
		elements.clear();
		debug_assert_eq!(sectors.len(), layout.len(), "Every layout element needs a sector entry.");
		elements.extend(
			layout
				.iter()
				.enumerate()
				.filter(|(_, element)| element.hit_testable)
				.map(|(index, element)| {
					let curve = curves
						.get(next_curve as usize)
						.filter(|curve| curve.id == element.id.get())
						.map(|_| {
							next_curve += 1;
							next_curve - 1
						});
					QueryElement {
						id: element.id.get(),
						position: element.position,
						size: element.size,
						curve,
						sector: sectors[index],
					}
				}),
		);
		// Patches left uncommitted by an abandoned refresh still need their cells rebuilt.
		if self.moved.is_empty() && self.elements == elements && self.curves == curves && self.points == points {
			self.scratch = elements;
			return;
		}
		// Swap so the previous entries become the next comparison's scratch.
		self.scratch = std::mem::replace(&mut self.elements, elements);
		self.curves.clear();
		self.curves.extend_from_slice(curves);
		self.points.clear();
		self.points.extend_from_slice(points);
		self.rebuild();
	}

	/// Moves one retained entry; call [`Self::commit`] once every patch of a frame is in.
	///
	/// A curve's polyline is overwritten in place, so it must keep its point count.
	/// Returns `false` when the index needs a rebuild instead: the entry is unknown,
	/// the polyline changed length, or the entry now reaches past the grid.
	pub(crate) fn patch(&mut self, offset: usize, position: Location3, size: Size, curve: Option<(f32, &[Location])>) -> bool {
		let bounds = self.bounds;
		let Some(element) = self.elements.get_mut(offset) else {
			return false;
		};
		// Nothing is written before every check passes: a half-applied patch would leave the entries
		// matching the next update's list, which then keeps the stale grid.
		let previous = (element.position, element.size);
		let moved = previous != (position, size);
		if moved && (position.x() + size.x() > bounds.0 || position.y() + size.y() > bounds.1) {
			return false;
		}
		if let Some((half_width, points)) = curve {
			let Some(hit_curve) = element.curve.and_then(|index| self.curves.get_mut(index as usize)) else {
				return false;
			};
			if hit_curve.count as usize != points.len() {
				return false;
			}
			hit_curve.half_width = half_width;
			let first = hit_curve.first as usize;
			self.points[first..first + points.len()].copy_from_slice(points);
		}
		element.position = position;
		element.size = size;
		if moved {
			self.moved.push((offset, previous.0, previous.1));
		}
		true
	}

	/// Re-indexes the entries patched since the last commit.
	///
	/// A few moved entries change only the cells they leave and enter. Once most
	/// entries moved, refilling every cell is cheaper than searching each one.
	pub(crate) fn commit(&mut self) {
		if self.moved.is_empty() {
			return;
		}
		if self.moved.len() * 4 >= self.elements.len() {
			self.moved.clear();
			self.refill_buckets();
			return;
		}
		let (cell_size, columns, rows) = (self.cell_size, self.columns, self.rows);
		let mut moved = std::mem::take(&mut self.moved);
		for &(offset, position, size) in &moved {
			if let Some((cols, rows)) = cell_span(position, size, cell_size, columns, rows) {
				for row in rows {
					for col in cols.clone() {
						let bucket = &mut self.buckets[row * columns + col];
						if let Some(slot) = bucket.iter().position(|&index| index == offset) {
							bucket.swap_remove(slot);
						}
					}
				}
			}
			let element = &self.elements[offset];
			if let Some((cols, rows)) = cell_span(element.position, element.size, cell_size, columns, rows) {
				for row in rows {
					for col in cols.clone() {
						self.buckets[row * columns + col].push(offset);
					}
				}
			}
		}
		moved.clear();
		self.moved = moved;
	}

	/// Refills grid cells without discarding their capacity between layout changes.
	fn rebuild(&mut self) {
		let mut max_x: f32 = 0.0;
		let mut max_y: f32 = 0.0;

		for element in &self.elements {
			max_x = max_x.max(element.position.x() + element.size.x());
			max_y = max_y.max(element.position.y() + element.size.y());
		}

		let bounds = (max_x.max(1.0), max_y.max(1.0));
		let largest_dimension = bounds.0.max(bounds.1);
		let cell_size = (largest_dimension / 32.0).max(1.0);

		let columns = (bounds.0 / cell_size).ceil() as usize;
		let rows = (bounds.1 / cell_size).ceil() as usize;
		// Keep spare cells when the grid shrinks so cyclic resizes need no allocation.
		self.buckets.resize_with(self.buckets.len().max(columns * rows), Vec::new);
		self.cell_size = cell_size;
		self.columns = columns;
		self.rows = rows;
		self.bounds = bounds;
		self.moved.clear();
		self.refill_buckets();
	}

	/// Clears every cell and indexes each entry again for the current grid.
	fn refill_buckets(&mut self) {
		for bucket in &mut self.buckets {
			bucket.clear();
		}
		let (cell_size, columns, rows) = (self.cell_size, self.columns, self.rows);
		for (index, element) in self.elements.iter().enumerate() {
			let Some((cols, rows)) = cell_span(element.position, element.size, cell_size, columns, rows) else {
				continue;
			};
			for row in rows {
				for col in cols.clone() {
					self.buckets[row * columns + col].push(index);
				}
			}
		}
	}

	/// Finds the frontmost surface at a point, skipping one element such as a held drag source.
	pub(crate) fn query_excluding(&self, mouse_position: Location, excluded: Option<u64>) -> Option<u64> {
		let (x, y) = mouse_position.into();
		if x < 0.0 || y < 0.0 || x >= self.bounds.0 || y >= self.bounds.1 {
			return None;
		}

		let col = (x / self.cell_size).floor() as usize;
		let row = (y / self.cell_size).floor() as usize;
		if col >= self.columns || row >= self.rows {
			return None;
		}

		let bucket_index = row * self.columns + col;
		let candidates = &self.buckets[bucket_index];
		let mut top_most: Option<(usize, &QueryElement)> = None;

		for &candidate_index in candidates {
			let candidate = &self.elements[candidate_index];
			if Some(candidate.id) == excluded || !hits(candidate, &self.curves, &self.points, mouse_position) {
				continue;
			}

			top_most = match top_most {
				Some((top_index, top_element))
					if top_element.position.z() > candidate.position.z()
						|| (top_element.position.z() == candidate.position.z() && top_index > candidate_index) =>
				{
					Some((top_index, top_element))
				}
				_ => Some((candidate_index, candidate)),
			};
		}

		top_most.map(|(_, element)| element.id)
	}
}

/// The grid cells a rectangle occupies, as column and row ranges; `None` for an empty one.
fn cell_span(
	position: Location3,
	size: Size,
	cell_size: f32,
	columns: usize,
	rows: usize,
) -> Option<(std::ops::RangeInclusive<usize>, std::ops::RangeInclusive<usize>)> {
	if size.x() <= 0.0 || size.y() <= 0.0 {
		return None;
	}
	// Bounds can start left of or above the viewport; those cells are simply not indexed.
	let start_col = (position.x().max(0.0) / cell_size).floor() as usize;
	let start_row = (position.y().max(0.0) / cell_size).floor() as usize;
	// Rectangles use half-open bounds, so an edge on a cell boundary does not occupy the next cell.
	let end_col = ((position.x() + size.x()) / cell_size).ceil().max(1.0) as usize - 1;
	let end_row = ((position.y() + size.y()) / cell_size).ceil().max(1.0) as usize - 1;
	Some((
		start_col..=end_col.min(columns.saturating_sub(1)),
		start_row..=end_row.min(rows.saturating_sub(1)),
	))
}

/// Tests a point against a surface's bounds and, for a curve, its polyline.
fn hits(element: &QueryElement, curves: &[HitCurve], points: &[Location], point: Location) -> bool {
	if !point_in_layout_element(element, point) {
		return false;
	}
	if let Some(sector) = element.sector {
		let (x, y) = point.into();
		let (left, top) = Into::<Location>::into(element.position).into();
		let half = (element.size.x() * 0.5, element.size.y() * 0.5);
		if !sector.contains(x - left - half.0, y - top - half.1, half.0.min(half.1)) {
			return false;
		}
	}
	let Some(curve) = element.curve.and_then(|index| curves.get(index as usize)) else {
		return true;
	};
	let polyline = &points[curve.first as usize..(curve.first + curve.count) as usize];
	polyline_distance(polyline, point).is_some_and(|distance| distance <= curve.half_width)
}

fn point_in_layout_element(element: &QueryElement, point: Location) -> bool {
	let (x, y) = point.into();
	let (left, top) = Into::<Location>::into(element.position).into();
	let right = left + element.size.x();
	let bottom = top + element.size.y();

	x >= left && x < right && y >= top && y < bottom
}

/// Distance from a point to the nearest span of a polyline, or to its only point.
fn polyline_distance(polyline: &[Location], point: Location) -> Option<f32> {
	let (x, y) = point.into();
	let distance_to = |location: Location| (x - location.x()).hypot(y - location.y());
	let mut best = f32::INFINITY;
	if polyline.len() == 1 {
		return Some(distance_to(polyline[0]));
	}
	for span in polyline.windows(2) {
		let (ax, ay) = span[0].into();
		let (bx, by) = span[1].into();
		let (dx, dy) = (bx - ax, by - ay);
		let length_squared = dx * dx + dy * dy;
		let t = if length_squared <= 0.0 {
			0.0
		} else {
			(((x - ax) * dx + (y - ay) * dy) / length_squared).clamp(0.0, 1.0)
		};
		best = best.min((x - (ax + dx * t)).hypot(y - (ay + dy * t)));
	}
	best.is_finite().then_some(best)
}

#[cfg(test)]
mod tests {

	use super::super::flow::{Location, Location3, Size};
	use crate::ui::element::Id;
	use crate::ui::intersection::{MouseClickAcceleration, QueryElement};
	use crate::ui::layout::LayoutElement;
	use crate::ui::{Context, ElementContext, Engine, Sector, UiPoint};

	/// Six petals stacked in one absolute-depth dial, probed at the middle of each petal's ring, while
	/// the dial is turned and scaled as an opening animation would leave it.
	#[test]
	fn every_petal_of_a_stacked_sector_ring_is_hit() {
		for (rotation, scale) in [(0.0, 1.0), (0.0, 0.9), (-0.3, 1.0), (-0.3, 0.8)] {
			petal_ring_probe(rotation, scale);
		}
	}

	fn petal_ring_probe(rotation: f32, scale: f32) {
		use std::f32::consts::{FRAC_PI_2, TAU};
		let mut allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx
				.element("root")
				.container(|c| c.width(1920.into()).height(1080.into()))
				.await;
			let mut row = root
				.element("row")
				.container(|c| c.width(300.into()).height(32.into()).clip(false))
				.await;
			let _button = row
				.element("button")
				.container(|c| c.width(96.into()).height(32.into()).hit_testable(true))
				.await;
			// Parked off-screen with collapsed petals, as a menu is before it opens.
			let mut dial = row
				.element("dial")
				.container(|c| {
					c.depth(crate::ui::layout::Depth::absolute(3))
						.absolute_position(2000, 2000)
						.size(236.into())
						.clip(false)
						.opacity(0.0)
				})
				.await;
			let mut petals = std::vec::Vec::new();
			for index in 0..6 {
				let middle = -FRAC_PI_2 + index as f32 * TAU / 6.0;
				petals.push(
					dial.element(format!("petal{index}"))
						.container(|c| {
							c.absolute_position(0, 0)
								.size(236.into())
								.clip(false)
								.hit_testable(true)
								.sector(Sector::new(middle, 0.0, 0.4).inset(3.0))
						})
						.await,
				);
			}
			ctx.render().await;
			// Open: move the dial under the button, transform it, and grow the petals.
			dial.update_container(|c| {
				c.position((1530, 46))
					.opacity(1.0)
					.transform(crate::ui::Transform::identity().rotate(rotation).scale(scale))
			})
			.await;
			for (index, petal) in petals.iter_mut().enumerate() {
				let middle = -FRAC_PI_2 + index as f32 * TAU / 6.0;
				petal
					.update_container(|c| c.sector(Some(Sector::new(middle - TAU / 12.0, TAU / 6.0, 0.4).inset(3.0))))
					.await;
			}
			loop {
				ctx.render().await;
			}
		});
		for _ in 0..3 {
			let _ = engine.evaluate(Size::new(1920, 1080), &allocator);
			allocator.reset();
		}
		let at = |x: f32, y: f32| engine.hit(UiPoint::new(x / 960.0 - 1.0, 1.0 - y / 540.0));
		// Nothing sits above the dial.
		assert_eq!(at(1648.0, 16.0), None, "above the dial");
		let mut found = std::vec::Vec::new();
		for index in 0..6 {
			let middle = -FRAC_PI_2 + index as f32 * TAU / 6.0;
			found.push(at(1648.0 + 83.0 * middle.cos(), 164.0 + 83.0 * middle.sin()));
		}
		assert!(
			found.iter().all(Option::is_some) && found.windows(2).all(|pair| pair[0] != pair[1]),
			"rotation {rotation} scale {scale}: petal probes hit {found:?}"
		);
		let button = at(48.0, 16.0);
		assert!(button.is_some() && !found.contains(&button), "the button");
	}

	#[test]
	fn sector_containers_are_hit_inside_their_wedge_only() {
		let mut allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			// A quarter ring in a 100 by 100 square at the origin, sweeping from right to down.
			let _wedge = root
				.element("wedge")
				.container(|c| {
					c.absolute_position(0, 0)
						.size(100.into())
						.hit_testable(true)
						.sector(Sector::new(0.0, std::f32::consts::FRAC_PI_2, 0.5).inset(6.0))
				})
				.await;
			loop {
				ctx.render().await;
			}
		});
		let _ = engine.evaluate(Size::new(200, 200), &allocator);
		allocator.reset();

		// Layout units map to normalized coordinates with y flipped.
		let at = |x: f32, y: f32| engine.hit(UiPoint::new(x / 100.0 - 1.0, 1.0 - y / 100.0));
		// In the ring, down-right of the center.
		assert!(at(50.0 + 30.0, 50.0 + 30.0).is_some());
		// Same radius, other quadrants, so outside the sweep.
		assert!(at(50.0 - 30.0, 50.0 + 30.0).is_none());
		assert!(at(50.0 + 30.0, 50.0 - 30.0).is_none());
		// In the hole and past the outer radius, both still inside the square bounds.
		assert!(at(50.0 + 10.0, 50.0 + 10.0).is_none());
		assert!(at(50.0 + 45.0, 50.0 + 45.0).is_none());
		// Within the inset of the straight edge along the x axis, and just past it.
		assert!(at(50.0 + 40.0, 50.0 + 4.0).is_none());
		assert!(at(50.0 + 40.0, 50.0 + 8.0).is_some());
	}

	/// The topmost element under a point wins: the deepest first, then the one laid out last, and every element's
	/// bounds keep their fractions.
	#[test]
	fn mouse_click_acceleration_hits_topmost_overlapping_element() {
		let query = |elements: &[(u64, Location3, Size)], points: &[((f32, f32), Option<u64>)]| {
			let mut acceleration = MouseClickAcceleration {
				elements: elements
					.iter()
					.map(|&(id, position, size)| QueryElement {
						id,
						position,
						size,
						curve: None,
						sector: None,
					})
					.collect(),
				..Default::default()
			};
			acceleration.rebuild();
			for &((x, y), expected) in points {
				assert_eq!(
					acceleration.query_excluding(Location::new(x, y), None),
					expected,
					"at ({x}, {y})"
				);
			}
		};
		query(
			&[
				(1, Location3::new(0, 0, 0), Size::new(200, 200)),
				(2, Location3::new(20, 20, 0), Size::new(120, 120)),
				(3, Location3::new(40, 40, 0), Size::new(60, 60)),
			],
			&[((50.0, 50.0), Some(3)), ((30.0, 30.0), Some(2)), ((10.0, 10.0), Some(1))],
		);
		query(
			&[
				(20, Location3::new(0, 0, 3), Size::new(100, 100)),
				(21, Location3::new(0, 0, 1), Size::new(100, 100)),
			],
			&[((50.0, 50.0), Some(20))],
		);
		query(
			&[(1, Location3::new(10.25, 20.5, 0), Size::new(5.5, 3.25))],
			&[
				((10.24, 21.0), None),
				((10.25, 20.5), Some(1)),
				((15.749, 23.749), Some(1)),
				((15.75, 22.0), None),
			],
		);
	}

	/// A target patched past the grid's right or bottom edge is refused before anything is written,
	/// so the next update sees the stale entry and rebuilds the grid around the new bounds.
	#[test]
	fn a_patch_past_the_grid_leaves_the_entry_for_the_rebuild() {
		let element = |x: f32, y: f32| LayoutElement {
			id: Id::new(1).unwrap(),
			index: 0,
			position: Location3::new(x, y, 0),
			size: Size::new(10.0, 10.0),
			display_size: Size::new(10.0, 10.0),
			content: Size::new(10.0, 10.0),
			hit_testable: true,
		};
		for (x, y) in [(95.0, 0.0), (0.0, 95.0)] {
			let mut acceleration = MouseClickAcceleration::default();
			acceleration.update(&[element(0.0, 0.0)], &[None], &[], &[]);
			assert_eq!(acceleration.query_excluding(Location::new(5.0, 5.0), None), Some(1));

			assert!(!acceleration.patch(0, Location3::new(x, y, 0), Size::new(10.0, 10.0), None));
			acceleration.update(&[element(x, y)], &[None], &[], &[]);

			assert_eq!(acceleration.query_excluding(Location::new(x + 5.0, y + 5.0), None), Some(1));
			assert_eq!(acceleration.query_excluding(Location::new(5.0, 5.0), None), None);
		}
	}
}
