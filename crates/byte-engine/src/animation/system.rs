//! The animation system an application steps on its world scope: it plays every animator's graph and posts each
//! one's pose and root motion, as the animation reference's Animators section specifies.

use crate::{
	animation::{
		graph::{
			AnimationGraph, AnimationGraphPlayer, AnimationPool, AnimationRequest, AnimationStateId, LookChainSettings,
			LookTarget, RootMotionSettings,
		},
		root_motion::RootMotionDelta,
	},
	application::Time,
	core::{
		alley::Alley,
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::{DeleteMessage, Message},
		message_bus::MessageScope,
		publisher::Publisher,
		targeted_message::TargetedMessagePublisher,
	},
	gameplay::World as _,
};

/// The `Animator` struct is the request to play one graph on one entity, created through the world under the handle
/// of the entity it poses, usually a skinned renderable.
///
/// Build it with [`Self::new`] and the graph the [`AnimationSystem`] has [registered](AnimationSystem::register),
/// then create it with the entity, as in `world.create(mesh).with(Animator::new(&graph))`. Next, send the states to
/// play with `world.set(request).on(handle)` and read the root motion the system posts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animator {
	graph: u64,
	root_motion: Option<RootMotionSettings<'static>>,
	look_chain: Option<LookChainSettings<'static>>,
}

impl Animator {
	/// Plays `graph`, which the system must have registered.
	pub fn new(graph: &AnimationGraph) -> Self {
		Self {
			graph: graph.id(),
			root_motion: None,
			look_chain: None,
		}
	}

	/// Extracts root motion from the pose as `settings` say; see [`RootMotionSettings`].
	pub const fn with_root_motion(mut self, settings: RootMotionSettings<'static>) -> Self {
		self.root_motion = Some(settings);
		self
	}

	/// Turns a chain of nodes toward the look targets sent to the entity; see [`LookChainSettings`].
	pub const fn with_look_chain(mut self, settings: LookChainSettings<'static>) -> Self {
		self.look_chain = Some(settings);
		self
	}
}

/// The `RootMotionUpdate` struct carries the root motion a step extracted for one animator, for the code that owns
/// the entity to move it by. The system never moves an entity itself.
///
/// Listen on [`World::root_motion_channel`](crate::gameplay::World::root_motion_channel) and read the updates right
/// after [`AnimationSystem::step`], so the transform you write matches the pose that frame draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotionUpdate {
	handle: Handle,
	delta: RootMotionDelta,
	state: Option<AnimationStateId>,
}

impl RootMotionUpdate {
	/// Returns the entity the animator poses.
	pub const fn handle(&self) -> Handle {
		self.handle
	}

	/// Returns the step's root motion delta, in the entity's object space; the identity for an animator without root
	/// motion settings.
	pub const fn delta(&self) -> RootMotionDelta {
		self.delta
	}

	/// Returns the state playing after the step, so a route taken this step, like a turn clip handing over, can be
	/// acted on with the pose it produced.
	pub const fn state(&self) -> Option<AnimationStateId> {
		self.state
	}
}

impl Message for RootMotionUpdate {}

// Requests and look targets travel as targeted messages on the scope, so `world.set(request).on(handle)` addresses
// an animator the way `world.set(transform).on(handle)` places an entity.
impl Publisher<CreateMessage<AnimationRequest>> for MessageScope {
	fn publish(&self, message: CreateMessage<AnimationRequest>) {
		self.channel().send(message);
	}
}

impl TargetedMessagePublisher<AnimationRequest> for MessageScope {
	type Message = CreateMessage<AnimationRequest>;
}

impl Publisher<CreateMessage<Option<LookTarget>>> for MessageScope {
	fn publish(&self, message: CreateMessage<Option<LookTarget>>) {
		self.channel().send(message);
	}
}

impl TargetedMessagePublisher<Option<LookTarget>> for MessageScope {
	type Message = CreateMessage<Option<LookTarget>>;
}

