use std::{collections::VecDeque, ffi::c_void, os::fd::OwnedFd, sync::Arc, time::Duration};

use utils::Extent;
use wayland_client::{
	Proxy,
	protocol::{
		wl_callback,
		wl_compositor::{self, WlCompositor},
		wl_keyboard,
		wl_output::{self, WlOutput},
		wl_pointer, wl_region, wl_registry,
		wl_seat::{self, WlSeat},
		wl_surface,
	},
};
use wayland_protocols::{
	wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1},
	wp::pointer_constraints::zv1::client::{zwp_locked_pointer_v1, zwp_pointer_constraints_v1},
	wp::relative_pointer::zv1::client::{
		zwp_relative_pointer_manager_v1::{self},
		zwp_relative_pointer_v1,
	},
	xdg::decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1},
	xdg::shell::client::{
		xdg_surface, xdg_toplevel,
		xdg_wm_base::{self, XdgWmBase},
	},
};
use xkbcommon::xkb;

use crate::window::{
	AppEvents, Event, Events, Features, Pointer, PointingDevice, Seat, Wait, WindowId,
	input::{Keys, MouseKeys},
	key_repeat::KeyRepeat,
};

/// The `App` struct owns the process's single Wayland connection and its event queue.
pub struct App {
	connection: wayland_client::Connection,
	event_queue: wayland_client::EventQueue<AppData>,
	data: AppData,
	id_name: String,
	next_window: u64,
	/// Every open window's protocol objects, in creation order.
	windows: Vec<Window>,
	/// The eventfd a waiting poll watches next to the display socket, shared with every [`AppWaker`]; see there.
	wake: Arc<OwnedFd>,
	/// Other descriptors a waiting poll watches, such as a HID device monitor's; their owners drain them.
	watched: Vec<OwnedFd>,
	/// The desktop portal connection dialogs go through, opened by the first dialog.
	portal: Option<dialogs::Portal>,
	/// Reports the desktop's accessibility settings, or `None` without a session bus.
	preferences: Option<preferences::PreferenceWatch>,
	/// The libdecor context that draws decorations when the compositor draws none, loaded by the first decorated
	/// window that needs it.
	libdecor: LibdecorState,
}

/// The `LibdecorState` enum keeps whether the app loaded libdecor, so a system without it is tried once.
enum LibdecorState {
	/// No decorated window needed libdecor yet.
	Unloaded,
	Loaded(libdecor::Libdecor),
	/// Loading failed, so windows open undecorated.
	Unavailable,
}

/// The `AppWaker` struct writes to the eventfd a waiting poll watches.
///
/// It shares the descriptor through `Arc` because wakers write to it from any thread and may outlive the [`App`], such
/// as one a HID monitor keeps; the descriptor stays open for as long as any of them can write.
#[derive(Clone)]
pub struct AppWaker(Arc<OwnedFd>);

impl AppWaker {
	pub fn wake(&self) {
		// A full counter already wakes the poll, so a failed write loses nothing.
		let _ = rustix::io::write(&*self.0, &1u64.to_ne_bytes());
	}
}

/// The `Window` struct keeps a toplevel's protocol objects while the [`App`] has it open; [`App::destroy_window`]
/// destroys them.
struct Window {
	id: WindowId,
	surface: wl_surface::WlSurface,
	/// The toplevel role that makes the compositor show the surface. A [`Features::BACKGROUND`] window has none, so
	/// it is never mapped.
	role: Option<Role>,
}

/// The `Role` enum holds the protocol objects that make a surface a toplevel window.
enum Role {
	/// A toplevel the backend drives itself, decorated by the compositor when it asked for server-side decorations.
	Xdg {
		xdg_surface: xdg_surface::XdgSurface,
		toplevel: xdg_toplevel::XdgToplevel,
		decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
	},
	/// A toplevel libdecor drives and decorates, on compositors that draw no decorations.
	Libdecor { frame: libdecor::Frame },
}

/// The `Handles` struct carries what a graphics context needs to create a surface on a window. A Wayland surface
/// reports no size of its own, so the context also takes the window's from [`crate::window::App::extent`].
pub struct Handles {
	pub display: *mut c_void,
	pub surface: *mut c_void,
}

/// The `Configuration` struct provides Wayland registry state while the connection starts.
#[derive(Debug)]
struct Configuration {
	compositor: Option<WlCompositor>,
	xdg_wm_base: Option<XdgWmBase>,
	wl_seat: Option<WlSeat>,
	wl_output: Option<WlOutput>,
	wl_callback: Option<wl_callback::WlCallback>,
	zwp_relative_pointer_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,
	/// Optional: without it, windows that capture the cursor only hide it.
	zwp_pointer_constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
	/// Optional: without it, decorated windows fall back to libdecor.
	decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
	/// Optional: without it, windows leave the cursor image to the compositor.
	cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,

	app_data_queue: wayland_client::QueueHandle<AppData>,
}

