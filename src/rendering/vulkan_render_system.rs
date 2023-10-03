use std::{collections::HashMap, num::NonZeroU64, hash::Hasher};

use crate::{orchestrator, window_system, render_debugger::RenderDebugger};

pub struct VulkanRenderSystem {
	entry: ash::Entry,
	instance: ash::Instance,
	debug_utils: ash::extensions::ext::DebugUtils,
	debug_utils_messenger: vk::DebugUtilsMessengerEXT,
	physical_device: vk::PhysicalDevice,
	device: ash::Device,
	queue_family_index: u32,
	queue: vk::Queue,
	swapchain: ash::extensions::khr::Swapchain,
	surface: ash::extensions::khr::Surface,
	acceleration_structure: ash::extensions::khr::AccelerationStructure,
	ray_tracing_pipeline: ash::extensions::khr::RayTracingPipeline,

	debugger: RenderDebugger,

	frames: u8,

	buffers: Vec<Buffer>,
	textures: Vec<Texture>,
	allocations: Vec<Allocation>,
	meshes: Vec<Mesh>,
	command_buffers: Vec<CommandBuffer>,
	synchronizers: Vec<Synchronizer>,
	swapchains: Vec<Swapchain>,
}

fn insert_return_length<T>(collection: &mut Vec<T>, value: T) -> usize {
	let length = collection.len();
	collection.push(value);
	return length;
}

impl orchestrator::Entity for VulkanRenderSystem {}
impl orchestrator::System for VulkanRenderSystem {}

