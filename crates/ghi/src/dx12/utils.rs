use crate::{ChannelBitSize, ChannelLayout, Formats};

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
