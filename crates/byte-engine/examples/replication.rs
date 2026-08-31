//! Creates connected replication endpoints as a networking smoke test.

use std::time::Instant;

use byte_engine::{
	core::factory::Factory,
	network::{Replicable, channel::ChannelServer as Server},
	space::Positionable,
};
use math::Point;
use serde::{Deserialize, Serialize};

fn main() {
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

	let replicable_factory = Factory::new();
	replicable_factory.create(Object {
		position: Point::new(0.5, 0.5, 0.5),
	});
	update();
	update();
	update();

	let mut data = [0_u8; 1024];
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
	position: Point,
}

impl Positionable for Object {
	fn position(&self) -> Point {
		self.position
	}
	fn set_position(&mut self, position: Point) {
		self.position = position;
	}
}

impl Replicable for Object {
	fn payload(&self) -> &u8 {
		todo!()
	}
}
