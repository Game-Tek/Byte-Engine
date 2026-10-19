//! The world routes shared by gameplay, physics, and rendering.
//!
//! The world is a [`MessageScope`]: entities are created through its factories with
//! [`Creator::create`](crate::core::factory::Creator::create), so every system that listens to the scope
//! receives the same lifecycle messages. The [`World`] trait names the routes those systems share: transforms,
//! deletions, poses, and material parameters. Systems such as [`SceneGraph`](crate::gameplay::SceneGraph) and
//! [`Physics`](crate::physics::Physics) are created on the scope and stepped by the loop that owns them, in
//! the order that loop chooses. Without the `headed` feature the scope keeps every route but poses and material
//! parameters, so servers run the same gameplay code with nothing listening.

/// The `World` trait names the entity routes of a world scope so systems agree on where transforms and deletions travel.
///
/// Implemented for [`MessageScope`]. Import it to reach the routes from an application's
/// [`world`](crate::application::graphics::GraphicsApplication::world) scope.
pub trait World {
	/// Returns the route authored and simulated transforms travel on.
	fn transforms_channel(&self) -> DefaultChannel<TransformationUpdate>;

	/// Creates a listener for terminal entity deletions, starting at the current tick's first one.
	///
	/// Next, keep the listener with the consuming system and remove matching state when it receives a
	/// [`DeleteMessage`]. Publish deletions through [`Self::delete`] so inspection diagnostics retire the same
	/// handle.
	fn deletions_listener(&self) -> DefaultListener<DeleteMessage>;

	/// Publishes a deletion for `handle`.
	///
	/// A [`SceneGraph::cascade`](crate::gameplay::SceneGraph::cascade) also deletes every scene member nested
	/// under it, children before their parents. `handle` is published here and is not published again.
	/// Consumers created through [`Self::deletions_listener`] receive every handle and can retire their state.
	fn delete(&self, handle: Handle);

	/// Returns the route skeletal poses travel on.
	#[cfg(feature = "headed")]
	fn poses_channel(&self) -> DefaultChannel<UpdatePose>;

	/// Returns the route the animation system posts each animator's root motion on. See [`RootMotionUpdate`].
	fn root_motion_channel(&self) -> DefaultChannel<RootMotionUpdate>;

	/// Returns the route dynamic material parameter changes travel on. See [`SetMaterialParameter`].
	#[cfg(feature = "headed")]
	fn material_parameters_channel(&self) -> DefaultChannel<SetMaterialParameter>;
}

impl World for MessageScope {
	fn transforms_channel(&self) -> DefaultChannel<TransformationUpdate> {
		self.channel()
	}

	#[track_caller]
	fn deletions_listener(&self) -> DefaultListener<DeleteMessage> {
		self.channel::<DeleteMessage>().listener()
	}

	fn delete(&self, handle: Handle) {
		publish_deletion(&self.channel(), handle);
	}

	#[cfg(feature = "headed")]
	fn poses_channel(&self) -> DefaultChannel<UpdatePose> {
		self.channel()
	}

	fn root_motion_channel(&self) -> DefaultChannel<RootMotionUpdate> {
		self.channel()
	}

	#[cfg(feature = "headed")]
	fn material_parameters_channel(&self) -> DefaultChannel<SetMaterialParameter> {
		self.channel()
	}
}

/// A scope publishes authored transforms on its transform route, so `scope.set(transform).on(handle)` places an
/// entity.
impl Publisher<TransformationUpdate> for MessageScope {
	fn publish(&self, message: TransformationUpdate) {
		self.transforms_channel().send(message);
	}
}

impl TargetedMessagePublisher<Transform> for MessageScope {
	type Message = TransformationUpdate;
}

/// Sends one deletion and removes the handle from inspection diagnostics.
pub(crate) fn publish_deletion(deletes: &DefaultChannel<DeleteMessage>, handle: Handle) {
	deletes.send(DeleteMessage::new(handle));
	deletes.forget_entity(handle);
}

