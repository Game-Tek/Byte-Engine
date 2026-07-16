//! Standard world composition shared by gameplay, physics, and rendering.
//!
//! Create objects through the factories exposed by [`DefaultWorld`] so
//! downstream systems receive creation and deletion messages. The graphics
//! application updates this world and attaches its listeners to render
//! pipelines.

use std::alloc::Allocator;

use crate::{
	application::Time,
	core::{
		channel::{Channel, DefaultChannel},
		factory::Factory,
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		EntityHandle,
	},
	gameplay::{anchor::AnchorSystem, transform::TransformationUpdate},
	physics::{self, dynabit},
	rendering::{lights::Lights, Camera, RenderableMesh, UpdatePose},
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

	pub fn camera_factory(&self) -> &Factory<Camera> {
		&self.cameras
	}

	pub fn camera_factory_mut(&mut self) -> &mut Factory<Camera> {
		&mut self.cameras
	}
}

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::*;
	use crate::{
		core::{channel::Channel, listener::Listener, message::DeleteMessage},
		gameplay::{Object, Transform},
		physics::Body,
		rendering::{lights::PointLight, RenderableMesh},
		space::Transformable,
	};

	#[test]
	fn world_routes_one_lifecycle_identity_to_physics_rendering_and_state_channels() {
		let mut world = DefaultWorld::new();
		let mut body_listener = world.body_factory().listener();
		let mut renderable_listener = world.renderable_factory().listener();
		let mut transform_listener = world.transforms_channel().listener();
		let mut delete_listener = world.delete_channel().listener();

		let mut object = Object::sphere(1.5);
		object.transform_mut().set_position(Vector3::new(1.0, 2.0, 3.0));
		let concrete = EntityHandle::from(object);
		let body: EntityHandle<dyn Body> = concrete.clone();
		let renderable: EntityHandle<dyn RenderableMesh> = concrete;

		let lifecycle_handle = world.body_factory_mut().create(body);
		world.renderable_factory_mut().derive(lifecycle_handle, renderable);
		TransformationUpdate::apply(
			world.transforms_channel_mut(),
			lifecycle_handle,
			Transform::from_position(Vector3::new(4.0, 5.0, 6.0)),
		);
		world.delete_channel_mut().send(DeleteMessage::new(lifecycle_handle));

		let body_creation = body_listener.read().expect("physics creation");
		let renderable_creation = renderable_listener.read().expect("render creation");
		let transform = transform_listener.read().expect("transform update");
		let deletion = delete_listener.read().expect("deletion update");

		assert_eq!(body_creation.handle(), &lifecycle_handle);
		assert_eq!(renderable_creation.handle(), &lifecycle_handle);
		assert_eq!(body_creation.data().transform().get_position(), Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(
			renderable_creation.data().transform().get_position(),
			Vector3::new(1.0, 2.0, 3.0)
		);
		assert_eq!(transform.handle(), &lifecycle_handle);
		assert_eq!(transform.transform().get_position(), Vector3::new(4.0, 5.0, 6.0));
		assert_eq!(deletion.handle(), &lifecycle_handle);
	}

	#[test]
	fn camera_and_light_factories_publish_typed_scene_payloads() {
		let mut world = DefaultWorld::new();
		let mut camera_listener = world.camera_factory().listener();
		let mut light_listener = world.light_factory().listener();

		let camera_handle = world.camera_factory_mut().create(Camera::new());
		let light_handle = world
			.light_factory_mut()
			.create(PointLight::new(Vector3::new(3.0, 2.0, 1.0), 5_000.0).into());

		let camera = camera_listener.read().expect("camera creation");
		let light = light_listener.read().expect("light creation");
		assert_eq!(camera.handle(), &camera_handle);
		assert_eq!(camera.data().get_fov(), 45.0);
		assert_eq!(light.handle(), &light_handle);
		assert!(matches!(light.data(), Lights::Point(point) if point.position == Vector3::new(3.0, 2.0, 1.0)));
	}
}
