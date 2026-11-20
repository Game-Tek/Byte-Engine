//! Temporal upscalers: vendor libraries, such as MetalFX, that resolve jittered frames rendered below the output
//! resolution into one antialiased image at the output resolution.
//!
//! Check [`crate::context::Context::temporal_upscaler_scale_range`] first; `None` means the device has no temporal
//! upscaler. Then build one with [`crate::context::ContextCreate::build_temporal_upscaler`] and record it each frame
//! with [`crate::command_buffer::CommandBufferRecording::upscale`].

use std::fmt;

use utils::Extent;

use crate::{BaseImageHandle, Formats};

crate::handle::index_handle! {
	/// The `TemporalUpscalerHandle` struct identifies a temporal upscaler of the context that built it.
	///
	/// Build one with [`crate::context::ContextCreate::build_temporal_upscaler`], then record it each frame with
	/// [`crate::command_buffer::CommandBufferRecording::upscale`]. Its sizes and formats are fixed, so build a new one
	/// when the input or output extent changes.
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct TemporalUpscalerHandle;
}

/// The `Builder` struct describes the images a temporal upscaler reads and writes, so the backend can build it once
/// and reuse it every frame.
///
/// Pass it to [`crate::context::ContextCreate::build_temporal_upscaler`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) input_extent: Extent,
	pub(crate) output_extent: Extent,
	pub(crate) color_format: Formats,
	pub(crate) depth_format: Formats,
	pub(crate) motion_format: Formats,
	pub(crate) reactive_format: Option<Formats>,
	pub(crate) output_format: Formats,
	pub(crate) depth_reversed: bool,
}

impl<'a> Builder<'a> {
	/// Describes an upscaler from `input_extent` images in the color, depth, and motion formats to an
	/// `output_extent` image in `output_format`. Depth is not reversed and there is no reactive mask until
	/// [`Self::depth_reversed`] and [`Self::reactive_format`] say otherwise.
	pub fn new(
		input_extent: Extent,
		output_extent: Extent,
		[color_format, depth_format, motion_format]: [Formats; 3],
		output_format: Formats,
	) -> Self {
		Self {
			name: None,
			input_extent,
			output_extent,
			color_format,
			depth_format,
			motion_format,
			reactive_format: None,
			output_format,
			depth_reversed: false,
		}
	}

	/// Names the upscaler in errors and capture tools.
	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	/// Reads a reactive mask in `format` each frame: 0 keeps the upscaler's normal blend and 1 ignores the history.
	pub fn reactive_format(mut self, format: Formats) -> Self {
		self.reactive_format = Some(format);
		self
	}

	/// Marks depth as reversed: 1 at the near plane and 0 at the far plane.
	pub fn depth_reversed(mut self, reversed: bool) -> Self {
		self.depth_reversed = reversed;
		self
	}
}

/// The `Inputs` struct holds one frame's images and parameters for a temporal upscaler.
///
/// Pass it to [`crate::command_buffer::CommandBufferRecording::upscale`]. Images with one copy per frame in flight
/// resolve to the recording frame's copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inputs {
	/// The jittered color the frame rendered, at the upscaler's input extent.
	pub color: BaseImageHandle,
	/// The frame's depth, at the input extent.
	pub depth: BaseImageHandle,
	/// Each pixel's motion in its red and green channels, at the input extent; see [`Self::motion_scale`].
	pub motion: BaseImageHandle,
	/// The reactive mask, when the upscaler was built with [`Builder::reactive_format`].
	pub reactive: Option<BaseImageHandle>,
	/// The image the upscaler writes, at its output extent.
	pub output: BaseImageHandle,
	/// The part of the input images that holds this frame, from their origin. Equal to the input extent unless the
	/// frame rendered smaller.
	pub content_extent: Extent,
	/// The sub-pixel offset the frame rendered through, in input pixels, in the upscaler's convention.
	pub jitter: [f32; 2],
	/// The factors that turn a motion texel into the input-pixel offset from the pixel to where it was in the previous
	/// frame.
	pub motion_scale: [f32; 2],
	/// The factor the color was multiplied by before upscaling, which the upscaler divides out of its history.
	pub pre_exposure: f32,
	/// Whether the upscaler drops its history this frame, such as after a cut or a resize.
	pub reset: bool,
}

/// The `UpscalerError` enum explains why a temporal upscaler could not be built, so the caller can resolve frames
/// another way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpscalerError {
	/// The backend or device has no temporal upscaler.
	Unsupported,
	/// The device has one, but it rejected the description, such as a format or scale it doesn't take.
	Rejected {
		/// The upscaler's name, or an empty string.
		name: String,
	},
}

impl fmt::Display for UpscalerError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Unsupported => write!(
				formatter,
				"The device has no temporal upscaler. The most likely cause is a backend or GPU without one, such as Vulkan, DX12, or a Mac without Metal 4 MetalFX. Check `Context::temporal_upscaler_scale_range` before building one, and resolve frames another way when it is `None`."
			),
			Self::Rejected { name } => write!(
				formatter,
				"The temporal upscaler '{name}' could not be built. The most likely cause is a format the device's upscaler doesn't take, or an input-to-output scale outside `Context::temporal_upscaler_scale_range`. Use RGBA16F color and motion, Depth32 depth, and a scale inside that range."
			),
		}
	}
}

impl std::error::Error for UpscalerError {}
