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
	let bind_address = parse_artnet_ipv4_parameter(application.get_parameter("artnet.bind-address"), Ipv4Addr::UNSPECIFIED);
	let poll_target = parse_artnet_ipv4_parameter(application.get_parameter("artnet.poll-target"), Ipv4Addr::BROADCAST);

	application
		.threads
		.push(Thread::new(application.application_events.0.listener(), move |mut events| {
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

impl crate::core::message::Message for ghi::window::Events {}

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
		ghi::window::Events::Key { pressed, key, .. } => {
			let trigger = match key {
				ghi::window::input::Keys::W => "Keyboard.W",
				ghi::window::input::Keys::S => "Keyboard.S",
				ghi::window::input::Keys::A => "Keyboard.A",
				ghi::window::input::Keys::D => "Keyboard.D",
				ghi::window::input::Keys::Space => "Keyboard.Space",
				ghi::window::input::Keys::Escape => "Keyboard.Escape",
				ghi::window::input::Keys::Backspace => "Keyboard.Backspace",
				_ => return None,
			};
			(keyboard, trigger, input::Value::Bool(pressed))
		}
		ghi::window::Events::Character { character, .. } => (keyboard, "Keyboard.Character", input::Value::Unicode(character)),
		_ => return None,
	};

	Some(record)
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
	fn maps_scroll_to_mouse_scroll_trigger() {
		let event = ghi::window::Events::Scroll {
			seat: ghi::window::Seat::stub(),
			dx: 0.0,
			dy: -0.75,
			time: 1,
		};
		assert_eq!(recorded(event, |mouse, _| mouse, "Mouse.Scroll"), input::Value::Float(-0.75));
	}

	#[test]
	fn maps_backspace_to_keyboard_backspace_trigger() {
		let event = ghi::window::Events::Key {
			seat: ghi::window::Seat::stub(),
			pressed: true,
			key: ghi::window::input::Keys::Backspace,
		};
		assert_eq!(
			recorded(event, |_, keyboard| keyboard, "Keyboard.Backspace"),
			input::Value::Bool(true)
		);
	}

	#[test]
	fn maps_character_to_keyboard_character_trigger() {
		let event = ghi::window::Events::Character {
			seat: ghi::window::Seat::stub(),
			character: 'é',
		};
		assert_eq!(
			recorded(event, |_, keyboard| keyboard, "Keyboard.Character"),
			input::Value::Unicode('é')
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

	#[cfg(feature = "dmx")]
	#[test]
	fn parses_artnet_ipv4_parameter() {
		let parameter = Parameter::new("artnet.bind-address", "2.0.0.15");

		assert_eq!(
			parse_artnet_ipv4_parameter(Some(&parameter), Ipv4Addr::UNSPECIFIED),
			Ipv4Addr::new(2, 0, 0, 15)
		);
	}
}
