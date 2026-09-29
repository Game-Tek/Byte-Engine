//! Identity of the UI elements layout components declare.
//!
//! Components declare elements through a [`crate::ui::layout::context::Context`], and the engine creates and owns
//! each one. Use an element's [`Id`] to refer to it across frames, such as when routing events or reparenting it.

use std::num::NonZeroU64;

/// Stable non-zero identifier of a UI element.
///
/// An element's id is a hash of the path it was declared at: the declaring context's path and the element's
/// [`crate::ui::layout::context::ElementKey`]. Declaring the same key under the same parent in a later frame gives the
/// same id, so components can refer to elements across frames and remounts.
pub type Id = NonZeroU64;
