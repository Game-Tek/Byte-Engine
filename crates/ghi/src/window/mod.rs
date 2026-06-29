//! The window module provides functionality for creating and managing windows on multiple platforms.

pub mod input;
pub(crate) mod os;
pub mod window;

pub use self::os::Handles;
pub use self::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The `Seat` struct identifies the input seat associated with a window input event.
pub struct Seat(u32);

impl Seat {
	/// Returns the placeholder seat used until platform input seats are wired through.
	pub fn stub() -> Self {
		Self(0)
	}
}

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
	Key { seat: Seat, pressed: bool, key: input::Keys },
	/// A mouse button has been pressed or released.
	Button {
		seat: Seat,
		pressed: bool,
		button: input::MouseKeys,
	},
	/// The mouse has moved relative to its previous position.
	/// Coordinates are normalized by the current window size.
	MouseMove {
		seat: Seat,
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// The mouse position has changed.
	/// Coordinates are normalized to the window in the range `-1.0..=1.0`.
	MousePosition {
		seat: Seat,
		x: f32,
		y: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// The mouse wheel or touch surface has scrolled.
	Scroll {
		seat: Seat,
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
	/// Bit flags for the features of a window.
	pub struct Features : u32 {
		/// The window has decorations (title bar, border, etc.).
		const DECORATIONS = 0b0001;
	}
}
