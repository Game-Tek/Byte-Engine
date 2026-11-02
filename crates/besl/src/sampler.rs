//! Sampler state that BESL fixes in source with a `sampler<{ ... }>` declaration.
//!
//! The lexer attaches a [`SamplerState`] to every texture descriptor that names a sampler, through
//! [`crate::BindingTypes::CombinedImageSampler`]. Shader generators then lower it to the platform's static sampler,
//! and emulate in shader code what the platform can't do. See the
//! [sampler specification](/docs/reference/besl/interface#samplers).

/// The `SamplerState` struct fixes how a texture descriptor filters and combines the texels it reads.
///
/// Shader generators read it from [`crate::BindingTypes::CombinedImageSampler`] to emit a static sampler, so the
/// application binds only the image. [`SamplerState::default`] holds the defaults the specification gives each
/// omitted property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serialization",
	derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub struct SamplerState {
	/// The texels a read combines within one level.
	pub filter: Filter,
	/// The levels a read combines.
	pub mip_filter: Filter,
	/// How a read combines the texels of its footprint.
	pub reduction: Reduction,
	/// What a coordinate outside `[0, 1]` reads on every axis.
	pub address: Address,
}

impl SamplerState {
	/// Returns whether reads return the smallest or largest texel of their footprint instead of a weighted average.
	pub fn reduces(&self) -> bool {
		self.reduction != Reduction::Average
	}
}

/// The `Filter` enum selects how many texels, or levels, a read combines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serialization",
	derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub enum Filter {
	/// Reads the one texel under the coordinate, or the level nearest the requested level of detail.
	Nearest,
	/// Reads the 2 × 2 texel footprint around the coordinate, or the two levels around the requested level of detail.
	#[default]
	Linear,
}

impl Filter {
	/// Returns the value's source spelling, as the `filter` and `mip_filter` sampler properties write it.
	pub fn token(self) -> &'static str {
		match self {
			Self::Nearest => "nearest",
			Self::Linear => "linear",
		}
	}
}

/// The `Reduction` enum selects how a read combines the texels of its footprint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serialization",
	derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub enum Reduction {
	/// Weights each texel by its distance from the coordinate.
	#[default]
	Average,
	/// Returns the smallest texel.
	Min,
	/// Returns the largest texel.
	Max,
}

impl Reduction {
	/// Returns the value's source spelling, as the `reduction` sampler property writes it.
	pub fn token(self) -> &'static str {
		match self {
			Self::Average => "average",
			Self::Min => "min",
			Self::Max => "max",
		}
	}
}

/// The `Address` enum selects what a coordinate outside `[0, 1]` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serialization",
	derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub enum Address {
	/// Reads the edge texel.
	#[default]
	Clamp,
	/// Wraps around.
	Repeat,
	/// Wraps around with every other repetition reversed.
	Mirror,
}

impl Address {
	/// Returns the value's source spelling, as the `address` sampler property writes it.
	pub fn token(self) -> &'static str {
		match self {
			Self::Clamp => "clamp",
			Self::Repeat => "repeat",
			Self::Mirror => "mirror",
		}
	}
}
