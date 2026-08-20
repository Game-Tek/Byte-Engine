//! UI-local pointer and scroll coordinate types.

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
/// Use `UiVector` for scroll input and spatial-navigation axes. It deliberately
/// has no world-space meaning. Next, pass it to
/// [`crate::ui::Engine::update_scroll_state`] or
/// [`crate::ui::layout::snapshot::Snapshot::move_cursor`].
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
