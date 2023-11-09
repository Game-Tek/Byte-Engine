use std::{collections::HashMap,};

use ash::vk;

use crate::{orchestrator, window_system, render_debugger::RenderDebugger, rendering::{render_system, self}};

#[cfg(not(test))]
use log::{warn, error, debug};

#[cfg(test)]
use std::{println as error, println as warn, println as debug};

pub struct VulkanRenderSystem {
	entry: ash::Entry,
	instance: ash::Instance,

	#[cfg(debug_assertions)]
	debug_utils: Option<ash::extensions::ext::DebugUtils>,
	#[cfg(debug_assertions)]
	debug_utils_messenger: Option<vk::DebugUtilsMessengerEXT>,

	#[cfg(debug_assertions)]
	debug_data: Box<DebugCallbackData>,

	physical_device: vk::PhysicalDevice,
	device: ash::Device,
	queue_family_index: u32,
	queue: vk::Queue,
	swapchain: ash::extensions::khr::Swapchain,
	surface: ash::extensions::khr::Surface,
	acceleration_structure: ash::extensions::khr::AccelerationStructure,
	ray_tracing_pipeline: ash::extensions::khr::RayTracingPipeline,
	mesh_shading: ash::extensions::ext::MeshShader,

	#[cfg(debug_assertions)]
	debugger: RenderDebugger,

	frames: u8,

	buffers: Vec<Buffer>,
	textures: Vec<Texture>,
	allocations: Vec<Allocation>,
	descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	bindings: Vec<Binding>,
	descriptor_sets: Vec<DescriptorSet>,
	meshes: Vec<Mesh>,
	acceleration_structures: Vec<AccelerationStructure>,
	pipelines: Vec<Pipeline>,
	command_buffers: Vec<CommandBuffer>,
	synchronizers: Vec<Synchronizer>,
	swapchains: Vec<Swapchain>,
}

impl orchestrator::Entity for VulkanRenderSystem {}
impl orchestrator::System for VulkanRenderSystem {}

fn uses_to_vk_usage_flags(usage: render_system::Uses) -> vk::BufferUsageFlags {
	let mut flags = vk::BufferUsageFlags::empty();
	flags |= if usage.contains(render_system::Uses::Vertex) { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::Index) { vk::BufferUsageFlags::INDEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::Uniform) { vk::BufferUsageFlags::UNIFORM_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::Storage) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::TransferSource) { vk::BufferUsageFlags::TRANSFER_SRC } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::TransferDestination) { vk::BufferUsageFlags::TRANSFER_DST } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::AccelerationStructure) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::Indirect) { vk::BufferUsageFlags::INDIRECT_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::ShaderBindingTable) { vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::AccelerationStructureBuildScratch) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(render_system::Uses::AccelerationStructureBuild) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR } else { vk::BufferUsageFlags::empty() };
	flags
}

