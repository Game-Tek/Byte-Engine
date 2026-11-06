use std::borrow::Cow;
use std::sync::atomic::AtomicU64;

use ::utils::Extent;
use ash::vk;

use crate::buffer::BufferHandle;
use crate::graphics_hardware_interface;
use crate::image::ImageHandle;

pub(crate) mod acceleration_structures;
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
pub mod io;
pub(crate) mod pipeline_binaries;
pub mod queue;
pub(crate) mod ray_tracing;
pub mod swapchain;
pub mod synchronizer;

mod utils;

pub(crate) use self::buffer::*;
pub use self::command_buffer::*;
pub use self::context::*;
pub(crate) use self::descriptor_heap::*;
pub use self::descriptor_set::*;
pub use self::device::*;
pub use self::factory::{DetachedImage, Factory, SharedBuffer};
pub use self::frame::*;
pub(crate) use self::image::*;
pub use self::instance::*;
pub use self::io::{ResourceIoQueue, ResourceIoTicket};
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
	},
	Buffer {
		buffer: BufferHandle,
	},
	Sampler {
		sampler: graphics_hardware_interface::SamplerHandle,
	},
	/// The copy of a top-level acceleration structure that frame sequence `copy` builds and traces.
	AccelerationStructure {
		handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
		copy: u8,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Handles {
	Image(ImageHandle),
	Buffer(BufferHandle),
	VkBuffer(vk::Buffer),
	/// Acceleration structure memory, whose accesses Vulkan names apart from shader reads and which global memory
	/// barriers order.
	AccelerationStructure(AccelerationStructureMemory),
}

/// The `AccelerationStructureMemory` struct names the acceleration structure memory that the resource tracker follows.
///
/// It packs what it names into one word, like the payload of every other [`Handles`] variant. Every draw hashes and
/// compares handles, and those stay branch-free only while all variants share one shape.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AccelerationStructureMemory(u64);

impl AccelerationStructureMemory {
	/// Every bottom-level acceleration structure at once: builds and compactions write it, and top-level builds read it.
	///
	/// A top-level build reads the bottom levels its instances name by address, which the tracker cannot see, so the
	/// structures share one state. Their barriers are global memory barriers, which cost the same for one structure as
	/// for all of them.
	pub(super) const BOTTOM_LEVELS: Self = Self(u64::MAX);

	/// Names the copy of the top-level acceleration structure `handle` that frame sequence `copy` builds and traces.
	pub(super) fn top_level(handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle, copy: u8) -> Self {
		debug_assert!((copy as usize) < MAX_FRAMES_IN_FLIGHT);
		Self(handle.bits() * MAX_FRAMES_IN_FLIGHT as u64 + copy as u64)
	}
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

	/// Builds a range from its bounds, keeping ranges that reach the end of the buffer as `WHOLE_SIZE`.
	pub(super) fn from_bounds(start: vk::DeviceSize, end: vk::DeviceSize) -> Self {
		let size = if end == vk::DeviceSize::MAX {
			vk::WHOLE_SIZE
		} else {
			end - start
		};
		Self::new(start, size)
	}

	pub(super) fn overlaps(self, other: Self) -> bool {
		self.offset < other.end() && other.offset < self.end()
	}

	pub(super) fn intersection(self, other: Self) -> Self {
		Self::from_bounds(self.offset.max(other.offset), self.end().min(other.end()))
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

/// The `Shader` struct holds what pipeline creation needs of one shader: its stage, its resources, and how to hand
/// its code to the driver.
///
/// There is no native shader module. A pipeline that is compiled gets the SPIR-V inline, and a pipeline that comes
/// from the pipeline cache only gets the identifier. Create one with [`Self::new`].
pub(crate) struct Shader {
	/// The shader's SPIR-V, or `None` for a keyed shader, whose caller lends the code to each pipeline instead.
	code: Option<ShaderCode>,
	/// The driver's name for the code, which stands in for it when a pipeline is looked up in the pipeline cache.
	/// `None` without a pipeline cache or without `VK_EXT_shader_module_identifier`.
	identifier: Option<pipeline_binaries::Key>,
	stage: crate::Stages,
	/// The sets the shader declares, in the order its source declares them: set `s` reads its pipeline index from
	/// specialization constant [`SET_SPECIALIZATION_BASE`] `+ s`. Ray tracing pipelines check that each closest hit
	/// declares a prefix of the ray generation shader's sets.
	descriptor_sets: Box<[crate::shader::ShaderDescriptorSet]>,
}

/// The `ShaderCode` enum is SPIR-V a shader keeps, in whichever buffer avoided a copy.
enum ShaderCode {
	/// Bytes the caller handed over, which happen to lie on a word boundary.
	Bytes(Vec<u8>),
	Words(Box<[u32]>),
}

impl ShaderCode {
	fn words(&self) -> &[u32] {
		match self {
			// Only word-aligned whole words are stored as bytes, so the cast cannot fail.
			Self::Bytes(bytes) => bytemuck::cast_slice(bytes),
			Self::Words(words) => words,
		}
	}
}

/// Views SPIR-V bytes as words: in place when they lie on a word boundary, and through a copy otherwise.
///
/// Returns `None` when the bytes are empty or not a whole number of words. Only the copy reads the bytes, so a
/// mapped file that is merely viewed is not read from disk.
pub(crate) fn spirv_words(bytes: &[u8]) -> Option<Cow<'_, [u32]>> {
	let (words, rest) = bytes.as_chunks::<4>();
	if words.is_empty() || !rest.is_empty() {
		return None;
	}
	Some(match bytemuck::try_cast_slice(bytes) {
		Ok(aligned) => Cow::Borrowed(aligned),
		Err(_) => Cow::Owned(words.iter().copied().map(u32::from_ne_bytes).collect()),
	})
}

impl Shader {
	/// Creates a shader from SPIR-V, keeping as little of it as the source allows.
	///
	/// Owned bytes are kept as they are and borrowed bytes are copied, because pipelines are created after the
	/// caller's buffer is gone. A keyed source keeps nothing: when the pipeline cache already knows the key, the code
	/// is not even read, and otherwise it is read once to ask the driver for its identifier, which the cache then
	/// remembers under the key. Returns an error when `source` is not SPIR-V or is not a whole number of words.
	pub(crate) fn new(
		source: crate::shader::Sources,
		stage: crate::ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
		pipeline_binaries: Option<&pipeline_binaries::PipelineBinaries>,
	) -> Result<Self, ()> {
		let descriptor_sets = descriptor_sets.into_iter().collect::<Box<[_]>>();
		let (code, identifier) = match source {
			crate::shader::Sources::SPIRV(bytes) => {
				let code = match bytes {
					// Owned bytes on a word boundary are kept as they are. A cast of anything else fails.
					Cow::Owned(bytes) if !bytes.is_empty() && bytemuck::try_cast_slice::<u8, u32>(&bytes).is_ok() => {
						ShaderCode::Bytes(bytes)
					}
					bytes => ShaderCode::Words(spirv_words(&bytes).ok_or(())?.into_owned().into_boxed_slice()),
				};
				let identifier =
					pipeline_binaries.and_then(|pipeline_binaries| pipeline_binaries.shader_identifier(code.words()));
				(Some(code), identifier)
			}
			crate::shader::Sources::KeyedSPIRV { key, code } => {
				// Checking the length reads none of the code.
				if code.is_empty() || !code.len().is_multiple_of(4) {
					return Err(());
				}
				let identifier = pipeline_binaries
					.and_then(|pipeline_binaries| pipeline_binaries.keyed_shader_identifier(key, || spirv_words(code)));
				(None, identifier)
			}
			_ => return Err(()),
		};
		Ok(Self {
			code,
			identifier,
			stage: stage.into(),
			descriptor_sets,
		})
	}
}

pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	layout: PipelineLayout,
	/// What traces need of a ray tracing pipeline, and `None` for other pipelines.
	ray_tracing: Option<Box<ray_tracing::RayTracingPipelineState>>,
}

/// The `MappedMemoryPointer` struct lets context-owned Vulkan mappings move with their context.
///
/// Access to the bytes still requires raw-pointer operations and the context's
/// host/GPU synchronization. This type deliberately does not implement `Sync`.
#[derive(Clone, Copy)]
pub(crate) struct MappedMemoryPointer(pub(crate) *mut u8);

// SAFETY: Vulkan mappings have no host-thread affinity. This wrapper grants no
// safe access to the pointee; callers must retain the allocation and synchronize
// all accesses, including accesses through copied addresses and GPU commands.
unsafe impl Send for MappedMemoryPointer {}

/// The `DescriptorHeapArena` struct owns one long-lived mapped Vulkan descriptor heap, which descriptor sets take
/// ranges of.
///
/// It counts in descriptors of `unit` bytes, so a range's start is the heap index shaders add their offsets to. The
/// implementation's reserved range sits after every index the arena hands out.
pub(crate) struct DescriptorHeapArena {
	buffer: vk::Buffer,
	pointer: crate::vulkan::MappedMemoryPointer,
	device_address: vk::DeviceAddress,
	size: u64,
	/// The bytes of one descriptor, the arena's unit.
	unit: u64,
	reserved_offset: u64,
	reserved_size: u64,
	/// The descriptors sets can take. The heap size fits in `usize` because the heap is mapped.
	free: ::utils::RangeAllocator,
}

/// The `DescriptorHeaps` struct groups the one resource heap and one sampler heap bound by Vulkan command buffers.
pub(crate) struct DescriptorHeaps {
	resource: DescriptorHeapArena,
	sampler: DescriptorHeapArena,
}

impl DescriptorHeapArena {
	/// Takes `count` consecutive descriptors and returns the heap index of the first.
	pub(crate) fn allocate(&mut self, count: u32) -> u32 {
		let range = self.free.take(count as usize, 1).expect(
			"Vulkan descriptor heap is exhausted. The most likely cause is that the live descriptor sets and their frame-local copies hold more descriptors than the heap. Create fewer sets, or sets with smaller arrays.",
		);
		// The heap holds at most `u32::MAX` bytes, which `Context::create_descriptor_heap_arena` asserts.
		range.start as u32
	}

	/// Returns `count` descriptors that [`Self::allocate`] handed out at `index`.
	pub(crate) fn release(&mut self, index: u32, count: u32) {
		self.free.give_back(index as usize..index as usize + count as usize);
	}

	/// Returns the host memory of descriptor `index`, `size` bytes long.
	pub(crate) fn host_range(&self, index: u32, size: u64) -> vk::HostAddressRangeEXT<'_> {
		let offset = u64::from(index) * self.unit;
		assert!(offset + size <= self.reserved_offset);
		let size = usize::try_from(size).expect(
			"Vulkan descriptor range exceeds addressable host memory. The most likely cause is an invalid descriptor size.",
		);
		// SAFETY: DescriptorHeapArena owns a persistently mapped allocation, and the
		// checked range remains valid for the lifetime of the arena.
		let bytes = unsafe { std::slice::from_raw_parts_mut(self.pointer.0.add(offset as usize), size) };
		vk::HostAddressRangeEXT::default().address(bytes)
	}

	pub(crate) fn bind_info(&self) -> vk::BindHeapInfoEXT<'static> {
		vk::BindHeapInfoEXT::default()
			.heap_range(
				vk::DeviceAddressRangeEXT::default()
					.address(self.device_address)
					.size(self.size),
			)
			.reserved_range_offset(self.reserved_offset)
			.reserved_range_size(self.reserved_size)
	}
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
	/// Builds the sampler the sampler heap holds for a shader's fixed sampler. It reads every level of the image.
	pub(crate) fn fixed(sampler: crate::sampler::StaticSampler) -> Self {
		let filter = utils::filter(sampler.filtering_mode);
		Self {
			mag_filter: filter,
			min_filter: filter,
			mipmap_mode: utils::mipmap_mode(sampler.mip_map_mode),
			address_mode: utils::address_mode(sampler.addressing_mode),
			reduction_mode: utils::reduction_mode(sampler.reduction_mode),
			anisotropy: (sampler.max_anisotropy > 1).then_some(f32::from(sampler.max_anisotropy)),
			min_lod: 0.0,
			max_lod: vk::LOD_CLAMP_NONE,
		}
	}

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
			.min_lod(self.min_lod)
			.max_lod(self.max_lod)
	}
}

