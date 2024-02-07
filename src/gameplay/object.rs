use maths_rs::mat::{MatScale, MatTranslate};

use crate::{core::{entity::EntityBuilder, event::Event, spawn, spawn_as_child, EntityHandle}, physics, rendering::mesh, Vector3};

pub struct Object {
	render: EntityHandle<mesh::Mesh>,
	collision: EntityHandle<physics::Sphere>,
}

impl Object {
	pub fn new<'a>(position: Vector3, velocity: Vector3) -> EntityBuilder<'a, Self> {
		let transform = maths_rs::Mat4f::from_translation(position) * maths_rs::Mat4f::from_scale(Vector3::new(0.05, 0.05, 0.05));

		EntityBuilder::new_from_closure_with_parent(move |parent| {
			Object {
				collision: spawn_as_child(parent.clone(), physics::Sphere::new(position, velocity, 0.1f32)),
				render: spawn_as_child(parent.clone(), mesh::Mesh::new("Sphere", "solid", transform)),
			}
		})
	}

	pub fn collision(&self) -> &EntityHandle<physics::Sphere> { &self.collision }

	// pub fn on_collision(&mut self) -> &mut Event<EntityHandle<physics::Sphere>> { self.collision.write_sync().on_collision() }
}