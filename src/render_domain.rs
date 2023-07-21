use std::collections::HashMap;

use maths_rs::{prelude::{MatProjection, MatRotate3D}, Mat4f};

use crate::{resource_manager, render_system::{RenderSystem, self}, render_backend, Extent, orchestrator::{Entity, System, self, OrchestratorReference, OwnedComponent}, Vector3, camera::{self, Camera}, math};

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	vertices_buffer: render_system::BufferHandle,
	indices_buffer: render_system::BufferHandle,
	render_target: render_system::TextureHandle,
	depth_target: render_system::TextureHandle,
	index_count: u32,
	instance_count: u32,
	frames: Vec<render_system::FrameHandle>,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,
	command_buffer: render_system::CommandBufferHandle,
	camera_data_buffer_handle: render_system::BufferHandle,
	current_frame: usize,

	meshes_data_buffer: render_system::BufferHandle,

	shaders_by_name: std::collections::HashMap<String, render_system::ShaderHandle>,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<EntityHandle<Mesh>, u32>,

	mesh_resources: HashMap<&'static str, u32>,
}

impl VisibilityWorldRenderDomain {
	pub fn new(mut orchestrator: orchestrator::OrchestratorReference, render_system: &mut RenderSystem) -> Self {
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
				mat4 view_projection;
			};

			layout(buffer_reference,scalar,buffer_reference_align=2) buffer Mesh {
				mat4 model;
			};

			layout(push_constant) uniform push_constants {
				Camera camera;
				Mesh meshes;
			} pc;

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec3 in_normal;

			layout(location = 0) flat out int out_instance_index;

			//layout(max_vertices=128, max_primitives=64) out;
			void main() {
				gl_Position = pc.camera.view_projection * pc.meshes[gl_InstanceIndex].model * vec4(in_position, 1.0);
				out_instance_index = gl_InstanceIndex;
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(row_major) uniform; layout(row_major) buffer;

			layout(location = 0) flat in int in_instance_index;

			layout(location = 0) out vec4 out_color;

			vec4 get_debug_color(int i) {
				vec4 colors[16] = vec4[16](
					vec4(0.16863, 0.40392, 0.77647, 1),
					vec4(0.32941, 0.76863, 0.21961, 1),
					vec4(0.81961, 0.16078, 0.67451, 1),
					vec4(0.96863, 0.98824, 0.45490, 1),
					vec4(0.75294, 0.09020, 0.75686, 1),
					vec4(0.30588, 0.95686, 0.54510, 1),
					vec4(0.66667, 0.06667, 0.75686, 1),
					vec4(0.78824, 0.91765, 0.27451, 1),
					vec4(0.40980, 0.12745, 0.48627, 1),
					vec4(0.89804, 0.28235, 0.20784, 1),
					vec4(0.93725, 0.67843, 0.33725, 1),
					vec4(0.95294, 0.96863, 0.00392, 1),
					vec4(1.00000, 0.27843, 0.67843, 1),
					vec4(0.29020, 0.90980, 0.56863, 1),
					vec4(0.30980, 0.70980, 0.27059, 1),
					vec4(0.69804, 0.16078, 0.39216, 1)
				);

				return colors[i % 16];
			}

			void main() {
				out_color = get_debug_color(in_instance_index);
			}
		";

		let vertex_layout = [
			crate::render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
			crate::render_system::VertexElement{ name: "NORMAL".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
		];

		let vertex_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, vertex_shader_code.as_bytes());
		let fragment_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, fragment_shader_code.as_bytes());

		let mut shaders_by_name = std::collections::HashMap::new();

		let pipeline_layout_handle = render_system.create_pipeline_layout(&[]);
		
