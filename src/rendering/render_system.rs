//! The [`RenderSystem`] implements easy to use rendering functionality.
//! It provides useful abstractions to interact with the GPU.
//! It's not tied to any particular render pipeline implementation.

use std::collections::HashMap;
use std::hash::Hasher;

use crate::{window_system, orchestrator::{self}, Extent};

/// Returns the best value from a slice of values based on a score function.
fn select_by_score<T>(values: &[T], score: impl Fn(&T) -> u64) -> Option<&T> {
	let mut best_score = 0 as u64;
	let mut best_value: Option<&T> = None;

	for value in values {
		let score = score(value);

		if score > best_score {
			best_score = score;
			best_value = Some(value);
		}
	}

	best_value
}

/// Possible types of a shader source
pub enum ShaderSourceType {
	/// GLSL code string
	GLSL,
	/// SPIR-V binary
	SPIRV,
}

/// Primitive GPU/shader data types.
#[derive(Hash, Clone, Copy)]
pub enum DataTypes {
	Float,
	Float2,
	Float3,
	Float4,
	Int,
	Int2,
	Int3,
	Int4,
	UInt,
	UInt2,
	UInt3,
	UInt4,
}

#[derive(Hash)]
pub struct VertexElement {
	pub name: String,
	pub format: DataTypes,
	pub binding: u32,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	pub struct DeviceAccesses: u16 {
		const CpuRead = 1 << 0;
		const CpuWrite = 1 << 1;
		const GpuRead = 1 << 2;
		const GpuWrite = 1 << 3;
	}
}

// HANDLES

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct BufferHandle(pub(super) u64);

#[derive(Clone, Copy)]
pub struct CommandBufferHandle(pub(super) u64);

#[derive(Clone, Copy)]
pub struct ShaderHandle(pub(super) u64);

#[derive(Clone, Copy)]
pub struct PipelineHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub(super) u64);

pub struct MeshHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SynchronizerHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetLayoutHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetHandle(pub(super) u64);

/// Handle to a Pipeline Layout
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineLayoutHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerHandle(pub(super) u64);

/// Handle to a Sampler
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SwapchainHandle(pub(super) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllocationHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureCopyHandle(pub(crate) u64);

// HANDLES

pub trait CommandBufferRecording {
	/// Enables recording on the command buffer.
	fn begin(&self);

	/// Starts a render pass on the GPU.
	/// A render pass is a particular configuration of render targets which will be used simultaneously to render certain imagery.
	fn start_render_pass(&mut self, extent: Extent, attachments: &[AttachmentInformation]);

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self);

	/// Binds a shader to the GPU.
	fn bind_shader(&self, shader_handle: ShaderHandle);

	/// Binds a pipeline to the GPU.
	fn bind_pipeline(&mut self, pipeline_handle: &PipelineHandle);

	/// Writes to the push constant register.
	fn write_to_push_constant(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, offset: u32, data: &[u8]);

	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &MeshHandle);

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[BufferDescriptor]);

	fn bind_index_buffer(&mut self, buffer_descriptor: &BufferDescriptor);

	fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32);

	/// Copies texture data from a CPU accessible buffer to a GPU accessible texture.
	fn write_texture_data(&mut self, texture_handle: TextureHandle, data: &[RGBAu8]);

	/// Ends recording on the command buffer.
	fn end(&mut self);

	/// Binds a decriptor set on the GPU.
	fn bind_descriptor_set(&self, pipeline_layout: &PipelineLayoutHandle, arg: u32, descriptor_set_handle: &DescriptorSetHandle);

	fn copy_to_swapchain(&mut self, source_texture_handle: TextureHandle, swapchain_handle: SwapchainHandle);

	fn sync_textures(&mut self, texture_handles: &[TextureHandle]) -> Vec<TextureCopyHandle>;

	fn execute(&mut self, wait_for_synchronizer_handles: &[SynchronizerHandle], signal_synchronizer_handles: &[SynchronizerHandle], execution_synchronizer_handle: SynchronizerHandle);
}

pub enum Descriptor {
	Buffer{
		handle: BufferHandle,
		size: usize,
	},
	Texture(TextureHandle),
	Sampler(SamplerHandle),
}

pub enum UseCases {
	STATIC,
	DYNAMIC
}

