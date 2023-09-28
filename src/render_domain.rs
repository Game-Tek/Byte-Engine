use std::collections::HashMap;

use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::{resource_manager::{self, mesh_resource_handler, shader_resource_handler::Shader}, rendering::render_system::{RenderSystem, self}, Extent, orchestrator::{Entity, System, self, OrchestratorReference}, Vector3, camera::{self, Camera}, math, window_system};

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,
	vertices_buffer: render_system::BufferHandle,
	indices_buffer: render_system::BufferHandle,
	render_target: render_system::TextureHandle,
	depth_target: render_system::TextureHandle,
	index_count: u32,
	instance_count: u32,
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
	shaders: std::collections::HashMap<u64, (u64, render_system::ShaderHandle, render_system::ShaderTypes)>,

	pipeline: Option<render_system::PipelineHandle>,

	swapchain_handles: Vec<render_system::SwapchainHandle>,
}

impl VisibilityWorldRenderDomain {
	pub fn new() -> orchestrator::EntityReturn<Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
			let mut render_system = render_system.get_mut();
			let render_system: &mut render_system::RenderSystemImplementation = render_system.downcast_mut().unwrap();

			let _vertex_layout = [
				render_system::VertexElement{ name: "POSITION".to_string(), format: render_system::DataTypes::Float3, binding: 0 },
				render_system::VertexElement{ name: "NORMAL".to_string(), format: render_system::DataTypes::Float3, binding: 0 },
			];

			let bindings = [
				render_system::DescriptorSetLayoutBinding {
					binding: 0,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::VERTEX,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::VERTEX,
					immutable_samplers: None,
				},
			];

			let descriptor_set_layout = render_system.create_descriptor_set_layout(&bindings);

			let descriptor_set = render_system.create_descriptor_set(&descriptor_set_layout, &bindings);

			let pipeline_layout_handle = render_system.create_pipeline_layout(&[descriptor_set_layout]);
			
			let vertices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_system::Uses::Vertex, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let indices_buffer_handle = render_system.create_buffer(1024 * 1024 * 16, render_system::Uses::Index, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let render_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_system::TextureFormats::RGBAu8, render_system::Uses::RenderTarget, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let depth_target = render_system.create_texture(Extent::new(1920, 1080, 1), render_system::TextureFormats::Depth32, render_system::Uses::DepthStencil, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let _attachments = [
				render_system::AttachmentInformation {
					texture: render_target,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::RGBAu8,
					clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					texture: depth_target,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::Depth32,
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

			let camera_data_buffer_handle = render_system.create_buffer(16 * 4 * 4, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let meshes_data_buffer = render_system.create_buffer(16 * 4 * 4 * 16, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

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

				pipeline: None,

				swapchain_handles: Vec::new(),
			}
		})
			.add_post_creation_function(Box::new(Self::load_needed_assets))
			.add_listener::<camera::Camera>()
			.add_listener::<Mesh>()
			.add_listener::<window_system::Window>()
	}

	fn load_needed_assets(&mut self, orchestrator: OrchestratorReference) {
		let resource_manager = orchestrator.get_by_class::<resource_manager::ResourceManager>();
		let mut resource_manager = resource_manager.get_mut();
		let resource_manager: &mut resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system: &mut render_system::RenderSystemImplementation = render_system.downcast_mut().unwrap();

		let (response, buffer) = resource_manager.get("cube").unwrap();

		for resource in &response.resources {
			match resource.class.as_str() {
				"Shader" => {
					let shader: &Shader = resource.resource.downcast_ref().unwrap();

					let hash = resource.hash; let resource_id = resource.id;

					if let Some((old_hash, _old_shader, _)) = self.shaders.get(&resource_id) {
						if *old_hash == hash { continue; }
					}

					let offset = resource.offset as usize;
					let size = resource.size as usize;

					let new_shader = render_system.add_shader(render_system::ShaderSourceType::SPIRV, shader.stage, &buffer[offset..(offset + size)]);

					self.shaders.insert(resource_id, (hash, new_shader, shader.stage));
				}
				"Material" => {
					let shaders = resource.required_resources.iter().map(|f| response.resources.iter().find(|r| &r.path == f).unwrap().id).collect::<Vec<_>>();

					let shaders = shaders.iter().map(|shader| {
						let (_hash, shader, shader_type) = self.shaders.get(shader).unwrap();

						(shader, *shader_type)
					}).collect::<Vec<_>>();

					let vertex_layout = [
						render_system::VertexElement{ name: "POSITION".to_string(), format: render_system::DataTypes::Float3, binding: 0 },
						render_system::VertexElement{ name: "NORMAL".to_string(), format: render_system::DataTypes::Float3, binding: 1 },
					];

					let targets = [
						render_system::AttachmentInformation {
							texture: self.render_target,
							layout: render_system::Layouts::RenderTarget,
							format: render_system::TextureFormats::RGBAu8,
							clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
							load: false,
							store: true,
						},
						render_system::AttachmentInformation {
							texture: self.depth_target,
							layout: render_system::Layouts::RenderTarget,
							format: render_system::TextureFormats::Depth32,
							clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
							load: false,
							store: true,
						},
					];

					let pipeline_layout_handle = self.pipeline_layout_handle;

					let pipeline = render_system.create_pipeline(&pipeline_layout_handle, &shaders, &vertex_layout, &targets);

					self.pipeline = Some(pipeline);
				}
				_ => {}
			}
		}
	}

	fn get_transform(&self) -> Mat4f { return Mat4f::identity(); }
	fn set_transform(&mut self, orchestrator: OrchestratorReference, value: Mat4f) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<&mut render_system::RenderSystemImplementation>().unwrap();

		// let closed_frame_index = self.current_frame % 2;

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let meshes_data = [
			value,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(0 as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}
	}

	/// Return the property for the transform of a mesh
	pub const fn transform() -> orchestrator::Property<(), Self, Mat4f> { orchestrator::Property::Component { getter: Self::get_transform, setter: Self::set_transform } }

	pub fn render(&mut self, orchestrator: OrchestratorReference) {
		if let None = self.pipeline { return; }
		if self.swapchain_handles.len() == 0 { return; }

		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut binding = render_system.get_mut();
  		let render_system = binding.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return; };

		{
			let mut command_buffer_recording = render_system.create_command_buffer_recording(self.transfer_command_buffer, None);

			// command_buffer_recording TODO: Copy the data from the CPU to the GPU

			command_buffer_recording.execute(&[], &[self.transfer_synchronizer], self.transfer_synchronizer);
		}

		render_system.wait(self.render_finished_synchronizer);

		//render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(self.camera_data_buffer_handle);

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

		let swapchain_handle = self.swapchain_handles[0];

		let image_index = render_system.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(self.render_command_buffer, Some(self.current_frame as u32));

		let attachments = self.get_attachments();

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		command_buffer_recording.bind_pipeline(&self.pipeline.as_ref().unwrap());

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

		let camera_data_buffer_address = render_system.get_buffer_address(self.camera_data_buffer_handle);
		let meshes_data_buffer_address = render_system.get_buffer_address(self.meshes_data_buffer);

		let data = [
			camera_data_buffer_address,
			meshes_data_buffer_address,
		];

		command_buffer_recording.bind_descriptor_set(&self.pipeline_layout_handle, 0, &self.descriptor_set);

		command_buffer_recording.write_to_push_constant(&self.pipeline_layout_handle, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });

		command_buffer_recording.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.copy_to_swapchain(self.render_target, swapchain_handle);

		command_buffer_recording.execute(&[self.transfer_synchronizer, self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		//render_system.end_frame_capture();

		render_system.present(image_index, &[swapchain_handle], self.render_finished_synchronizer);

		render_system.wait(self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over

		self.current_frame += 1;
	}

	fn get_attachments(&mut self) -> [render_system::AttachmentInformation; 2] {
		let attachments = [
			render_system::AttachmentInformation {
				texture: self.render_target,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::RGBAu8,
				clear: Some(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				texture: self.depth_target,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::Depth32,
				clear: Some(crate::RGBA { r: 1.0, g: 0.0, b: 0.0, a: 0.0 }),
				load: false,
				store: true,
			},
		];
		attachments
	}
}

impl orchestrator::EntitySubscriber<camera::Camera> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<camera::Camera>, camera: &camera::Camera) {
		self.camera = Some(handle);
	}
}

impl orchestrator::EntitySubscriber<Mesh> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Mesh>, mesh: &Mesh) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		orchestrator.tie_self(Self::transform, &handle, Mesh::transform);

