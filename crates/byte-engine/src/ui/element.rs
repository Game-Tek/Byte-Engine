//! Concrete UI elements emitted by layout components.
//!
//! Components typically construct elements through a
//! [`crate::ui::layout::engine::Context`] rather than using
//! [`ConcreteElement`] directly. Implement [`Element`] for reusable element
//! wrappers that expose a primitive.

use super::{
	flow::FlowFunction,
	primitive::{BasePrimitive, Primitive},
};
use crate::ui::{
	components::{shape::Shape, text::Text},
	flow::{Offset, Size},
	primitive::{Primitives, Shapes},
	Container,
};

/// The [`Element`] trait exposes the primitive represented by a UI element.
pub trait Element {
	fn primitive(&self) -> BasePrimitive;
}

use std::num::NonZeroU32;

pub type Id = NonZeroU32;

/// The [`ElementHandle`] trait exposes the stable identity assigned during layout.
pub trait ElementHandle {
	fn id(&self) -> Id;
}

/// The [`ConcreteElement`] struct stores one built-in primitive produced by a
/// layout component.
pub struct ConcreteElement {
	pub(crate) primitive: Primitives,
}

impl ConcreteElement {
	pub fn container(container: Container) -> Self {
		let primitive = Primitives::Container(container);

		Self { primitive }
	}

	pub fn shape(shape: Shape) -> Self {
		let primitive = Primitives::Shape(shape);

		Self { primitive }
	}

	pub fn text(text: Text) -> Self {
		let primitive = Primitives::Text(text);

		Self { primitive }
	}
}
