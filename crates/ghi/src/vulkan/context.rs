use std::num::NonZeroU32;

use ash::vk::{self, Handle as _, TaggedStructure as _};
use smallvec::SmallVec;
use utils::hash::HashSet;
use utils::{Extent, hash::HashMap};

use super::{
	Allocation, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DedicatedResource, DescriptorHeapArena,
	DescriptorHeaps, DescriptorSet, Image, MAX_FRAMES_IN_FLIGHT, MemoryBackedResourceCreationResult, Mesh, Pipeline,
	PipelineLayout, Sampler, Shader, Swapchain, Synchronizer, TransitionState,
	utils::{into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format, uses_to_vk_usage_flags},
};
use crate::vulkan::{Device, InnerDevice, StoredQueue};
use crate::{
	FrameKey, HandleLike, MasterHandle as _, ResourceCollection, Size, frame_index_with_offset, graphics_hardware_interface,
	image, sampler,
	synchronizer::SynchronizerHandle,
	vulkan::{
		BufferCopy, CommandBufferRecording, Descriptor, Frame, ImageCopy, ImageHandle, MAX_SWAPCHAIN_IMAGES, Task, Tasks,
	},
	window,
};

/// The `TextureReadbackStorage` struct keeps one Vulkan transfer result alive for later CPU mapping.
pub(crate) struct TextureReadbackStorage {
	pub(crate) buffer: vk::Buffer,
	pub(crate) memory: vk::DeviceMemory,
	pub(crate) pointer: crate::vulkan::MappedMemoryPointer,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	pub(crate) bytes_per_row: usize,
	pub(crate) bytes_per_image: usize,
	pub(crate) size: usize,
}

/// The `Context` struct owns Vulkan device state while presenting the GHI context API.
pub struct Context {
	pub(super) device: InnerDevice,

	pub(super) frames: u8,

	pub(super) queues: Vec<StoredQueue>,
	pub(super) buffers: ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(super) images: Vec<Image>,
	pub(super) samplers: Vec<Sampler>,
	pub(super) allocations: Vec<Allocation>,
	pub(super) descriptor_sets: Vec<DescriptorSet>,
	/// The heaps every descriptor set lives in, created with the first set, so a context without sets reserves none.
	pub(super) descriptor_heaps: Option<DescriptorHeaps>,
	/// The sets whose recording copy a recording outside frames bound and has not executed yet. See
	/// [`Self::settle_recording_reads`].
	pub(super) open_recording_sets: Vec<usize>,
	/// The heap ranges that recording read and that moved, to return once it completes.
	pub(super) open_recording_retirements: Vec<crate::vulkan::Tasks>,
	/// The set elements whose writes name each resource, by the resource's first frame-local copy, so replacing the
	/// resource's storage rewrites only them. A swapchain's writes are listed under its first image.
	descriptor_references: HashMap<super::Handles, SmallVec<[(u32, u32); 2]>>,
	/// The index that merges the descriptors of one resource while a large set copy lists its consumptions, kept empty
	/// in between so it keeps its memory.
	descriptor_merge_index: HashMap<(super::Handles, crate::Layouts), usize>,
	/// The vectors recordings plan barriers in. A recording takes them and hands them back when it drops.
	pub(super) planning_storage: crate::vulkan::command_buffer::PlanningStorage,
	pub(super) meshes: Vec<Mesh>,
	pub(super) shaders: Vec<Shader>,
	pub(super) pipelines: Vec<Pipeline>,
	pub(super) command_buffers: Vec<CommandBuffer>,
	pub(super) synchronizers: Vec<Synchronizer>,
	pub(super) swapchains: Vec<Swapchain>,
	pub(super) texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadbackStorage>,
	pub(super) counters: crate::counters::Counters,
	/// The timestamp query pool every frame sequence writes its counter slots into; see [`crate::counters::Counters`].
	/// A null handle when counters are disabled, because no slot is ever written.
	pub(super) counter_query_pool: vk::QueryPool,
	/// Reused storage for one frame sequence's resolved timestamps and their availability. Empty when counters are
	/// disabled.
	pub(super) counter_results: Vec<[u64; 2]>,

	pub(super) states: HashMap<super::Handles, TransitionState>,
	pub(super) buffer_states: HashMap<super::Handles, Vec<super::BufferTransitionState>>,

	/// Tracks pending buffer host to device, or device to host synchronization operations.
	pub(super) pending_buffer_syncs: HashSet<BufferHandle>,
	/// Tracks pending image host to device, or device to host synchronization operations.
	pub(super) pending_image_syncs: HashSet<(ImageHandle, Option<crate::image::Region>)>,

	/// The buffers [`Context::share_buffer`] handed to other contexts, which may write them between this context's
	/// frames. Sharing takes `&self`, so the list is interior-mutable.
	pub(super) shared_buffers: std::cell::RefCell<Vec<BufferHandle>>,

	/// Stores the debug names for resources.
	/// Used when inspecting resources from a rendering debugger such as RenderDoc.
	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,

	/// A queue of deferred tasks. Usually object deletions and resource updates.
	pub(crate) tasks: Vec<Task>,
	/// Index of the most recently started frame, which retired objects may still be referenced by.
	pub(super) last_started_frame: Option<u64>,
	/// Highest frame index whose fence has been observed signaled, gating deferred destructions.
	pub(super) completed_frame: Option<u64>,

	pub(crate) image_groups: crate::image_group::ImageGroups,
	/// The memory allocations backing each image group's members, indexed by group.
	pub(super) image_group_heaps: Vec<smallvec::SmallVec<[graphics_hardware_interface::AllocationHandle; 2]>>,

	/// Behind a pointer, so the context state every command reads stays as compact as it is without ray tracing.
	pub(crate) acceleration_structures: Box<crate::vulkan::acceleration_structures::AccelerationStructures>,
	pub(crate) shader_binding_tables: Vec<crate::vulkan::ray_tracing::ShaderBindingTable>,
	/// The ray tracing libraries this context compiled for the ray tracing pipelines it creates itself.
	pub(crate) ray_tracing_libraries: crate::vulkan::ray_tracing::Libraries,
}

mod acceleration_structures;
mod descriptors;
mod drop;
mod pipelines;
mod ray_tracing;
pub(crate) use pipelines::{build_compute_pipeline, build_raster_pipeline, with_shader_stages};
pub(crate) use resources::unbacked_image;
mod resources;
mod runtime;
mod traits;
