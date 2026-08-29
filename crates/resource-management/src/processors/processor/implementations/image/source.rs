use std::alloc::Allocator;

use utils::Extent;

use crate::types::Formats;

/// The `SourceChannels` enum describes how decoded source samples form one image pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceChannels {
	Luminance,
	LuminanceAlpha,
	RGB,
	RGBA,
}

impl SourceChannels {
	fn count(self) -> usize {
		match self {
			Self::Luminance => 1,
			Self::LuminanceAlpha => 2,
			Self::RGB => 3,
			Self::RGBA => 4,
		}
	}
}

/// The `SourceEncoding` enum identifies how one decoded channel sample is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceEncoding {
	U8,
	U16LittleEndian,
	U16BigEndian,
	U16NativeEndian,
	F16LittleEndian,
}

impl SourceEncoding {
	fn bytes_per_sample(self) -> usize {
		match self {
			Self::U8 => 1,
			Self::U16LittleEndian | Self::U16BigEndian | Self::U16NativeEndian | Self::F16LittleEndian => 2,
		}
	}
}

/// The `ImageSource` struct lends contiguous decoder output and its source layout to the common image processor.
///
/// Supply a two-dimensional extent with zero depth. The processor rejects
/// nonzero depth because this source path does not process volume images.
#[derive(Clone, Copy, Debug)]
pub struct ImageSource<'a> {
	pub extent: Extent,
	pub channels: SourceChannels,
	pub encoding: SourceEncoding,
	pub data: &'a [u8],
}

impl<'a> ImageSource<'a> {
	/// Creates a borrowed two-dimensional source view that can be passed to [`super::process_image`] or its allocator-aware variants.
	pub fn new(extent: Extent, channels: SourceChannels, encoding: SourceEncoding, data: &'a [u8]) -> Self {
		Self {
			extent,
			channels,
			encoding,
			data,
		}
	}

	/// Creates a source view for an existing processor format.
	pub fn from_format(extent: Extent, format: Formats, data: &'a [u8]) -> Option<Self> {
		let (channels, encoding) = match format {
			Formats::RGB8 => (SourceChannels::RGB, SourceEncoding::U8),
			Formats::RGBA8 => (SourceChannels::RGBA, SourceEncoding::U8),
			Formats::RGB16 => (SourceChannels::RGB, SourceEncoding::U16LittleEndian),
			Formats::RGBA16 => (SourceChannels::RGBA, SourceEncoding::U16LittleEndian),
			Formats::R16F => (SourceChannels::Luminance, SourceEncoding::F16LittleEndian),
			Formats::RGBA16F => (SourceChannels::RGBA, SourceEncoding::F16LittleEndian),
			_ => return None,
		};
		Some(Self::new(extent, channels, encoding, data))
	}

	pub(super) fn natural_format(self) -> Option<Formats> {
		match (self.channels, self.encoding) {
			(SourceChannels::Luminance | SourceChannels::RGB, SourceEncoding::U8) => Some(Formats::RGB8),
			(SourceChannels::LuminanceAlpha | SourceChannels::RGBA, SourceEncoding::U8) => Some(Formats::RGBA8),
			(
				SourceChannels::Luminance | SourceChannels::RGB,
				SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian,
			) => Some(Formats::RGB16),
			(
				SourceChannels::LuminanceAlpha | SourceChannels::RGBA,
				SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian,
			) => Some(Formats::RGBA16),
			(SourceChannels::Luminance, SourceEncoding::F16LittleEndian) => Some(Formats::R16F),
			(SourceChannels::RGBA, SourceEncoding::F16LittleEndian) => Some(Formats::RGBA16F),
			_ => None,
		}
	}
}

/// The `CanonicalImageData` enum borrows compatible decoder output and owns storage only when normalization is required.
pub(super) enum CanonicalImageData<'a, A: Allocator> {
	Borrowed(&'a [u8]),
	Owned(Box<[u8], A>),
}

impl<A: Allocator> CanonicalImageData<'_, A> {
	pub(super) fn as_slice(&self) -> &[u8] {
		match self {
			Self::Borrowed(data) => data,
			Self::Owned(data) => data,
		}
	}
}

