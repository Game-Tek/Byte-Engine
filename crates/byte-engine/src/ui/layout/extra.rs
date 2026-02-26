#[derive(Debug, Clone, Copy, PartialEq)]
enum Scalings {
	Fill,
	Fractional{
		value: f32,
	},
	Fixed {
		value: u32,
	},
	Fit,
}

impl Default for Scalings {
	fn default() -> Self {
		Scalings::Fill
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Direction {
	Positive,
	Negative,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Flow {
	Vertical,
	Horizontal,
}

struct Box {
	scaling_x: Scalings,
	scaling_y: Scalings,
	direction_x: Direction,
	direction_y: Direction,
	flow: Flow,
	children: Vec<Box>,
}

impl Box {
	fn fixed(width: u32, height: u32) -> Box {
		Box { 
			scaling_x: Scalings::Fixed { value: width },
			scaling_y: Scalings::Fixed { value: height },
			direction_x: Direction::Positive,
			direction_y: Direction::Negative,
			flow: if width > height { Flow::Horizontal } else { Flow::Vertical },
			children: Vec::new(),
		}
	}

	fn new() -> Box {
		Box {
			scaling_x: Scalings::Fill,
			scaling_y: Scalings::Fill,
			direction_x: Direction::Positive,
			direction_y: Direction::Negative,
			flow: Flow::Vertical,
			children: Vec::new(),
		}
	}

	fn add_child(&mut self, child: Box) {
		self.children.push(child);
	}
}

struct LayoutBox {
	x: i32,
	y: i32,
	width: u32,
	height: u32,
	children: Vec<LayoutBox>,
	index: Option<usize>,
}

impl LayoutBox {
	fn width(&self) -> u32 {
		self.width
	}

	fn height(&self) -> u32 {
		self.height
	}
}

fn evaluate_layout(root: &Box) -> LayoutBox {
	let width = match root.scaling_x {
		Scalings::Fill => 0,
		Scalings::Fractional { value } => 0,
		Scalings::Fixed { value } => value,
		Scalings::Fit => 0,
	};

	let height = match root.scaling_y {
		Scalings::Fill => 0,
		Scalings::Fractional { value } => 0,
		Scalings::Fixed { value } => value,
		Scalings::Fit => 0,
	};

	let pointer_x = match root.direction_x {
		Direction::Positive => -(width as i32 / 2),
		Direction::Negative => width as i32 / 2,
	};

	let pointer_y = match root.direction_y {
		Direction::Positive => -(height as i32 / 2),
		Direction::Negative => height as i32 / 2,
	};

	let mut children = Vec::with_capacity(root.children.len());
	
	let mut pointers = (pointer_x, pointer_y);

	let flow = match root.flow {
		Flow::Vertical => (0, 1),
		Flow::Horizontal => (1, 0),
	};

	for (i, child) in root.children.iter().enumerate() {
		let (layout, ptrs) = evaluate_layout_internal(child, (width, height), pointers, flow, i);
		children.push(layout);
		pointers = ptrs
	}

	let root = LayoutBox {
		x: 0, y: 0,
		width, height,
		children,
		index: None,
	};

	root
}

fn evaluate_layout_internal(this: &Box, extent: (u32, u32), pointers: (i32, i32), flow: (i32, i32), index: usize) -> (LayoutBox, (i32, i32)) {
	let (parent_width, parent_height) = extent;
	let (pointer_x, pointer_y) = pointers;
	let (flow_x, flow_y) = flow;

	let width = match this.scaling_x {
		Scalings::Fill => parent_width,
		Scalings::Fractional { value } => (parent_width as f32 * value) as u32,
		Scalings::Fixed { value } => value,
		Scalings::Fit => 0,
	};

	let height = match this.scaling_y {
		Scalings::Fill => parent_height,
		Scalings::Fractional { value } => (parent_height as f32 * value) as u32,
		Scalings::Fixed { value } => value,
		Scalings::Fit => 0,
	};

	let (signed_width, signed_height) = {
		let x = match this.direction_x {
			Direction::Positive => width as i32,
			Direction::Negative => -(width as i32),
		};

		let y = match this.direction_y {
			Direction::Positive => height as i32,
			Direction::Negative => -(height as i32),
		};

		(x, y)
	};

	let pointer_delta = (signed_width * flow_x, signed_height * flow_y);
	let (pointer_delta_x, pointer_delta_y) = pointer_delta;

	let (x, pointer_x) = (pointer_x + (signed_width / 2) as i32, pointer_x + pointer_delta_x);
	let (y, pointer_y) = (pointer_y + (signed_height / 2) as i32, pointer_y + pointer_delta_y);

	let mut children = Vec::with_capacity(this.children.len());

	let mut pointers = (pointer_x, pointer_y);

	let flow = match this.flow {
		Flow::Vertical => (0, 1),
		Flow::Horizontal => (1, 0),
	};

	for (i, child) in this.children.iter().enumerate() {
		let (layout, ptrs) = evaluate_layout_internal(child, (width, height), pointers, flow, i);
		children.push(layout);
		pointers = ptrs
	}

	let root = LayoutBox {
		x, y,
		width, height,
		children,
		index: Some(index),
	};

	(root, (pointer_x, pointer_y))
}

/// Test collisions with the layout boxes.
/// Returns None if no element was hit, None if the element is zero size, or Some if an element was hit.
fn evaluate_hit_test(element: &LayoutBox, x: i32, y: i32) -> Option<&LayoutBox> {
	let (semi_width, semi_height) = (element.width as i32 / 2, element.height as i32 / 2);

	if (x > (element.x - semi_width) && x < (element.x + semi_width)) && (y > (element.y - semi_height) && y < (element.y + semi_height)) {
		for (i, child) in element.children.iter().enumerate() {
			if let Some(c) = evaluate_hit_test(child, x, y) {
				return Some(c);
			}
		}

		return Some(element);
	}

	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_box() {
		let b = Box::fixed(1920, 1080);
		assert_eq!(b.scaling_x, Scalings::Fixed { value: 1920 });
		assert_eq!(b.scaling_y, Scalings::Fixed { value: 1080 });
		assert_eq!(b.direction_x, Direction::Positive);
		assert_eq!(b.direction_y, Direction::Negative);
		assert_eq!(b.flow, Flow::Horizontal);
		assert_eq!(b.children.len(), 0);
	}

	#[test]
	fn test_layout() {
		let mut root = Box::fixed(1920, 1080);
		let mut child = Box::fixed(100, 100);
		child.scaling_x = Scalings::Fractional { value: 0.5 };
		child.scaling_y = Scalings::Fill;
		root.add_child(child);

		let layout = evaluate_layout(&root);
		assert_eq!(layout.width(), 1920);
		assert_eq!(layout.height(), 1080);
		assert_eq!(layout.index, None);
		assert_eq!(layout.children.len(), 1);

		assert_eq!(layout.children[0].width(), 960);
		assert_eq!(layout.children[0].height(), 1080);
		assert_eq!(layout.children[0].index, Some(0));
	}

	#[test]
	fn test_layout_row_children() {
		let mut root = Box::fixed(1920, 1080);
		let mut child_a = Box::fixed(100, 100);
		child_a.scaling_x = Scalings::Fractional { value: 0.5 };
		child_a.scaling_y = Scalings::Fill;
		root.add_child(child_a);

		let mut child_b = Box::fixed(100, 100);
		child_b.scaling_x = Scalings::Fractional { value: 0.5 };
		child_b.scaling_y = Scalings::Fill;
		root.add_child(child_b);

		let layout = evaluate_layout(&root);
		assert_eq!(layout.width(), 1920);
		assert_eq!(layout.height(), 1080);
		assert_eq!(layout.children.len(), 2);

		assert_eq!(layout.children[0].x, -480);
		assert_eq!(layout.children[0].y, 0);
		assert_eq!(layout.children[0].width(), 960);
		assert_eq!(layout.children[0].height(), 1080);
		assert_eq!(layout.children[0].index, Some(0));

		assert_eq!(layout.children[1].x, 480);
		assert_eq!(layout.children[1].y, 0);
		assert_eq!(layout.children[1].width(), 960);
		assert_eq!(layout.children[1].height(), 1080);
		assert_eq!(layout.children[1].index, Some(1));
	}

	#[test]
	fn test_layout_column_children() {
		let mut root = Box::fixed(1000, 1000);
		let mut child_a = Box::fixed(100, 100);
		child_a.scaling_x = Scalings::Fill;
		child_a.scaling_y = Scalings::Fractional { value: 0.5 };
		root.add_child(child_a);

		let mut child_b = Box::fixed(100, 100);
		child_b.scaling_x = Scalings::Fill;
		child_b.scaling_y = Scalings::Fractional { value: 0.5 };
		root.add_child(child_b);

		let layout = evaluate_layout(&root);
		assert_eq!(layout.width(), 1000);
		assert_eq!(layout.height(), 1000);
		assert_eq!(layout.children.len(), 2);

		assert_eq!(layout.children[0].x, 0);
		assert_eq!(layout.children[0].y, 250);
		assert_eq!(layout.children[0].width(), 1000);
		assert_eq!(layout.children[0].height(), 500);
		assert_eq!(layout.children[0].index, Some(0));

		assert_eq!(layout.children[1].x, 0);
		assert_eq!(layout.children[1].y, -250);
		assert_eq!(layout.children[1].width(), 1000);
		assert_eq!(layout.children[1].height(), 500);
		assert_eq!(layout.children[1].index, Some(1));
	}

	#[test]
	fn test_hit_zero_size() {
		let root = LayoutBox {
			x: 0, y: 0,
			width: 0, height: 0,
			children: Vec::new(),
			index: None,
		};

		let hit = evaluate_hit_test(&root, 0, 0);

		assert_eq!(hit.is_some(), false);
	}

	#[test]
	fn test_hit_single_box() {
		let root = LayoutBox {
			x: 0, y: 0,
			width: 100, height: 100,
			children: Vec::new(),
			index: None,
		};

		let hit = evaluate_hit_test(&root, 0, 0);

		assert_eq!(hit.is_some(), true);
	}

	#[test]
	fn test_hit_single_box_outside() {
		let root = LayoutBox {
			x: 0, y: 0,
			width: 100, height: 100,
			children: Vec::new(),
			index: None,
		};

		let hit = evaluate_hit_test(&root, 101, 101);

		assert_eq!(hit.is_some(), false);
	}

	#[test]
	fn test_hit_single_box_child() {
		let root = LayoutBox {
			x: 0, y: 0,
			width: 100, height: 100,
			children: vec![
				LayoutBox {
					x: 0, y: 0,
					width: 50, height: 50,
					children: Vec::new(),
					index: Some(0),
				}
			],
			index: None,
		};

		let hit = evaluate_hit_test(&root, 0, 0).unwrap();

		assert_eq!(hit.index, Some(0));
	}
}