impl render_system::RenderSystem for VulkanRenderSystem {
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: render_system::Uses, resource_device_accesses: render_system::DeviceAccesses) -> render_system::AllocationHandle {
		self.create_allocation_internal(size, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[render_system::VertexElement]) -> render_system::MeshHandle {
		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(None, buffer_size, vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		unsafe {
			let vertex_buffer_pointer = pointer.expect("No pointer");
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size);
			let index_buffer_pointer = vertex_buffer_pointer.add(vertex_buffer_size.next_multiple_of(16));
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size);
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
				options.set_target_env(shaderc::TargetEnv::Vulkan, (1 << 22) | (3 << 12));
				options.set_generate_debug_info();
				options.set_target_spirv(shaderc::SpirvVersion::V1_6);
				options.set_invert_y(true);
		
				let shader_text = std::str::from_utf8(shader).unwrap();
		
				let binary = compiler.compile_into_spirv(shader_text, shaderc::ShaderKind::InferFromSource, "shader_name", "main", Some(&options));
				
				match binary {
					Ok(binary) => {		
						self.create_vulkan_shader(stage, binary.as_binary_u8())
					},
					Err(err) => {
						let error_string = err.to_string();
						let error_string = rendering::shader_compilation::format_glslang_error("shader_name:", &error_string, &shader_text).unwrap_or(error_string);

						error!("Error compiling shader:\n{}", error_string);
						panic!("Error compiling shader: {}", err);
					}
				}
			}
			render_system::ShaderSourceType::SPIRV => {
				self.create_vulkan_shader(stage, shader)
			}
		}
	}

	fn create_descriptor_set_template(&mut self, name: Option<&str>, bindings: &[render_system::DescriptorSetBindingTemplate]) -> render_system::DescriptorSetTemplateHandle {
		fn m(rs: &mut VulkanRenderSystem, bindings: &[render_system::DescriptorSetBindingTemplate], layout_bindings: &mut Vec<vk::DescriptorSetLayoutBinding>, map: &mut Vec<(vk::DescriptorType, u32)>) -> vk::DescriptorSetLayout {
			if let Some(binding) = bindings.get(0) {
				let b = vk::DescriptorSetLayoutBinding::default()
				.binding(binding.binding)
				.descriptor_type(match binding.descriptor_type {
					render_system::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
					render_system::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
					render_system::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
					render_system::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
					render_system::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
					render_system::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
					render_system::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
				})
				.descriptor_count(binding.descriptor_count)
				.stage_flags(binding.stages.into());

				let x = if let Some(inmutable_samplers) = &binding.immutable_samplers {
					inmutable_samplers.iter().map(|sampler| vk::Sampler::from_raw(sampler.0)).collect::<Vec<_>>()
				} else {
					Vec::new()
				};

				b.immutable_samplers(&x);

				map.push((b.descriptor_type, b.descriptor_count));

				layout_bindings.push(b);

				m(rs, &bindings[1..], layout_bindings, map)
			} else {
				let descriptor_set_layout_create_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(layout_bindings);
		
				let descriptor_set_layout = unsafe { rs.device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None).expect("No descriptor set layout") };

				descriptor_set_layout
			}
		}

		let mut bindings_list = Vec::with_capacity(8);

		let descriptor_set_layout = m(self, bindings, &mut Vec::new(), &mut bindings_list);

		unsafe{
			if let Some(name) = name {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils.set_debug_utils_object_name(
						self.device.handle(),
						&vk::DebugUtilsObjectNameInfoEXT::default()
							.object_handle(descriptor_set_layout)
							.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
							/* .build() */
					).expect("No debug utils object name");
				}
			}
		}

		let handle = render_system::DescriptorSetTemplateHandle(self.descriptor_sets_layouts.len() as u64);

		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings: bindings_list,
			descriptor_set_layout,
		});

		handle
	}

	fn create_descriptor_binding(&mut self, descriptor_set: render_system::DescriptorSetHandle, binding: &render_system::DescriptorSetBindingTemplate) -> render_system::DescriptorSetBindingHandle {
		let descriptor_type = match binding.descriptor_type {
			render_system::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
			render_system::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
			render_system::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
			render_system::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
			render_system::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
			render_system::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
			render_system::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
		};

		let created_binding = Binding {
			descriptor_set_handle: descriptor_set,
			descriptor_type,
			count: binding.descriptor_count,
			index: binding.binding,
		};

		let binding_handle = render_system::DescriptorSetBindingHandle(self.bindings.len() as u64);

		self.bindings.push(created_binding);

		binding_handle
	}

	fn create_descriptor_set(&mut self, name: Option<&str>, descriptor_set_layout_handle: &render_system::DescriptorSetTemplateHandle) -> render_system::DescriptorSetHandle {
		let pool_sizes = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize].bindings.iter().map(|(descriptor_type, descriptor_count)| {
			vk::DescriptorPoolSize::default()
				.ty(*descriptor_type)
				.descriptor_count(descriptor_count * self.frames as u32)
				/* .build() */
		})
		.collect::<Vec<_>>();

		let descriptor_pool_create_info = vk::DescriptorPoolCreateInfo::default()
			.max_sets(3/* LEAVE AS 3 AS THAT IS THE MAX AMOUNT OF BUFFERED FRAMES */)
			.pool_sizes(&pool_sizes);

		let descriptor_pool = unsafe { self.device.create_descriptor_pool(&descriptor_pool_create_info, None).expect("No descriptor pool") };

		let descriptor_set_layout = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize].descriptor_set_layout;

		// Allocate 2 descriptor sets from our pool.
		// TODO: Tie this count to the number of frames.
		let descriptor_set_layouts = [descriptor_set_layout, descriptor_set_layout];

		let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
			.descriptor_pool(descriptor_pool)
			.set_layouts(&descriptor_set_layouts)
			/* .build() */;

		let descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&descriptor_set_allocate_info).expect("No descriptor set") };

		let handle = render_system::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<render_system::DescriptorSetHandle> = None;

		for descriptor_set in descriptor_sets {
			let handle = render_system::DescriptorSetHandle(self.descriptor_sets.len() as u64);

			self.descriptor_sets.push(
				DescriptorSet {
					next: None,
					descriptor_set,
					descriptor_set_layout: *descriptor_set_layout_handle,
				}
			);

			if let Some(previous_handle) = previous_handle {
				self.descriptor_sets[previous_handle.0 as usize].next = Some(handle);
			}

			if let Some(name) = name {
				if let Some(debug_utils) = &self.debug_utils {
					unsafe {
						debug_utils.set_debug_utils_object_name(
							self.device.handle(),
							&vk::DebugUtilsObjectNameInfoEXT::default()
								.object_handle(descriptor_set)
								.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
								/* .build() */
						).expect("No debug utils object name");
					}
				}
			}

			previous_handle = Some(handle);
		}

		handle
	}

	fn write(&self, descriptor_set_writes: &[render_system::DescriptorWrite]) {		
		for descriptor_set_write in descriptor_set_writes {
			let binding = &self.bindings[descriptor_set_write.binding_handle.0 as usize];
			let descriptor_set = &self.descriptor_sets[binding.descriptor_set_handle.0 as usize];

			let layout = descriptor_set.descriptor_set_layout;

			let descriptor_set_layout = &self.descriptor_sets_layouts[layout.0 as usize];

			let descriptor_type = binding.descriptor_type;
			let binding_index = binding.index;

			match descriptor_set_write.descriptor {
				render_system::Descriptor::Buffer { handle, size } => {
					let mut descriptor_set_handle_option = Some(binding.descriptor_set_handle);
					let mut _buffer_handle_option = Some(handle);

					while let Some(descriptor_set_handle) = descriptor_set_handle_option {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						let buffer = &self.buffers[handle.0 as usize];

						let buffers = [vk::DescriptorBufferInfo::default().buffer(buffer.buffer).offset(0u64).range(match size { render_system::Ranges::Size(size) => { size as u64 } render_system::Ranges::Whole => { vk::WHOLE_SIZE } })];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.buffer_info(&buffers);

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };

						descriptor_set_handle_option = descriptor_set.next;

						// if let Some(_) = buffer.next { // If buffer spans multiple frames, write each frame's buffer to each frame's descriptor set. Else write the same buffer to each frame's descriptor set.
						// 	// buffer_handle_option = buffer.next;
						// }
					}
				},
				render_system::Descriptor::Image{ handle, layout } => {
					let mut descriptor_set_handle_option = Some(binding.descriptor_set_handle);
					let mut texture_handle_option = Some(handle);

					while let (Some(descriptor_set_handle), Some(texture_handle)) = (descriptor_set_handle_option, texture_handle_option) {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						let texture = &self.textures[texture_handle.0 as usize];

						let images = [
							vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(texture.format_, layout, None))
							.image_view(texture.image_view)
						];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&images);

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };

						descriptor_set_handle_option = descriptor_set.next;

						if let Some(_) = texture.next { // If texture spans multiple frames, write each frame's texture to each frame's descriptor set. Else write the same texture to each frame's descriptor set.
							texture_handle_option = texture.next;
						}
					}
				},
				render_system::Descriptor::CombinedImageSampler{ image_handle, sampler_handle, layout } => {
					let mut descriptor_set_handle_option = Some(binding.descriptor_set_handle);
					let mut texture_handle_option = Some(image_handle);

					while let (Some(descriptor_set_handle), Some(texture_handle)) = (descriptor_set_handle_option, texture_handle_option) {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						let texture = &self.textures[texture_handle.0 as usize];

						let images = [
							vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(texture.format_, layout, None))
							.image_view(texture.image_view)
							.sampler(vk::Sampler::from_raw(sampler_handle.0))
						];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&images);

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };

						descriptor_set_handle_option = descriptor_set.next;

						if let Some(_) = texture.next { // If texture spans multiple frames, write each frame's texture to each frame's descriptor set. Else write the same texture to each frame's descriptor set.
							texture_handle_option = texture.next;
						}
					}
				},
				render_system::Descriptor::Sampler(handle) => {
					let mut descriptor_set_handle_option = Some(binding.descriptor_set_handle);
					let sampler_handle_option = Some(handle);

					while let (Some(descriptor_set_handle), Some(sampler_handle)) = (descriptor_set_handle_option, sampler_handle_option) {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

						let images = [vk::DescriptorImageInfo::default().sampler(vk::Sampler::from_raw(sampler_handle.0))];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&images);

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };

						descriptor_set_handle_option = descriptor_set.next;
						// sampler_handle_option = sampler.next;
					}
				},
				render_system::Descriptor::Swapchain(handle) => {
					unimplemented!()
				}
				render_system::Descriptor::AccelerationStructure { handle } => {
					let mut descriptor_set_handle_option = Some(binding.descriptor_set_handle);
					let mut acceleration_structure_handle_option = Some(handle);

					while let (Some(descriptor_set_handle), Some(acceleration_structure_handle)) = (descriptor_set_handle_option, acceleration_structure_handle_option) {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						let acceleration_structure = &self.acceleration_structures[acceleration_structure_handle.0 as usize];

						let acceleration_structures = [acceleration_structure.acceleration_structure];

						let mut acc_str_descriptor_info = vk::WriteDescriptorSetAccelerationStructureKHR::default()
							.acceleration_structures(&acceleration_structures);

						let write_info = vk::WriteDescriptorSet{ descriptor_count: 1, ..vk::WriteDescriptorSet::default() }
							.push_next(&mut acc_str_descriptor_info)
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type);

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };

						descriptor_set_handle_option = descriptor_set.next;

						// if let Some(_) = acceleration_structure.next { // If acceleration structure spans multiple frames, write each frame's acceleration structure to each frame's descriptor set. Else write the same acceleration structure to each frame's descriptor set.
						// 	acceleration_structure_handle_option = acceleration_structure.next;
						// }
					}
				}
			}
		}
	}

	fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[render_system::DescriptorSetTemplateHandle], push_constant_ranges: &[render_system::PushConstantRange]) -> render_system::PipelineLayoutHandle {
		let push_constant_ranges = push_constant_ranges.iter().map(|push_constant_range| vk::PushConstantRange::default().size(push_constant_range.size).offset(push_constant_range.offset).stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE)).collect::<Vec<_>>();
		let set_layouts = descriptor_set_layout_handles.iter().map(|set_layout| self.descriptor_sets_layouts[set_layout.0 as usize].descriptor_set_layout).collect::<Vec<_>>();

  		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&set_layouts)
			.push_constant_ranges(&push_constant_ranges)
			/* .build() */;

		let pipeline_layout = unsafe { self.device.create_pipeline_layout(&pipeline_layout_create_info, None).expect("No pipeline layout") };

		render_system::PipelineLayoutHandle(pipeline_layout.as_raw())
	}

	fn create_raster_pipeline(&mut self, pipeline_blocks: &[render_system::PipelineConfigurationBlocks]) -> render_system::PipelineHandle {
		self.create_vulkan_pipeline(pipeline_blocks)
	}

	fn create_compute_pipeline(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, shader_parameter: render_system::ShaderParameter) -> render_system::PipelineHandle {
		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);

		let mut spcialization_map_entries = Vec::with_capacity(48);
		
		for specialization_map_entry in shader_parameter.2 {
			match specialization_map_entry.get_type().as_str() {
				"vec4f" => {
					for i in 0..4 {
						spcialization_map_entries.push(vk::SpecializationMapEntry::default()
						.constant_id(specialization_map_entry.get_constant_id() + i)
						.offset(specialization_entries_buffer.len() as u32 + i * 4)
						.size(specialization_map_entry.get_size() / 4));
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				_ => {
					spcialization_map_entries.push(vk::SpecializationMapEntry::default()
						.constant_id(specialization_map_entry.get_constant_id())
						.offset(specialization_entries_buffer.len() as u32)
						.size(specialization_map_entry.get_size()));
		
					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
			}
		}

		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&spcialization_map_entries);

		let create_infos = [
			vk::ComputePipelineCreateInfo::default()
				.stage(vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(vk::ShaderModule::from_raw(shader_parameter.0.0))
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					.specialization_info(&specialization_info)
					/* .build() */
				)
				.layout(vk::PipelineLayout::from_raw(pipeline_layout_handle.0))
		];

		let pipeline_handle = unsafe {
			self.device.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None).expect("No compute pipeline")[0]
		};

		let handle = render_system::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			shader_handles: HashMap::new(),
		});

		handle
	}

	fn create_ray_tracing_pipeline(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, shaders: &[render_system::ShaderParameter]) -> render_system::PipelineHandle {
		let mut groups = Vec::with_capacity(1024);
		
		let stages = shaders.iter().map(|shader| {
			vk::PipelineShaderStageCreateInfo::default()
				.stage(to_shader_stage_flags(shader.1))
				.module(vk::ShaderModule::from_raw(shader.0.0))
				.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
				// .specialization_info(&specilization_infos[specilization_info_count - 1])
				/* .build() */
		}).collect::<Vec<_>>();

		for (i, shader) in shaders.iter().enumerate() {
			match shader.1 {
				render_system::ShaderTypes::Raygen | render_system::ShaderTypes::Miss | render_system::ShaderTypes::Callable => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
						.general_shader(i as u32)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				render_system::ShaderTypes::ClosestHit => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(i as u32)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				render_system::ShaderTypes::AnyHit => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(i as u32)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				render_system::ShaderTypes::Intersection => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(i as u32));
				}
				_ => {
					warn!("Fed shader of type '{:?}' to ray tracing pipeline", shader.1)
				}
			}
		}

		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.layout(vk::PipelineLayout::from_raw(pipeline_layout_handle.0))
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let mut handles: HashMap<ShaderHandle, [u8; 32]> = HashMap::with_capacity(shaders.len());

		let pipeline_handle = unsafe {
			let pipeline = self.ray_tracing_pipeline.create_ray_tracing_pipelines(vk::DeferredOperationKHR::null(), vk::PipelineCache::null(), &[create_info], None).expect("No ray tracing pipeline")[0];
			let handle_buffer = self.ray_tracing_pipeline.get_ray_tracing_shader_group_handles(pipeline, 0, groups.len() as u32, 32 * groups.len()).expect("Could not get ray tracing shader group handles");

			for (i, shader) in shaders.iter().enumerate() {
				let mut h = [0u8; 32];
				h.copy_from_slice(&handle_buffer[i * 32..(i + 1) * 32]);

				handles.insert(*shader.0, h);
			}

			pipeline
		};

		let handle = render_system::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			shader_handles: handles,
		});

		handle
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
	fn create_buffer(&mut self, name: Option<&str>, size: usize, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, use_case: render_system::UseCases) -> render_system::BaseBufferHandle {
		if device_accesses.contains(render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead) {
			match use_case {
				render_system::UseCases::STATIC => {
					let buffer_creation_result = self.create_vulkan_buffer(name, size, uses_to_vk_usage_flags(resource_uses) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

					let buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size,
						device_address,
						pointer,
					});
					
					buffer_handle
				}
				render_system::UseCases::DYNAMIC => {	
					let buffer_creation_result = self.create_vulkan_buffer(name, size, uses_to_vk_usage_flags(resource_uses) | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
	
					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);
	
					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);
	
					let buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size,
						device_address,
						pointer,
					});

					let buffer_creation_result = self.create_vulkan_buffer(name, size, uses_to_vk_usage_flags(resource_uses) | vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

					let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

					let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

					self.buffers.push(Buffer {
						buffer: buffer_creation_result.resource,
						size,
						device_address,
						pointer,
					});

					buffer_handle
				}
			}
		} else if device_accesses.contains(render_system::DeviceAccesses::GpuWrite) {
			let buffer_creation_result = self.create_vulkan_buffer(name, size, uses_to_vk_usage_flags(resource_uses) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

			let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, device_accesses);

			let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			let buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

			self.buffers.push(Buffer {
				buffer: buffer_creation_result.resource,
				size,
				device_address,
				pointer,
			});

			buffer_handle
		} else {
			panic!("Invalid device accesses");
		}
	}

	fn get_buffer_address(&self, buffer_handle: render_system::BaseBufferHandle) -> u64 {
		self.buffers[buffer_handle.0 as usize].device_address
	}

	fn get_buffer_slice(&mut self, buffer_handle: render_system::BaseBufferHandle) -> &[u8] {
		let buffer = self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts(buffer.pointer, buffer.size)
		}
	}

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice(&self, buffer_handle: render_system::BaseBufferHandle) -> &mut [u8] {
		let buffer = self.buffers[buffer_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, buffer.size)
		}
	}

	fn get_texture_slice_mut(&self, texture_handle: render_system::ImageHandle) -> &mut [u8] {
		let texture = &self.textures[texture_handle.0 as usize];
		unsafe {
			std::slice::from_raw_parts_mut(texture.pointer as *mut u8, texture.size)
		}
	}

	/// Creates a texture.
	fn create_image(&mut self, name: Option<&str>, extent: crate::Extent, format: render_system::Formats, compression: Option<render_system::CompressionSchemes>, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, use_case: render_system::UseCases) -> render_system::ImageHandle {
		let size = (extent.width * extent.height * extent.depth * 4) as usize;

		let texture_handle = render_system::ImageHandle(self.textures.len() as u64);

		let mut previous_texture_handle: Option<render_system::ImageHandle> = None;

		let extent = vk::Extent3D::default().width(extent.width).height(extent.height).depth(extent.depth);

		for _ in 0..(match use_case { render_system::UseCases::DYNAMIC => { self.frames } render_system::UseCases::STATIC => { 1 }}) {
			let resource_uses = if resource_uses.contains(render_system::Uses::Image) {
				resource_uses | render_system::Uses::TransferDestination
			} else {
				resource_uses
			};

			let texture_creation_result = self.create_vulkan_texture(name, extent, format, compression, resource_uses | render_system::Uses::TransferSource, device_accesses, render_system::AccessPolicies::WRITE, 1);

			let (allocation_handle, pointer) = self.create_allocation_internal(texture_creation_result.size, device_accesses);

			let (address, pointer) = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);

			let texture_handle = render_system::ImageHandle(self.textures.len() as u64);

			let image_view = self.create_vulkan_texture_view(name, &texture_creation_result.resource, format, compression, 0);

			let (staging_buffer, pointer) = if device_accesses.contains(render_system::DeviceAccesses::CpuRead) {
				let staging_buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

				let (allocation_handle, pointer) = self.create_allocation_internal(staging_buffer_creation_result.size, render_system::DeviceAccesses::CpuRead);

				let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);

				let staging_buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

				self.buffers.push(Buffer {
					buffer: staging_buffer_creation_result.resource,
					size: staging_buffer_creation_result.size,
					device_address: address,
					pointer,
				});

				(Some(staging_buffer_handle), pointer)
			} else if device_accesses.contains(render_system::DeviceAccesses::CpuWrite) {
				let staging_buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

				let (allocation_handle, pointer) = self.create_allocation_internal(staging_buffer_creation_result.size, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

				let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);

				let staging_buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

				self.buffers.push(Buffer {
					buffer: staging_buffer_creation_result.resource,
					size: staging_buffer_creation_result.size,
					device_address: address,
					pointer,
				});

				(Some(staging_buffer_handle), pointer)
			} else {
				(None, std::ptr::null_mut())
			};

			self.textures.push(Texture {
				next: None,
				size: texture_creation_result.size,
				staging_buffer,
				allocation_handle,
				image: texture_creation_result.resource,
				image_view,
				pointer,
				extent,
				format: to_format(format, compression),
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

	fn create_acceleration_structure_instance_buffer(&mut self, name: Option<&str>, max_instance_count: u32) -> render_system::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

		let buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let buffer_handle = render_system::BaseBufferHandle(self.buffers.len() as u64);

		self.buffers.push(Buffer {
			buffer: buffer_creation_result.resource,
			size: buffer_creation_result.size,
			device_address: address,
			pointer,
		});

		buffer_handle
	}

	fn create_top_level_acceleration_structure(&mut self, name: Option<&str>,) -> render_system::TopLevelAccelerationStructureHandle {
		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR { instances: vk::AccelerationStructureGeometryInstancesDataKHR::default() });

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &build_info, &[0], &mut size_info);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let scratch_size = size_info.build_scratch_size as usize;

		let buffer = self.create_vulkan_buffer(None, acceleration_structure_size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer.size, render_system::DeviceAccesses::GpuWrite);

		let (_, _) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);

		let handle = render_system::TopLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure.create_acceleration_structure(&create_info, None).expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer.resource,
				scratch_size,
			});

			if let Some(name) = name {
				if let Some(debug_utils) = &self.debug_utils {
					unsafe {
						debug_utils.set_debug_utils_object_name(
							self.device.handle(),
							&vk::DebugUtilsObjectNameInfoEXT::default()
								.object_handle(handle)
								.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
								/* .build() */
						).expect("No debug utils object name");
					}
				}
			}
		}

		handle
	}

	fn create_bottom_level_acceleration_structure(&mut self, description: &render_system::BottomLevelAccelerationStructure,) -> render_system::BottomLevelAccelerationStructureHandle {
		let (geometry, primitive_count) = match &description.description {
			render_system::BottomLevelAccelerationStructureDescriptions::Mesh { vertex_count, vertex_position_encoding, triangle_count, index_format } => {
				(vk::AccelerationStructureGeometryKHR::default()
					.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_format(match vertex_position_encoding {
								render_system::Encodings::IEEE754 => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position format"),
							})
							.max_vertex(*vertex_count - 1)
							.index_type(match index_format {
								render_system::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								render_system::DataTypes::U16 => vk::IndexType::UINT16,
								render_system::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
					}),
				*triangle_count)
			}
			render_system::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				(vk::AccelerationStructureGeometryKHR::default()
					.geometry_type(vk::GeometryTypeKHR::AABBS)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default()
					}),
				*transform_count)
			}
		};

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &build_info, &[primitive_count], &mut size_info);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let scratch_size = size_info.build_scratch_size as usize;

		let buffer_descriptor = self.create_vulkan_buffer(None, acceleration_structure_size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer_descriptor.size, render_system::DeviceAccesses::GpuWrite);

		let (_, _) = self.bind_vulkan_buffer_memory(&buffer_descriptor, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer_descriptor.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

		let handle = render_system::BottomLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure.create_acceleration_structure(&create_info, None).expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer_descriptor.resource,
				scratch_size,
			});

			// if let Some(name) = None {
			// 	if let Some(debug_utils) = &self.debug_utils {
			// 		unsafe {
			// 			debug_utils.set_debug_utils_object_name(
			// 				self.device.handle(),
			// 				&vk::DebugUtilsObjectNameInfoEXT::default()
			// 					.object_handle(handle)
			// 					.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
			// 					/* .build() */
			// 			).expect("No debug utils object name");
			// 		}
			// 	}
			// }
		}

		handle
	}

	fn write_instance(&mut self, instances_buffer: BaseBufferHandle, transform: [[f32; 4]; 3], custom_index: u16, mask: u8, sbt_record_offset: usize, acceleration_structure: render_system::BottomLevelAccelerationStructureHandle) {
		let buffer = self.acceleration_structures[acceleration_structure.0 as usize].buffer;

		let address = unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) };

		let instance = vk::AccelerationStructureInstanceKHR{
			transform: vk::TransformMatrixKHR {
				matrix: [transform[0][0], transform[0][1], transform[0][2], transform[0][3], transform[1][0], transform[1][1], transform[1][2], transform[1][3], transform[2][0], transform[2][1], transform[2][2], transform[2][3]],
			},
			instance_custom_index_and_mask: vk::Packed24_8::new(custom_index as u32, mask),
			instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(sbt_record_offset as u32, 0),
			acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
				device_handle: address,
			},
		};

		let instance_buffer = &mut self.buffers[instances_buffer.0 as usize];

		let instance_buffer_slice = unsafe { std::slice::from_raw_parts_mut(instance_buffer.pointer as *mut vk::AccelerationStructureInstanceKHR, instance_buffer.size / std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) };

		instance_buffer_slice[0] = instance;
	}

	fn write_sbt_entry(&mut self, sbt_buffer_handle: BaseBufferHandle, sbt_record_offset: usize, pipeline_handle: render_system::PipelineHandle, shader_handle: render_system::ShaderHandle) {
		let slice = self.get_mut_buffer_slice(sbt_buffer_handle);

		let pipeline = &self.pipelines[pipeline_handle.0 as usize];

		assert!(slice.as_ptr().is_aligned_to(64));

		slice[sbt_record_offset..sbt_record_offset + 32].copy_from_slice(pipeline.shader_handles.get(&shader_handle).unwrap())
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

	fn get_image_data(&self, texture_copy_handle: render_system::TextureCopyHandle) -> &[u8] {
		// let texture = self.textures.iter().find(|texture| texture.parent.map_or(false, |x| texture_handle == x)).unwrap(); // Get the proxy texture
		let texture = &self.textures[texture_copy_handle.0 as usize];
		let buffer_handle = texture.staging_buffer.expect("No staging buffer");
		let buffer = &self.buffers[buffer_handle.0 as usize];
		if buffer.pointer.is_null() { panic!("Texture data was requested but texture has no memory associated."); }
		let slice = unsafe { std::slice::from_raw_parts::<'static, u8>(buffer.pointer, (texture.extent.width * texture.extent.height * texture.extent.depth) as usize) };
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

use ash::{vk::{ValidationFeatureEnableEXT, Handle}, Entry};

use super::render_system::{CommandBufferRecording, BaseBufferHandle, RenderSystem, ShaderHandle};

#[derive(Clone)]
pub(crate) struct Swapchain {
	surface: vk::SurfaceKHR,
	surface_present_mode: vk::PresentModeKHR,
	swapchain: vk::SwapchainKHR,
}

#[derive(Clone)]
pub(crate) struct DescriptorSetLayout {
	bindings: Vec<(vk::DescriptorType, u32)>,
	descriptor_set_layout: vk::DescriptorSetLayout,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorSet {
	next: Option<render_system::DescriptorSetHandle>,
	descriptor_set: vk::DescriptorSet,
	descriptor_set_layout: render_system::DescriptorSetTemplateHandle,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	shader_handles: HashMap<ShaderHandle, [u8; 32]>,
}

#[derive(Clone, Copy)]
pub(crate) struct CommandBufferInternal {
	command_pool: vk::CommandPool,
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone)]
pub(crate) struct Binding {
	descriptor_set_handle: render_system::DescriptorSetHandle,
	descriptor_type: vk::DescriptorType,
	index: u32,
	count: u32,
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
	next: Option<render_system::ImageHandle>,
	staging_buffer: Option<render_system::BaseBufferHandle>,
	allocation_handle: render_system::AllocationHandle,
	image: vk::Image,
	image_view: vk::ImageView,
	pointer: *const u8,
	extent: vk::Extent3D,
	format: vk::Format,
	format_: render_system::Formats,
	layout: vk::ImageLayout,
	size: usize,
}

unsafe impl Send for Texture {}

// #[derive(Clone, Copy)]
// pub(crate) struct AccelerationStructure {
// 	acceleration_structure: vk::AccelerationStructureKHR,
// }

unsafe extern "system" fn vulkan_debug_utils_callback(message_severity: vk::DebugUtilsMessageSeverityFlagsEXT, _message_type: vk::DebugUtilsMessageTypeFlagsEXT, p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT, p_user_data: *mut std::ffi::c_void,) -> vk::Bool32 {
    let message = std::ffi::CStr::from_ptr((*p_callback_data).p_message);

	match message_severity {
		vk::DebugUtilsMessageSeverityFlagsEXT::INFO => {
			debug!("{}", message.to_str().unwrap());
		}
		vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
			warn!("{}", message.to_str().unwrap());
		}
		vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
			error!("{}", message.to_str().unwrap());
			(*(p_user_data as *mut DebugCallbackData)).error_count += 1;
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

fn texture_format_and_resource_use_to_image_layout(_texture_format: render_system::Formats, layout: render_system::Layouts, access: Option<render_system::AccessPolicies>) -> vk::ImageLayout {
	match layout {
		render_system::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		render_system::Layouts::RenderTarget => if _texture_format != render_system::Formats::Depth32 { vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL } else { vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL },
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
		render_system::Layouts::Read => vk::ImageLayout::READ_ONLY_OPTIMAL,
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

fn to_format(format: render_system::Formats, compression: Option<render_system::CompressionSchemes>) -> vk::Format {
	match format {
		render_system::Formats::RGBAu8 => {
			if let Some(compression) = compression {
				match compression {
					render_system::CompressionSchemes::BC7 => vk::Format::BC7_SRGB_BLOCK,
				}
			} else {
				vk::Format::R8G8B8A8_UNORM
			}
		}
		render_system::Formats::RGBAu16 => vk::Format::R16G16B16A16_SFLOAT,
		render_system::Formats::RGBAu32 => vk::Format::R32G32B32A32_SFLOAT,
		render_system::Formats::RGBAf16 => vk::Format::R16G16B16A16_SFLOAT,
		render_system::Formats::RGBAf32 => vk::Format::R32G32B32A32_SFLOAT,
		render_system::Formats::RGBu10u10u11 => vk::Format::R16G16_S10_5_NV,
		render_system::Formats::BGRAu8 => vk::Format::B8G8R8A8_SRGB,
		render_system::Formats::Depth32 => vk::Format::D32_SFLOAT,
		render_system::Formats::U32 => vk::Format::R32_UINT,
	}
}

fn to_shader_stage_flags(shader_type: render_system::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		render_system::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		render_system::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		render_system::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,
		render_system::ShaderTypes::Task => vk::ShaderStageFlags::TASK_EXT,
		render_system::ShaderTypes::Mesh => vk::ShaderStageFlags::MESH_EXT,
		render_system::ShaderTypes::Raygen => vk::ShaderStageFlags::RAYGEN_KHR,
		render_system::ShaderTypes::ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
		render_system::ShaderTypes::AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
		render_system::ShaderTypes::Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
		render_system::ShaderTypes::Miss => vk::ShaderStageFlags::MISS_KHR,
		render_system::ShaderTypes::Callable => vk::ShaderStageFlags::CALLABLE_KHR,
	}
}

fn to_pipeline_stage_flags(stages: render_system::Stages) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(render_system::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER
	}

	if stages.contains(render_system::Stages::MESH) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::MESH_SHADER_EXT;
	}

	if stages.contains(render_system::Stages::FRAGMENT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
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

	if stages.contains(render_system::Stages::INDIRECT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT;
	}

	if stages.contains(render_system::Stages::RAYGEN) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}

	pipeline_stage_flags
}

fn to_pipeline_stage_flags_with_format(stages: render_system::Stages, format: render_system::Formats, access: render_system::AccessPolicies) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(render_system::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER
	}

	if stages.contains(render_system::Stages::FRAGMENT) {
		if format != render_system::Formats::Depth32 {
			if access.contains(render_system::AccessPolicies::READ) {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}

			if access.contains(render_system::AccessPolicies::WRITE) {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
			}
		} else {
			pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
		}
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

	if stages.contains(render_system::Stages::INDIRECT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT;
	}

	pipeline_stage_flags
}

fn to_access_flags(accesses: render_system::AccessPolicies, stages: render_system::Stages,) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(render_system::AccessPolicies::READ) {
		if stages.intersects(render_system::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(render_system::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(render_system::Stages::FRAGMENT) {
			access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ;
		}
		if stages.intersects(render_system::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_READ
		}
		if stages.intersects(render_system::Stages::INDIRECT) {
			access_flags |= vk::AccessFlags2::INDIRECT_COMMAND_READ
		}
		if stages.intersects(render_system::Stages::RAYGEN) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
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

fn to_access_flags_with_format(accesses: render_system::AccessPolicies, stages: render_system::Stages, format: render_system::Formats) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(render_system::AccessPolicies::READ) {
		if stages.intersects(render_system::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(render_system::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(render_system::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_READ
		}
		if stages.intersects(render_system::Stages::FRAGMENT) {
			if format != render_system::Formats::Depth32 {
				access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
			} else {
				access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
			}
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
			if format != render_system::Formats::Depth32 {
				access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
			} else {
				access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
			}
		}
	}

	access_flags
}

impl Into<vk::ShaderStageFlags> for render_system::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		shader_stage_flags |= if self.intersects(render_system::Stages::VERTEX) { vk::ShaderStageFlags::VERTEX } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::FRAGMENT) { vk::ShaderStageFlags::FRAGMENT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::COMPUTE) { vk::ShaderStageFlags::COMPUTE } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::MESH) { vk::ShaderStageFlags::MESH_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::TASK) { vk::ShaderStageFlags::TASK_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::RAYGEN) { vk::ShaderStageFlags::RAYGEN_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::CLOSEST_HIT) { vk::ShaderStageFlags::CLOSEST_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::ANY_HIT) { vk::ShaderStageFlags::ANY_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::INTERSECTION) { vk::ShaderStageFlags::INTERSECTION_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::MISS) { vk::ShaderStageFlags::MISS_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(render_system::Stages::CALLABLE) { vk::ShaderStageFlags::CALLABLE_KHR } else { vk::ShaderStageFlags::default() };

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
			render_system::DataTypes::U8 => vk::Format::R8_UINT,
			render_system::DataTypes::U16 => vk::Format::R16_UINT,
			render_system::DataTypes::Int => vk::Format::R32_SINT,
			render_system::DataTypes::U32 => vk::Format::R32_UINT,
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
			render_system::DataTypes::U8 => std::mem::size_of::<u8>(),
			render_system::DataTypes::U16 => std::mem::size_of::<u16>(),
			render_system::DataTypes::U32 => std::mem::size_of::<u32>(),
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

pub struct Settings {
	validation: bool,
	ray_tracing: bool,
}

struct DebugCallbackData {
	error_count: u64,
}

impl VulkanRenderSystem {
	pub fn new(settings: &Settings) -> VulkanRenderSystem {
		let entry: ash::Entry = Entry::linked();

		let application_info = vk::ApplicationInfo::default()
			.api_version(vk::make_api_version(0, 1, 3, 0));

		let mut layer_names = Vec::new();
		
		if settings.validation {
			layer_names.push(std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap().as_ptr());
		}

		let mut extension_names = Vec::new();
		
		extension_names.push(ash::extensions::khr::Surface::NAME.as_ptr());

		#[cfg(target_os = "linux")]
		extension_names.push(ash::extensions::khr::XcbSurface::NAME.as_ptr());

		if settings.validation {
			extension_names.push(ash::extensions::ext::DebugUtils::NAME.as_ptr());
		}

		let enabled_validation_features = [
			ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION,
			ValidationFeatureEnableEXT::BEST_PRACTICES,
			// ValidationFeatureEnableEXT::GPU_ASSISTED,
			// ValidationFeatureEnableEXT::GPU_ASSISTED_RESERVE_BINDING_SLOT,
			// ValidationFeatureEnableEXT::DEBUG_PRINTF,
		];

		let mut validation_features = vk::ValidationFeaturesEXT::default()
			.enabled_validation_features(&enabled_validation_features);

		let instance_create_info = vk::InstanceCreateInfo::default()
			.application_info(&application_info)
			.enabled_layer_names(&layer_names)
			.enabled_extension_names(&extension_names)
			/* .build() */;

		let instance_create_info = if settings.validation {
			instance_create_info.push_next(&mut validation_features)
		} else {
			instance_create_info
		};

		let instance = unsafe { entry.create_instance(&instance_create_info, None).expect("No instance") };

		let mut debug_data = Box::new(DebugCallbackData {
			error_count: 0,
		});

		let (debug_utils, debug_utils_messenger) = if settings.validation {
			let debug_utils = ash::extensions::ext::DebugUtils::new(&entry, &instance);

			let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
				.message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::INFO | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,)
				.message_type(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,)
				.pfn_user_callback(Some(vulkan_debug_utils_callback))
				.user_data(debug_data.as_mut() as *mut DebugCallbackData as *mut std::ffi::c_void);
	
			let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).expect("Debug Utils Callback") };

			(Some(debug_utils), Some(debug_utils_messenger))
		} else {
			(None, None)
		};

		let physical_devices = unsafe { instance.enumerate_physical_devices().expect("No physical devices.") };

		let physical_device;

		{
			let best_physical_device = physical_devices.iter().max_by_key(|physical_device| {
				let properties = unsafe { instance.get_physical_device_properties(*(*physical_device)) };
				let features = unsafe { instance.get_physical_device_features(*(*physical_device)) };

				// If the device doesn't support sample rate shading, don't even consider it.
				if features.sample_rate_shading == vk::FALSE { return 0; }

				let mut device_score = 0u64;

				device_score += if features.shader_storage_image_array_dynamic_indexing == vk::TRUE { 1 } else { 0 };
				device_score += if features.shader_sampled_image_array_dynamic_indexing == vk::TRUE { 1 } else { 0 };
				device_score += if features.shader_storage_buffer_array_dynamic_indexing == vk::TRUE { 1 } else { 0 };
				device_score += if features.shader_uniform_buffer_array_dynamic_indexing == vk::TRUE { 1 } else { 0 };

				device_score += if features.shader_storage_image_write_without_format == vk::TRUE { 1 } else { 0 };

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
			.find_map(|(index, info)| {
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

		let mut device_extension_names = Vec::new();
		
		device_extension_names.push(ash::extensions::khr::Swapchain::NAME.as_ptr());

		if settings.ray_tracing {
			device_extension_names.push(ash::extensions::khr::AccelerationStructure::NAME.as_ptr());
			device_extension_names.push(ash::extensions::khr::DeferredHostOperations::NAME.as_ptr());
			device_extension_names.push(ash::extensions::khr::RayTracingPipeline::NAME.as_ptr());
			device_extension_names.push(ash::extensions::khr::RayTracingMaintenance1::NAME.as_ptr());
		}

		device_extension_names.push(ash::extensions::ext::MeshShader::NAME.as_ptr());

		let queue_create_infos = [vk::DeviceQueueCreateInfo::default()
			.queue_family_index(queue_family_index)
			.queue_priorities(&[1.0])
			/* .build() */];

		let mut physical_device_vulkan_11_features = vk::PhysicalDeviceVulkan11Features::default()
			.uniform_and_storage_buffer16_bit_access(true)
			.storage_buffer16_bit_access(true)
		;

		let mut physical_device_vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default()
			.descriptor_indexing(true).descriptor_binding_partially_bound(true).runtime_descriptor_array(true)
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

		let mut physical_device_mesh_shading_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default()
			.task_shader(true)
			.mesh_shader(true);

		let (mut physical_device_acceleration_structure_features, mut physical_device_ray_tracing_pipeline_features) = if settings.ray_tracing {
			let physical_device_acceleration_structure_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
				.acceleration_structure(true);

			let physical_device_ray_tracing_pipeline_features = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default()
				.ray_tracing_pipeline(true)
				.ray_traversal_primitive_culling(true);

			(physical_device_acceleration_structure_features, physical_device_ray_tracing_pipeline_features)
		} else {
			(vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default(), vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default())
		};

  		let device_create_info = vk::DeviceCreateInfo::default()
			.push_next(&mut physical_device_vulkan_11_features/* .build() */)
			.push_next(&mut physical_device_vulkan_12_features/* .build() */)
			.push_next(&mut physical_device_vulkan_13_features/* .build() */)
			.push_next(&mut physical_device_mesh_shading_features/* .build() */)
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names)
			.enabled_features(&enabled_physical_device_features/* .build() */)
			/* .build() */;

		let device_create_info = if settings.ray_tracing {
			device_create_info
				.push_next(&mut physical_device_acceleration_structure_features)
				.push_next(&mut physical_device_ray_tracing_pipeline_features)
		} else {
			device_create_info
		};

		let device: ash::Device = unsafe { instance.create_device(physical_device, &device_create_info, None).expect("No device") };

		let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

		let acceleration_structure = ash::extensions::khr::AccelerationStructure::new(&instance, &device);
		let ray_tracing_pipeline = ash::extensions::khr::RayTracingPipeline::new(&instance, &device);

		let swapchain = ash::extensions::khr::Swapchain::new(&instance, &device);
		let surface = ash::extensions::khr::Surface::new(&entry, &instance);

		let mesh_shading = ash::extensions::ext::MeshShader::new(&instance, &device);

		VulkanRenderSystem { 
			entry,
			instance,

			debug_utils,
			debug_utils_messenger,
			debug_data,

			physical_device,
			device,
			queue_family_index,
			queue,
			swapchain,
			surface,
			acceleration_structure,
			ray_tracing_pipeline,
			mesh_shading,

			debugger: RenderDebugger::new(),

			frames: 2, // Assuming double buffering

			allocations: Vec::new(),
			buffers: Vec::new(),
			textures: Vec::new(),
			descriptor_sets_layouts: Vec::new(),
			bindings: Vec::new(),
			descriptor_sets: Vec::new(),
			acceleration_structures: Vec::new(),
			pipelines: Vec::new(),
			meshes: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers: Vec::new(),
			swapchains: Vec::new(),
		}
	}

	pub fn new_as_system() -> orchestrator::EntityReturn<render_system::RenderSystemImplementation> {
		let settings = Settings {
			validation: true,
			ray_tracing: true,
		};
		orchestrator::EntityReturn::new(render_system::RenderSystemImplementation::new(Box::new(VulkanRenderSystem::new(&settings))))
	}

	fn get_log_count(&self) -> u64 { self.debug_data.error_count }

	fn create_vulkan_shader(&self, stage: render_system::ShaderTypes, shader: &[u8]) -> render_system::ShaderHandle {
		let shader_module_create_info = vk::ShaderModuleCreateInfo::default()
			.code(unsafe { shader.align_to::<u32>().1 })
			/* .build() */;

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).expect("No shader module") };

		render_system::ShaderHandle(shader_module.as_raw())
	}

	fn create_vulkan_pipeline(&mut self, blocks: &[render_system::PipelineConfigurationBlocks]) -> render_system::PipelineHandle {
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

						build_block(vulkan_render_system, pipeline_create_info, block_iterator)
					}
					render_system::PipelineConfigurationBlocks::InputAssembly {  } => {
						let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
							.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
							.primitive_restart_enable(false);

						let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

						build_block(vulkan_render_system, pipeline_create_info, block_iterator)
					}
					render_system::PipelineConfigurationBlocks::RenderTargets { targets } => {
						let pipeline_color_blend_attachments = targets.iter().filter(|a| a.format != render_system::Formats::Depth32).map(|_| {
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
	
						let color_attachement_formats: Vec<vk::Format> = targets.iter().filter(|a| a.format != render_system::Formats::Depth32).map(|a| to_format(a.format, None)).collect::<Vec<_>>();

						let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
							.logic_op_enable(false)
							.logic_op(vk::LogicOp::COPY)
							.attachments(&pipeline_color_blend_attachments)
							.blend_constants([0.0, 0.0, 0.0, 0.0]);

						let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
							.color_attachment_formats(&color_attachement_formats)
							.depth_attachment_format(vk::Format::UNDEFINED);

						let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

						if let Some(_) = targets.iter().find(|a| a.format == render_system::Formats::Depth32) {
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

							build_block(vulkan_render_system, pipeline_create_info, block_iterator)
						} else {
							let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);

							build_block(vulkan_render_system, pipeline_create_info, block_iterator)
						}
					}
					render_system::PipelineConfigurationBlocks::Shaders { shaders } => {
						let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);
						let mut entries = [vk::SpecializationMapEntry::default(); 32];
						let mut entry_count = 0;
						let mut specilization_infos = [vk::SpecializationInfo::default(); 16];
						let mut specilization_info_count = 0;

						let stages = shaders
							.iter()
							.map(move |shader| {
								let entries_offset = entry_count;

								for entry in &shader.2 {
									specialization_entries_buffer.extend_from_slice(entry.get_data());

									entries[entry_count] = vk::SpecializationMapEntry::default()
										.constant_id(entry.get_constant_id())
										.size(entry.get_size())
										.offset(specialization_entries_buffer.len() as u32);

									entry_count += 1;
								}

								// specilization_infos[{ let c = specilization_info_count; specilization_info_count += 1; c }] = vk::SpecializationInfo::default()
								// 	.data(&specialization_entries_buffer)
								// 	.map_entries(&entries[entries_offset..entry_count]);

								vk::PipelineShaderStageCreateInfo::default()
									.stage(to_shader_stage_flags(shader.1))
									.module(vk::ShaderModule::from_raw(shader.0.0))
									.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
									// .specialization_info(&specilization_infos[specilization_info_count - 1])
									/* .build() */
							})
							.collect::<Vec<_>>();

						let pipeline_create_info = pipeline_create_info.stages(&stages);

						build_block(vulkan_render_system, pipeline_create_info, block_iterator)
					}
					render_system::PipelineConfigurationBlocks::Layout { layout } => {
						let pipeline_layout = vk::PipelineLayout::from_raw(layout.0);

						let pipeline_create_info = pipeline_create_info.layout(pipeline_layout);

						build_block(vulkan_render_system, pipeline_create_info, block_iterator)
					}
				}
			} else {
				let pipeline_create_infos = [pipeline_create_info];

				let pipelines = unsafe { vulkan_render_system.device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None).expect("No pipeline") };

				pipelines[0]
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
			.cull_mode(vk::CullModeFlags::NONE)
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

		let handle = render_system::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline { pipeline, shader_handles: HashMap::new() });

		handle
	}

	fn create_texture_internal(&mut self, texture: Texture, previous: Option<render_system::ImageHandle>) -> render_system::ImageHandle {
		let texture_handle = render_system::ImageHandle(self.textures.len() as u64);

		self.textures.push(texture);

		if let Some(previous_texture_handle) = previous {
			self.textures[previous_texture_handle.0 as usize].next = Some(texture_handle);
		}

		texture_handle
	}

	fn create_vulkan_buffer(&self, name: Option<&str>, size: usize, usage: vk::BufferUsageFlags) -> MemoryBackedResourceCreationResult<vk::Buffer> {
		let buffer_create_info = vk::BufferCreateInfo::default()
			.size(size as u64)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(usage);

		let buffer = unsafe { self.device.create_buffer(&buffer_create_info, None).expect("No buffer") };

		if let Some(name) = name {
			unsafe {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils.set_debug_utils_object_name(
						self.device.handle(),
						&vk::DebugUtilsObjectNameInfoEXT::default()
							.object_handle(buffer)
							.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
							/* .build() */
					).expect("No debug utils object name");
				}
			}
		}

		let memory_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

		MemoryBackedResourceCreationResult {
			resource: buffer,
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
		}
	}

	fn destroy_vulkan_buffer(&self, buffer: &render_system::BaseBufferHandle) {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.destroy_buffer(buffer, None) };
	}

	fn create_vulkan_allocation(&self, size: usize,) -> vk::DeviceMemory {
		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(0)
			/* .build() */;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		memory
	}

	fn get_vulkan_buffer_address(&self, buffer: &render_system::BaseBufferHandle, _allocation: &render_system::AllocationHandle) -> u64 {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) }
	}

	fn create_vulkan_texture(&self, name: Option<&str>, extent: vk::Extent3D, format: render_system::Formats, compression: Option<render_system::CompressionSchemes>, resource_uses: render_system::Uses, device_accesses: render_system::DeviceAccesses, _access_policies: render_system::AccessPolicies, mip_levels: u32) -> MemoryBackedResourceCreationResult<vk::Image> {
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
			.format(to_format(format, compression))
			.extent(extent)
			.mip_levels(mip_levels)
			.array_layers(1)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(if !device_accesses.intersects(render_system::DeviceAccesses::CpuRead | render_system::DeviceAccesses::CpuWrite) { vk::ImageTiling::OPTIMAL } else { vk::ImageTiling::LINEAR })
			.usage(
				if resource_uses.intersects(render_system::Uses::Image) { vk::ImageUsageFlags::SAMPLED } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::Storage) { vk::ImageUsageFlags::STORAGE } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::RenderTarget) && format != render_system::Formats::Depth32 { vk::ImageUsageFlags::COLOR_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
				|
				if resource_uses.intersects(render_system::Uses::DepthStencil) || format == render_system::Formats::Depth32 { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
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
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils.set_debug_utils_object_name(
						self.device.handle(),
						&vk::DebugUtilsObjectNameInfoEXT::default()
							.object_handle(image)
							.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
							/* .build() */
					).expect("No debug utils object name");
				}
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

	fn get_image_subresource_layout(&self, texture: &render_system::ImageHandle, mip_level: u32) -> render_system::ImageSubresourceLayout {
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

	fn create_vulkan_texture_view(&self, name: Option<&str>, texture: &vk::Image, format: render_system::Formats, compression: Option<render_system::CompressionSchemes>, _mip_levels: u32) -> vk::ImageView {
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(
				vk::ImageViewType::TYPE_2D
			)
			.format(to_format(format, compression))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if format != render_system::Formats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: 0,
				layer_count: 1,
			})
			/* .build() */;

		let vk_image_view = unsafe { self.device.create_image_view(&image_view_create_info, None).expect("No image view") };

		unsafe{
			if let Some(name) = name {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils.set_debug_utils_object_name(
						self.device.handle(),
						&vk::DebugUtilsObjectNameInfoEXT::default()
							.object_handle(vk_image_view)
							.object_name(std::ffi::CString::new(name).unwrap().as_c_str())
							/* .build() */
					).expect("No debug utils object name");
				}
			}
		}

		vk_image_view
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

