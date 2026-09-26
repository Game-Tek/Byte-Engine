//! Screenshot request coordination and image encoding.
//!
//! Protocol transports submit bounded requests through [`ScreenshotBroker`].
//! The graphics application drains those requests, captures every selection of
//! a request in one frame, and completes each request exactly once. Transports
//! then encode each readback with [`ScreenshotFormat::encode`] on their own
//! thread, so encoding never stalls a frame.

use std::sync::{
	Mutex,
	mpsc::{self, Receiver, SyncSender, TrySendError},
};

use ghi::Size as _;

const SCREENSHOT_QUEUE_CAPACITY: usize = 8;

/// The maximum number of captures one request can select.
///
/// Each capture holds a CPU-readable copy of its image until the transport encodes it, so this bounds the memory one
/// request can pin.
pub const MAX_SCREENSHOT_CAPTURES: usize = 16;

/// The `ScreenshotBroker` struct bounds screenshot work shared between protocol and graphics threads.
pub struct ScreenshotBroker {
	requests: SyncSender<ScreenshotRequest>,
	receiver: Mutex<Receiver<ScreenshotRequest>>,
}

impl ScreenshotBroker {
	/// Creates a broker with the inspector screenshot queue capacity.
	pub fn new() -> Self {
		Self::with_capacity(SCREENSHOT_QUEUE_CAPACITY)
	}

	fn with_capacity(capacity: usize) -> Self {
		let (requests, receiver) = mpsc::sync_channel(capacity);
		Self {
			requests,
			receiver: Mutex::new(receiver),
		}
	}

	/// Submits captures that must come from the same frame and returns their one-shot response receiver.
	pub fn request(&self, captures: Vec<ScreenshotSelection>) -> Result<ScreenshotResponse, ScreenshotSubmitError> {
		if captures.is_empty() || captures.len() > MAX_SCREENSHOT_CAPTURES {
			return Err(ScreenshotSubmitError::CaptureCount);
		}
		let (respond, response) = mpsc::sync_channel(1);
		match self.requests.try_send(ScreenshotRequest { captures, respond }) {
			Ok(()) => Ok(response),
			Err(TrySendError::Full(_)) => Err(ScreenshotSubmitError::QueueFull),
			Err(TrySendError::Disconnected(_)) => {
				unreachable!("ScreenshotBroker owns its request receiver for its entire lifetime")
			}
		}
	}

	/// Drains currently queued work without blocking the graphics thread.
	pub fn drain(&self) -> Vec<ScreenshotRequest> {
		self.receiver
			.lock()
			.expect(
				"Screenshot request queue lock is poisoned. The most likely cause is that a graphics thread panicked while draining requests.",
			)
			.try_iter()
			.collect()
	}
}

/// The `ScreenshotSelection` struct names one image to capture, so a request can mix sinks and capture points.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenshotSelection {
	/// The zero-based renderer sink, which is the window index.
	pub sink: usize,
	/// Where in the frame the image is read from.
	pub capture: ScreenshotCapture,
}

/// The `ScreenshotCapture` enum identifies where a screenshot is transferred from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenshotCapture {
	FinalSwapchain,
	AfterPass {
		pass: String,
		target: String,
	},
	/// A named render-graph target of a scene pipeline, such as an intermediate lighting buffer.
	///
	/// The renderer reads it after every scene pipeline has recorded its work for the frame, before post-processing.
	SceneTarget {
		target: String,
	},
	/// The copy of a history target that the previous frame wrote, which is what this frame's passes read as history.
	///
	/// Only targets created with
	/// [`create_history_target`](crate::rendering::render_pass::RenderPassBuilder::create_history_target) keep
	/// one. The copy holds no usable data on a sink's first frame or right after a resize.
	PreviousSceneTarget {
		target: String,
	},
}

/// The `ScreenshotRequest` struct carries the captures of one request and its one-shot completion channel.
pub struct ScreenshotRequest {
	pub(crate) captures: Vec<ScreenshotSelection>,
	respond: SyncSender<Screenshots>,
}

