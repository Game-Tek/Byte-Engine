//! Standard world composition shared by gameplay, physics, and rendering.
//!
//! Create objects through the factories exposed by [`DefaultWorld`] so
//! downstream systems receive creation and deletion messages. The graphics
//! application updates this world and attaches its listeners to render
//! pipelines.

/// The `DefaultWorld` struct owns the standard entity routes and coordinates transform, physics, and deletion updates.
pub struct DefaultWorld {
	messages: MessageScope,
	transforms: DefaultChannel<TransformationUpdate>,
	deletes: DefaultChannel<DeleteMessage>,
	poses: DefaultChannel<UpdatePose>,
	audio_graph_factory: AudioGraphFactory,

	physics_system: dynabit::World,

	scene_graph: SceneGraph,
	scenes: DefaultListener<CreateMessage<Scene>>,
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
	/// Creates a standalone world with its own message pool and no ticks, so its listeners are future-only.
	///
	/// Applications should use [`Self::with_messages`] so world routes appear in
	/// the application's unified diagnostics.
	pub fn new() -> Self {
		let bus = MessageBus::new(MessageBusConfig {
			ticks: false,
			..MessageBusConfig::default()
		})
		.unwrap_or_else(|error| panic!("{error}"));
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

		let physics_system = dynabit::World::new(body_factory.listener(), deletes.listener());
		let scenes = messages.factory::<Scene>().listener();
		let scene_nodes = messages.factory::<SceneNode>().listener();
		let posted_deletes = deletes.listener();

		Self {
			messages,
			transforms,
			deletes,
			poses,
			audio_graph_factory,

			physics_system,

			scene_graph: SceneGraph::default(),
			scenes,
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
		self.physics_system.update(time, transforms_rx, &self.transforms, allocator);
	}

	pub fn flush_deletions(&mut self) {
		self.physics_system.process_pending_deletions();
	}

	pub fn transforms_channel(&self) -> &DefaultChannel<TransformationUpdate> {
		&self.transforms
	}

	/// Creates a listener for terminal entity deletions, starting at the current tick's first one.
	///
	/// Next, keep the listener with the consuming system and remove matching
	/// state when it receives a [`DeleteMessage`]. Publish deletions through
	/// [`Self::delete`] so inspection diagnostics retire the same handle.
	#[track_caller]
	pub fn deletions_listener(&self) -> DefaultListener<DeleteMessage> {
		self.deletes.listener()
	}

	/// Publishes a deletion for `handle`.
	///
	/// The next [`Self::update`] also deletes every scene member nested under it, children before
	/// their parents. `handle` is published here and is not published again. Consumers created
	/// through [`Self::deletions_listener`] receive every handle and can retire their state.
	pub fn delete(&self, handle: Handle) {
		publish_deletion(&self.deletes, handle);
	}

	/// Links scene members created since the last call into the scene graph.
	///
	/// Scenes are registered first, so a member created in the same batch can attach under its scene.
	fn track_scene_nodes(&mut self) {
		while let Some(creation) = self.scenes.read() {
			self.scene_graph.register(creation.handle());
		}
		while let Some(creation) = self.scene_nodes.read() {
			self.scene_graph.attach(creation.handle(), creation.data().parent());
		}
	}

	/// Publishes scene members nested under deletions received since the last update.
	///
	/// Members are published children before their parents. The deleted handle already has its message,
	/// and a member published here is not expanded again when that message is read.
	fn cascade_posted_deletions(&mut self) {
		self.track_scene_nodes();
		let deletes = &self.deletes;
		while let Some(deletion) = self.posted_deletes.read() {
			self.scene_graph
				.remove_subtree(deletion.into_handle(), |member| publish_deletion(deletes, member));
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
	fn publish(&self, handle: Handle, value: T) {
		self.factory::<T>().derive(handle, value);
	}
}

impl Creator<&mut AudioGraph> for DefaultWorld {
	fn publish(&self, handle: Handle, graph: &mut AudioGraph) {
		self.audio_graph_factory.derive(handle, graph);
	}
}

use crate::{
	application::Time,
	audio::graph::{AudioGraph, AudioGraphFactory},
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageBusConfig, MessageScope},
		publisher::Publisher,
		targeted_message::TargetedMessagePublisher,
	},
	gameplay::{
		Transform,
		scene::{Scene, SceneGraph, SceneNode},
		transform::TransformationUpdate,
	},
	physics::dynabit,
	rendering::{Camera, UpdatePose},
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::listener::Listener;
	use crate::gameplay::{Name, Scene, SceneNode};

	/// Reads every pending deletion handle in publication order.
	fn deleted(deletions: &mut DefaultListener<DeleteMessage>) -> Vec<Handle> {
		std::iter::from_fn(|| deletions.read().map(DeleteMessage::into_handle)).collect()
	}

	/// Runs the update that cascades deletions posted since the last call.
	fn cascade(world: &mut DefaultWorld) {
		let mut transforms = world.transforms_channel().listener();
		let mut allocator = bumpalo::Bump::new();
		world.update(
			Time::new(crate::time::MediaTime::ZERO, crate::time::MediaTime::ZERO),
			&mut transforms,
			&mut allocator,
		);
	}

	#[test]
	fn deleting_a_scene_publishes_it_and_update_cascades_members_children_first() {
		let mut world = DefaultWorld::new();
		let mut deletions = world.deletions_listener();

		let scene: Handle = world.create(Scene).into();
		let tank: Handle = world.create(Name::new("tank")).with(SceneNode::under(scene)).into();
		let turret: Handle = world.create(Name::new("turret")).with(SceneNode::under(tank)).into();
		let rock: Handle = world.create(Name::new("rock")).with(SceneNode::under(scene)).into();
		let outsider: Handle = world.create(Name::new("outsider")).into();

		world.delete(scene);
		assert_eq!(deleted(&mut deletions), [scene]);

		cascade(&mut world);
		let deleted = deleted(&mut deletions);
		assert_eq!(deleted.len(), 3);
		let position = |handle| deleted.iter().position(|&h| h == handle).expect("member deleted");
		assert!(position(turret) < position(tank));
		assert!(deleted.contains(&rock));
		assert!(!deleted.contains(&scene));
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
		cascade(&mut world);
		assert_eq!(deleted(&mut deletions), [first]);

		world.delete(scene);
		cascade(&mut world);
		assert_eq!(deleted(&mut deletions), [scene, second]);
	}

	#[test]
	fn deletions_posted_to_the_channel_cascade_on_update() {
		let mut world = DefaultWorld::new();
		let mut deletions = world.deletions_listener();
		let scene: Handle = world.create(Scene).into();
		let member: Handle = world.create(Name::new("member")).with(SceneNode::under(scene)).into();

		world.deletes.send(DeleteMessage::new(scene));
		cascade(&mut world);

		assert_eq!(deleted(&mut deletions), [scene, member]);
	}

	#[test]
	#[cfg(debug_assertions)]
	#[should_panic(expected = "stale scene handle")]
	fn attaching_under_a_deleted_member_panics() {
		let mut world = DefaultWorld::new();
		let scene: Handle = world.create(Scene).into();
		let member: Handle = world.create(Name::new("member")).with(SceneNode::under(scene)).into();
		world.delete(member);
		cascade(&mut world);
		world.create(Name::new("late")).with(SceneNode::under(member));
		cascade(&mut world);
	}
}
