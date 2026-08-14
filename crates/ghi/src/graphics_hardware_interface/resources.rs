//! Backend-independent GHI resources types.

use utils::{Extent, RGBA};

use super::*;
use crate::{DataTypes, Encodings, Formats, Layouts};

/// The `DispatchExtent` struct converts invocation dimensions into workgroup counts.
pub struct DispatchExtent {
	workgroup_extent: Extent,
	dispatch_extent: Extent,
}

impl DispatchExtent {
	/// Creates dispatch dimensions from the invocation count and shader workgroup size.
	pub fn new(dispatch_extent: Extent, workgroup_extent: Extent) -> Self {
		Self {
			workgroup_extent,
			dispatch_extent,
		}
	}

	/// Returns the workgroup count, rounded up in each dimension.
	pub fn get_extent(&self) -> Extent {
		Extent::new(
			self.dispatch_extent
				.width()
				.max(1)
				.div_ceil(self.workgroup_extent.width().max(1)),
			self.dispatch_extent
				.height()
				.max(1)
				.div_ceil(self.workgroup_extent.height().max(1)),
			self.dispatch_extent
				.depth()
				.max(1)
				.div_ceil(self.workgroup_extent.depth().max(1)),
		)
	}

	pub fn get_workgroup_extent(&self) -> Extent {
		self.workgroup_extent
	}
}

pub enum BottomLevelAccelerationStructureDescriptions {
	Mesh {
		vertex_count: u32,
		vertex_position_encoding: Encodings,
		triangle_count: u32,
		index_format: DataTypes,
	},
	AABB {
		transform_count: u32,
	},
}

pub struct BottomLevelAccelerationStructure {
	pub description: BottomLevelAccelerationStructureDescriptions,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ranges {
	Size(usize),
	Whole,
}

pub struct BufferSplitter<'a, T: Copy> {
	buffer: &'a mut [T],
	offset: usize,
}

impl<'a, T: Copy> BufferSplitter<'a, T> {
	pub fn new(buffer: &'a mut [T], offset: usize) -> Self {
		Self { buffer, offset }
	}

	pub fn take(&mut self, size: usize) -> &'a mut [T] {
		let buffer = &mut self.buffer[self.offset..][..size];
		self.offset += size;
		// SAFETY: We know that the buffer is valid for the lifetime of the splitter.
		unsafe { std::mem::transmute(buffer) }
	}
}

/// The `FrameKey` struct identifies a submitted frame while selecting its reusable GPU sequence.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FrameKey {
	/// The monotonically increasing identity of the submitted frame.
	pub(crate) frame_index: u64,
	/// The bounded GPU sequence selected from the frame identity.
	pub(crate) sequence_index: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PresentKey {
	/// The index of the acquired swapchain image.
	pub(crate) image_index: u8,
	/// The index corresponding to the frame index.
	pub(crate) sequence_index: u8,
	/// The swapchain handle corresponding to the presentation request that this key is associated with.
	pub(crate) swapchain: SwapchainHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RGBAu8 {
	pub(crate) r: u8,
	pub(crate) g: u8,
	pub(crate) b: u8,
	pub(crate) a: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum PresentationModes {
	Inmediate,
	#[default]
	FIFO,
	Mailbox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearValue {
	None,
	Color(RGBA),
	Integer(u32, u32, u32, u32),
	Depth(f32),
}

#[derive(Clone, Copy)]
pub enum ImageOrSwapchain {
	Image(BaseImageHandle),
	Swapchain(SwapchainHandle),
}

impl From<BaseImageHandle> for ImageOrSwapchain {
	fn from(value: BaseImageHandle) -> Self {
		Self::Image(value)
	}
}

impl From<ImageHandle> for ImageOrSwapchain {
	fn from(value: ImageHandle) -> Self {
		Self::Image(value.into())
	}
}

impl From<DynamicImageHandle> for ImageOrSwapchain {
	fn from(value: DynamicImageHandle) -> Self {
		Self::Image(value.into())
	}
}

impl From<SwapchainHandle> for ImageOrSwapchain {
	fn from(value: SwapchainHandle) -> Self {
		Self::Swapchain(value)
	}
}

#[derive(Clone, Copy)]
/// The `AttachmentInformation` struct configures one render-pass attachment.
pub struct AttachmentInformation {
	/// The image view of the attachment.
	pub(crate) target: ImageOrSwapchain,
	/// The attachment format, or `None` to use the target image's format.
	pub(crate) format: Option<Formats>,
	/// The layout of the attachment.
	pub(crate) layout: Layouts,
	/// The clear color of the attachment.
	pub(crate) clear: ClearValue,
	/// Whether the render pass loads the attachment's existing contents.
	pub(crate) load: bool,
	/// Whether the render pass stores the attachment's final contents.
	pub(crate) store: bool,
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
	/// The number of array layers available to shader-selected render-target indices.
	pub(crate) layer_count: Option<std::num::NonZeroU32>,
}

impl AttachmentInformation {
	/// Creates one attachment that targets a single image layer.
	///
	/// Call [`Self::layer`] to select one array layer or [`Self::layers`] to let
	/// the raster shader select among several layers.
	pub fn new(target: impl Into<ImageOrSwapchain>, layout: Layouts, clear: ClearValue, load: bool, store: bool) -> Self {
		Self {
			target: target.into(),
			format: None,
			layout,
			clear,
			load,
			store,
			layer: None,
			layer_count: None,
		}
	}

	/// Selects one array layer for every draw in the render pass.
	pub fn layer(mut self, layer: u32) -> Self {
		assert!(
			self.layer_count.is_none(),
			"Cannot select one attachment layer after enabling layered rendering. The most likely cause is that layer and layers were both called for the same attachment."
		);
		self.layer = Some(layer);
		self
	}

	/// Lets the raster shader select one of the first `layer_count` array layers.
	///
	/// Every attachment in the render pass must declare the same layer count.
	pub fn layers(mut self, layer_count: u32) -> Self {
		let layer_count = std::num::NonZeroU32::new(layer_count).expect(
			"Layered rendering requires at least one attachment layer. The most likely cause is that an empty layer range was passed to AttachmentInformation::layers.",
		);
		assert!(
			self.layer.is_none(),
			"Cannot enable layered rendering after selecting one attachment layer. The most likely cause is that layer and layers were both called for the same attachment."
		);
		self.layer_count = Some(layer_count);
		self
	}

	/// Returns the pass-wide layer count after checking that all attachments agree.
	pub(crate) fn render_pass_layer_count(attachments: &[Self]) -> u32 {
		let layer_count = attachments.first().and_then(|attachment| attachment.layer_count);
		assert!(
			attachments.iter().all(|attachment| attachment.layer_count == layer_count),
			"Render-pass attachments use different layer counts. The most likely cause is that layered rendering was enabled on only some attachments."
		);
		layer_count.map_or(1, std::num::NonZeroU32::get)
	}
}
