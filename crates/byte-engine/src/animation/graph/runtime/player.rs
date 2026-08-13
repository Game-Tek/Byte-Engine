//! Per-skeleton graph playback, pose evaluation, and root-motion extraction.

use super::*;

struct RuntimeClip {
	state: AnimationStateId,
	lease: AnimationLease,
	pose_map: SkeletonPoseMap,
	playback: AnimationPlayback,
	duration: f32,
	time_seconds: f32,
}

impl RuntimeClip {
	fn new(
		state: AnimationStateId,
		lease: AnimationLease,
		resident: &ResidentAnimationLease<'_>,
		playback: AnimationPlayback,
		target: &Skeleton,
	) -> Self {
		let pose_map = SkeletonPoseMap::by_name(resident.skeleton(), target);
		Self {
			state,
			lease,
			pose_map,
			playback,
			duration: resident.packed().duration(),
			time_seconds: 0.0,
		}
	}

	fn is_finished(&self) -> bool {
		self.playback == AnimationPlayback::Once && self.time_seconds >= self.duration
	}

	fn advance(&mut self, delta: MediaTime) -> ClipAdvance {
		let previous_time = self.time_seconds;
		let duration = self.duration;
		if duration <= 0.0 {
			self.time_seconds = 0.0;
			return ClipAdvance { wrapped_loops: 0 };
		}

		let advanced = previous_time + delta.as_seconds_f32();
		match self.playback {
			AnimationPlayback::Loop => {
				let wrapped_loops = (advanced / duration).floor().max(0.0) as usize;
				self.time_seconds = advanced.rem_euclid(duration);
				ClipAdvance { wrapped_loops }
			}
			AnimationPlayback::Once => {
				self.time_seconds = advanced.min(duration);
				ClipAdvance { wrapped_loops: 0 }
			}
		}
	}
}

#[derive(Clone, Copy)]
struct ClipAdvance {
	wrapped_loops: usize,
}

struct ActiveTransition {
	source: RuntimeClip,
	destination: RuntimeClip,
	duration: MediaTime,
	elapsed: MediaTime,
	begun: bool,
}

struct PendingPlayerTransition {
	target: AnimationStateId,
	duration: Option<MediaTime>,
}

/// The `RootMotionTranslation` struct selects node-local translation axes for root motion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootMotionTranslation(u8);

impl RootMotionTranslation {
	/// Keeps all translation in the visual pose.
	pub const NONE: Self = Self(0);
	/// Extracts translation along the node's local X axis.
	pub const X: Self = Self(1 << 0);
	/// Extracts translation along the node's local Y axis.
	pub const Y: Self = Self(1 << 1);
	/// Extracts translation along the node's local Z axis.
	pub const Z: Self = Self(1 << 2);
	/// Extracts translation along every node-local axis.
	pub const XYZ: Self = Self(Self::X.0 | Self::Y.0 | Self::Z.0);

	/// Combines translation axes for clips that move along more than one local axis.
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	const fn contains(self, component: usize) -> bool {
		self.0 & (1 << component) != 0
	}
}

/// The `RootMotionRotation` enum selects whether node-local rotation drives the owning object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootMotionRotation {
	/// Keeps rotation in the visual pose.
	None,
	/// Extracts the node's full rotation.
	Full,
}

/// The `RootMotionSettings` struct defines which motion a target-skeleton node contributes to its owning object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootMotionSettings<'a> {
	/// Names the unique node that contains the authored motion.
	pub node_name: &'a str,
	/// Selects node-local translation axes to extract.
	pub translation: RootMotionTranslation,
	/// Selects whether to extract node-local rotation.
	pub rotation: RootMotionRotation,
}

impl<'a> RootMotionSettings<'a> {
	/// Selects all translation and rotation from a dedicated authored root node.
	pub const fn full(node_name: &'a str) -> Self {
		Self {
			node_name,
			translation: RootMotionTranslation::XYZ,
			rotation: RootMotionRotation::Full,
		}
	}
}

#[derive(Clone, Copy)]
struct RootMotionTarget {
	node: usize,
	reference: LocalTransform,
	translation: RootMotionTranslation,
	rotation: RootMotionRotation,
}

