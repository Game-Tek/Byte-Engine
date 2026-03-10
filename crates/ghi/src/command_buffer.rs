use utils::Extent;

use crate::{
	rt, AttachmentInformation, BaseBufferHandle, BufferDescriptor, BufferHandle, ClearValue, DescriptorSetHandle,
	DispatchExtent, ImageHandle, Layouts, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey, RGBAu8,
	SwapchainHandle, SynchronizerHandle, TextureCopyHandle,
};

pub trait CommandBufferRecording
where
	Self: Sized,
{
	type Result<'a>;

	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &rt::TopLevelAccelerationStructureBuild);
	fn build_bottom_level_acceleration_structures(
		&mut self,
		acceleration_structure_builds: &[rt::BottomLevelAccelerationStructureBuild],
	);

	/// Starts a render pass on the GPU.
	/// A render pass is a particular configuration of render targets which will be used simultaneously to render certain imagery.
	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode;

	fn clear_images(&mut self, textures: &[(ImageHandle, ClearValue)]);
	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]);

	fn transfer_textures(&mut self, texture_handles: &[ImageHandle]) -> Vec<TextureCopyHandle>;

	/// Copies image data from a CPU accessible buffer to a GPU accessible image.
	fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]);

	fn blit_image(
		&mut self,
		source_image: ImageHandle,
		source_layout: Layouts,
		destination_image: ImageHandle,
		destination_layout: Layouts,
	);

	fn copy_to_swapchain(
		&mut self,
		source_texture_handle: ImageHandle,
		present_key: PresentKey,
		swapchain_handle: SwapchainHandle,
	);

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]);

	fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor);

	fn execute(self, synchronizer: SynchronizerHandle);

	fn end<'a>(self, present_keys: &'a [PresentKey]) -> Self::Result<'a>;
}

pub trait CommonCommandBufferMode {
	fn bind_pipeline_layout(&mut self, pipeline_layout: PipelineLayoutHandle) -> &mut impl BoundPipelineLayoutMode;

	fn start_region(&self, name: &str);

	fn end_region(&self);

	/// Starts a debug region on the GPU and executes the closure.
	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self));
}

pub trait RasterizationRenderPassMode: CommonCommandBufferMode {
	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self);
}

pub trait BoundPipelineLayoutMode: CommonCommandBufferMode {
	/// Binds a raster pipeline to the GPU.
	fn bind_raster_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRasterizationPipelineMode;
	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode;
	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode;

	/// Binds a decriptor set on the GPU.
	fn bind_descriptor_sets(&mut self, sets: &[DescriptorSetHandle]) -> &mut Self;

	/// Write data to the push constant register
	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized;
}

pub trait BoundRasterizationPipelineMode: BoundPipelineLayoutMode + RasterizationRenderPassMode {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &MeshHandle);

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	);

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32);
}

pub trait BoundComputePipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	fn dispatch(&mut self, dispatch: DispatchExtent);

	fn indirect_dispatch<const N: usize>(&mut self, buffer: BufferHandle<[(u32, u32, u32); N]>, entry_index: usize);
}

pub trait BoundRayTracingPipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	fn trace_rays(&mut self, binding_tables: rt::BindingTables, x: u32, y: u32, z: u32);
}

/// Enumerates the types of command buffers that can be created.
pub enum CommandBufferType {
	/// A command buffer that can perform graphics operations. Draws, blits, presentations, etc.
	GRAPHICS,
	/// A command buffer that can perform compute operations. Dispatches, etc.
	COMPUTE,
	/// A command buffer that is optimized for transfer operations. Copies, etc.
	TRANSFER,
}
