use ash::vk;
use ::utils::hash::HashMap;

use crate::graphics_hardware_interface;

pub mod command_buffer;
pub mod instance;
pub mod device;
pub mod buffer;
pub mod image;

mod utils;

pub use self::instance::*;
pub use self::device::*;
pub use self::command_buffer::*;
pub use self::buffer::*;
pub use self::image::*;

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
pub(super) struct ImageHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BufferHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DescriptorSetHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DescriptorSetBindingHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SynchronizerHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Handle {
	Image(ImageHandle),
	Buffer(BufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

#[derive(Clone, PartialEq,)]
pub(super) struct Consumption {
	pub(super) handle: Handle,
	pub(super) stages: graphics_hardware_interface::Stages,
	pub(super) access: graphics_hardware_interface::AccessPolicies,
	pub(super) layout: graphics_hardware_interface::Layouts,
}

#[derive(Clone, PartialEq,)]
pub(super) struct VulkanConsumption {
	pub(super) handle: Handle,
	pub(super) stages: vk::PipelineStageFlags2,
	pub(super) access: vk::AccessFlags2,
	pub(super) layout: vk::ImageLayout,
}

const MAX_FRAMES_IN_FLIGHT: usize = 3;

#[derive(Clone)]
pub(crate) struct Swapchain {
	surface: vk::SurfaceKHR,
	swapchain: vk::SwapchainKHR,
	synchronizer: graphics_hardware_interface::SynchronizerHandle,
	extent: vk::Extent2D,
}

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
pub(crate) struct DescriptorSet {
	next: Option<DescriptorSetHandle>,
	descriptor_set: vk::DescriptorSet,
	descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,

	// resources: Vec<(graphics_hardware_interface::DescriptorSetBindingHandle, Handle)>,
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
	resource_access: Vec<((u32, u32), (graphics_hardware_interface::Stages, graphics_hardware_interface::AccessPolicies))>,
}

#[derive(Clone, Copy)]
pub(super) struct CommandBufferInternal {
	command_pool: vk::CommandPool,
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone)]
pub(crate) struct Binding {
	descriptor_set_handle: graphics_hardware_interface::DescriptorSetHandle,
	descriptor_type: vk::DescriptorType,
	index: u32,
	count: u32,
}

#[derive(Clone)]
pub(crate) struct CommandBuffer {
	frames: Vec<CommandBufferInternal>,
}

#[derive(Clone, Copy)]
pub(crate) struct Allocation {
	memory: vk::DeviceMemory,
	pointer: *mut u8,
}

unsafe impl Send for Allocation {}
unsafe impl Sync for Allocation {}

#[derive(Clone, Copy)]
pub(crate) struct Synchronizer {
	next: Option<SynchronizerHandle>,
	fence: vk::Fence,
	semaphore: vk::Semaphore,
}

// #[derive(Clone, Copy)]
// pub(crate) struct AccelerationStructure {
// 	acceleration_structure: vk::AccelerationStructureKHR,
// }

struct DebugCallbackData {
	error_count: u64,
	error_log_function: fn(&str),
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct TransitionState {
	stage: vk::PipelineStageFlags2,
	access: vk::AccessFlags2,
	layout: vk::ImageLayout,
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_triangle() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::render_triangle(&mut device);
	}

	#[test]
	fn render_present() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::present(&mut device);
	}

	#[test]
	fn render_multiframe_present() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_present(&mut device); // BUG: can see graphical artifacts, most likely synchronization issue
	}

	#[test]
	fn render_multiframe() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_rendering(&mut device);
	}

	#[test]
	fn render_dynamic_data() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::dynamic_data(&mut device);
	}

	#[test]
	fn render_with_descriptor_sets() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::descriptor_sets(&mut device);
	}

	#[test]
	fn render_with_multiframe_resources() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_resources(&mut device);
	}

	#[test]
	fn render_with_ray_tracing() {
		let features = graphics_hardware_interface::Features::new().validation(true).ray_tracing(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::ray_tracing(&mut device);
	}
}
