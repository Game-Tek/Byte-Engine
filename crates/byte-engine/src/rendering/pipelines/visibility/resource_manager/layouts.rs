//! Pure layout calculations shared by Visibility preparation and upload recording.
//!
//! Keep these functions free of resource I/O and renderer mutation. Preparers
//! use them to size and pack staging regions; the render-thread store uses the
//! same results to build copy descriptors. This shared representation prevents
//! the two sides of the asynchronous boundary from deriving incompatible row
//! pitches, mip extents, or offsets.

use super::*;
pub(crate) use crate::rendering::resource_loading::texture::TextureUploadLayout;

/// Computes the independently uploaded 2D extent for one baked specular roughness level.
pub(super) fn environment_mip_extent(base_extent: [u32; 3], level: u32) -> Extent {
	Extent::new(
		(base_extent[0] >> level).max(1),
		(base_extent[1] >> level).max(1),
		base_extent[2].max(1),
	)
}

/// Returns the compact byte count expected for one ordinary single-mip IBL image.
pub(super) fn compact_image_byte_size(format: ghi::Formats, extent: Extent) -> usize {
	format.compact_copy_layout(extent.width().max(1), extent.height().max(1)).2
}

/// Builds one GPU image-copy descriptor that reads directly from a completed staging lease.
pub(super) fn staged_texture_copy(
	staging_data_buffer: ghi::BaseBufferHandle,
	staging_offset: usize,
	image: ghi::BaseImageHandle,
	upload: &TextureUploadLayout,
	mip_level: u32,
) -> ghi::BufferImageCopyDescriptor {
	upload.copy_descriptor(staging_data_buffer, staging_offset, image, mip_level)
}

/// Computes the compact load target and row-padded GPU copy layout for one texture lease.
pub(super) fn texture_upload_layout(format: ghi::Formats, extent: Extent, layer_count: usize) -> Option<TextureUploadLayout> {
	TextureUploadLayout::new(format, extent, layer_count, 0)
}

/// Expands compact rows backward inside their final leased range, avoiding a second CPU allocation or full-resource copy.
pub(super) fn pack_texture_rows_in_place(bytes: &mut [u8], layout: &TextureUploadLayout) {
	layout.pack_rows(bytes);
}

/// Converts a resource-management image format into the matching GHI image format.
pub(crate) fn resource_image_format_to_ghi(format: resource_management::types::Formats) -> ghi::Formats {
	crate::rendering::resource_loading::texture::resource_format_to_ghi(format)
}

/// Builds the default sampler used by visibility material textures.
pub(crate) fn default_material_sampler_builder() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.filtering_mode(ghi::FilteringModes::Linear)
		.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
		.mip_map_mode(ghi::FilteringModes::Linear)
		.addressing_mode(ghi::SamplerAddressingModes::Repeat)
		.min_lod(0f32)
		.max_lod(0f32)
}

/// Builds the clamp sampler used by spherical IES profile textures.
pub(crate) fn photometric_profile_sampler_builder() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.filtering_mode(ghi::FilteringModes::Linear)
		.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
		.mip_map_mode(ghi::FilteringModes::Linear)
		.addressing_mode(ghi::SamplerAddressingModes::Clamp)
		.min_lod(0f32)
		.max_lod(0f32)
}
