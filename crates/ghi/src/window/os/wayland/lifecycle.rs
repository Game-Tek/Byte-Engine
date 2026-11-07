use std::os::fd::AsFd as _;

use super::*;

impl App {
	/// Connects to the windowing system; see [`crate::window::App::new`].
	pub(crate) fn try_new(id_name: &str) -> Result<Self, String> {
		let conn = wayland_client::Connection::connect_to_env().map_err(|e| e.to_string())?;

		let mut configuration_event_queue: wayland_client::EventQueue<Configuration> = conn.new_event_queue();
		let configuration_qh = configuration_event_queue.handle();

		let event_queue = conn.new_event_queue();

		let display = conn.display();

		let _ = display.get_registry(&configuration_qh, ());

		// Get globals
		let (compositor, xdg_wm_base, zwp_relative_pointer_manager, zwp_pointer_constraints, decoration_manager) = {
			let mut configuration = Configuration {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,

				wl_callback: None,

				zwp_relative_pointer_manager: None,
				zwp_pointer_constraints: None,
				decoration_manager: None,

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
				Ok((
					compositor,
					wm_base,
					zwp_relative_pointer_manager,
					configuration.zwp_pointer_constraints,
					configuration.decoration_manager,
				))
			} else {
				Err("Failed to acquire all required globals".to_string())
			}
		}?;

		let wake = rustix::event::eventfd(
			0,
			rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
		)
		.map_err(|error| format!("Failed to create the wake eventfd: {error}"))?;

		let mut app = App {
			connection: conn,
			event_queue,
			data: AppData {
				compositor,
				xdg_wm_base,
				zwp_relative_pointer_manager,
				zwp_pointer_constraints,
				decoration_manager,
				windows: Vec::with_capacity(4),
				output_scale: 1,
				monitor_extent: None,
				outputs: Vec::new(),
				pointer: None,
				pointer_focus: None,
				keyboard_focus: None,
				keyboard_state: None,
				events: VecDeque::with_capacity(64),
			},
			id_name: id_name.to_owned(),
			next_window: 1,
			wake: Arc::new(wake),
			watched: Vec::new(),
			portal: None,
			libdecor: std::cell::OnceCell::new(),
		};

		// Receive seat and output state before the first window picks its scale.
		app.event_queue
			.roundtrip(&mut app.data)
			.map_err(|e| format!("Failed to initialize Wayland app event queue: {}", e))?;

		Ok(app)
	}

