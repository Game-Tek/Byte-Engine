use std::collections::HashMap;

use maths_rs::{prelude::{MatProjection, MatRotate3D, MatTranslate}, Mat4f};

use crate::{resource_manager::{self, shader_resource_handler::{ShaderResourceHandler, Shader}, mesh_resource_handler}, render_system::{RenderSystem, self, FrameHandle}, render_backend, Extent, orchestrator::{Entity, System, self, OrchestratorReference, OwnedComponent}, Vector3, camera::{self, Camera}, math};

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,
	vertices_buffer: render_system::BufferHandle,
	indices_buffer: render_system::BufferHandle,
	render_target: render_system::TextureHandle,
	depth_target: render_system::TextureHandle,
	index_count: u32,
	instance_count: u32,
	frames: Vec<render_system::FrameHandle>,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,
	render_command_buffer: render_system::CommandBufferHandle,
	camera_data_buffer_handle: render_system::BufferHandle,
	current_frame: usize,

	descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	transfer_synchronizer: render_system::SynchronizerHandle,
	transfer_command_buffer: render_system::CommandBufferHandle,

	meshes_data_buffer: render_system::BufferHandle,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<EntityHandle<Mesh>, u32>,

	mesh_resources: HashMap<&'static str, u32>,

	/// Maps resource ids to shaders
	/// The hash and the shader handle are stored to determine if the shader has changed
	shaders: std::collections::HashMap<u64, (u64, render_system::ShaderHandle)>,
}

impl VisibilityWorldRenderDomain {
	pub fn new(mut orchestrator: orchestrator::OrchestratorReference, render_system: &mut RenderSystem) -> Self {
		let frames = (0..2).map(|_| render_system.create_frame()).collect::<Vec<_>>();

		let vertex_layout = [
			render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, binding: 0 },
			render_system::VertexElement{ name: "NORMAL".to_string(), format: crate::render_system::DataTypes::Float3, binding: 0 },
		];

		let bindings = [
			render_system::DescriptorSetLayoutBinding {
				binding: 0,
				descriptor_type: render_backend::DescriptorType::UniformBuffer,
				descriptor_count: 1,
				stage_flags: render_backend::Stages::VERTEX,
				immutable_samplers: None,
			},
			render_system::DescriptorSetLayoutBinding {
				binding: 1,
				descriptor_type: render_backend::DescriptorType::UniformBuffer,
				descriptor_count: 1,
				stage_flags: render_backend::Stages::VERTEX,
				immutable_samplers: None,
			},
		];

		let descriptor_set_layout = render_system.create_descriptor_set_layout(&bindings);

		let descriptor_set = render_system.create_descriptor_set(&descriptor_set_layout, &bindings);

		let pipeline_layout_handle = render_system.create_pipeline_layout(&[descriptor_set_layout]);
		
		let vertices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Vertex, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
		let indices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_backend::Uses::Index, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

