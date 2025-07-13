use std::{collections::VecDeque, ffi::c_void, marker::PhantomData};

use utils::Extent;
use wayland_client::{protocol::{wl_callback, wl_compositor::{self, WlCompositor}, wl_display, wl_keyboard, wl_output::{self, WlOutput}, wl_pointer, wl_region, wl_registry, wl_seat::{self, WlSeat}, wl_surface}, Proxy};
use wayland_protocols::{wp::{pointer_constraints::zv1::client::{zwp_confined_pointer_v1, zwp_locked_pointer_v1, zwp_pointer_constraints_v1}, relative_pointer::zv1::client::{zwp_relative_pointer_manager_v1::{self, ZwpRelativePointerManagerV1}, zwp_relative_pointer_v1}}, xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base::{self, XdgWmBase}}};

use crate::{Keys, MouseKeys, Events};

pub struct WaylandWindow {
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

/// The `Requests` enum contains requests that need to be queued for processing.
#[derive(Clone, Debug)]
enum Requests {
	/// Request to constrain the pointer to the window's bounds.
	/// This operation is queued because it requires pointer and keyboard focus or else it will fail.
	ConstrainPointer,
	/// Request to lock the pointer to it's current position.
	/// This operation is queued because it requires pointer and keyboard focus or else it will fail.
	LockPointer,
	/// Request to make the pointer invisible.
	/// This operation is queued because it requires the pointer to be created.
	HidePointer,
}

impl WaylandWindow {
	pub fn try_new(name: &str, extent: Extent, id_name: &str) -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let mut configuration_event_queue: wayland_client::EventQueue<Configuration> = conn.new_event_queue();
		let configuration_qh = configuration_event_queue.handle();

		let mut app_event_queue = conn.new_event_queue();
		let app_event_qh = app_event_queue.handle();

		let display = conn.display();

		let _ = display.get_registry(&configuration_qh, ());

		// Get globals
		let (compositor, wm_base, zwp_pointer_constraints, zwp_relative_pointer_manager) = {
			let mut configuration = Configuration {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,

				wl_surface: None,
				wl_callback: None,

				zwp_pointer_constraints: None,
				zwp_relative_pointer_manager: None,

				app_data_queue: app_event_qh.clone(),
			};

			configuration_event_queue.roundtrip(&mut configuration).map_err(|e| {
				format!("Failed to roundtrip configuration event queue: {}", e)
			})?;

			if let (Some(compositor), Some(wm_base), Some(zwp_pointer_constraints), Some(zwp_relative_pointer_manager)) = (configuration.compositor, configuration.xdg_wm_base, configuration.zwp_pointer_constraints, configuration.zwp_relative_pointer_manager) {
				Ok((compositor, wm_base, zwp_pointer_constraints, zwp_relative_pointer_manager,))
			} else {
				Err("Failed to acquire all required globals".to_string())
			}
		}?;

		let surface = compositor.create_surface(&app_event_qh, ());

		let xdg_surface = wm_base.get_xdg_surface(&surface, &app_event_qh, ());

		let toplevel = xdg_surface.get_toplevel(&app_event_qh, ());

		toplevel.set_title(name.to_string());
		toplevel.set_app_id(id_name.to_string());

		let state = {
			let mut app_data = AppData {
				wl_surface: surface.clone(),
				zwp_pointer_constraints: zwp_pointer_constraints.clone(),
				zwp_relative_pointer_manager: zwp_relative_pointer_manager.clone(),

				events: VecDeque::with_capacity(64),
				requests: VecDeque::with_capacity(16),

				state: WindowState::default(),
			};

			app_event_queue.roundtrip(&mut app_data).unwrap();

			surface.set_buffer_scale(app_data.state.scale as _);
			xdg_surface.set_window_geometry(0, 0, extent.width() as _, extent.height() as _);

			surface.commit();

			app_data.state
		};

