//! Screenshot request coordination and PNG encoding.
//!
//! Protocol transports submit bounded requests through [`ScreenshotBroker`].
//! The graphics application drains those requests, captures the selected sinks,
//! and completes each request exactly once.

use std::sync::{
	Mutex,
	mpsc::{self, Receiver, SyncSender, TrySendError},
};

const SCREENSHOT_QUEUE_CAPACITY: usize = 8;

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

	/// Submits one capture and returns its one-shot response receiver.
	pub fn request(&self, sink: usize, capture: ScreenshotCapture) -> Result<ScreenshotResponse, ScreenshotSubmitError> {
		let (respond, response) = mpsc::sync_channel(1);
		match self.requests.try_send(ScreenshotRequest { sink, capture, respond }) {
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

/// The `ScreenshotCapture` enum identifies where a screenshot is transferred from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenshotCapture {
	FinalSwapchain,
	AfterPass { pass: String, target: String },
}

/// The `ScreenshotRequest` struct carries one selected capture and its one-shot completion channel.
pub struct ScreenshotRequest {
	pub(crate) sink: usize,
	pub(crate) capture: ScreenshotCapture,
	respond: SyncSender<ScreenshotResult>,
}

impl ScreenshotRequest {
	/// Completes this request. A disconnected transport client discards the result.
	pub(crate) fn complete(self, result: ScreenshotResult) {
		let _ = self.respond.try_send(result);
	}
}

/// The `Screenshot` struct carries an encoded image and its graphics submission identity.
pub struct Screenshot {
	pub frame: u64,
	pub png: Vec<u8>,
}

/// Errors reported while capturing or encoding a screenshot.
#[derive(Debug)]
pub enum ScreenshotError {
	SinkNotFound,
	SinkUnavailable,
	PassNotFound,
	PassAmbiguous,
	TargetNotWritten,
	Internal(String),
}

/// The result produced after the graphics application handles one screenshot request.
pub type ScreenshotResult = Result<Screenshot, ScreenshotError>;

/// The response returned to a transport after it queues one screenshot request.
pub type ScreenshotResponse = Receiver<ScreenshotResult>;

/// Errors reported before a screenshot request enters the graphics queue.
#[derive(Debug)]
pub enum ScreenshotSubmitError {
	QueueFull,
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
			RendererScreenshotError::Transfer(error) => Self::Internal(error.to_string()),
		}
	}
}

/// Encodes a supported texture readback as an RGBA8 PNG image.
pub(crate) fn encode_screenshot_png(readback: ghi::TextureReadback) -> Result<Vec<u8>, String> {
	let bytes_per_pixel = match readback.format {
		ghi::Formats::BGRAu8 | ghi::Formats::BGRAsRGB => 4,
		ghi::Formats::RGBA16UNORM => 8,
		_ => return Err(ghi::TextureTransferError::UnsupportedFormat(readback.format).to_string()),
	};
	let width = readback.extent.width() as usize;
	let height = readback.extent.height() as usize;
	let bytes_per_row = readback.bytes_per_row;
	let row_size = width
		.checked_mul(bytes_per_pixel)
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

	// Convert only visible pixels so GPU row padding never enters the image.
	let mut rgba = Vec::with_capacity(width * 4 * height);
	for row in readback.bytes.chunks_exact(bytes_per_row).take(height) {
		match readback.format {
			ghi::Formats::BGRAu8 | ghi::Formats::BGRAsRGB => {
				for pixel in row[..row_size].chunks_exact(4) {
					rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
				}
			}
			ghi::Formats::RGBA16UNORM => {
				for channel in row[..row_size].chunks_exact(2) {
					let value = u32::from(u16::from_ne_bytes([channel[0], channel[1]]));
					rgba.push(((value * 255 + 32_767) / 65_535) as u8);
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

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::*;

	#[test]
	fn broker_bounds_requests_and_keeps_duplicates_independent() {
		let broker = ScreenshotBroker::with_capacity(2);
		let first = broker
			.request(3, ScreenshotCapture::FinalSwapchain)
			.expect("queue first screenshot");
		let second = broker
			.request(3, ScreenshotCapture::FinalSwapchain)
			.expect("queue duplicate screenshot");
		assert!(matches!(
			broker.request(4, ScreenshotCapture::FinalSwapchain),
			Err(ScreenshotSubmitError::QueueFull)
		));

		let mut requests = broker.drain();
		assert_eq!(requests.len(), 2);
		assert_eq!(requests[0].sink, 3);
		assert_eq!(requests[0].capture, ScreenshotCapture::FinalSwapchain);
		assert_eq!(requests[1].sink, 3);
		requests.remove(0).complete(Ok(Screenshot { frame: 9, png: vec![1] }));
		requests.remove(0).complete(Ok(Screenshot { frame: 9, png: vec![2] }));

		assert_eq!(first.recv_timeout(Duration::from_millis(10)).unwrap().unwrap().png, [1]);
		assert_eq!(second.recv_timeout(Duration::from_millis(10)).unwrap().unwrap().png, [2]);
	}

	#[test]
	fn png_converts_bgra_formats_and_ignores_pitched_padding() {
		for format in [ghi::Formats::BGRAu8, ghi::Formats::BGRAsRGB] {
			let png = encode_screenshot_png(readback(vec![10, 20, 30, 255, 99, 99, 99, 99], format, 8)).expect("encode PNG");
			assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
			assert_eq!(decode(&png), [30, 20, 10, 255]);
		}
	}

	#[test]
	fn png_converts_rgba16_unorm_and_ignores_pitched_padding() {
		let values = [0u16, 32_768, 65_535, 257];
		let mut bytes = values.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
		bytes.extend_from_slice(&[99; 8]);
		let png = encode_screenshot_png(readback(bytes, ghi::Formats::RGBA16UNORM, 16)).expect("encode PNG");
		assert_eq!(decode(&png), [0, 128, 255, 1]);
	}

	#[test]
	fn png_rejects_invalid_readbacks() {
		let error = encode_screenshot_png(readback(vec![0; 4], ghi::Formats::RGBA8UNORM, 4)).expect_err("reject RGBA readback");
		assert!(error.starts_with("Texture transfer format is unsupported."));

		let error = encode_screenshot_png(readback(vec![0; 3], ghi::Formats::BGRAu8, 4)).expect_err("reject incomplete row");
		assert!(error.starts_with("Screenshot buffer is incomplete."));
	}

	fn decode(png: &[u8]) -> Vec<u8> {
		let mut reader = png::Decoder::new(std::io::Cursor::new(png))
			.read_info()
			.expect("read encoded PNG");
		let mut pixels = vec![0; reader.output_buffer_size().expect("finite PNG buffer")];
		let size = reader.next_frame(&mut pixels).expect("decode encoded PNG").buffer_size();
		pixels.truncate(size);
		pixels
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
