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
	Text { content: String, font: String, size: u32 },
	// Custom(Box<dyn CustomShape>),
}

pub trait CustomShape {
	fn name(&self) -> Option<&str>;
}

pub trait Primitive {
	fn shape(&self) -> Shapes;
	fn style(&self) -> &dyn Style;
}

#[derive(Clone)]
pub struct BasePrimitive {
	shape: Shapes,
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
