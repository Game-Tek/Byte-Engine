//! What a graph state plays, and the requests that select a state and where to play its blend.

use std::ops::Deref;

use super::*;
use crate::animation::blend::{BlendSpace1D, BlendSpace2D, BlendSpaceError};

/// The `StateMotion` enum holds what one state plays, so the player can treat clips and blends alike.
///
/// Every clip of a blend shares one playback mode, which [`AnimationGraphBuilder::build`] checks.
pub(super) enum StateMotion {
	Clip(AnimationClip),
	Linear {
		clips: Box<[AnimationClip]>,
		space: BlendSpace1D,
	},
	Planar {
		clips: Box<[AnimationClip]>,
		space: BlendSpace2D,
	},
}

impl StateMotion {
	/// Returns every clip the state may sample, in the order its weights use.
	pub(super) fn clips(&self) -> &[AnimationClip] {
		match self {
			Self::Clip(clip) => std::slice::from_ref(clip),
			Self::Linear { clips, .. } | Self::Planar { clips, .. } => clips,
		}
	}

	/// Returns how the state's clips play once they reach their end.
	pub(super) fn playback(&self) -> AnimationPlayback {
		self.clips()[0].playback()
	}

	/// Writes each clip's weight for the blend position `value`.
	///
	/// A value of the wrong kind or a non-finite one leaves `weights` unchanged, so the state keeps its last mix.
	pub(super) fn write_weights(&self, value: BlendValue, weights: &mut [f32]) {
		match (self, value) {
			(Self::Clip(_), _) => weights[0] = 1.0,
			(Self::Linear { space, .. }, BlendValue::Linear(value)) => {
				let _ = space.write_weights(value, weights);
			}
			(Self::Planar { space, .. }, BlendValue::Planar(value)) => {
				let _ = space.write_weights(value, weights);
			}
			_ => {}
		}
	}
}

/// The `AnimationBlend` struct collects the clips a blend state mixes and where each one sits.
///
/// Build one with [`Self::new`] and [`Self::sample`], then give it to [`AnimationGraphStateBuilder::linear`] for
/// positions on a line (`f32`) or [`AnimationGraphStateBuilder::planar`] for positions on a plane (`[f32; 2]`).
/// Each frame, client code passes the position to play with [`LinearStateId::at`] or [`PlanarStateId::at`].
pub struct AnimationBlend<P> {
	samples: Vec<(P, AnimationClip)>,
	triangles: Option<Vec<[usize; 3]>>,
}

impl<P> AnimationBlend<P> {
	/// Starts an empty blend. Add its clips with [`Self::sample`].
	pub fn new() -> Self {
		Self {
			samples: Vec::new(),
			triangles: None,
		}
	}

	/// Places `clip` at `position`. Every clip of a blend must loop, or every one must play once.
	pub fn sample(mut self, position: P, clip: AnimationClip) -> Self {
		self.samples.push((position, clip));
		self
	}
}

impl<P> Default for AnimationBlend<P> {
	fn default() -> Self {
		Self::new()
	}
}

impl AnimationBlend<f32> {
	/// Orders the samples along the line and validates their positions.
	pub(super) fn into_motion(mut self) -> Result<StateMotion, BlendSpaceError> {
		self.samples.sort_by(|left, right| left.0.total_cmp(&right.0));
		let space = BlendSpace1D::new(self.samples.iter().map(|(position, _)| *position).collect::<Vec<_>>())?;
		Ok(StateMotion::Linear {
			clips: self.samples.into_iter().map(|(_, clip)| clip).collect(),
			space,
		})
	}
}

impl AnimationBlend<[f32; 2]> {
	/// Triangulates the plane explicitly, with triangles of sample indices in the order the samples were added.
	///
	/// Without it, the samples must form a ring, like the directions of a locomotion blend, optionally with one
	/// sample at its centre, and are fanned into triangles. Give a direction ring a centre sample, like standing
	/// still: without one, positions crossing the middle mix in clips from one side of the ring only. Use explicit
	/// triangles for any other layout.
	pub fn triangles(mut self, triangles: impl Into<Vec<[usize; 3]>>) -> Self {
		self.triangles = Some(triangles.into());
		self
	}

	/// Triangulates the samples, fanning them around their centre unless explicit triangles were given.
	pub(super) fn into_motion(self) -> Result<StateMotion, BlendSpaceError> {
		let positions: Vec<_> = self.samples.iter().map(|(position, _)| *position).collect();
		let triangles = self.triangles.unwrap_or_else(|| fan_triangles(&positions));
		let space = BlendSpace2D::new(positions, triangles)?;
		Ok(StateMotion::Planar {
			clips: self.samples.into_iter().map(|(_, clip)| clip).collect(),
			space,
		})
	}
}

