//! Motion matching: motion databases baked from `.motion.bead` files, and the search a matched animation state runs
//! to pick the frame that best continues a character's pose toward its requested motion.
//!
//! Register [`MotionDatabaseAssetHandler`] with the asset manager to bake databases; the graphics application's
//! default handlers already include it. Build a matched state with
//! [`AnimationGraphStateBuilder::matched`](super::graph::AnimationGraphStateBuilder::matched), then request it with a
//! velocity and a facing through [`MatchedStateId::toward`](super::graph::MatchedStateId::toward). The animation
//! reference's Motion databases and Matched states sections specify the features, the query, and the search.

mod bake;
mod bead;
mod features;
pub(crate) mod search;
#[cfg(test)]
pub(crate) mod tests;

pub use bake::MotionDatabaseAssetHandler;
use resource_management::resources::{animation::Animation, motion_database::MotionDatabase};

use super::math::wrap_angle;

/// Builds a motion database from clips in memory, each with its resource ID, as baking a `.motion.bead` file of
/// `source`'s settings builds one; `source`'s own `clips` are ignored.
///
/// Tests and benchmarks use it to admit a database to a pool without baking. Returns why the settings or the clips
/// can't form a database.
pub(crate) fn database_from_clips(source: &str, clips: &[(String, &Animation)]) -> Result<MotionDatabase, String> {
	let settings = bead::MotionBead::parse(source.as_bytes())?;
	features::build_database(&settings, clips).map_err(|error| error.to_string())
}

/// The most feature dimensions a motion database compares, which bounds a query to the stack.
pub(crate) const MAX_DIMENSIONS: usize = 64;

/// The `GroundRoot` struct is a motion database frame's ground root: the root node's position projected onto the
/// ground, and its yaw, in the clip's object space.
///
/// Features, root motion, and a matched state's visual pose are all measured in the root frame it defines, whose
/// origin is the ground root and whose -Z axis faces along its yaw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GroundRoot {
	pub(crate) x: f32,
	pub(crate) z: f32,
	/// The yaw about +Y, in radians, from -Z.
	pub(crate) yaw: f32,
}

impl GroundRoot {
	/// Reads a ground root as a motion database stores it: X, Z, then the yaw.
	pub(crate) const fn from_array([x, z, yaw]: [f32; 3]) -> Self {
		Self { x, z, yaw }
	}

	/// Expresses an object-space point's X and Z in this root frame.
	pub(crate) fn to_root_frame(self, x: f32, z: f32) -> (f32, f32) {
		turn_back(self.yaw, x - self.x, z - self.z)
	}
}

/// Turns the direction `(x, z)` about +Y the other way by `yaw`, as a root frame of that yaw sees it.
pub(crate) fn turn_back(yaw: f32, x: f32, z: f32) -> (f32, f32) {
	let (sin, cos) = yaw.sin_cos();
	(x * cos - z * sin, x * sin + z * cos)
}

/// Returns the ground root at `time` seconds into a clip of `frames` frames sampled at `rate`, which `frame` reads.
///
/// Between frames the position interpolates linearly and the yaw along the shorter way; times beyond the clip's ends
/// hold its first or last frame.
pub(crate) fn ground_at(rate: f32, frames: usize, time: f32, frame: impl Fn(usize) -> GroundRoot) -> GroundRoot {
	if frames < 2 {
		return frame(0);
	}
	let position = (time * rate).clamp(0.0, (frames - 1) as f32);
	let lower = (position as usize).min(frames - 2);
	let factor = position - lower as f32;
	let (lower, upper) = (frame(lower), frame(lower + 1));
	GroundRoot {
		x: lower.x + (upper.x - lower.x) * factor,
		z: lower.z + (upper.z - lower.z) * factor,
		yaw: wrap_angle(lower.yaw + wrap_angle(upper.yaw - lower.yaw) * factor),
	}
}

/// Returns -Z turned about +Y by `yaw`, as its X and Z.
pub(crate) fn direction(yaw: f32) -> [f32; 2] {
	let (sin, cos) = yaw.sin_cos();
	[-sin, -cos]
}
