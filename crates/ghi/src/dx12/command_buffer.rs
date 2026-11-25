use smallvec::SmallVec;
use utils::Extent;

use crate::{
	AttachmentInformation, BaseBufferHandle, BaseImageHandle, BufferCopyDescriptor, BufferDescriptor, BufferHandle,
	BufferImageCopyDescriptor, ClearValue, DescriptorSetHandle, DispatchExtent, ImageOrSwapchain, Layouts, MeshHandle,
	PipelineHandle, PipelineLayoutHandle, SynchronizerHandle, TextureCopyHandle,
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommonCommandBufferMode, RasterizationRenderPassMode,
	},
};

pub struct CommandBufferRecording<'a> {
	device: &'a mut super::Device,
	command_buffer: crate::CommandBufferHandle,
	frame_key: Option<crate::FrameKey>,
	bound_pipeline_layout: Option<PipelineLayoutHandle>,
	bound_pipeline: Option<PipelineHandle>,
	bound_descriptor_sets: SmallVec<[DescriptorSetHandle; 8]>,
	descriptor_tables_dirty: bool,
	/// Set when recorded GPU work may have written or moved the state of a resource the bound sets hold, so the next
	/// shader command transitions them again. A dispatch after a dispatch needs the UAV barrier, and a copy leaves its
	/// images in the common layout.
	descriptor_states_stale: bool,
	active_attachments: SmallVec<[BaseImageHandle; 8]>,
	active_extent: Option<Extent>,
	queued_for_submission: bool,
	/// The first counter slot this recording may write, so finishing resolves only the slots it wrote.
	counter_slots_start: u32,
	/// The first statistics slot this recording opens, so it resolves the ones it wrote.
	statistics_slots_start: u32,
}

