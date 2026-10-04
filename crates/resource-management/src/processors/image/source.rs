use std::alloc::Allocator;

use exr::prelude::f16;
use utils::{Extent, color::srgb_to_linear};

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
	/// Creates a borrowed two-dimensional source view that can be passed to [`super::process_image_in`].
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
	pub(crate) fn as_slice(&self) -> &[u8] {
		match self {
			Self::Borrowed(data) => data,
			Self::Owned(data) => data,
		}
	}

	/// Returns the texels for in-place editing, copying borrowed decoder output into `allocator` first.
	pub(crate) fn to_mut(&mut self, allocator: A) -> &mut [u8] {
		if let Self::Borrowed(data) = *self {
			*self = Self::Owned(data.to_vec_in(allocator).into_boxed_slice());
		}
		let Self::Owned(data) = self else {
			unreachable!("borrowed texels were copied above")
		};
		data
	}
}

/// Converts a high-precision source into the linear RGBA16F surface required by environment processing.
///
/// The surface is always a copy, because environment bakes keep it as their root stream.
pub(crate) fn canonicalize_rgba16f_in<A: Allocator>(
	source: ImageSource<'_>,
	gamma: Gamma,
	allocator: A,
) -> Option<Box<[u8], A>> {
	let pixel_count = validated_pixel_count(source)?;
	let mut output = Vec::with_capacity_in(pixel_count.checked_mul(target_stride(Formats::RGBA16F)?)?, allocator);
	if gamma == Gamma::Linear {
		append_canonical_image_in(source, Formats::RGBA16F, &mut output)?;
	} else {
		append_rgba16f(source, gamma, &mut output)?;
	}

	Some(output.into_boxed_slice())
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
	append_canonical_image_in(source, target_format, &mut output)?;
	Some(CanonicalImageData::Owned(output.into_boxed_slice()))
}

