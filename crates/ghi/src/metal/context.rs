use std::collections::VecDeque;
use std::ptr::NonNull;

use ::utils::hash::{HashMap, HashSet};
use objc2::ClassType;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSAutoreleasePool, NSString};
use objc2_metal::MTLBuffer;
use smallvec::SmallVec;

use super::*;
use crate::{
	DeviceAccesses, ResourceCollection, Uses,
	buffer::{self as buffer_builder, BufferHandle},
	descriptors::DescriptorSetHandle,
	image::{self as image_builder, ImageHandle},
	metal::swapchain::Swapchain,
	pipelines::raster as raster_pipeline,
	sampler::{self as sampler_builder, SamplerHandle},
	window,
};

/// The `TextureReadbackStorage` struct keeps one Metal transfer result alive for later CPU mapping.
pub(crate) struct TextureReadbackStorage {
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) bytes: Vec<u8>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	/// The compact layout callers receive.
	pub(crate) layout: crate::context::TextureTransferLayout,
	/// The padded row pitch of `buffer`. Its image pitch is this times the layout's row count.
	pub(crate) native_bytes_per_row: usize,
}

/// The frame-local descriptor sets of a context, one chain per public set with an entry per frame in flight.
pub(crate) type DescriptorSets =
	ResourceCollection<descriptor_set::DescriptorSet, graphics_hardware_interface::DescriptorSetHandle, DescriptorSetHandle>;

/// The `Context` struct owns resources created for rendering on a Metal GPU device.
pub struct Context {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::StoredQueue>,
	pub(crate) buffers: ResourceCollection<buffer::Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: ResourceCollection<image::Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: Vec<sampler::Sampler>,
	pub(crate) allocations: Vec<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>,
	pub(crate) descriptor_sets: DescriptorSets,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) acceleration_structures: Vec<AccelerationStructure>,
	pub(crate) shaders: Vec<Shader>,
	pub(crate) pipelines: Vec<Pipeline>,
	pub(crate) command_buffers: Vec<StoredCommandBuffer>,
	pub(crate) synchronizers: ResourceCollection<
		synchronizer::Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
	/// Signals when the internal uploads of each frame sequence complete.
	internal_upload_synchronizer: graphics_hardware_interface::SynchronizerHandle,
	internal_upload_queues: Vec<Option<graphics_hardware_interface::QueueHandle>>,
	pub(crate) swapchains: Vec<swapchain::Swapchain>,
	pub(crate) texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadbackStorage>,
	pub(crate) counters: crate::counters::Counters,
	/// One timestamp heap per frame sequence, which the sequence's frames write their counter slots into; see
	/// [`crate::counters::Counters`]. Metal forbids invalidating a heap the GPU still uses, and a completed sequence
	/// shares no heap with the frames in flight, so each one can be invalidated as soon as its frame completes.
	pub(crate) counter_heaps: [Retained<ProtocolObject<dyn mtl::MTL4CounterHeap>>; MAX_FRAMES_IN_FLIGHT],
	/// GPU timestamp ticks per second, which converts counter slots into durations.
	pub(crate) timestamp_frequency: u64,

	pub(crate) resource_to_descriptor:
		HashMap<PrivateHandles, HashSet<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8)>>,

	pub settings: crate::device::Features,
	pub(crate) pending_buffer_syncs: VecDeque<BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<(ImageHandle, Option<crate::image::Region>)>,
	pub(crate) tasks: Vec<Task>,
	/// One retained upload arena per in-flight frame, followed by the transient arena for detached recordings.
	pub(crate) upload_arenas: Vec<command_buffer::UploadArena>,
	pub(crate) argument_tables: command_buffer::CommandArgumentTables,
	pub(crate) image_groups: crate::image_group::ImageGroups,
	/// The serial the next group heap receives; see [`image::GroupSlot::heap_serial`].
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::command_buffer::CommandBufferRecording as _;

	fn test_context() -> Context {
		crate::metal::test_context(crate::WorkloadTypes::TRANSFER).0
	}

	#[test]
	fn texture_transfers_preserve_request_identity_and_layout() {
		let mut context = test_context();
		let extent = Extent::rectangle(3, 2);
		let image = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(extent),
		);
		let synchronizer = context.create_synchronizer(None, false);
		let command_buffer = context.create_command_buffer(None, crate::QueueHandle(0));
		let mut recording = context.create_command_buffer_recording(command_buffer);

		let first = recording.transfer_texture(image.into()).expect(
			"First Metal texture transfer failed. The most likely cause is that the test image lacks transfer-source support.",
		);
		let second = recording.transfer_texture(image.into()).expect(
			"Second Metal texture transfer failed. The most likely cause is that the test image lacks transfer-source support.",
		);
		assert_ne!(first, second);

		recording.execute(synchronizer);
		let mapped = context.get_image_data(first).expect(
			"Metal texture mapping failed. The most likely cause is that the transfer command did not complete successfully.",
		);
		assert_eq!(mapped.extent, extent);
		assert_eq!(mapped.format, crate::Formats::RGBA8UNORM);
		assert_eq!(mapped.bytes_per_row, 12);
		assert_eq!(mapped.bytes_per_image, 24);
		assert_eq!(mapped.bytes.len(), 24);
		assert!(context.texture_readbacks.get(first).is_none());
		assert_eq!(
			context.get_image_data(first),
			Err(crate::TextureTransferError::InvalidHandle(first))
		);

		context.get_image_data(second).expect(
			"Second Metal texture mapping failed. The most likely cause is that its transfer command did not complete successfully.",
		);
		assert_eq!(context.texture_readbacks.values().count(), 0);

		let synchronizer = context.create_synchronizer(None, false);
		let command_buffer = context.create_command_buffer(None, crate::QueueHandle(0));
		let mut recording = context.create_command_buffer_recording(command_buffer);
		let third = recording.transfer_texture(image.into()).expect(
			"Third Metal texture transfer failed. The most likely cause is that the test image lacks transfer-source support.",
		);
		assert!(third.0 > second.0);
		recording.execute(synchronizer);
		context.get_image_data(third).expect(
			"Third Metal texture mapping failed. The most likely cause is that its transfer command did not complete successfully.",
		);
		assert_eq!(context.texture_readbacks.values().count(), 0);
	}

	#[test]
	fn dropped_texture_transfer_releases_staging_and_cannot_be_mapped() {
		let mut context = test_context();
		let image = context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image | crate::Uses::TransferSource)
				.extent(Extent::square(1)),
		);
		let command_buffer = context.create_command_buffer(None, crate::QueueHandle(0));
		let mut recording = context.create_command_buffer_recording(command_buffer);
		let handle = recording
			.transfer_texture(image.into())
			.expect("Metal texture transfer recording must succeed for a valid 2D transfer source.");

		drop(recording);

		assert_eq!(context.texture_readbacks.values().count(), 0);
		assert_eq!(
			context.get_image_data(handle),
			Err(crate::TextureTransferError::MappingFailed)
		);
	}
}