impl Drop for CommandBufferRecording<'_> {
	fn drop(&mut self) {
		if !self.queued_for_submission {
			self.device.discard_command_buffer_recording(self.command_buffer);
		}
	}
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut super::Device,
		command_buffer: crate::CommandBufferHandle,
		frame_key: Option<crate::FrameKey>,
	) -> Self {
		let counter_slots_start = frame_key.map_or(0, |frame_key| device.counter_written_slots(frame_key.sequence_index).end);
		let statistics_slots_start =
			frame_key.map_or(0, |frame_key| device.statistics_written_slots(frame_key.sequence_index).end);
		Self {
			device,
			command_buffer,
			frame_key,
			bound_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_sets: SmallVec::new(),
			descriptor_tables_dirty: false,
			descriptor_states_stale: false,
			active_attachments: SmallVec::new(),
			active_extent: None,
			queued_for_submission: false,
			counter_slots_start,
			statistics_slots_start,
		}
	}

	/// Submits a workload's recording with `synchronizer`, for [`crate::context::Context::record`], which waits for it.
	pub(crate) fn submit_workload(self, synchronizer: SynchronizerHandle) {
		// Unwind cleanup stays armed until submission moves the recording into the native queue.
		self.device.finish_command_buffer_recording(self.command_buffer);
		self.device.submit_command_buffer(self.command_buffer, synchronizer);
	}

	/// Returns the bound pipeline, which every shader command and binding needs.
	fn pipeline(&self) -> PipelineHandle {
		self.bound_pipeline
			.expect("No pipeline is bound. The most likely cause is that a shader command or binding came before bind_*_pipeline. Bind a pipeline first.")
	}

	fn sequence_index(&self) -> u8 {
		self.frame_key.map(|frame_key| frame_key.sequence_index).unwrap_or(0)
	}

	/// Validates the complete descriptor access contract before recording barriers or native bindings.
	fn validate_descriptor_bindings(&self, pipeline: PipelineHandle, sets: &[DescriptorSetHandle]) {
		self.device
			.validate_descriptor_binding_contract(pipeline, sets, self.sequence_index(), &self.active_attachments);
	}

	/// Binds the sets again when the pipeline changed, which resets the root constants, or when a clear moved the
	/// state of a resource they hold.
	fn refresh_descriptor_tables_if_dirty(&mut self) {
		if !self.descriptor_tables_dirty || self.bound_descriptor_sets.is_empty() {
			return;
		}
		let pipeline = self.pipeline();
		self.validate_descriptor_bindings(pipeline, &self.bound_descriptor_sets);
		self.bind_descriptor_tables(pipeline);
	}

	/// Readies the bound sets for a shader command: binds them again when they are dirty, and otherwise transitions
	/// their resources again when recorded work since the last transition may have changed what they need.
	fn prepare_shader_work(&mut self) {
		if self.descriptor_tables_dirty {
			self.refresh_descriptor_tables_if_dirty();
			return;
		}
		if !self.descriptor_states_stale || self.bound_descriptor_sets.is_empty() {
			return;
		}
		let pipeline = self.pipeline();
		self.device.flush_pending_descriptor_texture_syncs(
			self.command_buffer,
			pipeline,
			&self.bound_descriptor_sets,
			self.sequence_index(),
		);
		self.descriptor_states_stale = false;
	}

	/// Transitions the resources the bound sets hold for the pipeline, then gives each stage its set bases.
	fn bind_descriptor_tables(&mut self, pipeline: PipelineHandle) {
		self.device.flush_pending_descriptor_texture_syncs(
			self.command_buffer,
			pipeline,
			&self.bound_descriptor_sets,
			self.sequence_index(),
		);
		self.device.bind_descriptor_sets_native(
			self.command_buffer,
			pipeline,
			&self.bound_descriptor_sets,
			self.sequence_index(),
		);
		self.descriptor_tables_dirty = false;
		self.descriptor_states_stale = false;
	}

	/// Binds a pipeline's native state and marks the bound sets for a bind before the next command that reads them.
	///
	/// The sets are checked against the pipeline then, so a caller can bind the sets of a pipeline with other layouts
	/// after it.
	fn bind_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut Self {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.index() as usize].layout);
		self.device.bind_pipeline_native_state(self.command_buffer, pipeline_handle);
		self.descriptor_tables_dirty = !self.bound_descriptor_sets.is_empty();
		self
	}

	/// Validates an image as a transfer source and records its readback from one frame sequence.
	fn transfer_image(
		&mut self,
		handle: BaseImageHandle,
		sequence_index: u8,
	) -> Result<TextureCopyHandle, crate::TextureTransferError> {
		self.device.validate_texture_transfer_source(crate::ImageHandle(handle))?;
		self.descriptor_states_stale = true;
		self.device
			.record_image_readback_for_copy(self.command_buffer, crate::ImageHandle(handle), sequence_index)
	}

	pub(crate) fn record_present_preparation(&mut self, present_keys: &[crate::PresentKey]) {
		self.device.record_present_preparation(self.command_buffer, present_keys);
	}

	/// Transfers readback ownership to the queue path that will submit this command buffer.
	pub(crate) fn finish_for_submission(&mut self) {
		if let Some(frame_key) = self.frame_key {
			// Timestamps live in an opaque heap until resolved, so the list copies the slots it wrote out last.
			let slots = self.counter_slots_start..self.device.counter_written_slots(frame_key.sequence_index).end;
			self.device.resolve_counter_slots(self.command_buffer, slots);
			let slots = self.statistics_slots_start..self.device.statistics_written_slots(frame_key.sequence_index).end;
			self.device.resolve_statistics_slots(self.command_buffer, slots);
		}
		self.device.finish_command_buffer_recording(self.command_buffer);
		self.queued_for_submission = true;
	}
}

