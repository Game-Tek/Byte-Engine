//! Adapters between the graphics application and external event protocols.
//!
//! [`process_default_window_input`] records into the device classes installed by
//! [`super::setup_default_input`]. `setup_default_dmx` is optional and should
//! only be installed by applications that publish color values to Art-Net.

#[cfg(feature = "dmx")]
use std::{
	net::{Ipv4Addr, UdpSocket},
	time::Duration,
};

#[cfg(feature = "dmx")]
use artnet_protocol::{ArtCommand, ArtTalkToMe, Output, Poll};
#[cfg(feature = "dmx")]
use utils::RGBA;

#[cfg(feature = "dmx")]
use super::GraphicsApplication;
use crate::input;
#[cfg(feature = "dmx")]
use crate::{
	application::{Events, Parameter, parameters::Parameters as _, thread::Thread},
	core::listener::{DefaultListener, Listener as _},
};

/// Starts an Art-Net worker that publishes received [`RGBA`] values as DMX
/// output.
#[cfg(feature = "dmx")]
pub fn setup_default_dmx(application: &mut GraphicsApplication, mut receiver: DefaultListener<RGBA>) {
	let parameters = application.parameters().scope("artnet");
	let bind_address = parse_artnet_ipv4_parameter(parameters.get_parameter("bind-address"), Ipv4Addr::UNSPECIFIED);
	let poll_target = parse_artnet_ipv4_parameter(parameters.get_parameter("poll-target"), Ipv4Addr::BROADCAST);

	application
		.threads
		.push(Thread::new(application.application.events().listener(), move |mut events| {
			const ARTNET_PORT: u16 = 6454;

			let socket = UdpSocket::bind((bind_address, ARTNET_PORT)).unwrap();
			let target = (poll_target, ARTNET_PORT);
			socket.set_broadcast(true).unwrap();

			loop {
				if matches!(events.read(), Some(Events::Close)) {
					return;
				}

				let poll = ArtCommand::Poll(Poll {
					talk_to_me: ArtTalkToMe::EMIT_CHANGES,
					diagnostics_priority: 0,
					..Poll::default()
				})
				.write_to_buffer()
				.unwrap();
				socket.send_to(&poll, target).unwrap();
				socket.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

				while let Some(color) = receiver.read() {
					let to_u8 = |value: f32| (value * 255.0).clamp(0.0, 255.0) as u8;
					let data = [to_u8(color.r), to_u8(color.g), to_u8(color.b), 0, 0, 0, 0];
					let command = ArtCommand::Output(Output {
						data: data.to_vec().into(),
						port_address: 0.into(),
						..Output::default()
					});
					let bytes = match command.write_to_buffer() {
						Ok(bytes) => bytes,
						Err(error) => {
							log::warn!(
								"Failed to serialize an Art-Net output packet. The most likely cause is that the DMX payload or universe is invalid: {error}"
							);
							continue;
						}
					};

					if let Err(error) = socket.send_to(&bytes, target) {
						log::warn!(
							"Failed to send an Art-Net output packet. The most likely cause is that the node address is unreachable from this host: {error}"
						);
					}
				}
			}
		}));
}

#[cfg(feature = "dmx")]
fn parse_artnet_ipv4_parameter(parameter: Option<&Parameter>, default: Ipv4Addr) -> Ipv4Addr {
	let Some(parameter) = parameter else {
		return default;
	};

	parameter.value().parse::<Ipv4Addr>().unwrap_or_else(|error| {
		log::warn!(
			"Invalid Art-Net IPv4 address parameter `{}`. The most likely cause is that the configured value is not a valid IPv4 address: {error}",
			parameter.name()
		);
		default
	})
}

impl crate::core::message::Message for ghi::window::Event {}

/// Records a GHI window event into the standard mouse and keyboard devices.
///
/// Focus loss, minimizing, closing, and resizing interrupt the interaction, so
/// the seat's queued and retained records are discarded and the function
/// returns `true`. Cancel your sinks' held actions when it does.
pub fn process_default_window_input<A: std::alloc::Allocator + Clone>(
	collector: &mut input::InputCollector<A>,
	event: ghi::window::Events,
) -> bool {
	let seat = input::SeatHandle::stub();
	if let Some((device, trigger, value)) = translate(collector, event) {
		collector.record(seat, device, input::TriggerReference::Name(trigger), value);
		return false;
	}
	let interrupted = matches!(
		event,
		ghi::window::Events::Resize { .. }
			| ghi::window::Events::FocusChanged(false)
			| ghi::window::Events::Minimize
			| ghi::window::Events::Close
	);
	if interrupted {
		collector.reset_seat(seat);
	}
	interrupted
}

