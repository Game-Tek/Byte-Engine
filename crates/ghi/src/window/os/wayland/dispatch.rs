use super::*;

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()> for Configuration {
	fn event(
		this: &mut Self,
		registry: &wl_registry::WlRegistry,
		event: wl_registry::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<Configuration>,
	) {
		let qh = &this.app_data_queue;

		match event {
			wayland_client::protocol::wl_registry::Event::Global {
				name,
				interface,
				version,
			} => match interface.as_str() {
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
			},
			wayland_client::protocol::wl_registry::Event::GlobalRemove { .. } => {}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_region::WlRegion, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &wl_region::WlRegion,
		event: wl_region::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_callback::WlCallback, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &wl_callback::WlCallback,
		event: wl_callback::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_callback::Event::Done { .. } => {}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_compositor::WlCompositor, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &wl_compositor::WlCompositor,
		event: wl_compositor::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_surface::WlSurface, ()> for AppData {
	fn event(
		this: &mut Self,
		surface: &wl_surface::WlSurface,
		event: wl_surface::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wayland_client::protocol::wl_surface::Event::Enter { .. } => {}
			wayland_client::protocol::wl_surface::Event::Leave { .. } => {
				this.state.extent = None;
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } => {
				this.state.scale = this.state.scale.max(factor as _);
				surface.set_buffer_scale(factor);
				surface.commit();
			}
			wayland_client::protocol::wl_surface::Event::PreferredBufferTransform { .. } => {}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<xdg_wm_base::XdgWmBase, ()> for AppData {
	fn event(
		_: &mut Self,
		s: &xdg_wm_base::XdgWmBase,
		event: xdg_wm_base::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			xdg_wm_base::Event::Ping { serial } => {
				s.pong(serial);
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<xdg_surface::XdgSurface, ()> for AppData {
	fn event(
		this: &mut Self,
		s: &xdg_surface::XdgSurface,
		event: xdg_surface::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			xdg_surface::Event::Configure { serial } => {
				s.ack_configure(serial);
				this.state.configured = true;
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<xdg_toplevel::XdgToplevel, ()> for AppData {
	fn event(
		this: &mut Self,
		_: &xdg_toplevel::XdgToplevel,
		event: xdg_toplevel::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			xdg_toplevel::Event::WmCapabilities { .. } => {}
			xdg_toplevel::Event::ConfigureBounds { .. } => {
				// Suggested size
			}
			xdg_toplevel::Event::Configure { width, height, .. } => {
				if width != 0 && height != 0 {
					let extent = Extent::rectangle(
						(width * (this.state.scale as i32)) as u32,
						(height * (this.state.scale as i32)) as u32,
					);
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
