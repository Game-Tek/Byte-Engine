//! Encode one mip level into its stored format on the CPU.
//!
//! [`super::encode_mip_chain_on_cpu`] encodes every level with [`encode_level_in`], and the image processor uses it for
//! textures stored without mips. The GPU path uses it for formats it doesn't encode itself.

use std::alloc::Allocator;

use utils::Extent;

use crate::types::Formats;

/// Writes one level of `width` by `height` texels into `output`, encoded as `output_format`.
///
/// Block-compressed formats take RGBA8 texels and repeat the last row and column into partial edge blocks. Packed
/// `RG8` and `RG16` take RGBA8 and RGBA16 texels and keep the first two channels of each. Other formats take texels
/// already in `output_format` and copy them. `output` must hold exactly the level's stored size, which
/// [`super::encoded_mip_level_size`] returns. `allocator` provides scratch for padded surfaces.
pub(crate) fn encode_level_in<A: Allocator + Clone>(
	output_format: Formats,
	extent: Extent,
	data: &[u8],
	output: &mut [u8],
	allocator: A,
) {
	match output_format {
		Formats::BC5 | Formats::BC5SNORM => {
			// RgSurface expects tightly packed RG pairs, not interleaved RGBA, so B and A can't leak into the
			// second texel's channels.
			let (rg_data, width, height) = rg_surface_in(data, extent, allocator);
			let surface = intel_tex_2::RgSurface {
				data: &rg_data,
				width,
				height,
				stride: width * 2,
			};
			intel_tex_2::bc5::compress_blocks_into(&surface, output);
		}
		Formats::BC7 | Formats::BC7SRGB => {
			let (padded, width, height) = padded_rgba8_surface_in(extent, data, allocator);
			let data = padded.as_deref().unwrap_or(data);
			let surface = intel_tex_2::RgbaSurface {
				data,
				width,
				height,
				stride: width * 4,
			};
			intel_tex_2::bc7::compress_blocks_into(&bc7_settings(data), &surface, output);
		}
		Formats::RG8 => truncate_texels::<4, 2>(data, output),
		Formats::RG16 => truncate_texels::<8, 4>(data, output),
		Formats::RGB8
		| Formats::RGBA8
		| Formats::RGBA8SRGB
		| Formats::RGB16
		| Formats::RGBA16
		| Formats::R16F
		| Formats::RGBA16F => {
			output.copy_from_slice(data);
		}
	}
}

/// Selects the `fast` BC7 profile, with alpha modes only when a texel is not fully opaque.
///
/// The GPU encoder in [`super::bc7`] implements the same search, so both backends produce comparable quality.
fn bc7_settings(data: &[u8]) -> intel_tex_2::bc7::EncodeSettings {
	if data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0xFF) {
		intel_tex_2::bc7::alpha_fast_settings()
	} else {
		intel_tex_2::bc7::opaque_fast_settings()
	}
}

/// Returns a copy of an RGBA8 level padded to whole blocks, or `None` when the level already fills them.
fn padded_rgba8_surface_in<A: Allocator + Clone>(
	extent: Extent,
	data: &[u8],
	allocator: A,
) -> (Option<Box<[u8], A>>, u32, u32) {
	let width = extent.width().max(1);
	let height = extent.height().max(1);

	let expected_source_bytes = width as usize * height as usize * 4;
	assert_eq!(
		data.len(),
		expected_source_bytes,
		"BC compression source size mismatch. The most likely cause is that image format conversion did not produce one RGBA8 texel per source pixel. extent={extent:?}, width={width}, height={height}, data_len={}, expected={expected_source_bytes}",
		data.len()
	);

	let padded_width = width.next_multiple_of(4);
	let padded_height = height.next_multiple_of(4);
	if padded_width == width && padded_height == height {
		return (None, width, height);
	}

	let mut padded = zeroed_boxed_slice_in(padded_width as usize * padded_height as usize * 4, allocator);
	for y in 0..padded_height {
		let source_y = y.min(height - 1);
		for x in 0..padded_width {
			let source_x = x.min(width - 1);
			let source_offset = ((source_y * width + source_x) * 4) as usize;
			let destination_offset = ((y * padded_width + x) * 4) as usize;
			padded[destination_offset..destination_offset + 4].copy_from_slice(&data[source_offset..source_offset + 4]);
		}
	}

	(Some(padded), padded_width, padded_height)
}

