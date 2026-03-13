//! The window module provides functionality for creating and managing windows on multiple platforms.

pub mod input;
pub(crate) mod os;
pub mod window;

pub use self::os::Handles;
pub use self::window::Window;

/// The events that can be received from a window.
#[derive(Debug, Clone, Copy)]
pub enum Events {
	/// The window has been resized.
	Resize { width: u32, height: u32 },
	/// The window has been minimized.
	Minimize,
	/// The window has been maximized.
	Maximize,
	/// The window has been closed.
	Close,
	/// A key has been pressed or released.
	Key { pressed: bool, key: input::Keys },
	/// A mouse button has been pressed or released.
	Button { pressed: bool, button: input::MouseKeys },
	/// The mouse has moved relative to its previous position.
	/// Coordinates are normalized by the current window size.
	MouseMove {
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// The mouse position has changed.
	/// Coordinates are normalized to the window in the range `-1.0..=1.0`.
	MousePosition {
		x: f32,
		y: f32,
		/// The time at which the event occurred.
		time: u64,
	},
}
