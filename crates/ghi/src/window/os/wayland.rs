use std::{cell::RefCell, collections::VecDeque, ffi::c_void, marker::PhantomData, rc::Rc};

use utils::Extent;
use wayland_client::{
	protocol::{
		wl_callback,
		wl_compositor::{self, WlCompositor},
		wl_display, wl_keyboard,
		wl_output::{self, WlOutput},
		wl_pointer, wl_region, wl_registry,
		wl_seat::{self, WlSeat},
		wl_surface,
	},
	Proxy,
};
use wayland_protocols::{
	wp::{
		pointer_constraints::zv1::client::{zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1},
		relative_pointer::zv1::client::{
			zwp_relative_pointer_manager_v1::{self},
			zwp_relative_pointer_v1,
		},
	},
	xdg::shell::client::{
		xdg_surface, xdg_toplevel,
		xdg_wm_base::{self, XdgWmBase},
	},
};
use xkbcommon::xkb::{self, keysyms};

use crate::{
	window::os::{Features, WindowLike},
	window::{
		input::{Keys, MouseKeys},
		Events, Seat,
	},
};

pub struct Window {
	connection: wayland_client::Connection,
	event_queue: wayland_client::EventQueue<AppData>,
	xdg_wm_base: xdg_wm_base::XdgWmBase,
	surface: wl_surface::WlSurface,
	xdg_surface: xdg_surface::XdgSurface,
	xdg_toplevel: xdg_toplevel::XdgToplevel,
	zwp_pointer_constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
	zwp_relative_pointer_manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,

	requests: VecDeque<Requests>,

	state: WindowState,
}

/// A window operation queued until Wayland can process it.
#[derive(Clone, Debug)]
enum Requests {
	/// Constrain the pointer after the window receives pointer and keyboard focus.
	ConstrainPointer,
	/// Lock the pointer after the window receives pointer and keyboard focus.
	LockPointer,
	/// Hide the pointer after Wayland creates it.
	HidePointer,
}

/// The `WindowIterator` struct yields [`Events`] collected by the window's poll operation.
pub struct WindowIterator<'a> {
	events: VecDeque<Events>,
	_phantom: PhantomData<&'a ()>,
}

impl<'a> Iterator for WindowIterator<'a> {
	type Item = Events;

	fn next(&mut self) -> Option<Events> {
		self.events.pop_front()
	}
}

pub struct Handles {
	pub display: *mut c_void,
	pub surface: *mut c_void,
}

/// The `Configuration` struct provides Wayland registry state while a window connection starts.
#[derive(Debug)]
struct Configuration {
	compositor: Option<WlCompositor>,
	xdg_wm_base: Option<XdgWmBase>,
	wl_seat: Option<WlSeat>,
	wl_output: Option<WlOutput>,
	wl_surface: Option<wl_surface::WlSurface>,
	wl_callback: Option<wl_callback::WlCallback>,
	zwp_pointer_constraints: Option<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1>,
	zwp_relative_pointer_manager: Option<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1>,

	app_data_queue: wayland_client::QueueHandle<AppData>,
}

/// The `AppData` struct provides Wayland callback state for an active window.
#[derive(Debug)]
struct AppData {
	wl_surface: wl_surface::WlSurface,
	zwp_pointer_constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
	zwp_relative_pointer_manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,

	state: WindowState,

	events: VecDeque<Events>,
	requests: VecDeque<Requests>,
}

/// The `WindowState` struct preserves the latest state reported by the Wayland event queue.
#[derive(Debug, Clone)]
struct WindowState {
	/// The scale factor of the window.
	scale: u32,
	/// The extent of the window.
	extent: Option<Extent>,
	/// The extent of the monitor.
	monitor_extent: Option<Extent>,
	/// Whether the initial xdg_surface configuration has been acknowledged.
	configured: bool,
	/// The focused pointer
	focused_pointer: Option<wl_pointer::WlPointer>,
	/// The focused keyboard
	focused_keyboard: Option<wl_keyboard::WlKeyboard>,
	/// Whether the pointer should remain confined to the window surface.
	should_confine_pointer: bool,
	/// Whether the compositor currently reports the pointer as confined.
	pointer_is_confined: bool,
	/// Whether the pointer should remain hidden.
	should_hide_pointer: bool,
	/// Whether the pointer is currently hidden.
	pointer_is_hidden: bool,
	/// Whether the pointer should remain locked.
	should_lock_pointer: bool,
	/// Whether the compositor currently reports the pointer as locked.
	pointer_is_locked: bool,
	/// The active confined pointer handle, if one is currently set.
	confined_pointer: Option<zwp_confined_pointer_v1::ZwpConfinedPointerV1>,
	/// The active locked pointer handle, if one is currently set.
	locked_pointer: Option<zwp_locked_pointer_v1::ZwpLockedPointerV1>,
	/// The XKB state for translating keycodes into keysyms.
	keyboard_state: Option<Rc<RefCell<KeyboardState>>>,
}

impl Default for WindowState {
	fn default() -> Self {
		Self {
			scale: 1,
			extent: None,
			monitor_extent: None,
			configured: false,
			focused_pointer: None,
			focused_keyboard: None,
			should_confine_pointer: false,
			pointer_is_confined: false,
			should_hide_pointer: false,
			pointer_is_hidden: false,
			should_lock_pointer: false,
			pointer_is_locked: false,
			confined_pointer: None,
			locked_pointer: None,
			keyboard_state: None,
		}
	}
}

mod dispatch;
mod input;
mod key_translation;
mod lifecycle;

use key_translation::{keysym_to_key, KeyboardState};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_wayland_window() {
		// Only run this test if we are on a Wayland session
		if std::env::vars().find(|(key, _)| key == "WAYLAND_DISPLAY").is_some()
			&& std::env::vars()
				.find(|(key, value)| key == "XDG_SESSION_TYPE" && value == "wayland")
				.is_some()
		{
			let _ = Window::try_new(
				"My Test Wayland Window",
				Extent::rectangle(1920, 1080),
				"my_test_wayland_window.byte_engine",
				Features::default(),
			);
		}
	}
}
