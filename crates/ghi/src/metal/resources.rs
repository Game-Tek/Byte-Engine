use super::*;

#[derive(Clone)]
pub(crate) struct StoredCommandBuffer {
	pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	pub(crate) name: Option<String>,
}

pub struct CommandBuffer<'a> {
	pub(crate) device: &'a mut context::Context,
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
}

#[derive(Clone)]
pub(crate) struct Allocation {
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) pointer: *mut u8,
	pub(crate) size: usize,
}

pub(crate) struct DebugCallbackData {
	pub(crate) error_count: AtomicU64,
	pub(crate) error_log_function: fn(&str),
}

pub(crate) struct Mesh {
	pub(crate) vertex_buffers: Vec<Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>>,
	pub(crate) index_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) vertex_count: u32,
	pub(crate) index_count: u32,
	pub(crate) vertex_size: usize,
}

/// The `AccelerationStructureGeometry` enum records the geometry class one acceleration structure was sized for.
///
/// Metal sizes a structure from its geometry counts and formats alone, so the counts captured at creation are
/// enough to allocate storage before any build supplies the geometry buffers.
#[derive(Clone, Copy)]
pub(crate) enum AccelerationStructureGeometry {
	/// One indexed triangle geometry with the given vertex format, index type, and triangle count.
	Triangles {
		vertex_format: mtl::MTLAttributeFormat,
		index_type: mtl::MTLIndexType,
		triangle_count: usize,
	},
	/// One axis-aligned bounding-box geometry with the given box count.
	BoundingBoxes { bounding_box_count: usize },
	/// An instance structure over at most `max_instance_count` bottom-level instances.
	Instances { max_instance_count: usize },
}

/// The `AccelerationStructure` struct owns one Metal acceleration structure and the build sizes it was created with.
pub(crate) struct AccelerationStructure {
	pub(crate) structure: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	pub(crate) geometry: AccelerationStructureGeometry,
	/// The scratch bytes Metal requires to build this structure.
	pub(crate) build_scratch_size: usize,
}

#[derive(Clone, Copy)]
/// The `MemoryBackedResourceCreationResult` struct provides a resource and its memory requirements for allocation.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	pub(crate) resource: T,
	/// The final size of the resource.
	pub(crate) size: usize,
	/// The memory flags that need used to create the resource.
	pub(crate) memory_flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BuildImage {
	pub(crate) previous: ImageHandle,
	pub(crate) master: graphics_hardware_interface::ImageHandle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BuildBuffer {
	pub(crate) previous: BufferHandle,
	pub(crate) master: graphics_hardware_interface::BaseBufferHandle,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Deletes a Metal texture at the frame selected by [`Task`].
	DeleteMetalTexture {
		handle: ImageHandle,
	},
	/// Deletes a Metal buffer at the frame selected by [`Task`].
	DeleteMetalBuffer {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the buffer.
	/// Usually, this is done when the buffer is resized because the Metal buffer will be swapped.
	UpdateBufferDescriptors {
		handle: BufferHandle,
	},
	/// Patch all descriptors that reference the image.
	/// Usually, this is done when the image is resized because the Metal texture will be swapped.
	UpdateImageDescriptors {
		handle: ImageHandle,
	},
	/// Resize an image.
	ResizeImage {
		handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
	},
	BuildImage(BuildImage),
	BuildBuffer(BuildBuffer),
}

#[derive(Debug, Clone, PartialEq)]
/// The `Task` struct schedules backend work for a required time or frame.
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
}

impl Task {
	pub(crate) fn new(task: Tasks, frame: Option<u8>) -> Self {
		Self { task, frame }
	}

	pub(crate) fn delete_metal_texture(handle: ImageHandle, frame: u8) -> Self {
		Self {
			task: Tasks::DeleteMetalTexture { handle },
			frame: Some(frame),
		}
	}

	pub(crate) fn delete_metal_buffer(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::DeleteMetalBuffer { handle },
			frame,
		}
	}

	pub(crate) fn update_buffer_descriptor(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateBufferDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn update_image_descriptor(handle: ImageHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateImageDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub(crate) fn task(&self) -> &Tasks {
		&self.task
	}

	pub(crate) fn into_task(self) -> Tasks {
		self.task
	}
}