/// Produces a tightly packed RG surface (2 bytes per texel) from RGBA8 data, padded to whole blocks.
///
/// Every row is repacked as one pass over texel chunks, which is what keeps a 2K normal map's repack to a few
/// milliseconds instead of the hundred a per-texel copy costs. Edge padding repeats the last row and column.
fn rg_surface_in<A: Allocator + Clone>(data: &[u8], extent: Extent, allocator: A) -> (Box<[u8], A>, u32, u32) {
	let width = extent.width().max(1) as usize;
	let height = extent.height().max(1) as usize;
	let padded_width = width.next_multiple_of(4);
	let padded_height = height.next_multiple_of(4);

	let mut padded = zeroed_boxed_slice_in(padded_width * padded_height * 2, allocator);
	let source_rows = data.as_chunks::<4>().0.chunks_exact(width);
	let mut destination_rows = padded.as_chunks_mut::<2>().0.chunks_exact_mut(padded_width);
	for (source_row, destination_row) in source_rows.zip(&mut destination_rows) {
		let (texels, pad) = destination_row.split_at_mut(width);
		for (texel, source) in texels.iter_mut().zip(source_row) {
			*texel = [source[0], source[1]];
		}
		pad.fill(texels[width - 1]);
	}
	// Rows past the image repeat the last image row.
	let (filled, pad_rows) = padded.split_at_mut(height * padded_width * 2);
	let last_row = &filled[(height - 1) * padded_width * 2..];
	for pad_row in pad_rows.chunks_exact_mut(padded_width * 2) {
		pad_row.copy_from_slice(last_row);
	}

	(padded, padded_width as u32, padded_height as u32)
}

/// Keeps the first `OUTPUT` bytes of every `SOURCE`-byte texel, so a filtered RGBA surface stores as its RG prefix.
fn truncate_texels<const SOURCE: usize, const OUTPUT: usize>(data: &[u8], output: &mut [u8]) {
	let (source, source_rest) = data.as_chunks::<SOURCE>();
	let (destination, destination_rest) = output.as_chunks_mut::<OUTPUT>();
	assert!(
		source_rest.is_empty() && destination_rest.is_empty() && source.len() == destination.len(),
		"Packed level size mismatch. The most likely cause is a filtering surface that is not one RGBA texel per stored texel: source={}, output={}",
		data.len(),
		output.len()
	);
	for (texel, stored) in source.iter().zip(destination) {
		stored.copy_from_slice(&texel[..OUTPUT]);
	}
}

fn zeroed_boxed_slice_in<A: Allocator + Clone>(len: usize, allocator: A) -> Box<[u8], A> {
	let mut buffer = Vec::with_capacity_in(len, allocator);
	buffer.resize(len, 0_u8);
	buffer.into_boxed_slice()
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::encode_level_in;
	use crate::{resources::mips::bc7::tests::decode_image, types::Formats};

	fn encode(format: Formats, extent: Extent, data: &[u8]) -> Vec<u8> {
		let mut output = vec![0; format.level_size(extent).expect("BC levels have a stored size")];
		encode_level_in(format, extent, data, &mut output, Global);
		output
	}

	#[test]
	fn bc5_ignores_blue_and_alpha() {
		// If the compressor read RGBA as 2-byte texels, B and A would become the next texel's R and G and change the
		// output, so two surfaces that differ only in B and A must compress identically.
		let extent = Extent::rectangle(4, 4);
		let opaque = [0, 1, 0xFF, 0xFF].repeat(16);
		let transparent = [0, 1, 0x00, 0x00].repeat(16);

		assert_eq!(
			encode(Formats::BC5, extent, &opaque),
			encode(Formats::BC5, extent, &transparent),
			"BC5 should ignore B and A channels"
		);
	}

	#[test]
	fn packed_rg_levels_keep_the_first_two_channels_of_each_texel() {
		let extent = Extent::rectangle(2, 1);

		assert_eq!(
			encode(Formats::RG8, extent, &[1, 2, 3, 4, 5, 6, 7, 8]),
			[1, 2, 5, 6],
			"RG8 should keep R and G of each RGBA8 texel"
		);
		assert_eq!(
			encode(
				Formats::RG16,
				extent,
				&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
			),
			[1, 2, 3, 4, 9, 10, 11, 12],
			"RG16 should keep R and G of each RGBA16 texel"
		);
	}

	#[test]
	fn partial_edge_blocks_repeat_the_last_row_and_column() {
		let (width, height) = (5_u32, 7_u32);
		let texels = (0..width * height * 4)
			.map(|value| (value * 37 % 251) as u8)
			.collect::<Vec<_>>();
		// The same level with its last column and row repeated out to whole blocks.
		let padded = (0..8 * 8)
			.flat_map(|index: u32| {
				let (x, y) = ((index % 8).min(width - 1), (index / 8).min(height - 1));
				let offset = ((y * width + x) * 4) as usize;
				texels[offset..offset + 4].to_vec()
			})
			.collect::<Vec<_>>();

		for format in [Formats::BC5, Formats::BC7] {
			assert_eq!(
				encode(format, Extent::rectangle(width, height), &texels),
				encode(format, Extent::rectangle(8, 8), &padded),
				"{format:?} edge blocks should repeat the level's last row and column"
			);
		}
	}

	#[test]
	fn bc7_keeps_translucent_alpha_and_exact_opacity() {
		let extent = Extent::rectangle(4, 4);
		let decode_alpha = |texel: [u8; 4]| {
			let decoded = decode_image(4, 4, encode(Formats::BC7, extent, &texel.repeat(16)).as_chunks().0);
			decoded.as_chunks::<4>().0.iter().map(|texel| texel[3]).collect::<Vec<_>>()
		};

		assert!(decode_alpha([200, 100, 50, 255]).iter().all(|alpha| *alpha == 255));
		assert!(decode_alpha([200, 100, 50, 128]).iter().all(|alpha| alpha.abs_diff(128) <= 1));
	}
}
