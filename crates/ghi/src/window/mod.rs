//! Creates platform windows and reports their input events.
//!
//! Every supported platform delivers events through one process-wide queue, so
//! [`App`] owns the only pump and tags each event with the [`WindowId`] it
//! targets. Input is routed to the window holding focus.

pub mod app;
pub mod dialog;
pub mod input;
// Only the backends that make their own repeats use it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) mod key_repeat;
pub(crate) mod os;

pub use self::app::{App, AppWaker};
pub use self::dialog::{Button, Buttons, Dialog, DialogError, DialogId, DialogResult, FileFilter, MessageLevel};
pub use self::os::Handles;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The `WindowId` struct identifies the window an [`Event`] targets.
#[repr(transparent)]
pub struct WindowId(std::num::NonZeroU64);

const _: () = assert!(size_of::<Option<WindowId>>() == size_of::<u64>());

impl WindowId {
	pub(crate) fn from_raw(raw: u64) -> Self {
		Self(std::num::NonZeroU64::new(raw).expect(
			"Window identity is zero. The platform did not supply a valid window. Create a window before identifying it.",
		))
	}
}

/// How long [`App::poll`] may wait for the first event before draining the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
	/// Drain what is already queued and return.
	Immediate,
	/// Wait until an event arrives, an [`AppWaker`] wakes the app, or the instant passes.
	Until(std::time::Instant),
	/// Wait until an event arrives or an [`AppWaker`] wakes the app.
	Forever,
}

/// An event reported by the application pump.
#[derive(Debug, Clone)]
pub enum Event {
	/// The event belongs to the application rather than to one window.
	App(AppEvents),
	/// The event targets one window. Input targets the window with focus.
	Window { window: WindowId, event: Events },
}

/// An event that belongs to the application as a whole.
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvents {
	/// The platform asked the application to quit, e.g. from the dock or a session end.
	Quit,
	/// A dialog from [`App::open_dialog`] sent a result. See [`DialogResult`] for the order results arrive in.
	Dialog { id: DialogId, result: DialogResult },
	/// The operating system's presentation preferences: once in the first poll, then after each change. See
	/// [Presentation preferences](/docs/reference/ghi#presentation-preferences).
	Preferences(Preferences),
}

/// The `Preferences` struct carries how the user asked the operating system to present applications, so a game can
/// follow the same accessibility settings as the rest of the desktop.
///
/// Read it from [`AppEvents::Preferences`]. Each field is `None` where the platform doesn't report it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Preferences {
	/// Whether the user asked for less movement on screen.
	pub reduce_motion: Option<bool>,
	/// Whether the user asked for stronger contrast.
	pub increase_contrast: Option<bool>,
	/// How many times larger than the platform's default size the user wants text, such as 1.5.
	pub text_scale: Option<f32>,
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
	/// The window's drawable size changed, in pixels, or the display scale it draws at did.
	///
	/// `scale` is the display's pixels per point, such as 2 on a Retina display, which a UI divides the size by to
	/// keep its physical size; see [Scale](/docs/reference/gui#scale).
	Resize { width: u32, height: u32, scale: f32 },
	/// The window moved to another display, or its display changed mode. Carries the refresh interval of the display
	/// the window is on now, when the platform reports it.
	DisplayChanged { refresh_interval: Option<std::time::Duration> },
	/// Keyboard focus changed. Cancel held interactions when focus is lost.
	FocusChanged(bool),
	/// The window was minimized.
	Minimize,
	/// Whether any part of the window can be seen changed. Carries [`App::is_visible`] after the change.
	///
	/// Platforms that cannot tell never send it.
	VisibilityChanged(bool),
	/// The window was maximized.
	Maximize,
	/// The window was closed.
	Close,
	/// A keyboard key changed state, or a held key repeated.
	///
	/// `repeat` marks another press of a key that is still held, at the delay and rate the platform's keyboard settings
	/// set; a repeat is always `pressed`. See [Key repeat](/docs/reference/input#key-repeat).
	Key {
		seat: Seat,
		pressed: bool,
		repeat: bool,
		key: input::Keys,
	},
	/// The user entered a text character.
	Character { seat: Seat, character: char },
	/// A mouse button changed state, as `device` sees it. Each press and release is reported once for each
	/// [`PointingDevice`].
	Button {
		seat: Seat,
		device: PointingDevice,
		pressed: bool,
		button: input::MouseKeys,
	},
	/// The mouse moved, as `device` measures it. `dx` grows to the right and `dy` grows upward. The values don't
	/// depend on the window's size; see [`PointingDevice`] for the units.
	MouseMove {
		seat: Seat,
		device: PointingDevice,
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// The cursor moved to an absolute position.
	/// Coordinates are normalized with window edges at `-1.0` and `1.0`.
	/// Captured pointer positions may extend beyond the window edges.
	PointerPosition {
		seat: Seat,
		x: f32,
		y: f32,
		/// The time at which the event occurred.
		time: u64,
	},
	/// A mouse wheel or touch surface scrolled, as `device` sees it.
	Scroll {
		seat: Seat,
		device: PointingDevice,
		dx: f32,
		dy: f32,
		/// The time at which the event occurred.
		time: u64,
	},
}

