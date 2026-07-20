//! Creates platform windows and reports their input events.

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

/// An event reported by a window.
#[derive(Debug, Clone, Copy)]
pub enum Events {
	/// The window changed size.
	Resize { width: u32, height: u32 },
	/// The window was minimized.
	Minimize,
	/// The window was maximized.
	Maximize,
	/// The window was closed.
	Close,
	/// A keyboard key changed state.
	Key { seat: Seat, pressed: bool, key: input::Keys },
	/// The user entered a text character.
	Character { seat: Seat, character: char },
	/// A mouse button changed state.
	Button {
		seat: Seat,
		pressed: bool,
		button: input::MouseKeys,
	},
	/// The mouse moved relative to its previous position.
	/// Coordinates are normalized by the current window size.
	MouseMove {
		seat: Seat,
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// The mouse moved to an absolute position.
	/// Coordinates are normalized to the window in the range `-1.0..=1.0`.
	MousePosition {
		seat: Seat,
		x: f32,
		y: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// A mouse wheel or touch surface scrolled.
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
	/// Optional features requested for a window.
	pub struct Features : u32 {
		/// A title bar and border decorate the window.
		const DECORATIONS = 0b0001;
	}
}
