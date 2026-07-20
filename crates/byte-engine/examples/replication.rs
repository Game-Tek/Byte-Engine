//! Creates connected replication endpoints as a networking smoke test.

use std::time::Instant;

use byte_engine::{
	core::factory::Factory,
	network::{channel::ChannelServer as Server, Replicable},
	space::Positionable,
};
use math::Vector3;
use serde::{Deserialize, Serialize};

fn main() {
	// let mut server_app = byte_engine::application::graphics::GraphicsApplication::new(
	// 	"Server",
	// 	&[
	// 		Parameter::new("kill-after", "60"),
	// 		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	// 	],
	// );

	// let mut client_a_app = byte_engine::application::graphics::GraphicsApplication::new(
	// 	"Client A",
	// 	&[
	// 		Parameter::new("kill-after", "60"),
	// 		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	// 	],
	// );

	// let mut client_b_app = byte_engine::application::graphics::GraphicsApplication::new(
	// 	"Client B",
	// 	&[
	// 		Parameter::new("kill-after", "60"),
	// 		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	// 	],
	// );

	let mut server = Server::new();
	let mut client_a = server.client();
	let mut client_b = server.client();

	client_a.connect(Instant::now());
	client_b.connect(Instant::now());

	let mut update = || {
		client_a.update().unwrap();
		client_b.update().unwrap();
		server.update(Instant::now()).unwrap();
	};

	let mut replicable_factory = Factory::new();

	let a = Object {
		position: Vector3::new(0.5f32, 0.5f32, 0.5f32),
	};

	replicable_factory.create(a);

	update();
	update();
	update();

	let mut data = [0u8; 1024];
	data[0] = Commands::Spawn as u8;

	server.send(true, data);

	client_a.update().unwrap();
	client_b.update().unwrap();
	server.update(Instant::now()).unwrap();

	for packet in server.drain_received() {
		if packet.data[0] == Commands::Spawn as u8 {
			println!("Requested spawn");
		}
	}
}

#[repr(u8)]
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Commands {
	Spawn,
}

#[derive(Clone)]
struct Object {
	position: Vector3,
}

impl Positionable for Object {
	fn position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}

impl Replicable for Object {
	fn payload(&self) -> &u8 {
		todo!()
	}
}
