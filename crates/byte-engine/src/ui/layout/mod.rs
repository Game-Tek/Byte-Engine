pub mod engine;
pub mod query;

use math::{Base as _, Vector2};
use utils::RGBA;

use crate::ui::primitive::Shapes;

use super::{
	element::{self, Element, ElementHandle, Id},
	flow::{self, Location, Location3, Offset, Size},
	layout::query::{ElementResult, Fetcher},
	primitive::BasePrimitive,
	Primitive,
};

pub struct ConcreteElement {
	flow: flow::FlowFunction,
	shape: Shapes,
	on_click: Option<Box<dyn Fn()>>,
}

impl ConcreteElement {
	pub fn new(flow: flow::FlowFunction, shape: Shapes) -> Self {
		Self {
			flow,
			shape,
			on_click: None,
		}
	}

	pub fn on_click(mut self, on_click: Option<Box<dyn Fn()>>) -> Self {
		self.on_click = on_click;
		self
	}
}

#[derive(Clone, Copy)]
/// Describes an element layed out for an screen an ready to be rendered,
pub(crate) struct LayoutElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) color: RGBA,
}

impl ElementHandle for LayoutElement {
	fn id(&self) -> Id {
		Id::new(self.id).unwrap()
	}
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

struct IdedElement {
	id: Id,
	element: ConcreteElement,
}

impl ElementHandle for IdedElement {
	fn id(&self) -> Id {
		self.id
	}
}

/// Lays out the given elements and returns a vector of layout elements with their calculated positions and sizes for a given viewport.
/// The relation map describes embedded elements.
fn layout_elements(elements: &[IdedElement], relation_map: &[(Id, Id)], available_space: Size) -> Vec<LayoutElement> {
	let mut lelements = Vec::with_capacity(elements.len());

	#[derive(Clone, Copy)]
	struct TraversalState {
		available_space: Size,
		offset: Offset,
		depth: u32,
	}

	#[derive(Clone, Copy)]
	struct Context<'a> {
		fetcher: &'a Fetcher<'a, IdedElement>,
		root_size: Size,
	}

	fn calculate_element(element: ElementResult<'_, IdedElement>, ctx: Context, ts: TraversalState) -> LayoutElement {
		let shape = &element.element().element.shape;

		let size = shape.bbox(ts.available_space);

		let position = Location3::from((ts.offset.into(), ts.depth));

		LayoutElement {
			id: element.id().into(),
			position,
			size,
			color: random_color_from_id(element.id().into()),
		}
	}

	fn layout_element(
		elements: &mut Vec<LayoutElement>,
		element: ElementResult<'_, IdedElement>,
		ctx: Context,
		ts: TraversalState,
	) -> LayoutElement {
		let l = calculate_element(element, ctx, ts);

		let available_space = l.size;
		let mut offset: Offset = Into::<Location>::into(l.position).into();

		elements.push(l);

		for child in element.children().elements() {
			let l = layout_element(
				elements,
				child,
				ctx,
				TraversalState {
					available_space,
					offset,
					depth: ts.depth + 1,
				},
			);

			offset = (element.element().element.flow)(offset, l.size);
		}

		l
	}

	let fetcher = Fetcher { elements, relation_map };

	let root = elements
		.iter()
		.find_map(|container| {
			let res = fetcher.get(container.id())?;
			if res.parent().is_none() {
				Some(res)
			} else {
				None
			}
		})
		.expect("Root container not found");

	layout_element(
		&mut lelements,
		root,
		Context {
			fetcher: &fetcher,
			root_size: available_space,
		},
		TraversalState {
			available_space,
			offset: Offset::new(0, 0),
			depth: 0,
		},
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

	use crate::ui::layout::IdedElement;

	use super::super::{
		components::container::{BaseContainer, ContainerSettings},
		element::{ElementHandle, Id},
		flow::{self, Location, Location3, Size},
		layout::{ConcreteElement, Sizing},
		Element,
	};

	use super::layout_elements;

	fn make_elements(elements: &[&dyn Element]) -> Vec<IdedElement> {
		let mut counter = Id::MIN;

		elements
			.iter()
			.map(|e| {
				let id = counter;

				counter = counter.checked_add(1).unwrap();

				IdedElement {
					id,
					element: ConcreteElement {
						flow: e.flow(),
						shape: e.primitive().shape,
						on_click: None,
					},
				}
			})
			.collect()
	}

	#[test]
	fn layout_root() {
		let root = BaseContainer::new(Default::default());

		let elements = make_elements(&[&root as &dyn Element]);

		let elements = layout_elements(&elements, &[], Size::new(1024, 10));

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(1024, 1024));
	}

	#[test]
	fn layout_root_half_size() {
		let root = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

		let elements = make_elements(&[&root as &dyn Element]);

		let elements = layout_elements(&elements, &[], Size::new(1024, 10));

		assert_eq!(elements.len(), 1);

		let element = &elements[0];

		assert_eq!(element.size, Size::new(512, 512));
	}

	#[test]
	fn layout_half_children() {
		let root = BaseContainer::new(Default::default());
		let a = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let b = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let c = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));
		let d = BaseContainer::new(ContainerSettings::default().size(Sizing::Relative(1, 2)));

		let elements = make_elements(&[
			&root as &dyn Element,
			&a as &dyn Element,
			&b as &dyn Element,
			&c as &dyn Element,
			&d as &dyn Element,
		]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];
		let c = &elements[3];
		let d = &elements[4];

		let elements = layout_elements(
			&elements,
			&[(root.id(), a.id()), (a.id(), b.id()), (b.id(), c.id()), (c.id(), d.id())],
			Size::new(1024, 1024),
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
		let root = BaseContainer::new(ContainerSettings::default().flow(flow::column));
		let a = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let b = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let c = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));
		let d = BaseContainer::new(ContainerSettings::default().size(Sizing::Absolute(64)));

		let elements = make_elements(&[
			&root as &dyn Element,
			&a as &dyn Element,
			&b as &dyn Element,
			&c as &dyn Element,
			&d as &dyn Element,
		]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];
		let c = &elements[3];
		let d = &elements[4];

		let elements = layout_elements(
			&elements,
			&[
				(root.id(), a.id()),
				(root.id(), b.id()),
				(root.id(), c.id()),
				(root.id(), d.id()),
			],
			Size::new(1024, 1024),
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
}
