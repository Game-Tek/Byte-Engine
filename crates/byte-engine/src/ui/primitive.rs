use super::{
	flow::{Location, Size},
	layout::Sizing,
	style::ConcreteStyle,
	transform::Transform,
	visual::Visual,
};
use crate::ui::{
	components::{curve::Curve, image::Image, shape::Shape, text::Text, text_field::TextField},
	Container,
};

#[derive(Clone)]
pub enum Shapes {
	Triangle { vertices: [Location; 3] },
	Circle { radius: f32 },
	Box { half: Scale, radius: f32, exponent: f32 },
}

pub trait CustomShape {
	fn name(&self) -> Option<&str>;
}

pub trait Primitive {
	fn shape(&self) -> Shapes;
	fn style(&self) -> &ConcreteStyle;
	fn transform(&self) -> &Transform;
	fn visual(&self) -> &Visual;
}

// #[derive(Clone)]
pub struct BasePrimitive {
	pub(crate) shape: Shapes,
	pub(crate) style: ConcreteStyle,
}

impl BasePrimitive {
	pub fn new(shape: Shapes) -> Self {
		BasePrimitive {
			shape,
			style: ConcreteStyle::default(),
		}
	}
}

impl Primitive for BasePrimitive {
	fn shape(&self) -> Shapes {
		self.shape.clone()
	}

	fn style(&self) -> &ConcreteStyle {
		&self.style
	}

	fn transform(&self) -> &Transform {
		&Transform::IDENTITY
	}

	fn visual(&self) -> &Visual {
		&Visual::DEFAULT
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Events {
	Actuated,
	Scrolled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
	Escape,
	Backspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEdit {
	Inserted(char),
	Deleted(char),
}

impl TextEdit {
	pub fn apply_to(self, content: &mut String) {
		match self {
			Self::Inserted(character) => content.push(character),
			Self::Deleted(character) => {
				if content.ends_with(character) {
					content.pop();
				}
			}
		}
	}
}

pub enum Primitives {
	Container(Container),
	Shape(Shape),
	Curve(Curve),
	Image(Image),
	Text(Text),
	TextField(TextField),
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

impl From<Image> for Primitives {
	fn from(image: Image) -> Self {
		Primitives::Image(image)
	}
}

impl From<TextField> for Primitives {
	fn from(text_field: TextField) -> Self {
		Primitives::TextField(text_field)
	}
}

impl Primitive for Primitives {
	fn shape(&self) -> Shapes {
		match self {
			Primitives::Container(container) => Shapes::Box {
				half: (container.width, container.height),
				radius: container.corner_radius,
				exponent: container.corner_exponent,
			},
			Primitives::Image(image) => Shapes::Box {
				half: (image.width, image.height),
				radius: 0.0,
				exponent: 2.0,
			},
			Primitives::Text(_) | Primitives::TextField(_) => Shapes::Box {
				half: (Sizing::Absolute(0), Sizing::Absolute(0)),
				radius: 0.0,
				exponent: 2.0,
			},
			Primitives::Shape(shape) => shape.shape.clone(),
			Primitives::Curve(_) => Shapes::Box {
				half: (Sizing::Absolute(0), Sizing::Absolute(0)),
				radius: 0.0,
				exponent: 2.0,
			},
		}
	}

	fn style(&self) -> &ConcreteStyle {
		match self {
			Primitives::Container(container) => container.style_ref(),
			Primitives::Image(image) => image.style_ref(),
			Primitives::Text(text) => text.style_ref(),
			Primitives::TextField(text_field) => text_field.style_ref(),
			Primitives::Shape(shape) => shape.style_ref(),
			Primitives::Curve(curve) => curve.style_ref(),
		}
	}

	fn transform(&self) -> &Transform {
		match self {
			Primitives::Container(container) => container.transform_ref(),
			Primitives::Image(image) => image.transform_ref(),
			Primitives::Text(text) => text.transform_ref(),
			Primitives::TextField(text_field) => text_field.transform_ref(),
			Primitives::Shape(shape) => shape.transform_ref(),
			Primitives::Curve(curve) => curve.transform_ref(),
		}
	}

	fn visual(&self) -> &Visual {
		match self {
			Primitives::Container(container) => container.visual_ref(),
			Primitives::Image(image) => image.visual_ref(),
			Primitives::Text(text) => text.visual_ref(),
			Primitives::TextField(text_field) => text_field.visual_ref(),
			Primitives::Shape(shape) => shape.visual_ref(),
			Primitives::Curve(curve) => curve.visual_ref(),
		}
	}
}