pub trait RenderSystem: orchestrator::System {
	/// Returns whether the underlying API has encountered any errors. Used during tests to assert whether the validation layers have caught any errors.
	fn has_errors(&self) -> bool;

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: Uses, resource_device_accesses: DeviceAccesses) -> AllocationHandle;

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[VertexElement]) -> MeshHandle;

	/// Creates a shader.
	fn add_shader(&mut self, shader_source_type: ShaderSourceType, stage: ShaderTypes, shader: &[u8]) -> ShaderHandle;

	fn create_descriptor_set_layout(&mut self, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetLayoutHandle;

	fn create_descriptor_set(&mut self, descriptor_set_layout_handle: &DescriptorSetLayoutHandle, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetHandle;

	fn write(&self, descriptor_set_writes: &[DescriptorWrite]);

	fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[DescriptorSetLayoutHandle]) -> PipelineLayoutHandle;

	fn create_pipeline(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, shader_handles: &[(&ShaderHandle, ShaderTypes)], vertex_layout: &[VertexElement], targets: &[AttachmentInformation]) -> PipelineHandle;

	fn create_command_buffer(&mut self) -> CommandBufferHandle;

	fn create_command_buffer_recording(&self, command_buffer_handle: CommandBufferHandle, frame_index: Option<u32>) -> Box<dyn CommandBufferRecording + '_>;

	/// Creates a new buffer.\
	/// If the access includes [`DeviceAccesses::CpuWrite`] and [`DeviceAccesses::GpuRead`] then multiple buffers will be created, one for each frame.\
	/// Staging buffers MAY be created if there's is not sufficient CPU writable, fast GPU readable memory.\
	/// 
	/// # Arguments
	/// 
	/// * `size` - The size of the buffer in bytes.
	/// * `resource_uses` - The uses of the buffer.
	/// * `device_accesses` - The accesses of the buffer.
	/// 
	/// # Returns
	/// 
	/// The handle of the buffer.
	fn create_buffer(&mut self, size: usize, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases) -> BufferHandle;

	fn get_buffer_address(&self, buffer_handle: BufferHandle) -> u64;

	fn get_buffer_slice(&mut self, buffer_handle: BufferHandle) -> &[u8];

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice(&self, buffer_handle: BufferHandle) -> &mut [u8];

	/// Creates a texture.
	fn create_texture(&mut self, extent: crate::Extent, format: TextureFormats, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases) -> TextureHandle;

	fn create_sampler(&mut self) -> SamplerHandle;

	fn bind_to_window(&mut self, window_os_handles: window_system::WindowOsHandles) -> SwapchainHandle;

	fn get_texture_data(&self, texture_copy_handle: TextureCopyHandle) -> &[u8];

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, signaled: bool) -> SynchronizerHandle;

	/// Acquires an image from the swapchain as to have it ready for presentation.
	/// 
	/// # Arguments
	/// 
	/// * `frame_handle` - The frame to acquire the image for. If `None` is passed, the image will be acquired for the next frame.
	/// * `synchronizer_handle` - The synchronizer to wait for before acquiring the image. If `None` is passed, the image will be acquired immediately.
	///
	/// # Panics
	///
	/// Panics if .
	fn acquire_swapchain_image(&self, swapchain_handle: SwapchainHandle, synchronizer_handle: SynchronizerHandle) -> u32;

	fn present(&self, image_index: u32, swapchains: &[SwapchainHandle], synchronizer_handle: SynchronizerHandle);

	fn wait(&self, synchronizer_handle: SynchronizerHandle);

	fn start_frame_capture(&self);

	fn end_frame_capture(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RGBAu8 {
	r: u8,
	g: u8,
	b: u8,
	a: u8,
}

#[cfg(test)]
pub(super) mod tests {
	use super::*;

	fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
		assert_eq!(pixels.len(), (extent.width * extent.height) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });

		if extent.width % 2 != 0 {
			let pixel = pixels[(extent.width / 2) as usize]; // middle top center
			assert_eq!(pixel, RGBAu8 { r: 255, g: 0, b: 0, a: 255 });
		}
		
		let pixel = pixels[(extent.width - 1) as usize]; // top right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 0, a: 255 });
		
		let pixel = pixels[(extent.width  * (extent.height - 1)) as usize]; // bottom left
		assert_eq!(pixel, RGBAu8 { r: 0, g: 0, b: 255, a: 255 });
		
		let pixel = pixels[(extent.width * extent.height - (extent.width / 2)) as usize]; // middle bottom center
		assert!(pixel == RGBAu8 { r: 0, g: 127, b: 127, a: 255 } || pixel == RGBAu8 { r: 0, g: 128, b: 127, a: 255 }); // FIX: workaround for CI, TODO: make near equal function
		
		let pixel = pixels[(extent.width * extent.height - 1) as usize]; // bottom right
		assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
	}

	pub(crate) fn render_triangle(renderer: &mut dyn RenderSystem) {
		log::set_boxed_logger(Box::new(simple_logger::SimpleLogger::new())).unwrap();

		let signal = renderer.create_synchronizer(false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], signal);

		renderer.end_frame_capture();

		renderer.wait(signal); // Wait for the render to finish before accessing the texture data

		assert!(!renderer.has_errors());

		// Get texture data and cast u8 slice to rgbau8
		let pixels = unsafe { std::slice::from_raw_parts(renderer.get_texture_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

		check_triangle(pixels, extent);

		// let mut file = std::fs::File::create("test.png").unwrap();

		// let mut encoder = png::Encoder::new(&mut file, extent.width, extent.height);

		// encoder.set_color(png::ColorType::Rgba);
		// encoder.set_depth(png::BitDepth::Eight);

		// let mut writer = encoder.write_header().unwrap();
		// writer.write_image_data(unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4) }).unwrap();
	}

	pub(crate) fn present(renderer: &mut dyn RenderSystem) {
		let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let window_handle = window_system.create_window("Renderer Test", extent, "test");

		let swapchain = renderer.bind_to_window(window_system.get_os_handles(&window_handle));

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: None,
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(false);
		let image_ready = renderer.create_synchronizer(false);

		let image_index = renderer.acquire_swapchain_image(swapchain, image_ready);

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.copy_to_swapchain(render_target, swapchain);

		command_buffer_recording.execute(&[image_ready], &[render_finished_synchronizer], render_finished_synchronizer);

		renderer.present(image_index, &[swapchain], render_finished_synchronizer);

		renderer.end_frame_capture();

		renderer.wait(render_finished_synchronizer);

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut dyn RenderSystem) {
		let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let window_handle = window_system.create_window("Renderer Test", extent, "test");

		let swapchain = renderer.bind_to_window(window_system.get_os_handles(&window_handle));

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::GpuWrite | DeviceAccesses::CpuRead, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: None,
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(true);
		let image_ready = renderer.create_synchronizer(true);

		for i in 0..2*64 {
			renderer.wait(render_finished_synchronizer);

			let image_index = renderer.acquire_swapchain_image(swapchain, image_ready);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					texture: render_target,
					layout: Layouts::RenderTarget,
					format: TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			command_buffer_recording.start_render_pass(extent, &attachments);

			command_buffer_recording.bind_pipeline(&pipeline);

			command_buffer_recording.draw_mesh(&mesh);

			command_buffer_recording.end_render_pass();

			command_buffer_recording.copy_to_swapchain(render_target, swapchain);

			let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[image_ready], &[render_finished_synchronizer], render_finished_synchronizer);

			renderer.present(image_index, &[swapchain], render_finished_synchronizer);

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());

			// Get texture data and cast u8 slice to rgbau8

			// let pixels = unsafe { std::slice::from_raw_parts(renderer.get_texture_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

			// check_triangle(pixels, extent);
		}
	}

	pub(crate) fn multiframe_rendering(renderer: &mut dyn RenderSystem) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(false);

		for i in 0..FRAMES_IN_FLIGHT*10 {
			// renderer.wait(render_finished_synchronizer);

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					texture: render_target,
					layout: Layouts::RenderTarget,
					format: TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			command_buffer_recording.start_render_pass(extent, &attachments);

			command_buffer_recording.bind_pipeline(&pipeline);

			command_buffer_recording.draw_mesh(&mesh);

			command_buffer_recording.end_render_pass();

			let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			renderer.wait(render_finished_synchronizer);

			assert!(!renderer.has_errors());

			// Get texture data and cast u8 slice to rgbau8

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_texture_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

			check_triangle(pixels, extent);
		}
	}

	// TODO: Test changing frames in flight count during rendering

	pub(crate) fn dynamic_data(renderer: &mut dyn RenderSystem) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(row_major) uniform;

			layout(push_constant) uniform PushConstants {
				mat4 matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.matrix * vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let pipeline_layout = renderer.create_pipeline_layout(&[]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::DYNAMIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let _buffer = renderer.create_buffer(64, Uses::Storage, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::DYNAMIC);

		let command_buffer_handle = renderer.create_command_buffer();

		let render_finished_synchronizer = renderer.create_synchronizer(false);

		for i in 0..FRAMES_IN_FLIGHT*10 {
			// renderer.wait(render_finished_synchronizer);

			//let pointer = renderer.get_buffer_pointer(Some(frames[i % FRAMES_IN_FLIGHT]), buffer);

			//unsafe { std::ptr::copy_nonoverlapping(matrix.as_ptr(), pointer as *mut f32, 16); }

			renderer.start_frame_capture();

			let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, Some(i as u32));

			let attachments = [
				AttachmentInformation {
					texture: render_target,
					layout: Layouts::RenderTarget,
					format: TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				}
			];

			command_buffer_recording.start_render_pass(extent, &attachments);

			command_buffer_recording.bind_pipeline(&pipeline);
			
			let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);

			let matrix: [f32; 16] = 
				[
					angle.cos(), -angle.sin(), 0f32, 0f32,
					angle.sin(), angle.cos(), 0f32, 0f32,
					0f32, 0f32, 1f32, 0f32,
					0f32, 0f32, 0f32, 1f32,
				];

			command_buffer_recording.write_to_push_constant(&pipeline_layout, 0, unsafe { std::slice::from_raw_parts(matrix.as_ptr() as *const u8, 16 * 4) });

			command_buffer_recording.draw_mesh(&mesh);

			command_buffer_recording.end_render_pass();

			let copy_texture_handles = command_buffer_recording.sync_textures(&[render_target]);

			command_buffer_recording.execute(&[], &[], render_finished_synchronizer);

			renderer.end_frame_capture();

			renderer.wait(render_finished_synchronizer);

			assert!(!renderer.has_errors());

			// Get texture data and cast u8 slice to rgbau8

			let pixels = unsafe { std::slice::from_raw_parts(renderer.get_texture_data(copy_texture_handles[0]).as_ptr() as *const RGBAu8, (extent.width * extent.height) as usize) };

			assert_eq!(pixels.len(), (extent.width * extent.height) as usize);
			
			// Track green corner as it should move through screen

			if i % 4 == 0 {
				let pixel = pixels[(extent.width * extent.height - 1) as usize]; // bottom right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 1 {
				let pixel = pixels[(extent.width - 1) as usize]; // top right
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 2 {
				let pixel = pixels[0]; // top left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			} else if i % 4 == 3 {
				let pixel = pixels[(extent.width  * (extent.height - 1)) as usize]; // bottom left
				assert_eq!(pixel, RGBAu8 { r: 0, g: 255, b: 0, a: 255 });
			}
		}

		assert!(!renderer.has_errors())
	}

	pub(crate) fn descriptor_sets(renderer: &mut dyn RenderSystem) {
		let signal = renderer.create_synchronizer(false);

		let floats: [f32;21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
			1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 
			-1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0
		];

		let vertex_layout = [
			VertexElement{ name: "POSITION".to_string(), format: DataTypes::Float3, binding: 0 },
			VertexElement{ name: "COLOR".to_string(), format: DataTypes::Float4, binding: 0 },
		];

		let mesh = unsafe { renderer.add_mesh_from_vertices_and_indices(3, 3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3*4 + 4*4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout
			) };

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0, binding=1) uniform UniformBufferObject {
				mat4 matrix;
			} ubo;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0,binding=0) uniform sampler smpl;
			layout(set=0,binding=2) uniform texture2D tex;

			void main() {
				out_color = texture(sampler2D(tex, smpl), vec2(0, 0));
			}
		";

		let vertex_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Vertex, vertex_shader_code.as_bytes());
		let fragment_shader = renderer.add_shader(ShaderSourceType::GLSL, ShaderTypes::Fragment, fragment_shader_code.as_bytes());

		let buffer = renderer.create_buffer(64, Uses::Uniform | Uses::Storage, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::DYNAMIC);

		let sampled_texture = renderer.create_texture(crate::Extent { width: 2, height: 2, depth: 1 }, TextureFormats::RGBAu8, Uses::Texture, DeviceAccesses::CpuWrite | DeviceAccesses::GpuRead, UseCases::STATIC);

		let pixels = vec![
			RGBAu8 { r: 255, g: 0, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 255, b: 0, a: 255 },
			RGBAu8 { r: 0, g: 0, b: 255, a: 255 },
			RGBAu8 { r: 255, g: 255, b: 0, a: 255 },
		];

		let sampler = renderer.create_sampler();

		let bindings = [
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: DescriptorType::Sampler,
				binding: 0,
				stage_flags: Stages::FRAGMENT,
				immutable_samplers: Some(vec![sampler]),
			},
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: DescriptorType::StorageBuffer,
				binding: 1,
				stage_flags: Stages::VERTEX,
				immutable_samplers: None,
			},
			DescriptorSetLayoutBinding {
				descriptor_count: 1,
				descriptor_type: DescriptorType::SampledImage,
				binding: 2,
				stage_flags: Stages::FRAGMENT,
				immutable_samplers: None,
			},
		];

		let descriptor_set_layout_handle = renderer.create_descriptor_set_layout(&bindings);

		let descriptor_set = renderer.create_descriptor_set(&descriptor_set_layout_handle, &bindings);

		renderer.write(&[
			DescriptorWrite { descriptor_set: descriptor_set, binding: 0, array_element: 0, descriptor: Descriptor::Sampler(sampler) },
			DescriptorWrite { descriptor_set: descriptor_set, binding: 1, array_element: 0, descriptor: Descriptor::Buffer{ handle: buffer, size: 64 } },
			DescriptorWrite { descriptor_set: descriptor_set, binding: 2, array_element: 0, descriptor: Descriptor::Texture(sampled_texture) },
		]);

		assert!(!renderer.has_errors());

		let pipeline_layout = renderer.create_pipeline_layout(&[descriptor_set_layout_handle]);

		// Use and odd width to make sure there is a middle/center pixel
		let extent = crate::Extent { width: 1920, height: 1080, depth: 1 };

		let render_target = renderer.create_texture(extent, TextureFormats::RGBAu8, Uses::RenderTarget, DeviceAccesses::CpuRead | DeviceAccesses::GpuWrite, UseCases::STATIC);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		let pipeline = renderer.create_pipeline(&pipeline_layout, &[(&vertex_shader, ShaderTypes::Vertex), (&fragment_shader, ShaderTypes::Fragment)], &vertex_layout, &attachments);

		let command_buffer_handle = renderer.create_command_buffer();

		renderer.start_frame_capture();

		let mut command_buffer_recording = renderer.create_command_buffer_recording(command_buffer_handle, None);

		command_buffer_recording.write_texture_data(sampled_texture, &pixels);

		// command_buffer_recording.transition_textures(&[(sampled_texture, true, Layouts::Texture, Stages::SHADER_READ, AccessPolicies::READ)]);

		let attachments = [
			AttachmentInformation {
				texture: render_target,
				layout: Layouts::RenderTarget,
				format: TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(extent, &attachments);

		command_buffer_recording.bind_pipeline(&pipeline);

		command_buffer_recording.bind_descriptor_set(&pipeline_layout, 0, &descriptor_set);

		command_buffer_recording.draw_mesh(&mesh);

		command_buffer_recording.end_render_pass();

		let texure_copy_handles = command_buffer_recording.sync_textures(&[render_target]);

		command_buffer_recording.execute(&[], &[], signal);

		renderer.end_frame_capture();

		renderer.wait(signal); // Wait for the render to finish before accessing the texture data

		// assert colored triangle was drawn to texture
		let _pixels = renderer.get_texture_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!renderer.has_errors());
	}
}