/// Keeps a player target borrowed from an asset owner or directly owned by the player.
enum PlayerTargetSkeleton<'a> {
	Borrowed(&'a Skeleton),
	Owned(Skeleton),
}

impl Deref for PlayerTargetSkeleton<'_> {
	type Target = Skeleton;

	fn deref(&self) -> &Self::Target {
		match self {
			Self::Borrowed(target) => target,
			Self::Owned(target) => target,
		}
	}
}

/// The `AnimationGraphPose` struct borrows the latest player pose and frame root motion.
pub struct AnimationGraphPose<'a> {
	local_pose: &'a [LocalTransform],
	global_pose: &'a [Matrix],
	root_motion: RootMotionDelta,
}

impl<'a> AnimationGraphPose<'a> {
	/// Returns blendable local transforms with extracted root motion removed when configured.
	pub fn local_pose(&self) -> &'a [LocalTransform] {
		self.local_pose
	}

	/// Returns renderer-facing global skeleton matrices.
	pub fn global_pose(&self) -> &'a [Matrix] {
		self.global_pose
	}

	/// Returns this frame's root translation and rotation delta.
	pub const fn root_motion(&self) -> RootMotionDelta {
		self.root_motion
	}
}

/// The `AnimationGraphPlayer` struct evaluates one graph for one target skeleton without steady-state allocation.
///
/// Call [`AnimationPool::update`] once per tick, then call [`Self::advance`]
/// with the current input and shared pool. Apply [`AnimationGraphPose::root_motion`]
/// to the owning transform, then send
/// [`AnimationGraphPose::global_pose`] through the renderer-facing pose update.
pub struct AnimationGraphPlayer<'graph, 'target, I> {
	graph: &'graph AnimationGraph<I>,
	target: PlayerTargetSkeleton<'target>,
	root_motion: Option<RootMotionTarget>,
	active: Option<RuntimeClip>,
	transition: Option<ActiveTransition>,
	pending: Option<PendingPlayerTransition>,
	active_previous: Vec<LocalTransform>,
	active_current: Vec<LocalTransform>,
	destination_previous: Vec<LocalTransform>,
	destination_current: Vec<LocalTransform>,
	loop_start: Vec<LocalTransform>,
	loop_end: Vec<LocalTransform>,
	local_pose: Vec<LocalTransform>,
	global_pose: Vec<Matrix>,
	inertializer: PoseInertializer,
}