impl render_system::RenderSystem for VulkanRenderSystem {
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: render_system::Uses, resource_device_accesses: render_system::DeviceAccesses) -> render_system::AllocationHandle {
		self.create_allocation_internal(size, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[render_system::VertexElement]) -> render_system::MeshHandle {
		let mut hasher = std::collections::hash_map::DefaultHasher::new();

		std::hash::Hash::hash_slice(&vertex_layout, &mut hasher);

		let vertex_layout_hash = hasher.finish();

		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(buffer_size, render_system::Uses::Vertex | render_system::Uses::Index);

		let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		unsafe {
			let vertex_buffer_pointer = pointer.expect("No pointer");
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size as usize);
			let index_buffer_pointer = vertex_buffer_pointer.offset(vertex_buffer_size.next_multiple_of(16) as isize);
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size as usize);
		}

		let mesh_handle = render_system::MeshHandle(self.meshes.len() as u64);

		self.meshes.push(Mesh {
			buffer: buffer_creation_result.resource,
			allocation: allocation_handle,
			vertex_count,
			index_count,
			vertex_size: vertex_layout.size(),
		});

		mesh_handle
	}

	/// Creates a shader.
	fn create_shader(&mut self, shader_source_type: render_system::ShaderSourceType, stage: render_system::ShaderTypes, shader: &[u8]) -> render_system::ShaderHandle {
		match shader_source_type {
			render_system::ShaderSourceType::GLSL => {
				let compiler = shaderc::Compiler::new().unwrap();
				let mut options = shaderc::CompileOptions::new().unwrap();
		
				options.set_optimization_level(shaderc::OptimizationLevel::Performance);
				options.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_2 as u32);
				options.set_generate_debug_info();
				options.set_target_spirv(shaderc::SpirvVersion::V1_5);
				options.set_invert_y(true);
		
				let shader_text = std::str::from_utf8(shader).unwrap();
		
				let binary = compiler.compile_into_spirv(shader_text, shaderc::ShaderKind::InferFromSource, "shader_name", "main", Some(&options));
				
				match binary {
					Ok(binary) => {		
						self.create_vulkan_shader(stage, binary.as_binary_u8())
					},
					Err(err) => {
						error!("Error compiling shader: {}", err);
						panic!("Error compiling shader: {}", err);
					}
				}
			}
			render_system::ShaderSourceType::SPIRV => {
				self.create_vulkan_shader(stage, shader)
			}
		}
	}

	fn create_descriptor_set_layout(&mut self, bindings: &[render_system::DescriptorSetLayoutBinding]) -> render_system::DescriptorSetLayoutHandle {
		fn m(rs: &mut VulkanRenderSystem, bindings: &[render_system::DescriptorSetLayoutBinding], layout_bindings: &mut Vec<vk::DescriptorSetLayoutBinding>) -> vk::DescriptorSetLayout {
			if let Some(binding) = bindings.get(0) {
				let b = vk::DescriptorSetLayoutBinding::default()
				.binding(binding.binding)
				.descriptor_type(match binding.descriptor_type {
					render_system::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
					render_system::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
					render_system::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
					render_system::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
					render_system::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
				})
				.descriptor_count(binding.descriptor_count)
				.stage_flags(binding.stage_flags.into());

				let x = if let Some(inmutable_samplers) = &binding.immutable_samplers {
					inmutable_samplers.iter().map(|sampler| vk::Sampler::from_raw(sampler.0)).collect::<Vec<_>>()
				} else {
					Vec::new()
				};

				b.immutable_samplers(&x);

				layout_bindings.push(b);

				m(rs, &bindings[1..], layout_bindings)
			} else {
				let descriptor_set_layout_create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&layout_bindings);
		
				let descriptor_set_layout = unsafe { rs.device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None).expect("No descriptor set layout") };

				descriptor_set_layout
			}
		}

		let descriptor_set_layout = m(self, bindings, &mut Vec::new());

		render_system::DescriptorSetLayoutHandle(descriptor_set_layout.as_raw())
	}

	fn create_descriptor_set(&mut self, descriptor_set_layout_handle: &render_system::DescriptorSetLayoutHandle, bindings: &[render_system::DescriptorSetLayoutBinding]) -> render_system::DescriptorSetHandle {
		let pool_sizes = bindings.iter().map(|binding| {
			vk::DescriptorPoolSize::default()
				.ty(match binding.descriptor_type {
					render_system::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
					render_system::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
					render_system::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
					render_system::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
					render_system::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
				})
				.descriptor_count(binding.descriptor_count)
				/* .build() */
		})
		.collect::<Vec<_>>();

		let descriptor_pool_create_info = vk::DescriptorPoolCreateInfo::default()
			.max_sets(3)
			.pool_sizes(&pool_sizes);

		let descriptor_pool = unsafe { self.device.create_descriptor_pool(&descriptor_pool_create_info, None).expect("No descriptor pool") };

		let descriptor_set_layouts = [vk::DescriptorSetLayout::from_raw(descriptor_set_layout_handle.0)];

		let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
			.descriptor_pool(descriptor_pool)
			.set_layouts(&descriptor_set_layouts)
			/* .build() */;

		let descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&descriptor_set_allocate_info).expect("No descriptor set") };

		let descriptor_set = descriptor_sets[0];

		render_system::DescriptorSetHandle(descriptor_set.as_raw())
	}

	fn write(&self, descriptor_set_writes: &[render_system::DescriptorWrite]) {
		for descriptor_set_write in descriptor_set_writes {
			let mut buffers: Vec<vk::DescriptorBufferInfo> = Vec::new();
			let mut images: Vec<vk::DescriptorImageInfo> = Vec::new();

			let descriptor_type = match descriptor_set_write.descriptor {
				render_system::Descriptor::Buffer { handle: _, size: _ } => {
					vk::DescriptorType::STORAGE_BUFFER
				},
				render_system::Descriptor::Texture(handle) => {
					let texture = &self.textures[handle.0 as usize];
					match texture.layout {
						vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => vk::DescriptorType::STORAGE_IMAGE,
						_ => vk::DescriptorType::STORAGE_IMAGE,
					}
				},
				render_system::Descriptor::Sampler(_) => {
					vk::DescriptorType::SAMPLER
				},
			};

			let write_info = vk::WriteDescriptorSet::default()
				.dst_set(vk::DescriptorSet::from_raw(descriptor_set_write.descriptor_set.0))
				.dst_binding(descriptor_set_write.binding)
				.dst_array_element(descriptor_set_write.array_element)
				.descriptor_type(descriptor_type)
			;

			let write_info = match descriptor_set_write.descriptor {
				render_system::Descriptor::Buffer { handle, size } => {
					let a = vk::DescriptorBufferInfo::default()
						.buffer(self.buffers[handle.0 as usize].buffer)
						.offset(0 as u64)
						.range(size as u64);
					buffers.push(a);
					write_info.buffer_info(&buffers)
				},
				render_system::Descriptor::Texture(handle) => {
					let texture = &self.textures[handle.0 as usize];
					let a = vk::DescriptorImageInfo::default()
						.image_layout(vk::ImageLayout::GENERAL)
						.image_view(texture.image_view);
					images.push(a);
					write_info.image_info(&images)
				},
				render_system::Descriptor::Sampler(handle) => {
					let a = vk::DescriptorImageInfo::default()
						.sampler(vk::Sampler::from_raw(handle.0));
					images.push(a);
					write_info.image_info(&images)
				},
				_ => panic!("Invalid descriptor info"),
			};

			unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
		}
	}

	fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[render_system::DescriptorSetLayoutHandle]) -> render_system::PipelineLayoutHandle {
		// self.create_vulkan_pipeline_layout(&descriptor_set_layout_handles.iter().map(|descriptor_set_layout_handle| vk::DescriptorSetLayout::from_raw(descriptor_set_layout_handle.0)).collect::<Vec<_>>())
		let push_constant_ranges = [vk::PushConstantRange::default().size(64).offset(0).stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::COMPUTE)];
		let set_layouts = descriptor_set_layout_handles.iter().map(|set_layout| vk::DescriptorSetLayout::from_raw(set_layout.0)).collect::<Vec<_>>();

  		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&set_layouts)
			.push_constant_ranges(&push_constant_ranges)
			/* .build() */;

		let pipeline_layout = unsafe { self.device.create_pipeline_layout(&pipeline_layout_create_info, None).expect("No pipeline layout") };

		render_system::PipelineLayoutHandle(pipeline_layout.as_raw())
	}

	fn create_raster_pipeline(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, shader_handles: &[(&render_system::ShaderHandle, render_system::ShaderTypes)], vertex_layout: &[render_system::VertexElement], targets: &[render_system::AttachmentInformation]) -> render_system::PipelineHandle {
		let pipeline_configuration_blocks = [
			render_system::PipelineConfigurationBlocks::Shaders { shaders: shader_handles, },
			render_system::PipelineConfigurationBlocks::Layout { layout: pipeline_layout_handle },
			render_system::PipelineConfigurationBlocks::VertexInput { vertex_elements: vertex_layout },
			render_system::PipelineConfigurationBlocks::RenderTargets { targets: &targets },
		];

		self.create_vulkan_pipeline(&pipeline_configuration_blocks)
	}

	fn create_compute_pipeline(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, shader_handle: &render_system::ShaderHandle) -> render_system::PipelineHandle {
		let create_infos = [
			vk::ComputePipelineCreateInfo::default()
				.stage(vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(vk::ShaderModule::from_raw(shader_handle.0))
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					/* .build() */
				)
				.layout(vk::PipelineLayout::from_raw(pipeline_layout_handle.0))
		];

		let pipeline_handle = unsafe {
			self.device.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None).expect("No compute pipeline")[0]
		};

		render_system::PipelineHandle(pipeline_handle.as_raw())
	}

	fn create_command_buffer(&mut self) -> render_system::CommandBufferHandle {
		let command_buffer_handle = render_system::CommandBufferHandle(self.command_buffers.len() as u64);

		let command_buffers = (0..self.frames).map(|_| {
			let command_buffer_handle = render_system::CommandBufferHandle(self.command_buffers.len() as u64);

			let command_pool_create_info = vk::CommandPoolCreateInfo::default()
			.queue_family_index(self.queue_family_index)
			/* .build() */;

			let command_pool = unsafe { self.device.create_command_pool(&command_pool_create_info, None).expect("No command pool") };

			let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
				.command_pool(command_pool)
				.level(vk::CommandBufferLevel::PRIMARY)
				.command_buffer_count(1)
				/* .build() */;

			let command_buffers = unsafe { self.device.allocate_command_buffers(&command_buffer_allocate_info).expect("No command buffer") };

			CommandBufferInternal { command_pool, command_buffer: command_buffers[0], }
		}).collect::<Vec<_>>();

		self.command_buffers.push(CommandBuffer {
			frames: command_buffers,
		});

		command_buffer_handle
	}

	fn create_command_buffer_recording(&self, command_buffer_handle: render_system::CommandBufferHandle, frmae_index: Option<u32>) -> Box<dyn render_system::CommandBufferRecording + '_> {
		let recording = VulkanCommandBufferRecording::new(self, command_buffer_handle, frmae_index);
		recording.begin();
		Box::new(recording)
	}

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
	fn create_buffer(&mut self, size: usize, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, use_case: render_system::UseCases) -> render_system::BufferHandle {
		if device_accesses.contains(render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead) {
			match use_case {
				render_system::UseCases::STATIC => {
					let buffer_creation_result = self.create_vulkan_buffer(size, resource_uses);

					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

					let buffer_handle = render_system::BufferHandle(self.buffers.len() as u64);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size: buffer_creation_result.size,
						device_address,
						pointer,
					});
					
					buffer_handle
				}
				render_system::UseCases::DYNAMIC => {	
					let buffer_creation_result = self.create_vulkan_buffer(size, resource_uses | render_system::Uses::TransferDestination);
	
					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);
	
					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);
	
					let buffer_handle = render_system::BufferHandle(self.buffers.len() as u64);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size: buffer_creation_result.size,
						device_address,
						pointer,
					});

					let buffer_creation_result = self.create_vulkan_buffer(size, resource_uses | render_system::Uses::TransferSource);

					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size: buffer_creation_result.size,
						device_address,
						pointer,
					});

					buffer_handle
				}
			}
		} else if device_accesses.contains(render_system::DeviceAccesses::GpuWrite) {
			let buffer_creation_result = self.create_vulkan_buffer(size, resource_uses);

			let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

			let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			let buffer_handle = render_system::BufferHandle(self.buffers.len() as u64);

			self.buffers.push(Buffer {
				buffer: buffer_creation_result.resource,
				size: buffer_creation_result.size,
				device_address,
				pointer,
			});

			buffer_handle
		} else {
			panic!("Invalid device accesses");
		}
	}

	fn get_buffer_address(&self, buffer_handle: render_system::BufferHandle) -> u64 {
		self.buffers[buffer_handle.0 as usize].device_address
	}

	fn get_buffer_slice(&mut self, buffer_handle: render_system::BufferHandle) -> &[u8] {
		let buffer = self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts(buffer.pointer, buffer.size as usize)
		}
	}

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice(&self, buffer_handle: render_system::BufferHandle) -> &mut [u8] {
		let buffer = self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, buffer.size as usize)
		}
	}

	/// Creates a texture.
	fn create_texture(&mut self, name: Option<&str>, extent: crate::Extent, format: render_system::TextureFormats, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, use_case: render_system::UseCases) -> render_system::TextureHandle {
		let size = (extent.width * extent.height * extent.depth * 4) as usize;

		let texture_handle = render_system::TextureHandle(self.textures.len() as u64);

		let mut previous_texture_handle: Option<render_system::TextureHandle> = None;

		let extent = vk::Extent3D::default().width(extent.width).height(extent.height).depth(extent.depth);

		for _ in 0..(match use_case { render_system::UseCases::DYNAMIC => { self.frames } render_system::UseCases::STATIC => { 1 }}) {
			let resource_uses = if resource_uses.contains(render_system::Uses::Texture) {
				resource_uses | render_system::Uses::TransferDestination
			} else {
				resource_uses
			};

			let texture_creation_result = self.create_vulkan_texture(name, extent, format, resource_uses | render_system::Uses::TransferSource, device_accesses, render_system::AccessPolicies::WRITE, 1);

			let (allocation_handle, pointer) = self.create_allocation_internal(texture_creation_result.size, device_accesses);

			let (address, pointer) = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);

			let texture_handle = render_system::TextureHandle(self.textures.len() as u64);

			let image_view = self.create_vulkan_texture_view(&texture_creation_result.resource, format, 0);

			let staging_buffer = if device_accesses.contains(render_system::DeviceAccesses::CpuRead) {
				let staging_buffer_creation_result = self.create_vulkan_buffer(size, render_system::Uses::TransferDestination);

				let (allocation_handle, pointer) = self.create_allocation_internal(staging_buffer_creation_result.size, render_system::DeviceAccesses::CpuRead);

				let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);

				let staging_buffer_handle = render_system::BufferHandle(self.buffers.len() as u64);

				self.buffers.push(Buffer {
					buffer: staging_buffer_creation_result.resource,
					size: staging_buffer_creation_result.size,
					device_address: address,
					pointer,
				});

				Some(staging_buffer_handle)
			} else if device_accesses.contains(render_system::DeviceAccesses::CpuWrite) {
				let staging_buffer_creation_result = self.create_vulkan_buffer(size, render_system::Uses::TransferSource);

				let (allocation_handle, pointer) = self.create_allocation_internal(staging_buffer_creation_result.size, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

				let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);

				let staging_buffer_handle = render_system::BufferHandle(self.buffers.len() as u64);

				self.buffers.push(Buffer {
					buffer: staging_buffer_creation_result.resource,
					size: staging_buffer_creation_result.size,
					device_address: address,
					pointer,
				});

				Some(staging_buffer_handle)
			} else {
				None
			};

			self.textures.push(Texture {
				next: None,
				staging_buffer,
				allocation_handle,
				image: texture_creation_result.resource,
				image_view,
				pointer,
				extent,
				format: to_format(format),
				format_: format,
				layout: vk::ImageLayout::UNDEFINED,
			});

			if let Some(previous_texture_handle) = previous_texture_handle {
				self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
			}

			previous_texture_handle = Some(texture_handle);
		}

		texture_handle
	}

	fn create_sampler(&mut self) -> render_system::SamplerHandle {
		render_system::SamplerHandle(self.create_vulkan_sampler().as_raw())
	}

	fn bind_to_window(&mut self, window_os_handles: &window_system::WindowOsHandles) -> render_system::SwapchainHandle {
		let surface = self.create_vulkan_surface(window_os_handles); 

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let extent = surface_capabilities.current_extent;

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.surface(surface)
			.min_image_count(surface_capabilities.min_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(surface_capabilities.current_extent)
			.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(surface_capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(vk::PresentModeKHR::FIFO)
			.image_array_layers(1)
			.clipped(true);

		// Load extension here and not during device creation because we rarely need it.
		let swapchain_loader = ash::extensions::khr::Swapchain::new(&self.instance, &self.device);

		let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None).expect("No swapchain") };

		let images = unsafe { self.swapchain.get_swapchain_images(swapchain).expect("No swapchain images") };

		let swapchain_handle = render_system::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			surface,
			surface_present_mode: vk::PresentModeKHR::FIFO,
			swapchain,
		});

		swapchain_handle
	}

	fn get_texture_data(&self, texture_copy_handle: render_system::TextureCopyHandle) -> &[u8] {
		// let texture = self.textures.iter().find(|texture| texture.parent.map_or(false, |x| texture_handle == x)).unwrap(); // Get the proxy texture
		let texture = &self.textures[texture_copy_handle.0 as usize];
		let buffer_handle = texture.staging_buffer.expect("No staging buffer");
		let buffer = &self.buffers[buffer_handle.0 as usize];
		if buffer.pointer.is_null() { panic!("Texture data was requested but texture has no memory associated."); }
		let slice = unsafe { std::slice::from_raw_parts::<'static, u8>(buffer.pointer as *mut u8, (texture.extent.width * texture.extent.height * texture.extent.depth) as usize) };
		slice
	}

	/// Creates a synchronization primitive (implemented as a semaphore/fence/event).\
	/// Multiple underlying synchronization primitives are created, one for each frame
	fn create_synchronizer(&mut self, signaled: bool) -> render_system::SynchronizerHandle {
		let synchronizer_handle = render_system::SynchronizerHandle(self.synchronizers.len() as u64);

		let mut previous_synchronizer_handle: Option<render_system::SynchronizerHandle> = None;

		for i in 0..self.frames {
			let synchronizer_handle = render_system::SynchronizerHandle(self.synchronizers.len() as u64);
			self.synchronizers.push(Synchronizer {
				fence: self.create_vulkan_fence(signaled),
				semaphore: self.create_vulkan_semaphore(signaled),
			});
			previous_synchronizer_handle = Some(synchronizer_handle);
		}

		synchronizer_handle
	}

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
	fn acquire_swapchain_image(&self, swapchain_handle: render_system::SwapchainHandle, synchronizer_handle: render_system::SynchronizerHandle) -> u32 {
		let synchronizer = &self.synchronizers[synchronizer_handle.0 as usize];
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];

		let acquisition_result = unsafe { self.swapchain.acquire_next_image(swapchain.swapchain, u64::MAX, synchronizer.semaphore, vk::Fence::null()) };

		let (index, swapchain_state) = if let Ok((index, state)) = acquisition_result {
			if !state {
				(index, render_system::SwapchainStates::Ok)
			} else {
				(index, render_system::SwapchainStates::Suboptimal)
			}
		} else {
			(0, render_system::SwapchainStates::Invalid)
		};

		if swapchain_state == render_system::SwapchainStates::Suboptimal || swapchain_state == render_system::SwapchainStates::Invalid {
			log::error!("Swapchain state is suboptimal or invalid. Recreation is yet to be implemented.");
		}

		index
	}

	fn present(&self, image_index: u32, swapchains: &[render_system::SwapchainHandle], synchronizer_handle: render_system::SynchronizerHandle) {
		let synchronizer = self.synchronizers[synchronizer_handle.0 as usize];

		let swapchains = swapchains.iter().map(|swapchain_handle| { let swapchain = &self.swapchains[swapchain_handle.0 as usize]; swapchain.swapchain }).collect::<Vec<_>>();
		let wait_semaphores = [synchronizer.semaphore];

		let image_indices = [image_index];

  		let present_info = vk::PresentInfoKHR::default()
			.swapchains(&swapchains)
			.wait_semaphores(&wait_semaphores)
			.image_indices(&image_indices);

		unsafe { self.swapchain.queue_present(self.queue, &present_info); }
	}

	fn wait(&self, synchronizer_handle: render_system::SynchronizerHandle) {
		let synchronizer = self.synchronizers[synchronizer_handle.0 as usize];
		unsafe { self.device.wait_for_fences(&[synchronizer.fence], true, u64::MAX).expect("No fence wait"); }
		unsafe { self.device.reset_fences(&[synchronizer.fence]).expect("No fence reset"); }
	}

	fn start_frame_capture(&self) {
		self.debugger.start_frame_capture();
	}

	fn end_frame_capture(&self) {
		self.debugger.end_frame_capture();
	}
}