/// Normalizes source channels and sample byte order into the surface required by mip generation and compression.
pub(super) fn canonicalize_image_in<A: Allocator + Clone>(
	source: ImageSource<'_>,
	target_format: Formats,
	allocator: A,
) -> Option<CanonicalImageData<'_, A>> {
	let pixel_count = validated_pixel_count(source)?;

	if source_can_be_borrowed(source, target_format) {
		return Some(CanonicalImageData::Borrowed(source.data));
	}

	let target_stride = target_stride(target_format)?;
	let mut output = Vec::with_capacity_in(pixel_count.checked_mul(target_stride)?, allocator);
	append_canonical_image_unchecked(source, target_format, &mut output)?;
	Some(CanonicalImageData::Owned(output.into_boxed_slice()))
}

/// Appends normalized source pixels directly to a final uncompressed image writer.
pub(super) fn append_canonical_image_in<A: Allocator>(
	source: ImageSource<'_>,
	target_format: Formats,
	output: &mut Vec<u8, A>,
) -> Option<()> {
	validated_pixel_count(source)?;
	append_canonical_image_unchecked(source, target_format, output)
}

fn validated_pixel_count(source: ImageSource<'_>) -> Option<usize> {
	if source.extent.width() == 0 || source.extent.height() == 0 || source.extent.depth() != 0 {
		return None;
	}
	let pixel_count = source.extent.width().checked_mul(source.extent.height())? as usize;
	let source_stride = source.channels.count().checked_mul(source.encoding.bytes_per_sample())?;
	if source.data.len() != pixel_count.checked_mul(source_stride)? {
		return None;
	}
	Some(pixel_count)
}

fn target_stride(target_format: Formats) -> Option<usize> {
	Some(match target_format {
		Formats::RGBA8 | Formats::RGBA8SRGB => 4,
		Formats::RGBA16 => 8,
		Formats::R16F => 2,
		Formats::RGBA16F => 8,
		_ => return None,
	})
}

fn append_canonical_image_unchecked<A: Allocator>(
	source: ImageSource<'_>,
	target_format: Formats,
	output: &mut Vec<u8, A>,
) -> Option<()> {
	if source_can_be_borrowed(source, target_format) {
		output.extend_from_slice(source.data);
		return Some(());
	}
	match target_format {
		Formats::RGBA8 | Formats::RGBA8SRGB => append_rgba8(source, output)?,
		Formats::RGBA16 => append_rgba16(source, output)?,
		_ => return None,
	}
	Some(())
}

fn source_can_be_borrowed(source: ImageSource<'_>, target_format: Formats) -> bool {
	matches!(
		(source.channels, source.encoding, target_format),
		(SourceChannels::RGBA, SourceEncoding::U8, Formats::RGBA8 | Formats::RGBA8SRGB)
			| (SourceChannels::RGBA, SourceEncoding::U16LittleEndian, Formats::RGBA16)
			| (SourceChannels::Luminance, SourceEncoding::F16LittleEndian, Formats::R16F)
			| (SourceChannels::RGBA, SourceEncoding::F16LittleEndian, Formats::RGBA16F)
	) || (cfg!(target_endian = "little")
		&& matches!(
			(source.channels, source.encoding, target_format),
			(SourceChannels::RGBA, SourceEncoding::U16NativeEndian, Formats::RGBA16)
		))
}

fn append_rgba8<A: Allocator>(source: ImageSource<'_>, output: &mut Vec<u8, A>) -> Option<()> {
	let source_stride = source.channels.count() * source.encoding.bytes_per_sample();
	for pixel in source.data.chunks_exact(source_stride) {
		let mut channels = [0_u8; 4];
		for (channel, bytes) in pixel.chunks_exact(source.encoding.bytes_per_sample()).enumerate() {
			channels[channel] = read_unorm8(bytes, source.encoding)?;
		}
		let rgba = match source.channels {
			SourceChannels::Luminance => [channels[0], channels[0], channels[0], u8::MAX],
			SourceChannels::LuminanceAlpha => [channels[0], channels[0], channels[0], channels[1]],
			SourceChannels::RGB => [channels[0], channels[1], channels[2], u8::MAX],
			SourceChannels::RGBA => channels,
		};
		output.extend_from_slice(&rgba);
	}
	Some(())
}

