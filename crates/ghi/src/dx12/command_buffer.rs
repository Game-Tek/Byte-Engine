use smallvec::SmallVec;
use utils::Extent;

use crate::{
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommonCommandBufferMode, RasterizationRenderPassMode,
	},
	rt::{BindingTables, BottomLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuild},
	AttachmentInformation, BaseBufferHandle, BaseImageHandle, BufferCopyDescriptor, BufferDescriptor, BufferHandle,
	BufferImageCopyDescriptor, ClearValue, DescriptorSetHandle, DispatchExtent, ImageOrSwapchain, Layouts, MeshHandle,
	PipelineHandle, PipelineLayoutHandle, RGBAu8, SynchronizerHandle, TextureCopyHandle,
};

pub struct CommandBufferRecording<'a> {
	device: &'a mut super::Device,
	command_buffer: crate::CommandBufferHandle,
	frame_key: Option<crate::FrameKey>,
	bound_pipeline_layout: Option<PipelineLayoutHandle>,
	bound_pipeline: Option<PipelineHandle>,
	bound_descriptor_sets: SmallVec<[DescriptorSetHandle; 8]>,
	descriptor_tables_dirty: bool,
	active_render_target: Option<BaseImageHandle>,
	active_extent: Option<Extent>,
	push_constants: Vec<u8>,
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut super::Device,
		command_buffer: crate::CommandBufferHandle,
		frame_key: Option<crate::FrameKey>,
	) -> Self {
		Self {
			device,
			command_buffer,
			frame_key,
			bound_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_sets: SmallVec::new(),
			descriptor_tables_dirty: false,
			active_render_target: None,
			active_extent: None,
			push_constants: Vec::new(),
		}
	}

	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T {
		unsafe { std::mem::transmute::<&mut T, &'static mut T>(self.device.get_mut_buffer_slice(buffer_handle)) }
	}

	fn sequence_index(&self) -> u8 {
		self.frame_key.map(|frame_key| frame_key.sequence_index).unwrap_or(0)
	}

	/// Rebinds descriptor tables when an intervening command changed the active DX12 descriptor heap.
	fn refresh_descriptor_tables_if_dirty(&mut self) {
		if !self.descriptor_tables_dirty || self.bound_descriptor_sets.is_empty() {
			return;
		}
		self.device.flush_pending_descriptor_texture_syncs(
			self.command_buffer,
			&self.bound_descriptor_sets,
			self.sequence_index(),
		);
		self.device.bind_descriptor_heaps_and_tables(
			self.command_buffer,
			self.bound_pipeline,
			&self.bound_descriptor_sets,
			self.sequence_index(),
		);
		self.descriptor_tables_dirty = false;
	}

	pub(crate) fn record_present_preparation(&mut self, present_keys: &[crate::PresentKey]) {
		self.device.record_present_preparation(self.command_buffer, present_keys);
	}
}