	/// Creates and shows a native window; see [`crate::window::App::create_window`].
	pub(crate) fn create_window(&mut self, name: &str, extent: Extent, features: Features) -> Result<Window, String> {
		let id = WindowId::from_raw(self.next_window);
		self.next_window = self
			.next_window
			.checked_add(1)
			.expect("Window identities are exhausted. The application created u64::MAX windows. Restart the application.");

		let qh = self.event_queue.handle();

		let surface = self.data.compositor.create_surface(&qh, id);
		// A surface without a role is never mapped, so a background window renders without appearing or taking focus.
		let background = features.contains(Features::BACKGROUND);
		let decorated = features.contains(Features::DECORATIONS);
		// The compositor's own decorations come first. Without them, libdecor draws them, and without libdecor the
		// window opens undecorated.
		let libdecor = (!background && decorated && self.data.decoration_manager.is_none())
			.then(|| self.libdecor())
			.flatten();
		let role = match libdecor {
			Some(libdecor) => Some(Role::Libdecor {
				_frame: libdecor.decorate(
					&surface,
					id,
					name,
					&self.id_name,
					(extent.width() as i32, extent.height() as i32),
				)?,
			}),
			None if background => None,
			None => {
				let xdg_surface = self.data.xdg_wm_base.get_xdg_surface(&surface, &qh, id);
				let toplevel = xdg_surface.get_toplevel(&qh, id);
				toplevel.set_title(name.to_string());
				toplevel.set_app_id(self.id_name.clone());
				let decoration = self.data.decoration_manager.as_ref().filter(|_| decorated).map(|manager| {
					let decoration = manager.get_toplevel_decoration(&toplevel, &qh, id);
					decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
					decoration
				});
				Some(Role::Xdg {
					xdg_surface,
					toplevel,
					decoration,
				})
			}
		};

		let scale = self.data.output_scale;
		let refresh = SharedRefresh::default();
		let shared_extent = WindowExtent::default();
		self.data.windows.push(WindowState {
			id,
			surface: surface.clone(),
			scale,
			extent: None,
			size: (extent.width() as i32, extent.height() as i32),
			shared_extent: shared_extent.clone(),
			// The compositor configures only surfaces with a role.
			configured: background,
			outputs: Vec::new(),
			refresh: refresh.clone(),
			captures_cursor: features.contains(Features::CAPTURE_CURSOR) && !features.contains(Features::BACKGROUND),
			cursor_lock: None,
		});
		self.data.lock_cursors(&qh);
		// The requested extent is in surface units, like a configuration's. It stands until the compositor picks
		// another size, which a floating window leaves to the client.
		self.data.resize_toplevel(id, extent.width() as i32, extent.height() as i32);

		surface.set_buffer_scale(scale as _);
		match &role {
			Some(Role::Xdg { xdg_surface, .. }) => {
				xdg_surface.set_window_geometry(0, 0, extent.width() as _, extent.height() as _);
				surface.commit();
			}
			// libdecor commits the surface itself when it maps the frame.
			Some(Role::Libdecor { .. }) => {}
			None => surface.commit(),
		}

		// Other windows' events dispatched while waiting stay queued for the next poll.
		while !self.data.window(id).is_some_and(|window| window.configured) {
			match &role {
				// A libdecor frame is configured on the default queue, which only libdecor dispatches. Its wait reads
				// the socket for every queue, so the app's own events are dispatched after it.
				Some(Role::Libdecor { .. }) => {
					self.event_queue
						.flush()
						.map_err(|e| format!("Failed to flush Wayland requests while creating a window: {e}"))?;
					self.dispatch_libdecor(-1);
					self.event_queue
						.dispatch_pending(&mut self.data)
						.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {e}"))?;
				}
				_ => {
					self.event_queue
						.blocking_dispatch(&mut self.data)
						.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {e}"))?;
				}
			}
		}

		Ok(Window {
			id,
			display: self.connection.display(),
			surface,
			role,
			refresh,
			extent: shared_extent,
		})
	}

	/// Pumps the native queue and yields its events; see [`crate::window::App::poll`].
	pub(crate) fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		self.data.forget_destroyed_windows();

		self.event_queue
			.flush()
			.expect("Failed to flush Wayland requests. The most likely cause is that the compositor connection was lost.");

		// Portal replies read during an earlier request wait in the connection's buffer, where polling its descriptor
		// cannot see them. Likewise, libdecor events already read wait on the default queue, where the socket can't
		// signal them.
		self.drain_portal();
		self.dispatch_libdecor(0);

		// A guard only exists when nothing is queued for dispatch, so waiting behind it cannot hide events.
		if let Some(guard) = self.event_queue.prepare_read() {
			let timeout = match wait {
				Wait::Immediate => Some(Duration::ZERO),
				Wait::Until(deadline) => Some(deadline.saturating_duration_since(std::time::Instant::now())),
				Wait::Forever => None,
			};
			if timeout != Some(Duration::ZERO) && self.data.events.is_empty() {
				let timeout = timeout.map(|timeout| rustix::event::Timespec {
					tv_sec: timeout.as_secs() as _,
					tv_nsec: timeout.subsec_nanos() as _,
				});
				let connection = guard.connection_fd();
				let mut fds = [connection, self.wake.as_fd()]
					.into_iter()
					.chain(self.watched.iter().map(|fd| fd.as_fd()))
					.chain(self.portal.as_ref().map(dialogs::Portal::fd))
					.map(|fd| rustix::event::PollFd::from_borrowed_fd(fd, rustix::event::PollFlags::IN))
					.collect::<smallvec::SmallVec<[_; 4]>>();
				// An interrupted wait is an early return, which the caller treats like any wake.
				let _ = rustix::event::poll(&mut fds, timeout.as_ref());
				// Reset the counter so the next wait sleeps again; an empty counter reports `WouldBlock`.
				let _ = rustix::io::read(&*self.wake, &mut [0u8; 8]);
			}
			match guard.read() {
				Ok(_) => {}
				// The socket is non-blocking, so an empty socket only means there is nothing new.
				Err(wayland_client::backend::WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
				Err(error) => panic!(
					"Failed to read Wayland events: {error}. The most likely cause is that the compositor connection was lost."
				),
			}
		}

		// The read guard is gone, so libdecor may prepare a read of its own queue.
		self.dispatch_libdecor(0);
		self.event_queue
			.dispatch_pending(&mut self.data)
			.expect("Failed to dispatch Wayland events. The most likely cause is a protocol error reported by the compositor.");
		self.drain_portal();

		std::iter::from_fn(move || self.data.events.pop_front())
	}

