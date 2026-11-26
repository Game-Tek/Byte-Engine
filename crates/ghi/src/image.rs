use std::num::NonZeroU32;

use utils::Extent;

use crate::{ClearValue, Copies, Formats, ImageGroupHandle, PrivateHandle, Uses};

/// The `Region` struct selects a rectangle in the base level of a 2D image, so a copy can update part of an image
/// that GPU work already uses.
///
/// Pass it to [`crate::BufferImageCopyDescriptor::region`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Region {
	pub offset: [u32; 2],
	pub size: [u32; 2],
}

impl Region {
	/// Checks that a partial copy addresses a nonempty, uncompressed 2D color region.
	pub(crate) fn validate(self, extent: Extent, format: Formats, layers: u32) {
		assert!(
			extent.depth() <= 1 && layers == 1 && format.bc_bytes_per_block().is_none() && !format.is_depth(),
			"Image region copy is unsupported. The most likely cause is a compressed, depth, layered, or 3D image. Copy whole mips of such images instead."
		);
		assert!(
			self.size[0] > 0
				&& self.size[1] > 0
				&& self.offset[0]
					.checked_add(self.size[0])
					.is_some_and(|end| end <= extent.width())
				&& self.offset[1]
					.checked_add(self.size[1])
					.is_some_and(|end| end <= extent.height()),
			"Image copy region is out of bounds. The most likely cause is stale image dimensions or an empty rectangle. Keep the region inside the image's base level."
		);
	}
}

/// Checks that a layer count passed to [`crate::frame::Frame::resize_image_layers`] fits the image it resizes.
pub(crate) fn assert_resizable_layers(cube_array_compatible: bool, array_layers: NonZeroU32) {
	assert!(
		!cube_array_compatible || array_layers.get().is_multiple_of(6),
		"Cube-array image cannot be resized to {array_layers} layers. The most likely cause is a layer count that is not six faces per cube."
	);
}

/// Returns the dimensions of one mip while preserving the image dimensionality; see [`Extent::mip`].
pub(crate) fn mip_extent(extent: Extent, level: u32) -> Extent {
	extent.mip(level)
}

/// The `Builder` struct defines the allocation and usage contract for an image.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) resource_uses: Uses,
	pub(crate) copies: Copies,
	pub(crate) mip_levels: u32,
	pub(crate) array_layers: Option<NonZeroU32>,
	pub(crate) cube_compatible: bool,
	pub(crate) cube_array_compatible: bool,
	pub(crate) optimized_clear_value: Option<ClearValue>,
	pub(crate) group: Option<ImageGroupHandle>,
}

impl<'a> Builder<'a> {
	/// Creates an image builder for the given format and resource uses.
	///
	/// The default image has one copy and one mip level. Its name and array-layer count are `None`, and its extent is
	/// zero.
	pub fn new(format: Formats, resource_uses: Uses) -> Self {
		Self {
			name: None,
			extent: Extent::cube(0, 0, 0),
			format,
			resource_uses,
			copies: Copies::One,
			mip_levels: 1,
			array_layers: None,
			cube_compatible: false,
			cube_array_compatible: false,
			optimized_clear_value: None,
			group: None,
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

	/// Declares how many copies of the image exist.
	///
	/// Use [`Copies::PerFrame`] for images that later frames read at a negative frame offset, such as history,
	/// through [`crate::DescriptorWrite::with_frame_offset`].
	pub fn copies(mut self, copies: Copies) -> Self {
		self.copies = copies;
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

	/// Makes the image a member of `group`, so it may share device memory with other members.
	///
	/// A member has no usable memory until [`crate::frame::Frame::place_image_group`] places the group. Only images
	/// with one copy can join a group. Next, build the image with
	/// [`crate::context::ContextCreate::build_image`].
	pub fn group(mut self, group: ImageGroupHandle) -> Self {
		self.group = Some(group);
		self
	}

	pub fn get_name(&self) -> Option<&'a str> {
		self.name
	}

	pub fn get_format(&self) -> Formats {
		self.format
	}

	/// Returns every use the image is built for, including the ones [`Self::additional_uses`] added.
	pub fn get_uses(&self) -> Uses {
		self.resource_uses
	}

	pub fn get_extent(&self) -> Extent {
		self.extent
	}

	/// Returns the number of array layers, which is 1 when none were declared.
	pub fn get_array_layers(&self) -> u32 {
		self.array_layers.map_or(1, NonZeroU32::get)
	}
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub(crate) struct ImageHandle;
}

impl PrivateHandle for ImageHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		ImageHandle::index(*self)
	}
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::mip_extent;

	#[test]
	fn mip_extent_halves_non_power_of_two_dimensions_without_changing_dimensionality() {
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 0), Extent::rectangle(17, 9));
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 1), Extent::rectangle(8, 4));
		assert_eq!(mip_extent(Extent::rectangle(17, 9), 4), Extent::rectangle(1, 1));
	}
}