/// Fans a ring of positions into triangles, around a position at the ring's centre when there is one.
///
/// With a centre position, every triangle joins it to two neighbours on the ring, so crossing the middle mixes only
/// the clips on the line crossed. Without one, the triangles share the ring's first position in angular order, which
/// still covers the ring's convex outline. Positions that are not on a convex ring produce degenerate or overlapping
/// triangles, which [`BlendSpace2D::new`] reports.
fn fan_triangles(positions: &[[f32; 2]]) -> Vec<[usize; 3]> {
	let distance = |position: [f32; 2], point: [f32; 2]| (position[0] - point[0]).hypot(position[1] - point[1]);
	let centroid = |skip: Option<usize>| {
		let (sum, count) = (0..positions.len())
			.filter(|&index| Some(index) != skip)
			.fold(([0.0, 0.0], 0.0), |(sum, count), index| {
				([sum[0] + positions[index][0], sum[1] + positions[index][1]], count + 1.0)
			});
		[sum[0] / count, sum[1] / count]
	};
	// The position nearest the centroid is the centre when it sits at the centroid of the other positions, the ring,
	// to within a small fraction of the ring's size.
	let all = centroid(None);
	let nearest = (0..positions.len())
		.min_by(|&left, &right| distance(positions[left], all).total_cmp(&distance(positions[right], all)))
		.expect("a blend has samples");
	let ring_centre = centroid(Some(nearest));
	let size = positions
		.iter()
		.map(|&position| distance(position, ring_centre))
		.fold(0.0, f32::max);
	let centre = (distance(positions[nearest], ring_centre) <= size * 1.0e-3).then_some(nearest);

	let pivot = centre.map_or(all, |index| positions[index]);
	let mut order: Vec<usize> = (0..positions.len()).filter(|&index| Some(index) != centre).collect();
	order.sort_by(|&left, &right| {
		let angle = |index: usize| (positions[index][1] - pivot[1]).atan2(positions[index][0] - pivot[0]);
		angle(left).total_cmp(&angle(right))
	});
	match centre {
		Some(centre) => (0..order.len())
			.map(|index| [centre, order[index], order[(index + 1) % order.len()]])
			.collect(),
		None => order.windows(2).skip(1).map(|pair| [order[0], pair[0], pair[1]]).collect(),
	}
}

/// The `LinearGraphState` struct is a blend state whose clips sit on a line, as returned by
/// [`AnimationGraphStateBuilder::linear`].
///
/// Connect it with routes like any [`AnimationGraphState`], then keep [`Self::id`] to request it.
#[derive(Clone, Copy)]
pub struct LinearGraphState<'builder>(pub(super) AnimationGraphState<'builder>);

impl<'builder> LinearGraphState<'builder> {
	/// Returns the ID client code requests with a position on the line, through [`LinearStateId::at`].
	pub const fn id(self) -> LinearStateId {
		LinearStateId(self.0.id)
	}
}

impl<'builder> Deref for LinearGraphState<'builder> {
	type Target = AnimationGraphState<'builder>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'builder> From<LinearGraphState<'builder>> for AnimationGraphState<'builder> {
	fn from(state: LinearGraphState<'builder>) -> Self {
		state.0
	}
}

/// The `PlanarGraphState` struct is a blend state whose clips sit on a plane, as returned by
/// [`AnimationGraphStateBuilder::planar`].
///
/// Connect it with routes like any [`AnimationGraphState`], then keep [`Self::id`] to request it.
#[derive(Clone, Copy)]
pub struct PlanarGraphState<'builder>(pub(super) AnimationGraphState<'builder>);

impl<'builder> PlanarGraphState<'builder> {
	/// Returns the ID client code requests with a position on the plane, through [`PlanarStateId::at`].
	pub const fn id(self) -> PlanarStateId {
		PlanarStateId(self.0.id)
	}
}