impl<'graph, 'target, I> AnimationGraphPlayer<'graph, 'target, I> {
	/// Creates a player with retained pose storage sized for the target skeleton.
	///
	/// `root_motion` selects a uniquely named target-skeleton node and the node-local
	/// channels delivered in object space and removed from the visual pose. Prefer
	/// all channels from a dedicated root. For locomotion authored on hips, select
	/// only the travel axes so the pose retains its vertical sway and rotation.
	pub fn new(
		graph: &'graph AnimationGraph<I>,
		target: &'target Skeleton,
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Borrowed(target), root_motion)
	}

	/// Initializes a player after selecting its borrowed or owned skeleton storage.
	fn with_target(
		graph: &'graph AnimationGraph<I>,
		target: PlayerTargetSkeleton<'target>,
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		let root_motion = resolve_root_motion_target(&target, root_motion)?;
		let node_count = target.nodes.len();
		let rest_pose: Vec<_> = target.nodes.iter().map(|node| node.rest_local).collect();
		let mut global_pose = Vec::with_capacity(node_count);
		write_global_pose(&target, &rest_pose, &mut global_pose).expect("Target rest pose must match its skeleton node count");

		Ok(Self {
			graph,
			target,
			root_motion,
			active: None,
			transition: None,
			pending: Some(PendingPlayerTransition {
				target: graph.initial_state(),
				duration: None,
			}),
			active_previous: rest_pose.clone(),
			active_current: rest_pose.clone(),
			destination_previous: rest_pose.clone(),
			destination_current: rest_pose.clone(),
			loop_start: rest_pose.clone(),
			loop_end: rest_pose.clone(),
			local_pose: rest_pose,
			global_pose,
			inertializer: PoseInertializer::new(node_count),
		})
	}

	/// Returns the currently playing destination state, if its initial clip has loaded.
	pub fn state(&self) -> Option<AnimationStateId> {
		self.transition
			.as_ref()
			.map(|transition| transition.destination.state)
			.or_else(|| self.active.as_ref().map(|active| active.state))
	}

	/// Advances playback, starts ready transitions, and borrows the resulting pose.
	///
	/// Call [`AnimationPool::update`] once before advancing any players for the
	/// current tick. This selects the first matching transition, evaluates the
	/// pose, and returns root motion for the same frame. While a selected target
	/// loads, the player retains the source clip and reevaluates its exits so stale
	/// input can cancel or retarget that request. Next, apply
	/// [`AnimationGraphPose::root_motion`] to the owning object and submit
	/// [`AnimationGraphPose::global_pose`] to the rendering system.
	pub fn advance(
		&mut self,
		delta: MediaTime,
		input: &I,
		pool: &mut AnimationPool,
	) -> Result<AnimationGraphPose<'_>, AnimationGraphPlayerError> {
		if delta < MediaTime::ZERO {
			return Err(AnimationGraphPlayerError::NegativeDelta);
		}
		self.refresh_pending_transition(input);
		self.start_pending(pool);

		if self.transition.is_none() && self.active.is_some() && self.pending.is_none() {
			self.select_transition(input);
			self.start_pending(pool);
		}

		// Resolve every clip before borrowing arena regions. The resulting leases
		// pin those regions until this evaluation and all root-motion samples finish.
		let root_motion = if self.transition.is_some() {
			let ready = {
				let transition = self.transition.as_ref().expect("transition was checked above");
				pool.request(&transition.source.lease) == AnimationPoolRequest::Ready
					&& pool.request(&transition.destination.lease) == AnimationPoolRequest::Ready
			};
			if ready {
				let (source, destination) = {
					let transition = self.transition.as_ref().expect("transition remains active while evaluating");
					(
						pool.acquire(&transition.source.lease)
							.expect("ready source lease must remain resident"),
						pool.acquire(&transition.destination.lease)
							.expect("ready destination lease must remain resident"),
					)
				};
				self.advance_transition(delta, &source, &destination)
			} else {
				RootMotionDelta::IDENTITY
			}
		} else if self.active.is_some() {
			let ready = {
				let active = self.active.as_ref().expect("active clip was checked above");
				pool.request(&active.lease) == AnimationPoolRequest::Ready
			};
			if ready {
				let resident = {
					let active = self.active.as_ref().expect("active clip remains active while evaluating");
					pool.acquire(&active.lease).expect("ready active lease must remain resident")
				};
				self.advance_active(delta, &resident)
			} else {
				RootMotionDelta::IDENTITY
			}
		} else {
			self.write_rest_pose();
			RootMotionDelta::IDENTITY
		};

		Ok(AnimationGraphPose {
			local_pose: &self.local_pose,
			global_pose: &self.global_pose,
			root_motion,
		})
	}

	fn start_pending(&mut self, pool: &mut AnimationPool) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		let target_state = self.graph.state(pending.target);
		let lease = target_state.clip.lease.clone();
		if pool.request(&lease) != AnimationPoolRequest::Ready {
			return;
		}
		let resident = pool.acquire(&lease).expect("ready pending lease must remain resident");
		let pending = self.pending.take().expect("pending state transition was checked above");
		let destination = RuntimeClip::new(pending.target, lease, &resident, target_state.clip.playback(), &self.target);

		if let Some(duration) = pending.duration {
			let source = self.active.take().expect("only a loaded state can start a graph transition");
			sample_target_pose(&destination, &resident, &mut self.destination_current);
			self.destination_previous.copy_from_slice(&self.destination_current);
			self.transition = Some(ActiveTransition {
				source,
				destination,
				duration,
				elapsed: MediaTime::ZERO,
				begun: false,
			});
		} else {
			sample_target_pose(&destination, &resident, &mut self.active_current);
			self.active_previous.copy_from_slice(&self.active_current);
			self.active = Some(destination);
		}
	}

	/// Cancels or retargets a loading state when its source's selected edge changes.
	fn refresh_pending_transition(&mut self, input: &I) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		if pending.duration.is_none() || self.transition.is_some() {
			return;
		}
		let selected = self
			.active
			.as_ref()
			.and_then(|active| self.selected_transition(active, input));
		self.pending = selected;
	}

	fn select_transition(&mut self, input: &I) {
		let selected = self.active.as_ref().expect("transition selection requires an active clip");
		self.pending = self.selected_transition(selected, input);
	}

	/// Resolves one source state to its highest-priority current destination.
	fn selected_transition(&self, active: &RuntimeClip, input: &I) -> Option<PendingPlayerTransition> {
		let source_state = self.graph.state(active.state);
		source_state
			.select_transition(input, active.is_finished())
			.map(|(target, duration)| PendingPlayerTransition {
				target,
				duration: Some(duration),
			})
	}

	fn advance_active(&mut self, delta: MediaTime, resident: &ResidentAnimationLease<'_>) -> RootMotionDelta {
		let active = self.active.as_mut().expect("active clip was checked before advancing");
		let advance = active.advance(delta);
		std::mem::swap(&mut self.active_previous, &mut self.active_current);
		sample_target_pose(active, resident, &mut self.active_current);
		let root_motion = root_delta(
			self.root_motion,
			active,
			&self.active_previous,
			&self.active_current,
			advance,
			resident,
			&self.target,
			&mut self.loop_start,
			&mut self.loop_end,
		);
		self.local_pose.copy_from_slice(&self.active_current);
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();
		root_motion
	}

	fn advance_transition(
		&mut self,
		delta: MediaTime,
		source_resident: &ResidentAnimationLease<'_>,
		destination_resident: &ResidentAnimationLease<'_>,
	) -> RootMotionDelta {
		let root_motion_target = self.root_motion;
		let target = &self.target;
		let (root_motion, completed) = {
			let transition = self.transition.as_mut().expect("transition was checked before advancing");
			let source_advance = transition.source.advance(delta);
			let destination_advance = transition.destination.advance(delta);
			std::mem::swap(&mut self.active_previous, &mut self.active_current);
			std::mem::swap(&mut self.destination_previous, &mut self.destination_current);
			sample_target_pose(&transition.source, source_resident, &mut self.active_current);
			sample_target_pose(&transition.destination, destination_resident, &mut self.destination_current);

			let source_root_motion = root_delta(
				root_motion_target,
				&transition.source,
				&self.active_previous,
				&self.active_current,
				source_advance,
				source_resident,
				target,
				&mut self.loop_start,
				&mut self.loop_end,
			);
			let destination_root_motion = root_delta(
				root_motion_target,
				&transition.destination,
				&self.destination_previous,
				&self.destination_current,
				destination_advance,
				destination_resident,
				target,
				&mut self.loop_start,
				&mut self.loop_end,
			);

			let duration = transition.duration;
			if duration == MediaTime::ZERO {
				self.local_pose.copy_from_slice(&self.destination_current);
			} else if delta == MediaTime::ZERO && !transition.begun {
				// Inertialization needs a non-zero sample interval to derive velocities.
				// Keep the source pose for this zero-time tick and begin next advance.
				self.local_pose.copy_from_slice(&self.active_current);
			} else {
				if !transition.begun {
					self.inertializer
						.begin(
							&self.active_previous,
							&self.active_current,
							&self.destination_previous,
							&self.destination_current,
							delta,
							duration,
						)
						.expect("player pose buffers must match the target skeleton");
					transition.begun = true;
				}
				self.inertializer
					.apply(&self.destination_current, delta, &mut self.local_pose)
					.expect("player pose buffers must match the target skeleton");
			}

			transition.elapsed = (transition.elapsed + delta).min(duration);
			let transition_factor = if duration == MediaTime::ZERO {
				1.0
			} else {
				(transition.elapsed.as_seconds_f32() / duration.as_seconds_f32()).clamp(0.0, 1.0)
			};
			(
				source_root_motion.blend(destination_root_motion, transition_factor),
				transition.elapsed == duration,
			)
		};
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();

		if completed {
			let destination = self
				.transition
				.take()
				.expect("transition remains owned until its duration completes")
				.destination;
			self.active_previous.copy_from_slice(&self.destination_current);
			self.active_current.copy_from_slice(&self.destination_current);
			self.active = Some(destination);
		}
		root_motion
	}

	fn write_rest_pose(&mut self) {
		for (output, node) in self.local_pose.iter_mut().zip(&self.target.nodes) {
			*output = node.rest_local;
		}
		self.write_global_pose();
	}

	fn remove_root_motion_from_visual_pose(&mut self) {
		let Some(root_motion) = self.root_motion else {
			return;
		};
		for component in 0..3 {
			if root_motion.translation.contains(component) {
				self.local_pose[root_motion.node].translation[component] = root_motion.reference.translation[component];
			}
		}
		if root_motion.rotation == RootMotionRotation::Full {
			self.local_pose[root_motion.node].rotation = root_motion.reference.rotation;
		}
	}

	fn write_global_pose(&mut self) {
		write_global_pose(&self.target, &self.local_pose, &mut self.global_pose)
			.expect("player local pose always matches its target skeleton");
	}
}

