use utils::Extent;

use crate::{
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommandBufferRecordable, CommonCommandBufferMode, RasterizationRenderPassMode,
	},
	AttachmentInformation, BaseBufferHandle, BindingTables, BottomLevelAccelerationStructureBuild, BufferDescriptor,
	BufferHandle, ClearValue, DescriptorSetHandle, DispatchExtent, ImageHandle, Layouts, MeshHandle, PipelineHandle,
	PipelineLayoutHandle, PresentKey, RGBAu8, SwapchainHandle, SynchronizerHandle, TextureCopyHandle,
	TopLevelAccelerationStructureBuild,
};

pub struct CommandBufferRecording<'a> {
	device: &'a mut super::Device,
	command_buffer: crate::CommandBufferHandle,
	bound_pipeline_layout: Option<PipelineLayoutHandle>,
	bound_pipeline: Option<PipelineHandle>,
	present_keys: Vec<PresentKey>,
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut super::Device,
		command_buffer: crate::CommandBufferHandle,
		_frame_key: Option<crate::FrameKey>,
	) -> Self {
		Self {
			device,
			command_buffer,
			bound_pipeline_layout: None,
			bound_pipeline: None,
			present_keys: Vec::new(),
		}
	}
}

impl CommandBufferRecordable for CommandBufferRecording<'_> {
	fn sync_buffers(&mut self) {
		// TODO: DX12 uploads require staging resources and copy command lists.
	}

	fn sync_textures(&mut self) {
		// TODO: DX12 texture uploads require staging resources and copy command lists.
	}

	fn build_top_level_acceleration_structure(&mut self, _acceleration_structure_build: &TopLevelAccelerationStructureBuild) {
		// TODO: DXR acceleration structure builds are not implemented yet.
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[BottomLevelAccelerationStructureBuild],
	) {
		// TODO: DXR acceleration structure builds are not implemented yet.
	}

	fn start_render_pass(
		&mut self,
		_extent: Extent,
		_attachments: &[AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		// TODO: Render pass setup requires render target binding and resource barriers.
		self
	}

	fn clear_images(&mut self, _textures: &[(ImageHandle, ClearValue)]) {
		// TODO: DX12 image clears require command list encoding.
	}

	fn clear_buffers(&mut self, _buffer_handles: &[BaseBufferHandle]) {
		// TODO: DX12 buffer clears require command list encoding.
	}

	fn transfer_textures(&mut self, texture_handles: &[ImageHandle]) -> Vec<TextureCopyHandle> {
		texture_handles
			.iter()
			.map(|handle| self.device.copy_image_to_cpu(*handle))
			.collect()
	}

	fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]) {
		self.device.write_image_data(image_handle, data);
	}

	fn blit_image(
		&mut self,
		_source_image: ImageHandle,
		_source_layout: Layouts,
		_destination_image: ImageHandle,
		_destination_layout: Layouts,
	) {
		// TODO: DX12 blit operations need copy command lists and resource transitions.
	}

	fn copy_to_swapchain(
		&mut self,
		_source_texture_handle: ImageHandle,
		_present_key: PresentKey,
		_swapchain_handle: SwapchainHandle,
	) {
		// TODO: DX12 swapchain copy requires DXGI back buffer access and resource barriers.
	}

	fn bind_vertex_buffers(&mut self, _buffer_descriptors: &[BufferDescriptor]) {
		// TODO: DX12 vertex buffer binding requires command list setup.
	}

	fn bind_index_buffer(&mut self, _buffer_descriptor: &BufferDescriptor) {
		// TODO: DX12 index buffer binding requires command list setup.
	}

	fn present(&mut self, present_key: PresentKey) {
		self.present_keys.push(present_key);
	}

	fn execute(
		self,
		_wait_for_synchronizer_handles: &[SynchronizerHandle],
		_signal_synchronizer_handles: &[SynchronizerHandle],
		_presentations: &[PresentKey],
		_execution_synchronizer_handle: SynchronizerHandle,
	) {
		for presentation in _presentations {
			self.device.present_swapchain(*presentation);
		}
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_pipeline_layout(&mut self, pipeline_layout: PipelineLayoutHandle) -> &mut impl BoundPipelineLayoutMode {
		self.bound_pipeline_layout = Some(pipeline_layout);
		self
	}

	fn start_region(&self, _name: &str) {
		// TODO: Use PIX markers when command list encoding is implemented.
	}

	fn end_region(&self) {
		// TODO: Use PIX markers when command list encoding is implemented.
	}

	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
		self.start_region(name);
		f(self);
		self.end_region();
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn end_render_pass(&mut self) {
		// TODO: End render pass by closing render target bindings.
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self
	}

	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self
	}

	fn bind_descriptor_sets(&mut self, _sets: &[DescriptorSetHandle]) -> &mut Self {
		// TODO: DX12 root signatures and descriptor heaps are not wired yet.
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, _offset: u32, _data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized, {
		// TODO: DX12 uses root constants or inline constant buffers.
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, _mesh_handle: &MeshHandle) {
		// TODO: DX12 draw calls require command list encoding.
	}

	fn draw_indexed(
		&mut self,
		_index_count: u32,
		_instance_count: u32,
		_first_index: u32,
		_vertex_offset: i32,
		_first_instance: u32,
	) {
		// TODO: DX12 draw calls require command list encoding.
	}

	fn dispatch_meshes(&mut self, _x: u32, _y: u32, _z: u32) {
		// TODO: DX12 mesh shading requires pipeline state support and command list encoding.
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, _dispatch: DispatchExtent) {
		// TODO: DX12 dispatch requires command list encoding.
	}

	fn indirect_dispatch<const N: usize>(&mut self, _buffer: BufferHandle<[(u32, u32, u32); N]>, _entry_index: usize) {
		// TODO: DX12 indirect dispatch requires command list encoding and command signature setup.
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: DX12 ray tracing dispatch requires state objects and shader tables.
	}
}
