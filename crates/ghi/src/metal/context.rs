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
	pub(crate) allocations: Vec<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>,
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
	internal_upload_queues: Vec<Option<graphics_hardware_interface::QueueHandle>>,
	pub(crate) swapchains: Vec<Swapchain>,
	pub(crate) texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadbackStorage>,
	pub(crate) counters: crate::counters::Counters,
	/// One timestamp heap per frame sequence, which the sequence's frames write their counter slots into; see
	/// [`crate::counters::Counters`]. Metal forbids invalidating a heap the GPU still uses, and a completed sequence
	/// shares no heap with the frames in flight, so each one can be invalidated as soon as its frame completes.
	pub(crate) counter_heaps: [Retained<ProtocolObject<dyn mtl::MTL4CounterHeap>>; MAX_FRAMES_IN_FLIGHT],
	/// GPU timestamp ticks per second, which converts counter slots into durations.
	pub(crate) timestamp_frequency: u64,

	/// The descriptor-set bindings that reference each resource, as frame-local set, slot, and array element.
	pub(crate) resource_to_descriptor:
		HashMap<PrivateHandles, HashSet<(DescriptorSetHandle, crate::shader::ResourceSlot, u32)>>,

	pub settings: crate::device::Features,
	pub(crate) pending_buffer_syncs: VecDeque<BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<(ImageHandle, Option<crate::image::Region>)>,
	pub(crate) tasks: Vec<Task>,
	/// One retained upload arena per in-flight frame, followed by the transient arena for detached recordings.
	pub(crate) upload_arenas: Vec<command_buffer::UploadArena>,
	pub(crate) argument_tables: command_buffer::CommandArgumentTables,
	pub(crate) image_groups: crate::image_group::ImageGroups,
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::command_buffer::CommandBufferRecording as _;

	fn test_context() -> Context {
		crate::metal::test_context(crate::WorkloadTypes::TRANSFER).0
	}

	#[test]
	fn created_resources_are_placed_in_heaps() {
		let mut context = test_context();
		context.build_buffer::<[u32; 16]>(
			crate::buffer::Builder::new(crate::Uses::Storage).device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.build_dynamic_buffer::<[u32; 16]>(crate::buffer::Builder::new(crate::Uses::Storage));
		context.build_image(
			crate::image::Builder::new(crate::Formats::RGBA8UNORM, crate::Uses::Image)
				.extent(Extent::square(4))
				.device_accesses(crate::DeviceAccesses::HostToDevice),
		);
		context.build_dynamic_image(
			crate::image::Builder::new(crate::Formats::RGBA16F, crate::Uses::RenderTarget).extent(Extent::square(4)),
		);
		let mesh = context.add_mesh_from_vertices_and_indices(
			3,
			3,
			&[0; 36],
			&[0; 6],
			&[crate::pipelines::VertexElement::new("POSITION", crate::DataTypes::Float3, 0)],
		);
		if crate::context::Context::supports_ray_tracing(&context) {
			context.create_top_level_acceleration_structure(None, 4);
		}

		assert!(context.buffers.iter().all(|buffer| buffer.buffer.heap().is_some()));
		assert!(context.images.iter().all(|image| image.texture.heap().is_some()));
		let mesh = &context.meshes[mesh.0 as usize];
		assert!(mesh.index_buffer.heap().is_some());
		assert!(mesh.vertex_buffers.iter().flatten().all(|buffer| buffer.heap().is_some()));
		assert!(
			context
				.acceleration_structures
				.top_levels
				.values()
				.flat_map(|top_level| &top_level.copies)
				.all(|copy| copy.structure.heap().is_some())
		);
	}

	/// Verifies the memory of a resized image goes back to the system once no command can reach the old image.
	#[test]
	fn replaced_image_releases_its_heap() {
		use crate::frame::Frame as _;
		use crate::queue::{FrameRequest, Queue as _, QueueExecution as _};
		use crate::shader::{ResourceKind, ResourceSlot, ShaderResourceDescriptor};

		let (mut context, queue_handle) = crate::metal::test_context(crate::WorkloadTypes::TRANSFER);
		// Large enough that it does not share its heap, so the heap has nothing else to stay for.
		let image = context
			.build_image(crate::image::Builder::new(crate::Formats::R8UNORM, crate::Uses::Image).extent(Extent::square(2048)));
		let replaced_heap = {
			let texture = &context.images.get_single(image.0).unwrap().texture;
			objc2::rc::Weak::from_retained(&texture.heap().unwrap())
		};
		// A descriptor set keeps the image resident, as a material that samples it would.
		let slot = ResourceSlot::new(0);
		let layout = crate::DescriptorSetLayout::new(
			"Replaced Image",
			[ShaderResourceDescriptor::new(
				slot,
				ResourceKind::SampledImage,
				1,
				crate::AccessPolicies::READ,
			)],
		);
		let descriptor_set = context.create_descriptor_set(None, &layout);
		context.write(&[crate::DescriptorWrite::image(
			descriptor_set,
			slot,
			image,
			crate::Layouts::Read,
		)]);
		let buffers = [(); 2].map(|_| {
			context.build_buffer::<[u8; 4]>(
				crate::buffer::Builder::new(crate::Uses::TransferSource | crate::Uses::TransferDestination)
					.device_accesses(crate::DeviceAccesses::HostOnly),
			)
		});
		let command_buffer = context.queue(queue_handle).create_command_buffer(None);
		let signal = context.create_synchronizer(None, true);
		let mut frame_index = 0;
		// Runs one frame that submits a small copy, and resizes the image first when given an extent.
		let mut run_frame = |context: &mut Context, extent: Option<Extent>| {
			context
				.queue(queue_handle)
				.execute(Some(FrameRequest::new(frame_index, signal)), &[], signal, |execution| {
					if let Some(extent) = extent {
						execution.frame().unwrap().resize_image(image.into(), extent);
					}
					execution.record(command_buffer, |recording| {
						recording.copy_buffers(&[crate::BufferCopyDescriptor::new(
							buffers[0].into(),
							0,
							buffers[1].into(),
							0,
							4,
						)]);
					});
					[]
				});
			context.wait();
			frame_index += 1;
		};

		run_frame(&mut context, None);
		run_frame(&mut context, Some(Extent::square(64)));
		assert!(replaced_heap.load().is_some());

		// The queue lets go of the image two sweeps after its last use, and of its heap one sweep after that.
		for _ in 0..3 {
			std::thread::sleep(queue::RESIDENCY_LINGER + std::time::Duration::from_millis(10));
			run_frame(&mut context, None);
		}

		assert!(
			replaced_heap.load().is_none(),
			"The replaced image's heap is still alive. The most likely cause is that the queue's residency still holds the heap of an image nothing uses.",
		);
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
