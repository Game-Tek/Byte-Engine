//! The animation system an application steps on its world scope: it plays every animator's graph and posts each
//! one's pose and root motion, as the animation reference's Animators section specifies.

#[cfg(feature = "headed")]
use crate::rendering::UpdatePose;
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
};

/// The `Animator` struct is the request to play one graph on one entity, created through the world under the handle
/// of the entity it poses, usually a skinned renderable.
///
/// Build it with [`Self::new`] and the ID [`AnimationSystem::register`] returned for its graph, then create it with
/// the entity, as in `world.create(mesh).with(Animator::new(graph))`. Next, send the states to play with
/// `world.set(request).on(handle)` and read the root motion the system posts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animator {
	graph: u64,
	root_motion: Option<RootMotionSettings<'static>>,
	look_chain: Option<LookChainSettings<'static>>,
}

impl Animator {
	/// Plays the graph the system registered under `graph`.
	pub fn new(graph: AnimationGraphId) -> Self {
		Self {
			graph: graph.0,
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

/// The `AnimationGraphId` struct names a graph an [`AnimationSystem`] owns, so animators and the application refer to
/// it without holding it.
///
/// Get one from [`AnimationSystem::register`], then pass it to [`Animator::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnimationGraphId(u64);

/// The `RootMotionUpdate` struct carries the root motion a step extracted for one animator, for the code that owns
/// the entity to move it by. The system never moves an entity itself.
///
/// Listen on the scope's `channel::<RootMotionUpdate>()` and read the updates right
/// after [`AnimationSystem::step`], so the transform you write matches the pose that frame draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotionUpdate {
	handle: Handle,
	delta: RootMotionDelta,
	state: Option<AnimationStateId>,
	waiting_for: Option<AnimationStateId>,
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

	/// Returns the requested state whose route waits for a moment of the playing clip, such as a frame, or `None`
	/// when no route waits.
	pub const fn waiting_for(&self) -> Option<AnimationStateId> {
		self.waiting_for
	}
}

impl Message for RootMotionUpdate {}

// Requests and look targets travel as targeted messages on the scope, so `world.set(request).on(handle)` addresses
// an animator.
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
/// transforms, such as [`Physics::step`](crate::physics::Physics::step). Read what each animator plays, and the root
/// motion to move it by, from the [`RootMotionUpdate`] each step posts.
pub struct AnimationSystem {
	pool: AnimationPool,
	/// The graphs the system plays, which it lends its players every step.
	graphs: Vec<AnimationGraph>,
	animators: DefaultListener<CreateMessage<Animator>>,
	stream_creations: DefaultListener<CreateMessage<crate::inspector::DebugDataStreamWriter>>,
	/// DDS subscriptions outlive animator replacement and can arrive before an animator.
	streams: Vec<(Handle, crate::inspector::DebugDataStreamWriter)>,
	requests: DefaultListener<CreateMessage<AnimationRequest>>,
	looks: DefaultListener<CreateMessage<Option<LookTarget>>>,
	deletions: DefaultListener<DeleteMessage>,
	#[cfg(feature = "headed")]
	poses: DefaultChannel<crate::rendering::UpdatePose>,
	root_motion: DefaultChannel<RootMotionUpdate>,
	/// The playing animators' entities, in the order of `players`.
	handles: Vec<Handle>,
	players: Vec<AnimationGraphPlayer>,
}

impl AnimationSystem {
	/// Creates the system for the animators created in `scope` from now on, playing clips from `pool`.
	pub fn new(scope: &MessageScope, pool: AnimationPool) -> Self {
		Self {
			pool,
			graphs: Vec::new(),
			animators: scope.factory().listener(),
			stream_creations: scope.factory().listener(),
			streams: Vec::new(),
			requests: scope.factory().listener(),
			looks: scope.factory().listener(),
			deletions: scope.channel::<DeleteMessage>().listener(),
			#[cfg(feature = "headed")]
			poses: scope.channel::<UpdatePose>(),
			root_motion: scope.channel::<RootMotionUpdate>(),
			handles: Vec::new(),
			players: Vec::new(),
		}
	}

	/// Takes `graph` for the animators that name it, and returns its ID for [`Animator::new`]. Register a graph
	/// before creating its animators; an animator whose graph is unknown is reported and never plays.
	///
	/// The system keeps the graph for as long as it lives, and its players share it, so nothing outside needs to.
	pub fn register(&mut self, graph: AnimationGraph) -> AnimationGraphId {
		let id = AnimationGraphId(graph.id());
		if !self.graphs.iter().any(|known| known.id() == id.0) {
			self.graphs.push(graph);
		}
		id
	}

	/// Advances every animator by the frame and posts its pose and root motion before returning.
	///
	/// Deleted animators leave and created ones join first; then the latest request and look target sent to each
	/// apply; then the players advance together on `alley`'s lanes. No other dispatch of `alley` may be running.
	pub fn step(&mut self, time: Time, alley: &mut Alley) {
		self.adopt_streams();
		self.retire_deleted();
		self.adopt_created();
		self.apply_requests();
		// Submit queued clip loads and admit completed animations once for this step.
		self.pool.update();
		if let Err(error) = self
			.pool
			.advance_players(&mut self.players, &self.graphs, time.delta(), alley)
		{
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
				waiting_for: player.waiting_for(),
			});
		}
	}

