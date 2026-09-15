use super::{
	UiPoint,
	element::Id,
	flow::Location,
	layout::{Geometry, LayoutElement},
};
use crate::ui::flow::{Location3, Size};

#[derive(Clone, Copy, PartialEq, Debug)]
struct QueryElement {
	id: u32,
	position: Location3,
	size: Size,
	/// Index of the polyline a curve is hit along, within its bounds.
	curve: Option<u32>,
}

/// The `HitCurve` struct describes a hit-testable curve as a polyline in layout units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct HitCurve {
	pub(crate) id: u32,
	pub(crate) half_width: f32,
	/// The polyline's points, as a range of the shared point list.
	pub(crate) first: u32,
	pub(crate) count: u32,
}

/// The `HitTest` struct keeps submitted UI geometry available between frames.
///
/// Refresh it with [`crate::ui::layout::snapshot::Snapshot::retain_hit_test`]
/// after preparing a render. Then use [`Self::query`] before action publication;
/// queries borrow retained geometry and never run layout or allocate.
#[derive(Default)]
pub struct HitTest {
	elements: Vec<QueryElement>,
	curves: Vec<HitCurve>,
	points: Vec<Location>,
	size: [f32; 2],
}

impl HitTest {
	/// Converts normalized window coordinates to this snapshot's layout units.
	///
	/// The layout origin is at the top left. Positions outside the viewport stay
	/// outside, so a drag captured by [`crate::ui::Engine::press`] can finish beyond its source.
	pub fn layout_position(&self, position: UiPoint) -> UiPoint {
		UiPoint::new(
			(position.x + 1.0) * 0.5 * self.size[0],
			(1.0 - position.y) * 0.5 * self.size[1],
		)
	}

	/// Returns a retained surface's visible bounds in layout units.
	///
	/// These bounds include visual transforms and clipping from the submitted
	/// snapshot. Use them with [`Self::layout_position`] to preserve a pointer's
	/// offset inside a drag source.
	pub fn bounds(&self, id: Id) -> Option<Geometry> {
		self.elements
			.iter()
			.find(|element| element.id == id.get())
			.map(|element| Geometry::new(element.position, element.size))
	}

	/// Returns the frontmost surface at normalized window coordinates.
	/// A missing hit passes through to a lower input context. The snapshot keeps
	/// stable IDs, so the receiving UI must still reject removed targets.
	pub fn query(&self, position: UiPoint) -> Option<Id> {
		let point = self.layout_position(position);
		let point = Location::new(point.x, point.y);
		self.elements
			.iter()
			.rev()
			.find(|element| hits(element, &self.curves, &self.points, point))
			.and_then(|element| Id::new(element.id))
	}
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
}

impl MouseClickAcceleration {
	/// Copies only hit geometry into reusable storage, preserving draw priority.
	pub(super) fn retain(&self, target: &mut HitTest, size: Size) {
		target.size = [size.x(), size.y()];
		target.elements.clear();
		target.elements.extend_from_slice(&self.elements);
		target.curves.clear();
		target.curves.extend_from_slice(&self.curves);
		target.points.clear();
		target.points.extend_from_slice(&self.points);
		// Stable sorting preserves layout order for surfaces at equal depth.
		target.elements.sort_by_key(|element| element.position.z());
	}

	/// Reuses the pointer index while clipped hit geometry stays unchanged.
	///
	/// `curves` follow the order of the curve entries in `layout`; each names the
	/// polyline, in `points`, that its element is hit along within its bounds.
	pub(crate) fn update(&mut self, layout: &[LayoutElement], curves: &[HitCurve], points: &[Location]) {
		let mut next_curve = 0u32;
		let elements = layout.iter().filter(|element| element.hit_testable).map(|element| {
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
			}
		});
		let elements = elements.collect::<Vec<_>>();
		if self.elements == elements && self.curves == curves && self.points == points {
			return;
		}
		self.elements = elements;
		self.curves.clear();
		self.curves.extend_from_slice(curves);
		self.points.clear();
		self.points.extend_from_slice(points);
		self.rebuild();
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
		for bucket in &mut self.buckets {
			bucket.clear();
		}

		for (index, element) in self.elements.iter().enumerate() {
			if element.size.x() <= 0.0 || element.size.y() <= 0.0 {
				continue;
			}

			// Bounds can start left of or above the viewport; those cells are simply not indexed.
			let start_col = (element.position.x().max(0.0) / cell_size).floor() as usize;
			let start_row = (element.position.y().max(0.0) / cell_size).floor() as usize;

			// Rectangles use half-open bounds, so an edge on a cell boundary does not occupy the next cell.
			let end_col = ((element.position.x() + element.size.x()) / cell_size).ceil().max(1.0) as usize - 1;
			let end_row = ((element.position.y() + element.size.y()) / cell_size).ceil().max(1.0) as usize - 1;

			for row in start_row..=end_row.min(rows.saturating_sub(1)) {
				for col in start_col..=end_col.min(columns.saturating_sub(1)) {
					let bucket_index = row * columns + col;
					self.buckets[bucket_index].push(index);
				}
			}
		}

