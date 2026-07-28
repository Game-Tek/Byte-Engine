use std::{ptr::NonNull, rc::Rc};

use ::utils::{hash::HashMap, Extent};
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSAutoreleasePool, NSRange, NSString};
use objc2_metal::{
	MTLArgumentEncoder, MTLBlitCommandEncoder, MTLBuffer, MTLCommandBuffer, MTLCommandEncoder, MTLComputeCommandEncoder,
	MTLDevice, MTLRenderCommandEncoder, MTLTexture,
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
	HandleLike as _, ImageOrSwapchain, ResourceCollection,
};

const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

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

/// Flushes CPU writes to a managed Metal buffer before a GPU read command uses that range.
fn flush_managed_buffer_range(buffer: &buffer::Buffer, offset: usize, size: usize) {
	if utils::storage_mode_from_access(buffer.access) != mtl::MTLStorageMode::Managed {
		return;
	}

	let end = offset.checked_add(size).expect(
		"Metal managed buffer flush range overflowed. The most likely cause is an invalid upload buffer offset or size.",
	);
	assert!(
		end <= buffer.size,
		"Metal managed buffer flush range is out of bounds. The most likely cause is that recorded upload ranges exceed the staging buffer. offset={offset}, size={size}, buffer_size={}",
		buffer.size
	);

	buffer.buffer.didModifyRange(NSRange::new(offset, size));
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

// TODO: use frame allocator for this
pub struct CommandBufferRecording<'a> {
	device: RecordingDevice<'a>,
	commit: Option<RecordingCommit<'a>>,
	command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	frame_key: Option<graphics_hardware_interface::FrameKey>,
	sequence_index: u8,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	#[cfg(debug_assertions)]
	debug_regions: SmallVec<[Retained<NSString>; 8]>,
	#[cfg(debug_assertions)]
	compute_debug_region_depth: usize,
	#[cfg(debug_assertions)]
	render_debug_region_depth: usize,
	#[cfg(debug_assertions)]
	blit_debug_region_depth: usize,
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
	active_compute_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>>>,
	active_render_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLRenderCommandEncoder>>>,
	active_blit_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLBlitCommandEncoder>>>,
	encoded_compute_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	encoded_render_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	applied_compute_descriptor_binding: Option<AppliedDescriptorBinding>,
	applied_render_descriptor_binding: Option<AppliedDescriptorBinding>,
	compute_resident_bindings: SmallVec<
		[(
			(DescriptorSetHandle, crate::shader::ResourceSlot),
			(u64, mtl::MTLResourceUsage),
		); 32],
	>,
	render_resident_bindings: SmallVec<
		[(
			(DescriptorSetHandle, crate::shader::ResourceSlot),
			(u64, mtl::MTLResourceUsage, mtl::MTLRenderStages),
		); 32],
	>,
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
	pub(crate) command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
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

impl<'a> CommandBufferRecording<'a> {
	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		let buffer = self.device.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.device.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { &mut *(buffer.pointer as *mut T) }
	}

	/// Records a staging-to-buffer upload on this command buffer.
	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into());
		let buffer = self.device.buffers.resource(buffer_handle);

		let Some(staging_handle) = buffer.staging else {
			return;
		};

		let staging = self.device.buffers.resource(staging_handle);
		let staging_buffer = staging.buffer.clone();
		let destination_buffer = buffer.buffer.clone();
		let destination_size = buffer.size;
		let blit_encoder = self.ensure_blit_encoder().clone();

		unsafe {
			blit_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
				staging_buffer.as_ref(),
				0,
				destination_buffer.as_ref(),
				0,
				destination_size as _,
			);
		}
	}

	pub(super) fn new(
		device: RecordingDevice<'a>,
		commit: Option<RecordingCommit<'a>>,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		drawables: SmallVec<
			[(
				graphics_hardware_interface::SwapchainHandle,
				Retained<ProtocolObject<dyn CAMetalDrawable>>,
			); 4],
		>,
		autorelease_pool: Option<Retained<NSAutoreleasePool>>,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);

		Self {
			device,
			commit,
			command_buffer_handle,
			frame_key,
			sequence_index,
			command_buffer,
			#[cfg(debug_assertions)]
			debug_regions: SmallVec::new(),
			#[cfg(debug_assertions)]
			compute_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			render_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			blit_debug_region_depth: 0,
			drawables,
			active_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_set_roots: SmallVec::new(),
			bound_descriptor_set_handles: SmallVec::new(),
			bound_descriptor_set_versions: SmallVec::new(),
			bound_vertex_buffers: SmallVec::new(),
			render_vertex_buffers_dirty: false,
			encoded_vertex_buffer_count: 0,
			bound_index_buffer: None,
			push_constant_data: SmallVec::new(),
			compute_push_constants_dirty: false,
			render_push_constants_dirty: false,
			active_compute_encoder: None,
			active_render_encoder: None,
			active_blit_encoder: None,
			encoded_compute_pipeline: None,
			encoded_render_pipeline: None,
			applied_compute_descriptor_binding: None,
			applied_render_descriptor_binding: None,
			compute_resident_bindings: SmallVec::new(),
			render_resident_bindings: SmallVec::new(),
			_autorelease_pool: autorelease_pool,
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	fn push_active_compute_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLComputeCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.compute_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	fn push_active_render_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLRenderCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.render_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	fn push_active_blit_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLBlitCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.blit_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Ends the active compute encoder and resets state that is native-encoder-local.
	fn end_compute_encoder(&mut self) {
		let Some(encoder) = self.active_compute_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.compute_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.compute_debug_region_depth = 0;
		}
		encoder.endEncoding();
		self.encoded_compute_pipeline = None;
		self.applied_compute_descriptor_binding = None;
		self.compute_resident_bindings.clear();
		self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
	}

	/// Ends the active render encoder and balances its mirrored debug regions.
	fn end_render_encoder(&mut self) {
		let Some(encoder) = self.active_render_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.render_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.render_debug_region_depth = 0;
		}
		encoder.endEncoding();
		self.encoded_render_pipeline = None;
		self.applied_render_descriptor_binding = None;
		self.render_resident_bindings.clear();
		self.render_push_constants_dirty = !self.push_constant_data.is_empty();
		self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
		self.encoded_vertex_buffer_count = 0;
	}

	/// Ends the active blit encoder and balances its mirrored debug regions.
	fn end_blit_encoder(&mut self) {
		let Some(encoder) = self.active_blit_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.blit_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.blit_debug_region_depth = 0;
		}
		encoder.endEncoding();
	}

	fn ensure_blit_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLBlitCommandEncoder>> {
		self.end_compute_encoder();
		self.end_render_encoder();

		if self.active_blit_encoder.is_none() {
			let encoder = self.command_buffer.blitCommandEncoder().expect(
				"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
			);
			#[cfg(debug_assertions)]
			if self.device.debug_labels {
				encoder.setLabel(Some(objc2_foundation::ns_string!("Blit Pass")));
				self.push_active_blit_debug_regions(encoder.as_ref());
			}
			self.active_blit_encoder = Some(encoder);
		}

		self.active_blit_encoder.as_ref().unwrap()
	}

	/// Retains acquired drawables that may be referenced directly while recording this frame.
	pub(crate) fn attach_drawables(
		&mut self,
		drawables: impl Iterator<
			Item = (
				graphics_hardware_interface::SwapchainHandle,
				Retained<ProtocolObject<dyn CAMetalDrawable>>,
			),
		>,
	) {
		self.drawables.extend(drawables);
	}

	pub(crate) fn into_finished(mut self) -> FinishedCommandBuffer<'static> {
		self.end_render_encoder();
		self.end_compute_encoder();
		self.end_blit_encoder();

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer,
			_marker: std::marker::PhantomData,
		}
	}

	fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>> {
		self.end_render_encoder();
		self.end_blit_encoder();

		if self.active_compute_encoder.is_none() {
			// The ordinary Metal compute encoder is serial. Its dispatch order supplies inter-dispatch dependencies;
			// Metal explicitly ignores memoryBarrier calls unless a concurrent encoder is requested.
			let encoder = self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			#[cfg(debug_assertions)]
			if self.device.debug_labels {
				encoder.setLabel(Some(objc2_foundation::ns_string!("Compute Pass")));
				self.push_active_compute_debug_regions(encoder.as_ref());
			}
			self.active_compute_encoder = Some(encoder);
			self.encoded_compute_pipeline = None;
			self.applied_compute_descriptor_binding = None;
			self.compute_resident_bindings.clear();
			self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
		}

		self.active_compute_encoder.as_ref().unwrap()
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device.images.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	/// Returns the acquired drawable texture for a direct swapchain.
	fn drawable_texture(&self, handle: crate::swapchain::SwapchainHandle) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
		self.drawables
			.iter()
			.find(|(swapchain, _)| swapchain.0 == handle.0)
			.map(|(_, drawable)| drawable.texture())
			.expect("Missing Metal drawable. The most likely cause is that a direct swapchain was used before its frame image was acquired.")
	}

	fn descriptors_at_slot(&self, slot: crate::shader::ResourceSlot) -> Option<&HashMap<u32, Descriptor>> {
		self.descriptors_at_slot_with_owner(slot).map(|(_, descriptors)| descriptors)
	}

	fn descriptors_at_slot_with_owner(
		&self,
		slot: crate::shader::ResourceSlot,
	) -> Option<(DescriptorSetHandle, &HashMap<u32, Descriptor>)> {
		self.bound_descriptor_set_handles.iter().find_map(|set_handle| {
			self.device.descriptor_sets[set_handle.0 as usize]
				.descriptors
				.get(&slot)
				.map(|descriptors| (*set_handle, descriptors))
		})
	}

	fn descriptor_matches_kind(descriptor: Descriptor, kind: crate::shader::ResourceKind) -> bool {
		match descriptor {
			Descriptor::Buffer { .. } => matches!(
				kind,
				crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer
			),
			Descriptor::Image { .. } | Descriptor::Swapchain { .. } => matches!(
				kind,
				crate::shader::ResourceKind::SampledImage
					| crate::shader::ResourceKind::StorageImage
					| crate::shader::ResourceKind::InputAttachment
			),
			Descriptor::CombinedImageSampler { .. } => kind == crate::shader::ResourceKind::CombinedImageSampler,
			Descriptor::Sampler { .. } => kind == crate::shader::ResourceKind::Sampler,
			Descriptor::AccelerationStructure { .. } => kind == crate::shader::ResourceKind::AccelerationStructure,
		}
	}

	/// Validates the retained set union against the active pipeline without requiring fixed arrays to be fully populated.
	fn validate_bound_descriptor_sets(&self, layout: &PipelineLayout) {
		for (left_index, left_handle) in self.bound_descriptor_set_handles.iter().enumerate() {
			let left = &self.device.descriptor_sets[left_handle.0 as usize];
			for right_handle in self.bound_descriptor_set_handles.iter().skip(left_index + 1) {
				let right = &self.device.descriptor_sets[right_handle.0 as usize];
				assert!(
					left.descriptors.keys().all(|slot| !right.descriptors.contains_key(slot)),
					"Overlapping retained descriptor sets. The most likely cause is that two bound sets write the same flat resource slot.",
				);
			}
		}

		for resource in &layout.resources {
			let descriptor = resource.descriptor;
			let range_start = descriptor.slot().index();
			let range_end = resource_range_end(descriptor);
			for set_handle in &self.bound_descriptor_set_handles {
				let descriptor_set = &self.device.descriptor_sets[set_handle.0 as usize];
				assert!(
					descriptor_set
						.descriptors
						.keys()
						.all(|slot| resource_accepts_retained_slot_key(descriptor, *slot)),
					"Invalid retained descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array's base slot.",
				);
			}
			let owner_count = self
				.bound_descriptor_set_handles
				.iter()
				.filter(|set_handle| {
					self.device.descriptor_sets[set_handle.0 as usize]
						.descriptors
						.keys()
						.any(|slot| (range_start..range_end).contains(&slot.index()))
				})
				.count();
			assert!(
				owner_count <= 1,
				"Overlapping retained descriptor sets. The most likely cause is that two bound sets own slots within the same active shader resource range.",
			);

			let descriptors = self.descriptors_at_slot(descriptor.slot());
			if descriptor.count() == 1 {
				assert!(
					descriptors.is_some_and(|descriptors| descriptors.contains_key(&0)),
					"Missing retained descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					descriptor.slot().index(),
				);
			}

			if let Some(descriptors) = descriptors {
				for (&array_element, &value) in descriptors {
					assert!(
						array_element < descriptor.count(),
						"Descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(value, descriptor.kind()),
						"Descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
				}
			}
		}
	}

	fn resize_push_constants_for_layout(&mut self, pipeline_layout: graphics_hardware_interface::PipelineLayoutHandle) {
		let push_constant_size = self.device.pipeline_layouts[pipeline_layout.0 as usize].push_constant_size;
		self.push_constant_data.clear();
		self.push_constant_data.resize(push_constant_size, 0);
		self.compute_push_constants_dirty = push_constant_size > 0;
		self.render_push_constants_dirty = push_constant_size > 0;
	}

	/// Applies changed logical vertex-buffer bindings once before the next ordinary draw.
	fn apply_bound_vertex_buffers(&mut self) {
		if !self.render_vertex_buffers_dirty {
			return;
		}
		let Some(encoder) = self.active_render_encoder.as_ref() else {
			return;
		};

		let mut buffers = SmallVec::<[*const ProtocolObject<dyn mtl::MTLBuffer>; 8]>::new();
		let mut offsets = SmallVec::<[usize; 8]>::new();
		for (buffer_handle, offset) in self.bound_vertex_buffers.iter().copied() {
			let buffer = &self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
			buffers.push(buffer.buffer.as_ref());
			offsets.push(offset);
		}

		if !buffers.is_empty() {
			let buffers = NonNull::new(buffers.as_mut_ptr()).expect("A non-empty Metal vertex buffer list had a null pointer.");
			let offsets =
				NonNull::new(offsets.as_mut_ptr()).expect("A non-empty Metal vertex buffer offset list had a null pointer.");
			unsafe {
				encoder.setVertexBuffers_offsets_withRange(buffers, offsets, NSRange::new(0, self.bound_vertex_buffers.len()));
			}
		}
		for index in self.bound_vertex_buffers.len()..self.encoded_vertex_buffer_count {
			unsafe {
				encoder.setVertexBuffer_offset_atIndex(None, 0, index);
			}
		}
		self.encoded_vertex_buffer_count = self.bound_vertex_buffers.len();
		self.render_vertex_buffers_dirty = false;
	}

	/// Uploads changed push constants once before the next render command.
	fn flush_render_push_constants(&mut self) {
		if !self.render_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pointer = NonNull::new(self.push_constant_data.as_ptr() as *mut std::ffi::c_void)
			.expect("Push constant data pointer was null. The most likely cause is an empty push constant buffer upload.");

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			unsafe {
				encoder.setObjectBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setMeshBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setVertexBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setFragmentBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
			}
		}
		self.render_push_constants_dirty = false;
	}

	/// Uploads changed push constants once before the next compute dispatch.
	fn flush_compute_push_constants(&mut self) {
		if !self.compute_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pointer = NonNull::new(self.push_constant_data.as_ptr() as *mut std::ffi::c_void)
			.expect("Push constant data pointer was null. The most likely cause is an empty push constant buffer upload.");
		let push_constant_size = self.push_constant_data.len();
		unsafe {
			self.ensure_compute_encoder().setBytes_length_atIndex(
				pointer,
				push_constant_size as _,
				PUSH_CONSTANT_BINDING_INDEX as _,
			);
		}
		self.compute_push_constants_dirty = false;
	}

	fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.end_compute_encoder();
		self.end_render_encoder();
		self.end_blit_encoder();

		if let Some(commit) = self.commit.as_mut() {
			let synchronizer = commit.synchronizer_for_sequence(synchronizer, self.sequence_index);
			// Retain the command buffer until a GHI wait observes completion.
			commit
				.synchronizers
				.resource(synchronizer)
				.signal_workload(self.command_buffer.clone());
		}

		device::submit_metal_command_buffer(self.command_buffer.as_ref());
	}
}