	fn index_of(&self, handle: Handle) -> Option<usize> {
		self.handles.iter().position(|&known| known == handle)
	}

	/// Removes the animators of deleted entities.
	fn retire_deleted(&mut self) {
		while let Some(deletion) = self.deletions.read() {
			let deleted = *deletion.handle();
			if let Some(index) = self.index_of(deleted) {
				self.handles.swap_remove(index);
				self.players.swap_remove(index);
			}
			// A DDS has its own entity lifetime; deleting it detaches its writer even when its target remains alive.
			let mut index = 0;
			while index < self.streams.len() {
				let (owner, stream) = &self.streams[index];
				if *owner == deleted || stream.entity() == deleted {
					let (_, removed) = self.streams.remove(index);
					if let Some(player) = self.index_of(removed.entity()) {
						let replacement = self
							.streams
							.iter()
							.rev()
							.find(|(_, stream)| stream.entity() == removed.entity())
							.map(|(_, stream)| stream.clone());
						self.players[player].set_stream(replacement);
					}
				} else {
					index += 1;
				}
			}
		}
	}

	/// Adopts DDS writers independently of animator arrival, attaching them to players already present.
	fn adopt_streams(&mut self) {
		while let Some(created) = self.stream_creations.read() {
			let owner = created.handle();
			let stream = created.data().clone();
			if let Some(index) = self.streams.iter().position(|(known, _)| *known == owner) {
				let previous = self.streams.remove(index).1;
				if previous.entity() != stream.entity()
					&& let Some(player) = self.index_of(previous.entity())
				{
					self.players[player].set_stream(None);
				}
			}
			if let Some(index) = self.index_of(stream.entity()) {
				self.players[index].set_stream(Some(stream.clone()));
			}
			self.streams.push((owner, stream));
		}
	}

