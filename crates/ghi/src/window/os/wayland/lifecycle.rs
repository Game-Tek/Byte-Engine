use super::*;

impl AppLike for App {
	type Window = Window;

	fn try_new(id_name: &str) -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let mut configuration_event_queue: wayland_client::EventQueue<Configuration> = conn.new_event_queue();
		let configuration_qh = configuration_event_queue.handle();

		let event_queue = conn.new_event_queue();

		let display = conn.display();

		let _ = display.get_registry(&configuration_qh, ());

		// Get globals
		let (compositor, xdg_wm_base, zwp_relative_pointer_manager) = {
			let mut configuration = Configuration {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,

				wl_callback: None,

				zwp_relative_pointer_manager: None,

				app_data_queue: event_queue.handle(),
			};

			configuration_event_queue
				.roundtrip(&mut configuration)
				.map_err(|e| format!("Failed to roundtrip configuration event queue: {}", e))?;

			if let (Some(compositor), Some(wm_base), Some(zwp_relative_pointer_manager)) = (
				configuration.compositor,
				configuration.xdg_wm_base,
				configuration.zwp_relative_pointer_manager,
			) {
				Ok((compositor, wm_base, zwp_relative_pointer_manager))
			} else {
				Err("Failed to acquire all required globals".to_string())
			}
		}?;

		let mut app = App {
			connection: conn,
			event_queue,
			data: AppData {
				compositor,
				xdg_wm_base,
				zwp_relative_pointer_manager,
				windows: Vec::with_capacity(4),
				output_scale: 1,
				monitor_extent: None,
				outputs: Vec::new(),
				pointer_focus: None,
				keyboard_focus: None,
				keyboard_state: None,
				events: VecDeque::with_capacity(64),
			},
			id_name: id_name.to_owned(),
			next_window: 1,
		};

		// Receive seat and output state before the first window picks its scale.
		app.event_queue
			.roundtrip(&mut app.data)
			.map_err(|e| format!("Failed to initialize Wayland app event queue: {}", e))?;

		Ok(app)
	}

	fn create_window(&mut self, name: &str, extent: Extent, _features: Features) -> Result<Window, String> {
		let id = WindowId::from_raw(self.next_window);
		self.next_window += 1;

		let qh = self.event_queue.handle();

		let surface = self.data.compositor.create_surface(&qh, id);
		let xdg_surface = self.data.xdg_wm_base.get_xdg_surface(&surface, &qh, id);
		let toplevel = xdg_surface.get_toplevel(&qh, id);

		toplevel.set_title(name.to_string());
		toplevel.set_app_id(self.id_name.clone());

		let scale = self.data.output_scale;
		let refresh = SharedRefresh::default();
		self.data.windows.push(WindowState {
			id,
			surface: surface.clone(),
			scale,
			extent: None,
			configured: false,
			outputs: Vec::new(),
			refresh: refresh.clone(),
		});

		surface.set_buffer_scale(scale as _);
		xdg_surface.set_window_geometry(0, 0, extent.width() as _, extent.height() as _);

		surface.commit();

		// Other windows' events dispatched while waiting stay queued for the next poll.
		while !self.data.window(id).is_some_and(|window| window.configured) {
			self.event_queue
				.blocking_dispatch(&mut self.data)
				.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {}", e))?;
		}

		Ok(Window {
			id,
			display: self.connection.display(),
			surface,
			xdg_surface,
			xdg_toplevel: toplevel,
			refresh,
		})
	}

	fn poll(&mut self) -> impl Iterator<Item = Event> + '_ {
		self.data.forget_destroyed_windows();

		self.event_queue
			.flush()
			.expect("Failed to flush Wayland requests. The most likely cause is that the compositor connection was lost.");

		if let Some(guard) = self.event_queue.prepare_read() {
			match guard.read() {
				Ok(_) => {}
				// The socket is non-blocking, so an empty socket only means there is nothing new.
				Err(wayland_client::backend::WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
				Err(error) => panic!(
					"Failed to read Wayland events: {error}. The most likely cause is that the compositor connection was lost."
				),
			}
		}

		self.event_queue
			.dispatch_pending(&mut self.data)
			.expect("Failed to dispatch Wayland events. The most likely cause is a protocol error reported by the compositor.");

		std::iter::from_fn(move || self.data.events.pop_front())
	}
}

impl Drop for App {
	fn drop(&mut self) {
		self.data.xdg_wm_base.destroy();
	}
}

impl WindowLike for Window {
	fn id(&self) -> WindowId {
		self.id
	}

	fn handles(&self) -> Handles {
		Handles {
			display: self.display.id().as_ptr() as _,
			surface: self.surface.id().as_ptr() as _,
		}
	}

	fn refresh_interval(&self) -> Option<Duration> {
		match self.refresh.load(Ordering::Relaxed) {
			0 => None,
			nanoseconds => Some(Duration::from_nanos(nanoseconds)),
		}
	}
}

impl Drop for Window {
	fn drop(&mut self) {
		self.xdg_toplevel.destroy();
		self.xdg_surface.destroy();
		self.surface.destroy();
	}
}

impl AppData {
	pub(super) fn window(&self, id: WindowId) -> Option<&WindowState> {
		self.windows.iter().find(|window| window.id == id)
	}

	pub(super) fn window_mut(&mut self, id: WindowId) -> Option<&mut WindowState> {
		self.windows.iter_mut().find(|window| window.id == id)
	}

	pub(super) fn push(&mut self, window: WindowId, event: Events) {
		self.events.push_back(Event::Window { window, event });
	}

	/// Recomputes a window's refresh interval from the outputs it is on and reports a change.
	pub(super) fn update_window_refresh(&mut self, id: WindowId) {
		let Some(window) = self.windows.iter().find(|window| window.id == id) else {
			return;
		};
		let interval = window
			.outputs
			.iter()
			.filter_map(|output| self.outputs.iter().find(|(known, _)| known == output))
			.filter_map(|(_, millihertz)| refresh_interval(*millihertz))
			.min();
		let nanoseconds = interval.map_or(0, |interval| interval.as_nanos() as u64);
		if window.refresh.swap(nanoseconds, Ordering::Relaxed) != nanoseconds {
			self.push(id, Events::DisplayChanged { refresh_interval: interval });
		}
	}

	/// Drops the state of windows whose [`Window`] handle destroyed their surface, along with any focus on them.
	fn forget_destroyed_windows(&mut self) {
		self.windows.retain(|window| window.surface.is_alive());

		if self.pointer_focus.as_ref().is_some_and(|(_, id)| self.window(*id).is_none()) {
			self.pointer_focus = None;
		}
		if self.keyboard_focus.as_ref().is_some_and(|(_, id)| self.window(*id).is_none()) {
			self.keyboard_focus = None;
		}
	}
}
