//! Creates platform windows and reports their input events.
//!
//! Every supported platform delivers events through one process-wide queue, so
//! [`App`] owns the only pump and tags each event with the [`WindowId`] it
//! targets. Input is routed to the window holding focus.

pub mod app;
pub mod input;
pub(crate) mod os;
pub mod window;

pub use self::app::App;
pub use self::os::Handles;
pub use self::window::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The `WindowId` struct identifies the window an [`Event`] targets.
pub struct WindowId(u64);

impl WindowId {
	pub(crate) fn from_raw(raw: u64) -> Self {
		Self(raw)
	}
}

/// An event reported by the application pump.
#[derive(Debug, Clone, Copy)]
pub enum Event {
	/// The event belongs to the application rather than to one window.
	App(AppEvents),
	/// The event targets one window. Input targets the window with focus.
	Window { window: WindowId, event: Events },
}

/// An event that belongs to the application as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvents {
	/// The platform asked the application to quit, e.g. from the dock or a session end.
	Quit,
}

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
	/// The window's drawable size changed, in pixels.
	Resize { width: u32, height: u32 },
	/// Keyboard focus changed. Cancel held interactions when focus is lost.
	FocusChanged(bool),
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
	/// Coordinates are normalized with window edges at `-1.0` and `1.0`.
	/// Captured pointer positions may extend beyond the window edges.
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
