//! Standard world composition shared by gameplay, physics, and rendering.
//!
//! Create objects through the factories exposed by [`DefaultWorld`] so
//! downstream systems receive creation and deletion messages. The graphics
//! application updates this world and attaches its listeners to render
//! pipelines.

/// The `DefaultWorld` struct owns the standard entity routes and coordinates transform, physics, anchoring, and deletion updates.
pub struct DefaultWorld {
	messages: MessageScope,
	transforms: DefaultChannel<TransformationUpdate>,
	deletes: DefaultChannel<DeleteMessage>,
	poses: DefaultChannel<UpdatePose>,
	audio_graph_factory: AudioGraphFactory,

	anchor_system: AnchorSystem,
	physics_system: dynabit::World,

	scene_graph: SceneGraph,
	scene_nodes: DefaultListener<CreateMessage<SceneNode>>,
	/// Deletions posted straight to the channel, such as inspector messages, which still need their scene subtree cascaded.
	posted_deletes: DefaultListener<DeleteMessage>,
}

impl Default for DefaultWorld {
	fn default() -> Self {
		Self::new()
	}
}

impl DefaultWorld {
	/// Creates a standalone world with its own fixed message arena.
	///
	/// Applications should use [`Self::with_messages`] so world routes appear in
	/// the application's unified diagnostics.
	pub fn new() -> Self {
		let bus = MessageBus::default();
		Self::with_messages(bus.new_scope("default-world"))
	}

	/// Creates a world whose typed routes use the supplied message scope.
	///
	/// Next, install system listeners before creating entities they must mirror.
	pub fn with_messages(messages: MessageScope) -> Self {
		let body_factory = messages.factory();
		let transforms = messages.channel();
		let deletes = messages.channel();
		let poses = messages.channel();
		let audio_graph_factory = AudioGraphFactory::in_scope(&messages);

		let anchor_system = AnchorSystem::new();
		let physics_system = dynabit::World::new(body_factory.listener(), deletes.listener());
		let scene_nodes = messages.factory::<SceneNode>().listener();
		let posted_deletes = deletes.listener();

		Self {
			messages,
			transforms,
			deletes,
			poses,
			audio_graph_factory,

			anchor_system,
			physics_system,

			scene_graph: SceneGraph::default(),
			scene_nodes,
			posted_deletes,
		}
	}

	/// Returns the world namespace used for lazy application-defined message types.
	pub fn messages(&self) -> &MessageScope {
		&self.messages
	}

	/// Acquires the world's canonical creation factory for `T`.
	///
	/// The type is registered only on first use. Create its listener before
	/// calling [`Creator::create`] when a system must observe every creation.
	pub fn factory<T>(&self) -> Factory<T>
	where
		T: Clone + Send + Sync + 'static,
	{
		self.messages.factory()
	}

	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		allocator: &mut bumpalo::Bump,
	) {
		self.cascade_posted_deletions();
		self.anchor_system.update();
		self.physics_system.update(time, transforms_rx, &self.transforms, allocator);
	}

	pub fn flush_deletions(&mut self) {
		self.physics_system.process_pending_deletions();
	}

	pub fn transforms_channel(&self) -> &DefaultChannel<TransformationUpdate> {
		&self.transforms
	}

	/// Creates a future-only listener for terminal entity deletions.
	///
	/// Next, keep the listener with the consuming system and remove matching
	/// state when it receives a [`DeleteMessage`]. Publish deletions through
	/// [`Self::delete`] so inspection diagnostics retire the same handle.
	pub fn deletions_listener(&self) -> DefaultListener<DeleteMessage> {
		self.deletes.listener()
	}

	/// Publishes terminal deletions for `handle` and every scene member nested under it.
	///
	/// Nested members are deleted children-first and `handle` last, so deleting
	/// a [`Scene`](crate::gameplay::Scene) tears down the whole level. Consumers
	/// created through [`Self::deletions_listener`] receive every handle and can
	/// retire their system-specific state.
	pub fn delete(&mut self, handle: Handle) {
		self.track_scene_nodes();
		let deletes = &self.deletes;
		self.scene_graph
			.remove_subtree(handle, |removed| publish_deletion(deletes, removed));
	}

	/// Links scene members created since the last call into the scene graph.
	fn track_scene_nodes(&mut self) {
		while let Some(creation) = self.scene_nodes.read() {
			self.scene_graph.attach(creation.handle(), creation.data().parent());
		}
	}

	/// Cascades deletions that bypassed [`Self::delete`] to their scene members.
	///
	/// Deletions from [`Self::delete`] also arrive here; their subtrees are
	/// already gone, so they publish nothing.
	fn cascade_posted_deletions(&mut self) {
		self.track_scene_nodes();
		let deletes = &self.deletes;
		while let Some(deletion) = self.posted_deletes.read() {
			let posted = deletion.into_handle();
			// The posted handle already has its deletion message.
			self.scene_graph.remove_subtree(posted, |removed| {
				if removed != posted {
					publish_deletion(deletes, removed)
				}
			});
		}
	}

	pub fn poses_channel(&self) -> &DefaultChannel<UpdatePose> {
		&self.poses
	}

	/// Returns the factory used to spawn resource-backed audio graphs.
	pub fn audio_graph_factory(&self) -> &AudioGraphFactory {
		&self.audio_graph_factory
	}
}

