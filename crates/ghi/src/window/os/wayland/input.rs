use super::*;

impl wayland_client::Dispatch<wl_seat::WlSeat, ()> for AppData {
	fn event(
		this: &mut Self,
		s: &wl_seat::WlSeat,
		event: wl_seat::Event,
		_: &(),
		_: &wayland_client::Connection,
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_seat::Event::Capabilities { capabilities } => {
				let capabilities = capabilities.into_result().unwrap();

				// A seat reports its capabilities again whenever its devices change, so each device is taken once, and
				// let go when the seat loses it.
				let has_pointer = capabilities.contains(wl_seat::Capability::Pointer);
				if has_pointer && this.pointer.is_none() {
					let pointer = s.get_pointer(qh, ());

					this.zwp_relative_pointer_manager.get_relative_pointer(&pointer, qh, ());
					this.cursor_shape = this
						.cursor_shape_manager
						.as_ref()
						.map(|manager| manager.get_pointer(&pointer, qh, ()));
					this.pointer = Some(pointer);
					this.lock_cursors(qh);
				} else if !has_pointer && let Some(pointer) = this.pointer.take() {
					if let Some(shape) = this.cursor_shape.take() {
						shape.destroy();
					}
					this.pointer_focus = None;
					this.buttons_down = 0;
					pointer.release();
				}

				let has_keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
				if has_keyboard && this.keyboard.is_none() {
					this.keyboard = Some(s.get_keyboard(qh, ()));
				} else if !has_keyboard && let Some(keyboard) = this.keyboard.take() {
					this.held_key = None;
					if let Some((_, id)) = this.keyboard_focus.take() {
						this.push(id, Events::FocusChanged(false));
					}
					keyboard.release();
				}
			}
			wl_seat::Event::Name { .. } => {}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_pointer::WlPointer, ()> for AppData {
	fn event(
		this: &mut Self,
		pointer: &wl_pointer::WlPointer,
		event: wl_pointer::Event,
		_: &(),
		_: &wayland_client::Connection,
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_pointer::Event::Enter { serial, surface, .. } => {
				if let Some(&id) = surface.data::<WindowId>() {
					this.pointer_focus = Some((pointer.clone(), id, serial));
					// Wayland leaves the cursor image undefined on entry, so every entry sets one.
					this.apply_cursor();
				}
			}
			wl_pointer::Event::Leave { .. } => {
				if this.pointer_focus.as_ref().is_some_and(|(focused, ..)| focused == pointer) {
					this.pointer_focus = None;
				}
			}
			wl_pointer::Event::Button { button, state, .. } => {
				let Some(id) = this.pointer_window() else {
					return;
				};

				let pressed = state.into_result().unwrap() == wl_pointer::ButtonState::Pressed;
				// Every button counts toward a waiting cursor lock, even one the engine doesn't name.
				if pressed {
					this.buttons_down += 1;
				} else {
					this.buttons_down = this.buttons_down.saturating_sub(1);
					if this.buttons_down == 0 {
						this.lock_cursors(qh);
					}
				}

				// Linux input event codes: `BTN_LEFT`, `BTN_RIGHT`, and `BTN_MIDDLE`.
				let button = match button {
					0x110 => MouseKeys::Left,
					0x111 => MouseKeys::Right,
					0x112 => MouseKeys::Middle,
					_ => return,
				};

				// Wayland reports buttons once, so both devices share them.
				for device in [PointingDevice::Pointer, PointingDevice::Mouse] {
					this.push(
						id,
						Events::Button {
							seat: Seat::stub(),
							device,
							pressed,
							button,
						},
					);
				}
			}
			wl_pointer::Event::Axis { time, axis, value } => {
				let Some(axis) = scroll_axis(axis) else {
					return;
				};
				this.scroll.distance[axis] += value;
				this.scroll.time = time;
				// Before version 5, a pointer sends no frames, so each axis event is a whole scroll.
				if pointer.version() < 5 {
					this.push_scroll();
				}
			}
			wl_pointer::Event::AxisValue120 { axis, value120 } => {
				if let Some(axis) = scroll_axis(axis) {
					*this.scroll.notches[axis].get_or_insert(0) += value120;
				}
			}
			// Versions 5 to 7 report whole notches; version 8 reports them in 120ths instead.
			wl_pointer::Event::AxisDiscrete { axis, discrete } => {
				if let Some(axis) = scroll_axis(axis) {
					*this.scroll.notches[axis].get_or_insert(0) += discrete * 120;
				}
			}
			wl_pointer::Event::Frame => this.push_scroll(),
			wl_pointer::Event::Motion {
				time,
				surface_x,
				surface_y,
			} => {
				let Some(id) = this.pointer_window() else {
					return;
				};
				let Some(window) = this.window(id) else {
					return;
				};

				if let Some(extent) = window.extent {
					let x = surface_x as f32 * window.scale as f32;
					let y = surface_y as f32 * window.scale as f32;

					let width = extent.width() as f32;
					let height = extent.height() as f32;

					let half_width = width / 2.0;
					let half_height = height / 2.0;

					let x = (x - half_width) / half_width;
					let y = (half_height - y) / half_height;

					this.push(
						id,
						Events::PointerPosition {
							seat: Seat::stub(),
							x,
							y,
							time: time as u64,
						},
					);
				}
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_keyboard::WlKeyboard, ()> for AppData {
	fn event(
		this: &mut Self,
		keyboard: &wl_keyboard::WlKeyboard,
		event: wl_keyboard::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_keyboard::Event::Key { key: code, state, .. } => {
				let state = state.into_result().unwrap();
				// XKB keycodes are evdev codes offset by 8.
				let keycode = xkb::Keycode::new(code + 8);
				let key = evdev_to_key(code);
				// A compositor on `wl_keyboard` version 10 repeats held keys itself, so the client stops making repeats.
				if state == wl_keyboard::KeyState::Repeated {
					this.compositor_repeats = true;
					this.held_key = None;
					this.repeat_key(keycode, key);
					return;
				}
				let pressed = state == wl_keyboard::KeyState::Pressed;

				let Some(keyboard_state) = this.keyboard_state.as_mut() else {
					return;
				};

				let direction = if pressed {
					xkb::KeyDirection::Down
				} else {
					xkb::KeyDirection::Up
				};

				// The text a press types depends on the modifiers before this key changes them, so read it first. Wayland
				// leaves text to the client, unlike AppKit and Win32, which report it themselves.
				let typed = if pressed { keyboard_state.typed(keycode) } else { Typed::Nothing };
				let character = match typed {
					Typed::Character(character) => Some(character),
					_ => None,
				};

				// XKB state tracks the physical keyboard, so it updates even without a focused window.
				keyboard_state.state.update_key(keycode, direction);

				let Some(&(_, id)) = this.keyboard_focus.as_ref() else {
					return;
				};
				// Only the last key pressed repeats, and only while it's held.
				if pressed {
					let repeats = !this.compositor_repeats && keyboard_state.state.get_keymap().key_repeats(keycode);
					this.held_key = repeats.then(|| HeldKey {
						keycode,
						key,
						character,
						window: id,
						pressed_at: std::time::Instant::now(),
						made: 0,
					});
				} else if this.held_key.as_ref().is_some_and(|held| held.keycode == keycode) {
					this.held_key = None;
				}
				if let Some(key) = key {
					this.push(
						id,
						Events::Key {
							seat: Seat::stub(),
							pressed,
							repeat: false,
							key,
						},
					);
				}
				// Keys such as Backspace and Enter type control characters; they arrive only as `Events::Key`.
				let text = match &typed {
					Typed::Text(text) => text.as_str(),
					_ => "",
				};
				for character in character.into_iter().chain(text.chars()) {
					this.push(
						id,
						Events::Character {
							seat: Seat::stub(),
							character,
						},
					);
				}
			}
			wl_keyboard::Event::Keymap { format, fd, size } => {
				let format = match format.into_result() {
					Ok(format) => format,
					Err(_) => return,
				};

				if format != wl_keyboard::KeymapFormat::XkbV1 {
					return;
				}

				let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

				let keymap = match unsafe {
					xkb::Keymap::new_from_fd(&context, fd, size as usize, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::COMPILE_NO_FLAGS)
				} {
					Ok(Some(keymap)) => keymap,
					Ok(None) => return,
					Err(_) => return,
				};

				// A new keymap keeps the compose table the last one read.
				let compose = match &this.keyboard_state {
					Some(previous) => previous.compose_table(),
					None => compose_table(&context),
				};
				this.keyboard_state = Some(KeyboardState::new(&keymap, compose.as_ref()));
			}
			wl_keyboard::Event::Enter { surface, .. } => {
				if let Some(&id) = surface.data::<WindowId>() {
					this.keyboard_focus = Some((keyboard.clone(), id));
					this.push(id, Events::FocusChanged(true));
				}
			}
			wl_keyboard::Event::Leave { .. } => {
				// A window without the keyboard hears no release, so a held key stops repeating here.
				this.held_key = None;
				if this.keyboard_focus.as_ref().is_some_and(|(focused, _)| focused == keyboard)
					&& let Some((_, id)) = this.keyboard_focus.take()
				{
					this.push(id, Events::FocusChanged(false));
				}
			}
			wl_keyboard::Event::RepeatInfo { rate, delay } => {
				// A negative value is a protocol error; treat it as no repeating.
				this.key_repeat = KeyRepeat {
					delay: Duration::from_millis(u64::try_from(delay).unwrap_or(0)),
					rate: u32::try_from(rate).unwrap_or(0),
				};
			}
			wl_keyboard::Event::Modifiers {
				mods_depressed,
				mods_latched,
				mods_locked,
				group,
				..
			} => {
				let Some(keyboard_state) = this.keyboard_state.as_mut() else {
					return;
				};

				keyboard_state
					.state
					.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<wl_output::WlOutput, ()> for AppData {
	fn event(
		this: &mut Self,
		output: &wl_output::WlOutput,
		event: wl_output::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_output::Event::Scale { factor } => {
				let factor = factor as u32;
				this.output_scale = this.output_scale.max(factor);
				// A surface from `wl_surface` version 6 on hears its own preferred scale instead; see the surface's
				// `PreferredBufferScale`.
				for window in this.windows.iter_mut().filter(|window| window.surface.version() < 6) {
					window.scale = window.scale.max(factor);
				}
			}
			wl_output::Event::Geometry { .. } => {}
			wl_output::Event::Mode {
				flags,
				width,
				height,
				refresh,
			} => {
				this.monitor_extent = Some(Extent::rectangle(width as _, height as _));
				// Outputs list every mode they support; only the current one sets the refresh rate.
				if let wayland_client::WEnum::Value(flags) = flags
					&& flags.contains(wl_output::Mode::Current)
				{
					match this.outputs.iter_mut().find(|(known, _)| known == output) {
						Some((_, millihertz)) => *millihertz = refresh,
						None => this.outputs.push((output.clone(), refresh)),
					}
				}
			}
			wl_output::Event::Description { .. } => {}
			wl_output::Event::Name { .. } => {}
			// Mode changes are atomic at `done`, so windows on this output report their new refresh rate here.
			wl_output::Event::Done => {
				let windows = this
					.windows
					.iter()
					.filter(|window| window.outputs.contains(output))
					.map(|window| window.id)
					.collect::<Vec<_>>();
				for id in windows {
					this.update_window_refresh(id);
				}
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
		event: zwp_relative_pointer_manager_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for AppData {
	fn event(
		this: &mut Self,
		_: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
		event: zwp_relative_pointer_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			zwp_relative_pointer_v1::Event::RelativeMotion {
				utime_lo,
				utime_hi,
				dx,
				dy,
				dx_unaccel,
				dy_unaccel,
			} => {
				let Some(id) = this.pointer_window() else {
					return;
				};

				// One report carries both the cursor's accelerated movement and the mouse's unaccelerated one.
				let time = ((utime_hi as u64) << 32 | utime_lo as u64) / 1000;
				for (device, dx, dy) in [
					(PointingDevice::Pointer, dx, dy),
					(PointingDevice::Mouse, dx_unaccel, dy_unaccel),
				] {
					this.push(
						id,
						Events::MouseMove {
							seat: Seat::stub(),
							device,
							dx: dx as f32,
							// Surface coordinates grow downward.
							dy: -dy as f32,
							time,
						},
					);
				}
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
		_: zwp_pointer_constraints_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
	}
}

impl wayland_client::Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for AppData {
	/// The compositor engages and releases a persistent lock on its own, so its reports need no handling.
	fn event(
		_: &mut Self,
		_: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
		_: zwp_locked_pointer_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
	}
}

// The cursor shape objects send no events.
wayland_client::delegate_noop!(AppData: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(AppData: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

/// Returns the index of a scroll axis in [`PendingScroll`], horizontal first, or `None` for an unknown axis.
fn scroll_axis(axis: wayland_client::WEnum<wl_pointer::Axis>) -> Option<usize> {
	match axis.into_result() {
		Ok(wl_pointer::Axis::HorizontalScroll) => Some(0),
		Ok(wl_pointer::Axis::VerticalScroll) => Some(1),
		_ => None,
	}
}

impl AppData {
	/// Sets the cursor image of the window under the pointer: none while the window captures the cursor, and the
	/// window's shape otherwise. Without the cursor shape protocol, the compositor keeps its own image.
	pub(super) fn apply_cursor(&self) {
		let Some(&(ref pointer, id, serial)) = self.pointer_focus.as_ref() else {
			return;
		};
		let Some(window) = self.window(id) else {
			return;
		};
		if window.captures_cursor {
			// A null cursor surface hides the cursor.
			pointer.set_cursor(serial, None, 0, 0);
		} else if let Some(shape) = &self.cursor_shape {
			shape.set_shape(serial, cursor_shape(window.pointer));
		}
	}

	/// Pushes the scroll the current pointer frame gathered to the window under the pointer, then clears it.
	fn push_scroll(&mut self) {
		let scroll = std::mem::take(&mut self.scroll);
		let Some(id) = self.pointer_window() else {
			return;
		};
		// A wheel scrolls one unit per notch; anything else scrolls its distance in surface units.
		let [dx, dy] = [0, 1].map(|axis| match scroll.notches[axis] {
			Some(notches) => notches as f32 / 120.0,
			None => scroll.distance[axis] as f32,
		});
		if dx == 0.0 && dy == 0.0 {
			return;
		}
		for device in [PointingDevice::Pointer, PointingDevice::Mouse] {
			self.push(
				id,
				Events::Scroll {
					seat: Seat::stub(),
					device,
					dx,
					// Scroll grows upward, but Wayland's axes grow downward.
					dy: -dy,
					time: u64::from(scroll.time),
				},
			);
		}
	}

	/// Returns the window under the pointer, which receives pointer input.
	fn pointer_window(&self) -> Option<WindowId> {
		self.pointer_focus.as_ref().map(|&(_, id, _)| id)
	}

	/// Pushes one repeat of a key, and the character it types, to the window that has the keyboard: a repeat the
	/// compositor sent. It reads the character with the modifiers held now.
	fn repeat_key(&mut self, keycode: xkb::Keycode, key: Option<Keys>) {
		let (Some(keyboard_state), Some(&(_, id))) = (self.keyboard_state.as_ref(), self.keyboard_focus.as_ref()) else {
			return;
		};
		let character = keyboard_state.character(keycode);
		self.push_repeat(id, key, character);
	}

	/// Pushes a repeat of `key` and the `character` it types to `window`.
	fn push_repeat(&mut self, window: WindowId, key: Option<Keys>, character: Option<char>) {
		if let Some(key) = key {
			self.push(
				window,
				Events::Key {
					seat: Seat::stub(),
					pressed: true,
					repeat: true,
					key,
				},
			);
		}
		if let Some(character) = character {
			self.push(
				window,
				Events::Character {
					seat: Seat::stub(),
					character,
				},
			);
		}
	}

	/// How long from `now` until the held key's next repeat is due, or `None` when no key repeats.
	pub(super) fn next_key_repeat(&self, now: std::time::Instant) -> Option<Duration> {
		let held = self.held_key.as_ref()?;
		let at = held.pressed_at + self.key_repeat.next(held.made)?;
		Some(at.saturating_duration_since(now))
	}

	/// Pushes every repeat the held key has made by `now` and not pushed yet.
	pub(super) fn repeat_held_key(&mut self, now: std::time::Instant) {
		let Some(held) = self.held_key.as_mut() else {
			return;
		};
		let due = self.key_repeat.due(now.saturating_duration_since(held.pressed_at));
		let missing = due.saturating_sub(held.made);
		held.made = due.max(held.made);
		let (window, key, character) = (held.window, held.key, held.character);
		for _ in 0..missing {
			self.push_repeat(window, key, character);
		}
	}
}

/// Returns the cursor shape protocol's name for `pointer`.
fn cursor_shape(pointer: Pointer) -> wp_cursor_shape_device_v1::Shape {
	use wp_cursor_shape_device_v1::Shape;
	match pointer {
		Pointer::Default => Shape::Default,
		Pointer::Hand => Shape::Pointer,
		Pointer::Text => Shape::Text,
		Pointer::Grab => Shape::Grab,
		Pointer::Grabbing => Shape::Grabbing,
		Pointer::Crosshair => Shape::Crosshair,
		Pointer::NotAllowed => Shape::NotAllowed,
		Pointer::ResizeColumn => Shape::ColResize,
		Pointer::ResizeRow => Shape::RowResize,
	}
}
