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

/// The `Task` struct defers work on a resource until frame sequence `frame` has completed its previous submission,
/// which is the last one that may use the resource.
///
/// [`Context::process_tasks`] runs it when that frame sequence starts again.
pub(crate) struct Task {
	pub(crate) kind: TaskKind,
	pub(crate) frame: u8,
}

/// The `TaskKind` enum is the work a [`Task`] defers.
pub(crate) enum TaskKind {
	/// Replaces one frame-local image with storage of a new extent.
	ResizeImage {
		handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		/// The new layer count, or `None` to keep the image's current one.
		array_layers: Option<u32>,
	},
	/// Frees an image that no context holds anymore.
	DeleteImage(Image),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Descriptor {
	Image {
		image: ImageHandle,
		mip_level: Option<u32>,
		array_layer: Option<u32>,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
	},
	Buffer {
		buffer: BufferHandle,
	},
	Sampler {
		sampler: SamplerHandle,
	},
	Swapchain {
		handle: crate::swapchain::SwapchainHandle,
	},
	/// The copy of a top-level structure that the set's frame sequence builds.
	TopLevelAccelerationStructure {
		handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
		copy: u8,
	},
}

/// The `PrivateHandles` enum names a resource whose native backing can change, so the context can find the
/// descriptor set entries to encode again when it does.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PrivateHandles {
	Image(crate::image::ImageHandle),
	Buffer(crate::buffer::BufferHandle),
	Swapchain(crate::swapchain::SwapchainHandle),
}

impl Descriptor {
	pub(crate) fn tracked_resource(self) -> Option<PrivateHandles> {
		match self {
			Descriptor::Buffer { buffer } => Some(PrivateHandles::Buffer(buffer)),
			Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
				Some(PrivateHandles::Image(image))
			}
			Descriptor::Swapchain { handle } => Some(PrivateHandles::Swapchain(handle)),
			Descriptor::Sampler { .. } | Descriptor::TopLevelAccelerationStructure { .. } => None,
		}
	}
}
