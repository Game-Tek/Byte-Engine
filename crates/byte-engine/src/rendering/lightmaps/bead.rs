//! The `.lightmap.bead` source format, as the asset formats reference's Lightmaps section specifies.

/// The `LightmapBead` struct is the authored form of a lightmap: the level it lights and how the bake traces it.
///
/// Unknown keys fail to parse. [`Self::parse`] checks the ranges the format requires.
#[derive(Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LightmapBead {
	#[serde(rename = "$schema", default)]
	_schema: Option<String>,
	/// The ID of the level whose meshes and lights the lightmap holds.
	pub(crate) level: String,
	/// The ID of the environment map whose sky lights the paths that leave the scene.
	#[serde(default)]
	pub(crate) environment: Option<String>,
	/// The lux the environment delivers to an upward-facing surface; `None` keeps the map's own values.
	#[serde(default)]
	pub(crate) illuminance: Option<f32>,
	/// Texels per meter of surface.
	#[serde(default = "default_texel_density")]
	pub(crate) texel_density: f32,
	/// Paths traced from each texel.
	#[serde(default = "default_samples")]
	pub(crate) samples: u32,
	/// The most surfaces a path hits.
	#[serde(default = "default_bounces")]
	pub(crate) bounces: u32,
	/// The image's largest side in texels.
	#[serde(default = "default_max_extent")]
	pub(crate) max_extent: u32,
}

fn default_texel_density() -> f32 {
	8.0
}

fn default_samples() -> u32 {
	256
}

fn default_bounces() -> u32 {
	3
}

fn default_max_extent() -> u32 {
	2048
}

impl LightmapBead {
	/// Parses a `.lightmap.bead` file and checks its settings, or returns the most likely cause of what is wrong with it.
	pub(crate) fn parse(source: &[u8]) -> Result<Self, String> {
		let source = std::str::from_utf8(source).map_err(|_| "that the file is not UTF-8 text".to_owned())?;
		let value = resource_management::asset::parse_json(source).map_err(|error| format!("invalid JSON5: {error}"))?;
		let bead: Self =
			serde_json::from_value(value).map_err(|error| format!("a field that doesn't match the format: {error}"))?;
		bead.validate()?;
		Ok(bead)
	}

	/// Returns why a setting is out of its range, or nothing when every setting is in range.
	fn validate(&self) -> Result<(), String> {
		let check = |valid: bool, cause: &str| if valid { Ok(()) } else { Err(cause.to_owned()) };
		check(!self.level.is_empty(), "that `level` is empty")?;
		check(
			self.environment.as_ref().is_none_or(|environment| !environment.is_empty()),
			"that `environment` is empty",
		)?;
		check(
			self.illuminance
				.is_none_or(|illuminance| illuminance.is_finite() && illuminance >= 0.0),
			"an `illuminance` that isn't finite and zero or more",
		)?;
		check(
			self.texel_density.is_finite() && self.texel_density > 0.0,
			"a `texel_density` that isn't finite and positive",
		)?;
		check(
			(1..=65_536).contains(&self.samples),
			"a `samples` count outside 1 through 65,536",
		)?;
		check((1..=16).contains(&self.bounces), "a `bounces` count outside 1 through 16")?;
		check(
			self.max_extent.is_power_of_two() && (256..=8_192).contains(&self.max_extent),
			"a `max_extent` that isn't a power of two from 256 through 8,192",
		)
	}
}

#[cfg(test)]
mod tests {
	use super::LightmapBead;

	#[test]
	fn a_bead_that_names_only_its_level_takes_the_defaults() {
		let bead = LightmapBead::parse(br#"{ $schema: "../schemas/lightmap.bead.schema.json", level: "arena.level" }"#)
			.expect("a bead that names its level should parse");

		assert_eq!(bead.level, "arena.level");
		assert_eq!((bead.environment, bead.illuminance), (None, None));
		assert_eq!(
			(bead.texel_density, bead.samples, bead.bounces, bead.max_extent),
			(8.0, 256, 3, 2048)
		);
	}

	#[test]
	fn settings_outside_their_ranges_fail_to_parse() {
		for (case, source) in [
			("no level", &br#"{ samples: 4 }"#[..]),
			("empty level", br#"{ level: "" }"#),
			("unknown key", br#"{ level: "a.level", quality: 2 }"#),
			("negative illuminance", br#"{ level: "a.level", illuminance: -1 }"#),
			("zero density", br#"{ level: "a.level", texel_density: 0 }"#),
			("no samples", br#"{ level: "a.level", samples: 0 }"#),
			("too many samples", br#"{ level: "a.level", samples: 65537 }"#),
			("too many bounces", br#"{ level: "a.level", bounces: 17 }"#),
			("odd extent", br#"{ level: "a.level", max_extent: 3000 }"#),
			("small extent", br#"{ level: "a.level", max_extent: 128 }"#),
		] {
			assert!(LightmapBead::parse(source).is_err(), "the {case} case should fail");
		}
	}
}