/// The `AnimationSystem` struct plays the animators of a world scope: each step it adopts new ones, applies the
/// latest request and look target sent to each, advances them together, and posts every pose and root motion.
///
/// Create it with [`Self::new`] before the animators it must play and [register](Self::register) their graphs, then
/// call [`Self::step`] once per frame after your requests for that frame and before the systems that read
/// transforms, such as [`Physics::step`](crate::physics::Physics::step). Between steps, [`Self::player`] answers
/// what each animator plays.
pub struct AnimationSystem<'graph> {
	pool: AnimationPool,
	graphs: Vec<&'graph AnimationGraph>,
	animators: DefaultListener<CreateMessage<Animator>>,
	requests: DefaultListener<CreateMessage<AnimationRequest>>,
	looks: DefaultListener<CreateMessage<Option<LookTarget>>>,
	deletions: DefaultListener<DeleteMessage>,
	#[cfg(feature = "headed")]
	poses: DefaultChannel<crate::rendering::UpdatePose>,
	root_motion: DefaultChannel<RootMotionUpdate>,
	/// The playing animators' entities, in the order of `players`.
	handles: Vec<Handle>,
	players: Vec<AnimationGraphPlayer<'graph>>,
}

impl<'graph> AnimationSystem<'graph> {
	/// Creates the system for the animators created in `scope` from now on, playing clips from `pool`.
	pub fn new(scope: &MessageScope, pool: AnimationPool) -> Self {
		Self {
			pool,
			graphs: Vec::new(),
			animators: scope.factory().listener(),
			requests: scope.factory().listener(),
			looks: scope.factory().listener(),
			deletions: scope.deletions_listener(),
			#[cfg(feature = "headed")]
			poses: scope.poses_channel(),
			root_motion: scope.root_motion_channel(),
			handles: Vec::new(),
			players: Vec::new(),
		}
	}

	/// Makes `graph` available to the animators that name it. Register a graph before creating them; an animator
	/// whose graph is unknown is reported and never plays.
	pub fn register(&mut self, graph: &'graph AnimationGraph) {
		if !self.graphs.iter().any(|known| known.id() == graph.id()) {
			self.graphs.push(graph);
		}
	}

	/// Advances every animator by the frame and posts its pose and root motion before returning.
	///
	/// Deleted animators leave and created ones join first; then the latest request and look target sent to each
	/// apply; then the players advance together on `alley`'s lanes. No other dispatch of `alley` may be running.
	pub fn step(&mut self, time: Time, alley: &mut Alley) {
		self.retire_deleted();
		self.adopt_created();
		self.apply_requests();
		// Submit queued clip loads and admit completed animations once for this step.
		self.pool.update();
		if let Err(error) = self.pool.advance_players(&mut self.players, time.delta(), alley) {
			log::error!("An animator was rejected and posts nothing: {error}");
		}
		for (handle, player) in self.handles.iter().zip(&self.players) {
			let Some(pose) = player.pose() else {
				continue;
			};
			#[cfg(feature = "headed")]
			self.poses
				.send(crate::rendering::UpdatePose::new(*handle, pose.global_pose.to_vec()));
			self.root_motion.send(RootMotionUpdate {
				handle: *handle,
				delta: pose.root_motion,
				state: pose.state,
			});
		}
	}

