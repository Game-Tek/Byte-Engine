//! Adapters between the graphics application and external event protocols.
//!
//! [`process_default_window_input`] targets device classes installed by
//! [`super::setup_default_input`]. [`setup_default_dmx`] is optional and should
//! only be installed by applications that publish color values to Art-Net.

use std::{
	net::{Ipv4Addr, UdpSocket},
	time::Duration,
};

use artnet_protocol::{ArtCommand, ArtTalkToMe, Output, Poll};
use math::Vector2;
use utils::RGBA;

use super::GraphicsApplication;
use crate::{
	application::{parameters::Parameters as _, thread::Thread, Events, Parameter},
	core::listener::{DefaultListener, Listener as _},
	input,
};

/// Starts an Art-Net worker that publishes received [`RGBA`] values as DMX
/// output.
pub fn setup_default_dmx(application: &mut GraphicsApplication, mut receiver: DefaultListener<RGBA>) {
	let bind_address = parse_artnet_ipv4_parameter(application.get_parameter("artnet.bind-address"), Ipv4Addr::UNSPECIFIED);
	let poll_target = parse_artnet_ipv4_parameter(application.get_parameter("artnet.poll-target"), Ipv4Addr::BROADCAST);

	application
		.threads
		.push(Thread::new(application.application_events.0.spawn_rx(), move |mut events| {
			const ARTNET_PORT: u16 = 6454;

			let socket = UdpSocket::bind((bind_address, ARTNET_PORT)).unwrap();
			let target = (poll_target, ARTNET_PORT);
			socket.set_broadcast(true).unwrap();

			loop {
				if let Ok(Events::Close) = events.try_recv() {
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

/// Converts GHI window events into records for the standard mouse and keyboard
/// device classes.
pub fn process_default_window_input(
	input_system: &mut input::InputManager,
	event: ghi::window::Events,
) -> Option<(
	input::SeatHandle,
	input::DeviceHandle,
	input::input_manager::TriggerReference,
	input::Value,
)> {
	let mouse = *input_system.get_devices_by_class_name("Mouse")?.first()?;
	let keyboard = *input_system.get_devices_by_class_name("Keyboard")?.first()?;
	let seat = input::SeatHandle::stub();

	let record = match event {
		ghi::window::Events::Button { pressed, button, .. } => {
			let trigger = match button {
				ghi::window::input::MouseKeys::Left => "Mouse.LeftButton",
				ghi::window::input::MouseKeys::Right => "Mouse.RightButton",
				ghi::window::input::MouseKeys::Middle => "Mouse.MiddleButton",
				ghi::window::input::MouseKeys::ScrollUp => {
					return Some((
						seat,
						mouse,
						input::input_manager::TriggerReference::Name("Mouse.Scroll"),
						input::Value::Float(1.0),
					));
				}
				ghi::window::input::MouseKeys::ScrollDown => {
					return Some((
						seat,
						mouse,
						input::input_manager::TriggerReference::Name("Mouse.Scroll"),
						input::Value::Float(-1.0),
					));
				}
			};
			(
				seat,
				mouse,
				input::input_manager::TriggerReference::Name(trigger),
				input::Value::Bool(pressed),
			)
		}
		ghi::window::Events::MousePosition { x, y, .. } => (
			seat,
			mouse,
			input::input_manager::TriggerReference::Name("Mouse.Position"),
			input::Value::Vector2(Vector2::new(x, y)),
		),
		ghi::window::Events::MouseMove { dx, dy, .. } => (
			seat,
			mouse,
			input::input_manager::TriggerReference::Name("Mouse.Movement"),
			input::Value::Vector2(Vector2::new(dx, dy)),
		),
		ghi::window::Events::Key { pressed, key, .. } => {
			let trigger = match key {
				ghi::window::input::Keys::W => "Keyboard.W",
				ghi::window::input::Keys::S => "Keyboard.S",
				ghi::window::input::Keys::A => "Keyboard.A",
				ghi::window::input::Keys::D => "Keyboard.D",
				ghi::window::input::Keys::Space => "Keyboard.Space",
				ghi::window::input::Keys::Escape => "Keyboard.Escape",
				_ => return None,
			};
			(
				seat,
				keyboard,
				input::input_manager::TriggerReference::Name(trigger),
				input::Value::Bool(pressed),
			)
		}
		_ => return None,
	};

	Some(record)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::channel::{Channel as _, DefaultChannel};
	use crate::input::utils::{register_keyboard_device_class, register_mouse_device_class};

	fn input_manager() -> input::InputManager {
		let actions = DefaultChannel::new();
		let mut manager = input::InputManager::new(actions.listener(), DefaultChannel::new());
		let mouse = register_mouse_device_class(&mut manager);
		let keyboard = register_keyboard_device_class(&mut manager);
		manager.create_device(&mouse);
		manager.create_device(&keyboard);
		manager
	}

	#[test]
	fn maps_mouse_move_to_mouse_movement_trigger() {
		let mut manager = input_manager();
		let result = process_default_window_input(
			&mut manager,
			ghi::window::Events::MouseMove {
				seat: ghi::window::Seat::stub(),
				dx: 0.25,
				dy: -0.5,
				time: 1,
			},
		)
		.unwrap();

		assert!(matches!(
			result.2,
			input::input_manager::TriggerReference::Name("Mouse.Movement")
		));
		assert_eq!(result.3, input::Value::Vector2(Vector2::new(0.25, -0.5)));
	}

	#[test]
	fn parses_artnet_ipv4_parameter() {
		let parameter = Parameter::new("artnet.bind-address", "2.0.0.15");
		assert_eq!(
			parse_artnet_ipv4_parameter(Some(&parameter), Ipv4Addr::UNSPECIFIED),
			Ipv4Addr::new(2, 0, 0, 15)
		);
	}
}
