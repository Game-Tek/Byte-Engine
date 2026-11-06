use crate::{FilteringModes, SamplerAddressingModes, SamplingReductionModes};

/// A maximum level of detail that leaves the mip range to each sampled image.
///
/// Pass it to [`Builder::max_lod`] when one sampler serves images with different mip counts. Vulkan names this
/// value `VK_LOD_CLAMP_NONE`.
pub const UNCLAMPED_MAX_LOD: f32 = 1000.0;

pub struct Builder {
	pub(crate) filtering_mode: FilteringModes,
	pub(crate) reduction_mode: SamplingReductionModes,
	pub(crate) mip_map_mode: FilteringModes,
	pub(crate) addressing_mode: SamplerAddressingModes,
	pub(crate) anisotropy: Option<f32>,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

impl Builder {
	/// Creates a sampler builder with linear, weighted-average filtering and clamp addressing.
	///
	/// # Defaults
	///
	/// - `filtering_mode`: `FilteringModes::Linear`
	/// - `reduction_mode`: `SamplingReductionModes::WeightedAverage`
	/// - `mip_map_mode`: `FilteringModes::Linear`
	/// - `addressing_mode`: `SamplerAddressingModes::Clamp`
	/// - `anisotropy`: `None`
	/// - `min_lod`: `0.0`
	/// - `max_lod`: `0.0`
	pub fn new() -> Self {
		Self {
			filtering_mode: FilteringModes::Linear,
			reduction_mode: SamplingReductionModes::WeightedAverage,
			mip_map_mode: FilteringModes::Linear,
			addressing_mode: SamplerAddressingModes::Clamp,
			anisotropy: None,
			min_lod: 0.0,
			max_lod: 0.0,
		}
	}

	pub fn filtering_mode(mut self, filtering_mode: FilteringModes) -> Self {
		self.filtering_mode = filtering_mode;
		self
	}

	pub fn reduction_mode(mut self, reduction_mode: SamplingReductionModes) -> Self {
		self.reduction_mode = reduction_mode;
		self
	}

	pub fn mip_map_mode(mut self, mip_map_mode: FilteringModes) -> Self {
		self.mip_map_mode = mip_map_mode;
		self
	}

	pub fn addressing_mode(mut self, addressing_mode: SamplerAddressingModes) -> Self {
		self.addressing_mode = addressing_mode;
		self
	}

	pub fn anisotropy(mut self, anisotropy: f32) -> Self {
		self.anisotropy = Some(anisotropy);
		self
	}

	pub fn min_lod(mut self, min_lod: f32) -> Self {
		self.min_lod = min_lod;
		self
	}

	pub fn max_lod(mut self, max_lod: f32) -> Self {
		self.max_lod = max_lod;
		self
	}
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub(crate) struct SamplerHandle;
}

/// The `StaticSampler` struct describes a sampler a shader fixes, so the application binds only the image it samples.
///
/// A shader declares one for a texture through a BESL `sampler` declaration, and the set holds only the image. Metal
/// compiles the sampler into the shader. Vulkan and DX12 write every fixed sampler once, at the start of the sampler
/// heap, and shaders read it at the index its state decides, as the
/// [fixed sampler table](/docs/reference/ghi#descriptor-sets) specifies. It reads every level of the image, so it has
/// no level-of-detail clamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StaticSampler {
	pub filtering_mode: FilteringModes,
	pub mip_map_mode: FilteringModes,
	pub reduction_mode: SamplingReductionModes,
	pub addressing_mode: SamplerAddressingModes,
	/// The most samples a read combines along its footprint's longer axis, 1 when it doesn't filter anisotropically.
	pub max_anisotropy: u8,
}

/// How many fixed samplers lead the sampler heap of the platforms that index samplers: every combination of filter,
/// mip filter, reduction, and address, then every combination of anisotropy level and address.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) const FIXED_SAMPLER_COUNT: u32 = 48;

/// How many of the fixed samplers don't filter anisotropically. The anisotropic ones follow them.
#[cfg(any(target_os = "windows", target_os = "linux"))]
const ISOTROPIC_SAMPLER_COUNT: u32 = 36;

// Metal compiles fixed samplers into its shaders, so only the platforms that index samplers build the table.
#[cfg(any(target_os = "windows", target_os = "linux"))]
impl StaticSampler {
	/// Returns the fixed sampler at sampler heap `index`, as the
	/// [fixed sampler table](/docs/reference/ghi#descriptor-sets) lays them out: `filter + 2 × mip filter +
	/// 4 × reduction + 12 × address` for the first 36, then `36 + 3 × level + address` for linear, averaging samplers
	/// that combine up to 2, 4, 8, and 16 samples.
	pub(crate) fn fixed(index: u32) -> Self {
		let address = |address: u32| match address {
			0 => SamplerAddressingModes::Clamp,
			1 => SamplerAddressingModes::Repeat,
			_ => SamplerAddressingModes::Mirror,
		};
		if let Some(anisotropic) = index.checked_sub(ISOTROPIC_SAMPLER_COUNT) {
			return Self {
				filtering_mode: FilteringModes::Linear,
				mip_map_mode: FilteringModes::Linear,
				reduction_mode: SamplingReductionModes::WeightedAverage,
				addressing_mode: address(anisotropic % 3),
				max_anisotropy: 2 << (anisotropic / 3),
			};
		}
		let filter = |bit: u32| match bit {
			0 => FilteringModes::Closest,
			_ => FilteringModes::Linear,
		};
		Self {
			filtering_mode: filter(index % 2),
			mip_map_mode: filter(index / 2 % 2),
			reduction_mode: match index / 4 % 3 {
				0 => SamplingReductionModes::WeightedAverage,
				1 => SamplingReductionModes::Min,
				_ => SamplingReductionModes::Max,
			},
			addressing_mode: address(index / 12),
			max_anisotropy: 1,
		}
	}
}
