use std::{
	collections::VecDeque,
	ffi::c_void,
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
	wp::relative_pointer::zv1::client::{
		zwp_relative_pointer_manager_v1::{self},
		zwp_relative_pointer_v1,
	},
	xdg::shell::client::{
		xdg_surface, xdg_toplevel,
		xdg_wm_base::{self, XdgWmBase},
	},
};
use xkbcommon::xkb::{self, keysyms};

use crate::window::{
	Event, Events, Features, Seat, WindowId,
	input::{Keys, MouseKeys},
	os::{AppLike, WindowLike},
};

/// The `App` struct owns the process's single Wayland connection and its event queue.
pub struct App {
	connection: wayland_client::Connection,
	event_queue: wayland_client::EventQueue<AppData>,
	data: AppData,
	id_name: String,
	next_window: u64,
}

/// The `Window` struct owns a toplevel's protocol objects; dropping it destroys them, and the [`App`]
/// forgets the window on its next poll.
pub struct Window {
	id: WindowId,
	display: wl_display::WlDisplay,
	surface: wl_surface::WlSurface,
	xdg_surface: xdg_surface::XdgSurface,
	xdg_toplevel: xdg_toplevel::XdgToplevel,
	/// The refresh interval the event queue last reported for this window.
	refresh: SharedRefresh,
}

/// A refresh interval in nanoseconds, where `0` means unknown, shared between a window and the event queue.
type SharedRefresh = Arc<AtomicU64>; // TODO: hmmmmm

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

	app_data_queue: wayland_client::QueueHandle<AppData>,
}

/// The `AppData` struct is the Wayland dispatch state shared by every window of the connection.
#[derive(Debug)]
struct AppData {
	compositor: WlCompositor,
	xdg_wm_base: XdgWmBase,
	zwp_relative_pointer_manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,

	windows: Vec<WindowState>,

	/// The largest output scale reported so far, used as the starting scale of new windows.
	output_scale: u32,
	/// The extent of the monitor.
	monitor_extent: Option<Extent>,
	/// Every output and the refresh rate of its current mode in millihertz, `0` when unknown.
	outputs: Vec<(WlOutput, i32)>,
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
	/// The extent of the window.
	extent: Option<Extent>,
	/// Whether the initial xdg_surface configuration has been acknowledged.
	configured: bool,
	/// The outputs the surface is shown on.
	outputs: Vec<WlOutput>,
	/// The refresh interval of the fastest output the surface is on.
	refresh: SharedRefresh,
}

/// Converts a refresh rate in millihertz to its interval, or `None` when unknown.
fn refresh_interval(millihertz: i32) -> Option<Duration> {
	(millihertz > 0).then(|| Duration::from_secs_f64(1000.0 / millihertz as f64))
}

mod dispatch;
mod input;
mod key_translation;
mod lifecycle;

use key_translation::{KeyboardState, keysym_to_key};
