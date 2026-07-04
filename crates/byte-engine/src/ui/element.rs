//! Concrete UI elements emitted by layout components.
//!
//! Components typically construct elements through a
//! [`crate::ui::layout::context::Context`] rather than using
//! [`ConcreteElement`] directly. Implement [`Element`] for reusable element
//! wrappers that expose a primitive.

use super::{
	flow::FlowFunction,
	primitive::{BasePrimitive, Primitive},
};
use crate::ui::{
	components::{curve::Curve, image::Image, shape::Shape, text::Text, text_field::TextField},
	flow::{Offset, Size},
	primitive::{Primitives, Shapes},
	Container,
};

/// The [`Element`] trait exposes the primitive represented by a UI element.
pub trait Element {
	/// Returns the primitive that layout and rendering systems consume.
	fn primitive(&self) -> BasePrimitive;
}

use std::num::NonZeroU32;

/// Stable non-zero identifier assigned to UI elements during layout.
pub type Id = NonZeroU32;

/// The [`ElementHandle`] trait exposes the stable identity assigned during layout.
pub trait ElementHandle {
	/// Returns the layout identity for this element.
	fn id(&self) -> Id;
}

/// The [`ConcreteElement`] struct stores one built-in primitive produced by a
/// layout component.
pub struct ConcreteElement {
	pub(crate) primitive: Primitives,
}

impl ConcreteElement {
	/// Creates an element backed by a container primitive.
	pub fn container(container: Container) -> Self {
		let primitive = Primitives::Container(container);

		Self { primitive }
	}

	/// Creates an element backed by a shape primitive.
	pub fn shape(shape: Shape) -> Self {
		let primitive = Primitives::Shape(shape);

		Self { primitive }
	}

	/// Creates an element backed by a curve primitive.
	pub fn curve(curve: Curve) -> Self {
		let primitive = Primitives::Curve(curve);

		Self { primitive }
	}

	/// Creates an element backed by an image primitive.
	pub fn image(image: Image) -> Self {
		let primitive = Primitives::Image(image);

		Self { primitive }
	}

	/// Creates an element backed by a text primitive.
	pub fn text(text: Text) -> Self {
		let primitive = Primitives::Text(text);

		Self { primitive }
	}

	/// Creates an element backed by a text field primitive.
	pub fn text_field(text_field: TextField) -> Self {
		let primitive = Primitives::TextField(text_field);

		Self { primitive }
	}
}
