//! Standard world composition shared by gameplay, physics, and rendering.
//!
//! Create objects through the factories exposed by [`DefaultWorld`] so
//! downstream systems receive creation and deletion messages. The graphics
//! application updates this world and attaches its listeners to render
//! pipelines.

use std::alloc::Allocator;

use crate::{
	application::Time,
	audio::graph::AudioGraphFactory,
	core::{
		channel::{Channel, DefaultChannel},
		factory::Factory,
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		EntityHandle,
	},
	gameplay::{anchor::AnchorSystem, transform::TransformationUpdate},
	physics::{self, dynabit},
	rendering::{lights::Lights, Camera, Environment, RenderableMesh, UpdatePose},
};

#[derive(Clone)]
/// The [`DefaultWorld`] struct owns the standard entity factories and coordinates
/// transform, physics, anchoring, and deletion updates.
pub struct DefaultWorld {
	body_factory: Factory<EntityHandle<dyn physics::Body>>,
	transforms: DefaultChannel<TransformationUpdate>,
	deletes: DefaultChannel<DeleteMessage>,
	poses: DefaultChannel<UpdatePose>,
	cameras: Factory<Camera>,
	renderable_factory: Factory<EntityHandle<dyn RenderableMesh>>,
	light_factory: Factory<Lights>,
	environment_factory: Factory<Environment>,
	audio_graph_factory: AudioGraphFactory,

	anchor_system: AnchorSystem,
	physics_system: dynabit::World,
}

impl Default for DefaultWorld {
	fn default() -> Self {
		Self::new()
	}
}

impl DefaultWorld {
	pub fn new() -> Self {
		let body_factory = Factory::new();
		let transforms = DefaultChannel::new();
		let deletes = DefaultChannel::new();
		let cameras = Factory::new();
		let renderable_factory = Factory::new();

		let anchor_system = AnchorSystem::new();
		let physics_system = dynabit::World::new(body_factory.listener(), deletes.listener());

		Self {
			body_factory,
			transforms,
			deletes,
			poses: DefaultChannel::new(),
			cameras,
			renderable_factory,
			light_factory: Factory::new(),
			environment_factory: Factory::new(),
			audio_graph_factory: AudioGraphFactory::new(),

			anchor_system,
			physics_system,
		}
	}

	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		allocator: &mut bumpalo::Bump,
	) {
		self.anchor_system.update();
		self.physics_system
			.update(time, transforms_rx, &mut self.transforms, allocator);
	}

	pub fn flush_deletions(&mut self) {
		self.physics_system.process_pending_deletions();
	}

	pub fn body_factory(&self) -> &Factory<EntityHandle<dyn physics::Body>> {
		&self.body_factory
	}

	pub fn body_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn physics::Body>> {
		&mut self.body_factory
	}

	pub fn transforms_channel(&self) -> &DefaultChannel<TransformationUpdate> {
		&self.transforms
	}

	pub fn transforms_channel_mut(&mut self) -> &mut DefaultChannel<TransformationUpdate> {
		&mut self.transforms
	}

	pub fn delete_channel(&self) -> &DefaultChannel<DeleteMessage> {
		&self.deletes
	}

	pub fn delete_channel_mut(&mut self) -> &mut DefaultChannel<DeleteMessage> {
		&mut self.deletes
	}

	pub fn poses_channel(&self) -> &DefaultChannel<UpdatePose> {
		&self.poses
	}

	pub fn poses_channel_mut(&mut self) -> &mut DefaultChannel<UpdatePose> {
		&mut self.poses
	}

	pub fn renderable_factory(&self) -> &Factory<EntityHandle<dyn RenderableMesh>> {
		&self.renderable_factory
	}

	pub fn renderable_factory_mut(&mut self) -> &mut Factory<EntityHandle<dyn RenderableMesh>> {
		&mut self.renderable_factory
	}

	pub fn light_factory(&self) -> &Factory<Lights> {
		&self.light_factory
	}

	pub fn light_factory_mut(&mut self) -> &mut Factory<Lights> {
		&mut self.light_factory
	}

	/// Returns the factory used to select the world's scene environment.
	pub fn environment_factory(&self) -> &Factory<Environment> {
		&self.environment_factory
	}

	/// Returns mutable access to the factory used to select the world's scene environment.
	///
	/// Next, call [`Factory::create`] with an [`Environment`] after installing
	/// the visibility pipeline.
	pub fn environment_factory_mut(&mut self) -> &mut Factory<Environment> {
		&mut self.environment_factory
	}

	/// Returns the factory used to spawn resource-backed audio graphs.
	pub fn audio_graph_factory(&self) -> &AudioGraphFactory {
		&self.audio_graph_factory
	}

	/// Returns mutable access to the factory used to spawn resource-backed
	/// audio graphs.
	pub fn audio_graph_factory_mut(&mut self) -> &mut AudioGraphFactory {
		&mut self.audio_graph_factory
	}

	pub fn camera_factory(&self) -> &Factory<Camera> {
		&self.cameras
	}

	pub fn camera_factory_mut(&mut self) -> &mut Factory<Camera> {
		&mut self.cameras
	}
}

#[cfg(test)]
mod tests {
	use math::Point;

	use super::*;
	use crate::{
		core::listener::Listener,
		gameplay::{Object, Transform},
		physics::Body,
		space::{Positionable, Transformable},
	};

	#[test]
	fn world_publishes_object_creation_with_a_branded_position() {
		let mut world = DefaultWorld::new();
		let mut listener = world.body_factory().listener();
		let mut object = Object::sphere(1.5);
		object.transform_mut().set_position(Point::new(1.0, 2.0, 3.0));
		let body: EntityHandle<dyn Body> = EntityHandle::from(object);
		world.body_factory_mut().create(body);
		assert_eq!(
			listener.read().expect("physics creation").data().position(),
			Point::new(1.0, 2.0, 3.0)
		);
	}

	#[test]
	fn world_routes_environment_creation_to_rendering_listeners() {
		let mut world = DefaultWorld::new();
		let mut listener = world.environment_factory().listener();
		world.environment_factory_mut().create(Environment::new("studio.exr"));
		assert_eq!(
			listener.read().expect("environment creation").data().resource_id(),
			"studio.exr"
		);
	}
}
