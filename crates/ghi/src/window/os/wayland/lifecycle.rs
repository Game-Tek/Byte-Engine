use super::*;

impl WindowLike for Window {
	fn try_new(name: &str, extent: Extent, id_name: &str, _features: Features) -> Result<Self, String> {
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

			configuration_event_queue
				.roundtrip(&mut configuration)
				.map_err(|e| format!("Failed to roundtrip configuration event queue: {}", e))?;

			if let (Some(compositor), Some(wm_base), Some(zwp_pointer_constraints), Some(zwp_relative_pointer_manager)) = (
				configuration.compositor,
				configuration.xdg_wm_base,
				configuration.zwp_pointer_constraints,
				configuration.zwp_relative_pointer_manager,
			) {
				Ok((compositor, wm_base, zwp_pointer_constraints, zwp_relative_pointer_manager))
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

			app_event_queue
				.roundtrip(&mut app_data)
				.map_err(|e| format!("Failed to initialize Wayland app event queue: {}", e))?;

			surface.set_buffer_scale(app_data.state.scale as _);
			xdg_surface.set_window_geometry(0, 0, extent.width() as _, extent.height() as _);

			surface.commit();

			while !app_data.state.configured {
				app_event_queue
					.blocking_dispatch(&mut app_data)
					.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {}", e))?;
			}

			app_data.state
		};

		let mut requests = VecDeque::with_capacity(16);

		requests.push_back(Requests::ConstrainPointer);
		// requests.push_back(Requests::LockPointer);
		// requests.push_back(Requests::HidePointer);

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

	fn handles(&self) -> Handles {
		Handles {
			display: self.display().id().as_ptr() as _,
			surface: self.surface().id().as_ptr() as _,
		}
	}

	fn show_cursor(&mut self, show: bool) {
		self.state.should_hide_pointer = !show;
		self.requests.push_back(Requests::HidePointer);
	}

	fn confine_cursor(&mut self, confine: bool) {
		self.state.should_confine_pointer = confine;
		self.requests.push_back(Requests::ConstrainPointer);
	}

	fn poll<'a>(&'a mut self) -> impl Iterator<Item = Events> + 'a {
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

impl Window {
	pub fn display(&self) -> wl_display::WlDisplay {
		self.connection.display()
	}

	pub fn surface(&self) -> wl_surface::WlSurface {
		self.surface.clone()
	}
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

impl Drop for Window {
	fn drop(&mut self) {
		if let Some(confined_pointer) = self.state.confined_pointer.take() {
			confined_pointer.destroy();
		}
		if let Some(locked_pointer) = self.state.locked_pointer.take() {
			locked_pointer.destroy();
		}

		self.xdg_toplevel.destroy();
		self.xdg_surface.destroy();
		self.surface.destroy();
		self.xdg_wm_base.destroy();
	}
}
impl AppData {
	fn process_requests(&mut self, qh: &wayland_client::QueueHandle<Self>) {
		let surface = &self.wl_surface;

		self.requests.retain(|e| match e {
			Requests::ConstrainPointer => {
				if !self.state.should_confine_pointer {
					if let Some(confined_pointer) = self.state.confined_pointer.take() {
						confined_pointer.destroy();
					}

					self.state.pointer_is_confined = false;

					return false;
				}

				if self.state.should_lock_pointer {
					if let Some(confined_pointer) = self.state.confined_pointer.take() {
						confined_pointer.destroy();
					}

					self.state.pointer_is_confined = false;

					return false;
				}

				if self.state.pointer_is_confined {
					return false;
				}

				let focused_pointer = self.state.focused_pointer.clone();
				let focused_keyboard = self.state.focused_keyboard.clone();

				if let (Some(pointer), Some(_)) = (focused_pointer, focused_keyboard) {
					if let Some(confined_pointer) = self.state.confined_pointer.take() {
						confined_pointer.destroy();
					}

					let confined_pointer = self.zwp_pointer_constraints.confine_pointer(
						surface,
						&pointer,
						None,
						zwp_pointer_constraints_v1::Lifetime::Persistent,
						&qh,
						(),
					);
					self.state.confined_pointer = Some(confined_pointer);
					self.state.pointer_is_confined = false;

					surface.commit();

					false
				} else {
					true
				}
			}
			Requests::LockPointer => {
				if !self.state.should_lock_pointer {
					if let Some(locked_pointer) = self.state.locked_pointer.take() {
						locked_pointer.destroy();
					}

					self.state.pointer_is_locked = false;

					return false;
				}

				if self.state.pointer_is_locked {
					return false;
				}

				let focused_pointer = self.state.focused_pointer.clone();
				let focused_keyboard = self.state.focused_keyboard.clone();

				if let (Some(pointer), Some(_)) = (focused_pointer, focused_keyboard) {
					if let Some(locked_pointer) = self.state.locked_pointer.take() {
						locked_pointer.destroy();
					}

					if let Some(confined_pointer) = self.state.confined_pointer.take() {
						confined_pointer.destroy();
					}

					self.state.pointer_is_confined = false;

					let locked_pointer = self.zwp_pointer_constraints.lock_pointer(
						surface,
						&pointer,
						None,
						zwp_pointer_constraints_v1::Lifetime::Persistent,
						&qh,
						(),
					);
					self.state.locked_pointer = Some(locked_pointer);
					self.state.pointer_is_locked = false;

					surface.commit();

					false
				} else {
					true
				}
			}
			Requests::HidePointer => {
				if !self.state.should_hide_pointer {
					self.state.pointer_is_hidden = false;
					return false;
				}

				if let Some(pointer) = &self.state.focused_pointer {
					pointer.set_cursor(0, None, 0, 0);
					self.state.pointer_is_hidden = true;

					surface.commit();

					false
				} else {
					true
				}
			}
		});
	}
}
