pub mod context;
pub mod engine;
pub mod query;
pub mod snapshot;

use math::{Base as _, Vector2};
use utils::{Box, RGBA};

use super::{
	element::{self, Element, ElementHandle, Id},
	flow::{self, FlowInput, FlowOutput, Location, Location3, Offset, Size},
	primitive::BasePrimitive,
	Primitive,
};
use crate::ui::{
	element::ConcreteElement,
	flow::FlowFunction,
	font::TextSystem,
	primitive::{Primitives, Shapes},
};

/// Describes an element layed out for an screen.
pub(crate) struct LayoutElement {
	pub(crate) id: Id,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) hit_testable: bool,
}

/// Describes an element ready for rendering.
#[derive(Clone)]
pub(crate) struct RenderElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) color: RGBA,
	pub(crate) corner_radius: f32,
}

#[derive(Clone)]
pub(crate) struct RenderTextElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) color: RGBA,
	pub(crate) font_size: f32,
	pub(crate) content: String,
}

fn random_color_from_id(id: u32) -> RGBA {
	let mut state = id.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
	state ^= state >> 16;
	state = state.wrapping_mul(2_246_822_519);
	state ^= state >> 13;

	let r = ((state & 0xFF) as f32) / 255.0;
	let g = (((state >> 8) & 0xFF) as f32) / 255.0;
	let b = (((state >> 16) & 0xFF) as f32) / 255.0;

	RGBA::new(0.25 + r * 0.75, 0.25 + g * 0.75, 0.25 + b * 0.75, 1.0)
}

pub struct IdedElement {
	pub(crate) id: Id,
	pub(crate) element: ConcreteElement,
	pub(crate) path: Vec<String>,
}

impl ElementHandle for IdedElement {
	fn id(&self) -> Id {
		self.id
	}
}

