pub mod input;
pub mod window;

pub use self::window::Window;

pub use self::window::OSHandles;

/// The events that can be received from a window.
#[derive(Debug, Clone, Copy)]
pub enum Events {
	/// The window has been resized.
	Resize {
		width: u32,
		height: u32,
	},
	/// The window has been minimized.
	Minimize,
	/// The window has been maximized.
	Maximize,
	/// The window has been closed.
	Close,
	/// A key has been pressed or released.
	Key {
		pressed: bool,
		key: input::Keys,
	},
	/// A mouse button has been pressed or released.
	Button {
		pressed: bool,
		button: input::MouseKeys,
	},
	/// The mouse has moved.
	/// Coordinates have no particular frame of reference but are normalized by the monitor size.
	/// Coordinates may get wrapped to preserve precision.
	MouseMove {
		x: f32,
		y: f32,
		/// The time at which the event occurred.
		time: u64,
	},
}