impl ScreenshotRequest {
	/// Completes this request. A disconnected transport client discards the result.
	pub(crate) fn complete(self, screenshots: Screenshots) {
		let _ = self.respond.try_send(screenshots);
	}
}

/// The `Screenshots` struct carries the readbacks of one request, which all come from the same graphics submission.
pub struct Screenshots {
	/// The graphics submission that produced every capture.
	pub frame: u64,
	/// One result per requested capture, in request order.
	pub captures: Vec<Result<ghi::TextureReadback, ScreenshotError>>,
}

/// Errors reported while capturing a screenshot.
#[derive(Debug)]
pub enum ScreenshotError {
	SinkNotFound,
	SinkUnavailable,
	PassNotFound,
	PassAmbiguous,
	TargetNotWritten,
	/// The target exists but keeps no copy from the previous frame.
	TargetHasNoHistory,
	Internal(String),
}

/// The response returned to a transport after it queues one screenshot request.
pub type ScreenshotResponse = Receiver<Screenshots>;

/// Errors reported before a screenshot request enters the graphics queue.
#[derive(Debug)]
pub enum ScreenshotSubmitError {
	QueueFull,
	/// The request selects no captures, or more than [`MAX_SCREENSHOT_CAPTURES`].
	CaptureCount,
}

impl From<crate::rendering::renderer::RendererScreenshotError> for ScreenshotError {
	fn from(error: crate::rendering::renderer::RendererScreenshotError) -> Self {
		use crate::rendering::renderer::RendererScreenshotError;
		match error {
			RendererScreenshotError::SinkNotFound => Self::SinkNotFound,
			RendererScreenshotError::SinkUnavailable => Self::SinkUnavailable,
			RendererScreenshotError::PassNotFound => Self::PassNotFound,
			RendererScreenshotError::PassAmbiguous => Self::PassAmbiguous,
			RendererScreenshotError::TargetNotWritten => Self::TargetNotWritten,
			RendererScreenshotError::TargetHasNoHistory => Self::TargetHasNoHistory,
			RendererScreenshotError::Transfer(error) => Self::Internal(error.to_string()),
		}
	}
}

/// The `ScreenshotFormat` enum selects how a transport encodes a readback, trading viewer support for precision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenshotFormat {
	/// An 8-bit RGBA PNG. HDR values are clamped to `[0, 1]`, so use it to preview images.
	#[default]
	Png,
	/// A lossless OpenEXR image with the texture's own precision: half floats stay half floats, normalized values
	/// become linear 32-bit floats, and `U32` stays `U32`. Use it to inspect HDR and high-precision targets.
	Exr,
	/// The texture bytes exactly as the GPU stores them, with `bytes_per_row` row pitch.
	Raw,
}

impl ScreenshotFormat {
	/// Encodes one readback. [`Self::Raw`] returns the readback bytes without copying them.
	pub fn encode(self, readback: ghi::TextureReadback) -> Result<Vec<u8>, String> {
		match self {
			Self::Png => encode_png(&readback),
			Self::Exr => encode_exr(&readback),
			Self::Raw => Ok(readback.bytes),
		}
	}

	/// Returns the media type of an image encoded in this format.
	pub fn content_type(self) -> &'static str {
		match self {
			Self::Png => "image/png",
			Self::Exr => "image/x-exr",
			Self::Raw => "application/octet-stream",
		}
	}

	/// Returns the file extension of an image encoded in this format.
	pub fn extension(self) -> &'static str {
		match self {
			Self::Png => "png",
			Self::Exr => "exr",
			Self::Raw => "bin",
		}
	}
}