struct TransitionState {
	stage: vk::PipelineStageFlags2,
	access: vk::AccessFlags2,
	layout: vk::ImageLayout,
}

pub struct VulkanCommandBufferRecording<'a> {
	render_system: &'a VulkanRenderSystem,
	command_buffer: render_system::CommandBufferHandle,
	in_render_pass: bool,
	modulo_frame_index: u32,
	states: HashMap<render_system::Handle, TransitionState>,
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
			states: HashMap::new(),
		}
	}

	// /// Retrieves the current state of a texture.\
	// /// If the texture has no known state, it will return a default state with undefined layout. This is useful for the first transition of a texture.\
	// /// If the texture has a known state, it will return the known state.
	// /// Inserts or updates state for a texture.\
	// /// If the texture has no known state, it will insert the given state.\
	// /// If the texture has a known state, it will update it with the given state.
	// /// It will return the given state.
	// /// This is useful to perform a transition on a texture.
	// fn get_texture_state(&mut self, texture_handle: render_system::TextureHandle, new_texture_state: TextureState) -> (TextureState, TextureState) {
	// 	let (texture_handle, _) = self.get_texture(texture_handle);
	// 	if let Some(old_state) = self.states.insert(render_system::Handle::TextureCopy(texture_handle), new_texture_state) {
	// 		(old_state, new_texture_state)
	// 	} else {
	// 		((vk::ImageLayout::UNDEFINED, vk::PipelineStageFlags2::NONE, vk::AccessFlags2::NONE), new_texture_state)
	// 	}
	// }

	fn get_buffer(&self, buffer_handle: render_system::BaseBufferHandle) -> (render_system::BaseBufferHandle, &Buffer) {
		(buffer_handle, &self.render_system.buffers[buffer_handle.0 as usize])
	}

	fn get_texture(&self, mut texture_handle: render_system::ImageHandle) -> (render_system::ImageHandle, &Texture) {
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

	fn get_top_level_acceleration_structure(&self, acceleration_structure_handle: render_system::TopLevelAccelerationStructureHandle) -> (render_system::TopLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.render_system.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_bottom_level_acceleration_structure(&self, acceleration_structure_handle: render_system::BottomLevelAccelerationStructureHandle) -> (render_system::BottomLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.render_system.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_command_buffer(&self) -> &CommandBufferInternal {
		&self.render_system.command_buffers[self.command_buffer.0 as usize].frames[self.modulo_frame_index as usize]
	}

	fn get_descriptor_set(&self, desciptor_set_handle: &render_system::DescriptorSetHandle) -> (render_system::DescriptorSetHandle, &DescriptorSet) {
		let mut i = 0;
		let mut handle = desciptor_set_handle.clone();
		loop {
			let descriptor_set = &self.render_system.descriptor_sets[handle.0 as usize];
			if i == self.modulo_frame_index {
				return (handle, descriptor_set);
			}
			handle = descriptor_set.next.unwrap();
			i += 1;
		}
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
		self.consume_resources(&attachments.iter().map(|attachment|
			render_system::Consumption{
				handle: render_system::Handle::Image(attachment.image),
				stages: render_system::Stages::FRAGMENT,
				access: render_system::AccessPolicies::WRITE,
				layout: attachment.layout,
			}
			// r(false, (texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None), if attachment.format == TextureFormats::Depth32 { vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS } else { vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT }, if attachment.format == TextureFormats::Depth32 { vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE } else { vk::AccessFlags2::COLOR_ATTACHMENT_WRITE })))
		).collect::<Vec<_>>());

		let render_area = vk::Rect2D::default()
			.offset(vk::Offset2D::default().x(0).y(0)/* .build() */)
			.extent(vk::Extent2D::default().width(extent.width).height(extent.height)/* .build() */)
			/* .build() */;

		let color_attchments = attachments.iter().filter(|a| a.format != render_system::Formats::Depth32).map(|attachment| {
			let (_, texture) = self.get_texture(attachment.image);
			vk::RenderingAttachmentInfo::default()
				.image_view(texture.image_view)
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
				/* .build() */
		}).collect::<Vec<_>>();

		let depth_attachment = attachments.iter().find(|attachment| attachment.format == render_system::Formats::Depth32).map(|attachment| {
			let (_, texture) = self.get_texture(attachment.image);
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

	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &render_system::TopLevelAccelerationStructureBuild) {
		let (acceleration_structure_handle, acceleration_structure) = self.get_top_level_acceleration_structure(acceleration_structure_build.acceleration_structure);

		let (as_geometries, offsets) = match acceleration_structure_build.description {
			render_system::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, instance_count } => {
				(vec![
					vk::AccelerationStructureGeometryKHR::default()
						.geometry_type(vk::GeometryTypeKHR::INSTANCES)
						.geometry(vk::AccelerationStructureGeometryDataKHR{ instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
							.array_of_pointers(false)
							.data(vk::DeviceOrHostAddressConstKHR {
								device_address: self.render_system.get_buffer_address(instances_buffer),
							})
							/* .build() */
						})
						.flags(vk::GeometryFlagsKHR::OPAQUE)
						/* .build() */
				], vec![
					vk::AccelerationStructureBuildRangeInfoKHR::default()
						.primitive_count(instance_count)
						.primitive_offset(0)
						.first_vertex(0)
						.transform_offset(0)
						/* .build() */
				])
			}
		};

		let scratch_buffer_address = unsafe {
			let (_, buffer) = self.get_buffer(acceleration_structure_build.scratch_buffer.buffer);
			self.render_system.device.get_buffer_device_address(
				&vk::BufferDeviceAddressInfo::default()
					.buffer(buffer.buffer)
					/* .build() */
			) + acceleration_structure_build.scratch_buffer.offset as u64
		};

		let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.dst_acceleration_structure(acceleration_structure.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR {
				device_address: scratch_buffer_address,
			})
			/* .build() */;

		self.states.insert(render_system::Handle::TopLevelAccelerationStructure(acceleration_structure_handle), TransitionState {
			stage: vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
			access: vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
			layout: vk::ImageLayout::UNDEFINED,
		});

		let infos = vec![build_geometry_info];
		let build_range_infos = vec![offsets];
		let geometries = vec![as_geometries];

		let command_buffer = self.get_command_buffer();

		let infos = infos.iter().zip(geometries.iter()).map(|(info, geos)| info.geometries(geos)).collect::<Vec<_>>();

		let build_range_infos = build_range_infos.iter().map(|build_range_info| build_range_info.as_slice()).collect::<Vec<_>>();

		unsafe {
			self.render_system.acceleration_structure.cmd_build_acceleration_structures(command_buffer.command_buffer, &infos, &build_range_infos)
		}
	}

	fn build_bottom_level_acceleration_structures(&mut self, acceleration_structure_builds: &[render_system::BottomLevelAccelerationStructureBuild]) {
		fn visit(this: &mut VulkanCommandBufferRecording, acceleration_structure_builds: &[render_system::BottomLevelAccelerationStructureBuild], mut infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR>, mut geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR>>, mut build_range_infos: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>>,) {
			if let Some(build) = acceleration_structure_builds.first() {
				let (acceleration_structure_handle, acceleration_structure) = this.get_bottom_level_acceleration_structure(build.acceleration_structure);

				let (as_geometries, offsets) = match &build.description {
					render_system::BottomLevelAccelerationStructureBuildDescriptions::AABB { aabb_buffer, transform_buffer, transform_count } => {
						(vec![], vec![])
					}
					render_system::BottomLevelAccelerationStructureBuildDescriptions::Mesh { vertex_buffer, index_buffer, vertex_position_encoding, index_format, triangle_count, vertex_count } => {
						let vertex_data_address = unsafe {
							let (_, buffer) = this.get_buffer(vertex_buffer.buffer);
							this.render_system.device.get_buffer_device_address(
								&vk::BufferDeviceAddressInfo::default()
									.buffer(buffer.buffer)
									/* .build() */
							) + vertex_buffer.offset
						};

						let index_data_address = unsafe {
							let (_, buffer) = this.get_buffer(index_buffer.buffer);
							this.render_system.device.get_buffer_device_address(
								&vk::BufferDeviceAddressInfo::default()
									.buffer(buffer.buffer)
									/* .build() */
							) + index_buffer.offset
						};

						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR {
								device_address: vertex_data_address,
							})
							.index_data(vk::DeviceOrHostAddressConstKHR {
								device_address: index_data_address,
							})
							.max_vertex(vertex_count - 1)
							.vertex_format(match vertex_position_encoding {
								render_system::Encodings::IEEE754 => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position encoding"),
							})
							.index_type(match index_format {
								render_system::DataTypes::U8 => vk::IndexType::UINT16,
								render_system::DataTypes::U16 => vk::IndexType::UINT16,
								render_system::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
							.vertex_stride(vertex_buffer.stride);

						let build_range_info = vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
							.primitive_count(*triangle_count)
							.primitive_offset(0)
							.first_vertex(0)
							.transform_offset(0)
							/* .build() */];

						(vec![vk::AccelerationStructureGeometryKHR::default()
							.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
							.geometry(vk::AccelerationStructureGeometryDataKHR{ triangles })],
						build_range_info)
					}
				};

				let scratch_buffer_address = unsafe {
					let (_, buffer) = this.get_buffer(build.scratch_buffer.buffer);
					this.render_system.device.get_buffer_device_address(
						&vk::BufferDeviceAddressInfo::default()
							.buffer(buffer.buffer)
							/* .build() */
					) + build.scratch_buffer.offset as u64
				};

				let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.dst_acceleration_structure(acceleration_structure.acceleration_structure)
					.scratch_data(vk::DeviceOrHostAddressKHR {
						device_address: scratch_buffer_address,
					})
					/* .build() */;
				
				this.states.insert(render_system::Handle::BottomLevelAccelerationStructure(acceleration_structure_handle), TransitionState {
					stage: vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
					access: vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
					layout: vk::ImageLayout::UNDEFINED,
				});

				infos.push(build_geometry_info);
				build_range_infos.push(offsets);
				geometries.push(as_geometries);

				visit(this, &acceleration_structure_builds[1..], infos, geometries, build_range_infos,);
			} else {
				let command_buffer = this.get_command_buffer();

				let infos = infos.iter().zip(geometries.iter()).map(|(info, geos)| info.geometries(geos)).collect::<Vec<_>>();

				let build_range_infos = build_range_infos.iter().map(|build_range_info| build_range_info.as_slice()).collect::<Vec<_>>();

				unsafe {
					this.render_system.acceleration_structure.cmd_build_acceleration_structures(command_buffer.command_buffer, &infos, &build_range_infos)
				}
			}
		}

		visit(self, acceleration_structure_builds, Vec::new(), Vec::new(), Vec::new(),);
	}

	/// Binds a shader to the GPU.
	fn bind_shader(&self, shader_handle: render_system::ShaderHandle) {
		panic!("Not implemented");
	}

	/// Binds a pipeline to the GPU.
	fn bind_raster_pipeline(&mut self, pipeline_handle: &render_system::PipelineHandle) {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.render_system.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.render_system.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline); }
		self.pipeline_bind_point = vk::PipelineBindPoint::GRAPHICS;
	}

	fn bind_compute_pipeline(&mut self, pipeline_handle: &render_system::PipelineHandle) {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.render_system.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.render_system.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline); }
		self.pipeline_bind_point = vk::PipelineBindPoint::COMPUTE;
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: &render_system::PipelineHandle) {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.render_system.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.render_system.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::RAY_TRACING_KHR, pipeline); }
		self.pipeline_bind_point = vk::PipelineBindPoint::RAY_TRACING_KHR;
	}

	/// Writes to the push constant register.
	fn write_to_push_constant(&mut self, pipeline_layout_handle: &render_system::PipelineLayoutHandle, offset: u32, data: &[u8]) {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = vk::PipelineLayout::from_raw(pipeline_layout_handle.0);
		unsafe { self.render_system.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE, offset, data); }
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
		let offsets = buffer_descriptors.iter().map(|buffer_descriptor| buffer_descriptor.offset).collect::<Vec<_>>();

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

	fn clear_textures(&mut self, textures: &[(render_system::ImageHandle, render_system::ClearValue)]) {
		self.consume_resources(textures.iter().map(|(texture_handle, _)| render_system::Consumption {
			handle: render_system::Handle::Image(*texture_handle),
			stages: render_system::Stages::TRANSFER,
			access: render_system::AccessPolicies::WRITE,
			layout: render_system::Layouts::Transfer,
		}).collect::<Vec<_>>().as_slice());

		for (texture_handle, clear_value) in textures {
			let (_, texture) = self.get_texture(*texture_handle);
	
			let clear_value = match clear_value {
				render_system::ClearValue::None => vk::ClearColorValue{ float32: [0.0, 0.0, 0.0, 0.0] },
				render_system::ClearValue::Color(color) => vk::ClearColorValue{ float32: [color.r, color.g, color.b, color.a] },
				render_system::ClearValue::Depth(depth) => vk::ClearColorValue{ float32: [*depth, 0.0, 0.0, 0.0] },
				render_system::ClearValue::Integer(r, g, b, a) => vk::ClearColorValue{ uint32: [*r, *g, *b, *a] },
			};
	
			unsafe {
				self.render_system.device.cmd_clear_color_image(self.get_command_buffer().command_buffer, texture.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear_value, &[vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				}]);
			}
		}
	}

	fn consume_resources(&mut self, consumptions: &[render_system::Consumption]) {
		let mut image_memory_barriers = Vec::new();
		let mut buffer_memory_barriers = Vec::new();
		let mut memory_barriers = Vec::new();

		for consumption in consumptions {
			let mut new_stage_mask = to_pipeline_stage_flags(consumption.stages);
			let mut new_access_mask = to_access_flags(consumption.access, consumption.stages);

			match consumption.handle {
				render_system::Handle::Image(texture_handle) => {
					let (_, texture) = self.get_texture(texture_handle);

					let new_layout = texture_format_and_resource_use_to_image_layout(texture.format_, consumption.layout, Some(consumption.access));

					new_stage_mask = to_pipeline_stage_flags_with_format(consumption.stages, texture.format_, consumption.access);
					new_access_mask = to_access_flags_with_format(consumption.access, consumption.stages, texture.format_);

					let image_memory_barrier = if let Some(barrier_source) = self.states.get(&consumption.handle) {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(barrier_source.layout)
							.src_stage_mask(barrier_source.stage)
							.src_access_mask(barrier_source.access)
						} else {
							vk::ImageMemoryBarrier2KHR::default()
							.old_layout(vk::ImageLayout::UNDEFINED)
							.src_stage_mask(vk::PipelineStageFlags2::empty())
							.src_access_mask(vk::AccessFlags2KHR::empty())
						}
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(new_layout)
						.dst_stage_mask(new_stage_mask)
						.dst_access_mask(new_access_mask)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(texture.image)
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: if texture.format != vk::Format::D32_SFLOAT { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
						/* .build() */;
					image_memory_barriers.push(image_memory_barrier);
				}
				render_system::Handle::Buffer(buffer_handle) => {
					let buffer_memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::BufferMemoryBarrier2KHR::default()
						.src_stage_mask(source.stage)
						.src_access_mask(source.access)
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					} else {
						vk::BufferMemoryBarrier2KHR::default()
						.src_stage_mask(vk::PipelineStageFlags2::empty())
						.src_access_mask(vk::AccessFlags2KHR::empty())
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask)
					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.buffer(self.render_system.buffers[buffer_handle.0 as usize].buffer)
					.offset(0)
					.size(vk::WHOLE_SIZE);

					buffer_memory_barriers.push(buffer_memory_barrier);

					let memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::MemoryBarrier2::default()
						.src_stage_mask(source.stage)
						.src_access_mask(source.access)
					} else {
						vk::MemoryBarrier2::default()
						.src_stage_mask(vk::PipelineStageFlags2::empty())
						.src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask);

					memory_barriers.push(memory_barrier);
				},
				render_system::Handle::TopLevelAccelerationStructure(_) | render_system::Handle::BottomLevelAccelerationStructure(_)=> {
					// let (handle, acceleration_structure) = self.get_top_level_acceleration_structure(handle);

					let memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::MemoryBarrier2::default()
						.src_stage_mask(source.stage)
						.src_access_mask(source.access)
					} else {
						vk::MemoryBarrier2::default()
						.src_stage_mask(vk::PipelineStageFlags2::empty())
						.src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask);

					memory_barriers.push(memory_barrier);
				}
				_ => unimplemented!(),
			};

			// Update current resource state, AFTER generating the barrier.
			self.states.insert(consumption.handle,
				TransitionState {
					stage: new_stage_mask,
					access: new_access_mask,
					layout: match consumption.handle {
						render_system::Handle::Image(texture_handle) => {
							let (_, texture) = self.get_texture(texture_handle);
							texture_format_and_resource_use_to_image_layout(texture.format_, consumption.layout, Some(consumption.access))
						}
						_ => vk::ImageLayout::UNDEFINED
					}
				}
			);
		}

		// let memory_barrier = if let Some(source) = barrier.source {
		// 	vk::MemoryBarrier2::default()
		// 		.src_stage_mask(to_pipeline_stage_flags(source.stage))
		// 		.src_access_mask(to_access_flags(source.access, source.stage))

		// } else {
		// 	vk::MemoryBarrier2::default()
		// 		.src_stage_mask(vk::PipelineStageFlags2::empty())
		// 		.src_access_mask(vk::AccessFlags2KHR::empty())
		// }
		// .dst_stage_mask(to_pipeline_stage_flags(barrier.destination.stage))
		// .dst_access_mask(to_access_flags(barrier.destination.access, barrier.destination.stage))
		// /* .build() */;

		// memory_barriers.push(memory_barrier);

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.buffer_memory_barriers(&buffer_memory_barriers)
			.memory_barriers(&memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
			/* .build() */;

		let command_buffer = self.get_command_buffer();

		unsafe { self.render_system.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	}

	fn clear_buffers(&mut self, buffer_handles: &[render_system::BaseBufferHandle]) {
		self.consume_resources(&buffer_handles.iter().map(|buffer_handle|
			render_system::Consumption{
				handle: render_system::Handle::Buffer(*buffer_handle),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			}
		).collect::<Vec<_>>());

		for buffer_handle in buffer_handles {
			unsafe {
				self.render_system.device.cmd_fill_buffer(self.get_command_buffer().command_buffer, self.render_system.buffers[buffer_handle.0 as usize].buffer, 0, vk::WHOLE_SIZE, 0);
			}

			self.states.insert(render_system::Handle::Buffer(*buffer_handle), TransitionState {
				stage: vk::PipelineStageFlags2::TRANSFER,
				access: vk::AccessFlags2::TRANSFER_WRITE,
				layout: vk::ImageLayout::UNDEFINED,
			});
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();

		unsafe {
			self.render_system.mesh_shading.cmd_draw_mesh_tasks(command_buffer.command_buffer, x, y, z);
		}
	}

	fn dispatch(&mut self, dispatch: render_system::DispatchExtent) {
		let command_buffer = self.get_command_buffer();

		let x = dispatch.dispatch_extent.width.div_ceil(dispatch.workgroup_extent.width);
		let y = dispatch.dispatch_extent.height.div_ceil(dispatch.workgroup_extent.height);
		let z = dispatch.dispatch_extent.depth.div_ceil(dispatch.workgroup_extent.depth);

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

	fn trace_rays(&mut self, binding_tables: render_system::BindingTables, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();

		let make_strided_range = |range: render_system::BufferStridedRange| -> vk::StridedDeviceAddressRegionKHR {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(self.render_system.get_buffer_address(range.buffer) + range.offset)
				.stride(range.stride)
				.size(range.size)
		};

		let raygen_shader_binding_tables = make_strided_range(binding_tables.raygen);
		let miss_shader_binding_tables = make_strided_range(binding_tables.miss);
		let hit_shader_binding_tables = make_strided_range(binding_tables.hit);
		let callable_shader_binding_tables = if let Some(binding_table) = binding_tables.callable { make_strided_range(binding_table) } else { vk::StridedDeviceAddressRegionKHR::default() };

		unsafe {
			self.render_system.ray_tracing_pipeline.cmd_trace_rays(command_buffer.command_buffer, &raygen_shader_binding_tables, &miss_shader_binding_tables, &hit_shader_binding_tables, &callable_shader_binding_tables, x, y, z)
		}
	}

	fn transfer_textures(&mut self, texture_handles: &[render_system::ImageHandle]) {
		self.consume_resources(&texture_handles.iter().map(|texture_handle|
			render_system::Consumption{
				handle: render_system::Handle::Image(*texture_handle),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			}
			// r(false, (texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None), if attachment.format == TextureFormats::Depth32 { vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS } else { vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT }, if attachment.format == TextureFormats::Depth32 { vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE } else { vk::AccessFlags2::COLOR_ATTACHMENT_WRITE })))
		).collect::<Vec<_>>());

		let command_buffer = self.get_command_buffer();

		for texture_handle in texture_handles {
			let (_, texture) = self.get_texture(*texture_handle);

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

			let (_, buffer) = self.get_buffer(texture.staging_buffer.expect("No staging buffer"));

			// Copy to images from staging buffer
			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(buffer.buffer)
				.dst_image(texture.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.render_system.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
			}
		}
	}

	fn write_image_data(&mut self, texture_handle: render_system::ImageHandle, data: &[render_system::RGBAu8]) {
		self.consume_resources(
			&[render_system::Consumption{
				handle: render_system::Handle::Image(texture_handle),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			}]
		);

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

		let command_buffer = self.get_command_buffer();

		unsafe {
			self.render_system.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
		}
	}

	fn copy_to_swapchain(&mut self, source_texture_handle: render_system::ImageHandle, present_image_index: u32, swapchain_handle: render_system::SwapchainHandle) {
		self.consume_resources(&[
			render_system::Consumption {
				handle: render_system::Handle::Image(source_texture_handle),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Transfer,
			},
		]);

		let (_, source_texture) = self.get_texture(source_texture_handle);
		let swapchain = &self.render_system.swapchains[swapchain_handle.0 as usize];

		let swapchain_images = unsafe {
			self.render_system.swapchain.get_swapchain_images(swapchain.swapchain).expect("No swapchain images found.")
		};

		let swapchain_image = swapchain_images[present_image_index as usize];

		// Transition source texture to transfer read layout and swapchain image to transfer write layout

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

		let image_blits = [vk::ImageBlit2::default()
			.src_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
				/* .build() */
			)
			.src_offsets([
				vk::Offset3D::default().x(0).y(0).z(0)/* .build() */,
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1)/* .build() */,
			])
			.dst_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
				/* .build() */
			)
			.dst_offsets([
				vk::Offset3D::default().x(0).y(0).z(0)/* .build() */,
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1)/* .build() */,
			])
		];

		let copy_image_info = vk::BlitImageInfo2::default()
			.src_image(source_texture.image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(swapchain_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_blits);
			/* .build() */

		unsafe { self.render_system.device.cmd_blit_image2(command_buffer.command_buffer, &copy_image_info); }

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
	fn bind_descriptor_sets(&self, pipeline_layout: &render_system::PipelineLayoutHandle, sets: &[(render_system::DescriptorSetHandle, u32)]) {
		let command_buffer = self.get_command_buffer();

		let pipeline_layout = vk::PipelineLayout::from_raw(pipeline_layout.0);

		assert!(sets.is_sorted_by(|a, b| Some(a.1.cmp(&b.1))));

		for (descriptor_set_handle, set_index) in sets {
			let (_, descriptor_set) = self.get_descriptor_set(descriptor_set_handle);
			let descriptor_sets = [descriptor_set.descriptor_set];
	
			unsafe {
				self.render_system.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, self.pipeline_bind_point, pipeline_layout, *set_index, &descriptor_sets, &[]);
			}
		}
	}

	fn sync_textures(&mut self, texture_handles: &[render_system::ImageHandle]) -> Vec<render_system::TextureCopyHandle> {
		self.consume_resources(&texture_handles.iter().map(|texture_handle| render_system::Consumption {
			handle: render_system::Handle::Image(*texture_handle),
			stages: render_system::Stages::TRANSFER,
			access: render_system::AccessPolicies::READ,
			layout: render_system::Layouts::Transfer,
		}).collect::<Vec<_>>());

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
				.stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR)
				/* .build() */
		}).collect::<Vec<_>>();

		let signal_semaphores = signal_synchronizer_handles.iter().map(|signal| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.render_system.synchronizers[signal.0 as usize].semaphore)
				.stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
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

	fn start_region(&self, name: &str) {
		let command_buffer = self.get_command_buffer();

		let name = std::ffi::CString::new(name).unwrap();

		let marker_info = vk::DebugUtilsLabelEXT::default()
			.label_name(name.as_c_str());

		unsafe {
			if let Some(debug_utils) = &self.render_system.debug_utils {
				debug_utils.cmd_begin_debug_utils_label(command_buffer.command_buffer, &marker_info);
			}
		}
	}

	fn end_region(&self) {
		let command_buffer = self.get_command_buffer();

		unsafe {
			if let Some(debug_utils) = &self.render_system.debug_utils {
				debug_utils.cmd_end_debug_utils_label(command_buffer.command_buffer);
			}
		}
	
	}
}

struct Mesh {
	buffer: vk::Buffer,
	allocation: render_system::AllocationHandle,
	vertex_count: u32,
	index_count: u32,
	vertex_size: usize,
}

struct AccelerationStructure {
	acceleration_structure: vk::AccelerationStructureKHR,
	buffer: vk::Buffer,
	scratch_size: usize,
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
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::render_triangle(&mut vulkan_render_system);
	}

	#[test]
	fn present() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::present(&mut vulkan_render_system);
	}

	#[test]
	fn multiframe_present() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::multiframe_present(&mut vulkan_render_system);
	}

	#[test]
	fn multiframe_rendering() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::multiframe_rendering(&mut vulkan_render_system);
	}

	#[test]
	fn dynamic_data() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::dynamic_data(&mut vulkan_render_system);
	}

	#[test]
	fn descriptor_sets() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: false });
		render_system::tests::descriptor_sets(&mut vulkan_render_system);
	}

	#[test]
	fn ray_tracing() {
		let mut vulkan_render_system = VulkanRenderSystem::new(&Settings { validation: true, ray_tracing: true });
		render_system::tests::ray_tracing(&mut vulkan_render_system);
	}
}