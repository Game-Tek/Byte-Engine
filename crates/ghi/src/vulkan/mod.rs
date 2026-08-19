use std::sync::{atomic::AtomicU64, Arc, Mutex};

use ::utils::hash::HashMap;
use ::utils::Extent;
use ash::vk;

use crate::buffer::BufferHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;

pub mod buffer;
pub mod command_buffer;
pub mod context;
pub(crate) mod descriptor_heap;
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

pub(crate) use self::buffer::*;
pub use self::command_buffer::*;
pub use self::context::*;
pub(crate) use self::descriptor_heap::*;
pub use self::descriptor_set::*;
pub use self::device::*;
pub use self::factory::Factory;
pub use self::frame::*;
pub(crate) use self::image::*;
pub use self::instance::*;
pub(crate) use self::swapchain::*;
pub(crate) use self::synchronizer::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
		mip_level: Option<u32>,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: graphics_hardware_interface::SamplerHandle,
		layout: crate::Layouts,
		layer: Option<u32>,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: graphics_hardware_interface::SamplerHandle,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
pub(crate) struct Shader {
	shader: vk::ShaderModule,
	stage: crate::Stages,
	shader_resource_descriptors: Vec<crate::shader::ShaderResourceDescriptor>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
}

/// The `DescriptorHeapArena` struct owns one long-lived mapped Vulkan descriptor heap.
pub(crate) struct DescriptorHeapArena {
	buffer: vk::Buffer,
	pointer: *mut u8,
	device_address: vk::DeviceAddress,
	size: u64,
	reserved_size: u64,
	free_ranges: Vec<DescriptorHeapRange>,
}

/// The `DescriptorHeapRange` struct identifies reusable application-owned bytes in a descriptor heap.
#[derive(Clone, Copy)]
struct DescriptorHeapRange {
	offset: u64,
	size: u64,
}

/// The `DescriptorHeaps` struct groups the one resource heap and one sampler heap bound by Vulkan command buffers.
pub(crate) struct DescriptorHeaps {
	resource: DescriptorHeapArena,
	sampler: DescriptorHeapArena,
}

impl DescriptorHeapArena {
	/// Reserves immutable heap bytes so descriptors referenced by in-flight command buffers are never overwritten.
	pub(crate) fn allocate(&mut self, size: u32, alignment: u64) -> u32 {
		let size = size as u64;
		for index in 0..self.free_ranges.len() {
			let range = self.free_ranges[index];
			let offset = descriptor_heap::align_up(range.offset, alignment);
			let end = offset.checked_add(size).expect(
				"Vulkan descriptor heap allocation overflowed. The most likely cause is an invalid materialization size.",
			);
			let range_end = range.offset + range.size;
			if end > range_end {
				continue;
			}

			let prefix_size = offset - range.offset;
			let suffix_size = range_end - end;
			match (prefix_size, suffix_size) {
				(0, 0) => {
					self.free_ranges.remove(index);
				}
				(0, suffix_size) => {
					self.free_ranges[index] = DescriptorHeapRange {
						offset: end,
						size: suffix_size,
					};
				}
				(prefix_size, 0) => {
					self.free_ranges[index].size = prefix_size;
				}
				(prefix_size, suffix_size) => {
					self.free_ranges[index].size = prefix_size;
					self.free_ranges.insert(
						index + 1,
						DescriptorHeapRange {
							offset: end,
							size: suffix_size,
						},
					);
				}
			}
			return u32::try_from(offset).expect(
				"Vulkan descriptor heap offset exceeded 32 bits. The most likely cause is a heap larger than push-index mappings support.",
			);
		}

		panic!(
			"Vulkan descriptor heap is exhausted. The most likely cause is that live immutable descriptor snapshots exceed the long-lived heap capacity."
		);
	}

	/// Returns a retired immutable snapshot range to the arena after its frame fence completes.
	pub(crate) fn release(&mut self, offset: u32, size: u32) {
		if size == 0 {
			return;
		}
		let range = DescriptorHeapRange {
			offset: offset as u64,
			size: size as u64,
		};
		let index = self.free_ranges.partition_point(|free| free.offset < range.offset);
		self.free_ranges.insert(index, range);

		let mut index = index.saturating_sub(1);
		while index + 1 < self.free_ranges.len() {
			let left = self.free_ranges[index];
			let right = self.free_ranges[index + 1];
			let left_end = left.offset + left.size;
			if left_end < right.offset {
				index += 1;
				continue;
			}
			let right_end = right.offset + right.size;
			self.free_ranges[index].size = left_end.max(right_end) - left.offset;
			self.free_ranges.remove(index + 1);
		}
	}

	pub(crate) fn host_range(&self, offset: u32, size: u64) -> vk::HostAddressRangeEXT<'_> {

		assert!(offset as u64 + size <= self.size);
		let size = usize::try_from(size).expect(
			"Vulkan descriptor range exceeds addressable host memory. The most likely cause is an invalid descriptor size.",
		);
		// SAFETY: DescriptorHeapArena owns a persistently mapped allocation, and the
		// checked range remains valid for the lifetime of the arena.
		let bytes = unsafe { std::slice::from_raw_parts_mut(self.pointer.add(offset as usize), size) };
		vk::HostAddressRangeEXT::default().address(bytes)
	}

	pub(crate) fn bind_info(&self) -> vk::BindHeapInfoEXT<'static> {
		vk::BindHeapInfoEXT::default()
			.heap_range(
				vk::DeviceAddressRangeEXT::default()
					.address(self.device_address)
					.size(self.size),
			)
			.reserved_range_offset(0)
			.reserved_range_size(self.reserved_size)
	}
}