/// Names the device and control a window input event drives, with its value.
fn translate<A: std::alloc::Allocator + Clone>(
	collector: &input::InputCollector<A>,
	event: ghi::window::Events,
) -> Option<(input::DeviceHandle, &'static str, input::Value)> {
	let mouse = collector.devices_by_class_name("Mouse")?.next()?;
	let keyboard = collector.devices_by_class_name("Keyboard")?.next()?;

	let record = match event {
		ghi::window::Events::Button { pressed, button, .. } => {
			let trigger = match button {
				ghi::window::input::MouseKeys::Left => "Mouse.LeftButton",
				ghi::window::input::MouseKeys::Right => "Mouse.RightButton",
				ghi::window::input::MouseKeys::Middle => "Mouse.MiddleButton",
				ghi::window::input::MouseKeys::ScrollUp => return Some((mouse, "Mouse.Scroll", input::Value::Float(1.0))),
				ghi::window::input::MouseKeys::ScrollDown => return Some((mouse, "Mouse.Scroll", input::Value::Float(-1.0))),
			};
			(mouse, trigger, input::Value::Bool(pressed))
		}
		ghi::window::Events::MousePosition { x, y, .. } => {
			(mouse, "Mouse.Position", input::Value::Vector2(input::Axis2::new(x, y)))
		}
		ghi::window::Events::MouseMove { dx, dy, .. } => {
			(mouse, "Mouse.Movement", input::Value::Vector2(input::Axis2::new(dx, dy)))
		}
		ghi::window::Events::Scroll { dy, .. } => (mouse, "Mouse.Scroll", input::Value::Float(dy)),
		ghi::window::Events::Key { pressed, key, .. } => (keyboard, keyboard_trigger(key), input::Value::Bool(pressed)),
		ghi::window::Events::Character { character, .. } => (keyboard, "Keyboard.Character", input::Value::Unicode(character)),
		_ => return None,
	};

	Some(record)
}