/// Enumerates the types of command buffers that can be created.
pub enum CommandBufferType {
	/// A command buffer that can perform graphics operations. Draws, blits, presentations, etc.
	GRAPHICS,
	/// A command buffer that can perform compute operations. Dispatches, etc.
	COMPUTE,
	/// A command buffer that is optimized for transfer operations. Copies, etc.
	TRANSFER
}

/// Enumerates the types of buffers that can be created.
pub enum BufferType {
	/// A buffer that can be used as a vertex buffer.
	VERTEX,
	/// A buffer that can be used as an index buffer.
	INDEX,
	/// A buffer that can be used as a uniform buffer.
	UNIFORM,
	/// A buffer that can be used as a storage buffer.
	STORAGE,
	/// A buffer that can be used as an indirect buffer.
	INDIRECT
}

/// Enumerates the types of shaders that can be created.
#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub enum ShaderTypes {
	/// A vertex shader.
	Vertex,
	/// A fragment shader.
	Fragment,
	/// A compute shader.
	Compute
}

#[derive(PartialEq, Eq, Clone, Copy)]
/// Enumerates the formats that textures can have.
pub enum TextureFormats {
	/// 8 bit unsigned per component normalized RGBA.
	RGBAu8,
	/// 16 bit unsigned per component normalized RGBA.
	RGBAu16,
	/// 32 bit unsigned per component normalized RGBA.
	RGBAu32,
	/// 16 bit float per component RGBA.
	RGBAf16,
	/// 32 bit float per component RGBA.
	RGBAf32,
	/// 10 bit unsigned for R, G and 11 bit unsigned for B normalized RGB.
	RGBu10u10u11,
	/// 8 bit unsigned per component normalized BGRA.
	BGRAu8,
	/// 32 bit float depth.
	Depth32,
}

