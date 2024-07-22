use std::ffi::c_void;

use wayland_client::{protocol::{wl_callback, wl_compositor, wl_display, wl_output, wl_registry, wl_seat, wl_surface}, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::WindowEvents;

pub struct WaylandWindow {
	connection: wayland_client::Connection,
	display: wl_display::WlDisplay,
	registry: wl_registry::WlRegistry,
	compositor: wl_compositor::WlCompositor,
	wl_output: wl_output::WlOutput,
	xdg_wm_base: xdg_wm_base::XdgWmBase,
	surface: wl_surface::WlSurface,
	xdg_surface: xdg_surface::XdgSurface,
	wl_seat: wl_seat::WlSeat,
	xdg_toplevel: xdg_toplevel::XdgToplevel,
	app_data: AppData,
}

impl WaylandWindow {
	pub fn try_new() -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let display = conn.display();

		let mut event_queue = conn.new_event_queue();
		let qh = event_queue.handle();

		let registry = display.get_registry(&qh, ());

		let mut app_data = AppData {
			compositor: None,
			xdg_wm_base: None,
			wl_seat: None,
			wl_output: None,

			scale: 1,

			wl_surface: None,
			wl_callback: None,
		};

		event_queue.roundtrip(&mut app_data).unwrap();

		let (compositor_name, compositor_version) = app_data.compositor.ok_or("Compositor not found")?;

		let compositor: wl_compositor::WlCompositor = registry.bind(compositor_name, compositor_version, &qh, ());

		let (wl_output_name, wl_output_version) = app_data.wl_output.ok_or("WlOutput not found")?;

		let wl_output: wl_output::WlOutput = registry.bind(wl_output_name, wl_output_version, &qh, ());

		let surface = compositor.create_surface(&qh, ());

		surface.set_buffer_scale(app_data.scale as _);

		surface.commit();

		app_data.wl_surface = Some(surface.clone());
		app_data.wl_callback = Some(surface.frame(&qh, ()));

		let (xdg_wm_base_name, xdg_wm_base_version) = app_data.xdg_wm_base.ok_or("XdgWmBase not found")?;

		let wm_base: xdg_wm_base::XdgWmBase = registry.bind(xdg_wm_base_name, xdg_wm_base_version, &qh, ());

		let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());

		let (wm_seat_name, wm_seat_version) = app_data.wl_seat.ok_or("WlSeat not found")?;

		let wl_seat: wl_seat::WlSeat = registry.bind(wm_seat_name, wm_seat_version, &qh, ());

		let toplevel = xdg_surface.get_toplevel(&qh, ());

		toplevel.set_title("My Wayland Window".to_string());

		toplevel.set_maximized();

		surface.commit();
		display.sync(&qh, ());
		surface.commit();

		event_queue.roundtrip(&mut app_data).unwrap();

		Ok(Self {
			connection: conn,
			display,
			registry,
			compositor,
			wl_output,
			xdg_wm_base: wm_base,
			surface,
			xdg_surface,
			wl_seat,
			xdg_toplevel: toplevel,

			app_data,
		})
	}

	pub fn display(&self) -> &wl_display::WlDisplay {
		&self.display
	}

	pub fn surface(&self) -> &wl_surface::WlSurface {
		&self.surface
	}

	pub fn get_os_handles(&self) -> OSHandles {
		OSHandles {
			display: self.display.id().as_ptr() as *mut c_void,
			surface: self.surface.id().as_ptr() as *mut c_void,
		}
	}
	
	pub fn poll(&mut self) -> WindowIterator {
		print!("Polling\n");
		let mut app_data = &mut self.app_data;
		let mut event_queue = self.connection.new_event_queue();
		event_queue.roundtrip(app_data).unwrap();
		event_queue.blocking_dispatch(app_data).unwrap();
		WindowIterator {
			window: self,
			event_queue,
		}
	}
}

