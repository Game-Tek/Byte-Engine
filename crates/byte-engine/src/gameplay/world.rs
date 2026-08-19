//! Standard world composition shared by gameplay, physics, and rendering.
//!
//! Create objects through the factories exposed by [`DefaultWorld`] so
//! downstream systems receive creation and deletion messages. The graphics
//! application updates this world and attaches its listeners to render
//! pipelines.

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

impl Publisher<TransformationUpdate> for DefaultWorld {
	fn publish(&self, message: TransformationUpdate) {
		self.transforms.send(message);
	}
}

impl Publisher<CreateMessage<Camera>> for DefaultWorld {
	fn publish(&self, message: CreateMessage<Camera>) {
		let handle = *message.handle();
		self.cameras.derive(handle, message.into_data());
	}
}

impl TargetedMessagePublisher<Transform> for DefaultWorld {
	type Message = TransformationUpdate;
}

impl TargetedMessagePublisher<Camera> for DefaultWorld {
	type Message = CreateMessage<Camera>;
}

impl Creator<Lights> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, light: Lights) -> Handle {
		publish_to_factory(&mut self.light_factory, handle, light)
	}
}

macro_rules! impl_light_creator {
	($light:ty) => {
		impl Creator<$light> for DefaultWorld {
			fn publish(&mut self, handle: Option<Handle>, light: $light) -> Handle {
				publish_to_factory(&mut self.light_factory, handle, light.into())
			}
		}
	};
}

impl_light_creator!(ConeLight);
impl_light_creator!(DirectionalLight);
impl_light_creator!(PointLight);

impl Creator<Camera> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, camera: Camera) -> Handle {
		publish_to_factory(&mut self.cameras, handle, camera)
	}
}

impl Creator<Environment> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, environment: Environment) -> Handle {
		publish_to_factory(&mut self.environment_factory, handle, environment)
	}
}

impl Creator<EntityHandle<dyn physics::Body>> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, body: EntityHandle<dyn physics::Body>) -> Handle {
		publish_to_factory(&mut self.body_factory, handle, body)
	}
}

impl Creator<EntityHandle<dyn RenderableMesh>> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, renderable: EntityHandle<dyn RenderableMesh>) -> Handle {
		publish_to_factory(&mut self.renderable_factory, handle, renderable)
	}
}

impl Creator<&mut AudioGraph> for DefaultWorld {
	fn publish(&mut self, handle: Option<Handle>, graph: &mut AudioGraph) -> Handle {
		if let Some(handle) = handle {
			self.audio_graph_factory.derive(handle, graph);
			handle
		} else {
			self.audio_graph_factory.create(graph)
		}
	}
}

/// Publishes through a factory while preserving an optional creation-chain handle.
fn publish_to_factory<T: Clone>(factory: &mut Factory<T>, handle: Option<Handle>, value: T) -> Handle {
	if let Some(handle) = handle {
		factory.derive(handle, value);
		handle
	} else {
		factory.create(value)
	}
}

use std::alloc::Allocator;

use crate::{
	application::Time,
	audio::graph::{AudioGraph, AudioGraphFactory},
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory, Handle},
		listener::{DefaultListener, Listener},
		message::{DeleteMessage, Message},
		publisher::Publisher,
		targeted_message::TargetedMessagePublisher,
		EntityHandle,
	},
	gameplay::{anchor::AnchorSystem, transform::TransformationUpdate, Transform},
	physics::{self, dynabit},
	rendering::{lights::Lights, Camera, ConeLight, DirectionalLight, Environment, PointLight, RenderableMesh, UpdatePose},
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::{listener::Listener, targeted_message::MessageTargeter};

	#[test]
	fn camera_set_publishes_an_upsert_under_the_existing_handle() {
		let mut world = DefaultWorld::new();
		let mut cameras = world.camera_factory().listener();
		let handle: Handle = world.create(Camera::new()).into();
		let _ = cameras.read().expect("camera creation");

		world.set(Camera::new().with_fov(72.0)).on(handle);

		let update = cameras.read().expect("camera update");

		assert_eq!(update.handle(), &handle);
		assert_eq!(update.data().vertical_fov(), 72.0);
	}
}