#[derive(Clone, Copy)]
/// Stores the information of a memory region.
pub struct Memory<'a> {
	/// The allocation that the memory region is associated with.
	allocation: &'a AllocationHandle,
	/// The offset of the memory region.
	offset: usize,
	/// The size of the memory region.
	size: usize,
}

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentInformation {
	/// The texture view of the attachment.
	pub texture: TextureHandle,
	/// The format of the attachment.
	pub format: TextureFormats,
	/// The layout of the attachment.
	pub layout: Layouts,
	/// The clear color of the attachment.
	pub clear: Option<crate::RGBA>,
	/// Whether to load the contents of the attchment when starting a render pass.
	pub load: bool,
	/// Whether to store the contents of the attachment when ending a render pass.
	pub store: bool,
}

#[derive(Clone, Copy)]
/// Stores the information of a texture copy.
pub struct TextureCopy {
	/// The source texture.
	pub(super) source: TextureHandle,
	pub(super) source_format: TextureFormats,
	/// The destination texture.
	pub(super) destination: TextureHandle,
	pub(super) destination_format: TextureFormats,
	/// The images extent.
	pub(super) extent: crate::Extent,
}

#[derive(Clone, Copy)]
/// Stores the information of a buffer copy.
pub struct BufferCopy {
	/// The source buffer.
	pub(super)	source: BufferHandle,
	/// The destination buffer.
	pub(super)	destination: BufferHandle,
	/// The size of the copy.
	pub(super) size: usize,
}