/// Names the keyboard trigger a window key drives, from [`input::utils::KEYBOARD_KEYS`].
///
/// The match is exhaustive, so a key added to the window layer cannot be dropped silently.
fn keyboard_trigger(key: ghi::window::input::Keys) -> &'static str {
	use ghi::window::input::Keys;

	match key {
		Keys::A => "Keyboard.A",
		Keys::B => "Keyboard.B",
		Keys::C => "Keyboard.C",
		Keys::D => "Keyboard.D",
		Keys::E => "Keyboard.E",
		Keys::F => "Keyboard.F",
		Keys::G => "Keyboard.G",
		Keys::H => "Keyboard.H",
		Keys::I => "Keyboard.I",
		Keys::J => "Keyboard.J",
		Keys::K => "Keyboard.K",
		Keys::L => "Keyboard.L",
		Keys::M => "Keyboard.M",
		Keys::N => "Keyboard.N",
		Keys::O => "Keyboard.O",
		Keys::P => "Keyboard.P",
		Keys::Q => "Keyboard.Q",
		Keys::R => "Keyboard.R",
		Keys::S => "Keyboard.S",
		Keys::T => "Keyboard.T",
		Keys::U => "Keyboard.U",
		Keys::V => "Keyboard.V",
		Keys::W => "Keyboard.W",
		Keys::X => "Keyboard.X",
		Keys::Y => "Keyboard.Y",
		Keys::Z => "Keyboard.Z",
		Keys::Num0 => "Keyboard.Num0",
		Keys::Num1 => "Keyboard.Num1",
		Keys::Num2 => "Keyboard.Num2",
		Keys::Num3 => "Keyboard.Num3",
		Keys::Num4 => "Keyboard.Num4",
		Keys::Num5 => "Keyboard.Num5",
		Keys::Num6 => "Keyboard.Num6",
		Keys::Num7 => "Keyboard.Num7",
		Keys::Num8 => "Keyboard.Num8",
		Keys::Num9 => "Keyboard.Num9",
		Keys::NumPad0 => "Keyboard.NumPad0",
		Keys::NumPad1 => "Keyboard.NumPad1",
		Keys::NumPad2 => "Keyboard.NumPad2",
		Keys::NumPad3 => "Keyboard.NumPad3",
		Keys::NumPad4 => "Keyboard.NumPad4",
		Keys::NumPad5 => "Keyboard.NumPad5",
		Keys::NumPad6 => "Keyboard.NumPad6",
		Keys::NumPad7 => "Keyboard.NumPad7",
		Keys::NumPad8 => "Keyboard.NumPad8",
		Keys::NumPad9 => "Keyboard.NumPad9",
		Keys::NumPadAdd => "Keyboard.NumPadAdd",
		Keys::NumPadSubtract => "Keyboard.NumPadSubtract",
		Keys::NumPadMultiply => "Keyboard.NumPadMultiply",
		Keys::NumPadDivide => "Keyboard.NumPadDivide",
		Keys::NumPadDecimal => "Keyboard.NumPadDecimal",
		Keys::NumPadEnter => "Keyboard.NumPadEnter",
		Keys::Backspace => "Keyboard.Backspace",
		Keys::Tab => "Keyboard.Tab",
		Keys::Enter => "Keyboard.Enter",
		Keys::ShiftLeft => "Keyboard.LeftShift",
		Keys::ShiftRight => "Keyboard.RightShift",
		Keys::ControlLeft => "Keyboard.LeftControl",
		Keys::ControlRight => "Keyboard.RightControl",
		Keys::AltLeft => "Keyboard.LeftAlt",
		Keys::AltRight => "Keyboard.RightAlt",
		Keys::Menu => "Keyboard.Menu",
		Keys::Space => "Keyboard.Space",
		Keys::Insert => "Keyboard.Insert",
		Keys::Delete => "Keyboard.Delete",
		Keys::Home => "Keyboard.Home",
		Keys::End => "Keyboard.End",
		Keys::PageUp => "Keyboard.PageUp",
		Keys::PageDown => "Keyboard.PageDown",
		Keys::ArrowUp => "Keyboard.Up",
		Keys::ArrowDown => "Keyboard.Down",
		Keys::ArrowLeft => "Keyboard.Left",
		Keys::ArrowRight => "Keyboard.Right",
		Keys::Escape => "Keyboard.Escape",
		Keys::F1 => "Keyboard.F1",
		Keys::F2 => "Keyboard.F2",
		Keys::F3 => "Keyboard.F3",
		Keys::F4 => "Keyboard.F4",
		Keys::F5 => "Keyboard.F5",
		Keys::F6 => "Keyboard.F6",
		Keys::F7 => "Keyboard.F7",
		Keys::F8 => "Keyboard.F8",
		Keys::F9 => "Keyboard.F9",
		Keys::F10 => "Keyboard.F10",
		Keys::F11 => "Keyboard.F11",
		Keys::F12 => "Keyboard.F12",
		Keys::NumLock => "Keyboard.NumLock",
		Keys::ScrollLock => "Keyboard.ScrollLock",
		Keys::CapsLock => "Keyboard.CapsLock",
		Keys::PrintScreen => "Keyboard.PrintScreen",
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::input::utils::{register_keyboard_device_class, register_mouse_device_class};

	/// Builds the standard mouse and keyboard devices the translation expects.
	fn window_input() -> (input::InputCollector, input::DeviceHandle, input::DeviceHandle) {
		let mut collector = input::InputCollector::new();
		let mouse = register_mouse_device_class(&mut collector);
		let keyboard = register_keyboard_device_class(&mut collector);
		let mouse = collector.create_device(&mouse);
		let keyboard = collector.create_device(&keyboard);
		(collector, mouse, keyboard)
	}

	/// Records one event and returns the queued value of a device's control.
	fn recorded(
		event: ghi::window::Events,
		device: impl Fn(input::DeviceHandle, input::DeviceHandle) -> input::DeviceHandle,
		name: &'static str,
	) -> input::Value {
		let (mut collector, mouse, keyboard) = window_input();
		assert!(!process_default_window_input(&mut collector, event));
		assert_eq!(collector.source_events().len(), 1);
		collector
			.value(
				input::SeatHandle::stub(),
				device(mouse, keyboard),
				input::TriggerReference::Name(name),
			)
			.expect("registered control")
	}

	#[test]
	fn every_window_key_drives_a_registered_keyboard_trigger() {
		use ghi::window::input::Keys;

		for key in [
			Keys::A,
			Keys::B,
			Keys::C,
			Keys::D,
			Keys::E,
			Keys::F,
			Keys::G,
			Keys::H,
			Keys::I,
			Keys::J,
			Keys::K,
			Keys::L,
			Keys::M,
			Keys::N,
			Keys::O,
			Keys::P,
			Keys::Q,
			Keys::R,
			Keys::S,
			Keys::T,
			Keys::U,
			Keys::V,
			Keys::W,
			Keys::X,
			Keys::Y,
			Keys::Z,
			Keys::Num0,
			Keys::Num1,
			Keys::Num2,
			Keys::Num3,
			Keys::Num4,
			Keys::Num5,
			Keys::Num6,
			Keys::Num7,
			Keys::Num8,
			Keys::Num9,
			Keys::NumPad0,
			Keys::NumPad1,
			Keys::NumPad2,
			Keys::NumPad3,
			Keys::NumPad4,
			Keys::NumPad5,
			Keys::NumPad6,
			Keys::NumPad7,
			Keys::NumPad8,
			Keys::NumPad9,
			Keys::NumPadAdd,
			Keys::NumPadSubtract,
			Keys::NumPadMultiply,
			Keys::NumPadDivide,
			Keys::NumPadDecimal,
			Keys::NumPadEnter,
			Keys::Backspace,
			Keys::Tab,
			Keys::Enter,
			Keys::ShiftLeft,
			Keys::ShiftRight,
			Keys::ControlLeft,
			Keys::ControlRight,
			Keys::AltLeft,
			Keys::AltRight,
			Keys::Menu,
			Keys::Space,
			Keys::Insert,
			Keys::Delete,
			Keys::Home,
			Keys::End,
			Keys::PageUp,
			Keys::PageDown,
			Keys::ArrowUp,
			Keys::ArrowDown,
			Keys::ArrowLeft,
			Keys::ArrowRight,
			Keys::Escape,
			Keys::F1,
			Keys::F2,
			Keys::F3,
			Keys::F4,
			Keys::F5,
			Keys::F6,
			Keys::F7,
			Keys::F8,
			Keys::F9,
			Keys::F10,
			Keys::F11,
			Keys::F12,
			Keys::NumLock,
			Keys::ScrollLock,
			Keys::CapsLock,
			Keys::PrintScreen,
		] {
			let event = ghi::window::Events::Key {
				seat: ghi::window::Seat::stub(),
				pressed: true,
				key,
			};
			assert_eq!(
				recorded(event, |_, keyboard| keyboard, keyboard_trigger(key)),
				input::Value::Bool(true),
				"{key:?}"
			);
		}
	}

	#[test]
	fn maps_mouse_move_to_mouse_movement_trigger() {
		let event = ghi::window::Events::MouseMove {
			seat: ghi::window::Seat::stub(),
			dx: 0.25,
			dy: -0.5,
			time: 1,
		};
		assert_eq!(
			recorded(event, |mouse, _| mouse, "Mouse.Movement"),
			input::Value::Vector2(input::Axis2::new(0.25, -0.5))
		);
	}

	#[test]
	fn focus_loss_and_resize_discard_the_seat_and_report_the_interruption() {
		let (mut collector, ..) = window_input();
		let press = ghi::window::Events::Button {
			seat: ghi::window::Seat::stub(),
			pressed: true,
			button: ghi::window::input::MouseKeys::Left,
		};
		for interruption in [
			ghi::window::Events::FocusChanged(false),
			ghi::window::Events::Resize { width: 4, height: 4 },
			ghi::window::Events::Minimize,
			ghi::window::Events::Close,
		] {
			assert!(!process_default_window_input(&mut collector, press));
			assert!(process_default_window_input(&mut collector, interruption));
			assert_eq!(collector.source_events().len(), 0);
		}
		assert!(!process_default_window_input(
			&mut collector,
			ghi::window::Events::FocusChanged(true)
		));
		assert!(!process_default_window_input(&mut collector, ghi::window::Events::Maximize));
	}
}
