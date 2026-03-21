pub mod engine;
pub mod query;

use math::{Base as _, Vector2};
use utils::{Box, RGBA};

use crate::ui::{
	components::container::OnEventFunction,
	element::ConcreteElement,
	flow::FlowFunction,
	font::TextSystem,
	primitive::{Primitives, Shapes},
	style::{Color, ConcreteStyle, Styler},
};

use super::{
	element::{self, Element, ElementHandle, Id},
	flow::{self, FlowInput, Location, Location3, Offset, Size},
	primitive::BasePrimitive,
	Primitive,
};

/// Describes an element layed out for an screen.
pub(crate) struct LayoutElement {
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) element: IdedElement,
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
}

impl ElementHandle for IdedElement {
	fn id(&self) -> Id {
		self.id
	}
}

/// Lays out the given elements and returns a vector of layout elements with their calculated positions and sizes for a given viewport.
/// The relation map describes embedded elements.
fn layout_elements<'a>(
	mut elements: Vec<IdedElement>,
	relation_map: &'a [(Id, Id)],
	available_space: Size,
	text_system: &mut TextSystem,
) -> Vec<LayoutElement> {
	let mut lelements = Vec::with_capacity(elements.len());

	#[derive(Clone, Copy)]
	struct TraversalState {
		available_space: Size,
		offset: Offset,
		depth: u32,
	}

	#[derive(Clone, Copy)]
	struct Context<'a> {
		relation_map: &'a [(Id, Id)],
		root_size: Size,
	}

	fn calculate_element<'a>(
		element: IdedElement,
		ctx: Context<'a>,
		ts: TraversalState,
		text_system: &mut TextSystem,
	) -> LayoutElement {
		let available_space = if ts.depth == 0 { ctx.root_size } else { ts.available_space };
		let size = calculate_element_size(&element, available_space, text_system);

		let position = Location3::from((ts.offset.into(), ts.depth));

		LayoutElement { position, size, element }
	}

	fn calculate_element_size(element: &IdedElement, available_space: Size, text_system: &mut TextSystem) -> Size {
		match &element.element.primitive {
			Primitives::Container(container) => Shapes::Box {
				half: (container.settings.width, container.settings.height),
				radius: container.settings.corner_radius,
			}
			.bbox(available_space),
			Primitives::Shape(shape) => shape.shape.bbox(available_space),
			Primitives::Text(text) => text_system.measure(text.content(), text.settings().font_size),
		}
	}

	fn layout_element<'a>(
		elements: &mut Vec<IdedElement>,
		lelements: &mut Vec<LayoutElement>,
		element: IdedElement,
		ctx: Context<'a>,
		ts: TraversalState,
		text_system: &mut TextSystem,
	) -> Size {
		let p = calculate_element(element, ctx, ts, text_system);

		let size = p.size;
		let mut cursor: Offset = Into::<Location>::into(p.position).into();
		let element_id = p.element.id;

		match &p.element.element.primitive {
			Primitives::Container(container) => {
				let flow = container.settings.flow;

				let child_ids = ctx
					.relation_map
					.iter()
					.filter_map(
						|&(parent_id, child_id)| {
							if parent_id == element_id {
								Some(child_id)
							} else {
								None
							}
						},
					)
					.collect::<Vec<_>>();

				lelements.push(p);

				for child_id in child_ids {
					let Some(child_index) = elements.iter().position(|element| element.id == child_id) else {
						continue;
					};
					let child = elements.swap_remove(child_index);
					let expected_child_size = calculate_element_size(&child, size, text_system);
					let flow_output = flow.call(FlowInput::new(size, cursor, expected_child_size));
					let child_size = layout_element(
						elements,
						lelements,
						child,
						ctx,
						TraversalState {
							available_space: size,
							offset: flow_output.child_offset(),
							depth: ts.depth + 1,
						},
						text_system,
					);

					cursor = flow_output.next_cursor();
					debug_assert_eq!(expected_child_size, child_size);
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
	let root = elements.swap_remove(root_index);

	layout_element(
		&mut elements,
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
		},
		text_system,
	);

	lelements
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

impl Into<Sizing> for u32 {
	fn into(self) -> Sizing {
		Sizing::Absolute(self)
	}
}

#[cfg(test)]
mod tests {
	use math::{Base as _, Vector2};

	use crate::ui::{
		font::TextSystem,
		layout::IdedElement,
		primitive::{Primitives, Shapes},
	};

	use super::super::{
		components::container::{Container, ContainerSettings},
		element::{ElementHandle, Id},
		flow::{self, Location, Location3, Size},
		layout::{ConcreteElement, Sizing},
		Element,
	};

	use super::layout_elements;

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
				}
			})
			.collect()
	}

	#[test]
	fn layout_root() {
		let root = Container::new(Default::default());

		let elements = make_elements([root]);

		let elements = layout_elements(elements, &[], Size::new(1024, 10), &mut TextSystem::new());

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(1024, 10));
	}

	#[test]
	fn layout_root_half_size() {
		let root = Container::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

		let elements = make_elements([root]);

		let elements = layout_elements(elements, &[], Size::new(1024, 10), &mut TextSystem::new());

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(512, 5));
	}

	#[test]
	fn layout_half_children() {
		let root = Container::new(Default::default());
		let a = Container::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let b = Container::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let c = Container::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let d = Container::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

		let elements = make_elements([root, a, b, c, d]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];
		let c = &elements[3];
		let d = &elements[4];

		let relations = [(root.id(), a.id()), (a.id(), b.id()), (b.id(), c.id()), (c.id(), d.id())];

		let elements = layout_elements(elements, &relations, Size::new(1024, 1024), &mut TextSystem::new());

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
		let root = Container::new(ContainerSettings::default().flow(flow::column));
		let a = Container::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let b = Container::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let c = Container::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let d = Container::new(ContainerSettings::default().size(Sizing::Absolute(64)));

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

		let elements = layout_elements(elements, &relations, Size::new(1024, 1024), &mut TextSystem::new());

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
	fn layout_centered_column() {
		let root = Container::new(ContainerSettings::default().flow(flow::centered_column));
		let a = Container::new(
			ContainerSettings::default()
				.width(Sizing::Absolute(64))
				.height(Sizing::Absolute(32)),
		);
		let b = Container::new(
			ContainerSettings::default()
				.width(Sizing::Absolute(20))
				.height(Sizing::Absolute(16)),
		);

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(elements, &relations, Size::new(100, 100), &mut TextSystem::new());

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
		let root = Container::new(ContainerSettings::default().flow(flow::center));
		let a = Container::new(
			ContainerSettings::default()
				.width(Sizing::Absolute(20))
				.height(Sizing::Absolute(10)),
		);
		let b = Container::new(
			ContainerSettings::default()
				.width(Sizing::Absolute(40))
				.height(Sizing::Absolute(20)),
		);

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(elements, &relations, Size::new(100, 80), &mut TextSystem::new());

		assert_eq!(elements.len(), 3);

		let element = &elements[1];
		assert_eq!(element.position, Location3::new(40, 35, 1));
		assert_eq!(element.size, Size::new(20, 10));

		let element = &elements[2];
		assert_eq!(element.position, Location3::new(30, 30, 1));
		assert_eq!(element.size, Size::new(40, 20));
	}
}