use ash::{vk::{self, ValidationFeatureEnableEXT, Handle}, Entry};
use log::{warn, error};

use super::render_system::{self, CommandBufferRecording, TextureFormats};

#[derive(Clone)]
pub(crate) struct Swapchain {
	surface: vk::SurfaceKHR,
	surface_present_mode: vk::PresentModeKHR,
	swapchain: vk::SwapchainKHR,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSet {
	descriptor_set: vk::DescriptorSet,
}

#[derive(Clone, Copy)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
}

#[derive(Clone, Copy)]
pub(crate) struct CommandBufferInternal {
	command_pool: vk::CommandPool,
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone)]
pub(crate) struct CommandBuffer {
	frames: Vec<CommandBufferInternal>,
}

#[derive(Clone, Copy)]
pub(crate) struct Allocation {
	memory: vk::DeviceMemory,
	pointer: *mut u8,
}

unsafe impl Send for Allocation {}

#[derive(Clone, Copy)]
pub(crate) struct Buffer {
	buffer: vk::Buffer,
	size: usize,
	device_address: vk::DeviceAddress,
	pointer: *mut u8,
}

unsafe impl Send for Buffer {}

#[derive(Clone, Copy)]
pub(crate) struct Synchronizer {
	fence: vk::Fence,
	semaphore: vk::Semaphore,
}

#[derive(Clone, Copy)]
pub(crate) struct Texture {
	next: Option<render_system::TextureHandle>,
	staging_buffer: Option<render_system::BufferHandle>,
	allocation_handle: render_system::AllocationHandle,
	image: vk::Image,
	image_view: vk::ImageView,
	pointer: *const u8,
	extent: vk::Extent3D,
	format: vk::Format,
	format_: render_system::TextureFormats,
	layout: vk::ImageLayout,
}

unsafe impl Send for Texture {}

// #[derive(Clone, Copy)]
// pub(crate) struct AccelerationStructure {
// 	acceleration_structure: vk::AccelerationStructureKHR,
// }

static mut COUNTER: u32 = 0;

unsafe extern "system" fn vulkan_debug_utils_callback(message_severity: vk::DebugUtilsMessageSeverityFlagsEXT, _message_type: vk::DebugUtilsMessageTypeFlagsEXT, p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT, _p_user_data: *mut std::ffi::c_void,) -> vk::Bool32 {
    let message = std::ffi::CStr::from_ptr((*p_callback_data).p_message);

	match message_severity {
		vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
			warn!("{}", message.to_str().unwrap());
		}
		vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
			error!("{}", message.to_str().unwrap());
			COUNTER += 1;
		}
		_ => {}
	}

    vk::FALSE
}

fn to_clear_value(clear: render_system::ClearValue) -> vk::ClearValue {
	match clear {
		render_system::ClearValue::None => vk::ClearValue::default(),
		render_system::ClearValue::Color(clear) => vk::ClearValue {
			color: vk::ClearColorValue {
				float32: [clear.r, clear.g, clear.b, clear.a],
			},
		},
		render_system::ClearValue::Depth(clear) => vk::ClearValue {
			depth_stencil: vk::ClearDepthStencilValue {
				depth: clear,
				stencil: 0,
			},
		},
		render_system::ClearValue::Integer(r, g, b, a) => vk::ClearValue {
			color: vk::ClearColorValue {
				uint32: [r, g, b, a],
			},
		},
	}
}

fn texture_format_and_resource_use_to_image_layout(_texture_format: render_system::TextureFormats, layout: render_system::Layouts, access: Option<render_system::AccessPolicies>) -> vk::ImageLayout {
	match layout {
		render_system::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		render_system::Layouts::RenderTarget => if _texture_format != render_system::TextureFormats::Depth32 { vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL } else { vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL },
		render_system::Layouts::Transfer => {
			match access {
				Some(a) => {
					if a.intersects(render_system::AccessPolicies::READ) {
						vk::ImageLayout::TRANSFER_SRC_OPTIMAL
					} else if a.intersects(render_system::AccessPolicies::WRITE) {
						vk::ImageLayout::TRANSFER_DST_OPTIMAL
					} else {
						vk::ImageLayout::UNDEFINED
					}
				}
				None => vk::ImageLayout::UNDEFINED
			}
		}
		render_system::Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		render_system::Layouts::Texture => vk::ImageLayout::READ_ONLY_OPTIMAL,
		render_system::Layouts::General => vk::ImageLayout::GENERAL,
	}
}

fn to_load_operation(value: bool) -> vk::AttachmentLoadOp {
	if value {
		vk::AttachmentLoadOp::LOAD
	} else {
		vk::AttachmentLoadOp::CLEAR
	}
}

fn to_store_operation(value: bool) -> vk::AttachmentStoreOp {
	if value {
		vk::AttachmentStoreOp::STORE
	} else {
		vk::AttachmentStoreOp::DONT_CARE
	}
}

fn to_format(format: render_system::TextureFormats) -> vk::Format {
	match format {
		render_system::TextureFormats::RGBAu8 => vk::Format::R8G8B8A8_UNORM,
		render_system::TextureFormats::RGBAu16 => vk::Format::R16G16B16A16_SFLOAT,
		render_system::TextureFormats::RGBAu32 => vk::Format::R32G32B32A32_SFLOAT,
		render_system::TextureFormats::RGBAf16 => vk::Format::R16G16B16A16_SFLOAT,
		render_system::TextureFormats::RGBAf32 => vk::Format::R32G32B32A32_SFLOAT,
		render_system::TextureFormats::RGBu10u10u11 => vk::Format::R16G16_S10_5_NV,
		render_system::TextureFormats::BGRAu8 => vk::Format::B8G8R8A8_SRGB,
		render_system::TextureFormats::Depth32 => vk::Format::D32_SFLOAT,
		TextureFormats::U32 => vk::Format::R32_UINT,
	}
}

fn to_shader_stage_flags(shader_type: render_system::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		render_system::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		render_system::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		render_system::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,		
	}
}

fn to_pipeline_stage_flags(stages: render_system::Stages) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(render_system::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER
	}

	if stages.contains(render_system::Stages::FRAGMENT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
	}

	if stages.contains(render_system::Stages::COMPUTE) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
	}

	if stages.contains(render_system::Stages::TRANSFER) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::TRANSFER
	}

	if stages.contains(render_system::Stages::PRESENTATION) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::BOTTOM_OF_PIPE
	}

	if stages.contains(render_system::Stages::SHADER_READ) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER; // TODO: not really?
	}

	if stages.contains(render_system::Stages::INDIRECT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT;
	}

	pipeline_stage_flags
}

fn to_access_flags(accesses: render_system::AccessPolicies, stages: render_system::Stages) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(render_system::AccessPolicies::READ) {
		if stages.intersects(render_system::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(render_system::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(render_system::Stages::SHADER_READ) {
			access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ;
		}
		if stages.intersects(render_system::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_READ
		}
		if stages.intersects(render_system::Stages::INDIRECT) {
			access_flags |= vk::AccessFlags2::INDIRECT_COMMAND_READ
		}
	}

	if accesses.contains(render_system::AccessPolicies::WRITE) {
		if stages.intersects(render_system::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_WRITE
		}
		if stages.intersects(render_system::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(render_system::Stages::FRAGMENT) {
			access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
		}
	}

	access_flags
}

impl Into<vk::ShaderStageFlags> for render_system::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		if self.intersects(render_system::Stages::VERTEX) {
			shader_stage_flags |= vk::ShaderStageFlags::VERTEX
		}

		if self.intersects(render_system::Stages::FRAGMENT) {
			shader_stage_flags |= vk::ShaderStageFlags::FRAGMENT
		}

		if self.intersects(render_system::Stages::COMPUTE) {
			shader_stage_flags |= vk::ShaderStageFlags::COMPUTE
		}

		shader_stage_flags
	}
}

impl Into<vk::Format> for render_system::DataTypes {
	fn into(self) -> vk::Format {
		match self {
			render_system::DataTypes::Float => vk::Format::R32_SFLOAT,
			render_system::DataTypes::Float2 => vk::Format::R32G32_SFLOAT,
			render_system::DataTypes::Float3 => vk::Format::R32G32B32_SFLOAT,
			render_system::DataTypes::Float4 => vk::Format::R32G32B32A32_SFLOAT,
			render_system::DataTypes::Int => vk::Format::R32_SINT,
			render_system::DataTypes::Int2 => vk::Format::R32G32_SINT,
			render_system::DataTypes::Int3 => vk::Format::R32G32B32_SINT,
			render_system::DataTypes::Int4 => vk::Format::R32G32B32A32_SINT,
			render_system::DataTypes::UInt => vk::Format::R32_UINT,
			render_system::DataTypes::UInt2 => vk::Format::R32G32_UINT,
			render_system::DataTypes::UInt3 => vk::Format::R32G32B32_UINT,
			render_system::DataTypes::UInt4 => vk::Format::R32G32B32A32_UINT,
		}
	}
}

trait Size {
	fn size(&self) -> usize;
}

impl Size for render_system::DataTypes {
	fn size(&self) -> usize {
		match self {
			render_system::DataTypes::Float => std::mem::size_of::<f32>(),
			render_system::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			render_system::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			render_system::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			render_system::DataTypes::Int => std::mem::size_of::<i32>(),
			render_system::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			render_system::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			render_system::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			render_system::DataTypes::UInt => std::mem::size_of::<u32>(),
			render_system::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			render_system::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			render_system::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}
}

impl Size for &[render_system::VertexElement] {
	fn size(&self) -> usize {
		let mut size = 0;

		for element in *self {
			size += element.format.size();
		}

		size
	}
}

impl VulkanRenderSystem {
	pub fn new() -> VulkanRenderSystem {
		let entry: ash::Entry = Entry::linked();

		let application_info = vk::ApplicationInfo::default()
			.api_version(vk::make_api_version(0, 1, 3, 0));

		let layer_names = [
			#[cfg(debug_assertions)]
			std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap().as_ptr(),
			/*std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_LUNARG_api_dump\0").unwrap().as_ptr(),*/
		];

		let extension_names = [
			#[cfg(debug_assertions)]
			ash::extensions::ext::DebugUtils::NAME.as_ptr(),
			ash::extensions::khr::Surface::NAME.as_ptr(),
			ash::extensions::khr::XcbSurface::NAME.as_ptr(),
		];

		let enabled_validation_features = [ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION, ValidationFeatureEnableEXT::BEST_PRACTICES];

		let mut validation_features = vk::ValidationFeaturesEXT::default()
			.enabled_validation_features(&enabled_validation_features);

		let instance_create_info = vk::InstanceCreateInfo::default()
			.push_next(&mut validation_features/* .build() */)
			.application_info(&application_info)
			.enabled_layer_names(&layer_names)
			.enabled_extension_names(&extension_names)
			/* .build() */;

		let instance = unsafe { entry.create_instance(&instance_create_info, None).expect("No instance") };

		let debug_utils = ash::extensions::ext::DebugUtils::new(&entry, &instance);

		let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
			.message_severity(
				vk::DebugUtilsMessageSeverityFlagsEXT::INFO | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
			)
			.message_type(
				vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
			)
			.pfn_user_callback(Some(vulkan_debug_utils_callback));

		let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).expect("Debug Utils Callback") };

		let physical_devices = unsafe { instance.enumerate_physical_devices().expect("No physical devices.") };

