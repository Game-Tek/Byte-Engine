use std::alloc::Allocator;

use exr::prelude::f16;
use utils::Extent;

use crate::types::{Formats, Gamma};

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
	F32NativeEndian,
}

impl SourceEncoding {
	fn bytes_per_sample(self) -> usize {
		match self {
			Self::U8 => 1,
			Self::U16LittleEndian | Self::U16BigEndian | Self::U16NativeEndian | Self::F16LittleEndian => 2,
			Self::F32NativeEndian => 4,
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
pub(crate) enum CanonicalImageData<'a, A: Allocator> {
	Borrowed(&'a [u8]),
	Owned(Box<[u8], A>),
}

impl<A: Allocator> CanonicalImageData<'_, A> {
	pub(crate) fn as_slice(&self) -> &[u8] {
		match self {
			Self::Borrowed(data) => data,
			Self::Owned(data) => data,
		}
	}
}

/// Converts a high-precision source into the linear RGBA16F surface required by environment processing.
pub(crate) fn canonicalize_rgba16f_in<A: Allocator + Clone>(
	source: ImageSource<'_>,
	gamma: Gamma,
	allocator: A,
) -> Option<CanonicalImageData<'_, A>> {
	if gamma == Gamma::Linear {
		return canonicalize_image_in(source, Formats::RGBA16F, allocator);
	}

	let pixel_count = validated_pixel_count(source)?;
	let mut output = Vec::with_capacity_in(pixel_count.checked_mul(target_stride(Formats::RGBA16F)?)?, allocator);
	append_rgba16f(source, gamma, &mut output)?;

	Some(CanonicalImageData::Owned(output.into_boxed_slice()))
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
		Formats::RGBA16F => append_rgba16f(source, Gamma::Linear, output)?,
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

/// Expands source channels, removes the RGB transfer function, and stores linear half-float RGBA pixels.
fn append_rgba16f<A: Allocator>(source: ImageSource<'_>, gamma: Gamma, output: &mut Vec<u8, A>) -> Option<()> {
	let bytes_per_sample = source.encoding.bytes_per_sample();
	let source_stride = source.channels.count().checked_mul(bytes_per_sample)?;

	for pixel in source.data.chunks_exact(source_stride) {
		let mut channels = [0.0_f32; 4];

		for (channel, bytes) in pixel.chunks_exact(bytes_per_sample).enumerate() {
			channels[channel] = read_linear_f32(bytes, source.encoding)?;
		}

		let mut rgba = match source.channels {
			SourceChannels::Luminance => [channels[0], channels[0], channels[0], 1.0],
			SourceChannels::LuminanceAlpha => [channels[0], channels[0], channels[0], channels[1]],
			SourceChannels::RGB => [channels[0], channels[1], channels[2], 1.0],
			SourceChannels::RGBA => channels,
		};
		if gamma == Gamma::SRGB {
			rgba[..3].iter_mut().for_each(|channel| *channel = srgb_to_linear(*channel));
		}

		for channel in rgba {
			output.extend_from_slice(&f16::from_f32(channel).to_le_bytes());
		}
	}

	Some(())
}

/// Removes the IEC 61966-2-1 transfer function from one normalized sRGB channel.
fn srgb_to_linear(channel: f32) -> f32 {
	if channel <= 0.040_45 {
		channel / 12.92
	} else {
		((channel + 0.055) / 1.055).powf(2.4)
	}
}

fn read_unorm8(bytes: &[u8], encoding: SourceEncoding) -> Option<u8> {
	match encoding {
		SourceEncoding::U8 => bytes.first().copied(),
		SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian => {
			Some((read_u16(bytes, encoding)? >> 8) as u8)
		}
		SourceEncoding::F16LittleEndian | SourceEncoding::F32NativeEndian => None,
	}
}

fn read_u16(bytes: &[u8], encoding: SourceEncoding) -> Option<u16> {
	let bytes = [*bytes.first()?, *bytes.get(1)?];
	match encoding {
		SourceEncoding::U16LittleEndian => Some(u16::from_le_bytes(bytes)),
		SourceEncoding::U16BigEndian => Some(u16::from_be_bytes(bytes)),
		SourceEncoding::U16NativeEndian => Some(u16::from_ne_bytes(bytes)),
		SourceEncoding::U8 | SourceEncoding::F16LittleEndian | SourceEncoding::F32NativeEndian => None,
	}
}

/// Reads one source sample as linear floating-point radiance without applying a transfer function.
fn read_linear_f32(bytes: &[u8], encoding: SourceEncoding) -> Option<f32> {
	match encoding {
		SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian => {
			Some(f32::from(read_u16(bytes, encoding)?) / f32::from(u16::MAX))
		}
		SourceEncoding::F16LittleEndian => {
			let bytes = [*bytes.first()?, *bytes.get(1)?];

			Some(f16::from_le_bytes(bytes).to_f32())
		}
		SourceEncoding::F32NativeEndian => {
			let bytes = [*bytes.first()?, *bytes.get(1)?, *bytes.get(2)?, *bytes.get(3)?];

			Some(f32::from_ne_bytes(bytes))
		}
		SourceEncoding::U8 => None,
	}
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::{
		CanonicalImageData, ImageSource, SourceChannels, SourceEncoding, canonicalize_image_in, canonicalize_rgba16f_in,
	};
	use crate::types::{Formats, Gamma};

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
	fn canonicalizes_high_precision_linear_sources_to_rgba16f() {
		let rgb16 = [0_u16, u16::MAX / 2, u16::MAX]
			.into_iter()
			.flat_map(u16::to_ne_bytes)
			.collect::<Vec<_>>();
		let canonical = canonicalize_rgba16f_in(
			ImageSource::new(
				Extent::rectangle(1, 1),
				SourceChannels::RGB,
				SourceEncoding::U16NativeEndian,
				&rgb16,
			),
			Gamma::Linear,
			Global,
		)
		.expect("16-bit RGB must normalize to RGBA16F");
		let values = canonical
			.as_slice()
			.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();

		assert_eq!(values, vec![0.0, 0.5, 1.0, 1.0]);

		let rgba16f = [0_u8; 8];
		let source = ImageSource::new(
			Extent::rectangle(1, 1),
			SourceChannels::RGBA,
			SourceEncoding::F16LittleEndian,
			&rgba16f,
		);
		let canonical = canonicalize_rgba16f_in(source, Gamma::Linear, Global).expect("RGBA16F must remain compatible");

		assert!(matches!(canonical, CanonicalImageData::Borrowed(_)));
		assert_eq!(canonical.as_slice().as_ptr(), rgba16f.as_ptr());
	}

	#[test]
	fn linearizes_srgb_rgb_without_changing_alpha() {
		let samples = [u16::MAX / 2, u16::MAX / 4, u16::MAX, u16::MAX / 8]
			.into_iter()
			.flat_map(u16::to_ne_bytes)
			.collect::<Vec<_>>();
		let source = ImageSource::new(
			Extent::rectangle(1, 1),
			SourceChannels::RGBA,
			SourceEncoding::U16NativeEndian,
			&samples,
		);
		let canonical =
			canonicalize_rgba16f_in(source, Gamma::SRGB, Global).expect("high-precision sRGB must normalize to linear RGBA16F");
		let values = canonical
			.as_slice()
			.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect::<Vec<_>>();

		assert!((values[0] - 0.214).abs() < 0.001);
		assert!((values[1] - 0.0509).abs() < 0.001);
		assert_eq!(values[2], 1.0);
		assert!((values[3] - 0.125).abs() < 0.001);
	}

	#[test]
	fn rejects_8_bit_sources_from_rgba16f_canonicalization() {
		let source = ImageSource::new(Extent::rectangle(1, 1), SourceChannels::RGBA, SourceEncoding::U8, &[0; 4]);

		assert!(canonicalize_rgba16f_in(source, Gamma::Linear, Global).is_none());
	}

	#[test]
	fn rejects_source_buffers_that_do_not_match_the_declared_layout() {
		let source = ImageSource::new(Extent::rectangle(2, 1), SourceChannels::RGBA, SourceEncoding::U8, &[0; 7]);
		assert!(canonicalize_image_in(source, Formats::RGBA8, Global).is_none());
	}
}