use serde::{Serialize, Deserialize};

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq)]
	/// Bit flags for the available access policies.
	pub struct AccessPolicies : u8 {
		/// Will perform read access.
		const READ = 0b00000001;
		/// Will perform write access.
		const WRITE = 0b00000010;
	}
}

#[derive(Clone, Copy)]
pub struct TextureState {
	/// The layout of the resource.
	pub(super) layout: Layouts,
	/// The format of the resource.
	pub(super) format: TextureFormats,
}

#[derive(Clone, Copy)]
/// Stores the information of a barrier.
pub enum Barrier {
	/// A texture barrier.
	Texture {
		source: Option<TextureState>,
		destination: TextureState,
		/// The texture of the barrier.
		texture: TextureHandle,
	},
	/// A buffer barrier.
	Buffer(BufferHandle),
	/// A memory barrier.
	Memory(),
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq)]
	/// Bit flags for the available pipeline stages.
	pub struct Stages : u64 {
		/// No stage.
		const NONE = 0b00000000;
		/// The vertex stage.
		const VERTEX = 0b00000001;
		/// The fragment stage.
		const FRAGMENT = 0b00000010;
		/// The compute stage.
		const COMPUTE = 0b00000100;
		/// The transfer stage.
		const TRANSFER = 0b00001000;
		/// The acceleration structure stage.
		const ACCELERATION_STRUCTURE = 0b00010000;
		/// The presentation stage.
		const PRESENTATION = 0b00100000;
		/// The host stage.
		const HOST = 0b01000000;
		/// The all graphics stage.
		const ALL_GRAPHICS = 0b10000000;
		/// The shader read stage.
		const SHADER_READ = 0b100000000;
		/// The all stage.
		const ALL = 0b11111111;
	}
}