	/// Creates a player for every animator created since the last step; one created again on a handle replaces the
	/// previous one.
	fn adopt_created(&mut self) {
		while let Some(created) = self.animators.read() {
			let animator = *created.data();
			let Some(graph) = self.graphs.iter().find(|graph| graph.id() == animator.graph) else {
				log::error!(
					"An animator names a graph the animation system does not know. The most likely cause is creating the animator before registering its graph with AnimationSystem::register."
				);
				continue;
			};
			let mut player = self.pool.create_player(graph, animator.root_motion);
			if let Some(look_chain) = animator.look_chain {
				player = player.with_look_chain(look_chain);
			}
			if let Some((_, stream)) = self
				.streams
				.iter()
				.rev()
				.find(|(_, stream)| stream.entity() == created.handle())
			{
				player.set_stream(Some(stream.clone()));
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
	fn one_jump_request_waits_for_takeoff_plays_the_clip_and_lands() {
		let scope = MessageScope::standalone("one jump request");
		let walking_clip = moving_clip("walking", 1.0);
		let jump_clip = moving_clip("jumping", 2.0);
		let mut pool = AnimationPool::detached(walking_clip.resident_bytes() + jump_clip.resident_bytes());
		pool.admit("walking.animation".into(), walking_clip);
		pool.admit("jumping.animation".into(), jump_clip);
		let builder = AnimationGraph::builder();
		let walking = builder.state("walking").with(AnimationClip::looping("walking.animation"));
		let landed = builder.state("landed").with(AnimationClip::looping("walking.animation"));
		let jumping = walking
			.to(landed)
			.at_frame(5)
			.with(AnimationClip::once("jumping.animation"))
			.when(MediaTime::ZERO)
			.id();
		let mut system = AnimationSystem::new(&scope, pool);
		let graph = system.register(builder.build(walking).unwrap());
		let entity = scope.create(Animator::new(graph)).handle();
		let mut motion = scope.channel::<RootMotionUpdate>().listener();
		let mut alley = Alley::new();
		system.step(frame(1), &mut alley);
		system.step(frame(2), &mut alley);
		motion.to_vec();

		// Request once. Subsequent steps receive no requests from gameplay.
		scope.set(AnimationRequest::from(landed.id())).on(entity);
		system.step(frame(3), &mut alley);
		let waiting = motion.to_vec().pop().unwrap();
		assert_eq!(waiting.state(), Some(walking.id()));
		assert_eq!(waiting.waiting_for(), Some(landed.id()));
		let mut saw_jump = false;
		let mut last = waiting;
		for tick in 4..25 {
			system.step(frame(tick), &mut alley);
			for update in motion.to_vec() {
				saw_jump |= update.state() == Some(jumping);
				last = update;
			}
		}
		assert!(saw_jump);
		assert_eq!(last.state(), Some(landed.id()));
		assert_eq!(last.waiting_for(), None);
	}

	#[test]
	fn world_streams_attach_before_or_after_animators_and_follow_their_lifetime() {
		use std::{sync::Arc, time::Instant};

		use crate::inspector::{DebugDataStream, DebugDataStreams, StreamOutput};

		// A DDS may precede its animator by a step, accompany it, or join an already playing animator.
		for arrival in 0..3 {
			let scope = MessageScope::standalone("animation streams");
			let clip = moving_clip("idle", 1.0);
			let mut pool = AnimationPool::detached(clip.resident_bytes());
			pool.admit("idle.animation".into(), clip);
			let builder = AnimationGraph::builder();
			let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
			let mut system = AnimationSystem::new(&scope, pool);
			let graph = system.register(builder.build(idle).expect("graph builds"));
			let entity = Handle::new();
			let path = std::env::temp_dir().join(format!("animation-dds-{}-{}.ndjson", std::process::id(), entity.id()));
			let output = Arc::new(StreamOutput::open(path.to_str().unwrap(), false, Instant::now()).unwrap());
			let mut debug = DebugDataStreams::new(&scope, Some(output));
			let mut alley = Alley::new();
			let mut publish_stream = || {
				let created = scope.create(DebugDataStream::new(entity, "Character"));
				debug.update();
				created.handle()
			};
			if arrival == 0 {
				publish_stream();
				system.step(frame(0), &mut alley);
				assert!(std::fs::read_to_string(&path).unwrap().is_empty());
			}
			scope.factory::<Animator>().derive(entity, Animator::new(graph));
			if arrival == 2 {
				system.step(frame(0), &mut alley);
				assert!(std::fs::read_to_string(&path).unwrap().is_empty());
			}
			if arrival != 0 {
				publish_stream();
			}
			system.step(frame(1), &mut alley);
			let records = std::fs::read_to_string(&path).unwrap();
			let record: serde_json::Value = serde_json::from_str(records.trim()).unwrap();
			assert_eq!(record["entity"], entity.id());
			assert_eq!(record["name"], "Character");
			assert_eq!(record["stream"], "animation");

			// Replacing the animator keeps the DDS; deleting the entity ends the subscription.
			scope.factory::<Animator>().derive(entity, Animator::new(graph));
			system.step(frame(2), &mut alley);
			assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
			scope.channel::<DeleteMessage>().send(DeleteMessage::new(entity));
			system.step(frame(3), &mut alley);
			scope.factory::<Animator>().derive(entity, Animator::new(graph));
			system.step(frame(4), &mut alley);
			assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
			// A fresh DDS can be removed independently of the still-playing animator.
			let active = scope.create(DebugDataStream::new(entity, "Replacement")).handle();
			debug.update();
			system.step(frame(5), &mut alley);
			assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
			scope.channel::<DeleteMessage>().send(DeleteMessage::new(active));
			system.step(frame(6), &mut alley);
			assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
			// A DDS created and deleted before output adoption must never begin reporting.
			let cancelled = scope.create(DebugDataStream::new(entity, "Cancelled")).handle();
			scope.channel::<DeleteMessage>().send(DeleteMessage::new(cancelled));
			debug.update();
			system.step(frame(7), &mut alley);
			assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
			std::fs::remove_file(path).unwrap();
		}
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
		let graph = system.register(graph);
		let mut root_motion = scope.channel::<RootMotionUpdate>().listener();
		let entity = scope
			.create(Animator::new(graph).with_root_motion(RootMotionSettings::full("root")))
			.handle();

		// The first step adopts the animator and starts its initial clip; the next posts a frame of root motion.
		system.step(frame(1), &mut alley);
		system.step(frame(2), &mut alley);
		let update = root_motion.to_vec();
		let last = update.last().expect("the step posts root motion");
		assert_eq!(last.handle(), entity);
		assert_eq!(last.state(), Some(idle.id()));
		math::assert_float_eq!(last.delta().translation.x(), 0.1);

		// A request sent to the handle is applied by the next step, and the root motion follows the new clip.
		scope.set(AnimationRequest::from(walk.id())).on(entity);
		system.step(frame(3), &mut alley);
		system.step(frame(4), &mut alley);
		let last = root_motion.to_vec().pop().expect("the step posts root motion");
		assert_eq!(last.state(), Some(walk.id()));
		math::assert_float_eq!(last.delta().translation.x(), 0.2);

		// An animator created again on the handle starts over; a deleted entity leaves and posts nothing more.
		scope.factory::<Animator>().derive(entity, Animator::new(graph));
		system.step(frame(5), &mut alley);
		system.step(frame(6), &mut alley);
		assert_eq!(root_motion.to_vec().last().and_then(|update| update.state()), Some(idle.id()));
		scope.channel::<DeleteMessage>().send(DeleteMessage::new(entity));
		system.step(frame(7), &mut alley);
		assert!(root_motion.to_vec().iter().all(|update| update.handle() != entity));
	}
}