/// The `PointingDevice` enum tells apart the two views every platform reports of one mouse, so applications can use
/// the system cursor for interfaces and the physical mouse for camera control.
///
/// Map each to its own input device, as `byte_engine`'s window integration maps them to the `Pointer` and `Mouse`
/// device classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointingDevice {
	/// The system cursor, as the platform moves it with pointer acceleration: points on macOS, pixels on Windows, and
	/// relative-pointer units on Wayland.
	Pointer,
	/// The physical mouse before pointer acceleration: Game Controller mouse counts on macOS, raw input counts on
	/// Windows, and unaccelerated relative-pointer units on Wayland. On macOS, while no Game Controller mouse is
	/// connected, it repeats the [`PointingDevice::Pointer`] events.
	Mouse,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
	/// Optional features requested for a window.
	pub struct Features : u32 {
		/// A title bar and border decorate the window.
		const DECORATIONS = 0b0001;
		/// The window serves runs no one watches, such as an agent debugging an application: it opens without being
		/// shown and never takes focus from the desktop. Bind it with
		/// [`crate::PresentationModes::Inmediate`], since an unseen window has no display refresh to wait for, and
		/// pace frames on the CPU.
		const BACKGROUND = 0b0010;
		/// The window hides the cursor and holds it at its center while it has focus, so the mouse can move without
		/// end and [`Events::MouseMove`] keeps reporting motion; [`Events::PointerPosition`] isn't sent meanwhile. Use
		/// it for camera control in games. A [`Features::BACKGROUND`] window never captures the cursor.
		const CAPTURE_CURSOR = 0b0100;
	}
}

/// The `Pointer` enum names the system pointer shape a window shows while the pointer is over it, so an interface can
/// tell the user what a press there does, such as a hand over a link or a two-headed arrow over a splitter.
///
/// Every shape is one the platform draws itself. Windows has no grabbing hands, so it shows the pointing hand for
/// [`Pointer::Grab`] and the move arrow for [`Pointer::Grabbing`]. Set it with [`App::set_pointer`]. A window that captures the cursor
/// ([`Features::CAPTURE_CURSOR`]) hides it whatever its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Pointer {
	/// The platform's arrow.
	#[default]
	Default,
	/// A pointing hand, over something a press activates, such as a link.
	Hand,
	/// A text cursor (I-beam), over text the user can select or edit.
	Text,
	/// An open hand, over something the user can drag.
	Grab,
	/// A closed hand, while the user drags something.
	Grabbing,
	/// A crosshair, for precise picking.
	Crosshair,
	/// A sign that a press here does nothing, such as over a drop target that refuses the dragged item.
	NotAllowed,
	/// A left-right arrow, over an edge or splitter that resizes horizontally.
	ResizeColumn,
	/// An up-down arrow, over an edge or splitter that resizes vertically.
	ResizeRow,
}