/// Returns the rows of a readback that hold pixels, without GPU row padding.
fn visible_rows(readback: &ghi::TextureReadback) -> Result<impl Iterator<Item = &[u8]>, String> {
	let height = readback.extent.height() as usize;
	let bytes_per_row = readback.bytes_per_row;
	let row_size = (readback.extent.width() as usize)
		.checked_mul(readback.format.size())
		.ok_or_else(|| "Screenshot row size overflowed. The most likely cause is an invalid sink extent.".to_string())?;
	let required = bytes_per_row
		.checked_mul(height)
		.ok_or_else(|| "Screenshot buffer size overflowed. The most likely cause is an invalid sink extent.".to_string())?;
	if bytes_per_row < row_size || readback.bytes.len() < required {
		return Err(
			"Screenshot buffer is incomplete. The most likely cause is that the GPU copy row pitch or allocation size did not match the acquired sink extent."
				.to_string(),
		);
	}
	Ok(readback
		.bytes
		.chunks_exact(bytes_per_row)
		.take(height)
		.map(move |row| &row[..row_size]))
}

/// Encodes a supported texture readback as an RGBA8 PNG image.
fn encode_png(readback: &ghi::TextureReadback) -> Result<Vec<u8>, String> {
	if !matches!(
		readback.format,
		ghi::Formats::R8UNORM
			| ghi::Formats::BGRAu8
			| ghi::Formats::BGRAsRGB
			| ghi::Formats::RGBA16UNORM
			| ghi::Formats::RGBA16F
			| ghi::Formats::RGBu11u11u10
	) {
		return Err(ghi::TextureTransferError::UnsupportedFormat(readback.format).to_string());
	}
	let width = readback.extent.width() as usize;
	let height = readback.extent.height() as usize;

	// Convert only visible pixels so GPU row padding never enters the image.
	let mut rgba = Vec::with_capacity(width * 4 * height);
	for row in visible_rows(readback)? {
		match readback.format {
			// Single-channel masks, such as contact shadows, read back as gray.
			ghi::Formats::R8UNORM => {
				for &value in row {
					rgba.extend_from_slice(&[value, value, value, 255]);
				}
			}
			ghi::Formats::BGRAu8 | ghi::Formats::BGRAsRGB => {
				for pixel in row.as_chunks::<4>().0 {
					rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
				}
			}
			ghi::Formats::RGBA16UNORM => {
				for channel in row.as_chunks::<2>().0 {
					let value = u32::from(u16::from_ne_bytes([channel[0], channel[1]]));
					rgba.push(((value * 255 + 32_767) / 65_535) as u8);
				}
			}
			// HDR intermediates are written as linear values clamped to [0, 1], without tone mapping, so each
			// channel reads back as the stored value.
			ghi::Formats::RGBA16F => {
				for channel in row.as_chunks::<2>().0 {
					let value = half::f16::from_bits(u16::from_ne_bytes([channel[0], channel[1]])).to_f32();
					let value = if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) };
					rgba.push((value * 255.0).round() as u8);
				}
			}
			// Packed HDR intermediates follow the same clamped linear convention, with opaque alpha.
			ghi::Formats::RGBu11u11u10 => {
				for pixel in row.as_chunks::<4>().0 {
					for value in unpack_r11g11b10f(u32::from_ne_bytes(*pixel)) {
						let value = if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) };
						rgba.push((value * 255.0).round() as u8);
					}
					rgba.push(255);
				}
			}
			_ => unreachable!("screenshot format was validated before encoding"),
		}
	}

	let width = u32::try_from(width)
		.map_err(|_| "Screenshot width is unsupported. The most likely cause is a sink wider than PNG permits.".to_string())?;
	let height = u32::try_from(height).map_err(|_| {
		"Screenshot height is unsupported. The most likely cause is a sink taller than PNG permits.".to_string()
	})?;
	let mut png = Vec::new();
	let mut encoder = png::Encoder::new(&mut png, width, height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	encoder.set_compression(png::Compression::Fast);
	encoder
		.write_header()
		.and_then(|mut writer| writer.write_image_data(&rgba))
		.map_err(|error| {
			format!("Screenshot PNG could not be encoded. The most likely cause is an in-memory encoder failure: {error}")
		})?;
	Ok(png)
}