impl<'graph, I> AnimationGraphPlayer<'graph, 'static, I> {
	/// Creates a player that owns the target skeleton for its full playback lifetime.
	///
	/// Use this after receiving a mesh or skeleton from an async loader. It avoids
	/// coupling the player lifetime to a temporary resource reference. Next, call
	/// [`Self::advance`] from the application's per-frame animation system.
	pub fn new_owned(
		graph: &'graph AnimationGraph<I>,
		target: Skeleton,
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Owned(target), root_motion)
	}
}

/// The `AnimationGraphPlayerError` enum reports invalid player inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationGraphPlayerError {
	RootMotionNodeNotFound { name: String },
	DuplicateRootMotionNodeName { name: String },
	NegativeDelta,
}

impl fmt::Display for AnimationGraphPlayerError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::RootMotionNodeNotFound { name } => write!(
				formatter,
				"Animation root-motion node was not found. The most likely cause is that the target skeleton has no node named '{name}'."
			),
			Self::DuplicateRootMotionNodeName { name } => write!(
				formatter,
				"Animation root-motion node name is ambiguous. The most likely cause is that the target skeleton has more than one node named '{name}'."
			),
			Self::NegativeDelta => write!(
				formatter,
				"Animation graph advance delta is negative. The most likely cause is passing a timeline offset instead of a frame duration."
			),
		}
	}
}