impl DescriptorHeaps {
	pub(crate) fn resource(&self) -> &DescriptorHeapArena {
		&self.resource
	}

	pub(crate) fn resource_mut(&mut self) -> &mut DescriptorHeapArena {
		&mut self.resource
	}

	pub(crate) fn sampler(&self) -> &DescriptorHeapArena {
		&self.sampler
	}

	pub(crate) fn sampler_mut(&mut self) -> &mut DescriptorHeapArena {
		&mut self.sampler
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct DescriptorMaterializationHandle(u64);

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct MaterializationKey {
	layout: graphics_hardware_interface::PipelineLayoutHandle,
	descriptor_sets: smallvec::SmallVec<[(graphics_hardware_interface::DescriptorSetHandle, u64, u64); 4]>,
	sequence_index: u8,
	resource_epochs: smallvec::SmallVec<[(u8, u64); MAX_FRAMES_IN_FLIGHT]>,
	swapchain_images: smallvec::SmallVec<[(graphics_hardware_interface::SwapchainHandle, u8); 4]>,
}

/// The `ResolvedPipelineDescriptor` struct retains the concrete resource used by one immutable descriptor snapshot.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedPipelineDescriptor {
	descriptor: Descriptor,
	stages: crate::Stages,
	access: crate::AccessPolicies,
}

/// The `DescriptorMaterialization` struct retains one immutable union of logical descriptor sets in the global heaps.
pub(crate) struct DescriptorMaterialization {
	resource_heap_offset: u32,
	resource_heap_size: u32,
	sampler_heap_offset: u32,
	sampler_heap_size: u32,
	resources: smallvec::SmallVec<[ResolvedPipelineDescriptor; 128]>,
}

/// The `Sampler` struct retains the create parameters needed to write sampler descriptors directly into the sampler heap.
#[derive(Clone, Copy)]
pub(crate) struct Sampler {
	mag_filter: vk::Filter,
	min_filter: vk::Filter,
	mipmap_mode: vk::SamplerMipmapMode,
	address_mode: vk::SamplerAddressMode,
	reduction_mode: vk::SamplerReductionMode,
	anisotropy: Option<f32>,
	min_lod: f32,
	max_lod: f32,
}

impl Sampler {
	pub(crate) fn create_info(&self) -> vk::SamplerCreateInfo<'static> {
		vk::SamplerCreateInfo::default()
			.mag_filter(self.mag_filter)
			.min_filter(self.min_filter)
			.mipmap_mode(self.mipmap_mode)
			.address_mode_u(self.address_mode)
			.address_mode_v(self.address_mode)
			.address_mode_w(self.address_mode)
			.border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK)
			.anisotropy_enable(self.anisotropy.is_some())
			.max_anisotropy(self.anisotropy.unwrap_or(0.0))
			.compare_enable(false)
			.compare_op(vk::CompareOp::NEVER)
			.min_lod(self.min_lod)
			.max_lod(self.max_lod)
			.mip_lod_bias(0.0)
			.unnormalized_coordinates(false)
	}
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
/// The `MemoryBackedResourceCreationResult` struct provides a resource and its memory requirements for allocation.
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
	/// Deletes a Vulkan image at the frame selected by [`Task`].
	DeleteVulkanImage {
		handle: vk::Image,
	},
	/// Deletes a Vulkan image view at the frame selected by [`Task`].
	DeleteVulkanImageView {
		handle: vk::ImageView,
	},
	/// Deletes a Vulkan buffer at the frame selected by [`Task`].
	DeleteVulkanBuffer {
		handle: vk::Buffer,
	},
	/// Resize an image.
	ResizeImage {
		handle: ImageHandle,
		extent: Extent,
	},
	/// Refreshes the frame-local descriptor snapshot after deferred backing resources are ready.
	UpdateDescriptor {
		descriptor_write: crate::descriptors::DescriptorWrite,
		expected_set_version: u64,
	},
	BuildImage(BuildImage),
	BuildBuffer(BuildBuffer),
}

/// The `Task` struct schedules backend work for a required time or frame.
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

	pub(crate) fn frame(&self) -> Option<u8> {
		self.frame
	}

	pub(crate) fn task(&self) -> &Tasks {
		&self.task
	}
}

/// The `StoredQueue` struct provides per-queue device data to internal submission paths.
#[derive(Clone)]
pub(super) struct StoredQueue {
	pub(crate) vk_queue: Arc<Mutex<vk::Queue>>,
	pub(crate) queue_family_index: u32,
	pub(crate) _queue_index: u32,
}

#[cfg(test)]
mod descriptor_heap_arena_tests {
	use super::*;

	fn arena() -> DescriptorHeapArena {
		DescriptorHeapArena {
			buffer: vk::Buffer::null(),
			pointer: std::ptr::null_mut(),
			device_address: 0,
			size: 256,
			reserved_size: 64,
			free_ranges: vec![DescriptorHeapRange { offset: 64, size: 192 }],
		}
	}

	#[test]
	fn retired_ranges_are_coalesced_and_reused() {
		let mut arena = arena();
		let first = arena.allocate(32, 32);
		let second = arena.allocate(64, 64);

		assert_eq!((first, second), (64, 128));

		arena.release(first, 32);
		arena.release(second, 64);

		assert_eq!(arena.allocate(128, 64), 64);
	}

	#[test]
	fn alignment_padding_returns_to_the_free_list() {
		let mut arena = arena();
		let first = arena.allocate(16, 16);
		let aligned = arena.allocate(32, 64);

		assert_eq!((first, aligned), (64, 128));

		arena.release(first, 16);
		arena.release(aligned, 32);

		assert_eq!(arena.allocate(96, 32), 64);
	}
}
