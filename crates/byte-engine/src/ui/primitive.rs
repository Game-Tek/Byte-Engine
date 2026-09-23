use super::{
	flow::{Location, Size},
	layout::Sizing,
	style::ConcreteStyle,
	transform::Transform,
	visual::Visual,
};
use crate::ui::{
	Container,
	components::{curve::Curve, image::Image, path::Path, shape::Shape, text::Text, text_field::TextField},
};

#[derive(Clone, PartialEq)]
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
			Self::Circle { radius } => Size::new(radius * 2.0, radius * 2.0),
			Self::Triangle { vertices } => {
				let min_x = vertices.iter().map(Location::x).fold(f32::INFINITY, f32::min);
				let max_x = vertices.iter().map(Location::x).fold(f32::NEG_INFINITY, f32::max);
				let min_y = vertices.iter().map(Location::y).fold(f32::INFINITY, f32::min);
				let max_y = vertices.iter().map(Location::y).fold(f32::NEG_INFINITY, f32::max);
				Size::new(max_x - min_x, max_y - min_y)
			}
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
	/// The pointer was pressed on this element and the engine holds it until
	/// release or cancellation. Delivered to the surface under the press.
	Grabbed,
	/// The held element moved past the drag threshold. Delivered to the source
	/// once per evaluation while the pointer moves, with
	/// [`super::UiEvent::delta`] set to the offset from the press point in layout units.
	Dragged,
	/// A source was released over this element or one of its descendants.
	/// Delivered to the target under the release point, then to each ancestor,
	/// with [`super::UiEvent::source`] set. The source never receives its own drop.
	Dropped,
	/// A grab ended by release or cancellation. Delivered to the source after any
	/// [`Self::Dropped`] the release produced, with [`super::UiEvent::source`]
	/// set to the surface the drag was dropped on, if any.
	DragEnded,
	/// The pointer moved onto this surface or one of its descendants. Delivered
	/// once per evaluation in which the surface under the pointer changed, to
	/// the new surface and then to each ancestor that did not already contain
	/// the pointer. A held drag source is skipped, so a target under a dragged
	/// item still hears about the pointer.
	PointerEntered,
	/// The pointer left this surface and all of its descendants. Delivered to
	/// the previous surface and then to each ancestor that no longer contains
	/// the pointer, before any [`Self::PointerEntered`] of the same evaluation.
	PointerExited,
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
	Path(Path),
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

impl From<Path> for Primitives {
	fn from(path: Path) -> Self {
		Primitives::Path(path)
	}
}

impl From<TextField> for Primitives {
	fn from(text_field: TextField) -> Self {
		Primitives::TextField(text_field)
	}
}

impl Primitives {
	/// Returns the style the engine writes a declaration's or an edit's layers into.
	pub(crate) fn style_mut(&mut self) -> &mut ConcreteStyle {
		match self {
			Primitives::Container(container) => &mut container.style,
			Primitives::Image(image) => &mut image.style,
			Primitives::Text(text) => &mut text.style,
			Primitives::TextField(text_field) => &mut text_field.style,
			Primitives::Shape(shape) => &mut shape.style,
			Primitives::Curve(curve) => &mut curve.style,
			Primitives::Path(path) => &mut path.style,
		}
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
				half: (Sizing::pixels(0.0), Sizing::pixels(0.0)),
				radius: 0.0,
				exponent: 2.0,
			},
			Primitives::Shape(shape) => shape.outline(),
			Primitives::Curve(_) | Primitives::Path(_) => Shapes::Box {
				half: (Sizing::pixels(0.0), Sizing::pixels(0.0)),
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
			Primitives::Path(path) => path.style_ref(),
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
			Primitives::Path(path) => path.transform_ref(),
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
			Primitives::Path(path) => path.visual_ref(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::Shapes;
	use crate::ui::{
		flow::{Location, Size},
		layout::Sizing,
	};

	#[test]
	fn shape_bounds_follow_each_variant_geometry_contract() {
		let triangle = Shapes::Triangle {
			vertices: [Location::new(7.0, -3.0), Location::new(-2.0, 4.0), Location::new(3.0, 11.0)],
		};

		assert_eq!(triangle.bbox(Size::new(100.0, 100.0)), Size::new(9.0, 14.0));
		let rectangle = Shapes::Box {
			half: (Sizing::Relative(3, 4), Sizing::pixels(24.0)),
			radius: 8.0,
			exponent: 2.0,
		};

		assert_eq!(rectangle.bbox(Size::new(200.0, 80.0)), Size::new(150.0, 24.0));
		let circle = Shapes::Circle { radius: 6.5 };

		assert_eq!(circle.bbox(Size::new(1.0, 1.0)), Size::new(13.0, 13.0));
	}
}