impl std::error::Error for AnimationGraphPlayerError {}

/// Resolves the stable root-motion name once so steady-state sampling retains a numeric node index.
fn resolve_root_motion_target(
	target: &Skeleton,
	root_motion: Option<RootMotionSettings<'_>>,
) -> Result<Option<RootMotionTarget>, AnimationGraphPlayerError> {
	let Some(settings) = root_motion else {
		return Ok(None);
	};
	let name = settings.node_name;
	let mut matches = target
		.nodes
		.iter()
		.enumerate()
		.filter(|(_, node)| node.name.as_deref() == Some(name));
	let Some((node, root)) = matches.next() else {
		return Err(AnimationGraphPlayerError::RootMotionNodeNotFound { name: name.into() });
	};
	if matches.next().is_some() {
		return Err(AnimationGraphPlayerError::DuplicateRootMotionNodeName { name: name.into() });
	}
	Ok(Some(RootMotionTarget {
		node,
		reference: root.rest_local,
		translation: settings.translation,
		rotation: settings.rotation,
	}))
}

/// Samples one loaded clip directly into target-skeleton local transforms.
fn sample_target_pose(clip: &RuntimeClip, resident: &ResidentAnimationLease<'_>, output: &mut [LocalTransform]) {
	resident
		.packed()
		.sample_target_local_pose(&clip.pose_map, clip.time_seconds, output);
}

/// Calculates one clip's root delta, preserving forward progress across loop boundaries.
#[allow(clippy::too_many_arguments)]
fn root_delta(
	root_motion: Option<RootMotionTarget>,
	clip: &RuntimeClip,
	previous: &[LocalTransform],
	current: &[LocalTransform],
	advance: ClipAdvance,
	resident: &ResidentAnimationLease<'_>,
	target: &Skeleton,
	loop_start: &mut Vec<LocalTransform>,
	loop_end: &mut Vec<LocalTransform>,
) -> RootMotionDelta {
	let Some(root_motion) = root_motion else {
		return RootMotionDelta::IDENTITY;
	};
	if advance.wrapped_loops == 0 {
		return object_space_root_delta(root_motion, target, previous, current);
	}

	// Sample the clip ends only for a loop crossing. This keeps the common
	// steady-state path to one clip sample while preserving forward root motion.
	resident
		.packed()
		.sample_target_local_pose(&clip.pose_map, clip.duration, loop_end);
	resident.packed().sample_target_local_pose(&clip.pose_map, 0.0, loop_start);

	let mut delta = object_space_root_delta(root_motion, target, previous, loop_end);
	let full_loop = object_space_root_delta(root_motion, target, loop_start, loop_end);
	for _ in 1..advance.wrapped_loops {
		delta = delta.then(full_loop);
	}
	delta.then(object_space_root_delta(root_motion, target, loop_start, current))
}

