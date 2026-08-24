use std::collections::VecDeque;
use std::ptr::NonNull;

use ::utils::hash::{HashMap, HashSet};
use dispatch2::DispatchData;
use objc2::runtime::ProtocolObject;
use objc2::ClassType;
use objc2_foundation::{NSAutoreleasePool, NSString};
use objc2_metal::{MTL4CommandEncoder, MTL4ComputeCommandEncoder, MTLBuffer, MTLDevice, MTLResource};
use smallvec::SmallVec;

use super::*;
use crate::{
	buffer::{self as buffer_builder, BufferHandle},
	descriptors::DescriptorSetHandle,
	image::{self as image_builder, ImageHandle},
	metal::swapchain::Swapchain,
	metal::utils::parse_threadgroup_size_metadata,
	pipelines::raster as raster_pipeline,
	sampler::{self as sampler_builder, SamplerHandle},
	window, DeviceAccesses, HandleLike as _, MasterHandle as _, ResourceCollection, Uses,
};

/// The `TextureReadbackStorage` struct keeps one Metal transfer result alive for later CPU mapping.
pub(crate) struct TextureReadbackStorage {
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) bytes: Vec<u8>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	pub(crate) bytes_per_row: usize,
	pub(crate) bytes_per_image: usize,
	pub(crate) native_bytes_per_row: usize,
	pub(crate) native_bytes_per_image: usize,
	pub(crate) row_count: usize,
	pub(crate) image_count: usize,
}

/// The `Context` struct owns resources created for rendering on a Metal GPU device.
pub struct Context {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::StoredQueue>,
	// Context-issued IDs validate queue-local handles without global coordination.
	pub(crate) next_resource_io_queue_id: u64,
	pub(crate) buffers: ResourceCollection<buffer::Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: ResourceCollection<image::Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: Vec<sampler::Sampler>,
	pub(crate) allocations: Vec<Allocation>,
	pub(crate) pipeline_layouts: Vec<PipelineLayout>,
	pub(crate) vertex_layouts: Vec<VertexLayout>,
	vertex_layout_indices: HashMap<VertexLayoutKey, VertexLayoutHandle>,
	pub(crate) descriptor_sets: Vec<descriptor_set::DescriptorSet>,
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
	internal_upload_synchronizer: Option<graphics_hardware_interface::SynchronizerHandle>,
	internal_upload_queues: Vec<Option<graphics_hardware_interface::QueueHandle>>,
	pub(crate) swapchains: Vec<swapchain::Swapchain>,
	pub(crate) texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadbackStorage>,

	pub(crate) resource_to_descriptor:
		HashMap<PrivateHandles, HashSet<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8)>>,
	pub(crate) descriptor_set_to_resource:
		HashMap<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8), HashSet<PrivateHandles>>,
	descriptor_sources:
		HashMap<(DescriptorSetHandle, crate::shader::ResourceSlot, u32, u8), (crate::descriptors::WriteData, i32)>,

	pub settings: crate::device::Features,
	pub(crate) pending_buffer_syncs: VecDeque<BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<ImageHandle>,
	pub(crate) tasks: Vec<Task>,

	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,
}

impl Drop for Context {
	fn drop(&mut self) {
		// Metal 4 command buffers do not retain resources, so all queue work must finish before context-owned resources drop.
		self.wait();
	}
}

/// Reports whether a CAMetalLayer drawable can satisfy the requested texture uses directly.
fn drawable_supports_uses(uses: crate::Uses) -> bool {
	let drawable_uses = Uses::RenderTarget
		| Uses::Storage
		| Uses::Image
		| Uses::InputAttachment
		| Uses::TransferSource
		| Uses::TransferDestination
		| Uses::Clear;
	!uses.is_empty() && drawable_uses.contains(uses)
}

mod recording;
mod resources;
mod traits;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::command_buffer::CommandBufferRecording as _;
	use crate::device::Device as _;

	/// Creates one real Metal context for transfer integration tests.
	fn test_context() -> Context {
		let features = crate::device::Features::new();
		let mut instance = crate::metal::Instance::new(features).expect(
			"Failed to create the Metal transfer test instance. The most likely cause is that no Metal device is available.",
		);
		let mut queue_handle = None;
		let device = instance
			.create_device(
				features,
				&mut [(crate::QueueSelection::new(crate::WorkloadTypes::TRANSFER), &mut queue_handle)],
			)
			.expect("Failed to create the Metal transfer test device. The most likely cause is unavailable Metal support.");
		assert_eq!(queue_handle, Some(crate::QueueHandle(0)));
		device
			.create_context()
			.expect("Failed to create the Metal transfer test context. The most likely cause is unavailable Metal 4 support.")
	}

	#[test]
	fn drawable_uses_accept_render_and_shader_output() {
		assert!(drawable_supports_uses(Uses::RenderTarget | Uses::Storage));
	}

	#[test]
	fn drawable_uses_reject_non_texture_roles() {
		assert!(!drawable_supports_uses(Uses::RenderTarget | Uses::Vertex));
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
