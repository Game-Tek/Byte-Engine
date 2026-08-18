use std::{ptr::NonNull, rc::Rc};

use ::utils::{hash::HashMap, Extent};
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSAutoreleasePool, NSRange, NSString};
use objc2_metal::{
	MTL4ArgumentTable, MTL4CommandEncoder, MTL4ComputeCommandEncoder, MTL4RenderCommandEncoder, MTLArgumentEncoder, MTLBuffer,
	MTLDevice, MTLTexture,
};
use smallvec::SmallVec;

use super::*;
use crate::metal::swapchain::Swapchain;
use crate::{
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode, RasterizationRenderPassMode,
	},
	descriptors::DescriptorSetHandle,
	ImageOrSwapchain, ResourceCollection,
};

const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
pub(super) const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;
const ARGUMENT_TABLE_BUFFER_COUNT: usize = 17;
const PUSH_UPLOAD_ALIGNMENT: usize = 256;
const PUSH_UPLOAD_PAGE_SIZE: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct AppliedDescriptorBinding {
	pipeline: graphics_hardware_interface::PipelineHandle,
	descriptor_sets: SmallVec<[DescriptorSetHandle; 4]>,
	versions: SmallVec<[u64; 4]>,
}

fn attachment_texture_view(
	texture: &Retained<ProtocolObject<dyn mtl::MTLTexture>>,
	format: crate::Formats,
	array_layers: u32,
	layer: Option<u32>,
) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
	if let Some(layer) = layer {
		if array_layers > 1 {
			unsafe {
				return texture
					.newTextureViewWithPixelFormat_textureType_levels_slices(
						utils::to_pixel_format(format),
						mtl::MTLTextureType::Type2D,
						NSRange::new(0, 1),
						NSRange::new(layer as usize, 1),
					)
					.expect(
						"Metal texture view creation failed. The most likely cause is an invalid array-layer render target view.",
					);
			}
		}
	}

	texture.clone()
}

/// Creates a descriptor-visible view when a descriptor selects one mip.
fn descriptor_texture_view(
	texture: &Retained<ProtocolObject<dyn mtl::MTLTexture>>,
	format: crate::Formats,
	mip_level: Option<u32>,
) -> Option<Retained<ProtocolObject<dyn mtl::MTLTexture>>> {
	let mip_level = mip_level?;

	Some(unsafe {
		texture
			.newTextureViewWithPixelFormat_textureType_levels_slices(
				utils::to_pixel_format(format),
				mtl::MTLTextureType::Type2D,
				NSRange::new(mip_level as usize, 1),
				NSRange::new(0, 1),
			)
			.expect(
				"Metal texture mip view creation failed. The most likely cause is that the selected mip exceeds the image mip count.",
			)
	})
}

/// Validates one attachment's declared layer selection against the native texture.
fn validate_attachment_layer_selection(
	layer: Option<u32>,
	layer_count: Option<std::num::NonZeroU32>,
	available_layer_count: u32,
) {
	if let Some(layer) = layer {
		assert!(
			layer < available_layer_count,
			"Render-pass attachment layer is out of bounds. The most likely cause is that the selected layer does not exist in the target image. layer={layer}, available_layers={available_layer_count}",
		);
	}
	let layer_count = layer_count.map_or(1, std::num::NonZeroU32::get);
	assert!(
		layer_count <= available_layer_count,
		"Render-pass attachment layer count is out of bounds. The most likely cause is that layered rendering requested more layers than the target image provides. requested_layers={layer_count}, available_layers={available_layer_count}",
	);
}

#[cfg(test)]
mod tests {
	use super::validate_attachment_layer_selection;

	#[test]
	#[should_panic(expected = "Render-pass attachment layer count is out of bounds")]
	fn layered_rendering_rejects_a_native_texture_with_too_few_layers() {
		validate_attachment_layer_selection(None, std::num::NonZeroU32::new(4), 3);
	}

	#[test]
	fn push_upload_ranges_are_aligned_and_do_not_overlap() {
		assert_eq!(super::push_upload_offset(0, 4, 1024), Some(0));
		assert_eq!(super::push_upload_offset(4, 4, 1024), Some(256));
		assert_eq!(super::push_upload_offset(260, 4, 1024), Some(512));
		assert_eq!(super::push_upload_offset(1020, 8, 1024), None);
	}
}