/// Calculates a root delta after converting both local poses into the owning object's space.
fn object_space_root_delta(
	root_motion: RootMotionTarget,
	target: &Skeleton,
	previous: &[LocalTransform],
	current: &[LocalTransform],
) -> RootMotionDelta {
	// Replace unselected channels with their reference values before hierarchy
	// composition. The selected local axes then inherit authored parent scale and
	// rotation while pose-only hip motion cannot leak into the owning object.
	let previous_root = extracted_root_transform(root_motion, previous[root_motion.node]);
	let current_root = extracted_root_transform(root_motion, current[root_motion.node]);
	RootMotionDelta::between(
		object_space_transform_with_node(target, previous, root_motion.node, previous_root),
		object_space_transform_with_node(target, current, root_motion.node, current_root),
	)
}

/// Keeps selected root-motion channels and restores all other channels to the reference pose.
fn extracted_root_transform(root_motion: RootMotionTarget, mut transform: LocalTransform) -> LocalTransform {
	for component in 0..3 {
		if !root_motion.translation.contains(component) {
			transform.translation[component] = root_motion.reference.translation[component];
		}
	}
	if root_motion.rotation == RootMotionRotation::None {
		transform.rotation = root_motion.reference.rotation;
	}
	transform
}

/// Composes a substituted node transform with its unchanged ancestors.
fn object_space_transform_with_node(
	skeleton: &Skeleton,
	local_pose: &[LocalTransform],
	node: usize,
	mut result: LocalTransform,
) -> LocalTransform {
	let mut parent = skeleton.nodes[node].parent;
	while let Some(parent_index) = parent {
		let parent_node = parent_index as usize;
		result = compose_local_transform(local_pose[parent_node], result);
		parent = skeleton.nodes[parent_node].parent;
	}
	result
}

/// Prepends `parent` to `child`, preserving the hierarchy's scale-rotate-translate order.
fn compose_local_transform(parent: LocalTransform, child: LocalTransform) -> LocalTransform {
	let scaled_translation = std::array::from_fn(|component| child.translation[component] * parent.scale[component]);
	let rotated_translation = rotate_vector(parent.rotation, scaled_translation);
	LocalTransform {
		translation: std::array::from_fn(|component| parent.translation[component] + rotated_translation[component]),
		rotation: multiply_quaternion(parent.rotation, child.rotation),
		scale: std::array::from_fn(|component| parent.scale[component] * child.scale[component]),
	}
}