impl crate::command_buffer::CommandBufferRecording for CommandBufferRecording<'_> {
	fn frame_key(&self) -> crate::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn build_top_level_acceleration_structure(&mut self, _acceleration_structure_build: &TopLevelAccelerationStructureBuild) {
		self.device.record_top_level_acceleration_structure_build(
			self.command_buffer,
			_acceleration_structure_build,
			self.sequence_index(),
		);
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[BottomLevelAccelerationStructureBuild],
	) {
		self.device.record_bottom_level_acceleration_structure_builds(
			self.command_buffer,
			_acceleration_structure_builds,
			self.sequence_index(),
		);
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		let sequence_index = self.sequence_index();
		self.active_extent = Some(extent);
		self.active_render_target = attachments.iter().find_map(|attachment| match attachment.target {
			ImageOrSwapchain::Image(image) => Some(image),
			ImageOrSwapchain::Swapchain(swapchain) => Some(
				self.device
					.get_swapchain_image_for_sequence(swapchain, crate::Uses::RenderTarget, sequence_index)
					.0
					.into(),
			),
		});

		self.device
			.bind_render_targets_native(self.command_buffer, attachments, sequence_index);
		self.device.set_render_area_native(self.command_buffer, extent);

		self
	}

	fn clear_images(&mut self, _textures: &[(BaseImageHandle, ClearValue)]) {
		for &(image, clear) in _textures {
			self.device
				.record_image_clear(self.command_buffer, crate::ImageHandle(image), clear, self.sequence_index());
		}
		self.descriptor_tables_dirty = true;
	}

	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]) {
		self.device
			.clear_buffers(self.command_buffer, buffer_handles, self.sequence_index());
		self.descriptor_tables_dirty = true;
	}

	fn copy_buffers(&mut self, copies: &[BufferCopyDescriptor]) {
		self.device.copy_buffers(self.command_buffer, copies, self.sequence_index());
	}

	fn copy_buffer_to_images(&mut self, copies: &[BufferImageCopyDescriptor]) {
		self.device
			.copy_buffer_to_images(self.command_buffer, copies, self.sequence_index());
	}

	fn copy_images_to_buffer(&mut self, _copies: &[crate::ImageBufferCopyDescriptor]) {
		panic!(
			"DX12 image-to-buffer copy is not implemented. The most likely cause is that this backend has not been wired for arbitrary texture readback buffers."
		);
	}

	fn transfer_textures(&mut self, texture_handles: &[BaseImageHandle]) -> Vec<TextureCopyHandle> {
		texture_handles
			.iter()
			.map(|handle| {
				self.device.flush_pending_texture_syncs(self.command_buffer, Some(*handle));
				let copy = self
					.device
					.copy_image_to_cpu_for_sequence(crate::ImageHandle(*handle), self.sequence_index());
				self.device.record_image_readback_for_copy(
					self.command_buffer,
					crate::ImageHandle(*handle),
					copy,
					self.sequence_index(),
				);
				copy
			})
			.collect()
	}

	fn write_image_data(&mut self, image_handle: BaseImageHandle, data: &[RGBAu8]) {
		self.device
			.write_image_data_for_sequence(crate::ImageHandle(image_handle), data, self.sequence_index());
		self.device.record_image_data_write(
			self.command_buffer,
			crate::ImageHandle(image_handle),
			data,
			self.sequence_index(),
		);
	}

	fn blit_image(
		&mut self,
		source_image: BaseImageHandle,
		_source_layout: Layouts,
		destination_image: BaseImageHandle,
		_destination_layout: Layouts,
	) {
		self.device
			.flush_pending_texture_syncs(self.command_buffer, Some(source_image));
		self.device
			.copy_image_for_sequences(source_image, destination_image, self.sequence_index(), self.sequence_index());
		self.device
			.record_image_copy(self.command_buffer, source_image, destination_image);
	}

	fn execute(self, synchronizer: SynchronizerHandle) {
		self.device.submit_command_buffer(self.command_buffer, synchronizer);
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self.device.bind_pipeline_native_state(self.command_buffer, pipeline_handle);
		self
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self.device.bind_pipeline_native_state(self.command_buffer, pipeline_handle);
		self
	}

	fn start_region(&self, _write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		#[cfg(debug_assertions)]
		let write_label = _write_label;
		#[cfg(debug_assertions)]
		{
			let mut label = crate::command_buffer::DebugLabelWriter::new();
			write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");
			self.device.begin_debug_region(self.command_buffer, label.as_str());
		}
	}

	fn end_region(&self) {
		#[cfg(debug_assertions)]
		self.device.end_debug_region(self.command_buffer);
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
	fn bind_raster_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self.device.bind_pipeline_native_state(self.command_buffer, pipeline_handle);
		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]) {
		self.refresh_descriptor_tables_if_dirty();
		self.device
			.bind_vertex_buffers_native(self.command_buffer, buffer_descriptors, self.sequence_index());
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor) {
		self.refresh_descriptor_tables_if_dirty();
		self.device
			.bind_index_buffer_native(self.command_buffer, buffer_descriptor, self.sequence_index());
	}

	fn end_render_pass(&mut self) {
		self.device.end_render_pass_native(self.command_buffer);
		self.active_render_target = None;
		self.active_extent = None;
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[DescriptorSetHandle]) -> &mut Self {
		self.bound_descriptor_sets.clear();
		self.bound_descriptor_sets.extend_from_slice(sets);
		self.device
			.flush_pending_descriptor_texture_syncs(self.command_buffer, sets, self.sequence_index());
		self.device
			.bind_descriptor_heaps_and_tables(self.command_buffer, self.bound_pipeline, sets, self.sequence_index());
		self.descriptor_tables_dirty = false;
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let offset = offset as usize;
		let size = std::mem::size_of::<T>();
		let end = offset + size;
		if self.push_constants.len() < end {
			self.push_constants.resize(end, 0);
		}
		let bytes = unsafe { std::slice::from_raw_parts((&data as *const T).cast::<u8>(), size) };
		self.push_constants[offset..end].copy_from_slice(bytes);
		self.device
			.write_push_constants_native(self.command_buffer, self.bound_pipeline, offset as u32, bytes);
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, _mesh_handle: &MeshHandle) {
		self.refresh_descriptor_tables_if_dirty();
		self.device.draw_mesh_native(self.command_buffer, *_mesh_handle);

		let Some(target) = self.active_render_target else {
			return;
		};
		let Some(extent) = self.active_extent else {
			return;
		};

		let transform = if self.push_constants.len() >= std::mem::size_of::<[f32; 16]>() {
			let mut matrix = [0.0f32; 16];
			let bytes =
				unsafe { std::slice::from_raw_parts_mut(matrix.as_mut_ptr().cast::<u8>(), std::mem::size_of::<[f32; 16]>()) };
			bytes.copy_from_slice(&self.push_constants[..std::mem::size_of::<[f32; 16]>()]);
			Some(matrix)
		} else {
			None
		};

		self.device
			.rasterize_mesh_to_image(*_mesh_handle, target, extent, transform, self.sequence_index());
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		self.refresh_descriptor_tables_if_dirty();
		self.device.draw_native(
			self.command_buffer,
			vertex_count,
			instance_count,
			first_vertex,
			first_instance,
		);
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		self.refresh_descriptor_tables_if_dirty();
		self.device.draw_indexed_native(
			self.command_buffer,
			index_count,
			instance_count,
			first_index,
			vertex_offset,
			first_instance,
		);
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		self.refresh_descriptor_tables_if_dirty();
		self.device
			.dispatch_meshes_native(self.command_buffer, self.bound_pipeline, x, y, z);
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: DispatchExtent) {
		self.refresh_descriptor_tables_if_dirty();
		self.device
			.dispatch_compute_native(self.command_buffer, self.bound_pipeline, dispatch);
	}

	fn indirect_dispatch<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 4]; N]>, entry_index: usize) {
		self.refresh_descriptor_tables_if_dirty();
		self.device
			.dispatch_compute_indirect_native(self.command_buffer, buffer, entry_index);
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: BindingTables, x: u32, y: u32, z: u32) {
		self.refresh_descriptor_tables_if_dirty();
		self.device.trace_rays_native(
			self.command_buffer,
			self.bound_pipeline,
			binding_tables,
			x,
			y,
			z,
			self.sequence_index(),
		);
	}
}
