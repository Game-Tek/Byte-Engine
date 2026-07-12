use crate::{FilteringModes, SamplerAddressingModes, SamplingReductionModes};

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
	/// Creates a new sampler builder.
	///
	/// Default values:
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SamplerHandle(pub(crate) u64);

#[cfg(test)]
mod tests {
	use super::Builder;
	use crate::{FilteringModes, SamplerAddressingModes, SamplingReductionModes};

	#[test]
	fn defaults_define_linear_clamped_weighted_sampling() {
		let builder = Builder::new();
		assert_eq!(builder.filtering_mode, FilteringModes::Linear);
		assert_eq!(builder.reduction_mode, SamplingReductionModes::WeightedAverage);
		assert_eq!(builder.mip_map_mode, FilteringModes::Linear);
		assert_eq!(builder.addressing_mode, SamplerAddressingModes::Clamp);
		assert_eq!(builder.anisotropy, None);
		assert_eq!(builder.min_lod, 0.0);
		assert_eq!(builder.max_lod, 0.0);
	}

	#[test]
	fn every_sampler_override_survives_builder_composition() {
		let builder = Builder::new()
			.filtering_mode(FilteringModes::Closest)
			.reduction_mode(SamplingReductionModes::Max)
			.mip_map_mode(FilteringModes::Closest)
			.addressing_mode(SamplerAddressingModes::Repeat)
			.anisotropy(8.0)
			.min_lod(2.0)
			.max_lod(6.0);

		assert_eq!(builder.filtering_mode, FilteringModes::Closest);
		assert_eq!(builder.reduction_mode, SamplingReductionModes::Max);
		assert_eq!(builder.mip_map_mode, FilteringModes::Closest);
		assert_eq!(builder.addressing_mode, SamplerAddressingModes::Repeat);
		assert_eq!(builder.anisotropy, Some(8.0));
		assert_eq!(builder.min_lod, 2.0);
		assert_eq!(builder.max_lod, 6.0);
	}
}