#[derive(Clone, Copy)]
/// Stores the information of a transition state.
pub struct TransitionState {
	/// The stages this transition will either wait or block on.
	pub(super) stage: Stages,
	/// The type of access that will be done on the resource by the process the operation that requires this transition.
	pub(super) access: AccessPolicies,
}

/// Stores the information of a barrier descriptor.
pub struct BarrierDescriptor {
	/// The barrier.
	pub(super) barrier: Barrier,
	/// The state of the resource previous to the barrier. If None, the resource state will be discarded.
	pub(super) source: Option<TransitionState>,
	/// The state of the resource after the barrier.
	pub(super) destination: TransitionState,
}

bitflags::bitflags! {
	#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
	/// Bit flags for the available resource uses.
	pub struct Uses : u32 {
		/// Resource will be used as a vertex buffer.
		const Vertex = 1 << 0;
		/// Resource will be used as an index buffer.
		const Index = 1 << 1;
		/// Resource will be used as a uniform buffer.
		const Uniform = 1 << 2;
		/// Resource will be used as a storage buffer.
		const Storage = 1 << 3;
		/// Resource will be used as an indirect buffer.
		const Indirect = 1 << 4;
		/// Resource will be used as a texture.
		const Texture = 1 << 5;
		/// Resource will be used as a render target.
		const RenderTarget = 1 << 6;
		/// Resource will be used as a depth stencil.
		const DepthStencil = 1 << 7;
		/// Resource will be used as an acceleration structure.
		const AccelerationStructure = 1 << 8;
		/// Resource will be used as a transfer source.
		const TransferSource = 1 << 9;
		/// Resource will be used as a transfer destination.
		const TransferDestination = 1 << 10;
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enumerates the available layouts.
pub enum Layouts {
	/// The layout is undefined. We don't mind what the layout is.
	Undefined,
	/// The texture will be used as render target.
	RenderTarget,
	/// The texture will be used in a transfer operation.
	Transfer,
	/// The texture will be used as a presentation source.
	Present,
	/// The texture will be used as a read only sample source.
	Texture,
}

#[derive(Clone, Copy)]
/// Enumerates the available descriptor types.
pub enum DescriptorType {
	/// A uniform buffer.
	UniformBuffer,
	/// A storage buffer.
	StorageBuffer,
	/// A combined image sampler.
	SampledImage,
	/// A storage image.
	StorageImage,
	/// A sampler.
	Sampler
}

/// Stores the information of a descriptor set layout binding.
pub struct DescriptorSetLayoutBinding {
	/// The binding of the descriptor set layout binding.
	pub binding: u32,
	/// The descriptor type of the descriptor set layout binding.
	pub descriptor_type: DescriptorType,
	/// The number of descriptors in the descriptor set layout binding.
	pub descriptor_count: u32,
	/// The stages the descriptor set layout binding will be used in.
	pub stage_flags: Stages,
	/// The immutable samplers of the descriptor set layout binding.
	pub immutable_samplers: Option<Vec<SamplerHandle>>,
}

/// Stores the information of a descriptor.
pub enum DescriptorInfo {
	/// A buffer descriptor.
	Buffer {
		/// The buffer of the descriptor.
		buffer: BufferHandle,
		/// The offset to start reading from inside the buffer.
		offset: usize,
		/// How much to read from the buffer after `offset`.
		range: usize,
	},
	/// A texture descriptor.
	Texture {
		/// The texture of the descriptor.
		texture: TextureHandle,
		/// The format of the texture.
		format: TextureFormats,
		/// The layout of the texture.
		layout: Layouts,
	},
	/// A sampler descriptor.
	Sampler {
		/// The sampler of the descriptor.
		sampler: u32,
	}
}

/// Stores the information of a descriptor set write.
pub struct DescriptorWrite {
	/// The descriptor set to write to.
	pub descriptor_set: DescriptorSetHandle,
	/// The binding to write to.
	pub 	binding: u32,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub array_element: u32,
	/// Information describing the descriptor.
	pub descriptor: Descriptor,
}

/// Describes the details of the memory layout of a particular texture.
pub struct ImageSubresourceLayout {
	/// The offset inside a memory region where the texture will read it's first texel from.
	pub(super) offset: u64,
	/// The size of the texture in bytes.
	pub(super) size: u64,
	/// The row pitch of the texture.
	pub(super) row_pitch: u64,
	/// The array pitch of the texture.
	pub(super) array_pitch: u64,
	/// The depth pitch of the texture.
	pub(super) depth_pitch: u64,
}

/// Describes the properties of a particular surface.
pub struct SurfaceProperties {
	/// The current extent of the surface.
	pub(super) extent: crate::Extent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// Enumerates the states of a swapchain's validity for presentation.
pub enum SwapchainStates {
	/// The swapchain is valid for presentation.
	Ok,
	/// The swapchain is suboptimal for presentation.
	Suboptimal,
	/// The swapchain can't be used for presentation.
	Invalid,
}

pub struct BufferDescriptor {
	pub buffer: BufferHandle,
	pub offset: u64,
	pub range: u64,
	pub slot: u32,
}

pub enum PipelineConfigurationBlocks<'a> {
	VertexInput {
		vertex_elements: &'a [VertexElement]
	},
	InputAssembly {
	
	},
	RenderTargets {
		targets: &'a [AttachmentInformation],
	},
	Shaders {
		shaders: &'a [(&'a ShaderHandle, ShaderTypes)],
	},
	Layout {
		layout: &'a PipelineLayoutHandle,
	}
}