	pub(crate) fn waker(&self) -> AppWaker {
		AppWaker(Arc::clone(&self.wake))
	}

	/// Sends a dialog to the desktop portal; see [`crate::window::App::open_dialog`].
	///
	/// Portal dialogs are not attached to `parent` yet, so they open on their own.
	pub(crate) fn open_dialog(
		&mut self,
		id: crate::window::DialogId,
		dialog: &crate::window::Dialog,
		_parent: Option<&Window>,
	) -> Result<(), crate::window::DialogError> {
		// Message boxes have no portal, so they never need the connection.
		if matches!(dialog, crate::window::Dialog::Message { .. }) {
			return Err(crate::window::DialogError::Unsupported);
		}
		let portal = match &mut self.portal {
			Some(portal) => portal,
			portal => portal.insert(dialogs::Portal::connect()?),
		};
		portal.open(id, dialog)
	}

	/// Returns the libdecor context, loading it the first time a window needs it.
	///
	/// A system without libdecor gets one warning, and its decorated windows open undecorated.
	fn libdecor(&self) -> Option<&libdecor::Libdecor> {
		self.libdecor
			.get_or_init(|| match libdecor::Libdecor::new(self.connection.display().id().as_ptr().cast()) {
				Ok(libdecor) => Some(libdecor),
				Err(error) => {
					log::warn!(
						"Windows open without decorations, because the compositor draws none and libdecor could not be loaded: {error}. The most likely cause is that libdecor isn't installed. Install the libdecor-0-0 package and its GTK plugin, libdecor-0-plugin-1-gtk."
					);
					None
				}
			})
			.as_ref()
	}

	/// Runs libdecor's queue, waiting as [`libdecor::Libdecor::dispatch`] describes, and applies what its frames
	/// reported to their windows.
	fn dispatch_libdecor(&mut self, timeout_ms: i32) {
		let Some(Some(libdecor)) = self.libdecor.get() else {
			return;
		};
		for (id, event) in libdecor.dispatch(timeout_ms) {
			match event {
				libdecor::FrameEvent::Configure { width, height } => {
					if let Some(window) = self.data.window_mut(id) {
						window.configured = true;
					}
					self.data.resize_toplevel(id, width, height);
				}
				libdecor::FrameEvent::Close => self.data.push(id, Events::Close),
			}
		}
	}

	/// Moves the results the portal delivered into the event queue.
	fn drain_portal(&mut self) {
		if let Some(portal) = &self.portal {
			self.data.events.extend(
				portal
					.drain()
					.map(|(id, result)| Event::App(AppEvents::Dialog { id, result })),
			);
		}
	}

	/// Makes a waiting poll also return when the HID monitor's descriptor becomes readable.
	pub(crate) fn wake_on_hid_changes(&mut self, monitor: &crate::hid::Monitor) -> Result<(), String> {
		let fd = monitor.os.fd().try_clone_to_owned().map_err(|error| {
			format!(
				"Failed to watch HID device changes: {error}. The most likely cause is that the process ran out of file descriptors."
			)
		})?;
		self.watched.push(fd);
		Ok(())
	}
}