		let physical_device;

		{
			let best_physical_device = physical_devices.iter().max_by_key(|physical_device| {
				let properties = unsafe { instance.get_physical_device_properties(*physical_device.clone()) };
				let features = unsafe { instance.get_physical_device_features(*physical_device.clone()) };

				// If the device doesn't support sample rate shading, don't even consider it.
				if features.sample_rate_shading == vk::FALSE { return 0; }

				let mut device_score = 0 as u64;

				device_score += match properties.device_type {
					vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
					_ => 0,
				};

				device_score
			});

			physical_device = *best_physical_device.expect("No physical devices.");
		}

		let queue_family_properties = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };

		let queue_family_index = queue_family_properties
			.iter()
			.enumerate()
			.find_map(|(index, ref info)| {
				let supports_graphics = info.queue_flags.contains(vk::QueueFlags::GRAPHICS);
				let supports_compute = info.queue_flags.contains(vk::QueueFlags::COMPUTE);
				let supports_transfer = info.queue_flags.contains(vk::QueueFlags::TRANSFER);

				if supports_graphics && supports_compute && supports_transfer {
					Some(index as u32)
				} else {
					None
				}
			})
			.expect("No queue family index found.");

		let device_extension_names = [
			ash::extensions::khr::Swapchain::NAME.as_ptr(),
		];

		let queue_create_infos = [vk::DeviceQueueCreateInfo::default()
			.queue_family_index(queue_family_index)
			.queue_priorities(&[1.0])
			/* .build() */];

		let mut physical_device_vulkan_11_features = vk::PhysicalDeviceVulkan11Features::default()
			.uniform_and_storage_buffer16_bit_access(true)
			.storage_buffer16_bit_access(true)
		;

		let mut physical_device_vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default()
			.descriptor_indexing(true).descriptor_binding_partially_bound(true)
			.shader_sampled_image_array_non_uniform_indexing(true).shader_storage_image_array_non_uniform_indexing(true)
			.scalar_block_layout(true)
			.buffer_device_address(true)
			.separate_depth_stencil_layouts(true)
			.shader_buffer_int64_atomics(true).shader_float16(true).shader_int8(true)
			.storage_buffer8_bit_access(true)
			.uniform_and_storage_buffer8_bit_access(true)
			.vulkan_memory_model(true)
			.vulkan_memory_model_device_scope(true)
		;

		let mut physical_device_vulkan_13_features = vk::PhysicalDeviceVulkan13Features::default()
			.pipeline_creation_cache_control(true)
			.subgroup_size_control(true)
			.compute_full_subgroups(true)
			.synchronization2(true)
			.dynamic_rendering(true)
			.maintenance4(true)
		;

		let enabled_physical_device_features = vk::PhysicalDeviceFeatures::default()
			.shader_int16(true)
			.shader_int64(true)
			.shader_uniform_buffer_array_dynamic_indexing(true)
			.shader_storage_buffer_array_dynamic_indexing(true)
			.shader_storage_image_array_dynamic_indexing(true)
			.shader_storage_image_write_without_format(true)
			.texture_compression_bc(true)
		;			

  		let device_create_info = vk::DeviceCreateInfo::default()
			.push_next(&mut physical_device_vulkan_11_features/* .build() */)
			.push_next(&mut physical_device_vulkan_12_features/* .build() */)
			.push_next(&mut physical_device_vulkan_13_features/* .build() */)
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names)
			.enabled_features(&enabled_physical_device_features/* .build() */)
			/* .build() */;

		let device: ash::Device = unsafe { instance.create_device(physical_device, &device_create_info, None).expect("No device") };

		let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

		let acceleration_structure = ash::extensions::khr::AccelerationStructure::new(&instance, &device);
		let ray_tracing_pipeline = ash::extensions::khr::RayTracingPipeline::new(&instance, &device);

		let swapchain = ash::extensions::khr::Swapchain::new(&instance, &device);
		let surface = ash::extensions::khr::Surface::new(&entry, &instance);

		VulkanRenderSystem { 
			entry,
			instance,
			debug_utils,
			debug_utils_messenger,
			physical_device,
			device,
			queue_family_index,
			queue,
			swapchain,
			surface,
			acceleration_structure,
			ray_tracing_pipeline,

			debugger: RenderDebugger::new(),

			frames: 2, // Assuming double buffering

			allocations: Vec::new(),
			buffers: Vec::new(),
			textures: Vec::new(),
			meshes: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers: Vec::new(),
			swapchains: Vec::new(),
		}
	}

	pub fn new_as_system() -> orchestrator::EntityReturn<render_system::RenderSystemImplementation> {
		orchestrator::EntityReturn::new(render_system::RenderSystemImplementation::new(Box::new(VulkanRenderSystem::new())))
	}

	fn get_log_count(&self) -> u32 { unsafe { COUNTER } }

	fn create_vulkan_shader(&self, stage: render_system::ShaderTypes, shader: &[u8]) -> render_system::ShaderHandle {
		let shader_module_create_info = vk::ShaderModuleCreateInfo::default()
			.code(unsafe { shader.align_to::<u32>().1 })
			/* .build() */;

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).expect("No shader module") };

		render_system::ShaderHandle(shader_module.as_raw())
	}

	fn create_vulkan_pipeline(&self, blocks: &[render_system::PipelineConfigurationBlocks]) -> render_system::PipelineHandle {
		/// This function calls itself recursively to build the pipeline the pipeline states.
		/// This is done because this way we can "dynamically" allocate on the stack the needed structs because the recursive call keep them alive.
		fn build_block(vulkan_render_system: &VulkanRenderSystem, pipeline_create_info: vk::GraphicsPipelineCreateInfo<'_>, mut block_iterator: std::slice::Iter<'_, render_system::PipelineConfigurationBlocks>) -> vk::Pipeline {
			if let Some(block) = block_iterator.next() {
				match block {
					render_system::PipelineConfigurationBlocks::VertexInput { vertex_elements } => {
						let mut vertex_input_attribute_descriptions = vec![];
	
						let mut offset_per_binding = [0, 0, 0, 0, 0, 0, 0, 0]; // Assume 8 bindings max

						for (i, vertex_element) in vertex_elements.iter().enumerate() {
							let ve = vk::VertexInputAttributeDescription::default()
								.binding(vertex_element.binding)
								.location(i as u32)
								.format(vertex_element.format.into())
								.offset(offset_per_binding[vertex_element.binding as usize]);
	
							vertex_input_attribute_descriptions.push(ve);
	
							offset_per_binding[vertex_element.binding as usize] += vertex_element.format.size() as u32;
						}

						let max_binding = vertex_elements.iter().map(|ve| ve.binding).max().unwrap() + 1;

						let mut vertex_binding_descriptions = Vec::with_capacity(max_binding as usize);

						for i in 0..max_binding {
							vertex_binding_descriptions.push(
								vk::VertexInputBindingDescription::default()
								.binding(i)
								.stride(offset_per_binding[i as usize])
								.input_rate(vk::VertexInputRate::VERTEX)
							)
						}
	
						let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
							.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
							.vertex_binding_descriptions(&vertex_binding_descriptions);

						let pipeline_create_info = pipeline_create_info.vertex_input_state(&vertex_input_state);

						return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
					}
					render_system::PipelineConfigurationBlocks::InputAssembly {  } => {
						let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
							.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
							.primitive_restart_enable(false);

						let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

						return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
					}
					render_system::PipelineConfigurationBlocks::RenderTargets { targets } => {
						let pipeline_color_blend_attachments = targets.iter().filter(|a| a.format != render_system::TextureFormats::Depth32).map(|_| {
							vk::PipelineColorBlendAttachmentState::default()
								.color_write_mask(vk::ColorComponentFlags::RGBA)
								.blend_enable(false)
								.src_color_blend_factor(vk::BlendFactor::ONE)
								.src_alpha_blend_factor(vk::BlendFactor::ONE)
								.dst_color_blend_factor(vk::BlendFactor::ZERO)
								.dst_alpha_blend_factor(vk::BlendFactor::ZERO)
								.color_blend_op(vk::BlendOp::ADD)
								.alpha_blend_op(vk::BlendOp::ADD)
						}).collect::<Vec<_>>();
	
						let color_attachement_formats: Vec<vk::Format> = targets.iter().filter(|a| a.format != render_system::TextureFormats::Depth32).map(|a| to_format(a.format)).collect::<Vec<_>>();

						let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
							.logic_op_enable(false)
							.logic_op(vk::LogicOp::COPY)
							.attachments(&pipeline_color_blend_attachments)
							.blend_constants([0.0, 0.0, 0.0, 0.0]);

						let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
							.color_attachment_formats(&color_attachement_formats)
							.depth_attachment_format(vk::Format::UNDEFINED);

						let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

						if let Some(_) = targets.iter().find(|a| a.format == render_system::TextureFormats::Depth32) {
							let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
								.depth_test_enable(true)
								.depth_write_enable(true)
								.depth_compare_op(vk::CompareOp::LESS)
								.depth_bounds_test_enable(false)
								.stencil_test_enable(false)
								.front(vk::StencilOpState::default()/* .build() */)
								.back(vk::StencilOpState::default()/* .build() */)
								/* .build() */;

							rendering_info = rendering_info.depth_attachment_format(vk::Format::D32_SFLOAT);

							let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);
							let pipeline_create_info = pipeline_create_info.depth_stencil_state(&depth_stencil_state);

							return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
						} else {
							let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);

							return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
						}
					}
					render_system::PipelineConfigurationBlocks::Shaders { shaders } => {
						let stages = shaders
							.iter()
							.map(|shader| {
								vk::PipelineShaderStageCreateInfo::default()
									.stage(to_shader_stage_flags(shader.1))
									.module(vk::ShaderModule::from_raw(shader.0.0))
									.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
									/* .build() */
							})
							.collect::<Vec<_>>();

						let pipeline_create_info = pipeline_create_info.stages(&stages);

						return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
					}
					render_system::PipelineConfigurationBlocks::Layout { layout } => {
						let pipeline_layout = vk::PipelineLayout::from_raw(layout.0);

						let pipeline_create_info = pipeline_create_info.layout(pipeline_layout);

						return build_block(vulkan_render_system, pipeline_create_info, block_iterator);
					}
				}
			} else {
				let pipeline_create_infos = [pipeline_create_info];

				let pipelines = unsafe { vulkan_render_system.device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None).expect("No pipeline") };

				return pipelines[0];
			}
		}

		let viewports = [vk::Viewport::default()
			.x(0.0)
			.y(9.0)
			.width(16.0)
			.height(-9.0)
			.min_depth(0.0)
			.max_depth(1.0)
			/* .build() */];

		let scissors = [vk::Rect2D::default()
			.offset(vk::Offset2D { x: 0, y: 0 })
			.extent(vk::Extent2D { width: 16, height: 9 })
			/* .build() */];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors)
			/* .build() */;

		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
			.dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

		let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
			.depth_clamp_enable(false)
			.rasterizer_discard_enable(false)
			.polygon_mode(vk::PolygonMode::FILL)
			.cull_mode(vk::CullModeFlags::BACK)
			.front_face(vk::FrontFace::CLOCKWISE)
			.depth_bias_enable(false)
			.depth_bias_constant_factor(0.0)
			.depth_bias_clamp(0.0)
			.depth_bias_slope_factor(0.0)
			.line_width(1.0);

		let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
			.sample_shading_enable(false)
			.rasterization_samples(vk::SampleCountFlags::TYPE_1)
			.min_sample_shading(1.0)
			.alpha_to_coverage_enable(false)
			.alpha_to_one_enable(false)
			/* .build() */;

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
				.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
				.primitive_restart_enable(false);

		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
			.dynamic_state(&dynamic_state)
			.viewport_state(&viewport_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state);

		let pipeline = build_block(self, pipeline_create_info, blocks.iter());

		render_system::PipelineHandle(pipeline.as_raw())
	}

	fn create_texture_internal(&mut self, texture: Texture, previous: Option<render_system::TextureHandle>) -> render_system::TextureHandle {
		let texture_handle = render_system::TextureHandle(self.textures.len() as u64);

		self.textures.push(texture);

		if let Some(previous_texture_handle) = previous {
			self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
		}

		texture_handle
	}

	fn create_vulkan_buffer(&self, size: usize, resource_uses: render_system::Uses) -> MemoryBackedResourceCreationResult<vk::Buffer> {
		let buffer_create_info = vk::BufferCreateInfo::default()
			.size(size as u64)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(
				if resource_uses.contains(render_system::Uses::Vertex) { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::Index) { vk::BufferUsageFlags::INDEX_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::Uniform) { vk::BufferUsageFlags::UNIFORM_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::Storage) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::TransferSource) { vk::BufferUsageFlags::TRANSFER_SRC } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::TransferDestination) { vk::BufferUsageFlags::TRANSFER_DST } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::AccelerationStructure) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR } else { vk::BufferUsageFlags::empty() }
				|
				if resource_uses.contains(render_system::Uses::Indirect) { vk::BufferUsageFlags::INDIRECT_BUFFER } else { vk::BufferUsageFlags::empty() }
				|
				vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS /*We allways use this feature so include constantly*/
			)
			/* .build() */;

		let buffer = unsafe { self.device.create_buffer(&buffer_create_info, None).expect("No buffer") };

		let memory_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

		MemoryBackedResourceCreationResult {
			resource: buffer,
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
		}
	}

	fn create_vulkan_allocation(&self, size: usize,) -> vk::DeviceMemory {
		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(0)
			/* .build() */;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		memory
	}

	fn get_vulkan_buffer_address(&self, buffer: &render_system::BufferHandle, _allocation: &render_system::AllocationHandle) -> u64 {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) }
	}

	fn create_vulkan_texture(&self, name: Option<&str>, extent: vk::Extent3D, format: render_system::TextureFormats, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, _access_policies: render_system::AccessPolicies, mip_levels: u32) -> MemoryBackedResourceCreationResult<vk::Image> {
		let image_type_from_extent = |extent: vk::Extent3D| {
			if extent.depth > 1 {
				vk::ImageType::TYPE_3D
			} else if extent.height > 1 {
				vk::ImageType::TYPE_2D
			} else {
				vk::ImageType::TYPE_1D
			}
		};

		let image_create_info = vk::ImageCreateInfo::default()
			.image_type(image_type_from_extent(extent))
			.format(to_format(format))
			.extent(extent)
			.mip_levels(mip_levels)
			.array_layers(1)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(if !device_accesses.intersects(render_system::DeviceAccesses::CpuRead | render_system::DeviceAccesses::CpuWrite) { vk::ImageTiling::OPTIMAL } else { vk::ImageTiling::LINEAR })
			.usage(
				if resource_uses.intersects(render_system::Uses::Texture) { vk::ImageUsageFlags::SAMPLED } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::Storage) { vk::ImageUsageFlags::STORAGE } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::RenderTarget) && format != render_system::TextureFormats::Depth32 { vk::ImageUsageFlags::COLOR_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::DepthStencil) || format == render_system::TextureFormats::Depth32 { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::TransferSource) { vk::ImageUsageFlags::TRANSFER_SRC } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::TransferDestination) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
			)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED)
			/* .build() */;

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };

		let memory_requirements = unsafe { self.device.get_image_memory_requirements(image) };

		unsafe{
			if let Some(name) = name {
				self.debug_utils.set_debug_utils_object_name(
					self.device.handle(),
					&vk::DebugUtilsObjectNameInfoEXT::default()
						.object_handle(image)
						.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
						/* .build() */
				).expect("No debug utils object name");
			}
		}

		MemoryBackedResourceCreationResult {
			resource: image.to_owned(),
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
		}
	}

	fn create_vulkan_sampler(&self) -> vk::Sampler {
		let sampler_create_info = vk::SamplerCreateInfo::default()
			.mag_filter(vk::Filter::NEAREST)
			.min_filter(vk::Filter::NEAREST)
			.mipmap_mode(vk::SamplerMipmapMode::NEAREST)
			.address_mode_u(vk::SamplerAddressMode::REPEAT)
			.address_mode_v(vk::SamplerAddressMode::REPEAT)
			.address_mode_w(vk::SamplerAddressMode::REPEAT)
			.anisotropy_enable(false)
			.max_anisotropy(1.0)
			.compare_enable(false)
			.compare_op(vk::CompareOp::ALWAYS)
			.min_lod(0.0)
			.max_lod(0.0)
			.mip_lod_bias(0.0)
			.unnormalized_coordinates(false)
			/* .build() */;

		let sampler = unsafe { self.device.create_sampler(&sampler_create_info, None).expect("No sampler") };

		sampler
	}

	fn get_image_subresource_layout(&self, texture: &render_system::TextureHandle, mip_level: u32) -> render_system::ImageSubresourceLayout {
		let image_subresource = vk::ImageSubresource {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			mip_level,
			array_layer: 0,
		};

		let texture = self.textures.get(texture.0 as usize).expect("No texture with that handle.");

		let image_subresource_layout = unsafe { self.device.get_image_subresource_layout(texture.image, image_subresource) };

		render_system::ImageSubresourceLayout {
			offset: image_subresource_layout.offset,
			size: image_subresource_layout.size,
			row_pitch: image_subresource_layout.row_pitch,
			array_pitch: image_subresource_layout.array_pitch,
			depth_pitch: image_subresource_layout.depth_pitch,
		}
	}

	fn bind_vulkan_buffer_memory(&self, info: &MemoryBackedResourceCreationResult<vk::Buffer>, allocation_handle: render_system::AllocationHandle, offset: usize) -> (u64, *mut u8) {
		let buffer = info.resource;
		let allocation = self.allocations.get(allocation_handle.0 as usize).expect("No allocation with that handle.");
		unsafe { self.device.bind_buffer_memory(buffer, allocation.memory, offset as u64).expect("No buffer memory binding") };
		unsafe {
			(self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)), allocation.pointer.add(offset))
		}
	}

	fn bind_vulkan_texture_memory(&self, info: &MemoryBackedResourceCreationResult<vk::Image>, allocation_handle: render_system::AllocationHandle, offset: usize) -> (u64, *mut u8) {
		let image = info.resource;
		let allocation = self.allocations.get(allocation_handle.0 as usize).expect("No allocation with that handle.");
		unsafe { self.device.bind_image_memory(image, allocation.memory, offset as u64).expect("No image memory binding") };
		(0, unsafe { allocation.pointer.add(offset) })
	}

	fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
		let fence_create_info = vk::FenceCreateInfo::default()
			.flags(vk::FenceCreateFlags::empty() | if signaled { vk::FenceCreateFlags::SIGNALED } else { vk::FenceCreateFlags::empty() })
			/* .build() */;
		unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") }
	}

	fn create_vulkan_semaphore(&self, signaled: bool) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default()
			/* .build() */;
		unsafe { self.device.create_semaphore(&semaphore_create_info, None).expect("No semaphore") }
	}

	fn create_vulkan_texture_view(&self, texture: &vk::Image, format: render_system::TextureFormats, _mip_levels: u32) -> vk::ImageView {
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(
				vk::ImageViewType::TYPE_2D
			)
			.format(to_format(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if format != render_system::TextureFormats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: 0,
				layer_count: 1,
			})
			/* .build() */;

		unsafe { self.device.create_image_view(&image_view_create_info, None).expect("No image view") }
	}

	fn create_vulkan_surface(&self, window_os_handles: &window_system::WindowOsHandles) -> vk::SurfaceKHR {
		let xcb_surface_create_info = vk::XcbSurfaceCreateInfoKHR::default()
			.connection(window_os_handles.xcb_connection)
			.window(window_os_handles.xcb_window);

		let xcb_surface = ash::extensions::khr::XcbSurface::new(&self.entry, &self.instance);

		let surface = unsafe { xcb_surface.create_xcb_surface(&xcb_surface_create_info, None).expect("No surface") };

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let surface_format = unsafe { self.surface.get_physical_device_surface_formats(self.physical_device, surface).expect("No surface formats") };

		let surface_format: vk::SurfaceFormatKHR = surface_format
			.iter()
			.find(|format| {
				format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
			})
			.expect("No surface format").to_owned();

		let surface_present_modes = unsafe { self.surface.get_physical_device_surface_present_modes(self.physical_device, surface).expect("No surface present modes") };

		let surface_present_mode: vk::PresentModeKHR = surface_present_modes
			.iter()
			.find(|present_mode| {
				**present_mode == vk::PresentModeKHR::FIFO
			})
			.expect("No surface present mode").to_owned();

		let _surface_resolution = surface_capabilities.current_extent;

		surface
	}

	// fn execute_vulkan_barriers(&self, command_buffer: &render_system::CommandBufferHandle, barriers: &[render_system::BarrierDescriptor]) {
	// 	let mut image_memory_barriers = Vec::new();
	// 	let mut buffer_memory_barriers = Vec::new();
	// 	let mut memory_barriers = Vec::new();

	// 	for barrier in barriers {
	// 		match barrier.barrier {
	// 			render_system::Barrier::Buffer(buffer_barrier) => {
	// 				let buffer_memory_barrier = if let Some(source) = barrier.source {
	// 						vk::BufferMemoryBarrier2KHR::default()
	// 						.src_stage_mask(to_pipeline_stage_flags(source.stage))
	// 						.src_access_mask(to_access_flags(source.access, source.stage))
	// 						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
	// 					} else {
	// 						vk::BufferMemoryBarrier2KHR::default()
	// 						.src_stage_mask(vk::PipelineStageFlags2::empty())
	// 						.src_access_mask(vk::AccessFlags2KHR::empty())
	// 						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
	// 					}
	// 					.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
	// 					.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
	// 					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
	// 					.buffer(self.buffers[buffer_barrier.0 as usize].buffer)
	// 					.offset(0)
	// 					.size(vk::WHOLE_SIZE)
	// 					/* .build() */;

	// 				buffer_memory_barriers.push(buffer_memory_barrier);
	// 			},
	// 			render_system::Barrier::Memory() => {
	// 				let memory_barrier = if let Some(source) = barrier.source {
	// 					vk::MemoryBarrier2::default()
	// 						.src_stage_mask(to_pipeline_stage_flags(source.stage))
	// 						.src_access_mask(to_access_flags(source.access, source.stage))

	// 				} else {
	// 					vk::MemoryBarrier2::default()
	// 						.src_stage_mask(vk::PipelineStageFlags2::empty())
	// 						.src_access_mask(vk::AccessFlags2KHR::empty())
	// 				}
	// 				.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
	// 				.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
	// 				/* .build() */;

	// 				memory_barriers.push(memory_barrier);
	// 			}
	// 			render_system::Barrier::Texture{ source, destination, texture } => {
	// 				let image_memory_barrier = if let Some(barrier_source) = barrier.source {
	// 						if let Some(texture_source) = source {
	// 							vk::ImageMemoryBarrier2KHR::default()
	// 							.old_layout(texture_format_and_resource_use_to_image_layout(texture_source.format, texture_source.layout, Some(barrier_source.access)))
	// 							.src_stage_mask(to_pipeline_stage_flags(barrier_source.stage))
	// 							.src_access_mask(to_access_flags(barrier_source.access, barrier_source.stage))
	// 						} else {
	// 							vk::ImageMemoryBarrier2KHR::default()
	// 							.old_layout(vk::ImageLayout::UNDEFINED)
	// 							.src_stage_mask(vk::PipelineStageFlags2::empty())
	// 							.src_access_mask(vk::AccessFlags2KHR::empty())
	// 						}
	// 					} else {
	// 						vk::ImageMemoryBarrier2KHR::default()
	// 						.old_layout(vk::ImageLayout::UNDEFINED)
	// 						.src_stage_mask(vk::PipelineStageFlags2::empty())
	// 						.src_access_mask(vk::AccessFlags2KHR::empty())
	// 					}
	// 					.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
	// 					.new_layout(texture_format_and_resource_use_to_image_layout(destination.format, destination.layout, Some(barrier.destination.access)))
	// 					.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
	// 					.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
	// 					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
	// 					.image(self.textures[texture.0 as usize].image)
	// 					.subresource_range(vk::ImageSubresourceRange {
	// 						aspect_mask: if destination.format != render_system::TextureFormats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
	// 						base_mip_level: 0,
	// 						level_count: vk::REMAINING_MIP_LEVELS,
	// 						base_array_layer: 0,
	// 						layer_count: vk::REMAINING_ARRAY_LAYERS,
	// 					})
	// 					/* .build() */;

	// 				image_memory_barriers.push(image_memory_barrier);
	// 			},
	// 		}
	// 	}

	// 	let dependency_info = vk::DependencyInfo::default()
	// 		.image_memory_barriers(&image_memory_barriers)
	// 		.buffer_memory_barriers(&buffer_memory_barriers)
	// 		.memory_barriers(&memory_barriers)
	// 		.dependency_flags(vk::DependencyFlags::BY_REGION)
	// 		/* .build() */;

	// 	let command_buffer = self.command_buffers.get(command_buffer.0 as usize).expect("No command buffer with that handle.");

	// 	unsafe { self.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	// }

	/// Allocates memory from the device.
	fn create_allocation_internal(&mut self, size: usize, device_accesses: render_system::DeviceAccesses) -> (render_system::AllocationHandle, Option<*mut u8>) {
		// get memory types
		let memory_properties = unsafe { self.instance.get_physical_device_memory_properties(self.physical_device) };

		let memory_type_index = memory_properties
			.memory_types
			.iter()
			.enumerate()
			.find_map(|(index, memory_type)| {
				let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

				memory_property_flags |= if device_accesses.contains(render_system::DeviceAccesses::CpuRead) { vk::MemoryPropertyFlags::HOST_VISIBLE } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(render_system::DeviceAccesses::CpuWrite) { vk::MemoryPropertyFlags::HOST_COHERENT } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(render_system::DeviceAccesses::GpuRead) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };
				memory_property_flags |= if device_accesses.contains(render_system::DeviceAccesses::GpuWrite) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };

				let memory_type = memory_type.property_flags.contains(memory_property_flags);

				if memory_type {
					Some(index as u32)
				} else {
					None
				}
			})
			.expect("No memory type index found.");

		let mut memory_allocate_flags_info = vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);

		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(memory_type_index)
			.push_next(&mut memory_allocate_flags_info)
			/* .build() */;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		let mut mapped_memory = None;

		if device_accesses.intersects(render_system::DeviceAccesses::CpuRead | render_system::DeviceAccesses::CpuWrite) {
			mapped_memory = Some(unsafe { self.device.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty()).expect("No mapped memory") as *mut u8 });
		}

		let allocation_handle = render_system::AllocationHandle(self.allocations.len() as u64);

		self.allocations.push(Allocation {
			memory,
			pointer: mapped_memory.unwrap_or(std::ptr::null_mut()),
		});

		(allocation_handle, mapped_memory)
	}
}