fn replace_texture_from_bytes(
	texture: &ProtocolObject<dyn mtl::MTLTexture>,
	format: crate::Formats,
	extent: Extent,
	array_layers: u32,
	bytes: &[u8],
) {
	let Some((bytes_per_row, _, bytes_per_image)) = utils::texture_upload_layout(format, extent) else {
		return;
	};

	let region = mtl::MTLRegion {
		origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
		size: {
			let mut size = utils::texture_copy_size(format, extent);
			size.depth = 1;
			size
		},
	};

	for slice in 0..array_layers as usize {
		let offset = slice * bytes_per_image;
		let end = offset + bytes_per_image;

		let Some(slice_bytes) = bytes.get(offset..end) else {
			break;
		};

		let staging_ptr = NonNull::new(slice_bytes.as_ptr() as *mut std::ffi::c_void)
			.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");

		unsafe {
			if array_layers > 1 {
				texture.replaceRegion_mipmapLevel_slice_withBytes_bytesPerRow_bytesPerImage(
					region,
					0,
					slice,
					staging_ptr,
					bytes_per_row as _,
					bytes_per_image as _,
				);
			} else {
				texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
			}
		}
	}

	if utils::is_block_compressed(format) {
		let expected_size = bytes_per_image * array_layers as usize;
		assert_eq!(
			bytes.len(),
			expected_size,
			"Metal compressed texture replacement size mismatch. The most likely cause is that the source payload was not packed as one compact BC image per slice. format={format:?}, extent={extent:?}, array_layers={array_layers}, bytes_len={}, expected_size={expected_size}",
			bytes.len()
		);
	}
}

/// The `RecordingDevice` struct provides command recording with immutable access to backend resources.
pub(super) struct RecordingDevice<'a> {
	pub(super) metal_device: &'a ProtocolObject<dyn mtl::MTLDevice>,
	pub(super) buffers: &'a ResourceCollection<buffer::Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(super) images: &'a ResourceCollection<image::Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(super) samplers: &'a [sampler::Sampler],
	pub(super) acceleration_structures: &'a [AccelerationStructure],
	pub(super) pipeline_layouts: &'a [PipelineLayout],
	pub(super) descriptor_sets: &'a [DescriptorSet],
	pub(super) meshes: &'a [Mesh],
	pub(super) pipelines: &'a [Pipeline],
	pub(super) swapchains: &'a [Swapchain],
	pub(super) debug_labels: bool,
}

/// The `RecordingCommit` struct carries recording results back into the owning device after encoding ends.
pub(super) struct RecordingCommit<'a> {
	pub(super) synchronizers: &'a mut ResourceCollection<
		synchronizer::Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComputeEncoderPhase {
	None,
	Transfer,
	Dispatch,
}

#[derive(Clone, Copy)]
pub(super) enum ArgumentTableStage {
	Compute,
	Vertex,
	Fragment,
	Object,
	Mesh,
}

impl ArgumentTableStage {
	fn index(self) -> usize {
		match self {
			Self::Compute => 0,
			Self::Vertex => 1,
			Self::Fragment => 2,
			Self::Object => 3,
			Self::Mesh => 4,
		}
	}

	fn render_stage(self) -> mtl::MTLRenderStages {
		match self {
			Self::Vertex => mtl::MTLRenderStages::Vertex,
			Self::Fragment => mtl::MTLRenderStages::Fragment,
			Self::Object => mtl::MTLRenderStages::Object,
			Self::Mesh => mtl::MTLRenderStages::Mesh,
			Self::Compute => unreachable!(
				"Invalid Metal render argument-table stage. The most likely cause is that the compute table was bound to a render encoder."
			),
		}
	}
}

/// The `CommandArgumentTables` struct keeps one mutable Metal 4 binding table per shader stage for command-local snapshots.
#[derive(Default)]
pub(super) struct CommandArgumentTables {
	tables: [Option<Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>>; 5],
}

impl CommandArgumentTables {
	fn get(&self, stage: ArgumentTableStage) -> Option<&Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>> {
		self.tables[stage.index()].as_ref()
	}

