use std::ffi::c_void;

use wayland_client::{protocol::{wl_callback, wl_compositor, wl_display, wl_registry, wl_surface}, Proxy};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::WindowEvents;

pub struct WaylandWindow {
	connection: wayland_client::Connection,
	display: wl_display::WlDisplay,
	registry: wl_registry::WlRegistry,
	compositor: wl_compositor::WlCompositor,
	xdg_wm_base: xdg_wm_base::XdgWmBase,
	surface: wl_surface::WlSurface,
	xdg_surface: xdg_surface::XdgSurface,
	xdg_toplevel: xdg_toplevel::XdgToplevel,
}

impl WaylandWindow {
	pub fn try_new() -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let display = conn.display();

		let mut event_queue = conn.new_event_queue();
		let qh = event_queue.handle();

		let registry = display.get_registry(&qh, ());

		event_queue.roundtrip(&mut AppData).unwrap();

		let compositor: wl_compositor::WlCompositor = registry.bind(1, 5, &qh, ()); // TODO: make dynamic from advertized globals

		let surface = compositor.create_surface(&qh, ());

		let wm_base: xdg_wm_base::XdgWmBase = registry.bind(13, 4, &qh, ()); // TODO: make dynamic from advertized globals

		let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());

		let toplevel = xdg_surface.get_toplevel(&qh, ());

		toplevel.set_title("My Wayland Window".to_string());

		toplevel.set_maximized();

		surface.commit();
		display.sync(&qh, ());
		surface.commit();

		event_queue.roundtrip(&mut AppData).unwrap();

		Ok(Self {
			connection: conn,
			display,
			registry,
			compositor,
			xdg_wm_base: wm_base,
			surface,
			xdg_surface,
			xdg_toplevel: toplevel,
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
	
	pub fn poll(&self) -> WindowIterator {
		let mut event_queue = self.connection.new_event_queue();
		// let qh = event_queue.handle();
		let n = event_queue.dispatch_pending(&mut  AppData).unwrap();
		WindowIterator {
			connection: &self.connection,
		}
	}
}

pub struct WindowIterator<'a> {
	connection: &'a wayland_client::Connection,
}

impl Iterator for WindowIterator<'_> {
	type Item = WindowEvents;

	fn next(&mut self) -> Option<WindowEvents> {	
		let connection = self.connection;

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
		// self.compositor.destroy();
		// self.registry.destroy();
		// self.display.disconnect();
	}
}

pub struct OSHandles {
	pub display: *mut c_void,
	pub surface: *mut c_void,
}

struct AppData;

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for AppData {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
		match event {
			wayland_client::protocol::wl_registry::Event::Global { name, interface, version } => {
				println!("[{}] {} (v{})", name, interface, version);
			}
			wayland_client::protocol::wl_registry::Event::GlobalRemove { name } => {
				println!("Removed global {}", name);
			}
			_ => {}
		}
    }
}

impl wayland_client::Dispatch<wl_callback::WlCallback, ()> for AppData {
    fn event(_: &mut Self, _: &wl_callback::WlCallback, event: wl_callback::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			wl_callback::Event::Done { callback_data } => {
				println!("Done: {}", callback_data);
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
    fn event(_: &mut Self, _: &wl_surface::WlSurface, event: wl_surface::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
        match event {
			wayland_client::protocol::wl_surface::Event::Enter { .. } => {
				println!("Enter");
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
    fn event(_: &mut Self, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &wayland_client::Connection, _: &wayland_client::QueueHandle<AppData>,) {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_wayland_window() {
		let window = WaylandWindow::try_new().unwrap();
	}
}