/// Encodes a texture readback as a lossless single-layer OpenEXR image.
///
/// EXR stores planar channels, so this splits each visible row into one sample list per channel. A single-channel
/// format becomes the luminance channel `Y` so viewers show it as gray, like the PNG encoder does.
fn encode_exr(readback: &ghi::TextureReadback) -> Result<Vec<u8>, String> {
	use exr::prelude::FlatSamples;

	let unsupported = || ghi::TextureTransferError::UnsupportedFormat(readback.format).to_string();
	if readback.format == ghi::Formats::RGBu11u11u10 {
		return write_exr(readback, &["R", "G", "B"], packed_r11g11b10f_planes(readback)?);
	}
	let names: &[&str] = match readback.format.channel_layout() {
		ghi::ChannelLayout::R => &["Y"],
		ghi::ChannelLayout::RG => &["R", "G"],
		ghi::ChannelLayout::RGB => &["R", "G", "B"],
		ghi::ChannelLayout::RGBA => &["R", "G", "B", "A"],
		// Names follow memory order, and EXR sorts channels by name, so BGRA needs no swizzle.
		ghi::ChannelLayout::BGRA => &["B", "G", "R", "A"],
		ghi::ChannelLayout::Packed | ghi::ChannelLayout::Depth | ghi::ChannelLayout::BC => return Err(unsupported()),
	};
	let (channel_size, max) = match readback.format.channel_bit_size() {
		ghi::ChannelBitSize::Bits8 => (1, f32::from(u8::MAX)),
		ghi::ChannelBitSize::Bits16 => (2, f32::from(u16::MAX)),
		ghi::ChannelBitSize::Bits32 => (4, u32::MAX as f32),
		ghi::ChannelBitSize::Bits11_11_10 | ghi::ChannelBitSize::Compressed => return Err(unsupported()),
	};
	let planes = |convert: &dyn Fn(u32) -> f32| planar_samples(readback, names.len(), channel_size, convert, FlatSamples::F32);
	let channels = match (readback.format.encoding(), channel_size) {
		(Some(ghi::Encodings::FloatingPoint), 2) => planar_samples(
			readback,
			names.len(),
			channel_size,
			|bits| half::f16::from_bits(bits as u16),
			FlatSamples::F16,
		),
		(Some(ghi::Encodings::FloatingPoint), 4) => planes(&f32::from_bits),
		(Some(ghi::Encodings::UnsignedNormalized), _) => planes(&|bits| bits as f32 / max),
		(Some(ghi::Encodings::SignedNormalized), _) => planes(&|bits| {
			// Sign-extend from the channel width. The signed maximum is half the unsigned range, rounded down.
			let shift = 32 - channel_size * 8;
			let value = ((bits << shift) as i32 >> shift) as f32;
			(value / ((max - 1.0) / 2.0)).max(-1.0)
		}),
		(Some(ghi::Encodings::sRGB), _) => planes(&|bits| srgb_to_linear(bits as f32 / max)),
		(None, 4) if readback.format == ghi::Formats::U32 => {
			planar_samples(readback, names.len(), channel_size, |bits| bits, FlatSamples::U32)
		}
		_ => return Err(unsupported()),
	}?;
	write_exr(readback, names, channels)
}

/// Writes one EXR image whose channel `names` pair with the planar `channels`.
fn write_exr(
	readback: &ghi::TextureReadback,
	names: &[&str],
	channels: Vec<exr::prelude::FlatSamples>,
) -> Result<Vec<u8>, String> {
	use exr::prelude::{AnyChannel, AnyChannels, Image, WritableImage as _};

	let channels = AnyChannels::sort(
		names
			.iter()
			.zip(channels)
			.map(|(name, samples)| AnyChannel::new(*name, samples))
			.collect(),
	);
	let size = (readback.extent.width() as usize, readback.extent.height() as usize);
	let mut exr = std::io::Cursor::new(Vec::new());
	Image::from_channels(size, channels)
		.write()
		.to_buffered(&mut exr)
		.map_err(|error| {
			format!("Screenshot EXR could not be encoded. The most likely cause is an in-memory encoder failure: {error}")
		})?;
	Ok(exr.into_inner())
}

