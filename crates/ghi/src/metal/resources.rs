use super::*;

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

/// The `Tasks` enum lists backend work that must wait until a frame's previous submission has completed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Replaces one frame-local image with a new extent.
	ResizeImage {
		handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
	},
}

/// The `Task` struct schedules backend work for the frame sequence that can safely perform it.
#[derive(Clone, PartialEq)]
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: u8,
}