	/// Returns the player of the animator on `handle`, for what it plays, what it waits for, and how long until a
	/// route, as of the last step.
	pub fn player(&self, handle: Handle) -> Option<&AnimationGraphPlayer<'graph>> {
		self.index_of(handle).map(|index| &self.players[index])
	}

	/// Returns the player of the animator on `handle` for settings that belong to the application, such as a data
	/// stream.
	pub fn player_mut(&mut self, handle: Handle) -> Option<&mut AnimationGraphPlayer<'graph>> {
		self.index_of(handle).map(|index| &mut self.players[index])
	}

	fn index_of(&self, handle: Handle) -> Option<usize> {
		self.handles.iter().position(|&known| known == handle)
	}

	/// Removes the animators of deleted entities.
	fn retire_deleted(&mut self) {
		while let Some(deletion) = self.deletions.read() {
			if let Some(index) = self.index_of(*deletion.handle()) {
				self.handles.swap_remove(index);
				self.players.swap_remove(index);
			}
		}
	}

	/// Creates a player for every animator created since the last step; one created again on a handle replaces the
	/// previous one.
	fn adopt_created(&mut self) {
		while let Some(created) = self.animators.read() {
			let animator = *created.data();
			let Some(graph) = self.graphs.iter().copied().find(|graph| graph.id() == animator.graph) else {
				log::error!(
					"An animator names a graph the animation system does not know. The most likely cause is creating the animator before registering its graph with AnimationSystem::register."
				);
				continue;
			};
			let mut player = self.pool.create_player(graph, animator.root_motion);
			if let Some(look_chain) = animator.look_chain {
				player = player.with_look_chain(look_chain);
			}
			match self.index_of(created.handle()) {
				Some(index) => self.players[index] = player,
				None => {
					self.handles.push(created.handle());
					self.players.push(player);
				}
			}
		}
	}

	/// Applies the requests and look targets sent since the last step; the last of each per animator wins.
	fn apply_requests(&mut self) {
		while let Some(request) = self.requests.read() {
			if let Some(index) = self.index_of(request.handle())
				&& let Err(error) = self.players[index].request(*request.data())
			{
				log::warn!("An animation request was discarded: {error}");
			}
		}
		while let Some(look) = self.looks.read() {
			if let Some(index) = self.index_of(look.handle()) {
				self.players[index].set_look(*look.data());
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, TranslationCurve},
			skeleton::{LocalTransform, Skeleton},
		},
	};

	use super::*;
	use crate::{
		MediaTime,
		animation::{
			graph::{AnimationClip, AnimationPool, RootMotionSettings},
			packed::PackedAnimationData,
			test_node,
		},
		core::{Creator as _, message_bus::MessageBus, targeted_message::MessageTargeter as _},
		math,
	};

	/// Packs a one-node clip that moves its root along x at `speed` for a second.
	fn moving_clip(name: &str, speed: f32) -> PackedAnimationData {
		PackedAnimationData::from_resource(Animation {
			name: Some(name.into()),
			skeleton: Reference::in_memory(
				"test.skeleton",
				Skeleton {
					nodes: vec![test_node(Some("root"), None, LocalTransform::identity())],
				},
			),
			duration: 1.0,
			frame_rate: Some(10.0),
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(TranslationCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![math::Vector::new(0.0, 0.0, 0.0), math::Vector::new(speed, 0.0, 0.0)],
				}),
				rotation: None,
				scale: None,
			}],
		})
	}

	fn frame(n: u32) -> Time {
		Time::new(MediaTime::from_millis(100 * i64::from(n)), MediaTime::from_millis(100))
	}

	#[test]
	fn system_plays_animators_created_after_it_and_posts_their_root_motion() {
		let bus = MessageBus::default();
		let scope = bus.new_scope("world");
		let idle_clip = moving_clip("idle", 1.0);
		let walk_clip = moving_clip("walk", 2.0);
		let mut pool = AnimationPool::detached(idle_clip.resident_bytes() + walk_clip.resident_bytes());
		pool.admit("idle.animation".into(), idle_clip);
		pool.admit("walk.animation".into(), walk_clip);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).anytime(MediaTime::ZERO);
		walk.to(idle).anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut alley = Alley::new();

		let mut system = AnimationSystem::new(&scope, pool);
		system.register(&graph);
		let mut root_motion = scope.root_motion_channel().listener();
		let entity = scope
			.create(Animator::new(&graph).with_root_motion(RootMotionSettings::full("root")))
			.handle();

		// The first step adopts the animator and starts its initial clip; the next posts a frame of root motion.
		system.step(frame(1), &mut alley);
		assert_eq!(system.player(entity).and_then(|player| player.state()), Some(idle.id()));
		system.step(frame(2), &mut alley);
		let update = root_motion.to_vec();
		let last = update.last().expect("the step posts root motion");
		assert_eq!(last.handle(), entity);
		assert_eq!(last.state(), Some(idle.id()));
		math::assert_float_eq!(last.delta().translation.x(), 0.1);

		// A request sent to the handle is applied by the next step, and the root motion follows the new clip.
		scope.set(AnimationRequest::from(walk.id())).on(entity);
		system.step(frame(3), &mut alley);
		assert_eq!(system.player(entity).and_then(|player| player.state()), Some(walk.id()));
		system.step(frame(4), &mut alley);
		let last = root_motion.to_vec().pop().expect("the step posts root motion");
		assert_eq!(last.state(), Some(walk.id()));
		math::assert_float_eq!(last.delta().translation.x(), 0.2);

		// An animator created again on the handle starts over; a deleted entity leaves and posts nothing more.
		scope.factory::<Animator>().derive(entity, Animator::new(&graph));
		system.step(frame(5), &mut alley);
		assert_eq!(system.player(entity).and_then(|player| player.state()), Some(idle.id()));
		root_motion.to_vec();
		scope.delete(entity);
		system.step(frame(6), &mut alley);
		assert!(system.player(entity).is_none());
		assert!(root_motion.to_vec().iter().all(|update| update.handle() != entity));
	}
}