/// Splits the visible rows of a readback into one sample plane per channel, converting each stored value.
///
/// `convert` receives the raw channel bits, zero-extended to 32 bits, and `wrap` stores a finished plane.
fn planar_samples<T>(
	readback: &ghi::TextureReadback,
	channel_count: usize,
	channel_size: usize,
	convert: impl Fn(u32) -> T,
	wrap: fn(Vec<T>) -> exr::prelude::FlatSamples,
) -> Result<Vec<exr::prelude::FlatSamples>, String> {
	let pixel_count = (readback.extent.width() * readback.extent.height()) as usize;
	let mut planes = (0..channel_count)
		.map(|_| Vec::with_capacity(pixel_count))
		.collect::<Vec<_>>();
	for row in visible_rows(readback)? {
		for pixel in row.chunks_exact(channel_size * channel_count) {
			for (plane, value) in planes.iter_mut().zip(pixel.chunks_exact(channel_size)) {
				let bits = match *value {
					[value] => u32::from(value),
					[low, high] => u32::from(u16::from_ne_bytes([low, high])),
					[a, b, c, d] => u32::from_ne_bytes([a, b, c, d]),
					_ => unreachable!("channel sizes are 1, 2, or 4 bytes"),
				};
				plane.push(convert(bits));
			}
		}
	}
	Ok(planes.into_iter().map(wrap).collect())
}

/// Splits a packed R11G11B10F readback into red, green, and blue `f32` planes.
fn packed_r11g11b10f_planes(readback: &ghi::TextureReadback) -> Result<Vec<exr::prelude::FlatSamples>, String> {
	let pixel_count = (readback.extent.width() * readback.extent.height()) as usize;
	let mut planes = [(); 3].map(|_| Vec::with_capacity(pixel_count));
	for row in visible_rows(readback)? {
		for pixel in row.as_chunks::<4>().0 {
			for (plane, value) in planes.iter_mut().zip(unpack_r11g11b10f(u32::from_ne_bytes(*pixel))) {
				plane.push(value);
			}
		}
	}
	Ok(planes.into_iter().map(exr::prelude::FlatSamples::F32).collect())
}

/// Decodes one packed R11G11B10F texel. Red and green use 6 mantissa bits, blue 5, and all share a 5 bit exponent
/// with no sign bit.
fn unpack_r11g11b10f(bits: u32) -> [f32; 3] {
	let unpack = |value: u32, mantissa_bits: u32| {
		let exponent = (value >> mantissa_bits) & 0x1f;
		let mantissa = (value & ((1 << mantissa_bits) - 1)) as f32 / (1 << mantissa_bits) as f32;
		match exponent {
			0 => mantissa * 2f32.powi(-14),
			31 if mantissa == 0.0 => f32::INFINITY,
			31 => f32::NAN,
			_ => (1.0 + mantissa) * 2f32.powi(exponent as i32 - 15),
		}
	};
	[unpack(bits & 0x7ff, 6), unpack((bits >> 11) & 0x7ff, 6), unpack(bits >> 22, 5)]
}