impl CommandBufferRecording<'_> {
	fn render_stages(stages: crate::Stages) -> mtl::MTLRenderStages {
		let mut render_stages = mtl::MTLRenderStages(0);

		if stages.intersects(crate::Stages::VERTEX) {
			render_stages |= mtl::MTLRenderStages::Vertex;
		}

		if stages.intersects(crate::Stages::FRAGMENT) {
			render_stages |= mtl::MTLRenderStages::Fragment;
		}

		if stages.intersects(crate::Stages::TASK) {
			render_stages |= mtl::MTLRenderStages::Object;
		}

		if stages.intersects(crate::Stages::MESH) {
			render_stages |= mtl::MTLRenderStages::Mesh;
		}

		if render_stages.is_empty() {
			mtl::MTLRenderStages(
				mtl::MTLRenderStages::Vertex.0
					| mtl::MTLRenderStages::Fragment.0
					| mtl::MTLRenderStages::Object.0
					| mtl::MTLRenderStages::Mesh.0,
			)
		} else {
			render_stages
		}
	}

	fn metal_resource_usage(access: crate::AccessPolicies) -> mtl::MTLResourceUsage {
		let mut usage = mtl::MTLResourceUsage(0);
		if access.intersects(crate::AccessPolicies::READ) {
			usage |= mtl::MTLResourceUsage::Read;
		}
		if access.intersects(crate::AccessPolicies::WRITE) {
			usage |= mtl::MTLResourceUsage::Write;
		}
		usage
	}

	/// Returns the residency usage that still needs to be declared for one compute descriptor slot.
	fn update_compute_binding_residency(
		&mut self,
		set_handle: DescriptorSetHandle,
		version: u64,
		slot: crate::shader::ResourceSlot,
		usage: mtl::MTLResourceUsage,
	) -> Option<mtl::MTLResourceUsage> {
		if let Some((_, (resident_version, resident_usage))) = self
			.compute_resident_bindings
			.iter_mut()
			.find(|(key, _)| *key == (set_handle, slot))
		{
			if *resident_version != version {
				*resident_version = version;
				*resident_usage = usage;
				return Some(usage);
			}

			let combined = mtl::MTLResourceUsage(resident_usage.0 | usage.0);
			if combined.0 == resident_usage.0 {
				return None;
			}
			*resident_usage = combined;
			return Some(combined);
		}

		self.compute_resident_bindings.push(((set_handle, slot), (version, usage)));
		Some(usage)
	}

	/// Returns the residency usage and stages that still need to be declared for one render descriptor slot.
	fn update_render_binding_residency(
		&mut self,
		set_handle: DescriptorSetHandle,
		version: u64,
		slot: crate::shader::ResourceSlot,
		usage: mtl::MTLResourceUsage,
		stages: mtl::MTLRenderStages,
	) -> Option<(mtl::MTLResourceUsage, mtl::MTLRenderStages)> {
		if let Some((_, (resident_version, resident_usage, resident_stages))) = self
			.render_resident_bindings
			.iter_mut()
			.find(|(key, _)| *key == (set_handle, slot))
		{
			if *resident_version != version {
				*resident_version = version;
				*resident_usage = usage;
				*resident_stages = stages;
				return Some((usage, stages));
			}

			let combined_usage = mtl::MTLResourceUsage(resident_usage.0 | usage.0);
			let combined_stages = mtl::MTLRenderStages(resident_stages.0 | stages.0);
			if combined_usage.0 == resident_usage.0 && combined_stages.0 == resident_stages.0 {
				return None;
			}
			*resident_usage = combined_usage;
			*resident_stages = combined_stages;
			return Some((combined_usage, combined_stages));
		}

		self.render_resident_bindings
			.push(((set_handle, slot), (version, usage, stages)));
		Some((usage, stages))
	}

	/// Makes the resources referenced by the flat pipeline interface resident for a render encoder.
	fn make_render_descriptor_resources_resident(
		&mut self,
		encoder: &ProtocolObject<dyn mtl::MTLRenderCommandEncoder>,
		layout: &PipelineLayout,
	) {
		for resource in &layout.resources {
			let slot = resource.descriptor.slot();
			let Some((set_handle, _)) = self.descriptors_at_slot_with_owner(slot) else {
				continue;
			};
			let version = self.device.descriptor_sets[set_handle.0 as usize].version;
			let usage = Self::metal_resource_usage(resource.descriptor.access());
			let stages = Self::render_stages(resource.stages);
			let Some((usage, stages)) = self.update_render_binding_residency(set_handle, version, slot, usage, stages) else {
				continue;
			};
			let descriptors = self
				.descriptors_at_slot(slot)
				.expect("A Metal descriptor slot disappeared while its residency declaration was being recorded.");
			let mut native_resources =
				SmallVec::<[NonNull<ProtocolObject<dyn mtl::MTLResource>>; 32]>::with_capacity(descriptors.len());
			let mut retained_drawable_textures = SmallVec::<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 1]>::new();

			for descriptor in descriptors.values().copied() {
				let native_resource = match descriptor {
					Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
						let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(image).texture;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
						NonNull::from(resource)
					}
					Descriptor::Buffer { buffer, .. } => {
						let buf: &ProtocolObject<dyn mtl::MTLBuffer> = &self.device.buffers.resource(buffer).buffer;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(buf);
						NonNull::from(resource)
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy_handle) =
							self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
						{
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(proxy_handle).texture;
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						} else {
							retained_drawable_textures.push(self.drawable_texture(handle));
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = retained_drawable_textures.last().unwrap().as_ref();
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() else {
							continue;
						};
						let structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure> = structure.as_ref();
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(structure);
						NonNull::from(resource)
					}
					Descriptor::Sampler { .. } => continue,
				};
				native_resources.push(native_resource);
			}

			if !native_resources.is_empty() {
				let resources = NonNull::new(native_resources.as_mut_ptr())
					.expect("A non-empty Metal render residency list had a null pointer.");
				unsafe {
					encoder.useResources_count_usage_stages(resources, native_resources.len(), usage, stages);
				}
			}
		}
	}

	/// Makes the resources referenced by the flat pipeline interface resident for a compute encoder.
	fn make_compute_descriptor_resources_resident(
		&mut self,
		encoder: &ProtocolObject<dyn mtl::MTLComputeCommandEncoder>,
		layout: &PipelineLayout,
	) {
		for resource in &layout.resources {
			let slot = resource.descriptor.slot();
			let Some((set_handle, _)) = self.descriptors_at_slot_with_owner(slot) else {
				continue;
			};
			let version = self.device.descriptor_sets[set_handle.0 as usize].version;
			let usage = Self::metal_resource_usage(resource.descriptor.access());
			let Some(usage) = self.update_compute_binding_residency(set_handle, version, slot, usage) else {
				continue;
			};
			let descriptors = self
				.descriptors_at_slot(slot)
				.expect("A Metal descriptor slot disappeared while its residency declaration was being recorded.");
			let mut native_resources =
				SmallVec::<[NonNull<ProtocolObject<dyn mtl::MTLResource>>; 32]>::with_capacity(descriptors.len());
			let mut retained_drawable_textures = SmallVec::<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 1]>::new();

			for descriptor in descriptors.values().copied() {
				let native_resource = match descriptor {
					Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
						let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(image).texture;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
						NonNull::from(resource)
					}
					Descriptor::Buffer { buffer, .. } => {
						let buf: &ProtocolObject<dyn mtl::MTLBuffer> = &self.device.buffers.resource(buffer).buffer;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(buf);
						NonNull::from(resource)
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy_handle) =
							self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
						{
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(proxy_handle).texture;
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						} else {
							retained_drawable_textures.push(self.drawable_texture(handle));
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = retained_drawable_textures.last().unwrap().as_ref();
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() else {
							continue;
						};
						let structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure> = structure.as_ref();
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(structure);
						NonNull::from(resource)
					}
					Descriptor::Sampler { .. } => continue,
				};
				native_resources.push(native_resource);
			}

			if !native_resources.is_empty() {
				let resources = NonNull::new(native_resources.as_mut_ptr())
					.expect("A non-empty Metal compute residency list had a null pointer.");
				unsafe {
					encoder.useResources_count_usage(resources, native_resources.len(), usage);
				}
			}
		}
	}

	/// Encodes one immutable stage-specific argument buffer from the currently bound retained set union.
	fn encode_stage_argument_buffer(&self, layout: &StageArgumentLayout) -> Retained<ProtocolObject<dyn mtl::MTLBuffer>> {
		let argument_buffer = self
			.device
			.metal_device
			.newBufferWithLength_options(layout.encoded_length as _, mtl::MTLResourceOptions::StorageModeShared)
			.expect("Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.");
		unsafe {
			// Metal does not guarantee fresh buffer contents are zeroed. Null all unwritten array elements deterministically.
			std::ptr::write_bytes(argument_buffer.contents().as_ptr() as *mut u8, 0, layout.encoded_length);
			layout
				.argument_encoder
				.setArgumentBuffer_offset(Some(argument_buffer.as_ref()), 0);
		}

		for binding in &layout.bindings {
			let Some(descriptors) = self.descriptors_at_slot(binding.descriptor.slot()) else {
				continue;
			};

			for (&array_element, &descriptor) in descriptors {
				let argument_slot = binding.slot_for_array_element(array_element);
				match (argument_slot, descriptor) {
					(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
						let buffer = self.device.buffers.resource(buffer);
						layout.argument_encoder.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
					},
					(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, .. }) => unsafe {
						let image = self.device.images.resource(image);
						layout.argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
					},
					(DescriptorBindingSlot::Texture(slot), Descriptor::Swapchain { handle }) => unsafe {
						if let Some(proxy) = self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize] {
							let image = self.device.images.resource(proxy);
							layout.argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
						} else {
							let texture = self.drawable_texture(handle);
							layout.argument_encoder.setTexture_atIndex(Some(texture.as_ref()), slot as _);
						}
					},
					(DescriptorBindingSlot::Sampler(slot), Descriptor::Sampler { sampler }) => unsafe {
						let sampler = &self.device.samplers[sampler.0 as usize];
						layout
							.argument_encoder
							.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
					},
					(
						DescriptorBindingSlot::CombinedImageSampler { texture, sampler },
						Descriptor::CombinedImageSampler {
							image,
							sampler: sampler_handle,
							..
						},
					) => unsafe {
						let image = self.device.images.resource(image);
						let sampler_state = &self.device.samplers[sampler_handle.0 as usize];
						layout.argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), texture as _);
						layout
							.argument_encoder
							.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _);
					},
					(
						DescriptorBindingSlot::AccelerationStructure(slot),
						Descriptor::AccelerationStructure { handle },
					) => {
						if let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() {
							unsafe {
								layout
									.argument_encoder
									.setAccelerationStructure_atIndex(Some(structure.as_ref()), slot as _);
							}
						}
					}
					_ => unreachable!(
						"Validated Metal descriptor kind changed during materialization. The most likely cause is internal descriptor state corruption."
					),
				}
			}
		}

		argument_buffer
	}

	/// Resolves logical descriptor-set roots to the frame-local handles used by this recording.
	fn update_bound_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) {
		if self.bound_descriptor_set_roots.as_slice() != sets {
			self.bound_descriptor_set_roots.clear();
			self.bound_descriptor_set_roots.extend_from_slice(sets);
			self.bound_descriptor_set_handles.clear();

			for descriptor_set_handle in sets {
				let mut resolved = DescriptorSetHandle(descriptor_set_handle.0);
				for _ in 0..self.sequence_index {
					resolved = self.device.descriptor_sets[resolved.0 as usize].next.expect(
						"Missing frame-local Metal descriptor set. The most likely cause is that the retained set chain is shorter than the frame count.",
					);
				}
				self.bound_descriptor_set_handles.push(resolved);
			}
		}
	}

	/// Refreshes retained-set versions so writes made after a logical bind are visible before execution.
	fn refresh_bound_descriptor_set_versions(&mut self) {
		self.bound_descriptor_set_versions.clear();
		self.bound_descriptor_set_versions.extend(
			self.bound_descriptor_set_handles
				.iter()
				.map(|handle| self.device.descriptor_sets[handle.0 as usize].version),
		);
	}

	fn descriptor_binding_matches(
		&self,
		applied: Option<&AppliedDescriptorBinding>,
		pipeline: graphics_hardware_interface::PipelineHandle,
	) -> bool {
		applied.is_some_and(|applied| {
			applied.pipeline == pipeline
				&& applied.descriptor_sets.as_slice() == self.bound_descriptor_set_handles.as_slice()
				&& applied.versions.as_slice() == self.bound_descriptor_set_versions.as_slice()
		})
	}

	/// Returns immutable native argument-buffer snapshots, reusing them while every retained set version is unchanged.
	fn materialize_argument_buffers(&self, pipeline_handle: graphics_hardware_interface::PipelineHandle) -> Materialization {
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let key = MaterializationKey {
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			sequence_index: self.sequence_index,
		};
		let versions = self.bound_descriptor_set_versions.clone();

		if let Some(materialization) = pipeline.materializations.borrow().get(&key) {
			if materialization.versions == versions {
				return materialization.clone();
			}
		}

		let layout = &self.device.pipeline_layouts[pipeline.layout.0 as usize];
		self.validate_bound_descriptor_sets(layout);
		let argument_buffers = Rc::new(
			layout
				.stage_argument_layouts
				.iter()
				.map(|stage_layout| (stage_layout.stage, self.encode_stage_argument_buffer(stage_layout)))
				.collect::<SmallVec<[_; 5]>>(),
		);
		let materialization = Materialization {
			versions,
			argument_buffers,
		};
		pipeline.materializations.borrow_mut().insert(key, materialization.clone());
		materialization
	}

	/// Applies the logical compute pipeline to the current native encoder when required.
	fn apply_bound_compute_pipeline(&mut self) {
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a compute dispatch was recorded before bind_compute_pipeline.",
		);
		if self.encoded_compute_pipeline == Some(pipeline_handle) {
			return;
		}

		let compute_pipeline_state = match &self.device.pipelines[pipeline_handle.0 as usize].pipeline {
			PipelineState::Compute(Some(compute_pipeline_state)) => compute_pipeline_state.clone(),
			PipelineState::Compute(None) => {
				panic!("Metal compute pipeline has no MTLComputePipelineState. The most likely cause is shader creation failed.")
			}
			_ => panic!(
				"Cannot dispatch a non-compute Metal pipeline. The most likely cause is that a raster or ray tracing pipeline handle was passed to bind_compute_pipeline."
			),
		};
		self.ensure_compute_encoder()
			.setComputePipelineState(compute_pipeline_state.as_ref());
		self.encoded_compute_pipeline = Some(pipeline_handle);
	}

	/// Applies the logical render pipeline to the active render pass when required.
	fn apply_bound_render_pipeline(&mut self) {
		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that a draw was recorded before bind_raster_pipeline.");
		if self.encoded_render_pipeline == Some(pipeline_handle) {
			return;
		}

		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let pipeline_state = pipeline.pipeline.clone();
		let depth_stencil_state = pipeline.depth_stencil_state.clone();
		let face_winding = pipeline.face_winding;
		let cull_mode = pipeline.cull_mode;
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that a draw was recorded outside start_render_pass.");

		encoder.setFrontFacingWinding(utils::winding(face_winding));
		encoder.setCullMode(utils::cull_mode(cull_mode));
		encoder.setDepthStencilState(depth_stencil_state.as_ref().map(|state| state.as_ref()));

		match &pipeline_state {
			PipelineState::Raster(Some(render_pipeline_state)) => {
				encoder.setRenderPipelineState(render_pipeline_state);
			}
			PipelineState::Raster(None) => panic!(
				"Metal raster pipeline has no MTLRenderPipelineState. The most likely cause is shader creation failed or SPIR-V was supplied to the Metal backend without translation to MSL or MTLB.",
			),
			_ => panic!(
				"Cannot draw with a non-raster Metal pipeline. The most likely cause is that a compute or ray tracing pipeline handle was passed to bind_raster_pipeline.",
			),
		}

		self.encoded_render_pipeline = Some(pipeline_handle);
	}

	/// Materializes and binds compute descriptors once per pipeline, set version, and native encoder.
	fn apply_bound_compute_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a compute dispatch was recorded before bind_compute_pipeline.",
		);
		if self.descriptor_binding_matches(self.applied_compute_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		let pipeline_layout_handle = self.device.pipelines[pipeline_handle.0 as usize].layout;
		let materialization = self.materialize_argument_buffers(pipeline_handle);
		let encoder = self.active_compute_encoder.clone().expect(
			"No active compute encoder. The most likely cause is that compute descriptors were prepared before a dispatch.",
		);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			if stage.intersects(crate::Stages::COMPUTE) {
				unsafe {
					encoder.setBuffer_offset_atIndex(Some(argument_buffer.as_ref()), 0, ARGUMENT_BUFFER_BINDING_BASE as _);
				}
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.make_compute_descriptor_resources_resident(encoder.as_ref(), pipeline_layout);
		self.applied_compute_descriptor_binding = Some(AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
		});
	}

	/// Materializes and binds render descriptors once per pipeline, set version, and native encoder.
	fn apply_bound_render_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that a draw was recorded before bind_raster_pipeline.");
		if self.descriptor_binding_matches(self.applied_render_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		let pipeline_layout_handle = self.device.pipelines[pipeline_handle.0 as usize].layout;
		let materialization = self.materialize_argument_buffers(pipeline_handle);
		let encoder = self.active_render_encoder.clone().expect(
			"No active render pass. The most likely cause is that render descriptors were prepared before start_render_pass.",
		);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			unsafe {
				if stage.intersects(crate::Stages::TASK) {
					encoder.setObjectBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
				if stage.intersects(crate::Stages::MESH) {
					encoder.setMeshBuffer_offset_atIndex(Some(argument_buffer.as_ref()), 0, ARGUMENT_BUFFER_BINDING_BASE as _);
				}
				if stage.intersects(crate::Stages::VERTEX) {
					encoder.setVertexBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
				if stage.intersects(crate::Stages::FRAGMENT) {
					encoder.setFragmentBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.make_render_descriptor_resources_resident(encoder.as_ref(), pipeline_layout);
		self.applied_render_descriptor_binding = Some(AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
		});
	}

	/// Restores encoder-local compute state immediately before a dispatch.
	fn prepare_compute_dispatch(&mut self) {
		self.apply_bound_compute_pipeline();
		self.apply_bound_compute_descriptors();
	}

	/// Restores encoder-local render state immediately before a draw.
	fn prepare_render_draw(&mut self) {
		self.apply_bound_render_pipeline();
		self.apply_bound_render_descriptors();
	}

	/// Encodes one render-pass clear for a compatible group of color and depth images.
	fn encode_image_clear_batch(&mut self, images: &[(ImageHandle, graphics_hardware_interface::ClearValue)]) {
		let Some((first_handle, _)) = images.first() else {
			return;
		};
		let first_image = self.device.images.resource(*first_handle);
		let rpd = mtl::MTLRenderPassDescriptor::new();
		if first_image.array_layers > 1 {
			rpd.setRenderTargetArrayLength(first_image.array_layers as _);
		}

		let mut color_index = 0;
		for (handle, clear_value) in images {
			let image = self.device.images.resource(*handle);
			if image.format == crate::Formats::Depth32 {
				let attachment = rpd.depthAttachment();
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearDepth(utils::clear_depth(*clear_value));
			} else {
				let attachment = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearColor(utils::clear_color(*clear_value));
				color_index += 1;
			}
		}

		let encoder = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
				"Metal render command encoder creation failed. The most likely cause is that the command buffer could not start an image clear pass.",
		);
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			encoder.setLabel(Some(objc2_foundation::ns_string!("Image Clear")));
			self.push_active_render_debug_regions(encoder.as_ref());
			for _ in 0..self.render_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.render_debug_region_depth = 0;
		}
		encoder.endEncoding();
	}
}

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	fn frame_key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		_acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		self.end_compute_encoder();
		self.end_blit_encoder();

		let attachments = attachments
			.iter()
			.map(|attachment| match attachment.target {
				ImageOrSwapchain::Image(image) => {
					let image = self.device.images.resource(self.get_internal_image_handle(image));

					(attachment, image.texture.clone(), image.format, image.array_layers)
				}
				ImageOrSwapchain::Swapchain(swapchain) => {
					let drawable = self
						.drawables
						.iter()
						.find(|(handle, _)| *handle == swapchain)
						.expect("Swapchain image not found");

					(attachment, drawable.1.texture(), crate::Formats::BGRAu8, 1) // TODO: get actual format
				}
			})
			.collect::<SmallVec<[_; 8]>>();

		let rpd = mtl::MTLRenderPassDescriptor::new();

		for (i, (attachment, image, format, array_layers)) in attachments
			.iter()
			.filter(|(_, _, format, _)| *format != crate::Formats::Depth32)
			.enumerate()
		{
			let att = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(i) };
			let texture_view = attachment_texture_view(image, *format, *array_layers, attachment.layer);

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearColor(utils::clear_color(attachment.clear));
		}

		if let Some((attachment, image, format, array_layers)) = attachments
			.iter()
			.find(|(_, _, format, _)| *format == crate::Formats::Depth32)
		{
			let att = rpd.depthAttachment();
			let texture_view = attachment_texture_view(image, *format, *array_layers, attachment.layer);

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearDepth(utils::clear_depth(attachment.clear));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).unwrap();
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			rce.setLabel(Some(objc2_foundation::ns_string!("Render Pass")));
			self.push_active_render_debug_regions(rce.as_ref());
		}

		rce.setViewport(mtl::MTLViewport {
			originX: 0.0,
			originY: 0.0,
			width: extent.width() as f64,
			height: extent.height() as f64,
			znear: 0.0,
			zfar: 1.0,
		});
		rce.setScissorRect(mtl::MTLScissorRect {
			x: 0,
			y: 0,
			width: extent.width() as _,
			height: extent.height() as _,
		});

		self.active_render_encoder = Some(rce);
		self.encoded_render_pipeline = None;
		self.applied_render_descriptor_binding = None;
		self.render_resident_bindings.clear();
		self.render_push_constants_dirty = !self.push_constant_data.is_empty();
		self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
		self.encoded_vertex_buffer_count = 0;

		self
	}

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		if textures.is_empty() {
			return;
		}

		self.end_compute_encoder();
		self.end_render_encoder();
		self.end_blit_encoder();

		let mut batch = SmallVec::<[(ImageHandle, graphics_hardware_interface::ClearValue); 9]>::new();
		let mut batch_extent = None;
		let mut batch_array_layers = 0;
		let mut color_count = 0;
		let mut has_depth = false;

		for (handle, clear_value) in textures {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);
			let is_depth = image.format == crate::Formats::Depth32;
			let compatible = batch.is_empty()
				|| (batch_extent == Some(image.extent)
					&& batch_array_layers == image.array_layers
					&& !batch.iter().any(|(resident_handle, _)| *resident_handle == image_handle)
					&& if is_depth { !has_depth } else { color_count < 8 });

			if !compatible {
				self.encode_image_clear_batch(&batch);
				batch.clear();
				color_count = 0;
				has_depth = false;
			}

			if batch.is_empty() {
				batch_extent = Some(image.extent);
				batch_array_layers = image.array_layers;
			}
			batch.push((image_handle, *clear_value));
			if is_depth {
				has_depth = true;
			} else {
				color_count += 1;
			}
		}

		self.encode_image_clear_batch(&batch);
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		if buffer_handles.is_empty() {
			return;
		}

		let blit_encoder = self.ensure_blit_encoder().clone();

		for buffer_handle in buffer_handles {
			let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(*buffer_handle));
			blit_encoder.fillBuffer_range_value(buffer.buffer.as_ref(), NSRange::new(0, buffer.size), 0);
		}
	}

	fn copy_buffers(&mut self, copies: &[crate::BufferCopyDescriptor]) {
		if !copies.iter().any(|copy| copy.size > 0) {
			return;
		}

		let blit_encoder = self.ensure_blit_encoder().clone();

		for copy in copies {
			if copy.size == 0 {
				continue;
			}
			let source = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.source_buffer));
			let destination = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.destination_buffer));
			flush_managed_buffer_range(source, copy.source_offset, copy.size);
			unsafe {
				blit_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
					source.buffer.as_ref(),
					copy.source_offset as _,
					destination.buffer.as_ref(),
					copy.destination_offset as _,
					copy.size as _,
				);
			}
		}
	}

	fn copy_buffer_to_images(&mut self, copies: &[crate::BufferImageCopyDescriptor]) {
		if copies.is_empty() {
			return;
		}

		let blit_encoder = self.ensure_blit_encoder().clone();

		for copy in copies {
			let source = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.source_buffer));
			let destination = self
				.device
				.images
				.resource(self.get_internal_image_handle(copy.destination_image));
			let Some((compact_bytes_per_row, row_count, compact_bytes_per_image)) =
				utils::texture_upload_layout(destination.format, destination.extent)
			else {
				panic!(
					"Metal texture copy layout is unsupported. The most likely cause is that the destination format has no upload layout. format={:?}, extent={:?}",
					destination.format, destination.extent
				);
			};
			let expected_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
			let expected_bytes_per_image = expected_bytes_per_row * row_count;
			assert_eq!(
				copy.source_offset % 256,
				0,
				"Metal texture copy source offset alignment mismatch. The most likely cause is that the staging allocator did not provide a 256-byte aligned texture upload offset. source_offset={}, source_bytes_per_row={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				copy.source_offset,
				copy.source_bytes_per_row,
				copy.source_bytes_per_image,
				destination.format,
				destination.extent
			);
			assert_eq!(
				copy.source_bytes_per_row, expected_bytes_per_row,
				"Metal texture copy row pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about BC block row padding. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_row={}, expected={expected_bytes_per_row}",
				destination.format, destination.extent, copy.source_bytes_per_row
			);
			assert_eq!(
				copy.source_bytes_per_image, expected_bytes_per_image,
				"Metal texture copy image pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about padded rows per image. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_image={}, expected={expected_bytes_per_image}",
				destination.format, destination.extent, copy.source_bytes_per_image
			);
			let required_source_bytes = copy
				.source_bytes_per_image
				.checked_mul(destination.array_layers as usize)
				.and_then(|copy_bytes| copy.source_offset.checked_add(copy_bytes))
				.expect(
					"Metal texture copy source bounds overflowed. The most likely cause is an invalid array layer count or image pitch.",
				);
			assert!(
				required_source_bytes <= source.size,
				"Metal texture copy source buffer is too small. The most likely cause is that the staging buffer allocation is smaller than the recorded texture copy. source_size={}, required_source_bytes={required_source_bytes}, source_offset={}, array_layers={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				source.size,
				copy.source_offset,
				destination.array_layers,
				copy.source_bytes_per_image,
				destination.format,
				destination.extent
			);

			flush_managed_buffer_range(source, copy.source_offset, required_source_bytes - copy.source_offset);

			let mut source_size = utils::texture_copy_size(destination.format, destination.extent);
			source_size.depth = 1;
			let destination_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

			for slice in 0..destination.array_layers as usize {
				let source_offset = copy.source_offset + slice * copy.source_bytes_per_image;

				unsafe {
					blit_encoder.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
						source.buffer.as_ref(),
						source_offset as _,
						copy.source_bytes_per_row as _,
						copy.source_bytes_per_image as _,
						source_size,
						destination.texture.as_ref(),
						slice,
						0,
						destination_origin,
					);
				}
			}
		}
	}

	fn copy_images_to_buffer(&mut self, copies: &[crate::ImageBufferCopyDescriptor]) {
		if copies.is_empty() {
			return;
		}

		let blit_encoder = self.ensure_blit_encoder().clone();

		for copy in copies {
			let (source_texture, source_format, source_extent, source_array_layers) = match copy.source {
				ImageOrSwapchain::Image(image) => {
					let source = self.device.images.resource(self.get_internal_image_handle(image));
					(source.texture.clone(), source.format, source.extent, source.array_layers)
				}
				ImageOrSwapchain::Swapchain(swapchain) => {
					if let Some(proxy) = self.device.swapchains[swapchain.0 as usize].images[self.sequence_index as usize] {
						let source = self.device.images.resource(proxy);
						(source.texture.clone(), source.format, source.extent, source.array_layers)
					} else {
						(
							self.drawable_texture(crate::swapchain::SwapchainHandle(swapchain.0)),
							crate::Formats::BGRAu8,
							self.device.swapchains[swapchain.0 as usize].extent,
							1,
						)
					}
				}
			};
			let destination = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.destination_buffer));
			let Some((compact_bytes_per_row, row_count, _)) = utils::texture_upload_layout(source_format, source_extent) else {
				panic!(
					"Metal texture copy layout is unsupported. The most likely cause is that the source format has no buffer copy layout. format={source_format:?}, extent={source_extent:?}"
				);
			};
			let expected_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
			let expected_bytes_per_image = expected_bytes_per_row * row_count;
			assert_eq!(
				copy.destination_offset % 256,
				0,
				"Metal image copy destination offset alignment mismatch. The most likely cause is that the destination buffer offset is not 256-byte aligned. destination_offset={}, destination_bytes_per_row={}, destination_bytes_per_image={}, format={source_format:?}, extent={source_extent:?}",
				copy.destination_offset,
				copy.destination_bytes_per_row,
				copy.destination_bytes_per_image,
			);
			assert_eq!(
				copy.destination_bytes_per_row, expected_bytes_per_row,
				"Metal image copy row pitch mismatch. The most likely cause is that readback preparation and Metal copy recording disagree about row padding. format={source_format:?}, extent={source_extent:?}, compact_bytes_per_row={compact_bytes_per_row}, row_count={row_count}, destination_bytes_per_row={}, expected={expected_bytes_per_row}",
				copy.destination_bytes_per_row
			);
			assert_eq!(
				copy.destination_bytes_per_image, expected_bytes_per_image,
				"Metal image copy image pitch mismatch. The most likely cause is that readback preparation and Metal copy recording disagree about padded rows per image. format={source_format:?}, extent={source_extent:?}, compact_bytes_per_row={compact_bytes_per_row}, row_count={row_count}, destination_bytes_per_image={}, expected={expected_bytes_per_image}",
				copy.destination_bytes_per_image
			);
			let required_destination_bytes = copy
				.destination_bytes_per_image
				.checked_mul(source_array_layers as usize)
				.and_then(|copy_bytes| copy.destination_offset.checked_add(copy_bytes))
				.expect(
					"Metal image copy destination bounds overflowed. The most likely cause is an invalid array layer count or image pitch.",
				);
			assert!(
				required_destination_bytes <= destination.size,
				"Metal image copy destination buffer is too small. The most likely cause is that the readback buffer allocation is smaller than the recorded texture copy. destination_size={}, required_destination_bytes={required_destination_bytes}, destination_offset={}, array_layers={source_array_layers}, destination_bytes_per_image={}, format={source_format:?}, extent={source_extent:?}",
				destination.size,
				copy.destination_offset,
				copy.destination_bytes_per_image,
			);

			let mut source_size = utils::texture_copy_size(source_format, source_extent);
			source_size.depth = 1;
			let source_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

			for slice in 0..source_array_layers as usize {
				let destination_offset = copy.destination_offset + slice * copy.destination_bytes_per_image;
				unsafe {
					blit_encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
						source_texture.as_ref(),
						slice as _,
						0,
						source_origin,
						source_size,
						destination.buffer.as_ref(),
						destination_offset as _,
						copy.destination_bytes_per_row as _,
						copy.destination_bytes_per_image as _,
					);
				}
			}

			if utils::storage_mode_from_access(destination.access) == mtl::MTLStorageMode::Managed {
				blit_encoder.synchronizeResource(destination.buffer.as_ref());
			}
		}
	}

	fn transfer_textures(
		&mut self,
		texture_handles: &[graphics_hardware_interface::BaseImageHandle],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		let mut copies = Vec::with_capacity(texture_handles.len());
		let mut blit_encoder = None;

		for handle in texture_handles {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);
			if !image.access.contains(crate::DeviceAccesses::CpuRead) {
				continue;
			}
			let storage_mode = utils::storage_mode_from_access(image.access);
			let array_layers = image.array_layers;
			let texture = image.texture.clone();

			// Managed Metal textures must be synchronized by the GPU before their compact CPU staging memory is refreshed.
			if storage_mode == mtl::MTLStorageMode::Managed {
				if blit_encoder.is_none() {
					blit_encoder = Some(self.ensure_blit_encoder().clone());
				}
				let encoder = blit_encoder.as_ref().unwrap();
				for slice in 0..array_layers as usize {
					unsafe {
						encoder.synchronizeTexture_slice_level(texture.as_ref(), slice, 0);
					}
				}
			}

			// Match Vulkan: the copy handle is the internal image whose CPU staging storage receives the readback.
			copies.push(graphics_hardware_interface::TextureCopyHandle(image_handle.0));
		}

		copies
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle);

		let image = self.device.images.resource(image_handle);

		let Some(_) = image.staging.as_ref() else {
			return;
		};

		// Metal accepts a CPU pointer for immediate texture replacement, so the caller-provided
		// pixel slice can be used directly instead of cloning through the image staging Vec.
		let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) };

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;

		replace_texture_from_bytes(texture.as_ref(), format, extent, array_layers, bytes);
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		_source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		_destination_layout: crate::Layouts,
	) {
		let source_internal = self.get_internal_image_handle(source_image);
		let destination_internal = self.get_internal_image_handle(destination_image);

		let source_texture = self.device.images.resource(source_internal).texture.clone();
		let destination_texture = self.device.images.resource(destination_internal).texture.clone();
		let blit_encoder = self.ensure_blit_encoder().clone();

		unsafe {
			blit_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
		}
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		CommandBufferRecording::sync_buffer(self, buffer_handle);
	}

	fn execute(self, _synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.finish(_synchronizer);
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline_layout = self.device.pipelines[pipeline_handle.0 as usize].layout;
		if self.active_pipeline_layout != Some(pipeline_layout) {
			self.active_pipeline_layout = Some(pipeline_layout);
			self.resize_push_constants_for_layout(pipeline_layout);
		}

		self
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.active_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self
	}

	fn start_region(&mut self, _write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		#[cfg(debug_assertions)]
		let write_label = _write_label;
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			let mut label = crate::command_buffer::DebugLabelWriter::new();
			write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");
			let name = label.as_str();
			let name = NSString::from_str(name);

			if let Some(encoder) = self.active_compute_encoder.as_ref() {
				encoder.pushDebugGroup(&name);
				self.compute_debug_region_depth += 1;
			}
			if let Some(encoder) = self.active_render_encoder.as_ref() {
				encoder.pushDebugGroup(&name);
				self.render_debug_region_depth += 1;
			}
			if let Some(encoder) = self.active_blit_encoder.as_ref() {
				encoder.pushDebugGroup(&name);
				self.blit_debug_region_depth += 1;
			}
			self.debug_regions.push(name);
		}
	}

	fn end_region(&mut self) {
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			self.debug_regions.pop().expect(
				"Unbalanced Metal debug region. The most likely cause is that end_region was called without start_region.",
			);

			if let Some(encoder) = self.active_compute_encoder.as_ref() {
				encoder.popDebugGroup();
				self.compute_debug_region_depth -= 1;
			}
			if let Some(encoder) = self.active_render_encoder.as_ref() {
				encoder.popDebugGroup();
				self.render_debug_region_depth -= 1;
			}
			if let Some(encoder) = self.active_blit_encoder.as_ref() {
				encoder.popDebugGroup();
				self.blit_debug_region_depth -= 1;
			}
		}
	}

	fn region(
		&mut self,
		write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result,
		f: impl FnOnce(&mut Self),
	) {
		self.start_region(write_label);
		f(self);
		self.end_region();
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline_layout = self.device.pipelines[pipeline_handle.0 as usize].layout;

		if self.active_pipeline_layout != Some(pipeline_layout) {
			self.active_pipeline_layout = Some(pipeline_layout);
			self.resize_push_constants_for_layout(pipeline_layout);
		}

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		assert!(
			buffer_descriptors.len() <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal vertex buffers were bound. The most likely cause is that ordinary vertex bindings overlap the reserved push-constant or argument-buffer slots."
		);
		let bindings = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| (buffer_descriptor.buffer, buffer_descriptor.offset))
			.collect::<SmallVec<[_; 8]>>();
		if self.bound_vertex_buffers != bindings {
			self.bound_vertex_buffers = bindings;
			self.render_vertex_buffers_dirty = true;
		}
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		let index_type = buffer_descriptor.index_type.expect(
			"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32).",
		);

		self.bound_index_buffer = Some((buffer_descriptor.buffer, buffer_descriptor.offset, index_type));
	}

	fn end_render_pass(&mut self) {
		self.end_render_encoder();
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		self.active_pipeline_layout.expect(
			"No pipeline layout is active. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		self.bound_pipeline.expect(
			"No pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		// Binding replaces the complete flat set union; native argument-buffer work is deferred until execution.
		self.update_bound_descriptor_sets(sets);
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let pipeline_layout_handle = self.active_pipeline_layout.expect(
			"No pipeline bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let end = offset as usize + std::mem::size_of::<T>();

		assert!(
			end <= pipeline_layout.push_constant_size,
			"Push constant write exceeds the Metal pipeline layout push constant storage. The most likely cause is that the write offset or type size does not match the pipeline's declared push constant ranges.",
		);

		if self.push_constant_data.len() < pipeline_layout.push_constant_size {
			self.resize_push_constants_for_layout(pipeline_layout_handle);
		}

		unsafe {
			std::ptr::copy_nonoverlapping(
				&data as *const T as *const u8,
				self.push_constant_data[offset as usize..end].as_mut_ptr(),
				std::mem::size_of::<T>(),
			);
		}

		self.compute_push_constants_dirty = true;
		self.render_push_constants_dirty = true;
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		self.prepare_render_draw();
		self.flush_render_push_constants();
		let mesh = &self.device.meshes[mesh_handle.0 as usize];
		assert!(
			mesh.vertex_buffers.len() <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal mesh vertex buffers were bound. The most likely cause is that mesh bindings overlap the reserved push-constant or argument-buffer slots."
		);
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that draw_mesh was called outside start_render_pass.");

		unsafe {
			let binding_count = mesh.vertex_buffers.len().max(self.encoded_vertex_buffer_count);
			for binding in 0..binding_count {
				let vertex_buffer = mesh
					.vertex_buffers
					.get(binding)
					.and_then(|vertex_buffer| vertex_buffer.as_ref())
					.map(|vertex_buffer| vertex_buffer.as_ref());
				encoder.setVertexBuffer_offset_atIndex(vertex_buffer, 0, binding as _);
			}
			encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
				mtl::MTLPrimitiveType::Triangle,
				mesh.index_count as _,
				mtl::MTLIndexType::UInt16,
				mesh.index_buffer.as_ref(),
				0,
			);
		}
		self.encoded_vertex_buffer_count = mesh.vertex_buffers.len();
		// Mesh-owned bindings replace the ordinary logical bindings even when that logical list is empty.
		self.render_vertex_buffers_dirty = true;
	}

	fn draw(&mut self, vertex_count: u32, _instance_count: u32, first_vertex: u32, _first_instance: u32) {
		self.prepare_render_draw();
		self.apply_bound_vertex_buffers();
		self.flush_render_push_constants();
		unsafe {
			self.active_render_encoder
				.as_ref()
				.unwrap()
				.drawPrimitives_vertexStart_vertexCount(mtl::MTLPrimitiveType::Triangle, first_vertex as _, vertex_count as _);
		}
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		self.prepare_render_draw();
		self.apply_bound_vertex_buffers();
		self.flush_render_push_constants();
		let (buffer_handle, offset, index_type) = self
			.bound_index_buffer
			.expect("No index buffer bound. The most likely cause is that draw_indexed was called before bind_index_buffer.");
		let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
		let (metal_index_type, index_size) = match index_type {
			crate::DataTypes::U16 => (mtl::MTLIndexType::UInt16, std::mem::size_of::<u16>()),
			crate::DataTypes::U32 => (mtl::MTLIndexType::UInt32, std::mem::size_of::<u32>()),
			_ => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
		};
		let index_buffer_offset = offset + first_index as usize * index_size;

		unsafe {
			self.active_render_encoder
				.as_ref()
				.unwrap()
				.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(
					mtl::MTLPrimitiveType::Triangle,
					index_count as _,
					metal_index_type,
					buffer.buffer.as_ref(),
					index_buffer_offset as _,
					instance_count as _,
					vertex_offset as _,
					first_instance as _,
				);
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		self.prepare_render_draw();
		self.flush_render_push_constants();
		let bound_pipeline = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that dispatch_meshes was called before bind_raster_pipeline.");
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let mesh_threadgroup_size = pipeline.mesh_threadgroup_size.expect(
			"Metal mesh dispatch requires mesh threadgroup metadata. The most likely cause is that the mesh shader was not generated with Metal mesh threadgroup size metadata.",
		);
		let object_threadgroup_size = pipeline.object_threadgroup_size.unwrap_or(Extent::new(1, 1, 1));

		self.active_render_encoder
			.as_ref()
			.expect(
				"No active render pass. The most likely cause is that dispatch_meshes was called outside start_render_pass.",
			)
			.drawMeshThreadgroups_threadsPerObjectThreadgroup_threadsPerMeshThreadgroup(
				mtl::MTLSize {
					width: x as _,
					height: y as _,
					depth: z as _,
				},
				mtl::MTLSize {
					width: object_threadgroup_size.width() as _,
					height: object_threadgroup_size.height() as _,
					depth: object_threadgroup_size.depth() as _,
				},
				mtl::MTLSize {
					width: mesh_threadgroup_size.width() as _,
					height: mesh_threadgroup_size.height() as _,
					depth: mesh_threadgroup_size.depth() as _,
				},
			);
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let threadgroups = dispatch.get_extent();
		let threads_per_threadgroup = dispatch.get_workgroup_extent();
		self.prepare_compute_dispatch();
		self.flush_compute_push_constants();

		self.ensure_compute_encoder().dispatchThreadgroups_threadsPerThreadgroup(
			mtl::MTLSize {
				width: threadgroups.width() as _,
				height: threadgroups.height() as _,
				depth: threadgroups.depth() as _,
			},
			mtl::MTLSize {
				width: threads_per_threadgroup.width().max(1) as _,
				height: threads_per_threadgroup.height().max(1) as _,
				depth: threads_per_threadgroup.depth().max(1) as _,
			},
		);
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<[[u32; 4]; N]>,
		entry_index: usize,
	) {
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle.into());
		let buffer = self.device.buffers.resource(internal_buffer).buffer.clone();

		self.prepare_compute_dispatch();
		self.flush_compute_push_constants();

		let bound_pipeline = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that indirect_dispatch was called before bind_compute_pipeline.",
		);
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let threadgroup_extent = pipeline.compute_threadgroup_size.unwrap_or(Extent::line(128));

		unsafe {
			self.ensure_compute_encoder()
				.dispatchThreadgroupsWithIndirectBuffer_indirectBufferOffset_threadsPerThreadgroup(
					buffer.as_ref(),
					(entry_index * std::mem::size_of::<[u32; 4]>()) as _,
					mtl::MTLSize {
						width: threadgroup_extent.width().max(1) as _,
						height: threadgroup_extent.height().max(1) as _,
						depth: threadgroup_extent.depth().max(1) as _,
					},
				);
		}
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: Encode Metal ray tracing dispatch.
	}
}
