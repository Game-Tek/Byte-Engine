use ash::vk;
use ::utils::hash::HashMap;

use crate::graphics_hardware_interface;

pub mod command_buffer;
pub mod device;
mod utils;

pub use self::device::*;
pub use self::command_buffer::*;

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

const MAX_FRAMES_IN_FLIGHT: usize = 3;

#[derive(Clone)]
pub(crate) struct Swapchain {
	surface: vk::SurfaceKHR,
	surface_present_mode: vk::PresentModeKHR,
	swapchain: vk::SwapchainKHR,
	semaphores: [vk::Semaphore; MAX_FRAMES_IN_FLIGHT],
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
	shaders: Vec<graphics_hardware_interface::ShaderHandle>,
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
	type_: graphics_hardware_interface::DescriptorType,
	descriptor_type: vk::DescriptorType,
	stages: graphics_hardware_interface::Stages,
	pipeline_stages: vk::PipelineStageFlags2,
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

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	next: Option<BufferHandle>,
	staging: Option<BufferHandle>,
	buffer: vk::Buffer,
	size: usize,
	device_address: vk::DeviceAddress,
	pointer: *mut u8,
	uses: graphics_hardware_interface::Uses,
	use_cases: Option<graphics_hardware_interface::UseCases>,
	frame: Option<u8>,
}

unsafe impl Send for Buffer {}

#[derive(Clone, Copy)]
pub(crate) struct Synchronizer {
	next: Option<SynchronizerHandle>,
	fence: vk::Fence,
	vk_semaphore: vk::Semaphore,
}

#[derive(Clone)]
pub(crate) struct Image {
	#[cfg(debug_assertions)]
	name: Option<String>,
	next: Option<ImageHandle>,
	staging_buffer: Option<BufferHandle>,
	allocation_handle: graphics_hardware_interface::AllocationHandle,
	image: vk::Image,
	image_view: vk::ImageView,
	image_views: [vk::ImageView; 8],
	pointer: *const u8,
	extent: vk::Extent3D,
	format: vk::Format,
	format_: graphics_hardware_interface::Formats,
	layout: vk::ImageLayout,
	size: usize,
	uses: graphics_hardware_interface::Uses,
	layers: u32,
}

unsafe impl Send for Image {}

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
	allocation: graphics_hardware_interface::AllocationHandle,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

struct AccelerationStructure {
	acceleration_structure: vk::AccelerationStructureKHR,
	buffer: vk::Buffer,
	scratch_size: usize,
}

struct Frame {

}

#[derive(Clone, Copy)]
/// Stores the information of a memory backed resource.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// The alignment the resources needs when bound to a memory region.
	alignment: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_triangle() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::render_triangle(&mut ghi);
	}

	#[test]
	fn render_present() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::present(&mut ghi);
	}

	#[test]
	fn render_multiframe_present() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_present(&mut ghi); // BUG: can see graphical artifacts, most likely synchronization issue
	}

	#[test]
	fn render_multiframe() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_rendering(&mut ghi);
	}

	#[test]
	fn render_dynamic_data() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::dynamic_data(&mut ghi);
	}

	#[test]
	fn render_with_descriptor_sets() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::descriptor_sets(&mut ghi);
	}

	#[test]
	fn render_with_multiframe_resources() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_resources(&mut ghi);
	}

	#[test]
	fn render_with_ray_tracing() {
		let mut ghi = Device::new(graphics_hardware_interface::Features::new().validation(true).ray_tracing(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::ray_tracing(&mut ghi);
	}
}
