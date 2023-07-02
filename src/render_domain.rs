use maths_rs::prelude::MatProjection;

use crate::{resource_manager, render_system::{RenderSystem, self}, render_backend, Extent, orchestrator::System, Vector3};

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	vertices_buffer: render_system::BufferHandle,
	indices_buffer: render_system::BufferHandle,
	render_target: render_system::TextureHandle,
	index_count: u32,
	instance_count: u32,
	frames: Vec<render_system::FrameHandle>,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,
	command_buffer: render_system::CommandBufferHandle,
	camera_data_buffer_handle: render_system::BufferHandle,

	shaders_by_name: std::collections::HashMap<String, render_system::ShaderHandle>,
}

impl VisibilityWorldRenderDomain {
	pub fn new(render_system: &mut RenderSystem) -> Self {
		let frames = (0..2).map(|_| render_system.create_frame()).collect::<Vec<_>>();

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)
			#extension GL_EXT_shader_16bit_storage : enable
			#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable
			#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable
			#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable
			#extension GL_EXT_nonuniform_qualifier : enable
			#extension GL_EXT_scalar_block_layout : enable
			#extension GL_EXT_buffer_reference : enable
			#extension GL_EXT_buffer_reference2 : enable
			#extension GL_EXT_shader_image_load_formatted : enable

			layout(row_major) uniform; layout(row_major) buffer;

			layout(buffer_reference,scalar,buffer_reference_align=2) buffer Camera {
				mat4 view;
				mat4 projection;
			};

			layout(push_constant) uniform push_constants {
				Camera camera;
			} pc;

			layout(location = 0) in vec3 in_position;

			//layout(max_vertices=128, max_primitives=64) out;
			void main() {
				gl_Position = pc.camera.projection * pc.camera.view * vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = vec4(1.0, 0.0, 0.0, 1.0);
			}
		";

		let vertex_layout = [
			crate::render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
		];

		let vertex_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let mut shaders_by_name = std::collections::HashMap::new();

		let pipeline_layout_handle = render_system.create_pipeline_layout(&[]);
		let pipeline = render_system.create_pipeline(pipeline_layout_handle, &[&vertex_shader, &fragment_shader], &vertex_layout);

		shaders_by_name.insert("vertex".to_string(), vertex_shader);
		shaders_by_name.insert("fragment".to_string(), fragment_shader);
		
