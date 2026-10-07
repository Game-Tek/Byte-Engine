use super::*;
use crate::sampler::SamplerHandle;

#[derive(Clone)]
pub(crate) struct StoredCommandBuffer {
	pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	pub(crate) name: Option<String>,
}

pub(crate) struct Mesh {
	pub(crate) vertex_buffers: Vec<Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>>,
	pub(crate) index_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) index_count: u32,
}

/// The `AccelerationStructure` struct owns one Metal acceleration structure and the scratch size its builds need.
pub(crate) struct AccelerationStructure {
	pub(crate) structure: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	/// The scratch bytes Metal requires to build this structure.
	pub(crate) build_scratch_size: usize,
}

/// The `Task` struct defers replacing one frame-local image with a new extent until the frame sequence that uses
/// it has completed its previous submission.
///
/// [`Context::process_tasks`] runs it when that frame sequence starts again.
#[derive(Clone, PartialEq)]
pub(crate) struct Task {
	pub(crate) handle: graphics_hardware_interface::BaseImageHandle,
	pub(crate) extent: Extent,
	/// The new layer count, or `None` to keep the image's current one.
	pub(crate) array_layers: Option<u32>,
	pub(crate) frame: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
		mip_level: Option<u32>,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
		layout: crate::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: SamplerHandle,
	},
	Swapchain {
		handle: crate::swapchain::SwapchainHandle,
	},
	AccelerationStructure {
		handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
	},
}

impl Descriptor {
	pub(crate) fn tracked_resource(self) -> Option<PrivateHandles> {
		match self {
			Descriptor::Buffer { buffer, .. } => Some(PrivateHandles::Buffer(buffer)),
			Descriptor::Image { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::CombinedImageSampler { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::Sampler { .. } => None,
			Descriptor::Swapchain { handle } => Some(PrivateHandles::Swapchain(handle)),
			Descriptor::AccelerationStructure { .. } => None,
		}
	}
}