#[derive(Clone)]
pub(super) struct CommandBufferInternal {
	/// Index into [`InnerDevice::vk_queues`] of the Vulkan queue this command buffer submits to.
	vk_queue_index: usize,
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
	pointer: crate::vulkan::MappedMemoryPointer,
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

	pub(super) fn reads_only(self, next: Self) -> bool {
		!Self::access_includes_write(self.access) && !Self::access_includes_write(next.access)
	}

	/// Whether the barrier that produced this read state already made the resource visible to `next`.
	pub(super) fn covers(self, next: Self) -> bool {
		self.stage.contains(next.stage) && self.access.contains(next.access)
	}

	pub(super) fn has_write_history(self) -> bool {
		!self.last_write_stage.is_empty() || !self.last_write_access.is_empty()
	}

	/// Accumulates another reader so later writers wait for every reader since the last write.
	pub(super) fn merge_reads(self, next: Self) -> Self {
		Self {
			stage: self.stage | next.stage,
			access: self.access | next.access,
			..self
		}
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

#[derive(Clone, Copy)]
/// The `MemoryBackedResourceCreationResult` struct provides a resource and its memory requirements for allocation.
pub(crate) struct MemoryBackedResourceCreationResult<T> {
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
	/// The alignment the resource's memory offset must satisfy.
	alignment: u64,
	/// Whether the driver prefers or requires an allocation that backs only this resource.
	dedicated: bool,
}

impl<T: Copy + Into<DedicatedResource>> MemoryBackedResourceCreationResult<T> {
	/// Returns the resource to name in a dedicated allocation, or `None` when the resource may share memory.
	///
	/// Pass the result to [`Context::create_allocation_internal`] to allocate the memory this resource is bound to.
	pub(crate) fn dedicated_resource(&self) -> Option<DedicatedResource> {
		self.dedicated.then(|| self.resource.into())
	}
}

/// The `DedicatedResource` enum names the single resource a dedicated Vulkan allocation backs, so the driver can
/// tailor that allocation to it.
///
/// A dedicated allocation must have the size the resource's memory requirements report, and the resource must be
/// bound at offset 0. Get one from [`MemoryBackedResourceCreationResult::dedicated_resource`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DedicatedResource {
	Buffer(vk::Buffer),
	Image(vk::Image),
}

impl From<vk::Buffer> for DedicatedResource {
	fn from(buffer: vk::Buffer) -> Self {
		Self::Buffer(buffer)
	}
}

impl From<vk::Image> for DedicatedResource {
	fn from(image: vk::Image) -> Self {
		Self::Image(image)
	}
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tasks {
	/// Deletes a Vulkan image once the frame recorded in [`Task`] has completed.
	DeleteVulkanImage { handle: vk::Image },
	/// Deletes a Vulkan image view once the frame recorded in [`Task`] has completed.
	DeleteVulkanImageView { handle: vk::ImageView },
	/// Deletes a Vulkan buffer once the frame recorded in [`Task`] has completed.
	DeleteVulkanBuffer { handle: vk::Buffer },
	/// Frees device memory once the frame recorded in [`Task`] has completed.
	FreeAllocation { handle: crate::AllocationHandle },
	/// Destroys an acceleration structure and returns its storage to the heap once the frame recorded in [`Task`] has
	/// completed.
	DeleteAccelerationStructure {
		handle: vk::AccelerationStructureKHR,
		storage: acceleration_structures::StorageRange,
	},
	/// Resize an image.
	ResizeImage {
		handle: ImageHandle,
		extent: Extent,
		/// The new layer count, or `None` to keep the image's current one.
		array_layers: Option<std::num::NonZeroU32>,
	},
	/// Returns a descriptor set copy's heap ranges once the frame recorded in [`Task`] has completed: `count`
	/// descriptors at each `index`.
	FreeDescriptorRanges {
		resource_index: u32,
		resource_count: u32,
		sampler_index: u32,
		sampler_count: u32,
	},
}

/// The `Completion` enum says which GPU work has to complete before a recording's results are readable, or before
/// what its commands read may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Completion {
	/// A frame recording, which completed once the context saw its frame complete.
	Frame(u64),
	/// A recording outside frames, which completed once its synchronizer's fence signaled.
	Synchronizer(crate::synchronizer::SynchronizerHandle),
}

/// The `Task` struct schedules backend work for a required time or frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Task {
	pub(crate) task: Tasks,
	pub(crate) frame: Option<u8>,
	/// The GPU work that must complete before the task runs, for objects that work may still use.
	pub(crate) after: Option<Completion>,
}