/// Rotates one translation vector by a normalized quaternion without changing its magnitude.
fn rotate_vector([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
	let quaternion_vector = [x, y, z];
	let twice_cross = [
		2.0 * (quaternion_vector[1] * vector[2] - quaternion_vector[2] * vector[1]),
		2.0 * (quaternion_vector[2] * vector[0] - quaternion_vector[0] * vector[2]),
		2.0 * (quaternion_vector[0] * vector[1] - quaternion_vector[1] * vector[0]),
	];
	let cross_again = [
		quaternion_vector[1] * twice_cross[2] - quaternion_vector[2] * twice_cross[1],
		quaternion_vector[2] * twice_cross[0] - quaternion_vector[0] * twice_cross[2],
		quaternion_vector[0] * twice_cross[1] - quaternion_vector[1] * twice_cross[0],
	];
	std::array::from_fn(|component| vector[component] + w * twice_cross[component] + cross_again[component])
}

#[cfg(test)]
mod tests {
	use std::{
		collections::{HashMap, VecDeque},
		num::NonZeroUsize,
	};

	use resource_management::{
		resources::{
			animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode},
		},
		Reference,
	};

	use super::*;
	use crate::MediaTime;

	fn test_skeleton() -> Skeleton {
		Skeleton {
			nodes: vec![SkeletonNode {
				name: Some("root".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		}
	}

	fn test_animation(name: &str, end_translation: f32) -> Animation {
		Animation {
			name: Some(name.into()),
			skeleton: Reference::in_memory("test.skeleton", test_skeleton()),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0; 3], [end_translation, 0.0, 0.0]],
				}),
				rotation: None,
				scale: None,
			}],
		}
	}

	/// Measures the representation retained by the pool rather than the transient resource representation.
	fn packed_test_animation_bytes(name: &str, end_translation: f32) -> usize {
		PackedAnimationData::resident_bytes(&test_animation(name, end_translation))
	}

	fn pool(byte_budget: usize) -> AnimationPool {
		let (commands, _command_receiver) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		let (_completion_sender, completions) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		let word_capacity = byte_budget / std::mem::size_of::<u32>();
		AnimationPool {
			commands: commands.to_sync(),
			completions: completions.to_sync(),
			storage: vec![0; word_capacity].into_boxed_slice(),
			free_regions: vec![AnimationArenaRegion {
				offset: 0,
				word_count: word_capacity,
			}],
			entries: HashMap::new(),
			events: VecDeque::with_capacity(super::ANIMATION_POOL_EVENT_CAPACITY),
			byte_budget,
			resident_bytes: 0,
			next_use: std::cell::Cell::new(0),
			commands_closed: false,
			completions_closed: false,
		}
	}

	#[test]
	fn player_returns_root_motion_and_removes_it_from_the_visual_pose() {
		let target = test_skeleton();
		let idle = test_animation("idle", 1.0);
		let run = test_animation("run", 3.0);
		let byte_budget = packed_test_animation_bytes("idle", 1.0).saturating_add(packed_test_animation_bytes("run", 3.0));
		let mut pool = pool(byte_budget);
		pool.admit("idle.animation".into(), idle);
		pool.admit("run.animation".into(), run);

		let builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let run = builder.state("run").with(AnimationClip::looping("run.animation"));
		builder.transition(idle.id, run.id, AnimationTransition::when(|running| *running));
		let graph = builder.build(idle).expect("graph should build");
		let mut player =
			AnimationGraphPlayer::new(&graph, &target, Some(RootMotionSettings::full("root"))).expect("player should build");

		let initial = player.advance(MediaTime::ZERO, &false, &mut pool).expect("initial pose");
		assert_eq!(initial.local_pose()[0], LocalTransform::identity());
		let root_motion = player
			.advance(MediaTime::from_millis(500), &false, &mut pool)
			.expect("idle pose")
			.root_motion();
		assert_eq!(root_motion.translation, [0.5, 0.0, 0.0]);
		assert_eq!(
			player
				.advance(MediaTime::ZERO, &false, &mut pool)
				.expect("visual pose")
				.local_pose()[0]
				.translation,
			[0.0; 3]
		);

		let switched = player.advance(MediaTime::ZERO, &true, &mut pool).expect("run transition");
		assert_eq!(switched.root_motion().translation, [0.0; 3]);
		assert_eq!(player.state(), Some(run.id));
		assert_eq!(
			player
				.advance(MediaTime::from_millis(500), &true, &mut pool)
				.expect("run pose")
				.root_motion()
				.translation,
			[1.5, 0.0, 0.0]
		);
		assert_eq!(
			player
				.advance(MediaTime::from_millis(750), &true, &mut pool)
				.expect("looped run pose")
				.root_motion()
				.translation,
			[2.25, 0.0, 0.0]
		);
	}

	#[test]
	fn player_cancels_loading_transition_states_and_completes_loaded_ones() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let walk_animation = test_animation("walk", 2.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = pool(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);

		let builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(AnimationTransition::when(|moving| *moving));
		let graph = builder.build(idle).expect("graph should build");
		let target = test_skeleton();
		let mut player = AnimationGraphPlayer::new(&graph, &target, None).expect("player should build");

		player.advance(MediaTime::ZERO, &false, &mut pool).expect("initial idle pose");
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("queues start-walk clip");
		assert_eq!(player.state(), Some(idle.id));
		player
			.advance(MediaTime::ZERO, &false, &mut pool)
			.expect("cancels stale start-walk request");
		assert_eq!(player.state(), Some(idle.id));

		pool.admit("start.animation".into(), start_animation);
		pool.admit("walk.animation".into(), walk_animation);
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("starts transition state");
		assert_eq!(player.state(), Some(start_walk.id));
		player
			.advance(MediaTime::from_seconds(1), &true, &mut pool)
			.expect("finishes transition-state clip");
		assert_eq!(player.state(), Some(start_walk.id));
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("enters completion state");
		assert_eq!(player.state(), Some(walk.id));
	}

	#[test]
	fn player_requires_one_uniquely_named_root_motion_node() {
		let builder = AnimationGraph::<()>::builder();
		let state = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let graph = builder.build(state).expect("graph should build");
		let target = test_skeleton();

		assert!(matches!(
			AnimationGraphPlayer::new(&graph, &target, Some(RootMotionSettings::full("Hips"))),
			Err(super::AnimationGraphPlayerError::RootMotionNodeNotFound { name }) if name == "Hips"
		));

		let duplicate_target = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
		};
		assert!(matches!(
			AnimationGraphPlayer::new(&graph, &duplicate_target, Some(RootMotionSettings::full("Hips"))),
			Err(super::AnimationGraphPlayerError::DuplicateRootMotionNodeName { name }) if name == "Hips"
		));
	}

	#[test]
	fn player_selectively_extracts_object_space_root_motion_across_a_remapped_scaled_loop() {
		let root_rotation = crate::animation::math::quaternion_exp([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
		let source = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Root".into()),
					parent: None,
					rest_local: LocalTransform {
						rotation: root_rotation,
						scale: [0.01; 3],
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform {
						translation: [0.0, 100.0, 0.0],
						..LocalTransform::identity()
					},
				},
			],
		};
		// The target inserts helper nodes before Hips, matching FBX rigs whose
		// compatible named joints do not share source indices.
		let target = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: None,
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Root".into()),
					parent: Some(0),
					rest_local: LocalTransform {
						rotation: root_rotation,
						scale: [0.01; 3],
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("IK Helper".into()),
					parent: Some(1),
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(1),
					rest_local: LocalTransform {
						translation: [0.0, 100.0, 0.0],
						..LocalTransform::identity()
					},
				},
			],
		};
		let animation = Animation {
			name: Some("walk".into()),
			skeleton: Reference::in_memory("scaled.skeleton", source),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0, 100.0, 0.0], [20.0, 110.0, -100.0]],
				}),
				rotation: Some(QuaternionCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.382_683_43, 0.0, 0.923_879_5]],
				}),
				scale: None,
			}],
		};
		let mut pool = pool(animation.estimated_resident_bytes());
		pool.admit("walk.animation".into(), animation);
		let builder = AnimationGraph::<()>::builder();
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let graph = builder.build(walk).expect("graph should build");
		let mut player = AnimationGraphPlayer::new(
			&graph,
			&target,
			Some(RootMotionSettings {
				node_name: "Hips",
				translation: RootMotionTranslation::Z,
				rotation: RootMotionRotation::None,
			}),
		)
		.expect("player should build");

		let initial = player.advance(MediaTime::ZERO, &(), &mut pool).expect("initial pose");
		assert_eq!(initial.root_motion().translation, [0.0; 3]);
		let first = player
			.advance(MediaTime::from_millis(750), &(), &mut pool)
			.expect("walk pose");
		math::assert_float_eq!(first.root_motion().translation[0], -0.75);
		math::assert_float_eq!(first.root_motion().translation[1], 0.0);
		math::assert_float_eq!(first.root_motion().translation[2], 0.0);
		assert_eq!(first.local_pose()[3].translation, [15.0, 107.5, 0.0]);
		assert_ne!(first.local_pose()[3].rotation, LocalTransform::identity().rotation);

		let wrapped = player
			.advance(MediaTime::from_millis(500), &(), &mut pool)
			.expect("wrapped walk pose");
		math::assert_float_eq!(wrapped.root_motion().translation[0], -0.5);
		math::assert_float_eq!(wrapped.root_motion().translation[1], 0.0);
		math::assert_float_eq!(wrapped.root_motion().translation[2], 0.0);
		assert_eq!(wrapped.local_pose()[3].translation, [5.0, 102.5, 0.0]);
		assert_ne!(wrapped.local_pose()[3].rotation, LocalTransform::identity().rotation);
	}
}