		// if let Some(pointer) = &wl_pointer {
		// 	zwp_relative_pointer_manager.get_relative_pointer(pointer, &app_event_qh, ());
		// }

		let mut requests = VecDeque::with_capacity(16);

		requests.push_back(Requests::ConstrainPointer);
		// requests.push_back(Requests::LockPointer);
		requests.push_back(Requests::HidePointer);

		Ok(Self {
			connection: conn,
			event_queue: app_event_queue,
			xdg_wm_base: wm_base,
			surface,
			xdg_surface,
			xdg_toplevel: toplevel,
			zwp_pointer_constraints,
			zwp_relative_pointer_manager,
			requests,

			state,
		})
	}

	pub fn display(&self) -> wl_display::WlDisplay {
		self.connection.display()
	}

	pub fn surface(&self) -> wl_surface::WlSurface {
		self.surface.clone()
	}

	pub fn get_os_handles(&self) -> OSHandles {
		OSHandles {
			display: self.display().id().as_ptr() as *mut c_void,
			surface: self.surface().id().as_ptr() as *mut c_void,
		}
	}

	pub fn poll(&mut self) -> WindowIterator {
		// This implementation first processes all events from the wayland event queue
		// while producing `Events` which are then handed to an iterator
		// which is then returned

		let mut app_data = AppData {
			wl_surface: self.surface.clone(),
			zwp_pointer_constraints: self.zwp_pointer_constraints.clone(),
			zwp_relative_pointer_manager: self.zwp_relative_pointer_manager.clone(),

			events: VecDeque::with_capacity(64),
			requests: self.requests.clone(),

			state: self.state.clone(),
		};

		let event_queue = &mut self.event_queue;

		event_queue.dispatch_pending(&mut app_data).unwrap();

		// Copy updated state back to window
		self.state = app_data.state;
		self.requests = app_data.requests;

		WindowIterator {
			events: app_data.events,
			_phantom: PhantomData,
		}
	}
}

/// The `WindowIterator` struct is used to iterate over `Events` produced by the `poll` method.
/// Wayland events are first processed in the `poll` method which then copies it's own event list to the iterator.
pub struct WindowIterator<'a> {
	events: VecDeque<Events>,
	_phantom: PhantomData<&'a ()>,
}

impl <'a> Iterator for WindowIterator<'a> {
	type Item = Events;

	fn next(&mut self) -> Option<Events> {
		self.events.pop_front()
	}
}

impl Drop for WaylandWindow {
	fn drop(&mut self) {
		self.xdg_toplevel.destroy();
		self.xdg_surface.destroy();
		self.surface.destroy();
		self.xdg_wm_base.destroy();
	}
}

pub struct OSHandles {
	pub display: *mut c_void,
	pub surface: *mut c_void,
}

/// The `Configuration` struct holds the necessary Wayland objects and data for creating a window.
/// This struct is handed to the `WlRegistry` binding to initialize the Wayland connection.
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

/// The `AppData` struct holds the necessary Wayland objects and state to process events and requests for an already created window.
#[derive(Debug)]
struct AppData {
	wl_surface: wl_surface::WlSurface,
	zwp_pointer_constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
	zwp_relative_pointer_manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,

	state: WindowState,

	events: VecDeque<Events>,
	requests: VecDeque<Requests>,
}

/// The `WindowState` struct holds the most recent tracked state of the Wayland window.
/// The properties reported by the event queue are used to update the window state.
#[derive(Debug, Clone)]
struct WindowState {
	/// The scale factor of the window.
	scale: u32,
	/// The location of the pointer.
	/// This gets calculated by accumulating the pointer motion events.
	/// This is relative to no reference point.
	pointer_location: (f32, f32),
	/// The extent of the window.
	extent: Option<Extent>,
	/// The extent of the monitor.
	monitor_extent: Option<Extent>,
	/// The focused pointer
	focused_pointer: Option<wl_pointer::WlPointer>,
	/// The focused keyboard
	focused_keyboard: Option<wl_keyboard::WlKeyboard>,
}

