use std::{
	collections::VecDeque,
	ffi::c_void,
	os::fd::OwnedFd,
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
	time::Duration,
};

use utils::Extent;
use wayland_client::{
	Proxy,
	protocol::{
		wl_callback,
		wl_compositor::{self, WlCompositor},
		wl_display, wl_keyboard,
		wl_output::{self, WlOutput},
		wl_pointer, wl_region, wl_registry,
		wl_seat::{self, WlSeat},
		wl_surface,
	},
};
use wayland_protocols::{
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
use xkbcommon::xkb::{self, keysyms};

use crate::window::{
	AppEvents, Event, Events, Features, PointingDevice, Seat, Wait, WindowId,
	input::{Keys, MouseKeys},
};

/// The `App` struct owns the process's single Wayland connection and its event queue.
pub struct App {
	connection: wayland_client::Connection,
	event_queue: wayland_client::EventQueue<AppData>,
	data: AppData,
	id_name: String,
	next_window: u64,
	/// The eventfd a waiting poll watches next to the display socket.
	wake: Arc<OwnedFd>,
	/// Other descriptors a waiting poll watches, such as a HID device monitor's; their owners drain them.
	watched: Vec<OwnedFd>,
	/// The desktop portal connection dialogs go through, opened by the first dialog.
	portal: Option<dialogs::Portal>,
	/// The libdecor context that draws decorations when the compositor draws none, loaded by the first decorated
	/// window that needs it. It holds `None` when libdecor couldn't be loaded, so later windows don't try again.
	libdecor: std::cell::OnceCell<Option<libdecor::Libdecor>>,
}

/// The `AppWaker` struct writes to the eventfd a waiting poll watches.
#[derive(Clone)]
pub struct AppWaker(Arc<OwnedFd>);

impl AppWaker {
	pub fn wake(&self) {
		// A full counter already wakes the poll, so a failed write loses nothing.
		let _ = rustix::io::write(&*self.0, &1u64.to_ne_bytes());
	}
}

/// The `Window` struct owns a toplevel's protocol objects; dropping it destroys them, and the [`App`]
/// forgets the window on its next poll.
pub struct Window {
	id: WindowId,
	display: wl_display::WlDisplay,
	surface: wl_surface::WlSurface,
	/// The toplevel role that makes the compositor show the surface. A [`Features::BACKGROUND`] window has none, so
	/// it is never mapped.
	role: Option<Role>,
	/// The refresh interval the event queue last reported for this window.
	refresh: SharedRefresh,
	/// The window's size in pixels, which its swapchain follows.
	extent: WindowExtent,
}

/// The `Role` enum holds the protocol objects that make a surface a toplevel window.
enum Role {
	/// A toplevel the backend drives itself, decorated by the compositor when it asked for server-side decorations.
	Xdg {
		xdg_surface: xdg_surface::XdgSurface,
		toplevel: xdg_toplevel::XdgToplevel,
		decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
	},
	/// A toplevel libdecor drives and decorates, on compositors that draw no decorations. The frame is held so
	/// dropping the window destroys it.
	Libdecor { _frame: libdecor::Frame },
}

/// A refresh interval in nanoseconds, where `0` means unknown, shared between a window and the event queue.
type SharedRefresh = Arc<AtomicU64>; // TODO: hmmmmm

pub struct Handles {
	pub display: *mut c_void,
	pub surface: *mut c_void,
	/// The window's size in pixels. A Wayland surface reports no size of its own, so a swapchain on it reads this one.
	pub extent: WindowExtent,
}

/// The `WindowExtent` struct shares a Wayland window's size in pixels between the event queue, which learns it from
/// the compositor, and the swapchain that presents to the window, which must match it.
///
/// Other platforms' surfaces report their own size to the graphics API. A Wayland surface leaves the size to the
/// client, so the swapchain reads it here and is recreated when it changes.
#[derive(Clone, Debug, Default)]
pub struct WindowExtent(Arc<AtomicU64>);

impl WindowExtent {
	/// Returns the window's size in pixels, or `None` before the compositor configured it.
	pub fn get(&self) -> Option<Extent> {
		let packed = self.0.load(Ordering::Relaxed);
		(packed != 0).then(|| Extent::rectangle((packed >> 32) as u32, packed as u32))
	}

	fn set(&self, extent: Extent) {
		self.0.store(
			u64::from(extent.width()) << 32 | u64::from(extent.height()),
			Ordering::Relaxed,
		);
	}
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

	windows: Vec<WindowState>,

	/// The largest output scale reported so far, used as the starting scale of new windows.
	output_scale: u32,
	/// The extent of the monitor.
	monitor_extent: Option<Extent>,
	/// Every output and the refresh rate of its current mode in millihertz, `0` when unknown.
	outputs: Vec<(WlOutput, i32)>,
	/// The seat's pointer, once the seat reports one.
	pointer: Option<wl_pointer::WlPointer>,
	/// The pointer and the window it is over.
	pointer_focus: Option<(wl_pointer::WlPointer, WindowId)>,
	/// The keyboard and the window it targets.
	keyboard_focus: Option<(wl_keyboard::WlKeyboard, WindowId)>,
	/// The XKB state for translating keycodes into keysyms.
	keyboard_state: Option<KeyboardState>,

	events: VecDeque<Event>,
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
	/// The extent shared with the window's swapchain.
	shared_extent: WindowExtent,
	/// Whether the initial xdg_surface configuration has been acknowledged.
	configured: bool,
	/// The outputs the surface is shown on.
	outputs: Vec<WlOutput>,
	/// The refresh interval of the fastest output the surface is on.
	refresh: SharedRefresh,
	/// Whether the window hides the cursor and locks the pointer while it has focus.
	captures_cursor: bool,
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

use key_translation::{KeyboardState, keysym_to_key};
