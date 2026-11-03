//! The `.motion.bead` source format, as the asset formats reference's Motion databases section specifies.

use std::collections::BTreeMap;

/// The `MotionBead` struct is the authored form of a motion database: the clips it samples and how it compares them.
///
/// Unknown keys fail to parse. [`Self::validate`] checks the ranges the format requires.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MotionBead {
	#[serde(rename = "$schema", default)]
	_schema: Option<String>,
	/// Clip asset IDs and `*` or `?` patterns, in database order.
	pub(crate) clips: Vec<String>,
	pub(crate) root: GroundRootSettings,
	#[serde(default = "default_sample_rate")]
	pub(crate) sample_rate: f32,
	/// The node feature weights, by node name; the map keeps the names in ascending order, which the features follow.
	#[serde(default)]
	pub(crate) nodes: BTreeMap<String, NodeWeights>,
	#[serde(default)]
	pub(crate) trajectory: TrajectorySettings,
	#[serde(default)]
	pub(crate) search: SearchSettings,
	#[serde(default)]
	pub(crate) halflife: HalflifeSettings,
}

/// The `GroundRootSettings` struct names the node the ground root projects and how much its motion is smoothed.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroundRootSettings {
	pub(crate) node: String,
	/// Seconds of standard deviation for the position's Gaussian.
	#[serde(default = "default_position_smoothing")]
	pub(crate) position_smoothing: f32,
	/// Seconds of standard deviation for the yaw's Gaussian.
	#[serde(default = "default_facing_smoothing")]
	pub(crate) facing_smoothing: f32,
}

/// The `NodeWeights` struct weighs one node's position and velocity features.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct NodeWeights {
	pub(crate) position: f32,
	pub(crate) velocity: f32,
}

/// The `TrajectorySettings` struct sets when the trajectory features sample and how much they weigh.
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TrajectorySettings {
	pub(crate) times: Vec<f32>,
	pub(crate) position: f32,
	pub(crate) direction: f32,
}

impl Default for TrajectorySettings {
	fn default() -> Self {
		Self {
			times: vec![0.333, 0.667, 1.0],
			position: 1.0,
			direction: 1.5,
		}
	}
}

/// The `SearchSettings` struct sets how often a matched state searches and how it switches.
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SearchSettings {
	pub(crate) interval: f32,
	pub(crate) exclusion: f32,
	pub(crate) blend: f32,
}

impl Default for SearchSettings {
	fn default() -> Self {
		Self {
			interval: 0.1,
			exclusion: 0.333,
			blend: 0.2,
		}
	}
}

/// The `HalflifeSettings` struct sets how fast the predicted trajectory reaches the requested velocity and facing.
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct HalflifeSettings {
	pub(crate) velocity: f32,
	pub(crate) facing: f32,
}

impl Default for HalflifeSettings {
	fn default() -> Self {
		Self {
			velocity: 0.27,
			facing: 0.27,
		}
	}
}

fn default_sample_rate() -> f32 {
	30.0
}

fn default_position_smoothing() -> f32 {
	0.1
}

fn default_facing_smoothing() -> f32 {
	0.25
}

impl MotionBead {
	/// Parses `.motion.bead` text, or returns why it doesn't match the format.
	pub(crate) fn parse(source: &[u8]) -> Result<Self, String> {
		let source = std::str::from_utf8(source).map_err(|_| "the file is not UTF-8 text".to_owned())?;
		let value = resource_management::asset::parse_json(source).map_err(|error| format!("invalid JSON5: {error}"))?;
		let bead: Self =
			serde_json::from_value(value).map_err(|error| format!("a field that does not match the format: {error}"))?;
		bead.validate()?;
		Ok(bead)
	}

	/// Returns why a setting is out of its range, or nothing when every setting is in range.
	fn validate(&self) -> Result<(), String> {
		let finite_at_least_zero = |value: f32| value.is_finite() && value >= 0.0;
		let positive = |value: f32| value.is_finite() && value > 0.0;
		let check = |valid: bool, cause: &str| if valid { Ok(()) } else { Err(cause.to_owned()) };
		check(!self.clips.is_empty(), "that `clips` is empty")?;
		check(positive(self.sample_rate), "a `sample_rate` that is not finite and positive")?;
		check(
			finite_at_least_zero(self.root.position_smoothing) && finite_at_least_zero(self.root.facing_smoothing),
			"a `root` smoothing that is negative or not finite",
		)?;
		for (name, weights) in &self.nodes {
			if !finite_at_least_zero(weights.position) || !finite_at_least_zero(weights.velocity) {
				return Err(format!("a weight of node '{name}' that is negative or not finite"));
			}
			if weights.position == 0.0 && weights.velocity == 0.0 {
				return Err(format!("that node '{name}' has no positive weight"));
			}
		}
		let times = &self.trajectory.times;
		check(
			!times.is_empty() && times.iter().all(|time| positive(*time)) && times.windows(2).all(|pair| pair[0] < pair[1]),
			"`trajectory.times` that are empty, not positive, or not strictly ascending",
		)?;
		check(
			finite_at_least_zero(self.trajectory.position) && finite_at_least_zero(self.trajectory.direction),
			"a `trajectory` weight that is negative or not finite",
		)?;
		check(positive(self.search.interval), "a `search.interval` that is not finite and positive")?;
		check(
			finite_at_least_zero(self.search.exclusion) && finite_at_least_zero(self.search.blend),
			"a `search.exclusion` or `search.blend` that is negative or not finite",
		)?;
		check(
			positive(self.halflife.velocity) && positive(self.halflife.facing),
			"a `halflife` that is not finite and positive",
		)
	}
}
