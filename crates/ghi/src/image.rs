use std::num::NonZeroU32;

use utils::Extent;

use crate::{ClearValue, DeviceAccesses, Formats, PrivateHandle, PrivateHandles, UseCases, Uses};

/// Returns the dimensions of one mip while preserving the image dimensionality.
pub(crate) fn mip_extent(extent: Extent, level: u32) -> Extent {
	Extent::new(
		extent.width().checked_shr(level).unwrap_or(0).max(1),
		if extent.height() == 0 {
			0
		} else {
			extent.height().checked_shr(level).unwrap_or(0).max(1)
		},
		if extent.depth() == 0 {
			0
		} else {
			extent.depth().checked_shr(level).unwrap_or(0).max(1)
		},
	)
}

/// The `Builder` struct defines the allocation and usage contract for an image.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) resource_uses: Uses,
	pub(crate) device_accesses: DeviceAccesses,
	pub(crate) use_case: UseCases,
	pub(crate) mip_levels: u32,
	pub(crate) array_layers: Option<NonZeroU32>,
	pub(crate) cube_compatible: bool,
	pub(crate) cube_array_compatible: bool,
	pub(crate) optimized_clear_value: Option<ClearValue>,
}

impl<'a> Builder<'a> {
	/// Creates an image builder for the given format and resource uses.
	///
	/// The default image is static, has one mip level, and uses GPU-only memory.
	/// Its name and array-layer count are `None`, and its extent is zero.
	pub fn new(format: Formats, resource_uses: Uses) -> Self {
		Self {
			name: None,
			extent: Extent::cube(0, 0, 0),
			format,
			resource_uses,
			device_accesses: DeviceAccesses::DeviceOnly,
			use_case: UseCases::STATIC,
			mip_levels: 1,
			array_layers: None,
			cube_compatible: false,
			cube_array_compatible: false,
			optimized_clear_value: None,
		}
	}

	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	/// Adds image uses while preserving every other builder option.
	pub fn additional_uses(mut self, uses: Uses) -> Self {
		self.resource_uses |= uses;
		self
	}

	pub fn extent(mut self, extent: Extent) -> Self {
		self.extent = extent;
		self
	}

	pub fn device_accesses(mut self, device_accesses: DeviceAccesses) -> Self {
		self.device_accesses = device_accesses;
		self
	}

	pub fn use_case(mut self, use_case: UseCases) -> Self {
		self.use_case = use_case;
		self
	}

	pub fn mip_levels(mut self, mip_levels: u32) -> Self {
		self.mip_levels = mip_levels;
		self
	}

	pub fn array_layers(mut self, array_layers: Option<NonZeroU32>) -> Self {
		self.array_layers = array_layers;
		self
	}

	/// Makes a six-layer 2D image usable through native cubemap views.
	pub fn cube_compatible(mut self) -> Self {
		self.array_layers = NonZeroU32::new(6);
		self.cube_compatible = true;
		self.cube_array_compatible = false;
		self
	}

	/// Makes a 2D image usable through native cubemap-array views.
	pub fn cube_array_compatible(mut self, cube_count: NonZeroU32) -> Self {
		let layers = cube_count.get().checked_mul(6).expect(
			"Cube-array image layer count is invalid. The most likely cause is that the requested cube count exceeds the supported image layer range.",
		);
		self.array_layers = NonZeroU32::new(layers);
		self.cube_compatible = false;
		self.cube_array_compatible = true;
		self
	}

	/// Declares the clear value the renderer normally uses for this attachment.
	///
	/// DX12 uses this value when allocating render-target and depth-stencil resources.
	pub fn optimized_clear_value(mut self, clear_value: ClearValue) -> Self {
		self.optimized_clear_value = Some(clear_value);
		self
	}

	pub fn get_name(&self) -> Option<&'a str> {
		self.name
	}

	pub fn get_format(&self) -> Formats {
		self.format
	}
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct ImageHandle(pub(crate) u64);

impl From<ImageHandle> for PrivateHandles {
	fn from(val: ImageHandle) -> Self {
		PrivateHandles::Image(val)
	}
}

impl PrivateHandle for ImageHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

#[cfg(test)]
mod tests {
	use std::num::NonZeroU32;

	use utils::Extent;

	use super::{Builder, mip_extent};
	use crate::{Formats, Uses};

	#[test]
	fn cube_array_builder_uses_six_layers_per_cube() {
		let builder = Builder::new(Formats::Depth16, Uses::Image)
			.extent(Extent::square(64))
			.cube_array_compatible(NonZeroU32::new(4).expect("nonzero cube count"));

		assert_eq!(builder.array_layers, NonZeroU32::new(24));
		assert!(!builder.cube_compatible);
		assert!(builder.cube_array_compatible);
	}

	#[test]
	fn mip_extent_halves_non_power_of_two_dimensions_without_changing_dimensionality() {
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 0), Extent::rectangle(17, 9));
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 1), Extent::rectangle(8, 4));
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 4), Extent::rectangle(1, 1));
	}
}