/// Lays out the given elements and returns a vector of layout elements with their calculated positions and sizes for a given viewport.
/// The relation map describes embedded elements.
fn layout_elements<'a>(
	elements: impl AsRef<[IdedElement]>,
	relation_map: &[(Id, Id)],
	available_space: Size,
	text_system: &mut TextSystem,
	frame_allocator: &'a bumpalo::Bump,
) -> Vec<LayoutElement, &'a bumpalo::Bump> {
	let elements = elements.as_ref();
	let mut lelements = Vec::with_capacity_in(elements.len(), frame_allocator);

	if elements.is_empty() {
		return lelements;
	}

	#[derive(Clone, Copy)]
	struct TraversalState {
		available_space: Size,
		offset: Offset,
		depth: i32,
		is_root: bool,
	}

	#[derive(Clone, Copy)]
	struct Context<'a> {
		relation_map: &'a [(Id, Id)],
		root_size: Size,
	}

	fn calculate_element<'a>(
		element: &IdedElement,
		ctx: Context<'a>,
		ts: TraversalState,
		text_system: &mut TextSystem,
	) -> LayoutElement {
		let available_space = if ts.is_root { ctx.root_size } else { ts.available_space };
		let size = calculate_element_size(&element, available_space, text_system);

		let position = Location3::from((ts.offset.into(), clamp_depth(ts.depth)));

		let hit_testable = matches!(element.element.primitive, Primitives::Container(_));

		LayoutElement {
			id: element.id,
			position,
			size,
			hit_testable,
		}
	}

	fn calculate_element_size(element: &IdedElement, available_space: Size, text_system: &mut TextSystem) -> Size {
		match &element.element.primitive {
			Primitives::Container(container) => Shapes::Box {
				half: (container.width, container.height),
				radius: container.corner_radius,
			}
			.bbox(available_space),
			Primitives::Shape(shape) => shape.shape.bbox(available_space),
			Primitives::Text(text) => text_system.measure(text.content(), text.settings().font_size),
		}
	}

	fn layout_element<'a>(
		elements: &[IdedElement],
		lelements: &mut Vec<LayoutElement, &bumpalo::Bump>,
		element: &IdedElement,
		ctx: Context<'a>,
		ts: TraversalState,
		text_system: &mut TextSystem,
		highest_depth: &mut i32,
	) -> Size {
		let p = calculate_element(element, ctx, ts, text_system);

		let size = p.size;
		let mut cursor: Offset = Into::<Location>::into(p.position).into();
		let element_id = p.id;

		match &element.element.primitive {
			Primitives::Container(container) => {
				let flow = container.flow;

				lelements.push(p);

				for layout_reset_layer in [false, true] {
					for &(_, child_id) in ctx.relation_map.iter().filter(|&&(parent_id, _)| parent_id == element_id) {
						let Some(child) = elements.iter().find(|element| element.id == child_id) else {
							continue;
						};
						let reset_layout = resets_layout(child);
						if reset_layout != layout_reset_layer {
							continue;
						}

						let child_available_space = if reset_layout { ctx.root_size } else { size };
						let expected_child_size = calculate_element_size(&child, child_available_space, text_system);
						let flow_output = if reset_layout {
							FlowOutput::new(Offset::new(0, 0), cursor)
						} else {
							flow.call(FlowInput::new(size, cursor, expected_child_size))
						};
						let child_depth = resolve_element_depth(child, ts.depth, *highest_depth);
						*highest_depth = (*highest_depth).max(child_depth);
						let child_size = layout_element(
							elements,
							lelements,
							child,
							ctx,
							TraversalState {
								available_space: child_available_space,
								offset: flow_output.child_offset(),
								depth: child_depth,
								is_root: false,
							},
							text_system,
							highest_depth,
						);

						if !reset_layout {
							cursor = flow_output.next_cursor();
						}
						debug_assert_eq!(expected_child_size, child_size);
					}
				}

				size
			}
			Primitives::Shape(shape) => {
				lelements.push(p);
				size
			}
			Primitives::Text(_) => {
				lelements.push(p);
				size
			}
		}
	}

	fn resets_layout(element: &IdedElement) -> bool {
		matches!(
			&element.element.primitive,
			Primitives::Container(container) if matches!(container.depth, Depth::Absolute(_))
		)
	}

	fn resolve_element_depth(element: &IdedElement, parent_depth: i32, highest_depth: i32) -> i32 {
		match &element.element.primitive {
			Primitives::Container(container) => match container.depth {
				Depth::Relative(depth) => parent_depth.saturating_add(depth),
				Depth::Absolute(depth) => highest_depth.saturating_add(depth),
			},
			_ => parent_depth.saturating_add(1),
		}
	}

	fn clamp_depth(depth: i32) -> u32 {
		depth.max(0) as u32
	}

	let root_id = elements
		.iter()
		.find_map(|element| {
			let has_parent = relation_map.iter().any(|(_, child_id)| *child_id == element.id);
			if has_parent {
				None
			} else {
				Some(element.id)
			}
		})
		.expect("Root container not found");
	let root_index = elements.iter().position(|element| element.id == root_id).unwrap();
	let root = &elements[root_index];
	let mut highest_depth = 0;

	layout_element(
		elements,
		&mut lelements,
		root,
		Context {
			relation_map,
			root_size: Size::new(available_space.x(), available_space.y()),
		},
		TraversalState {
			available_space,
			offset: Offset::new(0, 0),
			depth: 0,
			is_root: true,
		},
		text_system,
		&mut highest_depth,
	);

	lelements
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
	Relative(i32),
	Absolute(i32),
}

impl Depth {
	pub fn relative(depth: i32) -> Self {
		Self::Relative(depth)
	}

	pub fn absolute(depth: i32) -> Self {
		Self::Absolute(depth)
	}
}

impl Default for Depth {
	fn default() -> Self {
		Self::relative(1)
	}
}

impl From<i16> for Depth {
	fn from(value: i16) -> Self {
		Self::relative(value.into())
	}
}

impl From<i32> for Depth {
	fn from(value: i32) -> Self {
		Self::relative(value)
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sizing {
	Relative(u16, u16),
	Absolute(u32),
}

impl Sizing {
	pub fn full() -> Self {
		Self::Relative(1, 1)
	}

	pub fn pixels(value: u32) -> Self {
		Self::Absolute(value)
	}

	pub fn calculate(&self, available: u32) -> u32 {
		match self {
			Sizing::Relative(num, denom) => (available * *num as u32) / *denom as u32,
			Sizing::Absolute(value) => *value,
		}
	}
}

impl Default for Sizing {
	fn default() -> Self {
		Self::full()
	}
}

impl From<u32> for Sizing {
	fn from(val: u32) -> Self {
		Sizing::Absolute(val)
	}
}

#[cfg(test)]
mod tests {
	use math::{Base as _, Vector2};

	use super::super::{
		components::container::Container,
		element::{ElementHandle, Id},
		flow::{self, Location, Location3, Size},
		layout::{ConcreteElement, Depth, Sizing},
		Element,
	};
	use super::layout_elements;
	use crate::ui::{
		font::TextSystem,
		layout::IdedElement,
		primitive::{Primitives, Shapes},
	};

	fn make_elements(elements: impl IntoIterator<Item = Container>) -> Vec<IdedElement> {
		let mut counter = Id::MIN;

		elements
			.into_iter()
			.map(|e| {
				let id = counter;

				counter = counter.checked_add(1).unwrap();

				IdedElement {
					id,
					element: ConcreteElement {
						primitive: Primitives::Container(e),
					},
					path: Vec::new(),
				}
			})
			.collect()
	}

	#[test]
	fn layout_root() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();

		let elements = make_elements([root]);

		let elements = layout_elements(elements, &[], Size::new(1024, 10), &mut TextSystem::new(), &frame_allocator);

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(1024, 10));
	}