		if !self.mesh_resources.contains_key(mesh.resource_id) { // Load only if not already loaded
			let resource_manager = orchestrator.get_by_class::<resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

			let resource_request = resource_manager.request_resource(mesh.resource_id);

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let mut options = resource_manager::Options { resources: Vec::new(), };

			for resource in &resource_request.resources {
				match resource.class.as_str() {
					"Mesh" => {
						let vertex_buffer = render_system.get_mut_buffer_slice(self.vertices_buffer);
						let index_buffer = render_system.get_mut_buffer_slice(self.indices_buffer);

						options.resources.push(resource_manager::OptionResource {
							path: resource.path.clone(),
							buffers: vec![resource_manager::Buffer{ buffer: vertex_buffer, tag: "Vertex".to_string() }, resource_manager::Buffer{ buffer: index_buffer, tag: "Index".to_string() }],
						});
					}
					_ => {}
				}
			}

			let resource = if let Ok(a) = resource_manager.load_resource(resource_request, Some(options), None) { a } else { return; };

			let (response, _buffer) = (resource.0, resource.1.unwrap());

			for resource in &response.resources {
				match resource.class.as_str() {
					"Mesh" => {
						self.mesh_resources.insert(mesh.resource_id, self.index_count);

						let mesh: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						self.index_count += mesh.index_count;
					}
					_ => {}
				}
			}
		}

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let meshes_data = [
			mesh.transform,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(self.instance_count as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}

		self.meshes.insert(handle, self.instance_count);

		self.instance_count += 1;
	}
}

impl orchestrator::EntitySubscriber<window_system::Window> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<window_system::Window>, window: &window_system::Window) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let window_system = orchestrator.get_by_class::<window_system::WindowSystem>();
		let mut window_system = window_system.get_mut();
		let window_system = window_system.downcast_mut::<window_system::WindowSystem>().unwrap();

		let swapchain_handle = render_system.bind_to_window(&window_system.get_os_handles(&handle));

		self.swapchain_handles.push(swapchain_handle);
	}
}

impl Entity for VisibilityWorldRenderDomain {}
impl System for VisibilityWorldRenderDomain {}

use crate::orchestrator::{Component, EntityHandle};

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
	fn set_transform(&mut self, _orchestrator: orchestrator::OrchestratorReference, value: maths_rs::Mat4f) { self.transform = value; }

	fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

	pub const fn transform() -> orchestrator::Property<(), Self, maths_rs::Mat4f> { orchestrator::Property::Component { getter: Mesh::get_transform, setter: Mesh::set_transform } }
}

impl Component for Mesh {
	// type Parameters<'a> = MeshParameters;
}