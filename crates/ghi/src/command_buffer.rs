use smallvec::SmallVec;
use utils::Extent;

use crate::{
	rt, AttachmentInformation, BaseBufferHandle, BaseImageHandle, BufferCopyDescriptor, BufferDescriptor, BufferHandle,
	BufferImageCopyDescriptor, ClearValue, DescriptorSetHandle, DispatchExtent, FrameKey, ImageBufferCopyDescriptor, Layouts,
	MeshHandle, PipelineHandle, RGBAu8, SynchronizerHandle, TextureCopyHandle,
};

/// The `DebugLabelWriter` struct exists so command-buffer implementations can provide temporary label storage without forcing callers to allocate strings.
pub struct DebugLabelWriter {
	bytes: SmallVec<[u8; 128]>,
}

impl DebugLabelWriter {
	/// Creates an empty label writer with inline storage for common debug-label sizes.
	pub fn new() -> Self {
		Self { bytes: SmallVec::new() }
	}

	/// Returns the written label as UTF-8 text.
	pub fn as_str(&self) -> &str {
		std::str::from_utf8(&self.bytes).expect("Invalid debug label. The label writer most likely received non UTF-8 bytes.")
	}

	/// Writes text into the label buffer.
	pub fn write_str(&mut self, s: &str) -> std::fmt::Result {
		self.bytes.extend_from_slice(s.as_bytes());
		Ok(())
	}

	/// Appends a null terminator so the label can be passed to C APIs.
	pub fn null_terminate(&mut self) {
		self.bytes.push(0);
	}

	/// Returns the written bytes for backend-specific native API calls.
	pub fn as_bytes(&self) -> &[u8] {
		&self.bytes
	}
}

impl Default for DebugLabelWriter {
	fn default() -> Self {
		Self::new()
	}
}

impl std::fmt::Write for DebugLabelWriter {
	fn write_str(&mut self, s: &str) -> std::fmt::Result {
		self.write_str(s)
	}
}

pub trait CommandBuffer {
	/// Starts recording commands into an existing command buffer.
	fn create_command_buffer_recording(&mut self) -> impl CommandBufferRecording + CommonCommandBufferMode;
}

/// The `CommandBufferRecording` trait captures backend command encoding so GPU work can be recorded before submission.
pub trait CommandBufferRecording
where
	Self: Sized,
{
	/// Returns the frame key that scoped this command-buffer recording.
	fn frame_key(&self) -> FrameKey;

	/// Records a build for one top-level acceleration structure.
	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &rt::TopLevelAccelerationStructureBuild);
	/// Records builds for one or more bottom-level acceleration structures.
	fn build_bottom_level_acceleration_structures(
		&mut self,
		acceleration_structure_builds: &[rt::BottomLevelAccelerationStructureBuild],
	);

	/// Starts a render pass on the GPU.
	/// The render pass uses the supplied render targets for subsequent draw commands.
	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode;

	/// Clears the provided images to their requested values.
	fn clear_images(&mut self, textures: &[(BaseImageHandle, ClearValue)]);
	/// Clears the provided buffers before later GPU work consumes them.
	fn clear_buffers(&mut self, buffer_handles: &[BaseBufferHandle]);

	/// Copies byte ranges between buffers.
	fn copy_buffers(&mut self, copies: &[BufferCopyDescriptor]);
	/// Copies buffer byte ranges into images.
	fn copy_buffer_to_images(&mut self, copies: &[BufferImageCopyDescriptor]);
	/// Copies images into buffer byte ranges.
	fn copy_images_to_buffer(&mut self, copies: &[ImageBufferCopyDescriptor]);
	/// Synchronizes CPU-written buffer data before later commands read it.
	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>);

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
}

/// The `CommonCommandBufferMode` trait exposes commands that stay valid across multiple command-buffer recording states.
pub trait CommonCommandBufferMode {
	/// Binds a compute pipeline so subsequent commands can dispatch compute work.
	fn bind_compute_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundComputePipelineMode;
	/// Binds a ray-tracing pipeline so subsequent commands can trace rays.
	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: PipelineHandle) -> &mut impl BoundRayTracingPipelineMode;

	/// Starts a named GPU debug region.
	fn start_region(&self, write_label: impl FnOnce(&mut DebugLabelWriter) -> std::fmt::Result);

	/// Ends the current GPU debug region.
	fn end_region(&self);

	/// Starts a debug region on the GPU and executes the closure.
	fn region(&mut self, write_label: impl FnOnce(&mut DebugLabelWriter) -> std::fmt::Result, f: impl FnOnce(&mut Self));
}

/// The `RasterizationRenderPassMode` trait provides commands valid inside an active raster render pass.
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

/// The `BoundPipelineLayoutMode` trait provides resource binding for a selected pipeline layout.
pub trait BoundPipelineLayoutMode: CommonCommandBufferMode {
	/// Binds retained descriptor-set groups whose flat shader slots do not overlap.
	fn bind_descriptor_sets(&mut self, sets: &[DescriptorSetHandle]) -> &mut Self;

	/// Writes data to the push-constant register.
	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized;
}

/// The `BoundRasterizationPipelineMode` trait provides draw commands for a bound raster pipeline.
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

/// The `BoundComputePipelineMode` trait provides dispatch commands for a bound compute pipeline.
pub trait BoundComputePipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	/// Dispatches compute workgroups.
	fn dispatch(&mut self, dispatch: DispatchExtent);

	/// Dispatches compute workgroups using parameters stored in a buffer.
	fn indirect_dispatch<const N: usize>(&mut self, buffer: BufferHandle<[[u32; 4]; N]>, entry_index: usize);
}

/// The `BoundRayTracingPipelineMode` trait provides ray dispatch for a bound ray-tracing pipeline.
pub trait BoundRayTracingPipelineMode: BoundPipelineLayoutMode + CommandBufferRecording {
	/// Traces rays using the currently bound ray-tracing pipeline.
	fn trace_rays(&mut self, binding_tables: rt::BindingTables, x: u32, y: u32, z: u32);
}

/// A workload supported by a command buffer.
pub enum CommandBufferType {
	/// Graphics work, including drawing, blitting, and presentation.
	GRAPHICS,
	/// Compute dispatch work.
	COMPUTE,
	/// Transfer work, including buffer and image copies.
	TRANSFER,
}