		self.cell_size = cell_size;
		self.columns = columns;
		self.rows = rows;
		self.bounds = bounds;
	}

	/// Returns the ID of the topmost element under the pointer position.
	pub(crate) fn query(&self, mouse_position: Location) -> Option<u32> {
		self.query_excluding(mouse_position, None)
	}

	/// Finds the frontmost surface at a point, skipping one element such as a held drag source.
	pub(crate) fn query_excluding(&self, mouse_position: Location, excluded: Option<u32>) -> Option<u32> {
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

/// Tests a point against a surface's bounds and, for a curve, its polyline.
fn hits(element: &QueryElement, curves: &[HitCurve], points: &[Location], point: Location) -> bool {
	if !point_in_layout_element(element, point) {
		return false;
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
	use utils::RGBA;

	use super::super::flow::{Location, Location3, Size};
	use crate::ui::intersection::{MouseClickAcceleration, QueryElement};
	use crate::ui::{Container, Context, ElementContext, Engine, UiPoint};

	#[test]
	fn retained_layout_coordinates_and_bounds_support_drag_offsets_after_frame_reset() {
		let mut allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default().hit_testable(false));
				let _source = root.element("source").container(
					Container::default()
						.absolute_position(20, 30)
						.width(80.into())
						.height(40.into()),
				);
				loop {
					ctx.render().await;
				}
			})
		});
		let mut hits = super::HitTest::default();
		{
			let snapshot = engine.evaluate(Size::new(200, 100), &allocator);
			snapshot.retain_hit_test(&mut hits);
		}
		allocator.reset();

		let pointer = UiPoint::new(-0.5, 0.0);
		let source = hits.query(pointer).unwrap();
		let bounds = hits.bounds(source).unwrap();
		assert_eq!(
			(bounds.x(), bounds.y(), bounds.width(), bounds.height()),
			(20.0, 30.0, 80.0, 40.0)
		);
		assert_eq!(hits.layout_position(pointer), UiPoint::new(50.0, 50.0));
		assert_eq!(hits.layout_position(UiPoint::new(2.0, -2.0)), UiPoint::new(300.0, 150.0));
	}

	#[test]
	fn mouse_click_acceleration_hits_topmost_overlapping_element() {
		let layout = vec![
			QueryElement {
				id: 1,
				position: Location3::new(0, 0, 0),
				size: Size::new(200, 200),
				curve: None,
			},
			QueryElement {
				id: 2,
				position: Location3::new(20, 20, 0),
				size: Size::new(120, 120),
				curve: None,
			},
			QueryElement {
				id: 3,
				position: Location3::new(40, 40, 0),
				size: Size::new(60, 60),
				curve: None,
			},
		];

		let mut acceleration = MouseClickAcceleration {
			elements: layout,
			..Default::default()
		};
		acceleration.rebuild();

		assert_eq!(acceleration.query(Location::new(50, 50)), Some(3));
		assert_eq!(acceleration.query(Location::new(30, 30)), Some(2));
		assert_eq!(acceleration.query(Location::new(10, 10)), Some(1));
	}

	#[test]
	fn mouse_click_acceleration_returns_none_when_no_hit() {
		let layout = vec![
			QueryElement {
				id: 10,
				position: Location3::new(0, 0, 0),
				size: Size::new(100, 100),
				curve: None,
			},
			QueryElement {
				id: 11,
				position: Location3::new(150, 150, 0),
				size: Size::new(50, 50),
				curve: None,
			},
		];

		let mut acceleration = MouseClickAcceleration {
			elements: layout,
			..Default::default()
		};
		acceleration.rebuild();

		assert_eq!(acceleration.query(Location::new(125, 125)), None);
		assert_eq!(acceleration.query(Location::new(300, 300)), None);
	}

	#[test]
	fn mouse_click_acceleration_prefers_deeper_elements_over_layout_order() {
		let layout = vec![
			QueryElement {
				id: 20,
				position: Location3::new(0, 0, 3),
				size: Size::new(100, 100),
				curve: None,
			},
			QueryElement {
				id: 21,
				position: Location3::new(0, 0, 1),
				size: Size::new(100, 100),
				curve: None,
			},
		];

		let mut acceleration = MouseClickAcceleration {
			elements: layout,
			..Default::default()
		};
		acceleration.rebuild();

		assert_eq!(acceleration.query(Location::new(50, 50)), Some(20));
	}

	#[test]
	fn mouse_click_acceleration_preserves_fractional_visual_bounds() {
		let layout = vec![QueryElement {
			id: 1,
			position: Location3::new(10.25, 20.5, 0),
			size: Size::new(5.5, 3.25),
			curve: None,
		}];

		let mut acceleration = MouseClickAcceleration {
			elements: layout,
			..Default::default()
		};
		acceleration.rebuild();

		assert_eq!(acceleration.query(Location::new(10.24, 21.0)), None);
		assert_eq!(acceleration.query(Location::new(10.25, 20.5)), Some(1));
		assert_eq!(acceleration.query(Location::new(15.749, 23.749)), Some(1));
		assert_eq!(acceleration.query(Location::new(15.75, 22.0)), None);
	}
}
