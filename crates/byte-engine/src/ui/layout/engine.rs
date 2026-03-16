use std::marker::PhantomData;

use math::{Base as _, Vector2};
use utils::RGBA;

use crate::ui::{
	flow::Location,
	intersection::{build_mouse_click_acceleration, MouseClickAcceleration},
	layout::{IdedElement, RenderElement},
	style::{Color, ConcreteStyle},
};

use super::{
	element::{ElementHandle, Id},
	flow::Size,
	layout_elements,
	query::Fetcher,
	ConcreteElement, Element, LayoutElement,
};

pub struct Engine {
	viewports: Vec<VirtualViewport>,
	cursor_position: Vector2,
	is_clicking: bool,
	clicks: Vec<bool>,
}

impl Engine {
	pub fn new() -> Self {
		Self {
			viewports: Vec::new(),
			cursor_position: Vector2::zero(),
			is_clicking: false,
			clicks: Vec::new(),
		}
	}

	pub fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	/// Evaluates the layout of the given root component and returns a snapshot of the resulting layout.
	pub fn evaluate<'a>(&'a mut self, root: &impl Component) -> Snapshot {
		struct State<'a> {
			id: Id,
			counter: &'a mut u32,
			elements: &'a mut Vec<IdedElement>,
			relations: &'a mut Vec<(Id, Id)>,
		}

		impl<'state> Context for State<'state> {
			type Child<'a>
				= State<'a>
			where
				Self: 'a;

			fn element<'a>(&'a mut self, element: impl Into<ConcreteElement>) -> Self::Child<'a> {
				let id = Id::new(*self.counter).unwrap();

				*self.counter += 1;

				self.elements.push(IdedElement {
					id,
					element: element.into(),
				});

				if id != self.id {
					self.relations.push((self.id, id));
				}

				State {
					id,
					counter: &mut *self.counter,
					elements: &mut *self.elements,
					relations: &mut *self.relations,
				}
			}

			fn component(&mut self, component: &impl Component) {
				component.render(self);
			}
		}

		let mut elements = Vec::new();
		let mut relations = Vec::new();

		let mut counter = 1;

		let mut state = State {
			id: Id::new(counter).unwrap(),
			counter: &mut counter,
			elements: &mut elements,
			relations: &mut relations,
		};

		root.render(&mut state);

		let size = Size::new(1024, 1024);

		let mouse_pos = (self.cursor_position + 1f32) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		let elements = layout_elements(elements, &relations, size);

		let acc = build_mouse_click_acceleration(&elements);

		let snapshot = Snapshot {
			elements,
			relations,
			acceleration: acc,
		};

		while let Some(click) = self.clicks.pop() {
			if click {
				snapshot.click(self.cursor_position);
			}
		}

		snapshot
	}

	/// Renders the given snapshot into a [`Render`] object.
	pub fn render<'a>(&'a mut self, snapshot: Snapshot) -> Render {
		let size = Size::new(1024, 1024);

		let mouse_pos = (self.cursor_position + 1f32) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		let elements = snapshot
			.elements
			.iter()
			.map(|e| {
				let style = if let Some(styler) = e.element.element.styler.as_ref() {
					styler()
				} else {
					ConcreteStyle::default()
				};

				let layer = &style.layers[0];

				let color = match layer.color {
					Color::Value(rgba) => rgba,
					Color::Sample(_) => todo!(),
				};

				RenderElement {
					id: e.element.id.get(),
					position: e.position,
					size: e.size,
					color,
				}
			})
			.collect::<Vec<_>>();

		Render {
			elements,
			relations: snapshot.relations,
		}
	}

	pub fn set_cursor_position(&mut self, v: Vector2) {
		self.cursor_position = v;
	}

	pub fn update_click_state(&mut self, v: bool) {
		self.is_clicking = v;
		self.clicks.push(v);
	}
}

/// A `Snapshot` represents a layout snapshot at a given point in time, including elements and their relationships.
///
/// User interactions, such as mouse clicks or hovers, can be realized against this snapshot.
pub struct Snapshot {
	elements: Vec<LayoutElement>,
	relations: Vec<(Id, Id)>,
	acceleration: MouseClickAcceleration,
}

impl Snapshot {
	pub fn click(&self, mouse_pos: Vector2) {
		let size = Size::new(1024, 1024);

		let mouse_pos = (mouse_pos + 1f32) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		if let Some(id) = self.acceleration.query(Location::new(mouse_pos.x as u32, mouse_pos.y as u32)) {
			if let Some(e) = self.elements.iter().find(|e| e.element.id == Id::new(id).unwrap()) {
				if let Some(on_click) = &e.element.element.on_click {
					on_click();
				}
			}
		}
	}
}

/// A `Render` represents the result of rendering a [`Snapshot`] into a representation suitable for rendering.
#[derive(Clone)]
pub struct Render {
	elements: Vec<RenderElement>,
	relations: Vec<(Id, Id)>,
}

impl Render {
	pub fn root(&self) -> &RenderElement {
		self.elements.iter().find(|e| e.id == 1).unwrap()
	}

	pub fn size(&self) -> usize {
		self.elements.len()
	}

	pub fn elements(&self) -> impl Iterator<Item = &RenderElement> {
		self.elements.iter()
	}
}

pub trait Context: Sized {
	type Child<'a>: Context
	where
		Self: 'a;

	fn element<'a>(&'a mut self, element: impl Into<ConcreteElement>) -> Self::Child<'a>;
	fn component(&mut self, component: &impl Component);
}

struct VirtualViewport;

impl ElementHandle for VirtualViewport {
	fn id(&self) -> Id {
		unimplemented!()
	}
}

pub trait Component {
	fn render(&self, ctx: &mut impl Context);
}

#[cfg(test)]
mod tests {
	use math::{Base, Vector2};

	use super::super::super::{
		components::container::{BaseContainer, ContainerSettings},
		element::Id,
		flow::Size,
		layout::engine::{Context, Engine, VirtualViewport},
		Component,
	};

	struct Bar {
		options: Vec<BarOption>,
	}

	impl Bar {
		pub fn new(options: Vec<BarOption>) -> Self {
			Self { options }
		}
	}

	impl Component for Bar {
		fn render(&self, ctx: &mut impl Context) {
			let mut ctx = ctx.element(BaseContainer::new(ContainerSettings::default().height(32.into())));

			for option in &self.options {
				option.render(&mut ctx);
			}
		}
	}

	struct BarOption {
		label: String,
	}

	impl BarOption {
		pub fn new(label: String) -> Self {
			Self { label }
		}
	}

	impl Component for BarOption {
		fn render(&self, ctx: &mut impl Context) {
			ctx.element(BaseContainer::new(ContainerSettings::default()));
		}
	}

	struct BarList {}

	impl BarList {
		pub fn new() -> Self {
			Self {}
		}
	}

	struct BarListOption {}

	impl BarListOption {
		pub fn new() -> Self {
			Self {}
		}
	}

	struct Application {
		bar: Bar,
	}

	impl Application {
		pub fn new() -> Self {
			let options = vec![
				BarOption::new("File".to_string()),
				BarOption::new("Edit".to_string()),
				BarOption::new("View".to_string()),
			];

			Self { bar: Bar::new(options) }
		}
	}

	impl Component for Application {
		fn render(&self, ctx: &mut impl Context) {
			let mut ctx = ctx.element(BaseContainer::new(ContainerSettings::default()));
			self.bar.render(&mut ctx);
		}
	}
}
