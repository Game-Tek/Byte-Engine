//! This is a smoke test that creates a replicated environments.

use byte_engine::{
	application::{Application, Parameter},
	core::factory::Factory,
	gameplay::world::DefaultWorld,
	network::Replicable,
	space::Positionable,
};
use math::Vector3;

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Replication Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	// space_handle.spawn(Cube::new(Vector3::new(0.5f32, 0.5f32, 0.5f32)).builder());

	let world = DefaultWorld::new();

	let _world_handle = app.world_factory_mut().create(world);

	let mut replicable_factory = Factory::new();

	let a = Object {
		position: Vector3::new(0.5f32, 0.5f32, 0.5f32),
	};

	replicable_factory.create(a);

	// TODO: test replication

	app.do_loop();
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
