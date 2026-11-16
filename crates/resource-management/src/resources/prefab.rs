//! Baked prefabs: a level's entities plus the parameters an instance sets.
//!
//! Author a `.prefab` file and bake it into a [`Prefab`]. Its entities are laid out like a [`Level`]'s, in the same
//! payload, and its parameters' defaults are stored with the level's values. A payload refers to a parameter with a
//! [`Token::Parameter`](super::level::Token::Parameter) that holds the parameter's index in [`Prefab::parameters`].

use super::level::{Level, NamedValue};

/// The `Prefab` struct lets an application spawn one reusable group of entities many times, each with its own values.
///
/// Request it by the `.prefab` file's ID and hand it to the engine's spawner, directly or through an instance in a
/// level. Every parameter has a default, so a spawn that sets none still decodes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Prefab {
	/// The prefab's entities, laid out like a level's.
	pub level: Level,
	/// Each parameter's name and where its default starts in the level's values, in file order.
	pub parameters: Vec<NamedValue>,
}

super::impl_direct_resource!(Prefab, "Prefab", references = instance_resources);

/// Returns the prefabs the prefab's own instances place, so tools can follow them.
fn instance_resources(prefab: &Prefab) -> Vec<&str> {
	prefab
		.level
		.instances
		.iter()
		.map(|instance| instance.resource.as_str())
		.collect()
}

/// The `ParameterValues` struct holds parameter values an application sets in code, encoded as a prefab's values
/// are, so a spawn reads them the same way.
///
/// Build it with `encode_parameter_values` from the prefab asset handler, then hand it to the spawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParameterValues {
	/// Each value's parameter name and where it starts in `values`.
	pub names: Vec<NamedValue>,
	/// The values, one after another, as tokens.
	pub values: Vec<u8>,
	/// The distinct strings and keys of the values, as a strings section.
	pub strings: Vec<u8>,
}
