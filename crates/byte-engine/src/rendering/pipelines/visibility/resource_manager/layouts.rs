use super::*;

pub(super) struct MipStreamName {
	bytes: [u8; 16],
	len: usize,
}

impl MipStreamName {
	/// Formats one bounded mip stream identifier into inline storage.
	pub(super) fn new(level: u32) -> Self {
		let mut bytes = [0_u8; 16];
		bytes[..4].copy_from_slice(b"mip[");
		let mut digits = [0_u8; 10];
		let mut value = level;
		let mut digit_count = 0usize;
		loop {
			digits[digit_count] = b'0' + (value % 10) as u8;
			digit_count += 1;
			value /= 10;
			if value == 0 {
				break;
			}
		}
		for index in 0..digit_count {
			bytes[4 + index] = digits[digit_count - index - 1];
		}
		let len = 5 + digit_count;
		bytes[len - 1] = b']';
		Self { bytes, len }
	}

	pub(super) fn as_str(&self) -> &str {
		std::str::from_utf8(&self.bytes[..self.len]).expect("Mip stream names contain only ASCII bytes.")
	}
}

#[derive(Clone, Copy)]
pub(super) struct TextureUploadLayout {
	pub(super) offset: usize,
	pub(super) compact_bytes_per_row: usize,
	pub(super) row_count: usize,
	pub(super) compact_bytes_per_image: usize,
	pub(super) compact_size: usize,
	pub(super) source_bytes_per_row: usize,
	pub(super) source_bytes_per_image: usize,
	pub(super) padded_size: usize,
}

/// Computes the independently uploaded extent for one material texture mip level.
pub(super) fn texture_mip_extent(base_extent: Extent, level: u32) -> Extent {
	Extent::new(
		(base_extent.width() >> level).max(1),
		(base_extent.height() >> level).max(1),
		base_extent.depth().max(1),
	)
}

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
	ghi::BufferImageCopyDescriptor::new(
		staging_data_buffer,
		staging_offset + upload.offset,
		upload.source_bytes_per_row,
		upload.source_bytes_per_image,
		image,
		mip_level,
	)
}

/// Computes the compact load target and row-padded GPU copy layout for one texture lease.
pub(super) fn texture_upload_layout(format: ghi::Formats, extent: Extent, layer_count: usize) -> Option<TextureUploadLayout> {
	let (source_bytes_per_row, row_count, compact_bytes_per_image) =
		format.compact_copy_layout(extent.width().max(1), extent.height().max(1));
	let compact_size = compact_bytes_per_image.checked_mul(layer_count)?;
	let padded_bytes_per_row = source_bytes_per_row.next_multiple_of(256);
	let source_bytes_per_image = padded_bytes_per_row.checked_mul(row_count)?;
	let padded_size = source_bytes_per_image.checked_mul(layer_count)?;
	assert_eq!(
		padded_bytes_per_row % 256,
		0,
		"Texture upload row pitch alignment mismatch. The most likely cause is that the Metal upload layout was built without 256-byte row alignment. format={format:?}, extent={extent:?}, source_bytes_per_row={source_bytes_per_row}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	assert!(
		source_bytes_per_image >= compact_bytes_per_image,
		"Texture upload padded image is smaller than compact image. The most likely cause is an invalid row count or row pitch. format={format:?}, extent={extent:?}, compact_bytes_per_image={compact_bytes_per_image}, source_bytes_per_image={source_bytes_per_image}, row_count={row_count}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	Some(TextureUploadLayout {
		offset: 0,
		compact_bytes_per_row: source_bytes_per_row,
		row_count,
		compact_bytes_per_image,
		compact_size,
		source_bytes_per_row: padded_bytes_per_row,
		source_bytes_per_image,
		padded_size,
	})
}

/// Expands compact rows backward inside their final leased range, avoiding a second CPU allocation or full-resource copy.
pub(super) fn pack_texture_rows_in_place(bytes: &mut [u8], layout: &TextureUploadLayout) {
	assert_eq!(bytes.len(), layout.padded_size);
	let layer_count = layout.compact_size / layout.compact_bytes_per_image;
	for layer in (0..layer_count).rev() {
		for row in (0..layout.row_count).rev() {
			let source = layer * layout.compact_bytes_per_image + row * layout.compact_bytes_per_row;
			let destination = layer * layout.source_bytes_per_image + row * layout.source_bytes_per_row;
			bytes.copy_within(source..source + layout.compact_bytes_per_row, destination);
		}
	}
}

/// Converts a resource-management image format into the matching GHI image format.
pub(super) fn resource_image_format_to_ghi(format: resource_management::types::Formats) -> ghi::Formats {
	match format {
		resource_management::types::Formats::RG8 => ghi::Formats::RG8UNORM,
		resource_management::types::Formats::R16F => ghi::Formats::R16F,
		resource_management::types::Formats::RGB8 => ghi::Formats::RGB8UNORM,
		resource_management::types::Formats::RGB16 => ghi::Formats::RGB16UNORM,
		resource_management::types::Formats::RGBA8 => ghi::Formats::RGBA8UNORM,
		resource_management::types::Formats::RGBA16 => ghi::Formats::RGBA16UNORM,
		resource_management::types::Formats::RGBA16F => ghi::Formats::RGBA16F,
		resource_management::types::Formats::BC5 => ghi::Formats::BC5,
		resource_management::types::Formats::BC5SNORM => ghi::Formats::BC5SNORM,
		resource_management::types::Formats::BC7 => ghi::Formats::BC7,
		resource_management::types::Formats::BC7SRGB => ghi::Formats::BC7SRGB,
	}
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