/// Appends normalized source pixels directly to a final uncompressed image writer.
pub(super) fn append_canonical_image_in<A: Allocator>(
	source: ImageSource<'_>,
	target_format: Formats,
	output: &mut Vec<u8, A>,
) -> Option<()> {
	validated_pixel_count(source)?;
	if source_can_be_borrowed(source, target_format) {
		output.extend_from_slice(source.data);
		return Some(());
	}
	let encoding = source.encoding;
	match target_format {
		// Eight-bit RGB is how JPEG and most color textures decode, so it expands as whole texels, which vectorizes.
		Formats::RGBA8 | Formats::RGBA8SRGB if (source.channels, encoding) == (SourceChannels::RGB, SourceEncoding::U8) => {
			let start = output.len();
			output.resize(start + source.data.len() / 3 * 4, 0);
			for (texel, &[red, green, blue]) in output[start..].as_chunks_mut().0.iter_mut().zip(source.data.as_chunks().0) {
				*texel = [red, green, blue, u8::MAX];
			}
			Some(())
		}
		Formats::RGBA8 | Formats::RGBA8SRGB => for_each_rgba(
			source,
			encoding.bytes_per_sample(),
			u8::MAX,
			|bytes| read_unorm8(bytes, encoding),
			|rgba| output.extend_from_slice(&rgba),
		),
		Formats::RGBA16 => for_each_rgba(
			source,
			2,
			u16::MAX,
			|bytes| read_u16(bytes, encoding),
			|rgba| output.extend_from_slice(rgba.map(u16::to_le_bytes).as_flattened()),
		),
		Formats::RGBA16F => append_rgba16f(source, Gamma::Linear, output),
		_ => None,
	}
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

/// Returns the texel size of a canonical format images are converted into.
fn target_stride(target_format: Formats) -> Option<usize> {
	match target_format {
		Formats::RGBA8 | Formats::RGBA8SRGB | Formats::RGBA16 | Formats::R16F | Formats::RGBA16F => target_format.texel_bytes(),
		_ => None,
	}
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

/// Reads every source pixel's samples of `sample_bytes` bytes with `read`, spreads them over RGBA, and passes the
/// texel to `write`.
///
/// Luminance repeats into red, green, and blue, and a missing alpha is `opaque`. Returns `None` when `read` rejects a
/// sample. Each caller inlines it, so its sample size and closures are constants in the per-pixel loop.
#[inline(always)]
fn for_each_rgba<T: Copy + Default>(
	source: ImageSource<'_>,
	sample_bytes: usize,
	opaque: T,
	read: impl Fn(&[u8]) -> Option<T>,
	mut write: impl FnMut([T; 4]),
) -> Option<()> {
	for pixel in source.data.chunks_exact(source.channels.count() * sample_bytes) {
		let mut channels = [T::default(); 4];
		for (channel, bytes) in pixel.chunks_exact(sample_bytes).enumerate() {
			channels[channel] = read(bytes)?;
		}
		write(match source.channels {
			SourceChannels::Luminance => [channels[0], channels[0], channels[0], opaque],
			SourceChannels::LuminanceAlpha => [channels[0], channels[0], channels[0], channels[1]],
			SourceChannels::RGB => [channels[0], channels[1], channels[2], opaque],
			SourceChannels::RGBA => channels,
		});
	}
	Some(())
}

/// Expands source channels, removes the RGB transfer function, and stores linear half-float RGBA pixels.
fn append_rgba16f<A: Allocator>(source: ImageSource<'_>, gamma: Gamma, output: &mut Vec<u8, A>) -> Option<()> {
	let encoding = source.encoding;
	for_each_rgba(
		source,
		encoding.bytes_per_sample(),
		1.0,
		|bytes| read_linear_f32(bytes, encoding),
		|mut rgba| {
			if gamma == Gamma::SRGB {
				rgba[..3].iter_mut().for_each(|channel| *channel = srgb_to_linear(*channel));
			}
			output.extend_from_slice(rgba.map(|channel| f16::from_f32(channel).to_le_bytes()).as_flattened());
		},
	)
}

/// Reads one sample as an 8-bit unorm value. The readers are inlined so each per-sample loop compiles its encoding in.
#[inline]
fn read_unorm8(bytes: &[u8], encoding: SourceEncoding) -> Option<u8> {
	match encoding {
		SourceEncoding::U8 => bytes.first().copied(),
		SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian => {
			Some((read_u16(bytes, encoding)? >> 8) as u8)
		}
		SourceEncoding::F16LittleEndian | SourceEncoding::F32NativeEndian => None,
	}
}

#[inline]
fn read_u16(bytes: &[u8], encoding: SourceEncoding) -> Option<u16> {
	let bytes = *bytes.first_chunk()?;
	match encoding {
		SourceEncoding::U16LittleEndian => Some(u16::from_le_bytes(bytes)),
		SourceEncoding::U16BigEndian => Some(u16::from_be_bytes(bytes)),
		SourceEncoding::U16NativeEndian => Some(u16::from_ne_bytes(bytes)),
		SourceEncoding::U8 | SourceEncoding::F16LittleEndian | SourceEncoding::F32NativeEndian => None,
	}
}

/// Reads one source sample as linear floating-point radiance without applying a transfer function.
#[inline]
fn read_linear_f32(bytes: &[u8], encoding: SourceEncoding) -> Option<f32> {
	match encoding {
		SourceEncoding::U16LittleEndian | SourceEncoding::U16BigEndian | SourceEncoding::U16NativeEndian => {
			Some(f32::from(read_u16(bytes, encoding)?) / f32::from(u16::MAX))
		}
		SourceEncoding::F16LittleEndian => Some(f16::from_le_bytes(*bytes.first_chunk()?).to_f32()),
		SourceEncoding::F32NativeEndian => Some(f32::from_ne_bytes(*bytes.first_chunk()?)),
		SourceEncoding::U8 => None,
	}
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::{ImageSource, SourceChannels, SourceEncoding, canonicalize_image_in, canonicalize_rgba16f_in};
	use crate::types::{Formats, Gamma};

	/// Decodes RGBA16F bytes into their channel values.
	fn decode_f16(bytes: &[u8]) -> Vec<f32> {
		bytes
			.as_chunks::<2>()
			.0
			.iter()
			.map(|bytes| exr::prelude::f16::from_le_bytes(*bytes).to_f32())
			.collect()
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

		assert_eq!(decode_f16(&canonical), vec![0.0, 0.5, 1.0, 1.0]);

		let rgba16f = [0_u8; 8];
		let source = ImageSource::new(
			Extent::rectangle(1, 1),
			SourceChannels::RGBA,
			SourceEncoding::F16LittleEndian,
			&rgba16f,
		);
		let canonical = canonicalize_rgba16f_in(source, Gamma::Linear, Global).expect("RGBA16F must remain compatible");

		assert_eq!(*canonical, rgba16f);
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
		let values = decode_f16(&canonical);

		assert!((values[0] - 0.214).abs() < 0.001);
		assert!((values[1] - 0.0509).abs() < 0.001);
		assert_eq!(values[2], 1.0);
		assert!((values[3] - 0.125).abs() < 0.001);
	}

	#[test]
	fn rejects_source_buffers_that_do_not_match_the_declared_layout() {
		let source = ImageSource::new(Extent::rectangle(2, 1), SourceChannels::RGBA, SourceEncoding::U8, &[0; 7]);
		assert!(canonicalize_image_in(source, Formats::RGBA8, Global).is_none());
	}
}