/// The `AppData` struct is the Wayland dispatch state shared by every window of the connection.
#[derive(Debug)]
struct AppData {
	compositor: WlCompositor,
	xdg_wm_base: XdgWmBase,
	zwp_relative_pointer_manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
	/// Locks the pointer for windows that capture the cursor, when the compositor supports it.
	zwp_pointer_constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
	/// Draws decorations around windows that ask for them, when the compositor supports server-side decorations.
	decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
	/// Names the cursor image by shape, when the compositor supports it.
	cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,

	windows: Vec<WindowState>,

	/// The largest output scale reported so far, used as the starting scale of new windows.
	output_scale: u32,
	/// The extent of the monitor.
	monitor_extent: Option<Extent>,
	/// Every output and the refresh rate of its current mode in millihertz, `0` when unknown.
	outputs: Vec<(WlOutput, i32)>,
	/// The seat's pointer, once the seat reports one.
	pointer: Option<wl_pointer::WlPointer>,
	/// Sets the pointer's cursor image by shape, when the compositor supports it.
	cursor_shape: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
	/// The pointer and the window it is over.
	/// The pointer, the window it is over, and the serial of its entry into it, which every cursor image change names.
	pointer_focus: Option<(wl_pointer::WlPointer, WindowId, u32)>,
	/// How many mouse buttons are held. A cursor lock waits for none, like on macOS, so a click that started a drag
	/// isn't cut off.
	buttons_down: u32,
	/// The scroll of the current pointer frame, which Wayland reports as parts of one event.
	scroll: PendingScroll,
	/// The seat's keyboard, once the seat reports one.
	keyboard: Option<wl_keyboard::WlKeyboard>,
	/// The keyboard and the window it targets.
	keyboard_focus: Option<(wl_keyboard::WlKeyboard, WindowId)>,
	/// The XKB state for translating keycodes into keysyms.
	keyboard_state: Option<KeyboardState>,
	/// The delay and rate the compositor asks held keys to repeat at. Wayland leaves repeating to the client.
	key_repeat: KeyRepeat,
	/// The key that repeats while it's held, if any.
	held_key: Option<HeldKey>,
	/// Whether the compositor sends repeats itself (`wl_keyboard` version 10), so the client makes none.
	compositor_repeats: bool,

	events: VecDeque<Event>,
}

/// The `PendingScroll` struct gathers one `wl_pointer` frame's scroll, so a wheel reports its notches rather than its
/// distance.
///
/// A wheel reports both a distance and notches in 120ths; the notches make one unit per notch, like macOS lines and
/// Windows wheel deltas. Touchpads report only a distance, in surface units like macOS trackpad points.
#[derive(Debug, Default)]
struct PendingScroll {
	/// The distance on each axis, horizontal then vertical, growing right and down.
	distance: [f64; 2],
	/// The notches on each axis, in 120ths of a notch, when a wheel reported any.
	notches: [Option<i32>; 2],
	/// The time of the frame's last axis event.
	time: u32,
}

/// The `HeldKey` struct keeps what a held key repeats, read when it was pressed, so its repeats type what its press
/// typed even after the modifiers change.
#[derive(Debug)]
struct HeldKey {
	keycode: xkb::Keycode,
	key: Option<Keys>,
	character: Option<char>,
	/// The window that had the keyboard when the key was pressed.
	window: WindowId,
	pressed_at: std::time::Instant,
	/// How many repeats it has made.
	made: u32,
}

/// The `WindowState` struct preserves the latest state the Wayland event queue reported for one window.
#[derive(Debug)]
struct WindowState {
	id: WindowId,
	surface: wl_surface::WlSurface,
	/// The scale factor of the window.
	scale: u32,
	/// The extent of the window in pixels.
	extent: Option<Extent>,
	/// The window's content size in surface units, which the scale turns into `extent`.
	size: (i32, i32),
	/// Whether the initial xdg_surface configuration has been acknowledged.
	configured: bool,
	/// The outputs the surface is shown on.
	outputs: Vec<WlOutput>,
	/// The refresh interval of the fastest output the surface is on, when its outputs report one.
	refresh: Option<Duration>,
	/// Whether the window hides the cursor and locks the pointer while it has focus.
	captures_cursor: bool,
	/// The shape the cursor takes over the window; see [`crate::window::App::set_pointer`].
	pointer: Pointer,
	/// The persistent lock that the compositor engages while the window has focus and the pointer is over it.
	cursor_lock: Option<zwp_locked_pointer_v1::ZwpLockedPointerV1>,
}

/// Converts a refresh rate in millihertz to its interval, or `None` when unknown.
fn refresh_interval(millihertz: i32) -> Option<Duration> {
	(millihertz > 0).then(|| Duration::from_secs_f64(1000.0 / millihertz as f64))
}

mod dialogs;
mod dispatch;
mod input;
mod key_translation;
mod libdecor;
mod lifecycle;
mod preferences;

use key_translation::{KeyboardState, Typed, compose_table, evdev_to_key};