		let vertices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Vertex, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);
		let indices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Index, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		let render_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_backend::TextureFormats::RGBAu8, render_backend::Uses::RenderTarget, render_system::DeviceAccesses::GpuRead);
		let depth_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_backend::TextureFormats::Depth32, render_backend::Uses::DepthStencil, render_system::DeviceAccesses::GpuRead);

		let attachments = [
			render_system::AttachmentInfo {
				texture: render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			},
			render_system::AttachmentInfo {
				texture: depth_target,
				format: crate::render_backend::TextureFormats::Depth32,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
				load: false,
				store: true,
			},
		];

		let pipeline = render_system.create_pipeline(pipeline_layout_handle, &[&vertex_shader, &fragment_shader], &vertex_layout, &attachments);

		shaders_by_name.insert("vertex".to_string(), vertex_shader);
		shaders_by_name.insert("fragment".to_string(), fragment_shader);

		let render_finished_synchronizer = render_system.create_synchronizer(true);
		let image_ready = render_system.create_synchronizer(false);

		let command_buffer = render_system.create_command_buffer();

		let camera_data_buffer_handle = render_system.create_buffer(16 * 4 * 4, render_backend::Uses::Uniform, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		let meshes_data_buffer = render_system.create_buffer(16 * 4 * 4 * 16, render_backend::Uses::Uniform, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		orchestrator.subscribe_to_class(Self::listen_to_camera);
		orchestrator.subscribe_to_class(Self::listen_to_mesh);
		// orchestrator.subscribe(file_tracker::change, )

		Self {
			pipeline_layout_handle,
			pipeline,
			vertices_buffer: vertices_buffer_handle,
			indices_buffer: indices_buffer_handle,

			render_target,
			depth_target,

			index_count: 0,
			instance_count: 0,
			current_frame: 0,
			frames,
			render_finished_synchronizer,
			image_ready,
			command_buffer,
			camera_data_buffer_handle,

			meshes_data_buffer,

			shaders_by_name,

			camera: None,

			meshes: HashMap::new(),

			mesh_resources: HashMap::new(),
		}
	}

	pub fn update_shader(&mut self, render_system: &mut RenderSystem, shader_name: &str, shader_source: &str) {
		println!("Updating shader: {}", shader_name);

		let shader_source = shader_source.as_bytes();

		let new_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, shader_source);

		let vertex_layout = [
			crate::render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
			crate::render_system::VertexElement{ name: "NORMAL".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
		];

		let attachments = self.get_attachments();

		let vertex_shader_handle = self.shaders_by_name.get("vertex").unwrap().clone();

		let pipeline = render_system.create_pipeline(self.pipeline_layout_handle, &[vertex_shader_handle, &new_shader], &vertex_layout, &attachments);

		self.pipeline = pipeline;

		self.shaders_by_name.insert("fragment".to_string(), new_shader);
	}

	fn listen_to_camera(&mut self, orchestrator: orchestrator::OrchestratorReference, camera_handle: EntityHandle<camera::Camera>, camera: &Camera) {
		self.camera = Some(camera_handle);
	}

	fn get_transform(&self) -> Mat4f { return Mat4f::identity(); }
	fn set_transform(&mut self, orchestrator: OrchestratorReference, value: Mat4f) {
		let render_system = orchestrator.get_by_class::<RenderSystem>();
		let mut render_system = render_system.get_mut();
		let render_system: &mut render_system::RenderSystem = render_system.downcast_mut().unwrap();

		let meshes_data_slice = render_system.get_mut_buffer_slice(None, self.meshes_data_buffer);

		let meshes_data = [
			value,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(0 as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}
	}

	pub const fn trasnform() -> orchestrator::Property<(), Self, Mat4f> { orchestrator::Property::Component { getter: Self::get_transform, setter: Self::set_transform } }

	fn listen_to_mesh(&mut self, orchestrator: orchestrator::OrchestratorReference, mesh_handle: EntityHandle<Mesh>, mesh: &Mesh) {
		let render_system = orchestrator.get_by_class::<RenderSystem>();
		let mut render_system = render_system.get_mut();
		let render_system: &mut render_system::RenderSystem = render_system.downcast_mut().unwrap();

		orchestrator.tie_self(Self::trasnform, &mesh_handle, Mesh::transform);

		if !self.mesh_resources.contains_key(mesh.resource_id) { // Load only if not already loaded
			let resource_manager = orchestrator.get_by_class::<resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

			let resource_request = resource_manager.get_resource_info(mesh.resource_id);

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let resource_info = if let resource_manager::ResourceContainer::Mesh(s) = &resource_request.resource { s } else { return; };

			let vertex_buffer = render_system.get_mut_buffer_slice(None, self.vertices_buffer);
			let index_buffer = render_system.get_mut_buffer_slice(None, self.indices_buffer);

			let resource = resource_manager.load_resource_into_buffer(&resource_request, vertex_buffer, index_buffer); // TODO: add offset

			self.mesh_resources.insert(mesh.resource_id, self.index_count);

			self.index_count += resource_info.index_count;
		}

		let meshes_data_slice = render_system.get_mut_buffer_slice(None, self.meshes_data_buffer);

		let meshes_data = [
			mesh.transform,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(self.instance_count as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}

		self.meshes.insert(mesh_handle, self.instance_count);

		self.instance_count += 1;
	}

	pub fn render(&mut self, orchestrator: OrchestratorReference) {
		let render_system = orchestrator.get_by_class::<RenderSystem>();
		let mut binding = render_system.get_mut();
  		let render_system: &mut render_system::RenderSystem = binding.downcast_mut().unwrap();

		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return; };

		let frame_handle_option = Some(self.frames[self.current_frame % 2]);

		render_system.wait(frame_handle_option, self.render_finished_synchronizer);

		render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(None, self.camera_data_buffer_handle);

		let camera_position = orchestrator.get_property(camera_handle, camera::Camera::position);
		let camera_orientation = orchestrator.get_property(camera_handle, camera::Camera::orientation);

		let mut view_matrix = maths_rs::Mat4f::identity();
		view_matrix.set_column(3, maths_rs::Vec4f::new(-camera_position.x, -camera_position.y, -camera_position.z, 1f32));

		let rotation_matrix = math::look_at(Vector3::new(camera_orientation.x, camera_orientation.y, camera_orientation.z));

		view_matrix = rotation_matrix * view_matrix;

		let projection_matrix = math::projection_matrix(35f32, 16f32 / 9f32, 0.1f32, 100f32);

		let view_projection_matrix = projection_matrix * view_matrix;

		let camera_data = [
			view_matrix,
			projection_matrix,
			view_projection_matrix,
		];

		let camera_data_bytes = unsafe { std::slice::from_raw_parts(camera_data.as_ptr() as *const u8, std::mem::size_of_val(&camera_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(camera_data_bytes.as_ptr(), camera_data_buffer.as_mut_ptr(), camera_data_bytes.len());
		}

		let image_index = render_system.acquire_swapchain_image(frame_handle_option, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(frame_handle_option, self.command_buffer);

		let attachments = self.get_attachments();

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		command_buffer_recording.bind_pipeline(&self.pipeline);

		let vertex_buffer_descriptors = [
			render_system::BufferDescriptor {
				buffer: self.vertices_buffer,
				offset: 0,
				range: (24 * std::mem::size_of::<Vector3>() as u32) as u64,
				slot: 0,
			},
			render_system::BufferDescriptor {
				buffer: self.vertices_buffer,
				offset: (24 * std::mem::size_of::<Vector3>() as u32) as u64,
				range: (24 * std::mem::size_of::<Vector3>() as u32) as u64,
				slot: 1,
			},
		];

		command_buffer_recording.bind_vertex_buffers(&vertex_buffer_descriptors);

		let index_buffer_index_descriptor = render_system::BufferDescriptor {
			buffer: self.indices_buffer,
			offset: 0,
			range: (self.index_count * std::mem::size_of::<u16>() as u32) as u64,
			slot: 0,
		};

		command_buffer_recording.bind_index_buffer(&index_buffer_index_descriptor);

		let camera_data_buffer_address = render_system.get_buffer_address(None, self.camera_data_buffer_handle);
		let meshes_data_buffer_address = render_system.get_buffer_address(None, self.meshes_data_buffer);

		let data = [
			camera_data_buffer_address,
			meshes_data_buffer_address,
		];

		command_buffer_recording.write_to_push_constant(&self.pipeline_layout_handle, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });

		command_buffer_recording.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);

		command_buffer_recording.end_render_pass();

		let swapchain_texture_handle = render_system.get_swapchain_texture_handle(frame_handle_option);

		command_buffer_recording.copy_textures(&[(self.render_target, swapchain_texture_handle,)]);

		command_buffer_recording.end();

		render_system.execute(frame_handle_option, command_buffer_recording, Some(self.image_ready), Some(self.render_finished_synchronizer), self.render_finished_synchronizer);

		render_system.end_frame_capture();

		render_system.present(frame_handle_option, image_index, self.render_finished_synchronizer);

		self.current_frame += 1;
	}

	fn get_attachments(&mut self) -> [render_system::AttachmentInfo; 2] {
		let attachments = [
			render_system::AttachmentInfo {
				texture: self.render_target,
				format: crate::render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			},
			render_system::AttachmentInfo {
				texture: self.depth_target,
				format: crate::render_backend::TextureFormats::Depth32,
				clear: Some(crate::RGBA { r: 1.0, g: 0.0, b: 0.0, a: 0.0 }),
				load: false,
				store: true,
			},
		];
		attachments
	}
}

impl Entity for VisibilityWorldRenderDomain {}
impl System for VisibilityWorldRenderDomain {}

use crate::orchestrator::{Component, EntityHandle, Orchestrator};

#[derive(component_derive::Component)]
pub struct Mesh{
	pub resource_id: &'static str,
	#[field] pub transform: maths_rs::Mat4f,
}

pub struct MeshParameters {
	pub resource_id: &'static str,
	pub transform: maths_rs::Mat4f,
}

impl Entity for Mesh {}

impl Mesh {
	fn set_transform(&mut self, orchestrator: orchestrator::OrchestratorReference, value: maths_rs::Mat4f) { self.transform = value; }

	fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

	pub const fn transform() -> orchestrator::Property<(), Self, maths_rs::Mat4f> { orchestrator::Property::Component { getter: Mesh::get_transform, setter: Mesh::set_transform } }
}

impl Component for Mesh {
	type Parameters = MeshParameters;
	fn new(orchestrator: OrchestratorReference, params: MeshParameters) -> Self {
		Self {
			resource_id: params.resource_id,
			transform: params.transform,
		}
	}
}