/// Sends one deletion and removes the handle from inspection diagnostics.
fn publish_deletion(deletes: &DefaultChannel<DeleteMessage>, handle: Handle) {
	deletes.send(DeleteMessage::new(handle));
	deletes.forget_entity(handle);
}

impl Publisher<TransformationUpdate> for DefaultWorld {
	fn publish(&self, message: TransformationUpdate) {
		self.transforms.send(message);
	}
}

impl Publisher<CreateMessage<Camera>> for DefaultWorld {
	fn publish(&self, message: CreateMessage<Camera>) {
		let handle = message.handle();
		self.factory().derive(handle, message.into_data());
	}
}

impl TargetedMessagePublisher<Transform> for DefaultWorld {
	type Message = TransformationUpdate;
}

impl TargetedMessagePublisher<Camera> for DefaultWorld {
	type Message = CreateMessage<Camera>;
}

impl<T> Creator<T> for DefaultWorld
where
	T: Clone + Send + Sync + 'static,
{
	fn publish(&self, handle: Option<Handle>, value: T) -> Handle {
		let factory = self.factory::<T>();
		if let Some(handle) = handle {
			factory.derive(handle, value);
			handle
		} else {
			factory.create(value)
		}
	}
}

impl Creator<&mut AudioGraph> for DefaultWorld {
	fn publish(&self, handle: Option<Handle>, graph: &mut AudioGraph) -> Handle {
		if let Some(handle) = handle {
			self.audio_graph_factory.derive(handle, graph);
			handle
		} else {
			self.audio_graph_factory.create(graph)
		}
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
		message::DeleteMessage,
		message_bus::{MessageBus, MessageScope},
		publisher::Publisher,
		targeted_message::TargetedMessagePublisher,
	},
	gameplay::{
		Name, Transform,
		anchor::AnchorSystem,
		scene::{SceneGraph, SceneNode},
		transform::TransformationUpdate,
	},
	physics::{self, dynabit},
	rendering::{Camera, UpdatePose},
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::{listener::Listener, targeted_message::MessageTargeter};
	use crate::gameplay::{Scene, SceneNode};
	use crate::rendering::{PointLight, RenderableMesh};

	#[test]
	fn renderable_body_and_transform_creation_share_a_handle() {
		let world = DefaultWorld::new();
		let mut renderables = world.factory::<RenderableMesh>().listener();
		let mut bodies = world.factory::<physics::Body>().listener();
		let mut names = world.factory::<Name>().listener();
		let mut transforms = world.transforms_channel().listener();

		let handle: Handle = world
			.create(RenderableMesh::sphere(1.0))
			.with(physics::Body::new(
				physics::BodyTypes::Dynamic,
				physics::Shapes::Sphere { radius: 1.0 },
			))
			.with(Name::new("ball"))
			.with(Transform::from_position(math::Point::new(1.0, 2.0, 3.0)))
			.into();

		assert_eq!(renderables.read().expect("renderable creation").handle(), handle);
		assert_eq!(bodies.read().expect("body creation").handle(), handle);
		let name = names.read().expect("name creation");
		assert_eq!(name.handle(), handle);
		assert_eq!(name.data().as_str(), "ball");
		assert_eq!(transforms.read().expect("transform creation").handle(), handle);
	}

	#[test]
	fn camera_and_transform_creation_share_a_handle() {
		let world = DefaultWorld::new();
		let mut cameras = world.factory::<Camera>().listener();
		let mut transforms = world.transforms_channel().listener();

		let handle: Handle = world.create(Camera::new()).with(Transform::identity()).into();

		let camera = cameras.read().expect("camera creation");
		let transform = transforms.read().expect("transform creation");
		assert_eq!(camera.handle(), handle);
		assert_eq!(transform.handle(), handle);
	}

	#[test]
	fn light_and_transform_creation_share_a_handle() {
		let world = DefaultWorld::new();
		let mut lights = world.factory::<PointLight>().listener();
		let mut transforms = world.transforms_channel().listener();
		let light = PointLight::new(
			crate::rendering::LightColor::LinearSrgb(maths_rs::Vec3f::new(1.0, 1.0, 1.0)),
			crate::rendering::PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
		)
		.expect("physical point light");

		let handle: Handle = world
			.create(light)
			.with(Transform::from_position(math::Point::new(1.0, 2.0, 3.0)))
			.into();

		let light = lights.read().expect("light creation");
		let transform = transforms.read().expect("transform creation");
		assert_eq!(light.handle(), handle);
		assert_eq!(transform.handle(), handle);
	}

