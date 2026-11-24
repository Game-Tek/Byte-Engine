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
		let (
			compositor,
			xdg_wm_base,
			zwp_relative_pointer_manager,
			zwp_pointer_constraints,
			decoration_manager,
			cursor_shape_manager,
		) = {
			let mut configuration = Configuration {
				compositor: None,
				xdg_wm_base: None,
				wl_seat: None,
				wl_output: None,

				wl_callback: None,

				zwp_relative_pointer_manager: None,
				zwp_pointer_constraints: None,
				decoration_manager: None,
				cursor_shape_manager: None,

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
					configuration.cursor_shape_manager,
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
				cursor_shape_manager,
				windows: Vec::with_capacity(4),
				output_scale: 1,
				monitor_extent: None,
				outputs: Vec::new(),
				pointer: None,
				cursor_shape: None,
				pointer_focus: None,
				buttons_down: 0,
				scroll: PendingScroll::default(),
				keyboard: None,
				keyboard_focus: None,
				keyboard_state: None,
				key_repeat: KeyRepeat::OFF,
				held_key: None,
				compositor_repeats: false,
				events: VecDeque::with_capacity(64),
			},
			id_name: id_name.to_owned(),
			next_window: 1,
			windows: Vec::new(),
			wake: Arc::new(wake),
			watched: Vec::new(),
			portal: None,
			preferences: preferences::PreferenceWatch::connect(),
			libdecor: LibdecorState::Unloaded,
		};
		// The first poll reports the preferences as they stand at launch, all unknown without the portal.
		let first = app.preferences.as_ref().map_or(
			Event::App(AppEvents::Preferences(crate::window::Preferences::default())),
			preferences::PreferenceWatch::report,
		);
		app.data.events.push_back(first);

		// Receive seat and output state before the first window picks its scale.
		app.event_queue
			.roundtrip(&mut app.data)
			.map_err(|e| format!("Failed to initialize Wayland app event queue: {}", e))?;

		Ok(app)
	}

	/// Creates and shows a native window; see [`crate::window::App::create_window`].
	pub(crate) fn create_window(&mut self, name: &str, extent: Extent, features: Features) -> Result<WindowId, String> {
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
		let libdecor = if !background && decorated && self.data.decoration_manager.is_none() {
			ensure_libdecor(&mut self.libdecor, &self.connection)
		} else {
			None
		};
		let role = match libdecor {
			Some(libdecor) => {
				let frame = libdecor.decorate(&surface, name, &self.id_name, (extent.width() as i32, extent.height() as i32));
				match frame {
					Ok(frame) => Some(Role::Libdecor { frame }),
					Err(error) => {
						surface.destroy();
						return Err(error);
					}
				}
			}
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
		self.data.windows.push(WindowState {
			id,
			surface: surface.clone(),
			scale,
			extent: None,
			size: (extent.width() as i32, extent.height() as i32),
			// The compositor configures only surfaces with a role.
			configured: background,
			outputs: Vec::new(),
			refresh: None,
			captures_cursor: features.contains(Features::CAPTURE_CURSOR) && !features.contains(Features::BACKGROUND),
			pointer: Pointer::Default,
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
		let libdecorated = matches!(role, Some(Role::Libdecor { .. }));
		// The window is open from here on, so libdecor's dispatch finds its frame, and a failed wait destroys it.
		self.windows.push(Window { id, surface, role });

		if let Err(error) = self.wait_for_configuration(id, libdecorated) {
			self.destroy_window(id);
			return Err(error);
		}
		Ok(id)
	}

	/// Dispatches until the compositor configured the window `id`. Other windows' events dispatched meanwhile stay
	/// queued for the next poll.
	fn wait_for_configuration(&mut self, id: WindowId, libdecorated: bool) -> Result<(), String> {
		while !self.data.window(id).is_some_and(|window| window.configured) {
			if libdecorated {
				// A libdecor frame is configured on the default queue, which only libdecor dispatches. Its wait reads
				// the socket for every queue, so the app's own events are dispatched after it.
				self.event_queue
					.flush()
					.map_err(|e| format!("Failed to flush Wayland requests while creating a window: {e}"))?;
				self.dispatch_libdecor(-1);
				self.event_queue
					.dispatch_pending(&mut self.data)
					.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {e}"))?;
			} else {
				self.event_queue
					.blocking_dispatch(&mut self.data)
					.map_err(|e| format!("Failed to wait for initial Wayland surface configuration: {e}"))?;
			}
		}
		Ok(())
	}

	/// Destroys the window's protocol objects and forgets it; see [`crate::window::App::destroy_window`].
	pub(crate) fn destroy_window(&mut self, id: WindowId) {
		let Some(index) = self.windows.iter().position(|window| window.id == id) else {
			return;
		};
		let window = self.windows.remove(index);
		self.data.forget_window(id);
		// Roles go before their surface.
		match window.role {
			Some(Role::Xdg {
				xdg_surface,
				toplevel,
				decoration,
			}) => {
				if let Some(decoration) = decoration {
					decoration.destroy();
				}
				toplevel.destroy();
				xdg_surface.destroy();
			}
			Some(Role::Libdecor { frame }) => {
				// A frame only exists once libdecor loaded.
				if let LibdecorState::Loaded(libdecor) = &self.libdecor {
					libdecor.destroy_frame(frame);
				}
			}
			None => {}
		}
		window.surface.destroy();
	}

	/// Pumps the native queue and yields its events; see [`crate::window::App::poll`].
	pub(crate) fn poll(&mut self, wait: Wait) -> impl Iterator<Item = Event> + '_ {
		self.event_queue
			.flush()
			.expect("Failed to flush Wayland requests. The most likely cause is that the compositor connection was lost.");

		// Portal replies read during an earlier request wait in the connection's buffer, where polling its descriptor
		// cannot see them. Likewise, libdecor events already read wait on the default queue, where the socket can't
		// signal them.
		self.drain_portal();
		self.drain_preferences();
		self.dispatch_libdecor(0);

		// A guard only exists when nothing is queued for dispatch, so waiting behind it cannot hide events.
		if let Some(guard) = self.event_queue.prepare_read() {
			let now = std::time::Instant::now();
			let timeout = match wait {
				Wait::Immediate => Some(Duration::ZERO),
				Wait::Until(deadline) => Some(deadline.saturating_duration_since(now)),
				Wait::Forever => None,
			};
			// A held key wakes the wait when its next repeat is due.
			let timeout = match (timeout, self.data.next_key_repeat(now)) {
				(Some(timeout), Some(repeat)) => Some(timeout.min(repeat)),
				(timeout, repeat) => timeout.or(repeat),
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
					.chain(self.preferences.as_ref().map(preferences::PreferenceWatch::fd))
					.map(|fd| rustix::event::PollFd::from_borrowed_fd(fd, rustix::event::PollFlags::IN))
					.collect::<smallvec::SmallVec<[_; 6]>>();
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
		self.drain_preferences();
		self.data.repeat_held_key(std::time::Instant::now());

		std::iter::from_fn(move || self.data.events.pop_front())
	}

	pub(crate) fn waker(&self) -> AppWaker {
		AppWaker(Arc::clone(&self.wake))
	}

	/// Refuses the clipboard, which this platform doesn't have yet; see [`crate::window::App::set_clipboard_text`].
	pub(crate) fn set_clipboard_text(&mut self, _text: &str) -> Result<(), String> {
		Err("The clipboard was not set. Byte Engine has a clipboard only on macOS so far. Copy the text by hand on this platform.".to_owned())
	}

	/// Sets whether `window` hides the cursor and locks the pointer while it has focus; see
	/// [`crate::window::App::set_cursor_capture`].
	///
	/// Like on macOS, the cursor hides at once when it's over the window, and shows again at the window's centre on
	/// release.
	pub(crate) fn set_cursor_capture(&mut self, window: WindowId, captures: bool) {
		let Some(state) = self.data.window_mut(window) else {
			return;
		};
		state.captures_cursor = captures;
		if !captures && let Some(lock) = state.cursor_lock.take() {
			// The hint is surface state, so it applies at the window's next commit, which its next frame makes.
			let (width, height) = state.size;
			lock.set_cursor_position_hint(f64::from(width) / 2.0, f64::from(height) / 2.0);
			lock.destroy();
		}
		let qh = self.event_queue.handle();
		self.data.lock_cursors(&qh);
		self.data.apply_cursor();
	}

	/// Records the shape the cursor takes over `window`, and shows it at once when the pointer is over it; see
	/// [`crate::window::App::set_pointer`].
	pub(crate) fn set_pointer(&mut self, window: WindowId, pointer: Pointer) {
		let Some(state) = self.data.window_mut(window) else {
			return;
		};
		state.pointer = pointer;
		self.data.apply_cursor();
	}

	/// Sends a dialog to the desktop portal; see [`crate::window::App::open_dialog`].
	///
	/// Portal dialogs are not attached to `parent` yet, so they open on their own.
	pub(crate) fn open_dialog(
		&mut self,
		id: crate::window::DialogId,
		dialog: &crate::window::Dialog,
		_parent: Option<WindowId>,
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

	/// Runs libdecor's queue, waiting as [`libdecor::Libdecor::dispatch`] describes, and applies what its frames
	/// reported to their windows.
	fn dispatch_libdecor(&mut self, timeout_ms: i32) {
		let LibdecorState::Loaded(libdecor) = &self.libdecor else {
			return;
		};
		libdecor.dispatch(timeout_ms);
		for window in &mut self.windows {
			let Some(Role::Libdecor { frame }) = &mut window.role else {
				continue;
			};
			let report = frame.take_report();
			if let Some((width, height)) = report.configure {
				if let Some(state) = self.data.window_mut(window.id) {
					state.configured = true;
				}
				self.data.resize_toplevel(window.id, width, height);
			}
			if report.close {
				self.data.push(window.id, Events::Close);
			}
		}
	}

	/// Moves the results the portal delivered into the event queue.
	fn drain_portal(&mut self) {
		if let Some(portal) = &mut self.portal {
			portal.drain(|id, result| self.data.events.push_back(Event::App(AppEvents::Dialog { id, result })));
		}
	}

	/// Moves a new preferences report into the event queue when the settings portal signalled a change.
	fn drain_preferences(&mut self) {
		if let Some(report) = self.preferences.as_ref().and_then(preferences::PreferenceWatch::drain) {
			self.data.events.push_back(report);
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

/// Returns the libdecor context, loading it into `state` the first time a window needs it.
///
/// It takes the app's fields rather than the app, so the caller can keep using the app's other fields while it holds
/// the context. A system without libdecor gets one warning, and its decorated windows open undecorated.
fn ensure_libdecor<'a>(
	state: &'a mut LibdecorState,
	connection: &wayland_client::Connection,
) -> Option<&'a libdecor::Libdecor> {
	if matches!(state, LibdecorState::Unloaded) {
		*state = match libdecor::Libdecor::new(connection.display().id().as_ptr().cast()) {
			Ok(loaded) => LibdecorState::Loaded(loaded),
			Err(error) => {
				log::warn!(
					"Windows open without decorations, because the compositor draws none and libdecor could not be loaded: {error}. The most likely cause is that libdecor isn't installed. Install the libdecor-0-0 package and its GTK plugin, libdecor-0-plugin-1-gtk."
				);
				LibdecorState::Unavailable
			}
		};
	}
	match state {
		LibdecorState::Loaded(loaded) => Some(loaded),
		LibdecorState::Unloaded | LibdecorState::Unavailable => None,
	}
}

impl Drop for App {
	fn drop(&mut self) {
		while let Some(id) = self.windows.last().map(|window| window.id) {
			self.destroy_window(id);
		}
		// Every frame is gone, so libdecor can go while the connection it uses is still open.
		self.libdecor = LibdecorState::Unloaded;
		self.data.xdg_wm_base.destroy();
	}
}

impl App {
	pub(crate) fn handles(&self, id: WindowId) -> Option<Handles> {
		let window = self.windows.iter().find(|window| window.id == id)?;
		Some(Handles {
			display: self.connection.display().id().as_ptr() as _,
			surface: window.surface.id().as_ptr() as _,
		})
	}

	pub(crate) fn extent(&self, id: WindowId) -> Option<Extent> {
		self.data.window(id)?.extent
	}

	pub(crate) fn refresh_interval(&self, id: WindowId) -> Option<Duration> {
		self.data.window(id)?.refresh
	}

	/// Wayland does not tell clients whether their surface can be seen, so every open window counts as visible.
	pub(crate) fn is_visible(&self, id: WindowId) -> bool {
		self.data.window(id).is_some()
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
		// A lock waits for every button to be released; the release that lets go of the last one locks.
		if self.buttons_down > 0 {
			return;
		}
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
		self.push(
			id,
			Events::Resize {
				width: extent.width(),
				height: extent.height(),
				scale: scale as f32,
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
		let Some(window) = self.window_mut(id) else {
			return;
		};
		if std::mem::replace(&mut window.refresh, interval) != interval {
			self.push(
				id,
				Events::DisplayChanged {
					refresh_interval: interval,
				},
			);
		}
	}

	/// Drops the state of a window [`App::destroy_window`] destroys, along with any focus on it.
	fn forget_window(&mut self, id: WindowId) {
		self.windows.retain(|window| {
			let destroyed = window.id == id;
			if destroyed && let Some(lock) = &window.cursor_lock {
				lock.destroy();
			}
			!destroyed
		});

		if self.pointer_focus.as_ref().is_some_and(|(_, focus, _)| *focus == id) {
			self.pointer_focus = None;
		}
		if self.keyboard_focus.as_ref().is_some_and(|(_, focus)| *focus == id) {
			self.keyboard_focus = None;
		}
		// A key held over the window would otherwise repeat into a window that is gone.
		if self.held_key.as_ref().is_some_and(|held| held.window == id) {
			self.held_key = None;
		}
	}
}