impl<'builder> Deref for PlanarGraphState<'builder> {
	type Target = AnimationGraphState<'builder>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'builder> From<PlanarGraphState<'builder>> for AnimationGraphState<'builder> {
	fn from(state: PlanarGraphState<'builder>) -> Self {
		state.0
	}
}

/// The `LinearStateId` struct identifies a blend state on a line, so only a scalar position can be requested for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LinearStateId(AnimationStateId);

impl LinearStateId {
	/// Requests this state played at `value` on its line. Pass the request to [`AnimationGraphPlayer::advance`].
	pub const fn at(self, value: f32) -> AnimationRequest {
		AnimationRequest {
			state: self.0,
			value: BlendValue::Linear(value),
			inertialization: MediaTime::ZERO,
		}
	}
}

impl From<LinearStateId> for AnimationStateId {
	fn from(id: LinearStateId) -> Self {
		id.0
	}
}

impl From<LinearStateId> for AnimationRequest {
	fn from(id: LinearStateId) -> Self {
		id.0.into()
	}
}

/// The `PlanarStateId` struct identifies a blend state on a plane, so only a 2D position can be requested for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlanarStateId(AnimationStateId);

impl PlanarStateId {
	/// Requests this state played at `value` on its plane. Pass the request to [`AnimationGraphPlayer::advance`].
	pub const fn at(self, value: [f32; 2]) -> AnimationRequest {
		AnimationRequest {
			state: self.0,
			value: BlendValue::Planar(value),
			inertialization: MediaTime::ZERO,
		}
	}
}

impl From<PlanarStateId> for AnimationStateId {
	fn from(id: PlanarStateId) -> Self {
		id.0
	}
}

impl From<PlanarStateId> for AnimationRequest {
	fn from(id: PlanarStateId) -> Self {
		id.0.into()
	}
}

/// The `AnimationRequest` struct is what client code asks a player for each frame: a state, and where to play its
/// blend.
///
/// Make one from any state ID, or with [`LinearStateId::at`] and [`PlanarStateId::at`] for blend states, then pass
/// it to [`AnimationGraphPlayer::advance`]. A blend state requested without a position keeps its last one, and a
/// blend state the player enters on its own starts at the last position requested for a blend of its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationRequest {
	pub(super) state: AnimationStateId,
	pub(super) value: BlendValue,
	/// How long to blend toward this request's position from the pose on screen; zero changes the weights at once.
	pub(super) inertialization: MediaTime,
}

impl AnimationRequest {
	/// Returns the requested state.
	pub const fn state(&self) -> AnimationStateId {
		self.state
	}

	/// Blends a playing blend state to this request's position from the pose on screen over `duration`, instead of
	/// changing its clips' weights at once.
	///
	/// Ask for it on the frame the position jumps, like a turnaround on a direction ring, where moving the weights in
	/// one frame would pop the pose. Requests without it change the weights at once, which suits positions that move
	/// a little each frame.
	pub const fn inertialized(mut self, duration: MediaTime) -> Self {
		self.inertialization = duration;
		self
	}
}

impl From<AnimationStateId> for AnimationRequest {
	fn from(state: AnimationStateId) -> Self {
		Self {
			state,
			value: BlendValue::Unset,
			inertialization: MediaTime::ZERO,
		}
	}
}

/// The `BlendPositions` struct remembers the last position a player was given for each kind of blend.
///
/// A blend state the player enters without a requested position, like a landing a jump hands over to, starts where
/// the last blend of its kind left off rather than at the centre of its samples, where opposite directions can
/// cancel out. All linear blends share one position, and all planar blends another.
#[derive(Clone, Copy, Default)]
pub(super) struct BlendPositions {
	linear: Option<f32>,
	planar: Option<[f32; 2]>,
}

impl BlendPositions {
	/// Keeps a requested position for later blends of its kind. Non-finite positions are ignored.
	pub(super) fn remember(&mut self, value: BlendValue) {
		match value {
			BlendValue::Linear(value) if value.is_finite() => self.linear = Some(value),
			BlendValue::Planar(value) if value.iter().all(|component| component.is_finite()) => self.planar = Some(value),
			_ => {}
		}
	}

	/// Returns where `motion` starts: the last position of its kind, or the origin before the player was given one.
	pub(super) fn recall(&self, motion: &StateMotion) -> BlendValue {
		match motion {
			StateMotion::Clip(_) => BlendValue::Unset,
			StateMotion::Linear { .. } => BlendValue::Linear(self.linear.unwrap_or(0.0)),
			StateMotion::Planar { .. } => BlendValue::Planar(self.planar.unwrap_or([0.0, 0.0])),
		}
	}
}

/// The `BlendValue` enum carries a requested blend position of the dimensionality its state blends in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BlendValue {
	/// No position: a blend state keeps the one it has.
	Unset,
	Linear(f32),
	Planar([f32; 2]),
}