fn append_rgba16<A: Allocator>(source: ImageSource<'_>, output: &mut Vec<u8, A>) -> Option<()> {
	let source_stride = source.channels.count() * 2;
	for pixel in source.data.chunks_exact(source_stride) {
		let mut channels = [0_u16; 4];
		for (channel, bytes) in pixel.chunks_exact(2).enumerate() {
			channels[channel] = read_u16(bytes, source.encoding)?;
		}
		let rgba = match source.channels {
			SourceChannels::Luminance => [channels[0], channels[0], channels[0], u16::MAX],
			SourceChannels::LuminanceAlpha => [channels[0], channels[0], channels[0], channels[1]],
			SourceChannels::RGB => [channels[0], channels[1], channels[2], u16::MAX],
			SourceChannels::RGBA => channels,
		};
		for channel in rgba {
			output.extend_from_slice(&channel.to_le_bytes());
		}
	}
	Some(())
}

fn read_unorm8(bytes: &[u8], encoding: SourceEncoding) -> Option<u8> {
	match encoding {
		SourceEncoding::U8 => bytes.first().copied(),
		SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian => {
			Some((read_u16(bytes, encoding)? >> 8) as u8)
		}
		SourceEncoding::F16LittleEndian => None,
	}
}

fn read_u16(bytes: &[u8], encoding: SourceEncoding) -> Option<u16> {
	let bytes = [*bytes.first()?, *bytes.get(1)?];
	match encoding {
		SourceEncoding::U16LittleEndian => Some(u16::from_le_bytes(bytes)),
		SourceEncoding::U16BigEndian => Some(u16::from_be_bytes(bytes)),
		SourceEncoding::U16NativeEndian => Some(u16::from_ne_bytes(bytes)),
		SourceEncoding::U8 | SourceEncoding::F16LittleEndian => None,
	}
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::{CanonicalImageData, ImageSource, SourceChannels, SourceEncoding, canonicalize_image_in};
	use crate::types::Formats;

	#[test]
	fn borrows_compatible_rgba8_decoder_output() {
		let data = [1, 2, 3, 4, 5, 6, 7, 8];
		let source = ImageSource::new(Extent::rectangle(2, 1), SourceChannels::RGBA, SourceEncoding::U8, &data);
		let canonical = canonicalize_image_in(source, Formats::RGBA8, Global).expect("RGBA8 source should normalize");

		assert!(matches!(canonical, CanonicalImageData::Borrowed(_)));
		assert_eq!(canonical.as_slice().as_ptr(), data.as_ptr());
	}

	#[test]
	fn expands_luminance_and_luminance_alpha_in_the_common_writer() {
		let luminance = [10, 20];
		let canonical = canonicalize_image_in(
			ImageSource::new(
				Extent::rectangle(2, 1),
				SourceChannels::Luminance,
				SourceEncoding::U8,
				&luminance,
			),
			Formats::RGBA8,
			Global,
		)
		.expect("luminance source should normalize");
		assert_eq!(canonical.as_slice(), &[10, 10, 10, 255, 20, 20, 20, 255]);

		let luminance_alpha = [10, 30, 20, 40];
		let canonical = canonicalize_image_in(
			ImageSource::new(
				Extent::rectangle(2, 1),
				SourceChannels::LuminanceAlpha,
				SourceEncoding::U8,
				&luminance_alpha,
			),
			Formats::RGBA8,
			Global,
		)
		.expect("luminance-alpha source should normalize");
		assert_eq!(canonical.as_slice(), &[10, 10, 10, 30, 20, 20, 20, 40]);
	}

	#[test]
	fn converts_big_endian_16_bit_samples_to_little_endian_rgba() {
		let data = [0x12, 0x34];
		let canonical = canonicalize_image_in(
			ImageSource::new(
				Extent::rectangle(1, 1),
				SourceChannels::Luminance,
				SourceEncoding::U16BigEndian,
				&data,
			),
			Formats::RGBA16,
			Global,
		)
		.expect("16-bit luminance source should normalize");
		assert_eq!(canonical.as_slice(), &[0x34, 0x12, 0x34, 0x12, 0x34, 0x12, 0xff, 0xff]);
	}

	#[test]
	fn rejects_source_buffers_that_do_not_match_the_declared_layout() {
		let source = ImageSource::new(Extent::rectangle(2, 1), SourceChannels::RGBA, SourceEncoding::U8, &[0; 7]);
		assert!(canonicalize_image_in(source, Formats::RGBA8, Global).is_none());
	}
}
