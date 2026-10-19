//! The look chain: nodes a player turns toward a target on top of the graph's pose, as the animation reference's
//! Look chain section specifies.

use std::f32::consts::PI;

use super::*;
use crate::math::{Orientation, Vector};
use resource_management::resources::ParentSpace;

/// The `LookChainSettings` struct names the nodes a player turns toward its look target, from a spine to the head,
/// and how far and how fast they turn.
///
/// Give them to [`AnimationGraphPlayer::with_look_chain`], then set where to look each frame with
/// [`AnimationGraphPlayer::set_look`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookChainSettings<'a> {
	/// The chain's nodes from its base to its end, each with its share of the turn. Each node must exist once in the
	/// canonical skeleton and descend from the node before it; the shares normalize by their sum.
	pub nodes: &'a [(&'a str, f32)],
	/// The node whose animated yaw centers the yaw limit, such as the hips, so a clip that turns the body carries the
	/// look with it.
	pub body_node: &'a str,
	/// How far the chain turns from the body node's yaw, in radians, up to π.
	pub yaw_limit: f32,
	/// How far the chain looks up, in radians, up to π/2.
	pub up_limit: f32,
	/// How far the chain looks down, in radians, up to π/2.
	pub down_limit: f32,
	/// How quickly the chain follows the target, per second, or none to follow it at once.
	pub response: Option<f32>,
}

/// The `LookTarget` struct is where a player's look chain turns toward, in the entity's object space.
///
/// Yaw turns about +Y and pitch about +X, by the right-hand rule. The object faces -Z, so a positive pitch looks up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookTarget {
	pub yaw: f32,
	pub pitch: f32,
	/// How far the entity turned about +Y since the last step. The eased yaw turns the other way by this much before
	/// it eases, so the chain holds its direction in the world when the entity takes over a turn clip's rotation.
	pub turned: f32,
}

impl LookTarget {
	/// Looks along `yaw` and `pitch` in the entity's object space.
	pub const fn new(yaw: f32, pitch: f32) -> Self {
		Self { yaw, pitch, turned: 0.0 }
	}

	/// Tells the chain the entity turned by `turned` about +Y since the last step, so it holds its direction in the
	/// world across the turn.
	pub const fn turned_by(self, turned: f32) -> Self {
		Self { turned, ..self }
	}
}

/// The `LookChain` struct is a player's look chain resolved against its canonical skeleton, with the eased angles it
/// looks along between steps.
pub(super) struct LookChain {
	/// The chain's node indices from base to end, with their normalized shares.
	nodes: Vec<(usize, f32)>,
	body: usize,
	/// The rest object-space rotations the animated yaws of the body and end nodes measure from.
	body_rest: Orientation,
	end_rest: Orientation,
	yaw_limit: f32,
	up_limit: f32,
	down_limit: f32,
	response: Option<f32>,
	/// The eased yaw and pitch the chain looks along, or none while it is released.
	eased: Option<(f32, f32)>,
}

impl LookChain {
	/// Resolves `settings` against `skeleton`, or says which setting cannot be.
	pub(super) fn resolve(skeleton: &Skeleton, settings: LookChainSettings<'_>) -> Result<Self, AnimationGraphPlayerError> {
		let node = |name: &str| -> Result<usize, AnimationGraphPlayerError> {
			let mut matches = skeleton
				.nodes
				.iter()
				.enumerate()
				.filter(|(_, node)| node.name.as_deref() == Some(name))
				.map(|(index, _)| index);
			let index = matches
				.next()
				.ok_or_else(|| AnimationGraphPlayerError::LookChainNodeNotFound { name: name.into() })?;
			if matches.next().is_some() {
				return Err(AnimationGraphPlayerError::DuplicateLookChainNodeName { name: name.into() });
			}
			Ok(index)
		};
		let out_of_range = |setting: &'static str| AnimationGraphPlayerError::LookChainSettingOutOfRange { setting };
		if settings.nodes.is_empty() {
			return Err(out_of_range("nodes"));
		}
		let total: f32 = settings.nodes.iter().map(|&(_, share)| share).sum();
		if settings.nodes.iter().any(|&(_, share)| !(share.is_finite() && share > 0.0)) {
			return Err(out_of_range("shares"));
		}
		if !(0.0..=PI).contains(&settings.yaw_limit) {
			return Err(out_of_range("yaw_limit"));
		}
		if !(0.0..=PI / 2.0).contains(&settings.up_limit) {
			return Err(out_of_range("up_limit"));
		}
		if !(0.0..=PI / 2.0).contains(&settings.down_limit) {
			return Err(out_of_range("down_limit"));
		}
		if settings.response.is_some_and(|response| !(response.is_finite() && response > 0.0)) {
			return Err(out_of_range("response"));
		}
		let mut nodes = Vec::with_capacity(settings.nodes.len());
		for &(name, share) in settings.nodes {
			let index = node(name)?;
			// Each node descends from the one before it, so the turn of one carries the rest.
			if let Some(&(previous, _)) = nodes.last()
				&& !descends_from(skeleton, index, previous)
			{
				return Err(AnimationGraphPlayerError::LookChainNodeNotDescendant { name: name.into() });
			}
			nodes.push((index, share / total));
		}
		let body = node(settings.body_node)?;
		let rest = |node: usize| model_rotation(skeleton, |node| skeleton.nodes[node].rest_local.rotation, Some(node as u32));
		let end = nodes.last().map(|&(node, _)| node).expect("the chain has a node");
		Ok(Self {
			body_rest: rest(body),
			end_rest: rest(end),
			nodes,
			body,
			yaw_limit: settings.yaw_limit,
			up_limit: settings.up_limit,
			down_limit: settings.down_limit,
			response: settings.response,
			eased: None,
		})
	}