impl Task {
	pub(crate) fn new(task: Tasks, frame: Option<u8>) -> Self {
		Self {
			task,
			frame,
			after: None,
		}
	}

	/// Schedules a task for the first task pass after `completion` is reached on the GPU.
	pub(crate) fn after(task: Tasks, completion: Completion) -> Self {
		Self {
			after: Some(completion),
			..Self::new(task, None)
		}
	}

	/// Whether the GPU may still use what this task touches, given `reached`, which reports whether GPU work
	/// completed.
	pub(crate) fn is_pending(&self, reached: impl Fn(Completion) -> bool) -> bool {
		self.after.is_some_and(|completion| !reached(completion))
	}
}

/// The `StoredQueue` struct provides per-queue device data to internal submission paths.
#[derive(Clone)]
pub(super) struct StoredQueue {
	/// Index into [`InnerDevice::vk_queues`]. Several GHI queues can share one Vulkan queue.
	pub(crate) vk_queue_index: usize,
	pub(crate) queue_family_index: u32,
}

#[cfg(test)]
mod task_tests {
	use super::*;

	#[test]
	fn deferred_destruction_waits_for_its_frame_to_complete() {
		let task = Task::after(
			Tasks::DeleteVulkanBuffer {
				handle: vk::Buffer::null(),
			},
			Completion::Frame(5),
		);
		let completed = |completed_frame: Option<u64>| {
			move |completion| match completion {
				Completion::Frame(index) => completed_frame.is_some_and(|completed| completed >= index),
				Completion::Synchronizer(_) => unreachable!("The task waits for a frame."),
			}
		};

		assert!(task.is_pending(completed(None)));
		assert!(task.is_pending(completed(Some(4))));
		assert!(!task.is_pending(completed(Some(5))));
		assert!(!task.is_pending(completed(Some(6))));
	}
}
