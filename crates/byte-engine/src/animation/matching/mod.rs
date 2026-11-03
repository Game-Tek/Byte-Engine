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

/// Turns the direction `(x, z)` about +Y by `yaw`, the inverse of [`turn_back`].
pub(crate) fn turn(yaw: f32, x: f32, z: f32) -> (f32, f32) {
	turn_back(-yaw, x, z)
}

/// The `Cycle` struct is a looping clip's movement over one duration, which continues its ground root past its ends:
/// a turn about +Y, then a translation on X and Z, in the clip's object space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Cycle {
	pub(crate) x: f32,
	pub(crate) z: f32,
	pub(crate) yaw: f32,
}

impl Cycle {
	/// Returns the cycle that carries `start` onto `end`.
	pub(crate) fn between(start: GroundRoot, end: GroundRoot) -> Self {
		let yaw = end.yaw - start.yaw;
		let (x, z) = turn(yaw, start.x, start.z);
		Self {
			x: end.x - x,
			z: end.z - z,
			yaw,
		}
	}

	/// Reads a cycle as a motion database stores it: X, Z, then the yaw.
	pub(crate) const fn from_array([x, z, yaw]: [f32; 3]) -> Self {
		Self { x, z, yaw }
	}

	pub(crate) const fn to_array(self) -> [f32; 3] {
		[self.x, self.z, self.yaw]
	}

	/// Returns `root` moved by the cycle `times` times, or moved back for a negative count.
	pub(crate) fn moved(self, root: GroundRoot, times: i32) -> GroundRoot {
		let mut root = root;
		for _ in 0..times.unsigned_abs() {
			root = if times > 0 {
				let (x, z) = turn(self.yaw, root.x, root.z);
				GroundRoot {
					x: x + self.x,
					z: z + self.z,
					yaw: wrap_angle(root.yaw + self.yaw),
				}
			} else {
				let (x, z) = turn_back(self.yaw, root.x - self.x, root.z - self.z);
				GroundRoot {
					x,
					z,
					yaw: wrap_angle(root.yaw - self.yaw),
				}
			};
		}
		root
	}
}

/// The `Loop` struct is what continues a looping clip's ground root past its ends: its duration and its [`Cycle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Loop {
	pub(crate) duration: f32,
	pub(crate) cycle: Cycle,
}

/// Returns the ground root at `time` seconds into a clip of `frames` frames sampled at `rate`, which `frame` reads.
///
/// Between frames the position interpolates linearly and the yaw along the shorter way. A clip that plays once holds
/// its first or last frame past its ends; a looping clip continues by its cycle, interpolating from its last frame
/// toward its first moved by the cycle at its duration.
pub(crate) fn ground_at(
	rate: f32,
	frames: usize,
	looping: Option<Loop>,
	time: f32,
	frame: impl Fn(usize) -> GroundRoot,
) -> GroundRoot {
	let Some(Loop { duration, cycle }) = looping.filter(|looping| looping.duration > 0.0) else {
		if frames < 2 {
			return frame(0);
		}
		let position = (time * rate).clamp(0.0, (frames - 1) as f32);
		let lower = (position as usize).min(frames - 2);
		return interpolate(frame(lower), frame(lower + 1), position - lower as f32);
	};
	let cycles = (time / duration).floor();
	let within = (time - cycles * duration).clamp(0.0, duration);
	let last = (frames - 1) as f32 / rate;
	let root = if within <= last {
		let position = within * rate;
		let lower = (position as usize).min(frames.saturating_sub(2));
		match frames {
			1 => frame(0),
			_ => interpolate(frame(lower), frame(lower + 1), position - lower as f32),
		}
	} else {
		// The last frame leads into the next cycle's first, which stands at the clip's duration.
		let factor = (within - last) / (duration - last);
		interpolate(frame(frames - 1), cycle.moved(frame(0), 1), factor)
	};
	cycle.moved(root, cycles as i32)
}

/// Interpolates between two ground roots: the position linearly and the yaw along the shorter way.
fn interpolate(lower: GroundRoot, upper: GroundRoot, factor: f32) -> GroundRoot {
	GroundRoot {
		x: lower.x + (upper.x - lower.x) * factor,
		z: lower.z + (upper.z - lower.z) * factor,
		yaw: wrap_angle(lower.yaw + wrap_angle(upper.yaw - lower.yaw) * factor),
	}
}

/// The `PredictedVelocity` struct is the velocity and acceleration a matched state's trajectory starts from, which a
/// critically damped spring moves toward the requested velocity step by step, whatever the playing clip does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PredictedVelocity {
	/// X and Z, in object-space units per second.
	pub(crate) velocity: [f32; 2],
	/// X and Z, in object-space units per second squared.
	pub(crate) acceleration: [f32; 2],
}

impl PredictedVelocity {
	/// Returns the prediction turned about +Y the other way by `yaw`, as the entity sees it after turning by it.
	pub(crate) fn turned(self, yaw: f32) -> Self {
		let (sin, cos) = yaw.sin_cos();
		// As `turn_back` turns, with the sine and cosine shared by both vectors.
		let turned = |[x, z]: [f32; 2]| [x * cos - z * sin, x * sin + z * cos];
		Self {
			velocity: turned(self.velocity),
			acceleration: turned(self.acceleration),
		}
	}

	/// Returns the prediction `seconds` later, moved toward `goal` by a spring of `damping`, 2 × ln 2 over its
	/// halflife.
	pub(crate) fn toward(self, goal: [f32; 2], damping: f32, seconds: f32) -> Self {
		let decay = (-damping * seconds).exp();
		let mut next = self;
		for axis in 0..2 {
			let offset = self.velocity[axis] - goal[axis];
			let pull = self.acceleration[axis] + offset * damping;
			next.velocity[axis] = goal[axis] + decay * (offset + pull * seconds);
			next.acceleration[axis] = decay * (self.acceleration[axis] - pull * damping * seconds);
		}
		next
	}

	/// Returns where the spring that moves the prediction toward `goal` travels in `seconds`, on X and Z.
	pub(crate) fn travel(self, goal: [f32; 2], damping: f32, seconds: f32) -> [f32; 2] {
		let decay = (-damping * seconds).exp();
		std::array::from_fn(|axis| {
			let offset = self.velocity[axis] - goal[axis];
			let pull = self.acceleration[axis] + offset * damping;
			let damping_squared = damping * damping;
			goal[axis] * seconds + offset / damping + pull / damping_squared
				- decay * (pull / damping_squared + (offset + pull * seconds) / damping)
		})
	}
}

/// Returns -Z turned about +Y by `yaw`, as its X and Z.
pub(crate) fn direction(yaw: f32) -> [f32; 2] {
	let (sin, cos) = yaw.sin_cos();
	[-sin, -cos]
}