		let vertices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Vertex, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);
		let indices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Index, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		let vertices_buffer = render_system.get_mut_buffer_slice(None, vertices_buffer_handle);
		let indices_buffer = render_system.get_mut_buffer_slice(None, indices_buffer_handle);

		let floats: [f32;9] = [
			0.0, 1.0, 0.0,
			1.0, -1.0, 0.0,
			-1.0, -1.0, 0.0,
		];

		let indices: [u32;3] = [
			0, 1, 2,
		];

		let floats_bytes = unsafe { std::slice::from_raw_parts(floats.as_ptr() as *const u8, std::mem::size_of_val(&floats)) };
		let indices_bytes = unsafe { std::slice::from_raw_parts(indices.as_ptr() as *const u8, std::mem::size_of_val(&indices)) };

		unsafe {
			std::ptr::copy_nonoverlapping(floats_bytes.as_ptr(), vertices_buffer.as_mut_ptr(), floats_bytes.len());
			std::ptr::copy_nonoverlapping(indices_bytes.as_ptr(), indices_buffer.as_mut_ptr(), indices_bytes.len());
		}

		let render_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_backend::TextureFormats::RGBAu8, render_backend::Uses::RenderTarget, render_system::DeviceAccesses::GpuRead);

		let render_finished_synchronizer = render_system.create_synchronizer(true);
		let image_ready = render_system.create_synchronizer(false);

		let command_buffer = render_system.create_command_buffer();

		let camera_data_buffer_handle = render_system.create_buffer(16 * 4 * 4, render_backend::Uses::Uniform, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		Self {
			pipeline_layout_handle,
			pipeline,
			vertices_buffer: vertices_buffer_handle,
			indices_buffer: indices_buffer_handle,
			render_target,
			index_count: 3,
			instance_count: 1,
			frames,
			render_finished_synchronizer,
			image_ready,
			command_buffer,
			camera_data_buffer_handle,

			shaders_by_name,
		}
	}

	pub fn update_shader(&mut self, render_system: &mut RenderSystem, shader_name: &str, shader_source: &str) {
		println!("Updating shader: {}", shader_name);

		let shader_source = shader_source.as_bytes();

		let new_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, shader_source);

		let vertex_layout = [
			crate::render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: true },
		];

		let vertex_shader_handle = self.shaders_by_name.get("vertex").unwrap().clone();

		let pipeline = render_system.create_pipeline(self.pipeline_layout_handle, &[vertex_shader_handle, &new_shader], &vertex_layout);

		self.pipeline = pipeline;

		self.shaders_by_name.insert("fragment".to_string(), new_shader);
	}

	fn add_mesh(&self, resource_manager: &mut resource_manager::ResourceManager, render_system: &mut RenderSystem, resource_id: &str) {
		let resource_request = resource_manager.get_resource_info(resource_id);

		let resource_request = if let Some(resource_info) = resource_request {
			resource_info
		} else {
			return;
		};

		let resource_info = if let resource_manager::ResourceContainer::Mesh(s) = &resource_request.resource { s } else { return; };

		let vertex_components = resource_info.vertex_components.iter().map(|component| {
			let (semantic, format) = match component.semantic {
				resource_manager::VertexSemantics::Position => ("POSITION", render_system::DataTypes::Float3),
				resource_manager::VertexSemantics::Color => ("COLOR", render_system::DataTypes::Float4),
				resource_manager::VertexSemantics::Normal => ("NORMAL", render_system::DataTypes::Float3),
				resource_manager::VertexSemantics::Uv => ("TEXCOORD", render_system::DataTypes::Float2),
				resource_manager::VertexSemantics::Tangent => ("TANGENT", render_system::DataTypes::Float3),
				resource_manager::VertexSemantics::BiTangent => ("BITANGENT", render_system::DataTypes::Float3),
			};

			render_system::VertexElement{
				format,
				name: semantic.to_string(),
				shuffled: false,
			}
		}).collect::<Vec<_>>();

		let buffer = render_system.get_mut_buffer_slice(None, self.vertices_buffer);

		let resource = resource_manager.load_resource_into_buffer(resource_request, buffer);
	}

	pub fn render(&self, render_system: &mut RenderSystem, frame_index: u32) {
		let frame_handle_option = Some(self.frames[(frame_index % 2) as usize]);

		render_system.wait(frame_handle_option, self.render_finished_synchronizer);

		render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(None, self.camera_data_buffer_handle);

		let camera_position = Vector3::new(0f32, 0f32, -2f32);

		let mut view_matrix = maths_rs::Mat4f::identity();
		view_matrix.set_column(3, maths_rs::Vec4f::new(-camera_position.x, -camera_position.y, camera_position.z, 1f32));

		let projection_matrix = maths_rs::Mat4f::create_perspective_projection_lh_yup(0.785398, 16f32 / 9f32, 0.1f32, 100f32);

		let camera_data = [
			view_matrix,
			projection_matrix,
		];

		let camera_data_bytes = unsafe { std::slice::from_raw_parts(camera_data.as_ptr() as *const u8, std::mem::size_of_val(&camera_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(camera_data_bytes.as_ptr(), camera_data_buffer.as_mut_ptr(), camera_data_bytes.len());
		}

		let image_index = render_system.acquire_swapchain_image(frame_handle_option, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(frame_handle_option, self.command_buffer);

		let attachments = [
			crate::render_system::AttachmentInfo {
				texture: self.render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			}
		];

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		command_buffer_recording.bind_pipeline(&self.pipeline);

		let vertex_buffer_descriptors = [
			render_system::BufferDescriptor {
				buffer: self.vertices_buffer,
				offset: 0,
				range: 3 * 4 * 3,
				slot: 0,
			}
		];

		command_buffer_recording.bind_vertex_buffers(&vertex_buffer_descriptors);

		let index_buffer_index_descriptor = render_system::BufferDescriptor {
			buffer: self.indices_buffer,
			offset: 0,
			range: (self.index_count * std::mem::size_of::<u32>() as u32) as u64,
			slot: 0,
		};

		command_buffer_recording.bind_index_buffer(&index_buffer_index_descriptor);

		let camera_data_buffer_address = render_system.get_buffer_address(None, self.camera_data_buffer_handle);

		let data = [
			camera_data_buffer_address,
		];

		command_buffer_recording.write_to_push_constant(&self.pipeline_layout_handle, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });

		command_buffer_recording.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);

		command_buffer_recording.end_render_pass();

		let swapchain_texture_handle = render_system.get_swapchain_texture_handle(frame_handle_option);

		command_buffer_recording.copy_textures(&[
			(
				self.render_target, render_backend::Layouts::Transfer, render_backend::Stages::TRANSFER, render_backend::AccessPolicies::READ,
				swapchain_texture_handle, render_backend::Layouts::Transfer, render_backend::Stages::TRANSFER, render_backend::AccessPolicies::WRITE
			)
		]);

		command_buffer_recording.end();

		render_system.execute(frame_handle_option, command_buffer_recording, Some(self.image_ready), Some(self.render_finished_synchronizer), self.render_finished_synchronizer);

		render_system.end_frame_capture();

		render_system.present(frame_handle_option, image_index, self.render_finished_synchronizer);
	}
}

impl System for VisibilityWorldRenderDomain {}