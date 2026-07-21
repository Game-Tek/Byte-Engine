use super::{element::Id, flow::Location, layout::LayoutElement};
use crate::ui::flow::{Location3, Size};

#[derive(Clone, Copy, PartialEq, Debug)]
struct QueryElement {
	id: u32,
	position: Location3,
	size: Size,
}

/// The `MouseClickAcceleration` struct provides a uniform-grid index for pointer
/// hit testing.
pub(crate) struct MouseClickAcceleration<'a> {
	cell_size: f32,
	columns: usize,
	rows: usize,
	bounds: (f32, f32),
	elements: Vec<QueryElement, &'a bumpalo::Bump>,
	buckets: Vec<Vec<usize, &'a bumpalo::Bump>, &'a bumpalo::Bump>,
}

impl<'a> MouseClickAcceleration<'a> {
	fn new(layout: Vec<QueryElement, &'a bumpalo::Bump>, frame_allocator: &'a bumpalo::Bump) -> Self {
		if layout.is_empty() {
			let mut buckets = Vec::with_capacity_in(1, frame_allocator);
			buckets.push(Vec::new_in(frame_allocator));
			return Self {
				cell_size: 1.0,
				columns: 1,
				rows: 1,
				bounds: (1.0, 1.0),
				elements: Vec::new_in(frame_allocator),
				buckets,
			};
		}

		let mut max_x: f32 = 0.0;
		let mut max_y: f32 = 0.0;

		for element in layout.iter() {
			max_x = max_x.max(element.position.x() + element.size.x());
			max_y = max_y.max(element.position.y() + element.size.y());
		}

		let bounds = (max_x.max(1.0), max_y.max(1.0));
		let largest_dimension = bounds.0.max(bounds.1);
		let cell_size = (largest_dimension / 32.0).max(1.0);

		let columns = (bounds.0 / cell_size).ceil() as usize;
		let rows = (bounds.1 / cell_size).ceil() as usize;
		let mut buckets = Vec::with_capacity_in(columns * rows, frame_allocator);
		for _ in 0..columns * rows {
			buckets.push(Vec::new_in(frame_allocator));
		}

		for (index, element) in layout.iter().enumerate() {
			if element.size.x() <= 0.0 || element.size.y() <= 0.0 {
				continue;
			}

			let start_col = (element.position.x() / cell_size).floor() as usize;
			let start_row = (element.position.y() / cell_size).floor() as usize;

			// Rectangles use half-open bounds, so an edge on a cell boundary does not occupy the next cell.
			let end_col = ((element.position.x() + element.size.x()) / cell_size).ceil().max(1.0) as usize - 1;
			let end_row = ((element.position.y() + element.size.y()) / cell_size).ceil().max(1.0) as usize - 1;

			for row in start_row..=end_row.min(rows.saturating_sub(1)) {
				for col in start_col..=end_col.min(columns.saturating_sub(1)) {
					let bucket_index = row * columns + col;
					buckets[bucket_index].push(index);
				}
			}
		}

		Self {
			cell_size,
			columns,
			rows,
			bounds,
			elements: layout,
			buckets,
		}
	}

	/// Returns the ID of the topmost element under the pointer position.
	pub(crate) fn query(&self, mouse_position: Location) -> Option<u32> {
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
			if !point_in_layout_element(candidate, mouse_position) {
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

fn point_in_layout_element(element: &QueryElement, point: Location) -> bool {
	let (x, y) = point.into();
	let (left, top) = Into::<Location>::into(element.position).into();
	let right = left + element.size.x();
	let bottom = top + element.size.y();

	x >= left && x < right && y >= top && y < bottom
}

/// Builds an acceleration structure from `layout_containers` output for pointer
/// hit testing.
pub(crate) fn build_mouse_click_acceleration<'a>(
	layout: &[LayoutElement],
	frame_allocator: &'a bumpalo::Bump,
) -> MouseClickAcceleration<'a> {
	let mut query_elements = Vec::with_capacity_in(layout.len(), frame_allocator);
	for e in layout.iter().filter(|e| e.hit_testable) {
		query_elements.push(QueryElement {
			id: e.id.get(),
			position: e.position,
			size: e.size,
		});
	}

	MouseClickAcceleration::new(query_elements, frame_allocator)
}

#[cfg(test)]
mod tests {
	use utils::RGBA;