/// Decodes one sRGB-encoded value in `[0, 1]` to linear light.
fn srgb_to_linear(value: f32) -> f32 {
	if value <= 0.040_45 {
		value / 12.92
	} else {
		((value + 0.055) / 1.055).powf(2.4)
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;

	fn selection(sink: usize) -> ScreenshotSelection {
		ScreenshotSelection {
			sink,
			capture: ScreenshotCapture::FinalSwapchain,
		}
	}

	#[test]
	fn broker_bounds_requests_and_keeps_duplicates_independent() {
		let broker = ScreenshotBroker::with_capacity(2);
		let first = broker.request(vec![selection(3)]).expect("queue first screenshot");
		let second = broker.request(vec![selection(3)]).expect("queue duplicate screenshot");
		assert!(matches!(
			broker.request(vec![selection(4)]),
			Err(ScreenshotSubmitError::QueueFull)
		));

		let mut requests = broker.drain();
		assert_eq!(requests.len(), 2);
		assert_eq!(requests[0].captures, [selection(3)]);
		assert_eq!(requests[1].captures, [selection(3)]);
		requests.remove(0).complete(Screenshots {
			frame: 9,
			captures: vec![Err(ScreenshotError::SinkNotFound)],
		});
		requests.remove(0).complete(Screenshots {
			frame: 10,
			captures: vec![],
		});

		assert_eq!(first.recv_timeout(Duration::from_millis(10)).unwrap().frame, 9);
		assert_eq!(second.recv_timeout(Duration::from_millis(10)).unwrap().frame, 10);
	}

	#[test]
	fn broker_keeps_a_request_capture_list_together_and_rejects_invalid_counts() {
		let broker = ScreenshotBroker::with_capacity(2);
		let captures = vec![
			selection(0),
			ScreenshotSelection {
				sink: 1,
				capture: ScreenshotCapture::PreviousSceneTarget {
					target: "Diffuse Radiance History".to_string(),
				},
			},
		];
		broker.request(captures.clone()).expect("queue two captures");

		let requests = broker.drain();
		assert_eq!(requests.len(), 1);
		assert_eq!(requests[0].captures, captures);

		assert!(matches!(broker.request(vec![]), Err(ScreenshotSubmitError::CaptureCount)));
		assert!(matches!(
			broker.request(vec![selection(0); MAX_SCREENSHOT_CAPTURES + 1]),
			Err(ScreenshotSubmitError::CaptureCount)
		));
	}

	#[test]
	fn png_converts_bgra_formats_and_ignores_pitched_padding() {
		for format in [ghi::Formats::BGRAu8, ghi::Formats::BGRAsRGB] {
			let png = encode_png(&readback(vec![10, 20, 30, 255, 99, 99, 99, 99], format, 8)).expect("encode PNG");
			assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
			assert_eq!(decode_png(&png), [30, 20, 10, 255]);
		}
	}

	#[test]
	fn png_converts_rgba16_unorm_and_ignores_pitched_padding() {
		let values = [0u16, 32_768, 65_535, 257];
		let mut bytes = values.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
		bytes.extend_from_slice(&[99; 8]);
		let png = encode_png(&readback(bytes, ghi::Formats::RGBA16UNORM, 16)).expect("encode PNG");
		assert_eq!(decode_png(&png), [0, 128, 255, 1]);
	}

	#[test]
	fn png_converts_r8_unorm_to_gray_and_ignores_pitched_padding() {
		let png = encode_png(&readback(vec![128, 99, 99, 99], ghi::Formats::R8UNORM, 4)).expect("encode PNG");
		assert_eq!(decode_png(&png), [128, 128, 128, 255]);
	}

	#[test]
	fn png_rejects_invalid_readbacks() {
		let error = encode_png(&readback(vec![0; 4], ghi::Formats::RGBA8UNORM, 4)).expect_err("reject RGBA readback");
		assert!(error.starts_with("Texture transfer format is unsupported."));

		let error = encode_png(&readback(vec![0; 3], ghi::Formats::BGRAu8, 4)).expect_err("reject incomplete row");
		assert!(error.starts_with("Screenshot buffer is incomplete."));
	}

	#[test]
	fn exr_keeps_hdr_half_floats_exactly_and_ignores_pitched_padding() {
		let values = [4.5f32, -0.25, 1000.0, 1.0].map(half::f16::from_f32);
		let mut bytes = values
			.into_iter()
			.flat_map(|value| value.to_bits().to_ne_bytes())
			.collect::<Vec<_>>();
		bytes.extend_from_slice(&[99; 8]);

		let channels = decode_exr(
			&ScreenshotFormat::Exr
				.encode(readback(bytes, ghi::Formats::RGBA16F, 16))
				.unwrap(),
		);

		assert_eq!(
			channels,
			[
				("A".to_string(), vec![1.0]),
				("B".to_string(), vec![1000.0]),
				("G".to_string(), vec![-0.25]),
				("R".to_string(), vec![4.5]),
			]
		);
	}

	#[test]
	fn exr_normalizes_integer_formats_to_linear_floats() {
		let exr = ScreenshotFormat::Exr
			.encode(readback(vec![51, 255, 0, 255], ghi::Formats::BGRAsRGB, 4))
			.unwrap();
		let channels = decode_exr(&exr);
		assert_eq!(channels[0], ("A".to_string(), vec![1.0]));
		assert_eq!(channels[1].1, [srgb_to_linear(0.2)]);
		assert_eq!(channels[2], ("G".to_string(), vec![1.0]));
		assert_eq!(channels[3], ("R".to_string(), vec![0.0]));

		let exr = ScreenshotFormat::Exr
			.encode(readback(u16::MAX.to_ne_bytes().to_vec(), ghi::Formats::R16UNORM, 2))
			.unwrap();
		assert_eq!(decode_exr(&exr), [("Y".to_string(), vec![1.0])]);
	}

	#[test]
	fn exr_rejects_formats_without_a_per_channel_layout() {
		let error = ScreenshotFormat::Exr
			.encode(readback(vec![0; 4], ghi::Formats::Depth32, 4))
			.expect_err("reject depth format");
		assert!(error.starts_with("Texture transfer format is unsupported."));
	}

	#[test]
	fn decodes_packed_r11g11b10f() {
		// 1.0 is exponent 15 with a zero mantissa in every channel; blue's exponent starts at bit 27.
		let one = (15 << 6) | (15 << 17) | (15 << 27);
		assert_eq!(unpack_r11g11b10f(one), [1.0, 1.0, 1.0]);
		// Red 0.5 (exponent 14), green 2.0 (exponent 16), blue 1.5 (exponent 15, top mantissa bit).
		let mixed = (14 << 6) | (16 << 17) | (15 << 27) | (1 << 26);
		assert_eq!(unpack_r11g11b10f(mixed), [0.5, 2.0, 1.5]);

		let exr = ScreenshotFormat::Exr
			.encode(readback(u32::to_ne_bytes(mixed).to_vec(), ghi::Formats::RGBu11u11u10, 4))
			.unwrap();
		assert_eq!(
			decode_exr(&exr),
			[("B".to_string(), vec![1.5]), ("G".to_string(), vec![2.0]), ("R".to_string(), vec![0.5])]
		);
		let png = encode_png(&readback(u32::to_ne_bytes(mixed).to_vec(), ghi::Formats::RGBu11u11u10, 4)).unwrap();
		assert!(!png.is_empty());
	}

	#[test]
	fn raw_returns_the_readback_bytes_unchanged() {
		let bytes = vec![1, 2, 3, 4, 99, 99, 99, 99];
		assert_eq!(
			ScreenshotFormat::Raw.encode(readback(bytes.clone(), ghi::Formats::RGBA8UNORM, 8)),
			Ok(bytes)
		);
	}

	fn decode_png(png: &[u8]) -> Vec<u8> {
		let mut reader = png::Decoder::new(std::io::Cursor::new(png))
			.read_info()
			.expect("read encoded PNG");
		let mut pixels = vec![0; reader.output_buffer_size().expect("finite PNG buffer")];
		let size = reader.next_frame(&mut pixels).expect("decode encoded PNG").buffer_size();
		pixels.truncate(size);
		pixels
	}

	/// Decodes every channel of the first EXR layer as `f32` samples, in the file's sorted channel order.
	fn decode_exr(exr: &[u8]) -> Vec<(String, Vec<f32>)> {
		use exr::prelude::{ReadChannels as _, ReadLayers as _};

		let image = exr::prelude::read()
			.no_deep_data()
			.largest_resolution_level()
			.all_channels()
			.first_valid_layer()
			.all_attributes()
			.from_buffered(std::io::Cursor::new(exr))
			.expect("decode encoded EXR");
		image
			.layer_data
			.channel_data
			.list
			.iter()
			.map(|channel| {
				(
					channel.name.to_string(),
					channel.sample_data.values_as_f32().collect::<Vec<_>>(),
				)
			})
			.collect()
	}

	fn readback(bytes: Vec<u8>, format: ghi::Formats, bytes_per_row: usize) -> ghi::TextureReadback {
		ghi::TextureReadback {
			bytes,
			extent: utils::Extent::rectangle(1, 1),
			format,
			bytes_per_row,
			bytes_per_image: bytes_per_row,
		}
	}
}
