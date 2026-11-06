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
	/// How many texels along its longest axis a read may combine, for footprints stretched by a grazing view.
	pub anisotropy: Anisotropy,
}

/// The number of fixed sampler states: every combination of filter, mip filter, reduction, and address, then every
/// combination of anisotropy level and address.
pub const FIXED_SAMPLER_COUNT: u32 = 48;

/// The number of fixed sampler states that don't filter anisotropically. The anisotropic ones follow them.
const ISOTROPIC_SAMPLER_COUNT: u32 = 36;

impl SamplerState {
	/// Returns whether reads return the smallest or largest texel of their footprint instead of a weighted average.
	pub fn reduces(&self) -> bool {
		self.reduction != Reduction::Average
	}

	/// Returns the index of this state among the [`FIXED_SAMPLER_COUNT`] fixed samplers:
	/// `filter + 2 × mip filter + 4 × reduction + 12 × address`, with each property numbered in declaration order.
	/// An anisotropic state, which filters linearly and averages, is at `36 + 3 × (anisotropy - 1) + address`.
	///
	/// Shader generators for heap-indexed platforms read the fixed sampler at this sampler heap index, where the
	/// runtime writes it. See the [fixed sampler table](/docs/reference/ghi#descriptor-sets).
	pub fn fixed_index(&self) -> u32 {
		if self.anisotropy != Anisotropy::Off {
			return ISOTROPIC_SAMPLER_COUNT + 3 * (self.anisotropy as u32 - 1) + self.address as u32;
		}
		let filter = |filter: Filter| filter as u32;
		filter(self.filter) + 2 * filter(self.mip_filter) + 4 * self.reduction as u32 + 12 * self.address as u32
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

/// The `Anisotropy` enum selects how many texels along its longest axis a read may combine.
///
/// A surface seen at a grazing angle covers many more texels along one screen axis than the other. An isotropic read
/// takes the level the longer axis needs, which blurs the shorter one; an anisotropic read takes up to this many
/// samples along the longer axis of a finer level instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
	feature = "serialization",
	derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub enum Anisotropy {
	/// Reads the level the footprint's longer axis needs.
	#[default]
	Off,
	/// Combines up to 2 samples.
	X2,
	/// Combines up to 4 samples.
	X4,
	/// Combines up to 8 samples.
	X8,
	/// Combines up to 16 samples.
	X16,
}

impl Anisotropy {
	/// Returns the most samples a read combines, 1 when it doesn't filter anisotropically.
	pub fn samples(self) -> u8 {
		1 << self as u8
	}

	/// Returns the level that combines up to `samples` samples, as the `anisotropy` sampler property writes it.
	pub fn from_samples(samples: u8) -> Option<Self> {
		match samples {
			1 => Some(Self::Off),
			2 => Some(Self::X2),
			4 => Some(Self::X4),
			8 => Some(Self::X8),
			16 => Some(Self::X16),
			_ => None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fixed_indices_cover_the_table_once() {
		let mut states = Vec::new();
		for address in [Address::Clamp, Address::Repeat, Address::Mirror] {
			for reduction in [Reduction::Average, Reduction::Min, Reduction::Max] {
				for filter in [Filter::Nearest, Filter::Linear] {
					for mip_filter in [Filter::Nearest, Filter::Linear] {
						states.push(SamplerState {
							filter,
							mip_filter,
							reduction,
							address,
							anisotropy: Anisotropy::Off,
						});
					}
				}
			}
			for anisotropy in [Anisotropy::X2, Anisotropy::X4, Anisotropy::X8, Anisotropy::X16] {
				states.push(SamplerState {
					address,
					anisotropy,
					..SamplerState::default()
				});
			}
		}
		let mut indices: Vec<u32> = states.iter().map(SamplerState::fixed_index).collect();
		indices.sort_unstable();
		assert_eq!(indices, (0..FIXED_SAMPLER_COUNT).collect::<Vec<_>>());
	}

	#[test]
	fn anisotropic_states_follow_the_isotropic_ones() {
		let state = SamplerState {
			address: Address::Repeat,
			anisotropy: Anisotropy::X16,
			..SamplerState::default()
		};
		assert_eq!(state.fixed_index(), 36 + 3 * 3 + 1);
		assert_eq!(Anisotropy::X16.samples(), 16);
		assert_eq!(Anisotropy::from_samples(16), Some(Anisotropy::X16));
		assert_eq!(Anisotropy::from_samples(3), None);
	}
}
