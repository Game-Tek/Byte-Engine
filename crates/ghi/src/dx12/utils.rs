use utils::Extent;

use crate::{ChannelBitSize, ChannelLayout, Formats};

pub(crate) fn texture_copy_layout(format: Formats, extent: Extent) -> Option<(usize, usize, usize)> {
	if let Some(layout) = format.bc_layout(extent.width(), extent.height()) {
		let bytes_per_row = layout.bytes_per_row as usize;
		let row_count = layout.blocks_h as usize;
		let bytes_per_image = layout.bytes_per_image as usize;
		return Some((bytes_per_row, row_count, bytes_per_image));
	}

	let bytes_per_pixel = bytes_per_pixel(format)?;
	let bytes_per_row = extent.width() as usize * bytes_per_pixel;
	let row_count = extent.height() as usize;
	let bytes_per_image = bytes_per_row * row_count;
	Some((bytes_per_row, row_count, bytes_per_image))
}

pub(crate) fn texture_copy_size(format: Formats, extent: Extent) -> Option<usize> {
	let (_, _, bytes_per_image) = texture_copy_layout(format, extent)?;
	Some(bytes_per_image * extent.depth() as usize)
}

pub(crate) fn bytes_per_pixel(format: Formats) -> Option<usize> {
	let channel_bytes = match format.channel_bit_size() {
		ChannelBitSize::Bits8 => 1,
		ChannelBitSize::Bits16 => 2,
		ChannelBitSize::Bits32 => 4,
		ChannelBitSize::Bits11_11_10 => 4,
		ChannelBitSize::Compressed => return None,
	};

	let channels = match format.channel_layout() {
		ChannelLayout::R => 1,
		ChannelLayout::RG => 2,
		ChannelLayout::RGB => 3,
		ChannelLayout::RGBA => 4,
		ChannelLayout::BGRA => 4,
		ChannelLayout::Depth => 1,
		ChannelLayout::Packed => 1,
		ChannelLayout::BC => return None,
	};

	Some(channel_bytes * channels)
}
