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

				if capabilities.contains(wl_seat::Capability::Pointer) {
					let pointer = s.get_pointer(qh, ());

					this.zwp_relative_pointer_manager.get_relative_pointer(&pointer, qh, ());
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
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_pointer::Event::Enter { .. } => {
				this.state.focused_pointer = Some(pointer.clone());
				this.state.pointer_is_hidden = false;

				if this.state.should_hide_pointer
					&& !this.requests.iter().any(|request| matches!(request, Requests::HidePointer))
				{
					this.requests.push_back(Requests::HidePointer);
				}

				if this.state.should_lock_pointer
					&& !this.requests.iter().any(|request| matches!(request, Requests::LockPointer))
				{
					this.requests.push_back(Requests::LockPointer);
				}

				this.process_requests(qh);
			}
			wl_pointer::Event::Leave { .. } => {
				if let Some(focused_pointer) = &this.state.focused_pointer {
					if focused_pointer == pointer {
						this.state.focused_pointer = None;
						this.state.pointer_is_confined = false;
						this.state.pointer_is_hidden = false;
						this.state.pointer_is_locked = false;
					}
				}

				this.process_requests(qh);
			}
			wl_pointer::Event::Button { button, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_pointer::ButtonState::Pressed;

				let button = match button {
					272 => MouseKeys::Left,
					2 => MouseKeys::Middle,
					273 => MouseKeys::Right,
					4 => MouseKeys::ScrollUp,
					5 => MouseKeys::ScrollDown,
					_ => return,
				};

				this.events.push_back(Events::Button {
					seat: Seat::stub(),
					pressed,
					button,
				});
			}
			wl_pointer::Event::Axis { axis, value, .. } => {
				let _ = match axis.into_result().unwrap() {
					wl_pointer::Axis::VerticalScroll => MouseKeys::ScrollUp,
					wl_pointer::Axis::HorizontalScroll => MouseKeys::ScrollDown,
					_ => return,
				};

				let _ = value > 0.0;
			}
			wl_pointer::Event::Motion {
				time,
				surface_x,
				surface_y,
			} => {
				if let Some(extent) = this.state.extent {
					let x = surface_x as f32 * this.state.scale as f32;
					let y = surface_y as f32 * this.state.scale as f32;

					let width = extent.width() as f32;
					let height = extent.height() as f32;

					let half_width = width / 2.0;
					let half_height = height / 2.0;

					let x = (x - half_width) / half_width;
					let y = (half_height - y) / half_height;

					this.events.push_back(Events::MousePosition {
						seat: Seat::stub(),
						x,
						y,
						time: time as u64,
					});
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
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_keyboard::Event::Key { key, state, .. } => {
				let pressed = state.into_result().unwrap() == wl_keyboard::KeyState::Pressed;
				let keycode = xkb::Keycode::new(key + 8);

				let keyboard_state = match this.state.keyboard_state.as_ref() {
					Some(keyboard_state) => keyboard_state,
					None => return,
				};

				let mut keyboard_state = keyboard_state.borrow_mut();
				let direction = if pressed {
					xkb::KeyDirection::Down
				} else {
					xkb::KeyDirection::Up
				};

				keyboard_state.state.update_key(keycode, direction);

				if let Some(key) = keysym_to_key(keyboard_state.state.key_get_one_sym(keycode)) {
					this.events.push_back(Events::Key {
						seat: Seat::stub(),
						pressed,
						key,
					});
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

				this.state.keyboard_state = Some(Rc::new(RefCell::new(KeyboardState { context, keymap, state })));
			}
			wl_keyboard::Event::Enter { .. } => {
				this.state.focused_keyboard = Some(keyboard.clone());

				this.process_requests(qh);
			}
			wl_keyboard::Event::Leave { .. } => {
				if let Some(focused_keyboard) = &this.state.focused_keyboard {
					if focused_keyboard == keyboard {
						this.state.focused_keyboard = None;
						this.state.pointer_is_confined = false;
						this.state.pointer_is_locked = false;
					}
				}

				this.process_requests(qh);
			}
			wl_keyboard::Event::Modifiers {
				mods_depressed,
				mods_latched,
				mods_locked,
				group,
				..
			} => {
				let keyboard_state = match this.state.keyboard_state.as_ref() {
					Some(keyboard_state) => keyboard_state,
					None => return,
				};

				let mut keyboard_state = keyboard_state.borrow_mut();
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
		_: &wl_output::WlOutput,
		event: wl_output::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			wl_output::Event::Scale { factor } => {
				this.state.scale = this.state.scale.max(factor as _);
			}
			wl_output::Event::Geometry { .. } => {}
			wl_output::Event::Mode { width, height, .. } => {
				this.state.monitor_extent = Some(Extent::rectangle(width as _, height as _));
			}
			wl_output::Event::Description { .. } => {}
			wl_output::Event::Name { .. } => {}
			wl_output::Event::Done => {}
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
				dx_unaccel,
				dy_unaccel,
				..
			} => {
				this.events.push_back(Events::MouseMove {
					seat: Seat::stub(),
					dx: dx_unaccel as f32,
					dy: dy_unaccel as f32,
					time: (utime_hi as u64) << 32 | utime_lo as u64,
				});
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, ()> for AppData {
	fn event(
		_: &mut Self,
		_: &zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
		event: zwp_pointer_constraints_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		_: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_confined_pointer_v1::ZwpConfinedPointerV1, ()> for AppData {
	fn event(
		this: &mut Self,
		confined_pointer: &zwp_confined_pointer_v1::ZwpConfinedPointerV1,
		event: zwp_confined_pointer_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			zwp_confined_pointer_v1::Event::Confined => {
				this.state.confined_pointer = Some(confined_pointer.clone());
				this.state.pointer_is_confined = true;
				println!("Pointer is confined");
			}
			zwp_confined_pointer_v1::Event::Unconfined => {
				this.state.pointer_is_confined = false;

				if this.state.confined_pointer.as_ref().is_some_and(|p| p == confined_pointer) {
					if let Some(confined_pointer) = this.state.confined_pointer.take() {
						confined_pointer.destroy();
					}
				} else {
					confined_pointer.destroy();
				}

				if this.state.should_confine_pointer
					&& !this
						.requests
						.iter()
						.any(|request| matches!(request, Requests::ConstrainPointer))
				{
					this.requests.push_back(Requests::ConstrainPointer);
					this.process_requests(qh);
				}

				println!("Pointer is unconfined");
			}
			_ => {}
		}
	}
}

impl wayland_client::Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for AppData {
	fn event(
		this: &mut Self,
		locked_pointer: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
		event: zwp_locked_pointer_v1::Event,
		_: &(),
		_: &wayland_client::Connection,
		qh: &wayland_client::QueueHandle<AppData>,
	) {
		match event {
			zwp_locked_pointer_v1::Event::Locked => {
				this.state.locked_pointer = Some(locked_pointer.clone());
				this.state.pointer_is_locked = true;
				println!("Pointer is locked");
			}
			zwp_locked_pointer_v1::Event::Unlocked => {
				this.state.pointer_is_locked = false;

				if this.state.locked_pointer.as_ref().is_some_and(|p| p == locked_pointer) {
					if let Some(locked_pointer) = this.state.locked_pointer.take() {
						locked_pointer.destroy();
					}
				} else {
					locked_pointer.destroy();
				}

				if this.state.should_lock_pointer
					&& !this.requests.iter().any(|request| matches!(request, Requests::LockPointer))
				{
					this.requests.push_back(Requests::LockPointer);
					this.process_requests(qh);
				}

				println!("Pointer is unlocked");
			}
			_ => {}
		}
	}
}
