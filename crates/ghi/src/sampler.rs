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
/// A shader declares one for a texture through a BESL `sampler` declaration. Attach it to the texture's
/// [`crate::ShaderResourceDescriptor`] with [`crate::ShaderResourceDescriptor::static_sampler`], and each backend
/// builds it into the pipeline: Vulkan embeds it in the descriptor mapping, DX12 adds it to the root signature, and
/// Metal compiles it into the shader. It reads every level of the image, so it has no level-of-detail clamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StaticSampler {
	pub filtering_mode: FilteringModes,
	pub mip_map_mode: FilteringModes,
	pub reduction_mode: SamplingReductionModes,
	pub addressing_mode: SamplerAddressingModes,
	/// The most texels a read takes along the footprint's longest axis. `1` turns anisotropic filtering off.
	pub max_anisotropy: u8,
}
