use std::sync::atomic::AtomicU64;

use ::utils::hash::HashMap;
use ::utils::Extent;
use ash::vk;
use smallvec::SmallVec;

use crate::graphics_hardware_interface;
use crate::vulkan::sampler::SamplerHandle;

pub mod binding;
pub mod buffer;
pub mod command_buffer;
pub mod descriptor_set;
pub mod device;
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
pub use self::descriptor_set::*;
pub use self::device::*;
pub use self::frame::*;
pub(crate) use self::image::*;
pub use self::instance::*;
pub(crate) use self::swapchain::*;
pub(crate) use self::synchronizer::*;

pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: graphics_hardware_interface::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: vk::Sampler,
		layout: graphics_hardware_interface::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
	Image(ImageHandle),
	Buffer(BufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
	VkBuffer(vk::Buffer),
	Synchronizer(SynchronizerHandle),
}

#[derive(Clone, PartialEq)]
pub(super) struct Consumption {
	pub(super) handle: Handle,
	pub(super) stages: graphics_hardware_interface::Stages,
	pub(super) access: graphics_hardware_interface::AccessPolicies,
	pub(super) layout: graphics_hardware_interface::Layouts,
}

#[derive(Clone, PartialEq)]
pub(super) struct VulkanConsumption {
	pub(super) handle: Handle,
	pub(super) stages: vk::PipelineStageFlags2,
	pub(super) access: vk::AccessFlags2,
	pub(super) layout: vk::ImageLayout,
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
	descriptor_set_template_indices: HashMap<graphics_hardware_interface::DescriptorSetTemplateHandle, u32>,
}

#[derive(Clone)]
pub(crate) struct Shader {
	shader: vk::ShaderModule,
	stage: graphics_hardware_interface::Stages,
	shader_binding_descriptors: Vec<graphics_hardware_interface::ShaderBindingDescriptor>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	resource_access: Vec<(
		(u32, u32),
		(
			graphics_hardware_interface::Stages,
			graphics_hardware_interface::AccessPolicies,
		),
	)>,
}

#[derive(Clone, Copy)]
pub(super) struct CommandBufferInternal {
	vk_queue: vk::Queue,
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
		descriptor_write: graphics_hardware_interface::DescriptorWrite,
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
		layout: graphics_hardware_interface::Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		layout: graphics_hardware_interface::Layouts,
		sampler_handle: SamplerHandle,
		layer: Option<u32>,
	},
	Sampler {
		handle: SamplerHandle,
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

pub(crate) trait HandleLike
where
	Self: Sized,
	Self: PartialEq<Self>,
	Self: Clone,
	Self: Copy,
{
	type Item: Next<Handle = Self>;

	fn build(value: u64) -> Self;

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Self::Item;

	fn root(&self, collection: &[Self::Item]) -> Self {
		let handle_option = Some(*self);

		return if let Some(e) = collection
			.iter()
			.enumerate()
			.find(|(_, e)| e.next() == handle_option)
			.map(|(i, _)| Self::build(i as u64))
		{
			e.root(collection)
		} else {
			handle_option.unwrap()
		};
	}

	fn get_all(&self, collection: &[Self::Item]) -> SmallVec<[Self; MAX_FRAMES_IN_FLIGHT]> {
		let mut handles = SmallVec::new();
		let mut handle_option = Some(*self);

		while let Some(handle) = handle_option {
			let binding = handle.access(collection);
			handles.push(handle);
			handle_option = binding.next();
		}

		handles
	}
}

pub(crate) trait Next
where
	Self: Sized,
{
	type Handle: HandleLike<Item = Self>;

	fn next(&self) -> Option<Self::Handle>;
}

#[cfg(test)]
mod tests {
	use super::*;

	fn create_default_device_setup() -> (Instance, Device, graphics_hardware_interface::QueueHandle) {
		let features = graphics_hardware_interface::Features::new().validation(true);
		create_default_device_setup_with_features(features)
	}

	fn create_default_device_setup_with_features(
		features: graphics_hardware_interface::Features,
	) -> (Instance, Device, graphics_hardware_interface::QueueHandle) {
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut queue_handle = None;
		let device = instance
			.create_device(
				features.clone(),
				&mut [(
					graphics_hardware_interface::QueueSelection::new(graphics_hardware_interface::CommandBufferType::GRAPHICS),
					&mut queue_handle,
				)],
			)
			.expect("Failed to create VulkanGHI.");
		(instance, device, queue_handle.unwrap())
	}

	#[test]
	fn render_triangle() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::render_triangle(&mut device, queue_handle);
	}

	#[test]
	#[ignore = "test is broken because of WSI"]
	fn render_present() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::present(&mut device, queue_handle);
	}

	#[test]
	#[ignore = "test is broken because of WSI"]
	fn render_multiframe_present() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::multiframe_present(&mut device, queue_handle);
	}

	#[test]
	fn render_multiframe() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::multiframe_rendering(&mut device, queue_handle);
	}

	#[test]
	fn render_change_frames() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::change_frames(&mut device, queue_handle);
	}

	#[test]
	fn render_resize() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::resize(&mut device, queue_handle);
	}

	#[test]
	fn render_dynamic_data() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::dynamic_data(&mut device, queue_handle);
	}

	#[test]
	fn render_with_descriptor_sets() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::descriptor_sets(&mut device, queue_handle);
	}

	#[test]
	fn render_with_multiframe_resources() {
		let (_instance, mut device, queue_handle) = create_default_device_setup();
		graphics_hardware_interface::tests::multiframe_resources(&mut device, queue_handle);
	}

	#[test]
	#[ignore = "not working on supporting rt right now"]
	fn render_with_ray_tracing() {
		let (_instance, mut device, queue_handle) = create_default_device_setup_with_features(
			graphics_hardware_interface::Features::new()
				.validation(true)
				.ray_tracing(true),
		);
		graphics_hardware_interface::tests::ray_tracing(&mut device, queue_handle);
	}
}
