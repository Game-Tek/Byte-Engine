use crate::ui::{
	components::{shape::Shape, text::Text},
	Container,
};

use super::{
	flow::{Location, Size},
	layout::Sizing,
	style::Style,
};

#[derive(Clone)]
pub enum Shapes {
	Triangle { vertices: [Location; 3] },
	Circle { radius: f32 },
	Box { half: Scale, radius: f32 },
}

pub trait CustomShape {
	fn name(&self) -> Option<&str>;
}

pub trait Primitive {
	fn shape(&self) -> Shapes;
	fn style(&self) -> &dyn Style;
}

// #[derive(Clone)]
pub struct BasePrimitive {
	pub(crate) shape: Shapes,
}

impl BasePrimitive {
	pub fn new(shape: Shapes) -> Self {
		BasePrimitive { shape }
	}
}

impl Primitive for BasePrimitive {
	fn shape(&self) -> Shapes {
		self.shape.clone()
	}

	fn style(&self) -> &dyn Style {
		unimplemented!()
	}
}

type Scale = (Sizing, Sizing);

impl Shapes {
	pub fn bbox(&self, available_space: Size) -> Size {
		match self {
			Self::Box { half, .. } => Size::new(half.0.calculate(available_space.x()), half.1.calculate(available_space.y())),
			_ => todo!(),
		}
	}

	/// Returns the coordinates for the optical center of the shape.
	fn center(&self) {}

	/// Returns the coordinates for the geometrical center of the shape.
	fn geo_center(&self) {}
}

pub enum Events {
	HoverBegin {},
	HoverEnd {},
	FocusBegin {},
	FocusEnd {},
	Actuate {},
}

pub enum Primitives {
	Container(Container),
	Shape(Shape),
	Text(Text),
}

impl From<Container> for Primitives {
	fn from(container: Container) -> Self {
		Primitives::Container(container)
	}
}

impl From<Text> for Primitives {
	fn from(text: Text) -> Self {
		Primitives::Text(text)
	}
}

impl Primitive for Primitives {
	fn shape(&self) -> Shapes {
		match self {
			Primitives::Container(container) => Shapes::Box {
				half: (container.settings.width, container.settings.height),
				radius: container.settings.corner_radius,
			},
			Primitives::Text(_) => Shapes::Box {
				half: (Sizing::Absolute(0), Sizing::Absolute(0)),
				radius: 0.0,
			},
			Primitives::Shape(shape) => shape.shape.clone(),
		}
	}

	fn style(&self) -> &dyn Style {
		unimplemented!()
	}
}
