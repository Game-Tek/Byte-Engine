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

				if capabilities.contains(wl_seat::Capability::Pointer) && this.pointer.is_none() {
					let pointer = s.get_pointer(qh, ());

					this.zwp_relative_pointer_manager.get_relative_pointer(&pointer, qh, ());
					this.pointer = Some(pointer);
					this.lock_cursors(qh);
				}

				if capabilities.contains(wl_seat::Capability::Keyboard) {
					let _ = s.get_keyboard(qh, ());
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
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_pointer::Event::Enter { serial, surface, .. } => {
				if let Some(&id) = surface.data::<WindowId>() {
					this.pointer_focus = Some((pointer.clone(), id));
					// A null cursor surface hides the cursor while it's over a capturing window.
					if this.window(id).is_some_and(|window| window.captures_cursor) {
						pointer.set_cursor(serial, None, 0, 0);
					}
				}
			}
			wl_pointer::Event::Leave { .. } => {
				if this.pointer_focus.as_ref().is_some_and(|(focused, _)| focused == pointer) {
					this.pointer_focus = None;
				}
			}
			wl_pointer::Event::Button { button, state, .. } => {
				let Some(id) = this.pointer_window() else {
					return;
				};

				let pressed = state.into_result().unwrap() == wl_pointer::ButtonState::Pressed;

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
				let Some(id) = this.pointer_window() else {
					return;
				};
				// Axis values grow downward and to the right, in surface units.
				let (dx, dy) = match axis.into_result() {
					Ok(wl_pointer::Axis::VerticalScroll) => (0.0, -value as f32),
					Ok(wl_pointer::Axis::HorizontalScroll) => (value as f32, 0.0),
					_ => return,
				};
				for device in [PointingDevice::Pointer, PointingDevice::Mouse] {
					this.push(
						id,
						Events::Scroll {
							seat: Seat::stub(),
							device,
							dx,
							dy,
							time: time as u64,
						},
					);
				}
			}
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
			wl_keyboard::Event::Key { key, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_keyboard::KeyState::Pressed;
				let keycode = xkb::Keycode::new(key + 8);

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
				let character = pressed
					.then(|| char::from_u32(keyboard_state.state.key_get_utf32(keycode)))
					.flatten()
					.filter(|character| *character != '\0' && !character.is_control());

				// XKB state tracks the physical keyboard, so it updates even without a focused window.
				keyboard_state.state.update_key(keycode, direction);

				let key = keysym_to_key(keyboard_state.state.key_get_one_sym(keycode));

				let Some(&(_, id)) = this.keyboard_focus.as_ref() else {
					return;
				};
				if let Some(key) = key {
					this.push(
						id,
						Events::Key {
							seat: Seat::stub(),
							pressed,
							key,
						},
					);
				}
				// Keys such as Backspace and Enter type control characters; they arrive only as `Events::Key`.
				if let Some(character) = character {
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

				let state = xkb::State::new(&keymap);

				this.keyboard_state = Some(KeyboardState { state });
			}
			wl_keyboard::Event::Enter { surface, .. } => {
				if let Some(&id) = surface.data::<WindowId>() {
					this.keyboard_focus = Some((keyboard.clone(), id));
					this.push(id, Events::FocusChanged(true));
				}
			}
			wl_keyboard::Event::Leave { .. } => {
				if this.keyboard_focus.as_ref().is_some_and(|(focused, _)| focused == keyboard)
					&& let Some((_, id)) = this.keyboard_focus.take()
				{
					this.push(id, Events::FocusChanged(false));
				}
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
				for window in &mut this.windows {
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

impl AppData {
	/// Returns the window under the pointer, which receives pointer input.
	fn pointer_window(&self) -> Option<WindowId> {
		self.pointer_focus.as_ref().map(|(_, id)| *id)
	}
}