impl Default for WindowState {
	fn default() -> Self {
		Self {
			scale: 1,
			pointer_location: (0.0, 0.0),
			extent: None,
			monitor_extent: None,
			focused_pointer: None,
			focused_keyboard: None,
		}
	}
}

impl AppData {
	fn process_requests(&mut self, qh: &wayland_client::QueueHandle<Self>) {
		let surface = &self.wl_surface;

		self.requests.retain(|e| {
			match e {
				Requests::ConstrainPointer => {
					if let (Some(pointer), Some(_)) = (&self.state.focused_pointer, &self.state.focused_keyboard) {
						self.zwp_pointer_constraints.confine_pointer(surface, pointer, None, zwp_pointer_constraints_v1::Lifetime::Oneshot, &qh, ());

						surface.commit();

						false
					} else {
						true
					}
				}
				Requests::LockPointer => {
					if let (Some(pointer), Some(_)) = (&self.state.focused_pointer, &self.state.focused_keyboard) {
						self.zwp_pointer_constraints.lock_pointer(surface, pointer, None, zwp_pointer_constraints_v1::Lifetime::Oneshot, &qh, ());

						surface.commit();

						false
					} else {
						true
					}
				}
				Requests::HidePointer => {
					if let Some(pointer) = &self.state.focused_pointer {
						pointer.set_cursor(0, None, 0, 0);

						surface.commit();

						false
					} else {
						true
					}
				}
			}
		});
	}
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for Configuration {
    fn event(this: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<Configuration>,) {
    	let qh = &this.app_data_queue;

		match event {
			wayland_client::protocol::wl_registry::Event::Global { name, interface, version } => {
				dbg!(&interface);

				match interface.as_str() {
					"wl_compositor" => {
						this.compositor = Some(registry.bind(name, version, qh, ()));
					}
					"xdg_wm_base" => {
						this.xdg_wm_base = Some(registry.bind(name, version, qh, ()));
					}
					"wl_seat" => {
						this.wl_seat = Some(registry.bind(name, version, qh, ()));
					}
					"wl_output" => {
						this.wl_output = Some(registry.bind(name, version, qh, ()));
					}
					"wl_surface" => {
						this.wl_surface = Some(registry.bind(name, version, qh, ()));
					}
					"wl_callback" => {
						this.wl_callback = Some(registry.bind(name, version, qh, ()));
					}
					"zwp_relative_pointer_manager_v1" => {
						this.zwp_relative_pointer_manager = Some(registry.bind(name, version, qh, ()));
					}
					"zwp_pointer_constraints_v1" => {
						this.zwp_pointer_constraints = Some(registry.bind(name, version, qh, ()));
					}
					_ => {}
				}
			}
			wayland_client::protocol::wl_registry::Event::GlobalRemove { .. } => {
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_region::WlRegion, ()> for AppData {
    fn event(_: &mut Self, _: &wl_region::WlRegion, event: wl_region::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_callback::WlCallback, ()> for AppData {
    fn event(_: &mut Self, _: &wl_callback::WlCallback, event: wl_callback::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			wl_callback::Event::Done { .. } => {
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_compositor::WlCompositor, ()> for AppData {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, event: wl_compositor::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()> for AppData {
    fn event(this: &mut Self, surface: &wl_surface::WlSurface, event: wl_surface::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			wayland_client::protocol::wl_surface::Event::Enter { .. } => {
			}
			wayland_client::protocol::wl_surface::Event::Leave { .. } => {
				this.state.extent = None;
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } => {
				this.state.scale = this.state.scale.max(factor as _);
				surface.set_buffer_scale(factor);
				surface.commit();
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferTransform { .. } => {
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<xdg_wm_base::XdgWmBase, ()> for AppData {
    fn event(_: &mut Self, s: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			xdg_wm_base::Event::Ping { serial } => {
				s.pong(serial);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<xdg_surface::XdgSurface, ()> for AppData {
    fn event(_: &mut Self, s: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			xdg_surface::Event::Configure { serial } => {
				s.ack_configure(serial);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<xdg_toplevel::XdgToplevel, ()> for AppData {
    fn event(this: &mut Self, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			xdg_toplevel::Event::WmCapabilities { .. } => {
			}
			xdg_toplevel::Event::ConfigureBounds { .. } => {
				// Suggested size
			}
			xdg_toplevel::Event::Configure { width, height, .. } => {
				if width != 0 && height != 0 {
					let extent = Extent::rectangle((width * (this.state.scale as i32)) as u32, (height * (this.state.scale as i32)) as u32);
					this.state.extent = Some(extent);
				}
			}
			xdg_toplevel::Event::Close => {
				this.events.push_back(Events::Close);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_seat::WlSeat, ()> for AppData {
	fn event(this: &mut Self, s: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_seat::Event::Capabilities { capabilities } => {
				let capabilities = capabilities.into_result().unwrap();

				if capabilities.contains(wl_seat::Capability::Pointer) {
					let pointer = s.get_pointer(qh, ());

					this.zwp_relative_pointer_manager.get_relative_pointer(&pointer, qh, ());
				}

				if capabilities.contains(wl_seat::Capability::Keyboard) {
					let _ = s.get_keyboard(qh, ());
				}
			}
			wl_seat::Event::Name { .. } => {
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_pointer::WlPointer, ()> for AppData {
	fn event(this: &mut Self, pointer: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_pointer::Event::Enter { .. } => {
				this.state.focused_pointer = Some(pointer.clone());

				this.process_requests(qh);
			}
			wl_pointer::Event::Leave { .. } => {
				if let Some(pointer) = &this.state.focused_pointer {
					if pointer == pointer {
						this.state.focused_pointer = None;
					}
				}

				this.process_requests(qh);
			}
			wl_pointer::Event::Button { button, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_pointer::ButtonState::Pressed;

				let button = match button {
					272 => MouseKeys::Left,
					2 => MouseKeys::Middle,
					273 => MouseKeys::Right,
					4 => MouseKeys::ScrollUp,
					5 => MouseKeys::ScrollDown,
					_ => return,
				};

				this.events.push_back(Events::Button { pressed, button });
			}
			wl_pointer::Event::Axis { axis, value, .. } => {
				let _ = match axis.into_result().unwrap() {
					wl_pointer::Axis::VerticalScroll => MouseKeys::ScrollUp,
					wl_pointer::Axis::HorizontalScroll => MouseKeys::ScrollDown,
					_ => return,
				};

				let _ = value > 0.0;
			}
			wl_pointer::Event::Motion { time, surface_x, surface_y } => {
				// if let Some(extent) = this.state.extent {
				// 	let x = surface_x as f32 * this.state.scale as f32;
				// 	let y = surface_y as f32 * this.state.scale as f32;

				// 	let width = extent.width() as f32;
				// 	let height = extent.height() as f32;

				// 	let half_width = width / 2.0;
				// 	let half_height = height / 2.0;

				// 	let x = (x - half_width) / half_width;
				// 	let y = (half_height - y) / half_height;

				// 	this.events.push_back(WindowEvents::MouseMove { x, y, time: time as u64 });
				// }
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_keyboard::WlKeyboard, ()> for AppData {
	fn event(this: &mut Self, keyboard: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_keyboard::Event::Key { key, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_keyboard::KeyState::Pressed;

				let key = match key {
					1 => Keys::Escape,
					2 => Keys::F1,
					3 => Keys::F2,
					4 => Keys::F3,
					5 => Keys::F4,
					6 => Keys::F5,
					7 => Keys::F6,
					8 => Keys::F7,
					9 => Keys::F8,
					10 => Keys::F9,
					11 => Keys::F10,
					12 => Keys::F11,
					13 => Keys::F12,
					14 => Keys::PrintScreen,
					15 => Keys::ScrollLock,
					17 => Keys::W,
					18 => Keys::Home,
					19 => Keys::PageUp,
					20 => Keys::Delete,
					21 => Keys::End,
					22 => Keys::PageDown,
					23 => Keys::ArrowRight,
					24 => Keys::ArrowLeft,
					25 => Keys::ArrowDown,
					26 => Keys::ArrowUp,
					27 => Keys::NumLock,
					30 => Keys::A,
					31 => Keys::S,
					32 => Keys::D,
					57 => Keys::Space,
					_ => return,
				};

				this.events.push_back(Events::Key { pressed, key });
			}
			wl_keyboard::Event::Keymap { .. } => {
			}
			wl_keyboard::Event::Enter { .. } => {
				this.state.focused_keyboard = Some(keyboard.clone());

				this.process_requests(qh);
			}
			wl_keyboard::Event::Leave { .. } => {
				if let Some(focused_keyboard) = &this.state.focused_keyboard {
					if focused_keyboard == keyboard {
						this.state.focused_keyboard = None;
					}
				}

				this.process_requests(qh);
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_output::WlOutput, ()> for AppData {
	fn event(this: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_output::Event::Scale { factor } => {
				this.state.scale = this.state.scale.max(factor as _);
			}
			wl_output::Event::Geometry { .. } => {
			}
			wl_output::Event::Mode { width, height, .. } => {
				this.state.monitor_extent = Some(Extent::rectangle(width as _, height as _));

				dbg!(this.state.monitor_extent);
			}
			wl_output::Event::Description { .. } => {
			}
			wl_output::Event::Name { .. } => {
			}
			wl_output::Event::Done => {
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, ()> for AppData {
	fn event(_: &mut Self, _: &zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, event: zwp_relative_pointer_manager_v1::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for AppData {
	fn event(this: &mut Self, _: &zwp_relative_pointer_v1::ZwpRelativePointerV1, event: zwp_relative_pointer_v1::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			zwp_relative_pointer_v1::Event::RelativeMotion { utime_lo, utime_hi, dx_unaccel, dy_unaccel, .. } => {
				let location = &mut this.state.pointer_location;

				location.0 += dx_unaccel as f32;
				location.1 += -dy_unaccel as f32;

				if let Some(extent) = this.state.extent {
					let width = extent.width() as f32;
					let height = extent.height() as f32;

					let x = location.0 / width;
					let y = location.1 / height;

					this.events.push_back(Events::MouseMove { x, y, time: (utime_hi as u64) << 32 | utime_lo as u64 });
				}
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, ()> for AppData {
	fn event(_: &mut Self, _: &zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, event: zwp_pointer_constraints_v1::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_confined_pointer_v1::ZwpConfinedPointerV1, ()> for AppData {
	fn event(_: &mut Self, _: &zwp_confined_pointer_v1::ZwpConfinedPointerV1, event: zwp_confined_pointer_v1::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			zwp_confined_pointer_v1::Event::Confined => {
				println!("Pointer is confined");
			}
			zwp_confined_pointer_v1::Event::Unconfined => {
				println!("Pointer is unconfined");
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for AppData {
	fn event(_: &mut Self, _: &zwp_locked_pointer_v1::ZwpLockedPointerV1, event: zwp_locked_pointer_v1::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			zwp_locked_pointer_v1::Event::Locked => {
				println!("Pointer is locked");
			}
			zwp_locked_pointer_v1::Event::Unlocked => {
				println!("Pointer is unlocked");
			}
			_ => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_wayland_window() {
		// Only run this test if we are on a Wayland session
		if std::env::vars().find(|(key, _)| key == "WAYLAND_DISPLAY").is_some() && std::env::vars().find(|(key, value)| key == "XDG_SESSION_TYPE" && value == "wayland").is_some() {
			let _ = WaylandWindow::try_new("My Test Wayland Window", Extent::rectangle(1920, 1080), "my_test_wayland_window.byte_engine").unwrap();
		}
	}
}
