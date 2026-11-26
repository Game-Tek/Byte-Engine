use std::collections::VecDeque;

use ::utils::hash::{HashMap, HashSet};
use objc2::ClassType;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::MTLBuffer;
use smallvec::SmallVec;

use super::*;
use crate::{
	ResourceCollection, Uses,
	buffer::{self as buffer_builder, BufferHandle},
	descriptors::DescriptorSetHandle,
	image::{self as image_builder, ImageHandle},
	pipelines::raster as raster_pipeline,
	sampler::{self as sampler_builder, SamplerHandle},
	window,
};

/// The `TextureReadbackStorage` struct keeps one Metal transfer result alive for later CPU mapping.
pub(crate) struct TextureReadbackStorage {
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	/// Reserved for the compact image when the transfer is recorded, and filled when it is mapped.
	pub(crate) bytes: Vec<u8>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	/// The compact layout callers receive.
	pub(crate) layout: crate::context::TextureTransferLayout,
	/// The padded row pitch of `buffer`. Its image pitch is this times the layout's row count.
	pub(crate) native_bytes_per_row: usize,
}

/// The pixel format of every swapchain drawable and of the images frames render into before presentation.
pub(crate) const SWAPCHAIN_FORMAT: crate::Formats = crate::Formats::BGRAu8;

/// The frame-local descriptor sets of a context, one chain per public set with an entry per frame in flight.
pub(crate) type DescriptorSets =
	ResourceCollection<DescriptorSet, graphics_hardware_interface::DescriptorSetHandle, DescriptorSetHandle>;

/// The `Context` struct owns resources created for rendering on a Metal GPU device.
pub struct Context {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	/// Compiles the context's shaders and pipelines, so shader handles index the factory's shaders.
	pub(crate) factory: Factory,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::StoredQueue>,
	/// Places the resources this context creates in heaps.
	pub(crate) allocator: allocator::Allocator,
	pub(crate) buffers: ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: ResourceCollection<Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: Vec<Retained<ProtocolObject<dyn mtl::MTLSamplerState>>>,
	pub(crate) descriptor_sets: DescriptorSets,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) acceleration_structures: acceleration_structures::AccelerationStructures,
	/// Bottom-level builds that executed, each waiting for its submission to complete so it can compact.
	pub(crate) pending_compactions: Vec<acceleration_structures::PendingCompaction>,
	/// Compactions the next internal submission copies.
	pub(crate) compaction_copies: Vec<acceleration_structures::CompactionCopy>,
	/// One acceleration-structure scratch arena per in-flight frame, followed by the transient arena for detached
	/// recordings.
	pub(crate) scratch_arenas: Vec<acceleration_structures::ScratchArena>,
	pub(crate) pipelines: Vec<Pipeline>,
	pub(crate) shader_binding_tables: Vec<ShaderBindingTable>,
	pub(crate) command_buffers: Vec<StoredCommandBuffer>,
	pub(crate) synchronizers: ResourceCollection<
		Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
	/// Signals when the internal uploads of each frame sequence complete.
	internal_upload_synchronizer: graphics_hardware_interface::SynchronizerHandle,
	/// Signaled by every workload, which waits for it before it returns.
	pub(crate) workload_synchronizer: graphics_hardware_interface::SynchronizerHandle,
	internal_upload_queues: Vec<Option<graphics_hardware_interface::QueueHandle>>,
	pub(crate) swapchains: crate::swapchain::Swapchains<Swapchain>,
	pub(crate) texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadbackStorage>,
	pub(crate) counters: crate::counters::Counters,
	/// One timestamp heap per frame sequence, which the sequence's frames write their counter slots into; see
	/// [`crate::counters::Counters`]. Metal forbids invalidating a heap the GPU still uses, and a completed sequence
	/// shares no heap with the frames in flight, so each one can be invalidated as soon as its frame completes.
	/// Empty when counters are disabled.
	pub(crate) counter_heaps: Vec<Retained<ProtocolObject<dyn mtl::MTL4CounterHeap>>>,
	/// GPU timestamp ticks per second, which converts counter slots into durations.
	pub(crate) timestamp_frequency: u64,

	/// The descriptor-set bindings that reference each resource, as frame-local set, slot, and array element.
	pub(crate) resource_to_descriptor:
		HashMap<PrivateHandles, HashSet<(DescriptorSetHandle, crate::shader::ResourceSlot, u32)>>,

	pub settings: crate::device::Features,
	/// New private buffer copies the next internal submission zero-fills.
	pub(crate) pending_buffer_clears: VecDeque<BufferHandle>,
	pub(crate) tasks: Vec<Task>,
	/// One retained upload arena per in-flight frame, followed by the transient arena for detached recordings.
	pub(crate) upload_arenas: Vec<command_buffer::UploadArena>,
	pub(crate) argument_tables: command_buffer::CommandArgumentTables,
	pub(crate) image_groups: crate::image_group::ImageGroups,
	/// The temporal upscalers built with `build_temporal_upscaler`, indexed by their handles.
	pub(crate) upscalers: Vec<super::upscaler::Upscaler>,
	/// The serial the next group heap receives; see [`GroupSlot::heap_serial`].
	pub(crate) next_group_heap_serial: u64,
}

// SAFETY: Retained Metal objects are only `!Send` because objc2 cannot know an object's thread rules; Metal
// documents devices, queues, buffers, textures, samplers, and pipeline states as usable from any thread, and
// this backend already relies on that for the detached objects a factory hands to worker threads. The context
// itself adds no thread affinity: it stores those objects in plain collections and holds no thread-local state.
//
// Ownership may move between threads; concurrent use may not. `Sync` is deliberately not claimed, so sharing a
// context requires a lock that hands out one borrow at a time.
unsafe impl Send for Context {}

impl Drop for Context {
	fn drop(&mut self) {
		// Metal 4 command buffers do not retain resources, so all queue work must finish before context-owned resources drop.
		self.wait();
	}
}

mod recording;
pub(crate) use recording::synchronizer_for_sequence;
pub(in crate::metal) mod resources;
mod traits;