	fn insert(&mut self, stage: ArgumentTableStage, table: Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>) {
		self.tables[stage.index()] = Some(table);
	}
}

/// The `PushUploadPage` struct keeps immutable push-constant snapshots in one command-local shared Metal buffer.
pub(super) struct PushUploadPage {
	buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	cursor: usize,
}

/// The `PushUploadArena` struct provides aligned, non-overlapping push-constant ranges for one command recording.
#[derive(Default)]
pub(super) struct PushUploadArena {
	pages: SmallVec<[PushUploadPage; 2]>,
}

/// Returns the next aligned upload offset when the requested range fits in the current page.
fn push_upload_offset(cursor: usize, size: usize, capacity: usize) -> Option<usize> {
	let aligned = cursor.checked_add(PUSH_UPLOAD_ALIGNMENT - 1)? & !(PUSH_UPLOAD_ALIGNMENT - 1);
	(aligned.checked_add(size)? <= capacity).then_some(aligned)
}

// TODO: use frame allocator for this
pub struct CommandBufferRecording<'a> {
	device: RecordingDevice<'a>,
	commit: Option<RecordingCommit<'a>>,
	command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	frame_key: Option<graphics_hardware_interface::FrameKey>,
	sequence_index: u8,
	command_buffer: queue::NativeCommand,
	#[cfg(debug_assertions)]
	debug_regions: SmallVec<[Retained<NSString>; 8]>,
	#[cfg(debug_assertions)]
	compute_debug_region_depth: usize,
	#[cfg(debug_assertions)]
	render_debug_region_depth: usize,
	#[cfg(debug_assertions)]
	encoder_block_index: usize,
	active_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_roots: SmallVec<[graphics_hardware_interface::DescriptorSetHandle; 4]>,
	bound_descriptor_set_handles: SmallVec<[DescriptorSetHandle; 4]>,
	bound_descriptor_set_versions: SmallVec<[u64; 4]>,
	bound_vertex_buffers: SmallVec<[(graphics_hardware_interface::BaseBufferHandle, usize); 8]>,
	render_vertex_buffers_dirty: bool,
	encoded_vertex_buffer_count: usize,
	bound_index_buffer: Option<(graphics_hardware_interface::BaseBufferHandle, usize, crate::DataTypes)>,
	push_constant_data: SmallVec<[u8; 128]>,
	compute_push_constants_dirty: bool,
	render_push_constants_dirty: bool,
	active_compute_encoder: Option<Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>>>,
	active_render_encoder: Option<Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>>,
	compute_encoder_phase: ComputeEncoderPhase,
	argument_tables: CommandArgumentTables,
	push_upload_arena: PushUploadArena,
	encoded_compute_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	encoded_render_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	applied_compute_descriptor_binding: Option<AppliedDescriptorBinding>,
	applied_render_descriptor_binding: Option<AppliedDescriptorBinding>,
	drawables: SmallVec<
		[(
			graphics_hardware_interface::SwapchainHandle,
			Retained<ProtocolObject<dyn CAMetalDrawable>>,
		); 4],
	>,
	_autorelease_pool: Option<Retained<NSAutoreleasePool>>,
}

pub struct FinishedCommandBuffer<'a> {
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	pub(crate) command_buffer: queue::NativeCommand,
	pub(crate) _marker: std::marker::PhantomData<&'a ()>,
}

impl crate::command_buffer::CommandBuffer for super::CommandBuffer<'_> {
	fn create_command_buffer_recording(
		&mut self,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		self.device.create_command_buffer_recording(self.command_buffer_handle)
	}
}

impl super::CommandBuffer<'_> {
	pub fn create_command_buffer_recording(&mut self) -> super::CommandBufferRecording<'_> {
		self.device.create_command_buffer_recording(self.command_buffer_handle)
	}
}

impl RecordingCommit<'_> {
	fn synchronizer_for_sequence(
		&self,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		sequence_index: u8,
	) -> crate::synchronizer::SynchronizerHandle {
		self.synchronizers
			.nth_handle(synchronizer_handle, sequence_index as usize)
			.expect(
				"Missing Metal synchronizer. The most likely cause is that the synchronizer handle came from another context.",
			)
	}
}

mod encoding;
mod operations;
mod recording;