	use super::{
		super::{
			element::Id,
			flow::{Location, Location3, Size},
			layout::LayoutElement,
		},
		build_mouse_click_acceleration,
	};
	use crate::ui::intersection::{MouseClickAcceleration, QueryElement};

	#[test]
	fn mouse_click_acceleration_hits_topmost_overlapping_element() {
		let frame_allocator = bumpalo::Bump::new();
		let mut layout = Vec::with_capacity_in(3, &frame_allocator);
		layout.push(QueryElement {
			id: 1,
			position: Location3::new(0, 0, 0),
			size: Size::new(200, 200),
		});
		layout.push(QueryElement {
			id: 2,
			position: Location3::new(20, 20, 0),
			size: Size::new(120, 120),
		});
		layout.push(QueryElement {
			id: 3,
			position: Location3::new(40, 40, 0),
			size: Size::new(60, 60),
		});

		let acceleration = MouseClickAcceleration::new(layout, &frame_allocator);

		assert_eq!(acceleration.query(Location::new(50, 50)), Some(3));
		assert_eq!(acceleration.query(Location::new(30, 30)), Some(2));
		assert_eq!(acceleration.query(Location::new(10, 10)), Some(1));
	}

	#[test]
	fn mouse_click_acceleration_returns_none_when_no_hit() {
		let frame_allocator = bumpalo::Bump::new();
		let mut layout = Vec::with_capacity_in(2, &frame_allocator);
		layout.push(QueryElement {
			id: 10,
			position: Location3::new(0, 0, 0),
			size: Size::new(100, 100),
		});
		layout.push(QueryElement {
			id: 11,
			position: Location3::new(150, 150, 0),
			size: Size::new(50, 50),
		});

		let acceleration = MouseClickAcceleration::new(layout, &frame_allocator);

		assert_eq!(acceleration.query(Location::new(125, 125)), None);
		assert_eq!(acceleration.query(Location::new(300, 300)), None);
	}

	#[test]
	fn mouse_click_acceleration_prefers_deeper_elements_over_layout_order() {
		let frame_allocator = bumpalo::Bump::new();
		let mut layout = Vec::with_capacity_in(2, &frame_allocator);
		layout.push(QueryElement {
			id: 20,
			position: Location3::new(0, 0, 3),
			size: Size::new(100, 100),
		});
		layout.push(QueryElement {
			id: 21,
			position: Location3::new(0, 0, 1),
			size: Size::new(100, 100),
		});

		let acceleration = MouseClickAcceleration::new(layout, &frame_allocator);

		assert_eq!(acceleration.query(Location::new(50, 50)), Some(20));
	}

	#[test]
	fn mouse_click_acceleration_preserves_fractional_visual_bounds() {
		let frame_allocator = bumpalo::Bump::new();
		let mut layout = Vec::with_capacity_in(1, &frame_allocator);
		layout.push(QueryElement {
			id: 1,
			position: Location3::new(10.25, 20.5, 0),
			size: Size::new(5.5, 3.25),
		});

		let acceleration = MouseClickAcceleration::new(layout, &frame_allocator);

		assert_eq!(acceleration.query(Location::new(10.24, 21.0)), None);
		assert_eq!(acceleration.query(Location::new(10.25, 20.5)), Some(1));
		assert_eq!(acceleration.query(Location::new(15.749, 23.749)), Some(1));
		assert_eq!(acceleration.query(Location::new(15.75, 22.0)), None);
	}
}