impl crate::command_buffer::CommandBufferRecording for CommandBufferRecording<'_> {
	fn frame_key(&self) -> crate::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn unordered(&mut self, f: impl FnOnce(&mut Self)) {
		self.device.unordered_group = Some(SmallVec::new());
		f(self);
		self.device.unordered_group = None;
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		structure: crate::TopLevelAccelerationStructureHandle,
		instances: &[crate::rt::Instance],
	) {
		let frame_key = self.frame_key.unwrap_or_else(|| crate::rt::outside_frame("DX12"));
		self.device
			.record_top_level_build(self.command_buffer, frame_key, structure, instances);
	}

	fn build_bottom_level_acceleration_structures(&mut self, structures: &[crate::BottomLevelAccelerationStructureHandle]) {
		let frame_sequence = self.frame_key.map(|frame_key| frame_key.sequence_index);
		self.device
			.record_bottom_level_builds(self.command_buffer, frame_sequence, self.sequence_index(), structures);
	}

	#[allow(unreachable_code)]
	fn start_tile_pass(
		&mut self,
		_: PipelineHandle,
		_: Extent,
		_: &[AttachmentInformation],
	) -> &mut impl crate::command_buffer::BoundTilePipelineMode {
		crate::pipelines::tile::unsupported("DX12");
		self
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		AttachmentInformation::render_pass_layer_count(attachments);
		let sequence_index = self.sequence_index();
		let mut active_attachments = SmallVec::<[BaseImageHandle; 8]>::new();
		for attachment in attachments {
			let ImageOrSwapchain::Image(image) = attachment.target else {
				continue;
			};
			assert!(
				!active_attachments.contains(&image),
				"Incompatible DX12 attachment aliases. The most likely cause is that one whole image was bound more than once as a writable render-pass attachment. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#single-queue-simultaneous-access."
			);
			active_attachments.push(image);
		}
		// Sets waiting for a bind are checked against the active attachments when the next draw binds them. Only a
		// raster pipeline bound before the pass can draw in it; a compute pipeline left bound by earlier work can't, so
		// its sets may hold the attachments, as a copy into an image the pass then draws on does.
		if let Some(pipeline) = self.bound_pipeline.filter(|&pipeline| {
			!self.bound_descriptor_sets.is_empty()
				&& !self.descriptor_tables_dirty
				&& self.device.pipelines[pipeline.index() as usize].kind == crate::dx12::context::PipelineKind::Raster
		}) {
			self.device.validate_descriptor_binding_contract(
				pipeline,
				&self.bound_descriptor_sets,
				sequence_index,
				&active_attachments,
			);
		}
		self.active_attachments = active_attachments;
		self.active_extent = Some(extent);

		self.device
			.bind_render_targets_native(self.command_buffer, attachments, sequence_index);
		self.device.set_render_area_native(self.command_buffer, extent);

		self
	}

	fn discard_images(&mut self, images: &[BaseImageHandle]) {
		let sequence_index = self.sequence_index();
		for &image in images {
			self.device.initialize_image_contents(image, sequence_index);
		}
	}

	fn clear_images(&mut self, _textures: &[(BaseImageHandle, ClearValue)]) {
		self.device
			.clear_images(self.command_buffer, _textures, self.sequence_index());
		self.descriptor_tables_dirty = true;
	}

	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]) {
		self.device
			.clear_buffers(self.command_buffer, buffer_handles, self.sequence_index());
		self.descriptor_tables_dirty = true;
	}

	fn copy_buffers(&mut self, copies: &[BufferCopyDescriptor]) {
		self.device.copy_buffers(self.command_buffer, copies, self.sequence_index());
		self.descriptor_states_stale = true;
	}

	fn copy_buffer_to_images(&mut self, copies: &[BufferImageCopyDescriptor]) {
		self.device
			.copy_buffer_to_images(self.command_buffer, copies, self.sequence_index());
		self.descriptor_states_stale = true;
	}

	fn transfer_texture(&mut self, source: ImageOrSwapchain) -> Result<TextureCopyHandle, crate::TextureTransferError> {
		let sequence_index = self.sequence_index();
		let handle = match source {
			ImageOrSwapchain::Image(handle) => handle,
			// A swapchain's frames render through its proxy image, which presentation copies into the backbuffer.
			ImageOrSwapchain::Swapchain(swapchain) => self
				.device
				.get_swapchain_image(swapchain, crate::Uses::TransferSource)
				.0
				.into(),
		};
		self.transfer_image(handle, sequence_index)
	}

	fn transfer_texture_with_frame(
		&mut self,
		image: BaseImageHandle,
		frame_offset: i32,
	) -> Result<TextureCopyHandle, crate::TextureTransferError> {
		crate::descriptors::assert_frame_offset(self.device.image_copy_count(image), frame_offset);
		// Descriptor writes select other frames' copies the same way, so both paths agree on which copy is "previous".
		let sequence_index = self.device.frame_index_with_offset(self.sequence_index(), frame_offset);
		self.transfer_image(image, sequence_index)
	}

	fn blit_image(
		&mut self,
		source_image: BaseImageHandle,
		_source_layout: Layouts,
		destination_image: BaseImageHandle,
		_destination_layout: Layouts,
	) {
		self.device
			.record_image_copy(self.command_buffer, source_image, destination_image, self.sequence_index());
		self.descriptor_states_stale = true;
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode {
		self.bind_pipeline(pipeline_handle)
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode {
		self.bind_pipeline(pipeline_handle)
	}

	fn start_region(&mut self, _write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		_write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");
		self.device.begin_debug_region(self.command_buffer, label.as_str());
	}

	fn end_region(&mut self) {
		self.device.end_debug_region(self.command_buffer);
	}

	fn start_counter(&mut self, counter: crate::CounterHandle) {
		let sequence_index = crate::counters::frame_sequence(self.frame_key);
		self.device.record_counter(self.command_buffer, sequence_index, counter, true);
	}

	fn end_counter(&mut self, counter: crate::CounterHandle) {
		let sequence_index = crate::counters::frame_sequence(self.frame_key);
		self.device
			.record_counter(self.command_buffer, sequence_index, counter, false);
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRasterizationPipelineMode {
		self.bind_pipeline(pipeline_handle)
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

	fn set_scissor(&mut self, origin: [u32; 2], extent: Extent) {
		let active = self
			.active_extent
			.expect("No active render pass. The most likely cause is that set_scissor was called outside start_render_pass.");
		let (origin, extent) = crate::clamp_scissor(origin, extent, active);
		self.device.set_scissor_native(self.command_buffer, origin, extent);
	}

	fn end_render_pass(&mut self) {
		self.device.end_render_pass_native(self.command_buffer);
		self.active_attachments.clear();
		self.active_extent = None;
		// The pass left its attachments as render targets, which a set may hold for the next shader command.
		self.descriptor_states_stale = true;
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[DescriptorSetHandle]) -> &mut Self {
		let pipeline = self.pipeline();
		self.validate_descriptor_bindings(pipeline, sets);
		self.bound_descriptor_sets.clear();
		self.bound_descriptor_sets.extend_from_slice(sets);
		self.bind_descriptor_tables(pipeline);
		self
	}

	fn write_push_constant<T: crate::Pod>(&mut self, offset: u32, data: T) {
		let pipeline = self.pipeline();
		self.device
			.write_push_constants_native(self.command_buffer, pipeline, offset, bytemuck::bytes_of(&data));
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, _mesh_handle: &MeshHandle) {
		self.prepare_shader_work();
		self.device.draw_mesh_native(self.command_buffer, *_mesh_handle);
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		self.prepare_shader_work();
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
		self.prepare_shader_work();
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
		self.prepare_shader_work();
		self.device
			.dispatch_meshes_native(self.command_buffer, self.pipeline(), x, y, z);
	}

	fn dispatch_meshes_indirect<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 3]; N]>, entry_index: usize) {
		self.refresh_descriptor_tables_if_dirty();
		self.device.dispatch_meshes_indirect_native::<N>(
			self.command_buffer,
			self.pipeline(),
			buffer.into(),
			entry_index,
			self.sequence_index(),
		);
	}

	fn draw_indirect<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 4]; N]>, entry_index: usize) {
		self.prepare_shader_work();
		self.device
			.draw_indirect_native::<N>(self.command_buffer, buffer.into(), entry_index, self.sequence_index());
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: DispatchExtent) {
		self.prepare_shader_work();
		self.device
			.dispatch_compute_native(self.command_buffer, dispatch);
		self.descriptor_states_stale = true;
	}

	fn indirect_dispatch<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 3]; N]>, entry_index: usize) {
		self.prepare_shader_work();
		self.device.dispatch_compute_indirect_native::<N>(
			self.command_buffer,
			buffer.into(),
			entry_index,
			self.sequence_index(),
		);
		self.descriptor_states_stale = true;
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, table: crate::ShaderBindingTableHandle, extent: Extent) {
		self.prepare_shader_work();
		self.device
			.trace_rays_native(self.command_buffer, self.bound_pipeline, table, extent);
		self.descriptor_states_stale = true;
	}
}

impl crate::command_buffer::BoundTilePipelineMode for CommandBufferRecording<'_> {
	fn dispatch_tiles(&mut self) {
		crate::pipelines::tile::unsupported("DX12")
	}
}