	/// Stops turning at once; the next target starts the easing over.
	pub(super) fn release(&mut self) {
		self.eased = None;
	}

	/// Turns `pose` toward `target`, easing over `delta` from where the chain looked last step.
	pub(super) fn apply(&mut self, skeleton: &Skeleton, pose: &mut [LocalTransform], target: LookTarget, delta: MediaTime) {
		let yaw_of = |node: usize, rest: Orientation| {
			let posed = model_rotation(skeleton, |node| pose[node].rotation, Some(node as u32));
			animated_yaw(posed, rest)
		};
		let body_yaw = yaw_of(self.body, self.body_rest);
		let end_yaw = yaw_of(self.nodes.last().expect("the chain has a node").0, self.end_rest);

		// The entity turned under the chain; the eased yaw turns back by as much, so the look holds in the world.
		let eased = self.eased.map(|(yaw, pitch)| (yaw - target.turned, pitch));
		// The target yaw clamps around the body node's yaw. A target the chain cannot reach clamps toward the side the
		// chain already looks to, once it is well off the middle, so a target passing behind does not swing it across.
		let limit = self.yaw_limit;
		let offset = wrap_angle(target.yaw - body_yaw);
		let current = eased.map_or(offset, |(yaw, _)| wrap_angle(yaw - body_yaw));
		let clamped_yaw = body_yaw
			+ if offset.abs() <= limit {
				offset
			} else {
				limit.copysign(if current.abs() > limit * 0.5 { current } else { offset })
			};
		let clamped_pitch = target.pitch.clamp(-self.down_limit, self.up_limit);
		let (yaw, pitch) = match (eased, self.response) {
			(Some((yaw, pitch)), Some(response)) => {
				let follow = 1.0 - (-response * delta.as_seconds_f32()).exp();
				(
					wrap_angle(yaw + wrap_angle(clamped_yaw - yaw) * follow),
					pitch + (clamped_pitch - pitch) * follow,
				)
			}
			_ => (wrap_angle(clamped_yaw), clamped_pitch),
		};
		self.eased = Some((yaw, pitch));

		// The chain makes up the difference between where the clip already points its end and the eased yaw. The turn
		// is one rotation, split by share along its axis so the shares compose to the whole at the end node. Each node
		// turns in object space after its animated rotation, and its descendants turn with it, since their parents'
		// rotations are read after the turn.
		let turn_yaw = wrap_angle(yaw - end_yaw);
		let whole: Vector<ParentSpace> = yaw_pitch_rotation(turn_yaw, pitch).to_rotation_vector();
		for &(node, share) in &self.nodes {
			let parent = model_rotation(skeleton, |node| pose[node].rotation, skeleton.nodes[node].parent);
			let turn = Orientation::try_from_rotation_vector(whole * share).expect("a share of a finite turn is a valid rotation");
			let local = &mut pose[node];
			local.rotation = parent.inverse().compose(turn.compose(parent.compose(local.rotation)));
		}
	}
}

/// Returns whether `node` is `ancestor` or descends from it.
fn descends_from(skeleton: &Skeleton, node: usize, ancestor: usize) -> bool {
	let mut current = Some(node as u32);
	while let Some(index) = current {
		if index as usize == ancestor {
			return true;
		}
		current = skeleton.nodes[index as usize].parent;
	}
	false
}

/// Returns the object-space rotation of `node`, composed from its own and its ancestors' local rotations, or the
/// identity for no node.
fn model_rotation(skeleton: &Skeleton, local_rotation: impl Fn(usize) -> Orientation, node: Option<u32>) -> Orientation {
	let mut rotation = Orientation::identity();
	let mut ancestor = node;
	while let Some(node) = ancestor {
		rotation = local_rotation(node as usize).compose(rotation);
		ancestor = skeleton.nodes[node as usize].parent;
	}
	rotation
}

/// Returns how far `posed` turns about +Y from `rest`, in radians, measured as the angle from -Z to -Z turned by the
/// difference and projected onto the XZ plane.
fn animated_yaw(posed: Orientation, rest: Orientation) -> f32 {
	let forward: Vector<ParentSpace> = posed.compose(rest.inverse()).rotate_vector(Vector::new(0.0, 0.0, -1.0));
	wrap_angle(forward.x().atan2(forward.z()) - PI)
}

/// Returns a rotation that pitches about +X, then yaws about +Y.
fn yaw_pitch_rotation(yaw: f32, pitch: f32) -> Orientation {
	let about = |x: f32, y: f32| {
		Orientation::try_from_rotation_vector(Vector::<ParentSpace>::new(x, y, 0.0))
			.expect("a finite angle is a valid rotation")
	};
	about(0.0, yaw).compose(about(pitch, 0.0))
}

/// Wraps an angle in radians into -π..π.
fn wrap_angle(angle: f32) -> f32 {
	(angle + PI).rem_euclid(2.0 * PI) - PI
}
