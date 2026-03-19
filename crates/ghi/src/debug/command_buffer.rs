use crate::{
	AttachmentInformation, BaseBufferHandle, BufferDescriptor, BufferHandle, ClearValue, CommandBufferHandle,
	DescriptorSetHandle, DispatchExtent, FrameKey, ImageHandle, Layouts, MeshHandle, PipelineHandle, PipelineLayoutHandle,
	PresentKey, RGBAu8, SwapchainHandle, SynchronizerHandle, TextureCopyHandle,
};
use utils::Extent;

pub struct CommandBufferRecording<'a> {
	_device: &'a mut super::Device,
	_command_buffer_handle: CommandBufferHandle,
	_frame_key: Option<FrameKey>,
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut super::Device,
		command_buffer_handle: CommandBufferHandle,
		_buffer_copies: Vec<()>,
		_image_copies: Vec<()>,
		frame_key: Option<FrameKey>,
	) -> Self {
		Self {
			_device: device,
			_command_buffer_handle: command_buffer_handle,
			_frame_key: frame_key,
		}
	}

	pub fn sync_buffers(&mut self) {}

	pub fn sync_textures(&mut self) {}

	pub fn build_top_level_acceleration_structure(
		&mut self,
		_acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
	}

	pub fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
	}

	pub fn start_render_pass(&mut self, _extent: Extent, _attachments: &[AttachmentInformation]) -> &mut Self {
		self
	}

	pub fn clear_images<I: crate::graphics_hardware_interface::ImageHandleLike>(&mut self, _textures: &[(I, ClearValue)]) {}

	pub fn clear_buffers(&mut self, _buffer_handles: &[BaseBufferHandle]) {}

	pub fn transfer_textures(
		&mut self,
		_texture_handles: &[impl crate::graphics_hardware_interface::ImageHandleLike],
	) -> Vec<TextureCopyHandle> {
		Vec::new()
	}

	pub fn write_image_data(
		&mut self,
		_image_handle: impl crate::graphics_hardware_interface::ImageHandleLike,
		_data: &[RGBAu8],
	) {
	}

	pub fn blit_image(
		&mut self,
		_source_image: impl crate::graphics_hardware_interface::ImageHandleLike,
		_source_layout: Layouts,
		_destination_image: impl crate::graphics_hardware_interface::ImageHandleLike,
		_destination_layout: Layouts,
	) {
	}

	pub fn copy_to_swapchain(
		&mut self,
		_source_texture_handle: impl crate::graphics_hardware_interface::ImageHandleLike,
		_present_image_index: PresentKey,
		_swapchain_handle: SwapchainHandle,
	) {
	}

	pub fn bind_vertex_buffers(&mut self, _buffer_descriptors: &[BufferDescriptor]) {}

	pub fn bind_index_buffer(&mut self, _buffer_descriptor: &BufferDescriptor) {}

	pub fn present(&mut self, _present_key: PresentKey) {}

	pub fn execute(
		self,
		_wait_for_synchronizer_handles: &[SynchronizerHandle],
		_signal_synchronizer_handles: &[SynchronizerHandle],
		_presentations: &[PresentKey],
		_execution_synchronizer_handle: SynchronizerHandle,
	) {
	}

	pub fn bind_pipeline_layout(&mut self, _pipeline_layout: PipelineLayoutHandle) -> &mut Self {
		self
	}

	pub fn start_region(&self, _name: &str) {}

	pub fn end_region(&self) {}

	pub fn region(&mut self, _name: &str, _f: impl FnOnce(&mut Self)) {}

	pub fn end_render_pass(&mut self) {}

	pub fn bind_raster_pipeline(&mut self, _pipeline_handle: PipelineHandle) -> &mut Self {
		self
	}

	pub fn bind_compute_pipeline(&mut self, _pipeline_handle: PipelineHandle) -> &mut Self {
		self
	}

	pub fn bind_ray_tracing_pipeline(&mut self, _pipeline_handle: PipelineHandle) -> &mut Self {
		self
	}

	pub fn bind_descriptor_sets(&mut self, _sets: &[DescriptorSetHandle]) -> &mut Self {
		self
	}

	pub fn write_push_constant<T: Copy + 'static>(&mut self, _offset: u32, _data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
	}

	pub fn draw_mesh(&mut self, _mesh_handle: &MeshHandle) {}

	pub fn draw_indexed(
		&mut self,
		_index_count: u32,
		_instance_count: u32,
		_first_index: u32,
		_vertex_offset: i32,
		_first_instance: u32,
	) {
	}

	pub fn dispatch_meshes(&mut self, _x: u32, _y: u32, _z: u32) {}

	pub fn dispatch(&mut self, _dispatch: DispatchExtent) {}

	pub fn indirect_dispatch<const N: usize>(&mut self, _buffer: BufferHandle<[(u32, u32, u32); N]>, _entry_index: usize) {}

	pub fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {}
}
