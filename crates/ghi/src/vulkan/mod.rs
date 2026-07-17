use std::sync::{atomic::AtomicU64, Arc, Mutex};

use ::utils::hash::HashMap;
use ::utils::Extent;
use ash::vk;

use crate::binding::DescriptorSetBindingHandle;
use crate::buffer::BufferHandle;
use crate::descriptors::DescriptorSetHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;
use crate::sampler::SamplerHandle;
use crate::PrivateHandles;

pub mod binding;
pub mod buffer;
pub mod command_buffer;
pub mod context;
pub mod descriptor_set;
pub mod device;

pub mod factory;
pub mod frame;
pub mod image;
pub mod instance;
pub mod queue;
pub mod sampler;
pub mod swapchain;
pub mod synchronizer;

mod utils;

pub use self::binding::*;
pub(crate) use self::buffer::*;
pub use self::command_buffer::*;
pub use self::context::*;
pub use self::descriptor_set::*;
pub use self::device::*;
/// The `Factory` type alias keeps Vulkan detached resource creation aligned with the backend device API.
pub type Factory = Device;
pub use self::frame::*;
pub(crate) use self::image::*;
pub use self::instance::*;
pub(crate) use self::swapchain::*;
pub(crate) use self::synchronizer::*;

pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: vk::Sampler,
		layout: crate::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Swapchain {
		handle: graphics_hardware_interface::SwapchainHandle,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Handles {
	Image(ImageHandle),
	Buffer(BufferHandle),
	VkBuffer(vk::Buffer),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
	Synchronizer(crate::synchronizer::SynchronizerHandle),
}

#[derive(Clone, PartialEq)]
pub(super) struct Consumption {
	pub(super) handle: Handles,
	pub(super) stages: crate::Stages,
	pub(super) access: crate::AccessPolicies,
	pub(super) layout: crate::Layouts,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BufferRange {
	pub(super) offset: vk::DeviceSize,
	pub(super) size: vk::DeviceSize,
}

impl BufferRange {
	pub(super) fn new(offset: vk::DeviceSize, size: vk::DeviceSize) -> Self {
		Self { offset, size }
	}

	pub(super) fn end(self) -> vk::DeviceSize {
		self.offset.saturating_add(self.size)
	}

	pub(super) fn overlaps(self, other: Self) -> bool {
		self.offset < other.end() && other.offset < self.end()
	}
}

#[derive(Clone, PartialEq)]
pub(super) struct VulkanConsumption {
	pub(super) handle: Handles,
	pub(super) stages: vk::PipelineStageFlags2,
	pub(super) access: vk::AccessFlags2,
	pub(super) layout: vk::ImageLayout,
	pub(super) range: Option<BufferRange>,
}

const MAX_FRAMES_IN_FLIGHT: usize = 3;
const MAX_SWAPCHAIN_IMAGES: usize = 8;

#[derive(Clone)]
pub(crate) struct DescriptorSetLayout {
	bindings: Vec<(vk::DescriptorType, u32)>,
	descriptor_set_layout: vk::DescriptorSetLayout,
}

#[derive(Clone)]
pub(crate) struct PipelineLayout {
	pipeline_layout: vk::PipelineLayout,
	/// Maps set handles to their index in the pipeline layout. This is needed to know which set index to use when writing descriptors.
	descriptor_set_template_indices: HashMap<graphics_hardware_interface::DescriptorSetTemplateHandle, u32>,
}

#[derive(Clone)]
pub(crate) struct Shader {
	shader: vk::ShaderModule,
	stage: crate::Stages,
	shader_binding_descriptors: Vec<crate::shader::BindingDescriptor>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct PipelineLayoutKey {
	descriptor_set_templates: Vec<graphics_hardware_interface::DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
}

#[derive(Clone)]
pub(super) struct CommandBufferInternal {
	vk_queue: Arc<Mutex<vk::Queue>>,
	command_pool: vk::CommandPool,
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone)]
pub(crate) struct CommandBuffer {
	queue_handle: graphics_hardware_interface::QueueHandle,
	frames: Vec<CommandBufferInternal>,
}

#[derive(Clone, Copy)]
pub(crate) struct Allocation {
	memory: vk::DeviceMemory,
	pointer: *mut u8,
}

pub(crate) struct DebugCallbackData {
	error_count: AtomicU64,
	error_log_function: fn(&str),
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct TransitionState {
	pub stage: vk::PipelineStageFlags2,
	pub access: vk::AccessFlags2,
	pub layout: vk::ImageLayout,
	pub last_write_stage: vk::PipelineStageFlags2,
	pub last_write_access: vk::AccessFlags2,
}

impl TransitionState {
	pub(super) fn new(stage: vk::PipelineStageFlags2, access: vk::AccessFlags2, layout: vk::ImageLayout) -> Self {
		let (last_write_stage, last_write_access) = if Self::access_includes_write(access) {
			(stage, access)
		} else {
			(vk::PipelineStageFlags2::empty(), vk::AccessFlags2::empty())
		};

		Self {
			stage,
			access,
			layout,
			last_write_stage,
			last_write_access,
		}
	}

	pub(super) fn inherit_last_write_from(mut self, source: Self) -> Self {
		if !Self::access_includes_write(self.access) {
			self.last_write_stage = source.last_write_stage;
			self.last_write_access = source.last_write_access;
		}

		self
	}

	pub(super) fn access_includes_write(access: vk::AccessFlags2) -> bool {
		access.intersects(
			vk::AccessFlags2::MEMORY_WRITE
				| vk::AccessFlags2::TRANSFER_WRITE
				| vk::AccessFlags2::SHADER_WRITE
				| vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
				| vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
				| vk::AccessFlags2::HOST_WRITE
				| vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
		)
	}
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) struct BufferTransitionState {
	pub(super) range: BufferRange,
	pub(super) state: TransitionState,
}

struct Mesh {
	buffer: vk::Buffer,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

struct AccelerationStructure {
	acceleration_structure: vk::AccelerationStructureKHR,
	buffer: vk::Buffer,
}

#[derive(Clone, Copy)]
/// Stores the information of a memory backed resource.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BuildImage {
	previous: ImageHandle,
	master: graphics_hardware_interface::ImageHandle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct BuildBuffer {
	previous: BufferHandle,
	master: graphics_hardware_interface::BaseBufferHandle,
	/// When `PERSISTENT_WRITE` is enabled, carries the handle of the shared
	/// CPU-writable source buffer so per-frame buffers can reference it.
	pub(crate) source: Option<BufferHandle>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Delete a Vulkan image. Will be associated to a frame index in `Task`.
	DeleteVulkanImage {
		handle: vk::Image,
	},
	/// Delete a Vulkan image view. Will be associated to a frame index in `Task`.
	DeleteVulkanImageView {
		handle: vk::ImageView,
	},
	/// Delete a Vulkan buffer. Will be associated to a frame index in `Task`.
	DeleteVulkanBuffer {
		handle: vk::Buffer,
	},
	/// Patch all descriptors that reference the buffer.
	/// Usually, this is done when the buffer is resized because the Vulkan buffer will be swapped.
	UpdateBufferDescriptors {
		handle: BufferHandle,
	},
	/// Resize an image
	ResizeImage {
		handle: ImageHandle,
		extent: Extent,
	},
	/// Update the frame-specific (specified in `Task`) descriptor with the given write information for the master resource and descriptor.
	UpdateDescriptor {
		descriptor_write: crate::descriptors::Write,
	},
	BuildImage(BuildImage),
	BuildBuffer(BuildBuffer),
}

/// The `Task` struct represents a deferred task that needs to be executed at a later time.
/// This is because some tasks need to be executed at a particular time or frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
}

impl Task {
	pub(crate) fn new(task: Tasks, frame: Option<u8>) -> Self {
		Self { task, frame }
	}

	pub(crate) fn delete_vulkan_image(handle: vk::Image, frame: u8) -> Self {
		Self {
			task: Tasks::DeleteVulkanImage { handle },
			frame: Some(frame),
		}
	}

	pub(crate) fn delete_vulkan_image_view(handle: vk::ImageView, frame: u8) -> Self {
		Self {
			task: Tasks::DeleteVulkanImageView { handle },
			frame: Some(frame),
		}
	}

	pub(crate) fn delete_vulkan_buffer(handle: vk::Buffer, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::DeleteVulkanBuffer { handle },
			frame,
		}
	}

	pub(crate) fn update_buffer_descriptor(handle: BufferHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateBufferDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub(crate) fn task(&self) -> &Tasks {
		&self.task
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Descriptors {
	Buffer {
		handle: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Image {
		handle: ImageHandle,
		layout: crate::Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		layout: crate::Layouts,
		sampler_handle: SamplerHandle,
		layer: Option<u32>,
	},
	Sampler {
		handle: SamplerHandle,
	},
	Swapchain {
		handle: graphics_hardware_interface::SwapchainHandle,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DescriptorWrite {
	pub(crate) write: Descriptors,
	pub(crate) binding: DescriptorSetBindingHandle,
	pub(crate) array_element: u32,
}

impl DescriptorWrite {
	pub(crate) fn new(write: Descriptors, binding: DescriptorSetBindingHandle) -> Self {
		Self {
			write,
			binding,
			array_element: 0,
		}
	}

	pub(crate) fn index(mut self, index: u32) -> Self {
		self.array_element = index;
		self
	}
}

/// The `StoredQueue` struct stores per-queue device data for internal GPU queue management.
#[derive(Clone)]
pub(super) struct StoredQueue {
	pub(crate) vk_queue: Arc<Mutex<vk::Queue>>,
	pub(crate) queue_family_index: u32,
	pub(crate) _queue_index: u32,
}
