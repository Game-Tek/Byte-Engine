//! UI-local pointer and scroll coordinate types.

use super::flow::Size;

/// The `UiPoint` struct carries a two-dimensional UI position.
///
/// Use `UiPoint` for normalized pointer positions and layout-local points. It
/// deliberately has no world-space meaning. Next, pass it to
/// [`crate::ui::Engine::set_cursor_position`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiPoint {
	/// The horizontal UI coordinate.
	pub x: f32,
	/// The vertical UI coordinate.
	pub y: f32,
}

impl UiPoint {
	/// Creates a UI position from horizontal and vertical coordinates.
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	/// Returns the UI origin.
	pub const fn zero() -> Self {
		Self::new(0.0, 0.0)
	}
}

/// The `UiVector` struct carries a two-dimensional UI displacement.
///
/// Use `UiVector` for scroll input and drag offsets. It deliberately has no
/// world-space meaning. Next, pass it to
/// [`crate::ui::Engine::update_scroll_state`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiVector {
	/// The horizontal UI displacement.
	pub x: f32,
	/// The vertical UI displacement.
	pub y: f32,
}

impl UiVector {
	/// Creates a UI displacement from horizontal and vertical components.
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	/// Returns the neutral UI displacement.
	pub const fn zero() -> Self {
		Self::new(0.0, 0.0)
	}
}

/// Maps normalized window coordinates, -1 to 1 with y up, onto a layout of `size` with y down.
///
/// Every pointer query converts through this one function, so the engine, snapshots, and retained hit tests agree on
/// which element sits under a position, even at an element's exact edge.
pub(crate) fn normalized_to_layout(position: UiPoint, size: Size) -> UiPoint {
	UiPoint::new((position.x + 1.0) * 0.5 * size.x(), (1.0 - position.y) * 0.5 * size.y())
}
