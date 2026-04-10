use utils::Extent;

use crate::{
	rt, AttachmentInformation, BaseBufferHandle, BaseImageHandle, BufferDescriptor, BufferHandle, ClearValue,
	DescriptorSetHandle, DispatchExtent, Layouts, MeshHandle, PipelineHandle, PresentKey, RGBAu8, SynchronizerHandle,
	TextureCopyHandle,
};

/// The `CommandBufferRecording` trait captures backend command encoding so GPU work can be recorded before submission.
pub trait CommandBufferRecording
where
	Self: Sized,
{
	/// The backend-specific submission result produced when recording ends.
	type Result<'a>;

	/// Records a build for one top-level acceleration structure.
	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &rt::TopLevelAccelerationStructureBuild);
	/// Records builds for one or more bottom-level acceleration structures.
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

	/// Clears the provided images to their requested values.
	fn clear_images(&mut self, textures: &[(BaseImageHandle, ClearValue)]);
	/// Clears the provided buffers before later GPU work consumes them.
	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]);

	/// Records copies that make image data available to the CPU.
	fn transfer_textures(&mut self, texture_handles: &[BaseImageHandle]) -> Vec<TextureCopyHandle>;

	/// Copies image data from a CPU accessible buffer to a GPU accessible image.
	fn write_image_data(&mut self, image_handle: BaseImageHandle, data: &[RGBAu8]);

	/// Records an image blit between two images and layouts.
	fn blit_image(
		&mut self,
		source_image: BaseImageHandle,
		source_layout: Layouts,
		destination_image: BaseImageHandle,
		destination_layout: Layouts,
	);

	/// Submits the recorded commands for execution.
	fn execute(self, synchronizer: SynchronizerHandle);

	/// Finishes recording and returns the backend-specific submission payload.
	fn end<'a>(self, present_keys: &'a [PresentKey]) -> Self::Result<'a>;
}

/// The `CommonCommandBufferMode` trait exposes commands that stay valid across multiple command-buffer recording states.
pub trait CommonCommandBufferMode {
	/// Binds a compute pipeline so subsequent commands can dispatch compute work.
	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode;
	/// Binds a ray-tracing pipeline so subsequent commands can trace rays.
	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode;

	/// Starts a named GPU debug region.
	fn start_region(&self, name: &str);

	/// Ends the current GPU debug region.
	fn end_region(&self);

	/// Starts a debug region on the GPU and executes the closure.
	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self));
}

/// The `RasterizationRenderPassMode` trait represents the recording state inside an active raster render pass.
pub trait RasterizationRenderPassMode: CommonCommandBufferMode {
	/// Binds a raster pipeline for subsequent draw commands in the render pass.
	fn bind_raster_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRasterizationPipelineMode;

	/// Binds vertex buffers for subsequent draw commands.
	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]);

	/// Binds the index buffer for subsequent indexed draw commands.
	fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor);

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self);
}

/// The `BoundPipelineLayoutMode` trait represents a recording state where pipeline layout resources can be bound.
pub trait BoundPipelineLayoutMode: CommonCommandBufferMode {
	/// Binds a decriptor set on the GPU.
	fn bind_descriptor_sets(&mut self, sets: &[DescriptorSetHandle]) -> &mut Self;

	/// Write data to the push constant register
	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized;
}

/// The `BoundRasterizationPipelineMode` trait represents a render-pass recording state with a raster pipeline bound for draw commands.
pub trait BoundRasterizationPipelineMode: BoundPipelineLayoutMode + RasterizationRenderPassMode {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &MeshHandle);

	/// Records a non-indexed draw call.
	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32);

	/// Records an indexed draw call.
	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	);

	/// Dispatches mesh shading workgroups.
	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32);
}

/// The `BoundComputePipelineMode` trait represents a recording state with a compute pipeline bound for dispatch commands.
pub trait BoundComputePipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	/// Dispatches compute workgroups.
	fn dispatch(&mut self, dispatch: DispatchExtent);

	/// Dispatches compute workgroups using parameters stored in a buffer.
	fn indirect_dispatch<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 4]; N]>, entry_index: usize);
}

/// The `BoundRayTracingPipelineMode` trait represents a recording state with a ray-tracing pipeline bound for ray dispatch.
pub trait BoundRayTracingPipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	/// Traces rays using the currently bound ray-tracing pipeline.
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