type TextureState = (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2);

pub struct VulkanCommandBufferRecording<'a> {
	render_system: &'a VulkanRenderSystem,
	command_buffer: render_system::CommandBufferHandle,
	in_render_pass: bool,
	modulo_frame_index: u32,
	/// `texture_states` is used to perform resource tracking on textures.\ It is mainly useful for barriers and transitions and copies.
	texture_states: HashMap<render_system::TextureHandle, TextureState>,
	pipeline_bind_point: vk::PipelineBindPoint,
}

impl VulkanCommandBufferRecording<'_> {
	pub fn new(render_system: &'_ VulkanRenderSystem, command_buffer: render_system::CommandBufferHandle, frame_index: Option<u32>) -> VulkanCommandBufferRecording<'_> {
		VulkanCommandBufferRecording {
			pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
			render_system,
			command_buffer,
			modulo_frame_index: frame_index.map(|frame_index| frame_index % render_system.frames as u32).unwrap_or(0),
			in_render_pass: false,
			texture_states: HashMap::new(),
		}
	}

	/// Retrieves the current state of a texture.\
	/// If the texture has no known state, it will return a default state with undefined layout. This is useful for the first transition of a texture.\
	/// If the texture has a known state, it will return the known state.
	/// Inserts or updates state for a texture.\
	/// If the texture has no known state, it will insert the given state.\
	/// If the texture has a known state, it will update it with the given state.
	/// It will return the given state.
	/// This is useful to perform a transition on a texture.
	fn get_texture_state(&mut self, texture_handle: render_system::TextureHandle, new_texture_state: TextureState) -> (TextureState, TextureState) {
		let (texture_handle, _) = self.get_texture(texture_handle);
		if let Some(old_state) = self.texture_states.insert(texture_handle, new_texture_state) {
			(old_state, new_texture_state)
		} else {
			((vk::ImageLayout::UNDEFINED, vk::PipelineStageFlags2::NONE, vk::AccessFlags2::NONE), new_texture_state)
		}
	}

	fn transition_textures(&mut self, texture_handles: &[(render_system::TextureHandle, (bool, (vk::ImageLayout, vk::PipelineStageFlags2, vk::AccessFlags2)))]) {
		let mut image_memory_barriers = Vec::new();

		for texture_handle in texture_handles {
			let (old_state, new_state) = self.get_texture_state(texture_handle.0, texture_handle.1.1);
			 let (_, texture) = self.get_texture(texture_handle.0);
			let image_memory_barrier = vk::ImageMemoryBarrier2KHR::default()
				.old_layout(old_state.0)
				.src_stage_mask(old_state.1)
				.src_access_mask(old_state.2)
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(new_state.0)
				.dst_stage_mask(new_state.1)
				.dst_access_mask(new_state.2)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(texture.image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: if texture.format_ != render_system::TextureFormats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})
				/* .build() */;

			image_memory_barriers.push(image_memory_barrier);
		}

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		let command_buffer = self.get_command_buffer();

		unsafe { self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	}

	fn get_texture(&self, mut texture_handle: render_system::TextureHandle) -> (render_system::TextureHandle, &Texture) {
		let mut i = 0;
		loop {
			let texture = &self.render_system.textures[texture_handle.0 as usize];
			if i == self.modulo_frame_index {
				return (texture_handle, texture);
			}
			texture_handle = texture.next.unwrap();
			i += 1;
		}
	}

	fn get_command_buffer(&self) -> &CommandBufferInternal {
		&self.render_system.command_buffers[self.command_buffer.0 as usize].frames[self.modulo_frame_index as usize]
	}
}