impl Drop for App {
	fn drop(&mut self) {
		self.data.xdg_wm_base.destroy();
	}
}

impl Window {
	pub(crate) fn id(&self) -> WindowId {
		self.id
	}

	pub(crate) fn handles(&self) -> Handles {
		Handles {
			display: self.display.id().as_ptr() as _,
			surface: self.surface.id().as_ptr() as _,
			extent: self.extent.clone(),
		}
	}

	pub(crate) fn refresh_interval(&self) -> Option<Duration> {
		match self.refresh.load(Ordering::Relaxed) {
			0 => None,
			nanoseconds => Some(Duration::from_nanos(nanoseconds)),
		}
	}

	/// Wayland does not tell clients whether their surface can be seen, so every window counts as visible.
	pub(crate) fn is_visible(&self) -> bool {
		true
	}
}

impl Drop for Window {
	fn drop(&mut self) {
		// Roles go before their surface. A libdecor frame destroys its own objects when it drops.
		if let Some(Role::Xdg {
			xdg_surface,
			toplevel,
			decoration,
		}) = &self.role
		{
			if let Some(decoration) = decoration {
				decoration.destroy();
			}
			toplevel.destroy();
			xdg_surface.destroy();
		}
		self.role = None;
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

	/// Gives each window that captures the cursor a persistent pointer lock, once the seat has a pointer. The
	/// compositor engages a lock while its window has focus and the pointer is over it, and releases it otherwise.
	pub(super) fn lock_cursors(&mut self, qh: &wayland_client::QueueHandle<AppData>) {
		let (Some(constraints), Some(pointer)) = (&self.zwp_pointer_constraints, &self.pointer) else {
			return;
		};
		for window in self
			.windows
			.iter_mut()
			.filter(|window| window.captures_cursor && window.cursor_lock.is_none())
		{
			window.cursor_lock = Some(constraints.lock_pointer(
				&window.surface,
				pointer,
				None,
				zwp_pointer_constraints_v1::Lifetime::Persistent,
				qh,
				(),
			));
		}
	}

	/// Applies a toplevel configuration of `width` by `height` surface units to a window, and reports its extent in
	/// pixels when that changed. A zero size leaves the size to the client, so the window keeps the one it had.
	pub(super) fn resize_toplevel(&mut self, id: WindowId, width: i32, height: i32) {
		let Some(window) = self.window_mut(id) else {
			return;
		};
		if width > 0 && height > 0 {
			window.size = (width, height);
		}
		let scale = window.scale as i32;
		let extent = Extent::rectangle((window.size.0 * scale) as u32, (window.size.1 * scale) as u32);
		if window.extent == Some(extent) {
			return;
		}
		window.extent = Some(extent);
		window.shared_extent.set(extent);
		self.push(
			id,
			Events::Resize {
				width: extent.width(),
				height: extent.height(),
			},
		);
	}

	pub(super) fn push(&mut self, window: WindowId, event: Events) {
		self.events.push_back(Event::Window { window, event });
	}

	/// Recomputes a window's refresh interval from the outputs it is on and reports a change.
	pub(super) fn update_window_refresh(&mut self, id: WindowId) {
		let Some(window) = self.window(id) else {
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
			self.push(
				id,
				Events::DisplayChanged {
					refresh_interval: interval,
				},
			);
		}
	}

	/// Drops the state of windows whose [`Window`] handle destroyed their surface, along with any focus on them.
	fn forget_destroyed_windows(&mut self) {
		self.windows.retain(|window| {
			let alive = window.surface.is_alive();
			if !alive && let Some(lock) = &window.cursor_lock {
				lock.destroy();
			}
			alive
		});

		if self.pointer_focus.as_ref().is_some_and(|(_, id)| self.window(*id).is_none()) {
			self.pointer_focus = None;
		}
		if self.keyboard_focus.as_ref().is_some_and(|(_, id)| self.window(*id).is_none()) {
			self.keyboard_focus = None;
		}
	}
}
