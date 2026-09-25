//! Concrete UI elements emitted by layout components.
//!
//! Components declare elements through a [`crate::ui::layout::context::Context`], and the engine creates and owns
//! the [`ConcreteElement`] behind each one. Implement [`Element`] for reusable element wrappers that expose a
//! primitive.

use super::primitive::{BasePrimitive, Primitives};

/// The [`Element`] trait exposes the primitive represented by a UI element.
pub trait Element {
	/// Returns the primitive that layout and rendering systems consume.
	fn primitive(&self) -> BasePrimitive;
}

use std::num::NonZeroU64;

/// Stable non-zero identifier of a UI element.
///
/// An element's id is a hash of the path it was declared at: the declaring context's path and the element's
/// [`crate::ui::layout::context::ElementKey`]. Declaring the same key under the same parent in a later frame gives the
/// same id, so components can refer to elements across frames and remounts.
pub type Id = NonZeroU64;

/// The [`ElementHandle`] trait exposes the stable identity assigned during layout.
pub trait ElementHandle {
	/// Returns the layout identity for this element.
	fn id(&self) -> Id;
}

/// The [`ConcreteElement`] struct stores one built-in primitive the engine created for a declaration.
pub struct ConcreteElement {
	pub(crate) primitive: Primitives,
}