	#[test]
	fn layout_root_half_size() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().size(Sizing::Relative(1, 2));

		let elements = make_elements([root]);

		let elements = layout_elements(elements, &[], Size::new(1024, 10), &mut TextSystem::new(), &frame_allocator);

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(512, 5));
	}

	#[test]
	fn layout_half_children() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let a = Container::default().size(Sizing::Relative(1, 2));
		let b = Container::default().size(Sizing::Relative(1, 2));
		let c = Container::default().size(Sizing::Relative(1, 2));
		let d = Container::default().size(Sizing::Relative(1, 2));

		let elements = make_elements([root, a, b, c, d]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];
		let c = &elements[3];
		let d = &elements[4];

		let relations = [(root.id(), a.id()), (a.id(), b.id()), (b.id(), c.id()), (c.id(), d.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(1024, 1024),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 5);

		let element = &elements[0];
		assert_eq!(element.size, Size::new(1024, 1024));
		assert_eq!(element.position, Location3::new(0, 0, 0));

		let element = &elements[1];
		assert_eq!(element.size, Size::new(512, 512));
		assert_eq!(element.position, Location3::new(0, 0, 1));

		let element = &elements[2];
		assert_eq!(element.size, Size::new(256, 256));
		assert_eq!(element.position, Location3::new(0, 0, 2));

		let element = &elements[3];
		assert_eq!(element.size, Size::new(128, 128));
		assert_eq!(element.position, Location3::new(0, 0, 3));

		let element = &elements[4];
		assert_eq!(element.size, Size::new(64, 64));
		assert_eq!(element.position, Location3::new(0, 0, 4));
	}

	#[test]
	fn layout_column() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::column);
		let a = Container::default().size(Sizing::Absolute(64));
		let b = Container::default().size(Sizing::Absolute(64));
		let c = Container::default().size(Sizing::Absolute(64));
		let d = Container::default().size(Sizing::Absolute(64));

		let elements = make_elements([root, a, b, c, d]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];
		let c = &elements[3];
		let d = &elements[4];

		let relations = [
			(root.id(), a.id()),
			(root.id(), b.id()),
			(root.id(), c.id()),
			(root.id(), d.id()),
		];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(1024, 1024),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 5);

		let element = &elements[0];
		assert_eq!(element.size, Size::new(1024, 1024));
		assert_eq!(element.position, Location3::new(0, 0, 0));

		let element = &elements[1];
		assert_eq!(element.size, Size::new(64, 64));
		assert_eq!(element.position, Location3::new(0, 0, 1));

		let element = &elements[2];
		assert_eq!(element.size, Size::new(64, 64));
		assert_eq!(element.position, Location3::new(0, 64, 1));

		let element = &elements[3];
		assert_eq!(element.size, Size::new(64, 64));
		assert_eq!(element.position, Location3::new(0, 128, 1));

		let element = &elements[4];
		assert_eq!(element.size, Size::new(64, 64));
		assert_eq!(element.position, Location3::new(0, 192, 1));
	}

	#[test]
	fn layout_relative_depth_offsets_from_parent_depth() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let child = Container::default().depth(Depth::relative(2));
		let grandchild = Container::default();

		let elements = make_elements([root, child, grandchild]);

		let root = &elements[0];
		let child = &elements[1];
		let grandchild = &elements[2];

		let relations = [(root.id(), child.id()), (child.id(), grandchild.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[0].position, Location3::new(0, 0, 0));
		assert_eq!(elements[1].position, Location3::new(0, 0, 2));
		assert_eq!(elements[2].position, Location3::new(0, 0, 3));
	}

	#[test]
	fn layout_absolute_depth_offsets_from_current_highest_depth() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let regular = Container::default().depth(Depth::relative(3));
		let modal = Container::default().depth(Depth::absolute(1));
		let modal_child = Container::default();

		let elements = make_elements([root, regular, modal, modal_child]);

		let root = &elements[0];
		let regular = &elements[1];
		let modal = &elements[2];
		let modal_child = &elements[3];

		let relations = [
			(root.id(), regular.id()),
			(root.id(), modal.id()),
			(modal.id(), modal_child.id()),
		];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[0].position, Location3::new(0, 0, 0));
		assert_eq!(elements[1].position, Location3::new(0, 0, 3));
		assert_eq!(elements[2].position.z(), 4);
		assert_eq!(elements[3].position.z(), 5);
	}

	#[test]
	fn layout_absolute_depth_siblings_stack_in_layout_order() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let first_modal = Container::default().depth(Depth::absolute(1));
		let second_modal = Container::default().depth(Depth::absolute(1));

		let elements = make_elements([root, first_modal, second_modal]);

		let root = &elements[0];
		let first_modal = &elements[1];
		let second_modal = &elements[2];

		let relations = [(root.id(), first_modal.id()), (root.id(), second_modal.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[0].position, Location3::new(0, 0, 0));
		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position.z(), 2);
	}

	#[test]
	fn layout_absolute_depth_resets_position_to_root_origin() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row_with_gap(10));
		let menu_item = Container::default().width(Sizing::Absolute(20)).height(Sizing::Absolute(20));
		let modal = Container::default()
			.width(Sizing::Absolute(30))
			.height(Sizing::Absolute(30))
			.depth(Depth::absolute(1));

		let elements = make_elements([root, menu_item, modal]);

		let root = &elements[0];
		let menu_item = &elements[1];
		let modal = &elements[2];

		let relations = [(root.id(), menu_item.id()), (root.id(), modal.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(0, 0, 2));
	}

	#[test]
	fn layout_absolute_depth_does_not_advance_parent_flow_cursor() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row_with_gap(10));
		let first = Container::default().width(Sizing::Absolute(20)).height(Sizing::Absolute(20));
		let modal = Container::default()
			.width(Sizing::Absolute(30))
			.height(Sizing::Absolute(30))
			.depth(Depth::absolute(1));
		let second = Container::default().width(Sizing::Absolute(20)).height(Sizing::Absolute(20));

		let elements = make_elements([root, first, modal, second]);

		let root = &elements[0];
		let first = &elements[1];
		let modal_id = elements[2].id();
		let modal = &elements[2];
		let second = &elements[3];

		let relations = [(root.id(), first.id()), (root.id(), modal.id()), (root.id(), second.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(30, 0, 1));
		assert_eq!(elements[3].id, modal_id);
		assert_eq!(elements[3].position, Location3::new(0, 0, 2));
	}

	#[test]
	fn layout_absolute_depth_resolves_after_relative_siblings_even_when_declared_first() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let modal = Container::default().depth(Depth::absolute(1));
		let background = Container::default();

		let elements = make_elements([root, modal, background]);

		let root = &elements[0];
		let modal_id = elements[1].id();
		let background_id = elements[2].id();
		let modal = &elements[1];
		let background = &elements[2];

		let relations = [(root.id(), modal.id()), (root.id(), background.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].id, background_id);
		assert_eq!(elements[1].position.z(), 1);
		assert_eq!(elements[2].id, modal_id);
		assert_eq!(elements[2].position.z(), 2);
	}

	#[test]
	fn layout_centered_column() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::centered_column);
		let a = Container::default().width(Sizing::Absolute(64)).height(Sizing::Absolute(32));
		let b = Container::default().width(Sizing::Absolute(20)).height(Sizing::Absolute(16));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 3);

		let element = &elements[1];
		assert_eq!(element.position, Location3::new(18, 0, 1));
		assert_eq!(element.size, Size::new(64, 32));

		let element = &elements[2];
		assert_eq!(element.position, Location3::new(40, 32, 1));
		assert_eq!(element.size, Size::new(20, 16));
	}

	#[test]
	fn layout_center() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::center);
		let a = Container::default().width(Sizing::Absolute(20)).height(Sizing::Absolute(10));
		let b = Container::default().width(Sizing::Absolute(40)).height(Sizing::Absolute(20));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 80),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 3);

		let element = &elements[1];
		assert_eq!(element.position, Location3::new(40, 35, 1));
		assert_eq!(element.size, Size::new(20, 10));

		let element = &elements[2];
		assert_eq!(element.position, Location3::new(30, 30, 1));
		assert_eq!(element.size, Size::new(40, 20));
	}
}