use crate::animation::system::RootMotionUpdate;
#[cfg(feature = "headed")]
use crate::rendering::{SetMaterialParameter, UpdatePose};
use crate::{
	core::{
		channel::{Channel, DefaultChannel},
		factory::Handle,
		listener::DefaultListener,
		message::DeleteMessage,
		message_bus::MessageScope,
		publisher::Publisher,
		targeted_message::TargetedMessagePublisher,
	},
	gameplay::{Transform, transform::TransformationUpdate},
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::{
		alley::Alley,
		factory::Creator as _,
		listener::Listener,
		message_bus::{MessageBus, MessageBusConfig},
	};
	use crate::gameplay::{Name, Scene, SceneGraph, SceneNode};

	/// Reads every pending deletion handle in publication order.
	fn deleted(deletions: &mut DefaultListener<DeleteMessage>) -> Vec<Handle> {
		std::iter::from_fn(|| deletions.read().map(DeleteMessage::into_handle)).collect()
	}

	#[test]
	fn deleting_a_scene_publishes_it_and_the_cascade_publishes_members_children_first() {
		let world = MessageScope::standalone("world");
		let mut scenes = SceneGraph::new(&world);
		let mut deletions = world.deletions_listener();

		let scene: Handle = world.create(Scene).into();
		let tank: Handle = world.create(Name::new("tank")).with(SceneNode::under(scene)).into();
		let turret: Handle = world.create(Name::new("turret")).with(SceneNode::under(tank)).into();
		let rock: Handle = world.create(Name::new("rock")).with(SceneNode::under(scene)).into();
		let outsider: Handle = world.create(Name::new("outsider")).into();

		world.delete(scene);
		assert_eq!(deleted(&mut deletions), [scene]);

		scenes.cascade();
		let deleted = deleted(&mut deletions);
		assert_eq!(deleted.len(), 3);
		let position = |handle| deleted.iter().position(|&h| h == handle).expect("member deleted");
		assert!(position(turret) < position(tank));
		assert!(deleted.contains(&rock));
		assert!(!deleted.contains(&scene));
		assert!(!deleted.contains(&outsider));
	}

	#[test]
	fn a_ray_query_made_in_one_tick_is_answered_to_its_client_in_the_next() {
		use math::{Point, Vector};

		use crate::application::Time;
		use crate::gameplay::Transform;
		use crate::physics::{Body, BodyTypes, Physics, RayQuery, RayQueryResponse, Shapes};

		// A ticking bus, run the way the application runs it: a tick begins, application code runs, then physics
		// steps.
		let bus = MessageBus::new(MessageBusConfig::new(1 << 20)).expect("a valid bus");
		let world = bus.new_scope("world");
		let mut physics = Physics::new(&world, &[][..] as &[crate::application::Parameter]);
		let mut responses = world.factory::<RayQueryResponse>().listener();
		let mut alley = Alley::with_parallelism(1);
		let step = crate::time::MediaTime::from_seconds_f32(1.0 / 60.0);

		bus.begin_tick();
		// The ground's top is at height one.
		let ground: Handle = world
			.create(Body::new(BodyTypes::Static, Shapes::cube(Vector::new(4.0, 1.0, 4.0))))
			.into();
		world
			.transforms_channel()
			.send(TransformationUpdate::new(ground, Transform::identity()));
		let query: Handle = world
			.create(RayQuery::segment(Point::new(0.0, 10.0, 0.0), Point::new(0.0, -10.0, 0.0)).expect("a segment"))
			.into();
		physics.step(Time::new(step, step), &mut alley);
		physics.flush_deletions();

		bus.begin_tick();
		let response = responses.read().expect("the response arrives in the next tick");
		assert_eq!(response.data().query(), query);
		assert_eq!(response.data().hit().map(|hit| hit.body()), Some(ground));
		assert!(responses.read().is_none());
	}

	#[test]
	fn deleting_a_member_detaches_it_from_its_scene() {
		let world = MessageScope::standalone("world");
		let mut scenes = SceneGraph::new(&world);
		let mut deletions = world.deletions_listener();

		let scene: Handle = world.create(Scene).into();
		let first: Handle = world.create(Name::new("first")).with(SceneNode::under(scene)).into();
		let second: Handle = world.create(Name::new("second")).with(SceneNode::under(scene)).into();

		world.delete(first);
		scenes.cascade();
		assert_eq!(deleted(&mut deletions), [first]);

		world.delete(scene);
		scenes.cascade();
		assert_eq!(deleted(&mut deletions), [scene, second]);
	}

	#[test]
	fn deletions_posted_to_the_channel_cascade() {
		let world = MessageScope::standalone("world");
		let mut scenes = SceneGraph::new(&world);
		let mut deletions = world.deletions_listener();
		let scene: Handle = world.create(Scene).into();
		let member: Handle = world.create(Name::new("member")).with(SceneNode::under(scene)).into();

		world.channel::<DeleteMessage>().send(DeleteMessage::new(scene));
		scenes.cascade();

		assert_eq!(deleted(&mut deletions), [scene, member]);
	}

	#[test]
	#[cfg(debug_assertions)]
	#[should_panic(expected = "stale scene handle")]
	fn attaching_under_a_deleted_member_panics() {
		let world = MessageScope::standalone("world");
		let mut scenes = SceneGraph::new(&world);
		let scene: Handle = world.create(Scene).into();
		let member: Handle = world.create(Name::new("member")).with(SceneNode::under(scene)).into();
		world.delete(member);
		scenes.cascade();
		world.create(Name::new("late")).with(SceneNode::under(member));
		scenes.cascade();
	}
}
