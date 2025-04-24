use std::{collections::VecDeque, ffi::c_void};

use utils::Extent;
use wayland_client::{protocol::{wl_callback, wl_compositor::{self, WlCompositor}, wl_display, wl_keyboard, wl_output::{self, WlOutput}, wl_pointer, wl_registry, wl_seat::{self, WlSeat}, wl_surface}, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base::{self, XdgWmBase}};

use crate::{Keys, MouseKeys, WindowEvents};

pub struct WaylandWindow {
	connection: wayland_client::Connection,
	event_queue: wayland_client::EventQueue<AppData>,
	xdg_wm_base: xdg_wm_base::XdgWmBase,
	surface: wl_surface::WlSurface,
	xdg_surface: xdg_surface::XdgSurface,
	xdg_toplevel: xdg_toplevel::XdgToplevel,

	extent: Option<Extent>,
	scale: u32,
}

impl WaylandWindow {
	pub fn try_new(name: &str, extent: Extent, id_name: &str) -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let mut event_queue = conn.new_event_queue();
		let qh = event_queue.handle();

		let display = conn.display();
		let _ = display.get_registry(&qh, ());

		// Get globals
		let (compositor, wm_base) = {
			let mut app_data = AppData {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,
	
				scale: 1,
	
				wl_surface: None,
				wl_callback: None,
	
				events: VecDeque::with_capacity(64),

				extent: None,
			};

			event_queue.roundtrip(&mut app_data).unwrap();
	
			if let (Some(compositor), Some(wm_base)) = (app_data.compositor, app_data.xdg_wm_base) {
				Ok((compositor, wm_base))
			} else {
				Err("Failed to acquire all required globals".to_string())
			}
		}?;

		let surface = compositor.create_surface(&qh, ());

		let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());

		let toplevel = xdg_surface.get_toplevel(&qh, ());

		toplevel.set_title(name.to_string());
		toplevel.set_app_id(id_name.to_string());

		let reported_extent;
		let scale;

		{
			let mut app_data = AppData {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,
	
				scale: 1,
	
				wl_surface: None,
				wl_callback: None,
	
				events: VecDeque::with_capacity(64),

				extent: None,
			};
			
			event_queue.roundtrip(&mut app_data).unwrap();

			surface.set_buffer_scale(app_data.scale as _);
			xdg_surface.set_window_geometry(0, 0, extent.width() as _, extent.height() as _);

			surface.commit();

			reported_extent = app_data.extent;
			scale = app_data.scale;
		}

		Ok(Self {
			connection: conn,
			event_queue,
			xdg_wm_base: wm_base,
			surface,
			xdg_surface,
			xdg_toplevel: toplevel,
			extent: reported_extent,
			scale,
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
		let mut app_data = AppData {
			compositor: None,
			xdg_wm_base: None,
			wl_seat: None,
			wl_output: None,

			scale: self.scale,

			wl_surface: None,
			wl_callback: None,

			events: VecDeque::with_capacity(64),

			extent: self.extent,
		};

		let event_queue = &mut self.event_queue;
		event_queue.blocking_dispatch(&mut app_data).unwrap();

		self.extent = app_data.extent;

		WindowIterator {
			events: app_data.events,
		}
	}
}

pub struct WindowIterator {
	events: VecDeque<WindowEvents>,
}

impl Iterator for WindowIterator {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {
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

#[derive(Debug, Clone)]
struct AppData {
	compositor: Option<WlCompositor>,
	xdg_wm_base: Option<XdgWmBase>,
	wl_seat: Option<WlSeat>,
	wl_output: Option<WlOutput>,

	scale: u32,

	wl_surface: Option<wl_surface::WlSurface>,
	wl_callback: Option<wl_callback::WlCallback>,

	events: VecDeque<WindowEvents>,

	extent: Option<Extent>,
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for AppData {
    fn event(this: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wayland_client::protocol::wl_registry::Event::Global { name, interface, version } => {
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
					_ => {}
				}
			}
			wayland_client::protocol::wl_registry::Event::GlobalRemove { .. } => {
			}
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
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } => {
				this.scale = this.scale.max(factor as _);
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
					let extent = Extent::rectangle((width * (this.scale as i32)) as u32, (height * (this.scale as i32)) as u32);
					if this.extent != Some(extent) {
						this.events.push_back(WindowEvents::Resize{ width: extent.width(), height: extent.height() });
					}
					this.extent = Some(extent);
				}
			}
			xdg_toplevel::Event::Close => {
				this.events.push_back(WindowEvents::Close);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_seat::WlSeat, ()> for AppData {
	fn event(_: &mut Self, s: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_seat::Event::Capabilities { capabilities } => {
				let capabilities = capabilities.into_result().unwrap();

				if capabilities.contains(wl_seat::Capability::Pointer) {
					let _ = s.get_pointer(qh, ());
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
	fn event(this: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_pointer::Event::Button { button, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_pointer::ButtonState::Pressed;

				let button = match button {
					1 => MouseKeys::Left,
					2 => MouseKeys::Middle,
					3 => MouseKeys::Right,
					4 => MouseKeys::ScrollUp,
					5 => MouseKeys::ScrollDown,
					_ => return,
				};

				this.events.push_back(WindowEvents::Button { pressed, button });
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
				if let Some(extent) = this.extent {
					let x = surface_x as f32 * this.scale as f32;
					let y = surface_y as f32 * this.scale as f32;

					let width = extent.width() as f32;
					let height = extent.height() as f32;

					let half_width = width / 2.0;
					let half_height = height / 2.0;

					let x = (x - half_width) / half_width;
					let y = (half_height - y) / half_height;

					this.events.push_back(WindowEvents::MouseMove { x, y, time: time as u64 });
				}
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_keyboard::WlKeyboard, ()> for AppData {
	fn event(this: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
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
					_ => return,
				};

				this.events.push_back(WindowEvents::Key { pressed, key });
			}
			wl_keyboard::Event::Keymap { .. } => {
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_output::WlOutput, ()> for AppData {
	fn event(this: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_output::Event::Scale { factor } => {
				this.scale = this.scale.max(factor as _);
			}
			wl_output::Event::Geometry { .. } => {
			}
			wl_output::Event::Mode { .. } => {
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