pub struct WindowIterator<'a> {
	window: &'a WaylandWindow,
	event_queue: wayland_client::EventQueue<AppData>,
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {	
		let mut app_data = &self.window.app_data;
		let connection = &self.window.connection;

		loop {
			return None;
		}
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
	compositor: Option<(u32, u32)>,
	xdg_wm_base: Option<(u32, u32)>,
	wl_seat: Option<(u32, u32)>,
	wl_output: Option<(u32, u32)>,

	scale: u32,

	wl_surface: Option<wl_surface::WlSurface>,
	wl_callback: Option<wl_callback::WlCallback>,
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for AppData {
    fn event(this: &mut Self, _: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wayland_client::protocol::wl_registry::Event::Global { name, interface, version } => {
				match interface.as_str() {
					"wl_compositor" => {
						println!("Global: {}, {}, {}", name, interface, version);
						this.compositor = Some((name, version));
					}
					"xdg_wm_base" => {
						println!("Global: {}, {}, {}", name, interface, version);
						this.xdg_wm_base = Some((name, version));
					}
					"wl_seat" => {
						println!("Global: {}, {}, {}", name, interface, version);
						this.wl_seat = Some((name, version));
					}
					"wl_output" => {
						println!("Global: {}, {}, {}", name, interface, version);
						this.wl_output = Some((name, version));
					}
					_ => {}
				}
			}
			wayland_client::protocol::wl_registry::Event::GlobalRemove { name } => {
				println!("Removed global {}", name);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_callback::WlCallback, ()> for AppData {
    fn event(this: &mut Self, callback: &wl_callback::WlCallback, event: wl_callback::Event, _: &(), _: &wayland_client::Connection, qh: &wayland_client::QueueHandle<AppData>,) {
        match event {
			wl_callback::Event::Done { callback_data } => {
				println!("Callback done: {}", callback_data);

				if let Some(wl_surface) = &mut this.wl_surface {
					// this.wl_callback = Some(wl_surface.frame(qh, ()));
					// wl_surface.commit();
				}
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
			wayland_client::protocol::wl_surface::Event::Enter { output } => {
				println!("Enter: {:?}", output);

				surface.set_buffer_scale(this.scale as _);

				println!("Set buffer scale to {}", this.scale);
			}
			wayland_client::protocol::wl_surface::Event::Leave { .. } => {
				println!("Leave");
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } => {
				println!("Preferred buffer scale: {}", factor);
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferTransform { .. } => {
				println!("Preferred buffer transform");
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
				println!("Configure: {}", serial);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<xdg_toplevel::XdgToplevel, ()> for AppData {
    fn event(_: &mut Self, s: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			xdg_toplevel::Event::WmCapabilities { capabilities } => {
				println!("Capabilties:");
				for e in capabilities {
					println!("	- {}", e);
				}
			}
			xdg_toplevel::Event::ConfigureBounds { width, height } => {
				println!("Configure bounds: [{}, {}]", width, height);
			}
			xdg_toplevel::Event::Close => {
				println!("Closed!");
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_seat::WlSeat, ()> for AppData {
	fn event(_: &mut Self, s: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_seat::Event::Capabilities { capabilities } => {
				println!("Capabilities: {:?}", capabilities);
			}
			wl_seat::Event::Name { name } => {
				println!("Name: {:?}", name);
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_output::WlOutput, ()> for AppData {
	fn event(this: &mut Self, s: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wl_output::Event::Scale { factor } => {
				println!("Scale: {}", factor);
				this.scale = this.scale.max(factor as _);
			}
			wl_output::Event::Geometry { x, y, physical_width, physical_height, subpixel, make, model, transform } => {
				println!("Geometry: [{}, {}] {}x{} {:?} {} {} {:?}", x, y, physical_width, physical_height, subpixel, make, model, transform);
			}
			wl_output::Event::Mode { flags, width, height, refresh } => {
				println!("Mode: {:?} [{}, {}] @ {}", flags, width, height, refresh);
			}
			wl_output::Event::Description { description } => {
				println!("Description: {:?}", description);
			}
			wl_output::Event::Name { name } => {
				println!("Name: {:?}", name);
			}
			wl_output::Event::Done => {
				println!("Done");
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
			let window = WaylandWindow::try_new().unwrap();
		}
	}
}