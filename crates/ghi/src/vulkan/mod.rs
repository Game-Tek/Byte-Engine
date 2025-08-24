use std::sync::atomic::AtomicU64;

use ash::vk;
use ::utils::hash::HashMap;

use crate::graphics_hardware_interface;
use crate::vulkan::sampler::SamplerHandle;

pub mod command_buffer;
pub mod instance;
pub mod device;
pub mod buffer;
pub mod image;
pub mod sampler;

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct ImageHandle(pub(super) u64);

impl Into<Handle> for ImageHandle {
	fn into(self) -> Handle {
		Handle::Image(self)
	}
}

impl HandleLike for ImageHandle {
	type Item = Image;

	fn build(value: u64) -> Self {
		ImageHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Image {
		&collection[self.0 as usize]
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct BufferHandle(pub(super) u64);

impl Into<Handle> for BufferHandle {
	fn into(self) -> Handle {
		Handle::Buffer(self)
	}
}

impl HandleLike for BufferHandle {
	type Item = Buffer;

	fn build(value: u64) -> Self {
		BufferHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Buffer {
		&collection[self.0 as usize]
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TopLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BottomLevelAccelerationStructureHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct DescriptorSetHandle(pub(super) u64);

impl HandleLike for DescriptorSetHandle {
	type Item = DescriptorSet;

	fn build(value: u64) -> Self {
		DescriptorSetHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a DescriptorSet {
		&collection[self.0 as usize]
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct DescriptorSetBindingHandle(pub(super) u64);

impl HandleLike for DescriptorSetBindingHandle {
	type Item = Binding;

	fn build(value: u64) -> Self {
		DescriptorSetBindingHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Binding {
		&collection[self.0 as usize]
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SynchronizerHandle(pub(super) u64);

impl HandleLike for SynchronizerHandle {
	type Item = Synchronizer;

	fn build(value: u64) -> Self {
		SynchronizerHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Synchronizer {
		&collection[self.0 as usize]
	}
}

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
	acquire_synchronizers: [SynchronizerHandle; MAX_FRAMES_IN_FLIGHT],
	submit_synchronizers: [SynchronizerHandle; 8],
	extent: vk::Extent2D,
	sync_stage: vk::PipelineStageFlags2,
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
}

impl Next for DescriptorSet {
	type Handle = DescriptorSetHandle;

	fn next(&self) -> Option<DescriptorSetHandle> {
		self.next
	}
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
	next: Option<DescriptorSetBindingHandle>,
	descriptor_set_handle: DescriptorSetHandle,
	descriptor_type: vk::DescriptorType,
	index: u32,
	count: u32,
}

impl Next for Binding {
	type Handle = DescriptorSetBindingHandle;

	fn next(&self) -> Option<DescriptorSetBindingHandle> {
		self.next
	}
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

#[derive(Clone)]
pub(crate) struct Synchronizer {
	next: Option<SynchronizerHandle>,

	name: Option<String>,
	signaled: bool,

	fence: vk::Fence,
	semaphore: vk::Semaphore,
}

impl Next for Synchronizer {
	type Handle = SynchronizerHandle;

	fn next(&self) -> Option<SynchronizerHandle> {
		self.next
	}
}

// #[derive(Clone, Copy)]
// pub(crate) struct AccelerationStructure {
// 	acceleration_structure: vk::AccelerationStructureKHR,
// }

pub(crate) struct DebugCallbackData {
	error_count: AtomicU64,
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
	/// Patch all descriptors that reference the image.
	/// Usually, this is done when the image is resized because the Vulkan image will be swapped.
	UpdateImageDescriptors {
		handle: ImageHandle,
	},
	/// Update a particular descriptor.
	/// This task will most likely be enqueued for performance reasons. Since it is cheaper to update multiple descriptors at once instead of sporadically.
	WriteDescriptor {
		binding_handle: DescriptorSetBindingHandle,
		descriptor: Descriptors,
	},
	/// A miscellaneous task that may be associated with a frame index.
	Other(Box<dyn Fn()>),
}

pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
}

impl Task {
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

	pub(crate) fn update_image_descriptor(handle: ImageHandle, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::UpdateImageDescriptors { handle },
			frame,
		}
	}

	pub(crate) fn other(f: Box<dyn Fn()>, frame: Option<u8>) -> Self {
		Self {
			task: Tasks::Other(f),
			frame,
		}
	}

	pub fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub fn task(&self) -> &Tasks {
		&self.task
	}

	pub fn write_descriptor(binding_handle: DescriptorSetBindingHandle, descriptor: Descriptors, frame: Option<u8>) -> Task {
        Self {
            task: Tasks::WriteDescriptor { binding_handle, descriptor },
            frame,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Descriptors {
	Buffer{ handle: BufferHandle, size: graphics_hardware_interface::Ranges },
	Image{ handle: ImageHandle, layout: graphics_hardware_interface::Layouts },
	CombinedImageSampler{ image_handle: ImageHandle, layout: graphics_hardware_interface::Layouts, sampler_handle: SamplerHandle, layer: Option<u32> },
	Sampler{ handle: SamplerHandle },
	CombinedImageSamplerArray,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DescriptorWrite {
	pub(crate) write: Descriptors,
	pub(crate) binding: DescriptorSetBindingHandle,
	pub(crate) array_element: u32,
}

impl DescriptorWrite {
	pub(crate) fn new(write: Descriptors, binding: DescriptorSetBindingHandle) -> Self {
		Self { write, binding, array_element: 0 }
	}

	pub(crate) fn index(mut self, index: u32) -> Self {
		self.array_element = index;
		self
	}
}

pub(crate) trait HandleLike where Self: Sized, Self: PartialEq<Self>, Self: Clone, Self: Copy {
	type Item: Next<Handle = Self>;

	fn build(value: u64) -> Self;

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Self::Item;

	fn root(&self, collection: &[Self::Item]) -> Self {
		let handle_option = Some(*self);

		return if let Some(e) = collection.iter().enumerate().find(|(_, e)| e.next() == handle_option).map(|(i, _)| Self::build(i as u64)) {
			e.root(collection)
		} else {
			handle_option.unwrap()
		}
	}

	fn get_all(&self, collection: &[Self::Item]) -> Vec<Self> {
		let mut handles = Vec::with_capacity(3);
		let mut handle_option = Some(*self);

		while let Some(handle) = handle_option {
			let binding = handle.access(collection);
			handles.push(handle);
			handle_option = binding.next();
		}

		handles
	}
}

pub(crate) trait Next where Self: Sized {
	type Handle: HandleLike<Item = Self>;

    fn next(&self) -> Option<Self::Handle>;
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
	#[ignore = "test is broken because of WSI"]
	fn render_present() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::present(&mut device);
	}

	#[test]
	#[ignore = "test is broken because of WSI"]
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
	fn render_change_frames() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::change_frames(&mut device);
	}

	#[test]
	fn render_resize() {
		let features = graphics_hardware_interface::Features::new().validation(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::resize(&mut device);
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
	#[ignore = "not working on supporting rt right now"]
	fn render_with_ray_tracing() {
		let features = graphics_hardware_interface::Features::new().validation(true).ray_tracing(true);
		let mut instance = Instance::new(features.clone()).expect("Failed to create Vulkan instance.");
		let mut device = instance.create_device(features.clone()).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::ray_tracing(&mut device);
	}
}
