use super::{element::Id, flow::Location, layout::LayoutElement};

/// A uniform-grid spatial index for fast mouse click hit-testing.
pub(crate) struct MouseClickAcceleration {
	cell_size: u32,
	columns: usize,
	rows: usize,
	bounds: (u32, u32),
	elements: Vec<LayoutElement>,
	buckets: Vec<Vec<usize>>,
}

impl MouseClickAcceleration {
	fn from_layout(layout: &[LayoutElement]) -> Self {
		if layout.is_empty() {
			return Self {
				cell_size: 1,
				columns: 1,
				rows: 1,
				bounds: (1, 1),
				elements: Vec::new(),
				buckets: vec![Vec::new()],
			};
		}

		let mut max_x = 0;
		let mut max_y = 0;

		for element in layout {
			max_x = max_x.max(element.position.x().saturating_add(element.size.x()));
			max_y = max_y.max(element.position.y().saturating_add(element.size.y()));
		}

		let bounds = (max_x.max(1), max_y.max(1));
		let largest_dimension = bounds.0.max(bounds.1);
		let cell_size = (largest_dimension / 32).max(1);

		let columns = bounds.0.div_ceil(cell_size) as usize;
		let rows = bounds.1.div_ceil(cell_size) as usize;
		let mut buckets = vec![Vec::new(); columns * rows];

		for (index, element) in layout.iter().enumerate() {
			if element.size.x() == 0 || element.size.y() == 0 {
				continue;
			}

			let start_col = (element.position.x() / cell_size) as usize;
			let start_row = (element.position.y() / cell_size) as usize;

			let end_x = element.position.x().saturating_add(element.size.x()).saturating_sub(1);
			let end_y = element.position.y().saturating_add(element.size.y()).saturating_sub(1);

			let end_col = (end_x / cell_size) as usize;
			let end_row = (end_y / cell_size) as usize;

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
			elements: layout.to_vec(),
			buckets,
		}
	}

	/// Returns the id of the top-most element under the mouse position.
	pub(crate) fn query(&self, mouse_position: Location) -> Option<u32> {
		let (x, y) = mouse_position.into();
		if x >= self.bounds.0 || y >= self.bounds.1 {
			return None;
		}

		let col = (x / self.cell_size) as usize;
		let row = (y / self.cell_size) as usize;
		if col >= self.columns || row >= self.rows {
			return None;
		}

		let bucket_index = row * self.columns + col;
		let candidates = &self.buckets[bucket_index];

		for &candidate_index in candidates.iter().rev() {
			let candidate = self.elements[candidate_index];
			if point_in_layout_element(candidate, mouse_position) {
				return Some(candidate.id);
			}
		}

		None
	}
}

fn point_in_layout_element(element: LayoutElement, point: Location) -> bool {
	let (x, y) = point.into();
	let (left, top) = Into::<Location>::into(element.position).into();
	let right = left.saturating_add(element.size.x());
	let bottom = top.saturating_add(element.size.y());

	x >= left && x < right && y >= top && y < bottom
}

/// Builds an acceleration structure from `layout_containers` output for mouse click hit-testing.
pub(crate) fn build_mouse_click_acceleration(layout: &[LayoutElement]) -> MouseClickAcceleration {
	MouseClickAcceleration::from_layout(layout)
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

	#[test]
	fn mouse_click_acceleration_hits_topmost_overlapping_element() {
		let layout = vec![
			LayoutElement {
				id: 1,
				position: Location3::new(0, 0, 0),
				size: Size::new(200, 200),
				color: RGBA::black(),
			},
			LayoutElement {
				id: 2,
				position: Location3::new(20, 20, 0),
				size: Size::new(120, 120),
				color: RGBA::black(),
			},
			LayoutElement {
				id: 3,
				position: Location3::new(40, 40, 0),
				size: Size::new(60, 60),
				color: RGBA::black(),
			},
		];

		let acceleration = build_mouse_click_acceleration(&layout);

		assert_eq!(acceleration.query(Location::new(50, 50)), Some(3));
		assert_eq!(acceleration.query(Location::new(30, 30)), Some(2));
		assert_eq!(acceleration.query(Location::new(10, 10)), Some(1));
	}

	#[test]
	fn mouse_click_acceleration_returns_none_when_no_hit() {
		let layout = vec![
			LayoutElement {
				id: 10,
				position: Location3::new(0, 0, 0),
				size: Size::new(100, 100),
				color: RGBA::black(),
			},
			LayoutElement {
				id: 11,
				position: Location3::new(150, 150, 0),
				size: Size::new(50, 50),
				color: RGBA::black(),
			},
		];

		let acceleration = build_mouse_click_acceleration(&layout);

		assert_eq!(acceleration.query(Location::new(125, 125)), None);
		assert_eq!(acceleration.query(Location::new(300, 300)), None);
	}
}