		let render_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_backend::TextureFormats::RGBAu8, render_backend::Uses::RenderTarget, render_system::DeviceAccesses::GpuRead);
		let depth_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_backend::TextureFormats::Depth32, render_backend::Uses::DepthStencil, render_system::DeviceAccesses::GpuRead);

		let attachments = [
			render_system::AttachmentInfo {
				texture: render_target,
				format: render_backend::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			},
			render_system::AttachmentInfo {
				texture: depth_target,
				format: render_backend::TextureFormats::Depth32,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
				load: false,
				store: true,
			},
		];

		let render_finished_synchronizer = render_system.create_synchronizer(true);
		let image_ready = render_system.create_synchronizer(false);

		let transfer_synchronizer = render_system.create_synchronizer(false);

		let render_command_buffer = render_system.create_command_buffer();
		let transfer_command_buffer = render_system.create_command_buffer();

		let camera_data_buffer_handle = render_system.create_buffer(16 * 4 * 4, render_backend::Uses::Uniform, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

		let meshes_data_buffer = render_system.create_buffer(16 * 4 * 4 * 16, render_backend::Uses::Uniform, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

		render_system.write(&[
			render_system::DescriptorWrite {
				descriptor_set,
				binding: 0,
				array_element: 0,
				descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: 64 },
			},
			render_system::DescriptorWrite {
				descriptor_set,
				binding: 1,
				array_element: 0,
				descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: 64 },
			},
		]);

		orchestrator.subscribe_to_class(Self::listen_to_camera);
		orchestrator.subscribe_to_class(Self::listen_to_mesh);
		// orchestrator.subscribe(file_tracker::change, )

		Self {
			pipeline_layout_handle,
			vertices_buffer: vertices_buffer_handle,
			indices_buffer: indices_buffer_handle,

			descriptor_set_layout,
			descriptor_set,

			render_target,
			depth_target,

			index_count: 0,
			instance_count: 0,
			current_frame: 0,
			frames,

			render_finished_synchronizer,
			image_ready,
			render_command_buffer,

			camera_data_buffer_handle,

			transfer_synchronizer,
			transfer_command_buffer,

			meshes_data_buffer,

			shaders: HashMap::new(),

			camera: None,

			meshes: HashMap::new(),

			mesh_resources: HashMap::new(),
		}
	}

	// pub fn update_shader(&mut self, render_system: &mut RenderSystem, shader_name: &str, shader_source: &str) {
	// 	println!("Updating shader: {}", shader_name);

	// 	let shader_source = shader_source.as_bytes();

	// 	let new_shader = render_system.add_shader(crate::render_system::ShaderSourceType::GLSL, shader_source);

	// 	let vertex_layout = [
	// 		render_system::VertexElement{ name: "POSITION".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
	// 		render_system::VertexElement{ name: "NORMAL".to_string(), format: crate::render_system::DataTypes::Float3, shuffled: false },
	// 	];

	// 	let attachments = self.get_attachments();

	// 	let vertex_shader_handle = self.shaders_by_name.get("vertex").unwrap().clone();

	// 	let pipeline = render_system.create_pipeline(self.pipeline_layout_handle, &[vertex_shader_handle, &new_shader], &vertex_layout, &attachments);

	// 	self.pipeline = pipeline;

	// 	self.shaders_by_name.insert("fragment".to_string(), new_shader);
	// }

	fn listen_to_camera(&mut self, orchestrator: orchestrator::OrchestratorReference, camera_handle: EntityHandle<camera::Camera>, camera: &Camera) {
		self.camera = Some(camera_handle);
	}

	fn get_transform(&self) -> Mat4f { return Mat4f::identity(); }
	fn set_transform(&mut self, orchestrator: OrchestratorReference, value: Mat4f) {
		let render_system = orchestrator.get_by_class::<RenderSystem>();
		let mut render_system = render_system.get_mut();
		let render_system: &mut render_system::RenderSystem = render_system.downcast_mut().unwrap();

		let closed_frame_index = self.current_frame % 2;

		let meshes_data_slice = render_system.get_mut_buffer_slice(Some(FrameHandle(closed_frame_index as u32)), self.meshes_data_buffer);

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

			let resource_request = resource_manager.request_resource(mesh.resource_id);

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let mut options = resource_manager::Options { resources: Vec::new(), };

			for resource in &resource_request.resources {
				match resource.class.as_str() {
					"Shader" => {}
					"Material" => {
						// TODO: update pipeline
					}
					"Mesh" => {
						let vertex_buffer = render_system.get_mut_buffer_slice(None, self.vertices_buffer);
						let index_buffer = render_system.get_mut_buffer_slice(None, self.indices_buffer);

						options.resources.push(resource_manager::OptionResource {
							path: resource.path.clone(),
							buffer: vertex_buffer,
						});
					}
					_ => {}
				}
			}

			let resource = if let Ok(a) = resource_manager.load_resource(resource_request, Some(options), None) { a } else { return; };

			let (response, buffer) = (resource.0, resource.1.unwrap());

			for resource in &response.resources {
				match resource.class.as_str() {
					"Shader" => {
						let shader: &Shader = resource.resource.downcast_ref().unwrap();

						let hash = resource.hash; let resource_id = resource.id;

						if let Some((old_hash, old_shader)) = self.shaders.get(&resource_id) {
							if *old_hash == hash { continue; }
						}

						let offset = resource.offset as usize;
						let size = resource.size as usize;

						let new_shader = render_system.add_shader(crate::render_system::ShaderSourceType::SPIRV, &buffer[offset..(offset + size)]);

						self.shaders.insert(resource_id, (hash, new_shader));
					}
					"Material" => {}
					"Mesh" => {
						self.mesh_resources.insert(mesh.resource_id, self.index_count);

						let mesh: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						self.index_count += mesh.index_count;
					}
					_ => {}
				}
			}
		}

		let closed_frame_index = self.current_frame % 2;

		let meshes_data_slice = render_system.get_mut_buffer_slice(Some(FrameHandle(closed_frame_index as u32)), self.meshes_data_buffer);

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

		{
			let mut command_buffer_recording = render_system.create_command_buffer_recording(frame_handle_option, self.transfer_command_buffer);

			command_buffer_recording.synchronize_buffers();

			render_system.execute(frame_handle_option, command_buffer_recording, &[], &[self.transfer_synchronizer], self.transfer_synchronizer);
		}

		render_system.wait(frame_handle_option, self.render_finished_synchronizer);

		//render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(frame_handle_option, self.camera_data_buffer_handle);

		let camera_position = orchestrator.get_property(camera_handle, camera::Camera::position);
		let camera_orientation = orchestrator.get_property(camera_handle, camera::Camera::orientation);

		let view_matrix = maths_rs::Mat4f::from_translation(-camera_position) * math::look_at(camera_orientation);

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

		let mut command_buffer_recording = render_system.create_command_buffer_recording(frame_handle_option, self.render_command_buffer);

		let attachments = self.get_attachments();

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		//command_buffer_recording.bind_pipeline(&self.pipeline);
		// TODO: bind pipelines

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

		let camera_data_buffer_address = render_system.get_buffer_address(frame_handle_option, self.camera_data_buffer_handle);
		let meshes_data_buffer_address = render_system.get_buffer_address(frame_handle_option, self.meshes_data_buffer);

		let data = [
			camera_data_buffer_address,
			meshes_data_buffer_address,
		];

		command_buffer_recording.bind_descriptor_set(self.pipeline_layout_handle, 0, &self.descriptor_set);

		command_buffer_recording.write_to_push_constant(&self.pipeline_layout_handle, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });

		command_buffer_recording.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);

		command_buffer_recording.end_render_pass();

		let swapchain_texture_handle = render_system.get_swapchain_texture_handle(frame_handle_option);

		command_buffer_recording.copy_textures(&[(self.render_target, swapchain_texture_handle,)]);

		render_system.execute(frame_handle_option, command_buffer_recording, &[self.transfer_synchronizer, self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		//render_system.end_frame_capture();

		render_system.present(frame_handle_option, image_index, self.render_finished_synchronizer);

		render_system.wait(frame_handle_option, self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over

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