impl render_system::CommandBufferRecording for VulkanCommandBufferRecording<'_> {
	/// Enables recording on the command buffer.
	fn begin(&self) {
		let command_buffer = self.get_command_buffer();

		unsafe { self.render_system.device.reset_command_pool(command_buffer.command_pool, vk::CommandPoolResetFlags::empty()); } 

		let command_buffer_begin_info = vk::CommandBufferBeginInfo::default()
			.flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
			/* .build() */;

		unsafe { self.render_system.device.begin_command_buffer(command_buffer.command_buffer, &command_buffer_begin_info).expect("No command buffer begin") };
	}

	/// Starts a render pass on the GPU.
	/// A render pass is a particular configuration of render targets which will be used simultaneously to render certain imagery.
	fn start_render_pass(&mut self, extent: crate::Extent, attachments: &[render_system::AttachmentInformation]) {
		self.transition_textures(&attachments.iter().map(|attachment| {
			(attachment.texture, (false, (texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None), if attachment.format == TextureFormats::Depth32 { vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS } else { vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT }, if attachment.format == TextureFormats::Depth32 { vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE } else { vk::AccessFlags2::COLOR_ATTACHMENT_WRITE })))
		}).collect::<Vec<_>>());

		let render_area = vk::Rect2D::default()
			.offset(vk::Offset2D::default().x(0).y(0)/* .build() */)
			.extent(vk::Extent2D::default().width(extent.width).height(extent.height)/* .build() */)
			/* .build() */;

		let color_attchments = attachments.iter().filter(|a| a.format != render_system::TextureFormats::Depth32).map(|attachment| {
			let (_, texture) = self.get_texture(attachment.texture);
			vk::RenderingAttachmentInfo::default()
				.image_view(texture.image_view)
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
				/* .build() */
		}).collect::<Vec<_>>();

		let depth_attachment = attachments.iter().find(|attachment| attachment.format == render_system::TextureFormats::Depth32).map(|attachment| {
			let (_, texture) = self.get_texture(attachment.texture);
			vk::RenderingAttachmentInfo::default()
				.image_view(texture.image_view)
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
				/* .build() */
		}).or(Some(vk::RenderingAttachmentInfo::default())).unwrap();

		let rendering_info = vk::RenderingInfoKHR::default()
			.color_attachments(color_attchments.as_slice())
			.depth_attachment(&depth_attachment)
			.render_area(render_area)
			.layer_count(1)
			/* .build() */;

		let viewports = [
			vk::Viewport {
				x: 0.0,
				y: extent.height as f32,
				width: extent.width as f32,
				height: -(extent.height as f32),
				min_depth: 0.0,
				max_depth: 1.0,
			}
		];

		let command_buffer = self.get_command_buffer();

		unsafe { self.render_system.device.cmd_set_scissor(command_buffer.command_buffer, 0, &[render_area]); }
		unsafe { self.render_system.device.cmd_set_viewport(command_buffer.command_buffer, 0, &viewports); }
		unsafe { self.render_system.device.cmd_begin_rendering(command_buffer.command_buffer, &rendering_info); }

		self.in_render_pass = true;
	}

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self) {
		let command_buffer = self.get_command_buffer();
		unsafe { self.render_system.device.cmd_end_rendering(command_buffer.command_buffer); }
		self.in_render_pass = false;
	}

	fn barriers(&mut self, barriers: &[render_system::BarrierDescriptor]) {
		let mut image_memory_barriers = Vec::new();
		let mut buffer_memory_barriers = Vec::new();
		let mut memory_barriers = Vec::new();

		for barrier in barriers {
			match barrier.barrier {
				render_system::Barrier::Buffer(buffer_barrier) => {
					let buffer_memory_barrier = if let Some(source) = barrier.source {
							vk::BufferMemoryBarrier2KHR::default()
							.src_stage_mask(to_pipeline_stage_flags(source.stage))
							.src_access_mask(to_access_flags(source.access, source.stage))
							.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						} else {
							vk::BufferMemoryBarrier2KHR::default()
							.src_stage_mask(vk::PipelineStageFlags2::empty())
							.src_access_mask(vk::AccessFlags2KHR::empty())
							.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						}
						.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
						.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.buffer(self.render_system.buffers[buffer_barrier.0 as usize].buffer)
						.offset(0)
						.size(vk::WHOLE_SIZE)
						/* .build() */;

					buffer_memory_barriers.push(buffer_memory_barrier);
				},
				render_system::Barrier::Memory => {
					let memory_barrier = if let Some(source) = barrier.source {
						vk::MemoryBarrier2::default()
							.src_stage_mask(to_pipeline_stage_flags(source.stage))
							.src_access_mask(to_access_flags(source.access, source.stage))

					} else {
						vk::MemoryBarrier2::default()
							.src_stage_mask(vk::PipelineStageFlags2::empty())
							.src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
					.dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
					/* .build() */;

					memory_barriers.push(memory_barrier);
				}
				render_system::Barrier::Texture{ source, destination, texture } => {
					let new_layout = texture_format_and_resource_use_to_image_layout(destination.format, destination.layout, Some(barrier.destination.access));
					let new_stage_mask = to_pipeline_stage_flags(barrier.destination.stage);
					let new_access_mask = to_access_flags(barrier.destination.access, barrier.destination.stage);

					let (old_state, new_state) = self.get_texture_state(texture, (new_layout, new_stage_mask, new_access_mask));

					let texture = self.get_texture(texture);

					let image_memory_barrier = if let Some(barrier_source) = barrier.source {
						if let Some(texture_source) = source {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(old_state.0)
							.src_stage_mask(old_state.1)
							.src_access_mask(old_state.2)
						} else {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(vk::ImageLayout::UNDEFINED)
							.src_stage_mask(vk::PipelineStageFlags2::empty())
							.src_access_mask(vk::AccessFlags2KHR::empty())
						}
					} else {
						vk::ImageMemoryBarrier2KHR::default()
						.old_layout(vk::ImageLayout::UNDEFINED)
						.src_stage_mask(vk::PipelineStageFlags2::empty())
						.src_access_mask(vk::AccessFlags2KHR::empty())
					}
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(new_state.0)
						.dst_stage_mask(new_state.1)
						.dst_access_mask(new_state.2)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(texture.1.image)
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: if destination.format != render_system::TextureFormats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
						/* .build() */;
					image_memory_barriers.push(image_memory_barrier);
				},
			}
		}

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.buffer_memory_barriers(&buffer_memory_barriers)
			.memory_barriers(&memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		let command_buffer = self.get_command_buffer();

		unsafe { self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	}

	/// Binds a shader to the GPU.
	fn bind_shader(&self, shader_handle: render_system::ShaderHandle) {
		panic!("Not implemented");
	}

	/// Binds a pipeline to the GPU.
	fn bind_pipeline(&mut self, pipeline_handle: &render_system::PipelineHandle) {
		let command_buffer = self.get_command_buffer();
		let pipeline = vk::Pipeline::from_raw(pipeline_handle.0);
		unsafe { self.render_system.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline); }
	}

	fn bind_compute_pipeline(&mut self, pipeline_handle: &render_system::PipelineHandle) {
		let command_buffer = self.get_command_buffer();
		let pipeline = vk::Pipeline::from_raw(pipeline_handle.0);
		unsafe { self.render_system.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline); }
		self.pipeline_bind_point = vk::PipelineBindPoint::COMPUTE;
	}

	/// Writes to the push constant register.
	fn write_to_push_constant(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, offset: u32, data: &[u8]) {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = vk::PipelineLayout::from_raw(pipeline_layout_handle.0);
		unsafe { self.render_system.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::COMPUTE, offset, data); }
	}

	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &render_system::MeshHandle) {
		let command_buffer = self.get_command_buffer();

		let mesh = &self.render_system.meshes[mesh_handle.0 as usize];

		let buffers = [mesh.buffer];
		let offsets = [0];

		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;

		unsafe { self.render_system.device.cmd_bind_vertex_buffers(command_buffer.command_buffer, 0, &buffers, &offsets); }
		unsafe { self.render_system.device.cmd_bind_index_buffer(command_buffer.command_buffer, mesh.buffer, index_data_offset, vk::IndexType::UINT16); }

		unsafe { self.render_system.device.cmd_draw_indexed(command_buffer.command_buffer, mesh.index_count, 1, 0, 0, 0); }
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[render_system::BufferDescriptor]) {
		let command_buffer = self.get_command_buffer();

		let buffers = buffer_descriptors.iter().map(|buffer_descriptor| self.render_system.buffers[buffer_descriptor.buffer.0 as usize].buffer).collect::<Vec<_>>();
		let offsets = buffer_descriptors.iter().map(|buffer_descriptor| buffer_descriptor.offset as u64).collect::<Vec<_>>();

		// TODO: implent slot splitting
		unsafe { self.render_system.device.cmd_bind_vertex_buffers(command_buffer.command_buffer, 0, &buffers, &offsets); }
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &render_system::BufferDescriptor) {
		let command_buffer = self.get_command_buffer();

		let buffer = self.render_system.buffers[buffer_descriptor.buffer.0 as usize];

		let command_buffer = unsafe { self.render_system.device.cmd_bind_index_buffer(command_buffer.command_buffer, buffer.buffer, buffer_descriptor.offset, vk::IndexType::UINT16); };
	}

	fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
		let command_buffer = self.get_command_buffer();
		unsafe {
			self.render_system.device.cmd_draw_indexed(command_buffer.command_buffer, index_count, instance_count, first_index, vertex_offset, first_instance);
		}
	}

	fn clear_buffer(&mut self, buffer_handle: render_system::BufferHandle) {
		unsafe {
			self.render_system.device.cmd_fill_buffer(self.get_command_buffer().command_buffer, self.render_system.buffers[buffer_handle.0 as usize].buffer, 0, vk::WHOLE_SIZE, 0);
		}
	}

	fn dispatch(&mut self, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();
		unsafe {
			self.render_system.device.cmd_dispatch(command_buffer.command_buffer, x, y, z);
		}
	}

	fn indirect_dispatch(&mut self, buffer_descriptor: &render_system::BufferDescriptor) {
		let command_buffer = self.get_command_buffer();
		let buffer = self.render_system.buffers[buffer_descriptor.buffer.0 as usize];
		unsafe {
			self.render_system.device.cmd_dispatch_indirect(command_buffer.command_buffer, buffer.buffer, buffer_descriptor.offset);
		}
	}

	fn write_texture_data(&mut self, texture_handle: render_system::TextureHandle, data: &[render_system::RGBAu8]) {
		let (texture_handle, texture) = self.get_texture(texture_handle);

		let staging_buffer_handle = texture.staging_buffer.expect("No staging buffer");

		let buffer = &self.render_system.buffers[staging_buffer_handle.0 as usize];

		let pointer = buffer.pointer;

		let subresource_layout = self.render_system.get_image_subresource_layout(&texture_handle, 0);

		if pointer.is_null() {
			for i in data.len()..texture.extent.width as usize * texture.extent.height as usize * texture.extent.depth as usize {
				unsafe {
					std::ptr::write(pointer.offset(i as isize), if i % 4 == 0 { 255 } else { 0 });
				}
			}
		} else {
			let pointer = unsafe { pointer.offset(subresource_layout.offset as isize) };

			for i in 0..texture.extent.height {
				let pointer = unsafe { pointer.offset(subresource_layout.row_pitch as isize * i as isize) };

				unsafe {
					std::ptr::copy_nonoverlapping((data.as_ptr().add(i as usize * texture.extent.width as usize)) as *mut u8, pointer, texture.extent.width as usize * 4);
				}
			}
		}

		let command_buffer = self.get_command_buffer();

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::UNDEFINED)
				.src_stage_mask(vk::PipelineStageFlags2::empty())
				.src_access_mask(vk::AccessFlags2KHR::empty())
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
				.dst_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(texture.image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})
				/* .build() */,
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		unsafe {
			self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info);
		}

		let regions = [vk::BufferImageCopy2::default()
			.buffer_offset(0)
			.buffer_row_length(0)
			.buffer_image_height(0)
			.image_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
				/* .build() */
			)
			.image_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
			.image_extent(vk::Extent3D::default().width(texture.extent.width).height(texture.extent.height).depth(texture.extent.depth)/* .build() */)/* .build() */];

		// Copy to images from staging buffer
		let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
			.src_buffer(buffer.buffer)
			.dst_image(texture.image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&regions);

		unsafe {
			self.render_system.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
		}		

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
				.src_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
				.dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
				.dst_access_mask(vk::AccessFlags2KHR::SHADER_READ)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(texture.image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})
				/* .build() */
		];

		// Transition destination texture to shader read
		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		unsafe {
			self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info);
		}
	}

	/// Performs a series of texture copies.
	// fn copy_textures(&mut self, copies: &[((render_system::TextureHandle, bool, render_system::Layouts, render_system::Stages, render_system::AccessPolicies), (render_system::TextureHandle, bool, render_system::Layouts, render_system::Stages, render_system::AccessPolicies))]) {
	// 	let mut transitions = Vec::new();

	// 	for (f, t) in copies {
	// 		transitions.push((*f, true, render_system::Layouts::Transfer, render_system::Stages::TRANSFER, render_system::AccessPolicies::READ));
	// 		transitions.push((*t, false, render_system::Layouts::Transfer, render_system::Stages::TRANSFER, render_system::AccessPolicies::WRITE));
	// 	}

	// 	self.transition_textures(&transitions);

	// 	let command_buffer = &self.render_system.command_buffers[self.command_buffer.0 as usize];

	// 	for (source, destination) in copies {
	// 		let source_texture = &self.render_system.textures[source.0.0 as usize];
	// 		let destination_texture = &self.render_system.textures[destination.0.0 as usize];
	// 		let (source_layout) = self.get_texture_state(source.0).expect("xx");
	// 		let (destination_layout) = self.get_texture_state(destination.0).expect("xx");

	// 		if source_texture.format == destination_texture.format {
	// 			let image_copies = [vk::ImageCopy2::default()
	// 				.src_subresource(vk::ImageSubresourceLayers::default()
	// 					.aspect_mask(vk::ImageAspectFlags::COLOR)
	// 					.mip_level(0)
	// 					.base_array_layer(0)
	// 					.layer_count(1)
	// 					/* .build() */
	// 				)
	// 				.src_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
	// 				.dst_subresource(vk::ImageSubresourceLayers::default()
	// 					.aspect_mask(vk::ImageAspectFlags::COLOR)
	// 					.mip_level(0)
	// 					.base_array_layer(0)
	// 					.layer_count(1)
	// 					/* .build() */
	// 				)
	// 				.dst_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
	// 				.extent(source_texture.extent/* .build() */)
	// 			];

	// 			let copy_image_info = vk::CopyImageInfo2::default()
	// 				.src_image(source_texture.image)
	// 				.src_image_layout(source.2)
	// 				.dst_image(destination_texture.image)
	// 				.dst_image_layout(destination.2)
	// 				.regions(&image_copies);
	// 				/* .build() */

	// 			unsafe { self.render_system.device.cmd_copy_image2(command_buffer.command_buffer, &copy_image_info); }
	// 		} else {
	// 			let regions = [
	// 				vk::ImageBlit2::default()
	// 				.src_offsets([
	// 					vk::Offset3D::default().x(0).y(0).z(0)/* .build() */,
	// 					vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1)/* .build() */,
	// 				])
	// 				.src_subresource(vk::ImageSubresourceLayers::default()
	// 					.aspect_mask(vk::ImageAspectFlags::COLOR)
	// 					.mip_level(0)
	// 					.base_array_layer(0)
	// 					.layer_count(1)
	// 					/* .build() */
	// 				)
	// 				.dst_offsets([
	// 					vk::Offset3D::default().x(0).y(0).z(0)/* .build() */,
	// 					vk::Offset3D::default().x(destination_texture.extent.width as i32).y(destination_texture.extent.height as i32).z(1)/* .build() */,
	// 				])
	// 				.dst_subresource(vk::ImageSubresourceLayers::default()
	// 					.aspect_mask(vk::ImageAspectFlags::COLOR)
	// 					.mip_level(0)
	// 					.base_array_layer(0)
	// 					.layer_count(1)
	// 					/* .build() */
	// 				)
	// 			];

	// 			let blit_image_info = vk::BlitImageInfo2::default()
	// 				.src_image(source_texture.image)
	// 				.src_image_layout(texture_format_and_resource_use_to_image_layout(source_texture.format, source.2, Some(source.4)))
	// 				.dst_image(destination_texture.image)
	// 				.dst_image_layout(texture_format_and_resource_use_to_image_layout(destination_texture.format, destination.2, Some(destination.4)))
	// 				.regions(&regions);
	// 				/* .build() */
	// 			unsafe { self.render_system.device.cmd_blit_image2(command_buffer.command_buffer, &blit_image_info); }
	// 		}
	// 	}
	// }

	/// Copies GPU accessible texture data to a CPU accessible buffer.
	// fn synchronize_texture(&mut self, texture_handle: render_system::TextureHandle) {
	// 	let mut texture_copies = Vec::new();

	// 	let texture = self.render_system.get_texture(self.frame_handle, texture_handle);

	// 	let copy_dst_texture = self.render_system.textures.iter().enumerate().find(|(_, texture)| texture.parent == Some(texture_handle) && texture.role == "CPU_READ").expect("No CPU_READ texture found. Texture must be created with the CPU read access flag.");
		
	// 	let source_texture_handle = texture_handle;
	// 	let destination_texture_handle = TextureHandle(copy_dst_texture.0 as u32);
		
	// 	let transitions = [
	// 		(source_texture_handle, true, Layouts::Transfer, Stages::TRANSFER, AccessPolicies::READ),
	// 		(destination_texture_handle, false, Layouts::Transfer, Stages::TRANSFER, AccessPolicies::WRITE)
	// 	];

	// 	self.transition_textures(&transitions);

	// 	texture_copies.push(TextureCopy {
	// 		source: texture.texture,
	// 		source_format: texture.format,
	// 		destination: copy_dst_texture.1.texture,
	// 		destination_format: copy_dst_texture.1.format,
	// 		extent: texture.extent,
	// 	});

	// 	self.render_system.render_backend.copy_textures(&self.render_system.command_buffers[self.command_buffer.0 as usize].command_buffer, &texture_copies);
	// }

	fn copy_to_swapchain(&mut self, source_texture_handle: render_system::TextureHandle, swapchain_handle: render_system::SwapchainHandle) {
		// let (old_source_texture_state, new_source_texture_state) = self.get_texture_state(source_texture_handle, (vk::ImageLayout::TRANSFER_SRC_OPTIMAL));

		let (_, source_texture) = self.get_texture(source_texture_handle);
		let swapchain = &self.render_system.swapchains[swapchain_handle.0 as usize];

		let swapchain_images = unsafe {
			self.render_system.swapchain.get_swapchain_images(swapchain.swapchain).expect("No swapchain images found.")
		};

		let swapchain_image = swapchain_images[self.modulo_frame_index as usize];

		// Transition source texture to transfer read layout and swapchain image to transfer write layout

		let command_buffer = self.get_command_buffer();

		let image_memory_barriers = [
			// vk::ImageMemoryBarrier2KHR::default()
			// 	.old_layout(old_source_texture_state)
			// 	.src_stage_mask(vk::PipelineStageFlags2::empty())
			// 	.src_access_mask(vk::AccessFlags2KHR::empty())
			// 	.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			// 	.new_layout(new_source_texture_state)
			// 	.dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
			// 	.dst_access_mask(vk::AccessFlags2KHR::TRANSFER_READ)
			// 	.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			// 	.image(source_texture.image)
			// 	.subresource_range(vk::ImageSubresourceRange {
			// 		aspect_mask: vk::ImageAspectFlags::COLOR,
			// 		base_mip_level: 0,
			// 		level_count: vk::REMAINING_MIP_LEVELS,
			// 		base_array_layer: 0,
			// 		layer_count: vk::REMAINING_ARRAY_LAYERS,
			// 	})
			// 	/* .build() */,
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::UNDEFINED)
				.src_stage_mask(vk::PipelineStageFlags2::empty())
				.src_access_mask(vk::AccessFlags2KHR::empty())
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
				.dst_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(swapchain_image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				}),
				/* .build() */
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		unsafe {
			self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info);
		}

		// Copy texture to swapchain image

		let image_copies = [vk::ImageCopy2::default()
			.src_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
				/* .build() */
			)
			.src_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
			.dst_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
				/* .build() */
			)
			.dst_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
			.extent(source_texture.extent/* .build() */)
		];

		let copy_image_info = vk::CopyImageInfo2::default()
			.src_image(source_texture.image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(swapchain_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_copies);
			/* .build() */

		unsafe { self.render_system.device.cmd_copy_image2(command_buffer.command_buffer, &copy_image_info); }

		// Transition swapchain image to present layout

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
				.src_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
				.dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
				.dst_access_mask(vk::AccessFlags2KHR::empty())
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(swapchain_image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})
				/* .build() */
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		unsafe {
			self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info);
		}
	}

	/// Ends recording on the command buffer.
	fn end(&mut self) {
		let command_buffer = self.get_command_buffer();

		if self.in_render_pass {
			unsafe {
				self.render_system.device.cmd_end_render_pass(command_buffer.command_buffer);
			}
		}
		
		unsafe {
			self.render_system.device.end_command_buffer(command_buffer.command_buffer);
		}
	}

	/// Binds a decriptor set on the GPU.
	fn bind_descriptor_set(&self, pipeline_layout: &render_system::PipelineLayoutHandle, first_set: u32, descriptor_set_handle: &render_system::DescriptorSetHandle) {
		let command_buffer = self.get_command_buffer();

		let pipeline_layout = vk::PipelineLayout::from_raw(pipeline_layout.0);
		let descriptor_sets = [vk::DescriptorSet::from_raw(descriptor_set_handle.0)];

		unsafe {
			self.render_system.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, self.pipeline_bind_point, pipeline_layout, first_set, &descriptor_sets, &[]);
		}
	}

	fn sync_textures(&mut self, texture_handles: &[render_system::TextureHandle]) -> Vec<render_system::TextureCopyHandle> {
		self.transition_textures(&texture_handles.iter().map(|texture_handle| (*texture_handle, (false, (vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2KHR::TRANSFER_READ)))).collect::<Vec<_>>());

		let command_buffer = self.get_command_buffer();

		for texture_handle in texture_handles {
			let (_, texture) = self.get_texture(*texture_handle);
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(staging_buffer_handle) = texture.staging_buffer {
				let staging_buffer = &self.render_system.buffers[staging_buffer_handle.0 as usize];

				let regions = [vk::BufferImageCopy2KHR::default()
					.buffer_offset(0)
					.buffer_row_length(0)
					.buffer_image_height(0)
					.image_subresource(vk::ImageSubresourceLayers::default()
						.aspect_mask(vk::ImageAspectFlags::COLOR)
						.mip_level(0)
						.base_array_layer(0)
						.layer_count(1)
						/* .build() */
					)
					.image_offset(vk::Offset3D::default().x(0).y(0).z(0)/* .build() */)
					.image_extent(texture.extent/* .build() */)
				];

				let copy_image_to_buffer_info = vk::CopyImageToBufferInfo2KHR::default()
					.src_image(texture.image)
					.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
					.dst_buffer(staging_buffer.buffer)
					.regions(&regions);
					/* .build() */	

				unsafe {
					self.render_system.device.cmd_copy_image_to_buffer2(command_buffer.command_buffer, &copy_image_to_buffer_info);
				}
			}
		}

		let mut texture_copies = Vec::new();

		for texture_handle in texture_handles {
			let (handle, texture) = self.get_texture(*texture_handle);
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(_) = texture.staging_buffer {
				texture_copies.push(render_system::TextureCopyHandle(handle.0));
			}
		}

		texture_copies
	}

	fn execute(&mut self, wait_for_synchronizer_handles: &[render_system::SynchronizerHandle], signal_synchronizer_handles: &[render_system::SynchronizerHandle], execution_synchronizer_handle: render_system::SynchronizerHandle) {
		self.end();

		let command_buffer = self.get_command_buffer();

		let command_buffers = [command_buffer.command_buffer];

		let command_buffer_infos = [
			vk::CommandBufferSubmitInfo::default()
				.command_buffer(
					command_buffers[0]
				)
				/* .build() */
		];

		// TODO: Take actual stage masks

		let wait_semaphores = wait_for_synchronizer_handles.iter().map(|wait_for| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.render_system.synchronizers[wait_for.0 as usize].semaphore)
				.stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
				/* .build() */
		}).collect::<Vec<_>>();

		let signal_semaphores = signal_synchronizer_handles.iter().map(|signal| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.render_system.synchronizers[signal.0 as usize].semaphore)
				.stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
				/* .build() */
		}).collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores)
			/* .build() */;

		let execution_completion_synchronizer = &self.render_system.synchronizers[execution_synchronizer_handle.0 as usize];

		unsafe { self.render_system.device.queue_submit2(self.render_system.queue, &[submit_info], execution_completion_synchronizer.fence); }
	}
}

struct Mesh {
	buffer: vk::Buffer,
	allocation: render_system::AllocationHandle,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

struct Frame {

}

#[derive(Clone, Copy)]
/// Stores the information of a memory backed resource.
pub struct MemoryBackedResourceCreationResult<T> {
	/// The resource.
	resource: T,
	/// The final size of the resource.
	size: usize,
	/// Tha alignment the resources needs when bound to a memory region.
	alignment: usize,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_triangle() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::render_triangle(&mut vulkan_render_system);
	}

	#[ignore = "CI doesn't support presentation"]
	#[test]
	fn present() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::present(&mut vulkan_render_system);
	}


	#[ignore = "CI doesn't support presentation"]
	#[test]
	fn multiframe_present() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::multiframe_present(&mut vulkan_render_system);
	}

	#[test]
	fn multiframe_rendering() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::multiframe_rendering(&mut vulkan_render_system);
	}

	#[test]
	fn dynamic_data() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::dynamic_data(&mut vulkan_render_system);
	}

	#[test]
	fn descriptor_sets() {
		let mut vulkan_render_system = VulkanRenderSystem::new();
		render_system::tests::descriptor_sets(&mut vulkan_render_system);
	}
}