pub struct RenderSystemImplementation {
	pointer: Box<dyn RenderSystem>,
}

impl RenderSystemImplementation {
	pub fn new(pointer: Box<dyn RenderSystem>) -> Self {
		Self {
			pointer: pointer,
		}
	}
}

impl orchestrator::Entity for RenderSystemImplementation {}
impl orchestrator::System for RenderSystemImplementation {}

impl RenderSystem for RenderSystemImplementation {
	fn has_errors(&self) -> bool {
		self.pointer.has_errors()
	}

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[VertexElement]) -> MeshHandle {
		self.pointer.add_mesh_from_vertices_and_indices(vertex_count, index_count, vertices, indices, vertex_layout)
	}

	fn add_shader(&mut self, shader_source_type: ShaderSourceType, stage: ShaderTypes, shader: &[u8]) -> ShaderHandle {
		self.pointer.add_shader(shader_source_type, stage, shader)
	}

	fn get_buffer_address(&self, buffer_handle: BufferHandle) -> u64 {
		self.pointer.get_buffer_address(buffer_handle)
	}

	fn write(&self, descriptor_set_writes: &[DescriptorWrite]) {
		self.pointer.write(descriptor_set_writes)
	}

	fn get_buffer_slice(&mut self, buffer_handle: BufferHandle) -> &[u8] {
		self.pointer.get_buffer_slice(buffer_handle)
	}

	fn get_mut_buffer_slice(&self, buffer_handle: BufferHandle) -> &mut [u8] {
		self.pointer.get_mut_buffer_slice(buffer_handle)
	}

	fn get_texture_data(&self, texture_copy_handle: TextureCopyHandle) -> &[u8] {
		self.pointer.get_texture_data(texture_copy_handle)
	}

	fn bind_to_window(&mut self, window_os_handles: window_system::WindowOsHandles) -> SwapchainHandle {
		self.pointer.bind_to_window(window_os_handles)
	}

	fn present(&self, image_index: u32, swapchains: &[SwapchainHandle], synchronizer_handle: SynchronizerHandle) {
		self.pointer.present(image_index, swapchains, synchronizer_handle)
	}

	fn wait(&self, synchronizer_handle: SynchronizerHandle) {
		self.pointer.wait(synchronizer_handle)
	}

	fn start_frame_capture(&self) {
		self.pointer.start_frame_capture()
	}

	fn end_frame_capture(&self) {
		self.pointer.end_frame_capture()
	}

	fn acquire_swapchain_image(&self, swapchain_handle: SwapchainHandle, synchronizer_handle: SynchronizerHandle) -> u32 {
		self.pointer.acquire_swapchain_image(swapchain_handle, synchronizer_handle)
	}

	fn create_buffer(&mut self, size: usize, uses: Uses, accesses: DeviceAccesses, use_case: UseCases) -> BufferHandle {
		self.pointer.create_buffer(size, uses, accesses, use_case)
	}

	fn create_allocation(&mut self, size: usize, _resource_uses: Uses, resource_device_accesses: DeviceAccesses) -> AllocationHandle {
		self.pointer.create_allocation(size, _resource_uses, resource_device_accesses)
	}

	fn create_command_buffer(&mut self) -> CommandBufferHandle {
		self.pointer.create_command_buffer()
	}

	fn create_command_buffer_recording<'a>(&'a self, command_buffer_handle: CommandBufferHandle, frame: Option<u32>) -> Box<dyn CommandBufferRecording + 'a> {
		self.pointer.create_command_buffer_recording(command_buffer_handle, frame)
	}

	fn create_descriptor_set(&mut self, descriptor_set_layout: &DescriptorSetLayoutHandle, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetHandle {
		self.pointer.create_descriptor_set(descriptor_set_layout, bindings)
	}

	fn create_descriptor_set_layout(&mut self, bindings: &[DescriptorSetLayoutBinding]) -> DescriptorSetLayoutHandle {
		self.pointer.create_descriptor_set_layout(bindings)
	}

	fn create_pipeline(&mut self, pipeline_layout_handle: &PipelineLayoutHandle, shader_handles: &[(&ShaderHandle, ShaderTypes)], vertex_layout: &[VertexElement], targets: &[AttachmentInformation]) -> PipelineHandle {
		self.pointer.create_pipeline(pipeline_layout_handle, shader_handles, vertex_layout, targets)
	}

	fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[DescriptorSetLayoutHandle]) -> PipelineLayoutHandle {
		self.pointer.create_pipeline_layout(descriptor_set_layout_handles)
	}

	fn create_sampler(&mut self) -> SamplerHandle {
		self.pointer.create_sampler()
	}

	fn create_synchronizer(&mut self, signaled: bool) -> SynchronizerHandle {
		self.pointer.create_synchronizer(signaled)
	}

	fn create_texture(&mut self, extent: crate::Extent, format: TextureFormats, resource_uses: Uses, device_accesses: DeviceAccesses, use_case: UseCases) -> TextureHandle {
		self.pointer.create_texture(extent, format, resource_uses, device_accesses, use_case)
	}
}