	#[test]
	fn camera_set_publishes_an_upsert_under_the_existing_handle() {
		let world = DefaultWorld::new();
		let mut cameras = world.factory::<Camera>().listener();
		let handle: Handle = world.create(Camera::new()).into();
		let _ = cameras.read().expect("camera creation");

		world.set(Camera::new().with_fov(math::Degrees::new(72.0))).on(handle);

		let update = cameras.read().expect("camera update");

		assert_eq!(update.handle(), handle);
		assert_eq!(update.data().vertical_fov(), math::Degrees::new(72.0));
	}

	#[test]
	fn application_defined_creation_type_is_registered_on_first_use() {
		#[derive(Clone, Debug, PartialEq, Eq)]
		struct Sprite(&'static str);

		let world = DefaultWorld::new();
		let mut sprites = world.factory::<Sprite>().listener();
		let mut transforms = world.transforms_channel().listener();

		let handle: Handle = world
			.create(Sprite("floor.png"))
			.with(Transform::from_position(math::Point::new(1.0, 0.0, 2.0)))
			.into();

		let sprite = sprites.read().expect("application-defined creation");
		let transform = transforms.read().expect("shared transform creation");
		assert_eq!(sprite.handle(), handle);
		assert_eq!(sprite.data(), &Sprite("floor.png"));
		assert_eq!(transform.handle(), handle);
	}

	#[test]
	fn world_deletion_retires_the_factory_handle_from_inspection() {
		let message_bus = MessageBus::default();
		let observer = message_bus.observe().expect("attach observer");
		let mut world = DefaultWorld::with_messages(message_bus.new_scope("observed-world"));
		let mut deletions = world.deletions_listener();
		let handle = world.factory::<String>().create("temporary".to_string());

		assert_eq!(observer.entities()[0].handle(), handle);
		world.delete(handle);

		assert_eq!(deletions.read().expect("world deletion").into_handle(), handle);
		assert!(observer.entities().is_empty());
	}

	/// Reads every pending deletion handle in publication order.
	fn deleted(deletions: &mut DefaultListener<DeleteMessage>) -> Vec<Handle> {
		std::iter::from_fn(|| deletions.read().map(DeleteMessage::into_handle)).collect()
	}

	#[test]
	fn deleting_a_scene_deletes_every_nested_member_children_first() {
		let mut world = DefaultWorld::new();
		let mut deletions = world.deletions_listener();

		let scene: Handle = world.create(Scene).into();
		let tank: Handle = world.create(Name::new("tank")).with(SceneNode::under(scene)).into();
		let turret: Handle = world.create(Name::new("turret")).with(SceneNode::under(tank)).into();
		let rock: Handle = world.create(Name::new("rock")).with(SceneNode::under(scene)).into();
		let outsider: Handle = world.create(Name::new("outsider")).into();

		world.delete(scene);

		let deleted = deleted(&mut deletions);
		assert_eq!(deleted.len(), 4);
		assert_eq!(deleted.last(), Some(&scene));
		let position = |handle| deleted.iter().position(|&h| h == handle).expect("member deleted");
		assert!(position(turret) < position(tank));
		assert!(deleted.contains(&rock));
		assert!(!deleted.contains(&outsider));
	}

	#[test]
	fn deleting_a_member_detaches_it_from_its_scene() {
		let mut world = DefaultWorld::new();
		let mut deletions = world.deletions_listener();

		let scene: Handle = world.create(Scene).into();
		let first: Handle = world.create(Name::new("first")).with(SceneNode::under(scene)).into();
		let second: Handle = world.create(Name::new("second")).with(SceneNode::under(scene)).into();

		world.delete(first);
		assert_eq!(deleted(&mut deletions), [first]);

		world.delete(scene);
		assert_eq!(deleted(&mut deletions), [second, scene]);
	}

	#[test]
	fn deletions_posted_to_the_channel_cascade_on_update() {
		let mut world = DefaultWorld::new();
		let mut deletions = world.deletions_listener();
		let mut transforms = world.transforms_channel().listener();
		let mut allocator = bumpalo::Bump::new();

		let scene: Handle = world.create(Scene).into();
		let member: Handle = world.create(Name::new("member")).with(SceneNode::under(scene)).into();

		world.deletes.send(DeleteMessage::new(scene));
		world.update(
			Time::new(crate::time::MediaTime::ZERO, crate::time::MediaTime::ZERO),
			&mut transforms,
			&mut allocator,
		);

		assert_eq!(deleted(&mut deletions), [scene, member]);
	}
}
