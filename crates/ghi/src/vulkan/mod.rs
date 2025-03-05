use std::{borrow::Cow, collections::{HashMap, HashSet}, mem::align_of};

use ash::vk::{self, Handle as _};
use utils::{partition, Extent};

use crate::{graphics_hardware_interface, image::Builder, render_debugger::RenderDebugger, sampler, window, Size};

pub struct VulkanGHI {
	entry: ash::Entry,
	instance: ash::Instance,

	debug_utils: Option<ash::ext::debug_utils::Device>,
	debug_utils_messenger: Option<vk::DebugUtilsMessengerEXT>,
	debug_data: Box<DebugCallbackData>,

	physical_device: vk::PhysicalDevice,
	device: ash::Device,
	queue_family_index: u32,
	queue: vk::Queue,
	swapchain: ash::khr::swapchain::Device,
	surface: ash::khr::surface::Instance,
	acceleration_structure: ash::khr::acceleration_structure::Device,
	ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device,
	mesh_shading: ash::ext::mesh_shader::Device,

	#[cfg(debug_assertions)]
	debugger: RenderDebugger,

	frames: u8,

	buffers: Vec<Buffer>,
	images: Vec<Image>,
	allocations: Vec<Allocation>,
	descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	pipeline_layouts: Vec<PipelineLayout>,
	bindings: Vec<Binding>,
	descriptor_sets: Vec<DescriptorSet>,
	meshes: Vec<Mesh>,
	acceleration_structures: Vec<AccelerationStructure>,
	shaders: Vec<Shader>,
	pipelines: Vec<Pipeline>,
	command_buffers: Vec<CommandBuffer>,
	synchronizers: Vec<Synchronizer>,
	swapchains: Vec<Swapchain>,

	resource_to_descriptor: HashMap<Handle, HashSet<(DescriptorSetBindingHandle, u32)>>,

	descriptors: HashMap<DescriptorSetHandle, HashMap<u32, HashMap<u32, Descriptor>>>,
	descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32), HashSet<Handle>>,

	settings: graphics_hardware_interface::Features,

	states: HashMap<Handle, TransitionState>,

	pending_images: Vec<graphics_hardware_interface::ImageHandle>,
	pending_buffers: Vec<graphics_hardware_interface::BaseBufferHandle>,
}

enum Descriptor {
	Image {
		image: ImageHandle,
		layout: graphics_hardware_interface::Layouts,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: vk::Sampler,
		layout: graphics_hardware_interface::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageHandle(u64);
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BufferHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TopLevelAccelerationStructureHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BottomLevelAccelerationStructureHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DescriptorSetHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DescriptorSetBindingHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SynchronizerHandle(u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Handle {
	Image(ImageHandle),
	Buffer(BufferHandle),
	TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle),
	BottomLevelAccelerationStructure(BottomLevelAccelerationStructureHandle),
}

#[derive(Clone, PartialEq,)]
struct Consumption {
	handle: Handle,
	stages: graphics_hardware_interface::Stages,
	access: graphics_hardware_interface::AccessPolicies,
	layout: graphics_hardware_interface::Layouts,
}

impl graphics_hardware_interface::GraphicsHardwareInterface for VulkanGHI {
	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(&mut self, size: usize, _resource_uses: graphics_hardware_interface::Uses, resource_device_accesses: graphics_hardware_interface::DeviceAccesses) -> graphics_hardware_interface::AllocationHandle {
		self.create_allocation_internal(size, None, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(&mut self, vertex_count: u32, index_count: u32, vertices: &[u8], indices: &[u8], vertex_layout: &[graphics_hardware_interface::VertexElement]) -> graphics_hardware_interface::MeshHandle {
		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(None, buffer_size, vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, pointer) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);

		self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		unsafe {
			let vertex_buffer_pointer = pointer.expect("No pointer");
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size);
			let index_buffer_pointer = vertex_buffer_pointer.add(vertex_buffer_size.next_multiple_of(16));
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size);
		}

		let mesh_handle = graphics_hardware_interface::MeshHandle(self.meshes.len() as u64);

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
	fn create_shader(&mut self, name: Option<&str>, shader_source_type: graphics_hardware_interface::ShaderSource, stage: graphics_hardware_interface::ShaderTypes, shader_binding_descriptors: &[graphics_hardware_interface::ShaderBindingDescriptor],) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
			graphics_hardware_interface::ShaderSource::GLSL(source_code) => {
				let compiler = shaderc::Compiler::new().unwrap();
				let mut options = shaderc::CompileOptions::new().unwrap();
		
				options.set_optimization_level(shaderc::OptimizationLevel::Performance);
				options.set_target_env(shaderc::TargetEnv::Vulkan, (1 << 22) | (3 << 12));
				options.set_generate_debug_info();
				options.set_target_spirv(shaderc::SpirvVersion::V1_6);
				options.set_invert_y(true);
		
				let binary = compiler.compile_into_spirv(source_code.as_str(), shaderc::ShaderKind::InferFromSource, "shader_name", "main", Some(&options));
				
				match binary {
					Ok(binary) => {
						Cow::Owned(binary.as_binary().to_vec())
					},
					Err(err) => {
						let compiler_error_string = err.to_string();

						println!("{}", source_code.as_str());
						println!("------");
						println!("{}", compiler_error_string);

						return Err(());
					}
				}
			}
			graphics_hardware_interface::ShaderSource::SPIRV(spirv) => {
				if !spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
					return Err(());
				}

				// SAFETY: shader was checked to be aligned to 4 bytes.
				Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
			}
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).unwrap() };

		let handle = graphics_hardware_interface::ShaderHandle(self.shaders.len() as u64);

		self.shaders.push(Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_binding_descriptors: shader_binding_descriptors.to_vec(),
		});

		self.set_name(shader_module, name);

		Ok(handle)
	}

	fn create_descriptor_set_template(&mut self, name: Option<&str>, bindings: &[graphics_hardware_interface::DescriptorSetBindingTemplate]) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		fn m(rs: &mut VulkanGHI, bindings: &[graphics_hardware_interface::DescriptorSetBindingTemplate], layout_bindings: &mut Vec<vk::DescriptorSetLayoutBinding>, map: &mut Vec<(vk::DescriptorType, u32)>) -> vk::DescriptorSetLayout {
			if let Some(binding) = bindings.get(0) {
				let b = vk::DescriptorSetLayoutBinding::default()
					.binding(binding.binding)
					.descriptor_type(match binding.descriptor_type {
						graphics_hardware_interface::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
						graphics_hardware_interface::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
						graphics_hardware_interface::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
						graphics_hardware_interface::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
						graphics_hardware_interface::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
						graphics_hardware_interface::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
						graphics_hardware_interface::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
					})
					.descriptor_count(binding.descriptor_count)
					.stage_flags(binding.stages.into())
				;

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

		self.set_name(descriptor_set_layout, name);

		let handle = graphics_hardware_interface::DescriptorSetTemplateHandle(self.descriptor_sets_layouts.len() as u64);

		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings: bindings_list,
			descriptor_set_layout,
		});

		handle
	}

	fn create_descriptor_binding(&mut self, descriptor_set: graphics_hardware_interface::DescriptorSetHandle, constructor: graphics_hardware_interface::BindingConstructor) -> graphics_hardware_interface::DescriptorSetBindingHandle {
		let binding = constructor.descriptor_set_binding_template;

		let descriptor_type = match binding.descriptor_type {
			graphics_hardware_interface::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
			graphics_hardware_interface::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
			graphics_hardware_interface::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
			graphics_hardware_interface::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
			graphics_hardware_interface::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
			graphics_hardware_interface::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
			graphics_hardware_interface::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
		};

		let created_binding = Binding {
			descriptor_set_handle: descriptor_set,
			descriptor_type,
			type_: binding.descriptor_type,
			count: binding.descriptor_count,
			index: binding.binding,
			stages: binding.stages,
			pipeline_stages: to_pipeline_stage_flags(binding.stages, None, None),
		};

		let binding_handle = graphics_hardware_interface::DescriptorSetBindingHandle(self.bindings.len() as u64);

		self.bindings.push(created_binding);

		self.write_binding(&graphics_hardware_interface::DescriptorWrite {
			array_element: constructor.array_element,
			binding_handle,
			descriptor: constructor.descriptor,
			frame_offset: constructor.frame_offset,
		});

		binding_handle
	}

	fn create_descriptor_binding_array(&mut self, descriptor_set: crate::DescriptorSetHandle, binding_template: &crate::DescriptorSetBindingTemplate) -> crate::DescriptorSetBindingHandle {
		let descriptor_type = match binding_template.descriptor_type {
			graphics_hardware_interface::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
			graphics_hardware_interface::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
			graphics_hardware_interface::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
			graphics_hardware_interface::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
			graphics_hardware_interface::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
			graphics_hardware_interface::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
			graphics_hardware_interface::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
		};

		let handle = graphics_hardware_interface::DescriptorSetBindingHandle(self.bindings.len() as u64);

		self.bindings.push(Binding {
			descriptor_set_handle: descriptor_set,
			descriptor_type,
			type_: binding_template.descriptor_type,
			count: binding_template.descriptor_count,
			index: binding_template.binding,
			stages: binding_template.stages,
			pipeline_stages: to_pipeline_stage_flags(binding_template.stages, None, None),
		});

		handle
	}

	fn create_descriptor_set(&mut self, name: Option<&str>, descriptor_set_layout_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle) -> graphics_hardware_interface::DescriptorSetHandle {
		let pool_sizes = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize].bindings.iter().map(|(descriptor_type, descriptor_count)| {
			vk::DescriptorPoolSize::default().ty(*descriptor_type).descriptor_count(descriptor_count * self.frames as u32)
		}).collect::<Vec<_>>();

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
		;

		let descriptor_sets = unsafe { self.device.allocate_descriptor_sets(&descriptor_set_allocate_info).expect("No descriptor set") };

		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<DescriptorSetHandle> = None;

		for descriptor_set in descriptor_sets {
			let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);

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

			self.set_name(descriptor_set, name);

			previous_handle = Some(handle);
		}

		handle
	}

	fn write(&mut self, descriptor_set_writes: &[graphics_hardware_interface::DescriptorWrite]) {		
		for dsw in descriptor_set_writes {
			self.write_binding(dsw);
		}
	}

	fn create_pipeline_layout(&mut self, descriptor_set_layout_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle], push_constant_ranges: &[graphics_hardware_interface::PushConstantRange]) -> graphics_hardware_interface::PipelineLayoutHandle {
		let push_constant_ranges = push_constant_ranges.iter().map(|push_constant_range| vk::PushConstantRange::default().size(push_constant_range.size).offset(push_constant_range.offset).stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE)).collect::<Vec<_>>();
		let set_layouts = descriptor_set_layout_handles.iter().map(|set_layout| self.descriptor_sets_layouts[set_layout.0 as usize].descriptor_set_layout).collect::<Vec<_>>();

  		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&set_layouts)
			.push_constant_ranges(&push_constant_ranges)
		;

		let pipeline_layout = unsafe { self.device.create_pipeline_layout(&pipeline_layout_create_info, None).expect("No pipeline layout") };

		let handle = graphics_hardware_interface::PipelineLayoutHandle(self.pipeline_layouts.len() as u64);

		self.pipeline_layouts.push(PipelineLayout {
			pipeline_layout,
			descriptor_set_template_indices: descriptor_set_layout_handles.iter().enumerate().map(|(i, handle)| (handle.clone(), i as u32)).collect(),
		});

		handle
	}

	fn create_raster_pipeline(&mut self, pipeline_blocks: &[graphics_hardware_interface::PipelineConfigurationBlocks]) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_pipeline(pipeline_blocks)
	}

	fn create_compute_pipeline(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, shader_parameter: graphics_hardware_interface::ShaderParameter) -> graphics_hardware_interface::PipelineHandle {
		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);

		let mut specialization_map_entries = Vec::with_capacity(48);
		
		for specialization_map_entry in shader_parameter.specialization_map {
			// TODO: accumulate offset
			match specialization_map_entry.get_type().as_str() {
				"vec2f" => {
					for i in 0..2 {
						specialization_map_entries.push(vk::SpecializationMapEntry::default()
						.constant_id(specialization_map_entry.get_constant_id() + i)
						.offset(specialization_entries_buffer.len() as u32 + i * 4)
						.size(4));
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec3f" => {
					for i in 0..3 {
						specialization_map_entries.push(vk::SpecializationMapEntry::default()
						.constant_id(specialization_map_entry.get_constant_id() + i)
						.offset(specialization_entries_buffer.len() as u32 + i * 4)
						.size(4));
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec4f" => {
					for i in 0..4 {
						specialization_map_entries.push(vk::SpecializationMapEntry::default()
						.constant_id(specialization_map_entry.get_constant_id() + i)
						.offset(specialization_entries_buffer.len() as u32 + i * 4)
						.size(4));
					}

					assert_eq!(specialization_map_entry.get_size(), 16);

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				_ => {
					panic!("Unknown specialization map entry type");
				}
			}
		}

		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let shader = &self.shaders[shader_parameter.handle.0 as usize];

		let create_infos = [
			vk::ComputePipelineCreateInfo::default()
				.stage(vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					.specialization_info(&specialization_info)
				)
				.layout(pipeline_layout.pipeline_layout)
		];

		let pipeline_handle = unsafe {
			self.device.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None).expect("No compute pipeline")[0]
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		let resource_access = shader.shader_binding_descriptors.iter().map(|descriptor| {
			((descriptor.set, descriptor.binding), (graphics_hardware_interface::Stages::COMPUTE, descriptor.access))
		}).collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			shader_handles: HashMap::new(),
			shaders: vec![*shader_parameter.handle],
			resource_access,
		});

		handle
	}

	fn create_ray_tracing_pipeline(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, shaders: &[graphics_hardware_interface::ShaderParameter]) -> graphics_hardware_interface::PipelineHandle {
		let mut groups = Vec::with_capacity(1024);
		
		let stages = shaders.iter().map(|stage| {
			let shader = &self.shaders[stage.handle.0 as usize];

			vk::PipelineShaderStageCreateInfo::default()
				.stage(to_shader_stage_flags(stage.stage))
				.module(shader.shader)
				.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
		}).collect::<Vec<_>>();

		for (i, shader) in shaders.iter().enumerate() {
			match shader.stage {
				graphics_hardware_interface::ShaderTypes::RayGen | graphics_hardware_interface::ShaderTypes::Miss | graphics_hardware_interface::ShaderTypes::Callable => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
						.general_shader(i as u32)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				graphics_hardware_interface::ShaderTypes::ClosestHit => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(i as u32)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				graphics_hardware_interface::ShaderTypes::AnyHit => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(i as u32)
						.intersection_shader(vk::SHADER_UNUSED_KHR));
				}
				graphics_hardware_interface::ShaderTypes::Intersection => {
					groups.push(vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
						.general_shader(vk::SHADER_UNUSED_KHR)
						.closest_hit_shader(vk::SHADER_UNUSED_KHR)
						.any_hit_shader(vk::SHADER_UNUSED_KHR)
						.intersection_shader(i as u32));
				}
				_ => {
					// warn!("Fed shader of type '{:?}' to ray tracing pipeline", shader.stage)
				}
			}
		}

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.layout(pipeline_layout.pipeline_layout)
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let mut handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]> = HashMap::with_capacity(shaders.len());

		let pipeline_handle = unsafe {
			let pipeline = self.ray_tracing_pipeline.create_ray_tracing_pipelines(vk::DeferredOperationKHR::null(), vk::PipelineCache::null(), &[create_info], None).expect("No ray tracing pipeline")[0];
			let handle_buffer = self.ray_tracing_pipeline.get_ray_tracing_shader_group_handles(pipeline, 0, groups.len() as u32, 32 * groups.len()).expect("Could not get ray tracing shader group handles");

			for (i, shader) in shaders.iter().enumerate() {
				let mut h = [0u8; 32];
				h.copy_from_slice(&handle_buffer[i * 32..(i + 1) * 32]);

				handles.insert(*shader.handle, h);
			}

			pipeline
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		let resource_access = shaders.iter().map(|shader| {
			let shader = &self.shaders[shader.handle.0 as usize];

			shader.shader_binding_descriptors.iter().map(|descriptor| {
				((descriptor.set, descriptor.binding), (shader.stage, descriptor.access))
			}).collect::<Vec<_>>()
		}).flatten().collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			shader_handles: handles,
			shaders: shaders.iter().map(|shader| *shader.handle).collect(),
			resource_access,
		});

		handle
	}

	fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
		let command_buffer_handle = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

		let command_buffers = (0..self.frames).map(|_| {
			let _ = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

			let command_pool_create_info = vk::CommandPoolCreateInfo::default().queue_family_index(self.queue_family_index);

			let command_pool = unsafe { self.device.create_command_pool(&command_pool_create_info, None).expect("No command pool") };

			let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
				.command_pool(command_pool)
				.level(vk::CommandBufferLevel::PRIMARY)
				.command_buffer_count(1)
			;

			let command_buffers = unsafe { self.device.allocate_command_buffers(&command_buffer_allocate_info).expect("No command buffer") };

			let command_buffer = command_buffers[0];

			self.set_name(command_buffer, name);

			CommandBufferInternal { command_pool, command_buffer, }
		}).collect::<Vec<_>>();

		self.command_buffers.push(CommandBuffer {
			frames: command_buffers,
		});

		command_buffer_handle
	}

	fn create_command_buffer_recording(&mut self, command_buffer_handle: graphics_hardware_interface::CommandBufferHandle, frmae_index: Option<u32>) -> crate::CommandBufferRecording {
		use graphics_hardware_interface::CommandBufferRecordable;
		let pending_images = self.pending_images.clone();
		self.pending_images.clear();
		let mut recording = VulkanCommandBufferRecording::new(self, command_buffer_handle, frmae_index);
		recording.begin();
		recording.transfer_textures(&pending_images);
		recording
	}

	fn create_buffer(&mut self, name: Option<&str>, size: usize, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses, use_case: graphics_hardware_interface::UseCases) -> graphics_hardware_interface::BaseBufferHandle {
		let buffer_count = match use_case {
			graphics_hardware_interface::UseCases::STATIC => 1,
			graphics_hardware_interface::UseCases::DYNAMIC => self.frames,
		};

		let mut uses = uses_to_vk_usage_flags(resource_uses);

		if !self.settings.ray_tracing {
			// Remove acc struct build flag
			uses &= !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
		}

		let staging_buffer_handle_option = if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) {
			let buffer = if size != 0 {
				let buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
				let (allocation_handle, _) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuWrite);
				let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);
				Buffer {
					next: None,
					staging: None,
					buffer: buffer_creation_result.resource,
					size,
					device_address,
					pointer,
					uses: resource_uses,
					use_cases: None,
					frame: None,
				} 
			} else {
				Buffer {
					next: None,
					staging: None,
					buffer: vk::Buffer::null(),
					size: 0,
					device_address: 0,
					pointer: std::ptr::null_mut(),
					uses: resource_uses,
					use_cases: None,
					frame: None,
				}
			};

			let buffer_handle = BufferHandle(self.buffers.len() as u64);

			self.buffers.push(buffer);

			Some(buffer_handle)
		} else {
			None
		};

		let buffer_handle = graphics_hardware_interface::BaseBufferHandle(self.buffers.len() as u64);

		let mut previous: Option<BufferHandle> = None;

		for f in 0..buffer_count {
			let handle = BufferHandle(self.buffers.len() as u64);

			let buffer = if size != 0 {
				let buffer_creation_result = self.create_vulkan_buffer(name, size, uses | vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
				let (allocation_handle, _) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::GpuWrite);
				let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

				Buffer {
					next: None,
					staging: staging_buffer_handle_option,
					buffer: buffer_creation_result.resource,
					size,
					device_address,
					pointer,
					uses: resource_uses,
					use_cases: Some(use_case),
					frame: Some(f as _),
				}
			} else {
				Buffer {
					next: None,
					staging: staging_buffer_handle_option,
					buffer: vk::Buffer::null(),
					size: 0,
					device_address: 0,
					pointer: std::ptr::null_mut(),
					uses: resource_uses,
					use_cases: Some(use_case),
					frame: Some(f as _),
				}
			};

			self.buffers.push(buffer);

			if let Some(p) = previous {
				self.buffers[p.0 as usize].next = Some(handle);
			}

			previous = Some(handle);
		}

		return buffer_handle;
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers[buffer_handle.0 as usize].device_address
	}

	fn get_buffer_slice(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> &[u8] {
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe {
			std::slice::from_raw_parts(buffer.pointer, buffer.size)
		}
	}

	fn get_mut_buffer_slice<'a>(&'a mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> &'a mut [u8] {
		self.pending_buffers.push(buffer_handle);
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, buffer.size)
		}
	}

	fn get_splitter<'a>(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, offset: usize) -> graphics_hardware_interface::BufferSplitter<'a> {
		self.pending_buffers.push(buffer_handle);
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		let slice = unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer as *mut u8, buffer.size)
		};
		graphics_hardware_interface::BufferSplitter::new(slice, offset)
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		self.pending_images.push(texture_handle);
		let texture = &self.images[texture_handle.0 as usize];
		assert!(texture.pointer != std::ptr::null());
		unsafe {
			std::slice::from_raw_parts_mut(texture.pointer as *mut u8, texture.size)
		}
	}

	fn create_image(&mut self, name: Option<&str>, extent: Extent, format: graphics_hardware_interface::Formats, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses, use_case: graphics_hardware_interface::UseCases, array_layers: u32) -> graphics_hardware_interface::ImageHandle {
		let size = (extent.width() * extent.height() * extent.depth()) as usize * format.size();

		let texture_handle = graphics_hardware_interface::ImageHandle(self.images.len() as u64);

		let mut previous_texture_handle: Option<ImageHandle> = None;

		let extent = vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth());

		for _ in 0..(match use_case { graphics_hardware_interface::UseCases::DYNAMIC => { self.frames } graphics_hardware_interface::UseCases::STATIC => { 1 }}) {
			let resource_uses = resource_uses | if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) { graphics_hardware_interface::Uses::TransferDestination } else { graphics_hardware_interface::Uses::empty() };

			let texture_handle = ImageHandle(self.images.len() as u64);

			if extent.width != 0 && extent.height != 0 && extent.depth != 0 {
				let m_device_accesses = if device_accesses.intersects(graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::CpuRead) {
					graphics_hardware_interface::DeviceAccesses::GpuRead | graphics_hardware_interface::DeviceAccesses::GpuWrite
				} else {
					device_accesses
				};

				let texture_creation_result = self.create_vulkan_texture(name, extent, format, resource_uses | graphics_hardware_interface::Uses::TransferSource, m_device_accesses, graphics_hardware_interface::AccessPolicies::WRITE, 1, array_layers);
	
				let (allocation_handle, _) = self.create_allocation_internal(texture_creation_result.size, texture_creation_result.memory_flags.into(), m_device_accesses);
	
				let _ = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);
	
				let image_view = self.create_vulkan_image_view(name, &texture_creation_result.resource, format, 0, 0, array_layers);
	
				let (staging_buffer, pointer) = if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuRead) {
					let staging_buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
	
					let (allocation_handle, _) = self.create_allocation_internal(staging_buffer_creation_result.size, staging_buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuRead);
	
					let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);
	
					let staging_buffer_handle = BufferHandle(self.buffers.len() as u64);
	
					self.buffers.push(Buffer {
						next: None,
						staging: None,
						buffer: staging_buffer_creation_result.resource,
						size: staging_buffer_creation_result.size,
						device_address: address,
						pointer,
						uses: graphics_hardware_interface::Uses::TransferDestination,
						use_cases: None,
						frame: None,
					});
	
					(Some(staging_buffer_handle), pointer)
				} else if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) {
					let staging_buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);
	
					let (allocation_handle, _) = self.create_allocation_internal(staging_buffer_creation_result.size, staging_buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);
	
					let (address, pointer) = self.bind_vulkan_buffer_memory(&staging_buffer_creation_result, allocation_handle, 0);
	
					let staging_buffer_handle = BufferHandle(self.buffers.len() as u64);
	
					self.buffers.push(Buffer {
						next: None,
						staging: None,
						buffer: staging_buffer_creation_result.resource,
						size: staging_buffer_creation_result.size,
						device_address: address,
						pointer,
						uses: graphics_hardware_interface::Uses::TransferSource,
						use_cases: None,
						frame: None,
					});
	
					(Some(staging_buffer_handle), pointer)
				} else {
					(None, std::ptr::null_mut())
				};

				let image_views = {
					let mut image_views = [vk::ImageView::null(); 8];

					for i in 0..array_layers {
						image_views[i as usize] = self.create_vulkan_image_view(name, &texture_creation_result.resource, format, 0, i, 1);
					}

					image_views
				};

				self.images.push(Image {
					#[cfg(debug_assertions)]
					name: name.map(|name| name.to_string()),
					next: None,
					size: texture_creation_result.size,
					staging_buffer,
					allocation_handle,
					image: texture_creation_result.resource,
					image_view,
					image_views,
					pointer,
					extent,
					format: to_format(format),
					format_: format,
					layout: vk::ImageLayout::UNDEFINED,
					uses: resource_uses,
					layers: array_layers,
				});
			} else {
				self.images.push(Image {
					#[cfg(debug_assertions)]
					name: name.map(|name| name.to_string()),
					next: None,
					size: 0,
					staging_buffer: None,
					allocation_handle: crate::AllocationHandle(!0u64),
					image: vk::Image::null(),
					image_view: vk::ImageView::null(),
					image_views: [vk::ImageView::null(); 8],
					pointer: std::ptr::null_mut(),
					extent,
					format: to_format(format),
					format_: format,
					layout: vk::ImageLayout::UNDEFINED,
					uses: resource_uses,
					layers: array_layers,
				});
			}

			if let Some(previous_texture_handle) = previous_texture_handle {
				self.images[previous_texture_handle.0 as usize].next = Some(texture_handle);
			}

			previous_texture_handle = Some(texture_handle);
		}

		texture_handle
	}

	fn build_image(&mut self, builder: Builder) -> graphics_hardware_interface::ImageHandle {
		self.create_image(builder.name, builder.extent, builder.format, builder.resource_uses, builder.device_accesses, builder.use_case, builder.array_layers)
	}

	fn create_sampler(&mut self, filtering_mode: graphics_hardware_interface::FilteringModes, reduction_mode: graphics_hardware_interface::SamplingReductionModes, mip_map_filter: graphics_hardware_interface::FilteringModes, address_mode: graphics_hardware_interface::SamplerAddressingModes, anisotropy: Option<f32>, min_lod: f32, max_lod: f32) -> graphics_hardware_interface::SamplerHandle {
		let filtering_mode = match filtering_mode {
			graphics_hardware_interface::FilteringModes::Closest => { vk::Filter::NEAREST }
			graphics_hardware_interface::FilteringModes::Linear => { vk::Filter::LINEAR }
		};

		let mip_map_filter = match mip_map_filter {
			graphics_hardware_interface::FilteringModes::Closest => { vk::SamplerMipmapMode::NEAREST }
			graphics_hardware_interface::FilteringModes::Linear => { vk::SamplerMipmapMode::LINEAR }
		};

		let address_mode = match address_mode {
			graphics_hardware_interface::SamplerAddressingModes::Repeat => { vk::SamplerAddressMode::REPEAT }
			graphics_hardware_interface::SamplerAddressingModes::Mirror => { vk::SamplerAddressMode::MIRRORED_REPEAT }
			graphics_hardware_interface::SamplerAddressingModes::Clamp => { vk::SamplerAddressMode::CLAMP_TO_EDGE }
			graphics_hardware_interface::SamplerAddressingModes::Border{ .. } => { vk::SamplerAddressMode::CLAMP_TO_BORDER }
		};

		let reduction_mode = match reduction_mode {
			graphics_hardware_interface::SamplingReductionModes::WeightedAverage => { vk::SamplerReductionMode::WEIGHTED_AVERAGE }
			graphics_hardware_interface::SamplingReductionModes::Min => { vk::SamplerReductionMode::MIN }
			graphics_hardware_interface::SamplingReductionModes::Max => { vk::SamplerReductionMode::MAX }
		};

		graphics_hardware_interface::SamplerHandle(self.create_vulkan_sampler(filtering_mode, reduction_mode, mip_map_filter, address_mode, anisotropy, min_lod, max_lod).as_raw())
	}

	fn build_sampler(&mut self, builder: sampler::Builder) -> crate::SamplerHandle {
		self.create_sampler(builder.filtering_mode, builder.reduction_mode, builder.mip_map_mode, builder.addressing_mode, builder.anisotropy, builder.min_lod, builder.max_lod)
	}

	fn create_acceleration_structure_instance_buffer(&mut self, name: Option<&str>, max_instance_count: u32) -> graphics_hardware_interface::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

		let buffer_creation_result = self.create_vulkan_buffer(name, size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let buffer_handle = graphics_hardware_interface::BaseBufferHandle(self.buffers.len() as u64);

		self.buffers.push(Buffer {
			next: None,
			staging: None,
			buffer: buffer_creation_result.resource,
			size: buffer_creation_result.size,
			device_address: address,
			pointer,
			uses: graphics_hardware_interface::Uses::empty(),
			use_cases: None,
			frame: None,
		});

		buffer_handle
	}

	fn create_top_level_acceleration_structure(&mut self, name: Option<&str>, max_instance_count: u32) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR { instances: vk::AccelerationStructureGeometryInstancesDataKHR::default() })
		;

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries)
		;

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &build_info, &[max_instance_count], &mut size_info);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let scratch_size = size_info.build_scratch_size as usize;

		let buffer = self.create_vulkan_buffer(None, acceleration_structure_size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer.size, buffer.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::GpuWrite);

		let (_, _) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
		;

		let handle = graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure.create_acceleration_structure(&create_info, None).expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer.resource,
				scratch_size,
			});

			self.set_name(handle, name);
		}

		handle
	}

	fn create_bottom_level_acceleration_structure(&mut self, description: &graphics_hardware_interface::BottomLevelAccelerationStructure,) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let (geometry, primitive_count) = match &description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh { vertex_count, vertex_position_encoding, triangle_count, index_format } => {
				(vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_format(match vertex_position_encoding {
								graphics_hardware_interface::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position format"),
							})
							.max_vertex(*vertex_count - 1)
							.index_type(match index_format {
								graphics_hardware_interface::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								graphics_hardware_interface::DataTypes::U16 => vk::IndexType::UINT16,
								graphics_hardware_interface::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
					}),
				*triangle_count)
			}
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				(vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::AABBS)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default()
					}),
				*transform_count)
			}
		};

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
			.geometries(&geometries)
		;

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(vk::AccelerationStructureBuildTypeKHR::DEVICE, &build_info, &[primitive_count], &mut size_info);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let scratch_size = size_info.build_scratch_size as usize;

		let buffer_descriptor = self.create_vulkan_buffer(None, acceleration_structure_size, vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer_descriptor.size, buffer_descriptor.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::GpuWrite);

		let (_, _) = self.bind_vulkan_buffer_memory(&buffer_descriptor, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer_descriptor.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

		let handle = graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure.create_acceleration_structure(&create_info, None).expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer_descriptor.resource,
				scratch_size,
			});
		}

		handle
	}

	fn write_instance(&mut self, instances_buffer: graphics_hardware_interface::BaseBufferHandle, instance_index: usize, transform: [[f32; 4]; 3], custom_index: u16, mask: u8, sbt_record_offset: usize, acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle) {
		let buffer = self.acceleration_structures[acceleration_structure.0 as usize].buffer;

		let address = unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) };

		let instance = vk::AccelerationStructureInstanceKHR{
			transform: vk::TransformMatrixKHR {
				matrix: [transform[0][0], transform[0][1], transform[0][2], transform[0][3], transform[1][0], transform[1][1], transform[1][2], transform[1][3], transform[2][0], transform[2][1], transform[2][2], transform[2][3]],
			},
			instance_custom_index_and_mask: vk::Packed24_8::new(custom_index as u32, mask),
			instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(sbt_record_offset as u32, vk::GeometryInstanceFlagsKHR::FORCE_OPAQUE.as_raw() as u8),
			acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
				device_handle: address,
			},
		};

		let instance_buffer = &mut self.buffers[instances_buffer.0 as usize];

		let instance_buffer_slice = unsafe { std::slice::from_raw_parts_mut(instance_buffer.pointer as *mut vk::AccelerationStructureInstanceKHR, instance_buffer.size / std::mem::size_of::<vk::AccelerationStructureInstanceKHR>()) };

		instance_buffer_slice[instance_index] = instance;
	}

	fn write_sbt_entry(&mut self, sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle, sbt_record_offset: usize, pipeline_handle: graphics_hardware_interface::PipelineHandle, shader_handle: graphics_hardware_interface::ShaderHandle) {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let shader_handles = pipeline.shader_handles.clone();

		self.get_mut_buffer_slice(sbt_buffer_handle)[sbt_record_offset..sbt_record_offset + 32].copy_from_slice(shader_handles.get(&shader_handle).unwrap())
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::ImageHandle, extent: Extent) {
		let image_handles = {
			let mut image_handles = Vec::with_capacity(3);
			let mut image_handle_option = Some(ImageHandle(image_handle.0));

			while let Some(image_handle) = image_handle_option {
				image_handles.push(image_handle);
				let image = &self.images[image_handle.0 as usize];
				image_handle_option = image.next;
			}

			image_handles
		};

		for image_handle in &image_handles {
			#[cfg(debug_assertions)]
			let Image { ref name, image: vk_image, image_view, format_, uses, layers, .. } = self.images[image_handle.0 as usize];
			#[cfg(not(debug_assertions))]
			let Image { image: vk_image, image_view, format_, uses, .. } = self.images[image_handle.0 as usize];
	
			unsafe {
				self.device.destroy_image(vk_image, None);
				self.device.destroy_image_view(image_view, None);
	
				// TODO: release memory/allocation
			}
	
			let size = (extent.width() * extent.height() * extent.depth()) as usize * format_.size();
	
			#[cfg(debug_assertions)]
			let r = self.create_vulkan_texture(name.as_ref().map(|s| s.as_str()), vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth()), format_, uses | graphics_hardware_interface::Uses::TransferSource, graphics_hardware_interface::DeviceAccesses::GpuRead, graphics_hardware_interface::AccessPolicies::WRITE, 1, layers);
			#[cfg(not(debug_assertions))]
			let r = self.create_vulkan_texture(None, vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth()), format_, uses | graphics_hardware_interface::Uses::TransferSource, graphics_hardware_interface::DeviceAccesses::GpuRead, graphics_hardware_interface::AccessPolicies::WRITE, 1, layers);
	
			let (allocation_handle, _) = self.create_allocation_internal(r.size, r.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::GpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);
	
			let (_, pointer) = self.bind_vulkan_texture_memory(&r, allocation_handle, 0);
	
			let image_view = self.create_vulkan_image_view(None, &r.resource, format_, 0, 0, layers);
	
			let image = &mut self.images[image_handle.0 as usize];
			image.pointer = pointer;
			image.size = size;
			image.extent = vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth());
			image.image_view = image_view;
			image.image = r.resource;
		}

		let mut entries = Vec::new();

		for image_handle in &image_handles {
			if let Some(bindings) = self.resource_to_descriptor.get(&Handle::Image(*image_handle)) {
				for &(binding_handle, descriptor_index) in bindings {
					let binding = &self.bindings[binding_handle.0 as usize];
			
					let descriptor_set_handles = {
						let mut descriptor_set_handles = Vec::with_capacity(3);
						let mut descriptor_set_handle_option = Some(DescriptorSetHandle(binding.descriptor_set_handle.0));
						while let Some(descriptor_set_handle) = descriptor_set_handle_option {
							descriptor_set_handles.push(descriptor_set_handle);
							let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
							descriptor_set_handle_option = descriptor_set.next;
						}
						descriptor_set_handles
					};

					let descriptor_type = binding.descriptor_type;
					let binding_index = binding.index;

					let resource_handle = Handle::Image(*image_handle);

					for (i, &descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
						let offset = None.unwrap_or(0);

						let descriptor = self.descriptors.get(&descriptor_set_handle).unwrap().get(&binding_index).unwrap().get(&descriptor_index).unwrap();

						let images = image_handles.iter().map(|ih| &self.images[ih.0 as usize]).collect::<Vec<_>>();
						let image = &images[((i as i32 - offset) % images.len() as i32) as usize];

						let images = match descriptor {
							Descriptor::Image { layout, .. } => {
								[
									vk::DescriptorImageInfo::default()
									.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, *layout, None))
									.image_view(image.image_view)
								]
							}
							Descriptor::CombinedImageSampler { sampler, layout, .. } => {
								[
									vk::DescriptorImageInfo::default()
									.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, *layout, None))
									.image_view(image.image_view)
									.sampler(*sampler)
								]
							}
							_ => { panic!("Invalid descriptor type"); }
						};

						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_index)
							.descriptor_type(descriptor_type)
							.image_info(&images)
						;

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
					}

					entries.push((resource_handle, (binding_handle, descriptor_index))); // Write into entries instead of straight into map because of borrow rules
				}
			}
		}

		for (k, v) in entries {
			self.resource_to_descriptor.entry(k).or_insert_with(HashSet::new).insert(v);
		}
	}

	fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let buffer_handle = BufferHandle(buffer_handle.0);

		let buffer = &self.buffers[buffer_handle.0 as usize];

		if buffer.size != 0 {
			todo!("Resize staging buffer");
		}

		if buffer.size >= size {
			return;
		}

		let buffer_creation_result = self.create_vulkan_buffer(None, size, uses_to_vk_usage_flags(buffer.uses) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS);

		let (allocation_handle, _) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let buffer = &mut self.buffers[buffer_handle.0 as usize];

		buffer.buffer = buffer_creation_result.resource;
		buffer.size = buffer_creation_result.size;
		buffer.device_address = address;
		buffer.pointer = pointer;

		let mut entries = Vec::new();

		if let Some(bindings) = self.resource_to_descriptor.get(&Handle::Buffer(buffer_handle)) {
			for &(binding_handle, descriptor_index) in bindings {
				let binding = &self.bindings[binding_handle.0 as usize];
		
				let descriptor_set_handle = DescriptorSetHandle(binding.descriptor_set_handle.0);

				let descriptor_type = binding.descriptor_type;
				let binding_index = binding.index;

				let descriptor_set_handles = {
					let mut descriptor_set_handles = Vec::with_capacity(3);

					let mut descriptor_set_handle_option = Some(descriptor_set_handle);

					while let Some(descriptor_set_handle) = descriptor_set_handle_option {
						descriptor_set_handles.push(descriptor_set_handle);
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						descriptor_set_handle_option = descriptor_set.next;
					}

					descriptor_set_handles
				};

				for &descriptor_set_handle in descriptor_set_handles.iter() {
					let descriptor = self.descriptors.get(&descriptor_set_handle).unwrap().get(&binding_index).unwrap().get(&descriptor_index).unwrap();

					let buffer = self.buffers[buffer_handle.0 as usize];

					let buffers = match descriptor {
						Descriptor::Buffer { size, .. } => {
							[
								vk::DescriptorBufferInfo::default().buffer(buffer.buffer).range(match size { graphics_hardware_interface::Ranges::Size(size) => { *size as u64 } graphics_hardware_interface::Ranges::Whole => { vk::WHOLE_SIZE } })
							]
						}
						_ => { panic!("Invalid descriptor type"); }
					};

					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

					let write_info = vk::WriteDescriptorSet::default()
						.dst_set(descriptor_set.descriptor_set)
						.dst_binding(binding_index)
						.dst_array_element(descriptor_index)
						.descriptor_type(descriptor_type)
						.buffer_info(&buffers)
					;

					unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
				}

				entries.push((Handle::Buffer(buffer_handle), (binding_handle, descriptor_index))); // Write into entries instead of straight into map because of borrow rules
			}
		}

		for (k, v) in entries {
			self.resource_to_descriptor.entry(k).or_insert_with(HashSet::new).insert(v);
		}
	}

	fn bind_to_window(&mut self, window_os_handles: &window::OSHandles, presentation_mode: graphics_hardware_interface::PresentationModes, fallback_extent: Extent) -> graphics_hardware_interface::SwapchainHandle {
		let surface = self.create_vulkan_surface(window_os_handles); 

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let extent = if surface_capabilities.current_extent.width != u32::MAX && surface_capabilities.current_extent.height != u32::MAX {
			surface_capabilities.current_extent
		} else {
			vk::Extent2D::default().width(fallback_extent.width()).height(fallback_extent.height())
		};

		let presentation_mode = match presentation_mode {
			graphics_hardware_interface::PresentationModes::FIFO => vk::PresentModeKHR::FIFO,
			graphics_hardware_interface::PresentationModes::Inmediate => vk::PresentModeKHR::IMMEDIATE,
			graphics_hardware_interface::PresentationModes::Mailbox => vk::PresentModeKHR::MAILBOX,
		};

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.surface(surface)
			.min_image_count(surface_capabilities.min_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(extent)
			.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(surface_capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(presentation_mode)
			.image_array_layers(1)
			.clipped(true)
		;

		let swapchain = unsafe { self.swapchain.create_swapchain(&swapchain_create_info, None).expect("No swapchain") };

		let swapchain_handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			surface,
			surface_present_mode: presentation_mode,
			swapchain,
		});

		swapchain_handle
	}

	fn get_image_data(&self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &[u8] {
		let image = &self.images[texture_copy_handle.0 as usize]; // This must always be the start image because that contains the buffer
		let buffer_handle = image.staging_buffer.expect("No staging buffer");
		let buffer = &self.buffers[buffer_handle.0 as usize];
		if buffer.pointer.is_null() { panic!("Texture data was requested but texture has no memory associated."); }
		let slice = unsafe { std::slice::from_raw_parts::<'static, u8>(buffer.pointer, (image.extent.width * image.extent.height * image.extent.depth) as usize) };
		slice
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		let synchronizer_handle = graphics_hardware_interface::SynchronizerHandle(self.synchronizers.len() as u64);

		{
			let mut previous: Option<SynchronizerHandle> = None;
	
			for _ in 0..self.frames {
				let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u64);

				self.synchronizers.push(Synchronizer {
					next: None,
					fence: self.create_vulkan_fence(signaled),
					vk_semaphore: self.create_vulkan_semaphore(name, signaled),
				});

				if let Some(pr) = previous {
					self.synchronizers[pr.0 as usize].next = Some(synchronizer_handle);
				}

				previous = Some(synchronizer_handle);
			}
		}

		synchronizer_handle
	}

	fn acquire_swapchain_image(&mut self, frame_index: u32, swapchain_handle: graphics_hardware_interface::SwapchainHandle, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> (graphics_hardware_interface::PresentKey, Option<Extent>) {
		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);

		let synchronizer = &self.synchronizers[synchronizer_handles[frame_index as usize].0 as usize];
		let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];

		let timeout = if true {
			std::time::Duration::from_secs(5).as_micros() as u64
		} else {
			u64::MAX
		};

		let acquisition_result = unsafe { self.swapchain.acquire_next_image(swapchain.swapchain, timeout, synchronizer.vk_semaphore, vk::Fence::null()) };

		let (index, swapchain_state) = if let Ok((index, is_suboptimal)) = acquisition_result {
			if !is_suboptimal {
				(index, graphics_hardware_interface::SwapchainStates::Ok)
			} else {
				(index, graphics_hardware_interface::SwapchainStates::Suboptimal)
			}
		} else {
			(0, graphics_hardware_interface::SwapchainStates::Invalid)
		};

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, swapchain.surface).expect("No surface capabilities") };

		if swapchain_state == graphics_hardware_interface::SwapchainStates::Suboptimal || swapchain_state == graphics_hardware_interface::SwapchainStates::Invalid {
			println!("Recreating swapchain");

			unsafe { // TODO: consider deadlock https://vulkan-tutorial.com/Drawing_a_triangle/Swap_chain_recreation
				self.device.device_wait_idle().unwrap();

				let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
					.surface(swapchain.surface)
					.min_image_count(surface_capabilities.min_image_count)
					.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
					.image_format(vk::Format::B8G8R8A8_SRGB)
					.image_extent(vk::Extent2D::default().width(1920).height(1080))
					.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
					.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
					.pre_transform(surface_capabilities.current_transform)
					.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
					.present_mode(swapchain.surface_present_mode)
					.image_array_layers(1)
					.clipped(true)
				;

				let new_swapchain = self.swapchain.create_swapchain(&swapchain_create_info, None).expect("No swapchain");

				self.swapchain.destroy_swapchain(swapchain.swapchain, None);

				swapchain.swapchain = new_swapchain;
			}
		}

		let extent = if surface_capabilities.current_extent.width != u32::MAX && surface_capabilities.current_extent.height != u32::MAX {
			Some(Extent::rectangle(surface_capabilities.current_extent.width, surface_capabilities.current_extent.height))
		} else {
			None
		};

		(graphics_hardware_interface::PresentKey(index), extent)
	}

	fn present(&self, frame_index: u32, image_index: graphics_hardware_interface::PresentKey, swapchains: &[graphics_hardware_interface::SwapchainHandle], synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = self.synchronizers[synchronizer_handles[frame_index as usize].0 as usize];

		let swapchains = swapchains.iter().map(|swapchain_handle| { let swapchain = &self.swapchains[swapchain_handle.0 as usize]; swapchain.swapchain }).collect::<Vec<_>>();
		let wait_semaphores = [synchronizer.vk_semaphore];

		let image_indices = [image_index.0];

		let mut results = [vk::Result::default()];

  		let present_info = vk::PresentInfoKHR::default()
			.results(&mut results)
			.swapchains(&swapchains)
			.wait_semaphores(&wait_semaphores)
			.image_indices(&image_indices)
		;

		let is_suboptimal = unsafe { self.swapchain.queue_present(self.queue, &present_info).expect("No present") };

		if is_suboptimal {
			println!("¡¡Suboptimal present!!");
		}
	}

	fn wait(&self, frame_index: u32, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = self.synchronizers[synchronizer_handles[frame_index as usize].0 as usize];
		unsafe { self.device.wait_for_fences(&[synchronizer.fence], true, u64::MAX).expect("No fence wait"); }
		unsafe { self.device.reset_fences(&[synchronizer.fence]).expect("No fence reset"); }
	}

	#[inline]
	fn start_frame_capture(&self) {
		#[cfg(debug_assertions)]
		self.debugger.start_frame_capture();
	}

	#[inline]
	fn end_frame_capture(&self) {
		#[cfg(debug_assertions)]
		self.debugger.end_frame_capture();
	}
}

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

#[derive(Clone)]
pub(crate) struct PipelineLayout {
	pipeline_layout: vk::PipelineLayout,
	descriptor_set_template_indices: HashMap<graphics_hardware_interface::DescriptorSetTemplateHandle, u32>,
}

#[derive(Clone)]
pub(crate) struct DescriptorSet {
	next: Option<DescriptorSetHandle>,
	descriptor_set: vk::DescriptorSet,
	descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,

	// resources: Vec<(graphics_hardware_interface::DescriptorSetBindingHandle, Handle)>,
}

#[derive(Clone)]
pub(crate) struct Shader {
	shader: vk::ShaderModule,
	stage: graphics_hardware_interface::Stages,
	shader_binding_descriptors: Vec<graphics_hardware_interface::ShaderBindingDescriptor>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
	pipeline: vk::Pipeline,
	shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	shaders: Vec<graphics_hardware_interface::ShaderHandle>,
	resource_access: Vec<((u32, u32), (graphics_hardware_interface::Stages, graphics_hardware_interface::AccessPolicies))>,
}

#[derive(Clone, Copy)]
pub(crate) struct CommandBufferInternal {
	command_pool: vk::CommandPool,
	command_buffer: vk::CommandBuffer,
}

#[derive(Clone)]
pub(crate) struct Binding {
	descriptor_set_handle: graphics_hardware_interface::DescriptorSetHandle,
	type_: graphics_hardware_interface::DescriptorType,
	descriptor_type: vk::DescriptorType,
	stages: graphics_hardware_interface::Stages,
	pipeline_stages: vk::PipelineStageFlags2,
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
	next: Option<BufferHandle>,
	staging: Option<BufferHandle>,
	buffer: vk::Buffer,
	size: usize,
	device_address: vk::DeviceAddress,
	pointer: *mut u8,
	uses: graphics_hardware_interface::Uses,
	use_cases: Option<graphics_hardware_interface::UseCases>,
	frame: Option<u8>,
}

unsafe impl Send for Buffer {}

#[derive(Clone, Copy)]
pub(crate) struct Synchronizer {
	next: Option<SynchronizerHandle>,
	fence: vk::Fence,
	vk_semaphore: vk::Semaphore,
}

#[derive(Clone)]
pub(crate) struct Image {
	#[cfg(debug_assertions)]
	name: Option<String>,
	next: Option<ImageHandle>,
	staging_buffer: Option<BufferHandle>,
	allocation_handle: graphics_hardware_interface::AllocationHandle,
	image: vk::Image,
	image_view: vk::ImageView,
	image_views: [vk::ImageView; 8],
	pointer: *const u8,
	extent: vk::Extent3D,
	format: vk::Format,
	format_: graphics_hardware_interface::Formats,
	layout: vk::ImageLayout,
	size: usize,
	uses: graphics_hardware_interface::Uses,
	layers: u32,
}

unsafe impl Send for Image {}

// #[derive(Clone, Copy)]
// pub(crate) struct AccelerationStructure {
// 	acceleration_structure: vk::AccelerationStructureKHR,
// }

unsafe extern "system" fn vulkan_debug_utils_callback(message_severity: vk::DebugUtilsMessageSeverityFlagsEXT, _message_type: vk::DebugUtilsMessageTypeFlagsEXT, p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT, p_user_data: *mut std::ffi::c_void,) -> vk::Bool32 {
	let callback_data = if let Some(callback_data) = p_callback_data.as_ref() { callback_data } else { return vk::FALSE; };
	
	if callback_data.p_message.is_null() {
		return vk::FALSE;
	}

    let message = std::ffi::CStr::from_ptr(callback_data.p_message);

	let message = if let Some(message) = message.to_str().ok() { message } else { return vk::FALSE; };

	let user_data = if let Some(p_user_data) = (p_user_data as *mut DebugCallbackData).as_mut() { p_user_data } else { return vk::FALSE; };

	match message_severity {
		vk::DebugUtilsMessageSeverityFlagsEXT::INFO => {
			// debug!("{}", message.to_str().unwrap());
		}
		vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => {
			// warn!("{}", message.to_str().unwrap());
		}
		vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
			(user_data.error_log_function)(message);
			user_data.error_count += 1;
		}
		_ => {}
	}

    vk::FALSE
}

fn uses_to_vk_usage_flags(usage: graphics_hardware_interface::Uses) -> vk::BufferUsageFlags {
	let mut flags = vk::BufferUsageFlags::empty();
	flags |= if usage.contains(graphics_hardware_interface::Uses::Vertex) { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Index) { vk::BufferUsageFlags::INDEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Uniform) { vk::BufferUsageFlags::UNIFORM_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Storage) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::TransferSource) { vk::BufferUsageFlags::TRANSFER_SRC } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::TransferDestination) { vk::BufferUsageFlags::TRANSFER_DST } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructure) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Indirect) { vk::BufferUsageFlags::INDIRECT_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::ShaderBindingTable) { vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructureBuildScratch) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructureBuild) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR } else { vk::BufferUsageFlags::empty() };
	flags
}

fn to_clear_value(clear: graphics_hardware_interface::ClearValue) -> vk::ClearValue {
	match clear {
		graphics_hardware_interface::ClearValue::None => vk::ClearValue::default(),
		graphics_hardware_interface::ClearValue::Color(clear) => vk::ClearValue { color: vk::ClearColorValue { float32: [clear.r, clear.g, clear.b, clear.a], }, },
		graphics_hardware_interface::ClearValue::Depth(clear) => vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: clear, stencil: 0, }, },
		graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearValue { color: vk::ClearColorValue { uint32: [r, g, b, a], }, },
	}
}

fn texture_format_and_resource_use_to_image_layout(texture_format: graphics_hardware_interface::Formats, layout: graphics_hardware_interface::Layouts, access: Option<graphics_hardware_interface::AccessPolicies>) -> vk::ImageLayout {
	match layout {
		graphics_hardware_interface::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		graphics_hardware_interface::Layouts::RenderTarget => if texture_format != graphics_hardware_interface::Formats::Depth32 { vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL } else { vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL },
		graphics_hardware_interface::Layouts::Transfer => {
			match access {
				Some(a) => {
					if a.intersects(graphics_hardware_interface::AccessPolicies::READ) {
						vk::ImageLayout::TRANSFER_SRC_OPTIMAL
					} else if a.intersects(graphics_hardware_interface::AccessPolicies::WRITE) {
						vk::ImageLayout::TRANSFER_DST_OPTIMAL
					} else {
						vk::ImageLayout::UNDEFINED
					}
				}
				None => vk::ImageLayout::UNDEFINED
			}
		}
		graphics_hardware_interface::Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		graphics_hardware_interface::Layouts::Read => {
			if texture_format != graphics_hardware_interface::Formats::Depth32 { vk::ImageLayout::READ_ONLY_OPTIMAL } else { vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL }
		},
		graphics_hardware_interface::Layouts::General => vk::ImageLayout::GENERAL,
		graphics_hardware_interface::Layouts::ShaderBindingTable => vk::ImageLayout::UNDEFINED,
		graphics_hardware_interface::Layouts::Indirect => vk::ImageLayout::UNDEFINED,
	}
}

fn to_load_operation(value: bool) -> vk::AttachmentLoadOp {	if value { vk::AttachmentLoadOp::LOAD } else { vk::AttachmentLoadOp::CLEAR } }

fn to_store_operation(value: bool) -> vk::AttachmentStoreOp { if value { vk::AttachmentStoreOp::STORE } else { vk::AttachmentStoreOp::DONT_CARE } }

fn to_format(format: graphics_hardware_interface::Formats) -> vk::Format {
	match format {
		graphics_hardware_interface::Formats::R8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8_SNORM }
			}
		}
		graphics_hardware_interface::Formats::R16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16_SNORM }
			}
		}
		graphics_hardware_interface::Formats::R32(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R32_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R32_UINT }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R32_SINT }
			}
		}
		graphics_hardware_interface::Formats::RG8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RG16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RGB8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8B8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8B8_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RGB16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16B16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16B16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16B16_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RGBA8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8B8A8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8B8A8_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RGBA16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16B16A16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16B16A16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16B16A16_SNORM }
			}
		}
		graphics_hardware_interface::Formats::RGBu10u10u11 => vk::Format::B10G11R11_UFLOAT_PACK32,
		graphics_hardware_interface::Formats::BGRAu8 => vk::Format::B8G8R8A8_SRGB,
		graphics_hardware_interface::Formats::Depth32 => vk::Format::D32_SFLOAT,
		graphics_hardware_interface::Formats::U32 => vk::Format::R32_UINT,
		graphics_hardware_interface::Formats::BC5 => vk::Format::BC5_UNORM_BLOCK,
		graphics_hardware_interface::Formats::BC7 => vk::Format::BC7_SRGB_BLOCK,
	}
}

fn to_shader_stage_flags(shader_type: graphics_hardware_interface::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		graphics_hardware_interface::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		graphics_hardware_interface::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		graphics_hardware_interface::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,
		graphics_hardware_interface::ShaderTypes::Task => vk::ShaderStageFlags::TASK_EXT,
		graphics_hardware_interface::ShaderTypes::Mesh => vk::ShaderStageFlags::MESH_EXT,
		graphics_hardware_interface::ShaderTypes::RayGen => vk::ShaderStageFlags::RAYGEN_KHR,
		graphics_hardware_interface::ShaderTypes::ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
		graphics_hardware_interface::ShaderTypes::AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
		graphics_hardware_interface::ShaderTypes::Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
		graphics_hardware_interface::ShaderTypes::Miss => vk::ShaderStageFlags::MISS_KHR,
		graphics_hardware_interface::ShaderTypes::Callable => vk::ShaderStageFlags::CALLABLE_KHR,
	}
}

fn to_pipeline_stage_flags(stages: graphics_hardware_interface::Stages, layout: Option<graphics_hardware_interface::Layouts>, format: Option<graphics_hardware_interface::Formats>) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(graphics_hardware_interface::Stages::VERTEX) { pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER }

	if stages.contains(graphics_hardware_interface::Stages::MESH) { pipeline_stage_flags |= vk::PipelineStageFlags2::MESH_SHADER_EXT; }

	if stages.contains(graphics_hardware_interface::Stages::FRAGMENT) {
		if let Some(layout) = layout {
			if layout == graphics_hardware_interface::Layouts::Read {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}

			if layout == graphics_hardware_interface::Layouts::RenderTarget {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
			}

			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
				} else {
					pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
					pipeline_stage_flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
				}
			}
		} else {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
				} else {
					pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
					pipeline_stage_flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
				}
			} else {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}
		}
	}

	if stages.contains(graphics_hardware_interface::Stages::COMPUTE) {
		if let Some(layout) = layout {
			if layout == graphics_hardware_interface::Layouts::Indirect {
				pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT
			} else {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
			}
		} else {
			pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
		}
	}

	if stages.contains(graphics_hardware_interface::Stages::TRANSFER) { pipeline_stage_flags |= vk::PipelineStageFlags2::TRANSFER 	}
	if stages.contains(graphics_hardware_interface::Stages::PRESENTATION) { pipeline_stage_flags |= vk::PipelineStageFlags2::TOP_OF_PIPE }
	if stages.contains(graphics_hardware_interface::Stages::RAYGEN) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::CLOSEST_HIT) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::ANY_HIT) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::INTERSECTION) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::MISS) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::CALLABLE) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) { pipeline_stage_flags |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR; }

	pipeline_stage_flags
}

fn to_access_flags(accesses: graphics_hardware_interface::AccessPolicies, stages: graphics_hardware_interface::Stages, layout: graphics_hardware_interface::Layouts, format: Option<graphics_hardware_interface::Formats>) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(graphics_hardware_interface::AccessPolicies::READ) {
		if stages.intersects(graphics_hardware_interface::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(graphics_hardware_interface::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(graphics_hardware_interface::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				} else {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::COMPUTE) {
			if layout == graphics_hardware_interface::Layouts::Indirect {
				access_flags |= vk::AccessFlags2::INDIRECT_COMMAND_READ
			} else {
				access_flags |= vk::AccessFlags2::SHADER_READ
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::RAYGEN) {
			if layout == graphics_hardware_interface::Layouts::ShaderBindingTable {
				access_flags |= vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR
			} else {
				access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
		}
	}

	if accesses.contains(graphics_hardware_interface::AccessPolicies::WRITE) {
		if stages.intersects(graphics_hardware_interface::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				} else {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::RAYGEN) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR
		}
	}

	access_flags
}

impl Into<vk::ShaderStageFlags> for graphics_hardware_interface::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::VERTEX) { vk::ShaderStageFlags::VERTEX } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::FRAGMENT) { vk::ShaderStageFlags::FRAGMENT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::COMPUTE) { vk::ShaderStageFlags::COMPUTE } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::MESH) { vk::ShaderStageFlags::MESH_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::TASK) { vk::ShaderStageFlags::TASK_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::RAYGEN) { vk::ShaderStageFlags::RAYGEN_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::CLOSEST_HIT) { vk::ShaderStageFlags::CLOSEST_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::ANY_HIT) { vk::ShaderStageFlags::ANY_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::INTERSECTION) { vk::ShaderStageFlags::INTERSECTION_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::MISS) { vk::ShaderStageFlags::MISS_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::CALLABLE) { vk::ShaderStageFlags::CALLABLE_KHR } else { vk::ShaderStageFlags::default() };

		shader_stage_flags
	}
}

impl Into<vk::Format> for graphics_hardware_interface::DataTypes {
	fn into(self) -> vk::Format {
		match self {
			graphics_hardware_interface::DataTypes::Float => vk::Format::R32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float2 => vk::Format::R32G32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float3 => vk::Format::R32G32B32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float4 => vk::Format::R32G32B32A32_SFLOAT,
			graphics_hardware_interface::DataTypes::U8 => vk::Format::R8_UINT,
			graphics_hardware_interface::DataTypes::U16 => vk::Format::R16_UINT,
			graphics_hardware_interface::DataTypes::Int => vk::Format::R32_SINT,
			graphics_hardware_interface::DataTypes::U32 => vk::Format::R32_UINT,
			graphics_hardware_interface::DataTypes::Int2 => vk::Format::R32G32_SINT,
			graphics_hardware_interface::DataTypes::Int3 => vk::Format::R32G32B32_SINT,
			graphics_hardware_interface::DataTypes::Int4 => vk::Format::R32G32B32A32_SINT,
			graphics_hardware_interface::DataTypes::UInt => vk::Format::R32_UINT,
			graphics_hardware_interface::DataTypes::UInt2 => vk::Format::R32G32_UINT,
			graphics_hardware_interface::DataTypes::UInt3 => vk::Format::R32G32B32_UINT,
			graphics_hardware_interface::DataTypes::UInt4 => vk::Format::R32G32B32A32_UINT,
		}
	}
}

impl Size for graphics_hardware_interface::DataTypes {
	fn size(&self) -> usize {
		match self {
			graphics_hardware_interface::DataTypes::Float => std::mem::size_of::<f32>(),
			graphics_hardware_interface::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			graphics_hardware_interface::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			graphics_hardware_interface::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			graphics_hardware_interface::DataTypes::U8 => std::mem::size_of::<u8>(),
			graphics_hardware_interface::DataTypes::U16 => std::mem::size_of::<u16>(),
			graphics_hardware_interface::DataTypes::U32 => std::mem::size_of::<u32>(),
			graphics_hardware_interface::DataTypes::Int => std::mem::size_of::<i32>(),
			graphics_hardware_interface::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			graphics_hardware_interface::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			graphics_hardware_interface::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			graphics_hardware_interface::DataTypes::UInt => std::mem::size_of::<u32>(),
			graphics_hardware_interface::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			graphics_hardware_interface::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			graphics_hardware_interface::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}
}

impl Size for &[graphics_hardware_interface::VertexElement] {
	fn size(&self) -> usize {
		let mut size = 0;

		for element in *self {
			size += element.format.size();
		}

		size
	}
}

impl Into<graphics_hardware_interface::Stages> for graphics_hardware_interface::ShaderTypes {
	fn into(self) -> graphics_hardware_interface::Stages {
		match self {
			graphics_hardware_interface::ShaderTypes::Vertex => graphics_hardware_interface::Stages::VERTEX,
			graphics_hardware_interface::ShaderTypes::Fragment => graphics_hardware_interface::Stages::FRAGMENT,
			graphics_hardware_interface::ShaderTypes::Compute => graphics_hardware_interface::Stages::COMPUTE,
			graphics_hardware_interface::ShaderTypes::Task => graphics_hardware_interface::Stages::TASK,
			graphics_hardware_interface::ShaderTypes::Mesh => graphics_hardware_interface::Stages::MESH,
			graphics_hardware_interface::ShaderTypes::RayGen => graphics_hardware_interface::Stages::RAYGEN,
			graphics_hardware_interface::ShaderTypes::ClosestHit => graphics_hardware_interface::Stages::CLOSEST_HIT,
			graphics_hardware_interface::ShaderTypes::AnyHit => graphics_hardware_interface::Stages::ANY_HIT,
			graphics_hardware_interface::ShaderTypes::Intersection => graphics_hardware_interface::Stages::INTERSECTION,
			graphics_hardware_interface::ShaderTypes::Miss => graphics_hardware_interface::Stages::MISS,
			graphics_hardware_interface::ShaderTypes::Callable => graphics_hardware_interface::Stages::CALLABLE,
		}
	}
}

struct DebugCallbackData {
	error_count: u64,
	error_log_function: fn(&str),
}

impl VulkanGHI {
	pub fn new(settings: graphics_hardware_interface::Features) -> Result<VulkanGHI, ()> {
		let entry = ash::Entry::linked();

		let available_instance_extensions = unsafe { entry.enumerate_instance_extension_properties(None).unwrap() };

		let is_instance_extension_available = |name: &str| {
			available_instance_extensions.iter().any(|extension| {
				unsafe { std::ffi::CStr::from_ptr(extension.extension_name.as_ptr()).to_str().unwrap() == name }
			})
		};

		let application_info = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 3, 0));

		let mut layer_names = Vec::new();
		
		if settings.validation {
			layer_names.push(std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap().as_ptr());
		}

		if settings.api_dump {
			layer_names.push(std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_LUNARG_api_dump\0").unwrap().as_ptr());
		}

		let mut extension_names = Vec::new();
		
		extension_names.push(ash::khr::surface::NAME.as_ptr());

		#[cfg(target_os = "linux")]
		{
			if is_instance_extension_available(ash::khr::xlib_surface::NAME.to_str().unwrap()) {
				extension_names.push(ash::khr::xlib_surface::NAME.as_ptr());
			}

			if is_instance_extension_available(ash::khr::xcb_surface::NAME.to_str().unwrap()) {
				extension_names.push(ash::khr::xcb_surface::NAME.as_ptr());
			}

			if is_instance_extension_available(ash::khr::wayland_surface::NAME.to_str().unwrap()) {
				extension_names.push(ash::khr::wayland_surface::NAME.as_ptr());
			}
		}

		#[cfg(target_os = "windows")]
		{
			if is_instance_extension_available(ash::khr::win32_surface::NAME.to_str().unwrap()) {
				extension_names.push(ash::khr::win32_surface::NAME.as_ptr());
			}
		}

		if settings.validation {
			extension_names.push(ash::ext::debug_utils::NAME.as_ptr());
		}

		let enabled_validation_features = {
			let mut enabled_features = Vec::with_capacity(6);
			enabled_features.push(vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION);
			enabled_features.push(vk::ValidationFeatureEnableEXT::BEST_PRACTICES);
			if settings.gpu_validation { enabled_features.push(vk::ValidationFeatureEnableEXT::GPU_ASSISTED); }
			enabled_features
		};

		let mut validation_features = vk::ValidationFeaturesEXT::default()
			.enabled_validation_features(&enabled_validation_features);

		let instance_create_info = vk::InstanceCreateInfo::default()
			.application_info(&application_info)
			.enabled_layer_names(&layer_names)
			.enabled_extension_names(&extension_names)
		;

		let instance_create_info = if settings.validation {
			instance_create_info.push_next(&mut validation_features)
		} else {
			instance_create_info
		};

		let instance = unsafe { entry.create_instance(&instance_create_info, None).or(Err(()))? };

		let mut debug_data = Box::new(DebugCallbackData {
			error_count: 0,
			error_log_function: settings.debug_log_function.unwrap_or(|message| { println!("{}", message); }),
		});

		let debug_utils_messenger = if settings.validation {
			let debug_utils = ash::ext::debug_utils::Instance::new(&entry, &instance);

			let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
				.message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::INFO | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,)
				.message_type(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,)
				.pfn_user_callback(Some(vulkan_debug_utils_callback))
				.user_data(debug_data.as_mut() as *mut DebugCallbackData as *mut std::ffi::c_void)
			;

			let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).or(Err(()))? };

			Some(debug_utils_messenger)
		} else {
			None
		};

		let physical_devices = unsafe { instance.enumerate_physical_devices().or(Err(()))? };

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

			physical_device = *best_physical_device.ok_or(())?;
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
			.expect("No queue family index found.")
		;

		let mut device_extension_names = Vec::new();
		
		device_extension_names.push(ash::khr::swapchain::NAME.as_ptr());

		if settings.ray_tracing {
			device_extension_names.push(ash::khr::acceleration_structure::NAME.as_ptr());
			device_extension_names.push(ash::khr::deferred_host_operations::NAME.as_ptr());
			device_extension_names.push(ash::khr::ray_tracing_pipeline::NAME.as_ptr());
			device_extension_names.push(ash::khr::ray_tracing_maintenance1::NAME.as_ptr());
		}

		device_extension_names.push(ash::ext::mesh_shader::NAME.as_ptr());

		let queue_create_infos = [vk::DeviceQueueCreateInfo::default()
			.queue_family_index(queue_family_index)
			.queue_priorities(&[1.0])
		];

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
			.sampler_filter_minmax(true)
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

		let mut shader_atomic_float_features = vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT::default()
			.shader_buffer_float32_atomics(true)
			.shader_image_float32_atomics(true)
		;

		device_extension_names.push("VK_EXT_shader_atomic_float\0".as_ptr() as *const i8);

  		let device_create_info = vk::DeviceCreateInfo::default()
			.push_next(&mut physical_device_vulkan_11_features)
			.push_next(&mut physical_device_vulkan_12_features)
			.push_next(&mut physical_device_vulkan_13_features)
			.push_next(&mut physical_device_mesh_shading_features)
			.push_next(&mut shader_atomic_float_features)
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names)
			.enabled_features(&enabled_physical_device_features)
		;

		let device_create_info = if settings.ray_tracing {
			device_create_info
				.push_next(&mut physical_device_acceleration_structure_features)
				.push_next(&mut physical_device_ray_tracing_pipeline_features)
		} else {
			device_create_info
		};

		let device: ash::Device = unsafe { instance.create_device(physical_device, &device_create_info, None).or(Err(()))? };

		let queue = unsafe { device.get_device_queue(queue_family_index, 0) };

		let acceleration_structure = ash::khr::acceleration_structure::Device::new(&instance, &device);
		let ray_tracing_pipeline = ash::khr::ray_tracing_pipeline::Device::new(&instance, &device);

		let swapchain = ash::khr::swapchain::Device::new(&instance, &device);
		let surface = ash::khr::surface::Instance::new(&entry, &instance);

		let mesh_shading = ash::ext::mesh_shader::Device::new(&instance, &device);

		let debug_utils = if settings.validation {
			Some(ash::ext::debug_utils::Device::new(&instance, &device))
		} else {
			None
		};

		Ok(VulkanGHI { 
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

			#[cfg(debug_assertions)]
			debugger: RenderDebugger::new(),

			frames: 2, // Assuming double buffering

			allocations: Vec::new(),
			buffers: Vec::with_capacity(1024),
			images: Vec::with_capacity(512),
			descriptor_sets_layouts: Vec::with_capacity(128),
			pipeline_layouts: Vec::with_capacity(64),
			bindings: Vec::with_capacity(1024),
			descriptor_sets: Vec::with_capacity(512),
			acceleration_structures: Vec::new(),
			shaders: Vec::with_capacity(1024),
			pipelines: Vec::with_capacity(1024),
			meshes: Vec::new(),
			command_buffers: Vec::with_capacity(32),
			synchronizers: Vec::with_capacity(32),
			swapchains: Vec::with_capacity(4),

			resource_to_descriptor: HashMap::with_capacity(4096),
			descriptors: HashMap::with_capacity(4096),
			descriptor_set_to_resource: HashMap::with_capacity(4096),

			settings,

			states: HashMap::with_capacity(4096),

			pending_images: Vec::with_capacity(128),
			pending_buffers: Vec::with_capacity(128),
		})
	}

	#[cfg(debug_assertions)]
	fn get_log_count(&self) -> u64 { self.debug_data.error_count }

	fn get_syncronizer_handles(&self, synchroizer_handle: graphics_hardware_interface::SynchronizerHandle) -> Vec<SynchronizerHandle> {
		let mut synchronizer_handles = Vec::with_capacity(3);
		let mut synchronizer_handle = Some(SynchronizerHandle(synchroizer_handle.0));
		while let Some(sh) = synchronizer_handle {
			synchronizer_handles.push(sh);
			synchronizer_handle = self.synchronizers[sh.0 as usize].next;
		}
		synchronizer_handles
	}

	fn create_vulkan_pipeline(&mut self, blocks: &[graphics_hardware_interface::PipelineConfigurationBlocks]) -> graphics_hardware_interface::PipelineHandle {
		/// This function calls itself recursively to build the pipeline the pipeline states.
		/// This is done because this way we can "dynamically" allocate on the stack the needed structs because the recursive call keep them alive.
		fn build_block(ghi: &VulkanGHI, pipeline_create_info: vk::GraphicsPipelineCreateInfo<'_>, mut block_iterator: std::slice::Iter<'_, graphics_hardware_interface::PipelineConfigurationBlocks>) -> vk::Pipeline {
			if let Some(block) = block_iterator.next() {
				match block {
					graphics_hardware_interface::PipelineConfigurationBlocks::VertexInput { vertex_elements } => {
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

						build_block(ghi, pipeline_create_info, block_iterator)
					}
					graphics_hardware_interface::PipelineConfigurationBlocks::InputAssembly {  } => {
						let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
							.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
							.primitive_restart_enable(false);

						let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

						build_block(ghi, pipeline_create_info, block_iterator)
					}
					graphics_hardware_interface::PipelineConfigurationBlocks::RenderTargets { targets } => {
						let pipeline_color_blend_attachments = targets.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|_| {
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
	
						let color_attachement_formats: Vec<vk::Format> = targets.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|a| to_format(a.format)).collect::<Vec<_>>();

						let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
							.logic_op_enable(false)
							.logic_op(vk::LogicOp::COPY)
							.attachments(&pipeline_color_blend_attachments)
							.blend_constants([0.0, 0.0, 0.0, 0.0]);

						let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
							.color_attachment_formats(&color_attachement_formats)
							.depth_attachment_format(vk::Format::UNDEFINED);

						let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

						if let Some(_) = targets.iter().find(|a| a.format == graphics_hardware_interface::Formats::Depth32) {
							let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
								.depth_test_enable(true)
								.depth_write_enable(true)
								.depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
								.depth_bounds_test_enable(false)
								.stencil_test_enable(false)
								.front(vk::StencilOpState::default())
								.back(vk::StencilOpState::default())
							;

							rendering_info = rendering_info.depth_attachment_format(vk::Format::D32_SFLOAT);

							let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);
							let pipeline_create_info = pipeline_create_info.depth_stencil_state(&depth_stencil_state);

							build_block(ghi, pipeline_create_info, block_iterator)
						} else {
							let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);

							build_block(ghi, pipeline_create_info, block_iterator)
						}
					}
					graphics_hardware_interface::PipelineConfigurationBlocks::Shaders { shaders } => {
						let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);
						let mut entries = [vk::SpecializationMapEntry::default(); 32];
						let mut entry_count = 0;
						let specilization_info_count = 0;

						let stages = shaders
							.iter()
							.map(move |stage| {
								for entry in stage.specialization_map.iter() {
									specialization_entries_buffer.extend_from_slice(entry.get_data());

									entries[entry_count] = vk::SpecializationMapEntry::default()
										.constant_id(entry.get_constant_id())
										.size(entry.get_size())
										.offset(specialization_entries_buffer.len() as u32);

									entry_count += 1;
								}

								let shader = &ghi.shaders[stage.handle.0 as usize];

								assert!(specilization_info_count == 0);

								vk::PipelineShaderStageCreateInfo::default()
									.stage(to_shader_stage_flags(stage.stage))
									.module(shader.shader)
									.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
							})
							.collect::<Vec<_>>();

						let pipeline_create_info = pipeline_create_info.stages(&stages);

						build_block(ghi, pipeline_create_info, block_iterator)
					}
					graphics_hardware_interface::PipelineConfigurationBlocks::Layout { layout } => {
						let pipeline_layout = &ghi.pipeline_layouts[layout.0 as usize];

						let pipeline_create_info = pipeline_create_info.layout(pipeline_layout.pipeline_layout);

						build_block(ghi, pipeline_create_info, block_iterator)
					}
				}
			} else {
				let pipeline_create_infos = [pipeline_create_info];

				let pipelines = unsafe { ghi.device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None).expect("No pipeline") };

				pipelines[0]
			}
		}

		let viewports = [vk::Viewport::default().x(0.0).y(9.0).width(16.0).height(9.0).min_depth(0.0).max_depth(1.0)];

		let scissors = [vk::Rect2D::default().offset(vk::Offset2D { x: 0, y: 0 }).extent(vk::Extent2D { width: 16, height: 9 })];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors)
		;

		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
			.dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR])
		;

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
			.line_width(1.0)
		;

		let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
			.sample_shading_enable(false)
			.rasterization_samples(vk::SampleCountFlags::TYPE_1)
			.min_sample_shading(1.0)
			.alpha_to_coverage_enable(false)
			.alpha_to_one_enable(false)
		;

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false)
		;

		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
			.dynamic_state(&dynamic_state)
			.viewport_state(&viewport_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state)
		;

		let pipeline = build_block(self, pipeline_create_info, blocks.iter());

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		let resource_access: Vec<((u32, u32), (graphics_hardware_interface::Stages, graphics_hardware_interface::AccessPolicies))> = blocks.iter().find_map(|b| {
			match b {
				graphics_hardware_interface::PipelineConfigurationBlocks::Shaders { shaders } => {
					Some(shaders.iter().map(|s| {
						let shader = &self.shaders[s.handle.0 as usize];
						shader.shader_binding_descriptors.iter().map(|sbd| {
							((sbd.set, sbd.binding), (Into::<graphics_hardware_interface::Stages>::into(s.stage), sbd.access))
						}).collect::<Vec<_>>()
					}))
				},
				_ => None,
			}
		}).unwrap().flatten().collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline,
			shader_handles: HashMap::new(),
			shaders: Vec::new(),
			resource_access,
		});

		handle
	}

	fn create_vulkan_buffer(&self, name: Option<&str>, size: usize, usage: vk::BufferUsageFlags) -> MemoryBackedResourceCreationResult<vk::Buffer> {
		let buffer_create_info = vk::BufferCreateInfo::default()
			.size(size as u64)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(usage);

		let buffer = unsafe { self.device.create_buffer(&buffer_create_info, None).expect("No buffer") };

		self.set_name(buffer, name);

		let memory_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

		MemoryBackedResourceCreationResult {
			resource: buffer,
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	fn destroy_vulkan_buffer(&self, buffer: &graphics_hardware_interface::BaseBufferHandle) {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.destroy_buffer(buffer, None) };
	}

	fn create_vulkan_allocation(&self, size: usize,) -> vk::DeviceMemory {
		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(0)
		;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		memory
	}

	fn get_vulkan_buffer_address(&self, buffer: &graphics_hardware_interface::BaseBufferHandle, _allocation: &graphics_hardware_interface::AllocationHandle) -> u64 {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) }
	}

	fn create_vulkan_texture(&self, name: Option<&str>, extent: vk::Extent3D, format: graphics_hardware_interface::Formats, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses, _access_policies: graphics_hardware_interface::AccessPolicies, mip_levels: u32, array_layers: u32) -> MemoryBackedResourceCreationResult<vk::Image> {
		let image_create_info = vk::ImageCreateInfo::default()
			.image_type(image_type_from_extent(extent).expect("Failed to get VkImageType from extent"))
			.format(to_format(format))
			.extent(extent)
			.mip_levels(mip_levels)
			.array_layers(array_layers)
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(vk::ImageTiling::OPTIMAL)
			.usage(into_vk_image_usage_flags(resource_uses, format))
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED)
		;

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };

		let memory_requirements = unsafe { self.device.get_image_memory_requirements(image) };		

		self.set_name(image, name);

		MemoryBackedResourceCreationResult {
			resource: image.to_owned(),
			size: memory_requirements.size as usize,
			alignment: memory_requirements.alignment as usize,
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	fn create_vulkan_sampler(&self, min_mag_filter: vk::Filter, reduction_mode: vk::SamplerReductionMode, mip_map_filter: vk::SamplerMipmapMode, address_mode: vk::SamplerAddressMode, anisotropy: Option<f32>, min_lod: f32, max_lod: f32) -> vk::Sampler {
		let mut vk_sampler_reduction_mode_create_info = vk::SamplerReductionModeCreateInfo::default().reduction_mode(reduction_mode);

		let sampler_create_info = vk::SamplerCreateInfo::default()
			.push_next(&mut vk_sampler_reduction_mode_create_info)
			.mag_filter(min_mag_filter)
			.min_filter(min_mag_filter)
			.mipmap_mode(mip_map_filter)
			.address_mode_u(address_mode)
			.address_mode_v(address_mode)
			.address_mode_w(address_mode)
			.border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK)
			.anisotropy_enable(anisotropy.is_some())
			.max_anisotropy(anisotropy.unwrap_or(0f32))
			.compare_enable(false)
			.compare_op(vk::CompareOp::NEVER)
			.min_lod(min_lod)
			.max_lod(max_lod)
			.mip_lod_bias(0.0)
			.unnormalized_coordinates(false)
		;

		let sampler = unsafe { self.device.create_sampler(&sampler_create_info, None).expect("No sampler") };

		sampler
	}

	fn get_image_subresource_layout(&self, texture: &graphics_hardware_interface::ImageHandle, mip_level: u32) -> graphics_hardware_interface::ImageSubresourceLayout {
		let image_subresource = vk::ImageSubresource {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			mip_level,
			array_layer: 0,
		};

		let texture = self.images.get(texture.0 as usize).expect("No texture with that handle.");

		if true /* TILING_OPTIMAL */ {
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: 0,
				size: texture.size,
				row_pitch: texture.extent.width as usize * texture.format_.size(),
				array_pitch: texture.extent.width as usize * texture.extent.height as usize * texture.format_.size(),
				depth_pitch: texture.extent.width as usize * texture.extent.height as usize * texture.extent.depth as usize * texture.format_.size(),
			}
		} else {
			let image_subresource_layout = unsafe { self.device.get_image_subresource_layout(texture.image, image_subresource) };
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: image_subresource_layout.offset as usize,
				size: image_subresource_layout.size as usize,
				row_pitch: image_subresource_layout.row_pitch as usize,
				array_pitch: image_subresource_layout.array_pitch as usize,
				depth_pitch: image_subresource_layout.depth_pitch as usize,
			}
		}
	}

	fn bind_vulkan_buffer_memory(&self, info: &MemoryBackedResourceCreationResult<vk::Buffer>, allocation_handle: graphics_hardware_interface::AllocationHandle, offset: usize) -> (u64, *mut u8) {
		let buffer = info.resource;
		let allocation = self.allocations.get(allocation_handle.0 as usize).expect("No allocation with that handle.");
		unsafe { self.device.bind_buffer_memory(buffer, allocation.memory, offset as u64).expect("No buffer memory binding") };
		unsafe {
			(self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)), allocation.pointer.add(offset))
		}
	}

	fn bind_vulkan_texture_memory(&self, info: &MemoryBackedResourceCreationResult<vk::Image>, allocation_handle: graphics_hardware_interface::AllocationHandle, offset: usize) -> (u64, *mut u8) {
		let image = info.resource;
		let allocation = self.allocations.get(allocation_handle.0 as usize).expect("No allocation with that handle.");
		unsafe { self.device.bind_image_memory(image, allocation.memory, offset as u64).expect("No image memory binding") };
		(0, unsafe { allocation.pointer.add(offset) })
	}

	fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
		let fence_create_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::empty() | if signaled { vk::FenceCreateFlags::SIGNALED } else { vk::FenceCreateFlags::empty() });
		unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") }
	}

	fn set_name<T: vk::Handle>(&self, handle: T, name: Option<&str>) {
		if let Some(name) = name {
			let name = std::ffi::CString::new(name).unwrap();
			let name = name.as_c_str();
			#[cfg(debug_assertions)]
			unsafe {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils.set_debug_utils_object_name(
						&vk::DebugUtilsObjectNameInfoEXT::default()
							.object_handle(handle)
							.object_name(name)
					).ok(); // Ignore errors, if the name can't be set, it's not a big deal.
				}
			}
		}
	}

	fn create_vulkan_semaphore(&self, name: Option<&str>, _: bool) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default();
		let handle = unsafe { self.device.create_semaphore(&semaphore_create_info, None).expect("No semaphore") };

		self.set_name(handle, name);

		handle
	}

	fn create_vulkan_image_view(&self, name: Option<&str>, texture: &vk::Image, format: graphics_hardware_interface::Formats, _mip_levels: u32, base_layer: u32, layer_count: u32) -> vk::ImageView {
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(if layer_count == 1 { vk::ImageViewType::TYPE_2D } else { vk::ImageViewType::TYPE_2D_ARRAY })
			.format(to_format(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if format != graphics_hardware_interface::Formats::Depth32 { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: base_layer,
				layer_count: layer_count,
			})
		;

		let vk_image_view = unsafe { self.device.create_image_view(&image_view_create_info, None).expect("No image view") };

		self.set_name(vk_image_view, name);

		vk_image_view
	}

	fn create_vulkan_surface(&self, window_os_handles: &window::OSHandles) -> vk::SurfaceKHR {
		let surface = match window_os_handles {
			#[cfg(target_os = "linux")]
			window::OSHandles::Wayland(os_handles) => {
				let wayland_surface = ash::khr::wayland_surface::Instance::new(&self.entry, &self.instance);

				let wayland_surface_create_info = vk::WaylandSurfaceCreateInfoKHR::default()
					.display(os_handles.display)
					.surface(os_handles.surface);

				unsafe { wayland_surface.create_wayland_surface(&wayland_surface_create_info, None).expect("No surface") }
			}
			#[cfg(target_os = "linux")]
			window::OSHandles::X11(os_handles) => {
				let xcb_surface = ash::khr::xcb_surface::Instance::new(&self.entry, &self.instance);

				let xcb_surface_create_info = vk::XcbSurfaceCreateInfoKHR::default()
					.connection(os_handles.xcb_connection)
					.window(os_handles.xcb_window);
		
				unsafe { xcb_surface.create_xcb_surface(&xcb_surface_create_info, None).expect("No surface") }
			}
			#[cfg(target_os = "windows")]
			window::OSHandles::Win32(os_handles) => {
				let win32_surface = ash::khr::win32_surface::Instance::new(&self.entry, &self.instance);

				let win32_surface_create_info = vk::Win32SurfaceCreateInfoKHR::default()
					.hinstance(os_handles.hinstance.0)
					.hwnd(os_handles.hwnd.0);

				unsafe { win32_surface.create_win32_surface(&win32_surface_create_info, None).expect("No surface") }
			}
		};

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let surface_format = unsafe { self.surface.get_physical_device_surface_formats(self.physical_device, surface).expect("No surface formats") };

		let _: vk::SurfaceFormatKHR = surface_format
			.iter()
			.find(|format| format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.expect("No surface format").to_owned();

		let surface_present_modes = unsafe { self.surface.get_physical_device_surface_present_modes(self.physical_device, surface).expect("No surface present modes") };

		let _: vk::PresentModeKHR = surface_present_modes
			.iter()
			.find(|present_mode| {
				**present_mode == vk::PresentModeKHR::FIFO
			})
			.expect("No surface present mode").to_owned();

		let _surface_resolution = surface_capabilities.current_extent;

		surface
	}

	/// Allocates memory from the device.
	fn create_allocation_internal(&mut self, size: usize, memory_bits: Option<u32>, device_accesses: graphics_hardware_interface::DeviceAccesses) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		let memory_properties = unsafe { self.instance.get_physical_device_memory_properties(self.physical_device) };

		let memory_property_flags = {
			let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuRead) { vk::MemoryPropertyFlags::HOST_VISIBLE } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) { vk::MemoryPropertyFlags::HOST_COHERENT } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::GpuRead) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::GpuWrite) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };

			memory_property_flags
		};

		let memory_type_index = memory_properties
			.memory_types
			.iter()
			.enumerate()
			.find_map(|(index, memory_type)| {
				let memory_type = memory_type.property_flags.contains(memory_property_flags);

				if (memory_bits.unwrap_or(0) & (1 << index)) != 0 && memory_type {
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
		;

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		let mut mapped_memory = None;

		if device_accesses.intersects(graphics_hardware_interface::DeviceAccesses::CpuRead | graphics_hardware_interface::DeviceAccesses::CpuWrite) {
			mapped_memory = Some(unsafe { self.device.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty()).expect("No mapped memory") as *mut u8 });
		}

		let allocation_handle = graphics_hardware_interface::AllocationHandle(self.allocations.len() as u64);

		self.allocations.push(Allocation {
			memory,
			pointer: mapped_memory.unwrap_or(std::ptr::null_mut()),
		});

		(allocation_handle, mapped_memory)
	}

	fn get_descriptor_set(&self, descriptor_set_handle: DescriptorSetHandle) -> &DescriptorSet {
        self.descriptor_sets.get(descriptor_set_handle.0 as usize).expect("No descriptor set with that handle.")
    }

	fn write_binding(&mut self, descriptor_set_write: &graphics_hardware_interface::DescriptorWrite) {
		let binding_handle = DescriptorSetBindingHandle(descriptor_set_write.binding_handle.0);

		let binding = &self.bindings[binding_handle.0 as usize];
		let descriptor_type = binding.descriptor_type;
		let binding_index = binding.index;

		assert!(descriptor_set_write.array_element < binding.count, "Binding index out of range.");
		
		let descriptor_set_handles = {
			let descriptor_set_handle = DescriptorSetHandle(binding.descriptor_set_handle.0);
			let mut descriptor_set_handles = Vec::with_capacity(3);
	
			let mut descriptor_set_handle_option = Some(descriptor_set_handle);
	
			while let Some(descriptor_set_handle) = descriptor_set_handle_option {
				let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
				descriptor_set_handles.push(descriptor_set_handle);
				descriptor_set_handle_option = descriptor_set.next;
			}
	
			descriptor_set_handles
		};

		match descriptor_set_write.descriptor {
			graphics_hardware_interface::Descriptor::Buffer { handle, size } => {
				let buffer_handles = {
					let mut buffer_handles = Vec::with_capacity(3);
					let mut buffer_handle_option = Some(BufferHandle(handle.0));

					while let Some(buffer_handle) = buffer_handle_option {
						buffer_handles.push(buffer_handle);
						let buffer = &self.buffers[buffer_handle.0 as usize];
						buffer_handle_option = buffer.next;
					}

					buffer_handles
				};

				for (i, &descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let offset = descriptor_set_write.frame_offset.unwrap_or(0);

					let buffer_handle = buffer_handles[((i as i32 - offset) % buffer_handles.len() as i32) as usize];
					let buffer = &self.buffers[buffer_handle.0 as usize];

					if !buffer.buffer.is_null() {
						let buffers = [vk::DescriptorBufferInfo::default().buffer(buffer.buffer).offset(0u64).range(match size { graphics_hardware_interface::Ranges::Size(size) => { size as u64 } graphics_hardware_interface::Ranges::Whole => { vk::WHOLE_SIZE } })];
	
						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.buffer_info(&buffers)
						;
	
						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
					}

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Buffer{ size, buffer: buffer_handle });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Buffer(buffer_handle));
				}
			},
			graphics_hardware_interface::Descriptor::Image{ handle, layout } => {
				let image_handles = {
					let mut image_handles = Vec::with_capacity(3);
					let mut image_handle_option = Some(ImageHandle(handle.0));

					while let Some(image_handle) = image_handle_option {
						image_handles.push(image_handle);
						let image = &self.images[image_handle.0 as usize];
						image_handle_option = image.next;
					}

					image_handles
				};

				for (i, &descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let offset = descriptor_set_write.frame_offset.unwrap_or(0);

					let image_handle = image_handles[((i as i32 - offset) % image_handles.len() as i32) as usize];

					let image = &self.images[image_handle.0 as usize];

					if !image.image.is_null() && !image.image_view.is_null() {
						let images = [
							vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, layout, None))
							.image_view(image.image_view)
						];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&images)
						;

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
					}

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Image{ layout, image: image_handle });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Image(image_handle));
				}
			},
			graphics_hardware_interface::Descriptor::CombinedImageSampler{ image_handle, sampler_handle, layout, layer } => {
				let image_handles = {
					let mut image_handles = Vec::with_capacity(3);
					let mut image_handle_option = Some(ImageHandle(image_handle.0));

					while let Some(image_handle) = image_handle_option {
						image_handles.push(image_handle);
						let image = &self.images[image_handle.0 as usize];
						image_handle_option = image.next;
					}

					image_handles
				};

				for (i, &descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let offset = descriptor_set_write.frame_offset.unwrap_or(0);

					let image_handle = image_handles[((i as i32 - offset) % image_handles.len() as i32) as usize];

					let image = &self.images[image_handle.0 as usize];

					if !image.image.is_null() {
						let image_view = if let Some(layer) = layer { // If the descriptor asks for a subresource, we need to create a new image view
							image.image_views[layer as usize]
						} else {
							image.image_view
						};

						let images = [
							vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, layout, None))
							.image_view(image_view)
							.sampler(vk::Sampler::from_raw(sampler_handle.0))
						];

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&images)
						;

						unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
					}

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::CombinedImageSampler{ image: image_handle, sampler: vk::Sampler::from_raw(sampler_handle.0), layout });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Image(image_handle));
				}
			},
			graphics_hardware_interface::Descriptor::Sampler(handle) => {
				for (_, descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let sampler_handle = handle;
					let images = [vk::DescriptorImageInfo::default().sampler(vk::Sampler::from_raw(sampler_handle.0))];

					let write_info = vk::WriteDescriptorSet::default()
						.dst_set(descriptor_set.descriptor_set)
						.dst_binding(binding_index)
						.dst_array_element(descriptor_set_write.array_element)
						.descriptor_type(descriptor_type)
						.image_info(&images)
					;

					unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
				}
			},
			graphics_hardware_interface::Descriptor::StaticSamplers => {}
			graphics_hardware_interface::Descriptor::Swapchain(_) => {
				unimplemented!()
			}
			graphics_hardware_interface::Descriptor::AccelerationStructure { handle } => {
				for (_, descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let acceleration_structure_handle = handle;
					let acceleration_structure = &self.acceleration_structures[acceleration_structure_handle.0 as usize];

					let acceleration_structures = [acceleration_structure.acceleration_structure];

					let mut acc_str_descriptor_info = vk::WriteDescriptorSetAccelerationStructureKHR::default()
						.acceleration_structures(&acceleration_structures);

					let write_info = vk::WriteDescriptorSet{ descriptor_count: 1, ..vk::WriteDescriptorSet::default() }
						.push_next(&mut acc_str_descriptor_info)
						.dst_set(descriptor_set.descriptor_set)
						.dst_binding(binding_index)
						.dst_array_element(descriptor_set_write.array_element)
						.descriptor_type(descriptor_type)
					;

					unsafe { self.device.update_descriptor_sets(&[write_info], &[]) };
				}
			}
		}
		
		let binding = &self.bindings[descriptor_set_write.binding_handle.0 as usize];

		let descriptor_set_handle = DescriptorSetHandle(binding.descriptor_set_handle.0);

		let descriptor_sets = {
			let mut descriptor_sets = Vec::with_capacity(3);

			let mut descriptor_set_handle_option = Some(descriptor_set_handle);

			while let Some(descriptor_set_handle) = descriptor_set_handle_option {
				let descriptor_set = self.get_descriptor_set(descriptor_set_handle);

				descriptor_sets.push(descriptor_set_handle);

				descriptor_set_handle_option = descriptor_set.next;
			}

			descriptor_sets
		};

		for (i, _) in descriptor_sets.iter().enumerate() {
			match descriptor_set_write.descriptor {
				graphics_hardware_interface::Descriptor::Buffer { handle, .. } => {
					let buffer_handles = {
						let mut buffer_handles = Vec::with_capacity(3);
						let mut buffer_handle_option = Some(BufferHandle(handle.0));
	
						while let Some(buffer_handle) = buffer_handle_option {
							buffer_handles.push(buffer_handle);
							let buffer = &self.buffers[buffer_handle.0 as usize];
							buffer_handle_option = buffer.next;
						}
	
						buffer_handles
					};
					let offset = descriptor_set_write.frame_offset.unwrap_or(0);
					let buffer_handle = buffer_handles[((i as i32 - offset) % buffer_handles.len() as i32) as usize];
					self.resource_to_descriptor.entry(Handle::Buffer(buffer_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));
				}
				graphics_hardware_interface::Descriptor::Image { handle, .. } => {						
					let images_handles = {
						let mut images = Vec::with_capacity(3);
						let mut image_handle_option = Some(ImageHandle(handle.0));
						
						while let Some(image_handle) = image_handle_option {
							let image = &self.images[image_handle.0 as usize];
							images.push(image_handle);
							image_handle_option = image.next;
						}
						
						images
					};

					let offset = descriptor_set_write.frame_offset.unwrap_or(0);

					let image = &images_handles[((i as i32 - offset) % images_handles.len() as i32) as usize];
					
					self.resource_to_descriptor.entry(Handle::Image(*image)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));
				}
				graphics_hardware_interface::Descriptor::CombinedImageSampler { image_handle, .. } => {
					let image_handles = {
						let mut images = Vec::with_capacity(3);
						let mut image_handle_option = Some(ImageHandle(image_handle.0));
						
						while let Some(image_handle) = image_handle_option {
							let image = &self.images[image_handle.0 as usize];
							images.push(image_handle);
							image_handle_option = image.next;
						}
						
						images
					};

					let offset = descriptor_set_write.frame_offset.unwrap_or(0);

					let image_handle = image_handles[((i as i32 - offset) % image_handles.len() as i32) as usize];

					self.resource_to_descriptor.entry(Handle::Image(image_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));
				}
				_ => {}
			}
		}
	}
}

#[derive(PartialEq, Eq, Clone, Copy)]
struct TransitionState {
	stage: vk::PipelineStageFlags2,
	access: vk::AccessFlags2,
	layout: vk::ImageLayout,
}

pub struct VulkanCommandBufferRecording<'a> {
	ghi: &'a mut VulkanGHI,
	command_buffer: graphics_hardware_interface::CommandBufferHandle,
	in_render_pass: bool,
	modulo_frame_index: u32,
	states: HashMap<Handle, TransitionState>,
	pipeline_bind_point: vk::PipelineBindPoint,

	stages: vk::PipelineStageFlags2,

	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<(u32, DescriptorSetHandle)>,
}

impl VulkanCommandBufferRecording<'_> {
	pub fn new(ghi: &'_ mut VulkanGHI, command_buffer: graphics_hardware_interface::CommandBufferHandle, frame_index: Option<u32>) -> VulkanCommandBufferRecording<'_> {
		VulkanCommandBufferRecording {
			pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
			command_buffer,
			modulo_frame_index: frame_index.map(|frame_index| frame_index % ghi.frames as u32).unwrap_or(0),
			in_render_pass: false,
			states: ghi.states.clone(),
			
			stages: vk::PipelineStageFlags2::empty(),
			
			bound_pipeline: None,
			bound_descriptor_set_handles: Vec::new(),

			ghi,
		}
	}

	fn get_buffer(&self, buffer_handle: BufferHandle) -> &Buffer {
		&self.ghi.buffers[buffer_handle.0 as usize]
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> ImageHandle {
		let mut i = 0;
		let mut internal_image_handle = ImageHandle(handle.0);
		loop {
			let image = &self.ghi.images[internal_image_handle.0 as usize];
			if i == self.modulo_frame_index || image.next.is_none() {
				return internal_image_handle;
			}
			internal_image_handle = image.next.unwrap();
			i += 1;
		}
    }

	fn get_image(&self, image_handle: ImageHandle) -> &Image {
		&self.ghi.images[image_handle.0 as usize]
	}

	fn get_synchronizer(&self, syncronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> &Synchronizer {
		&self.ghi.synchronizers[self.ghi.get_syncronizer_handles(syncronizer_handle)[self.modulo_frame_index as usize].0 as usize]
	}

	fn get_internal_top_level_acceleration_structure_handle(&self, acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle) -> TopLevelAccelerationStructureHandle {
		TopLevelAccelerationStructureHandle(acceleration_structure_handle.0)
    }

	fn get_top_level_acceleration_structure(&self, acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle) -> (graphics_hardware_interface::TopLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.ghi.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_internal_bottom_level_acceleration_structure_handle(&self, acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle) -> BottomLevelAccelerationStructureHandle {
		BottomLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	fn get_bottom_level_acceleration_structure(&self, acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle) -> (graphics_hardware_interface::BottomLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.ghi.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_command_buffer(&self) -> &CommandBufferInternal {
		&self.ghi.command_buffers[self.command_buffer.0 as usize].frames[self.modulo_frame_index as usize]
	}

	fn get_internal_descriptor_set_handle(&self, descriptor_set_handle: graphics_hardware_interface::DescriptorSetHandle) -> DescriptorSetHandle {
		let mut i = 0;
		let mut handle = DescriptorSetHandle(descriptor_set_handle.0);
		loop {
			let descriptor_set = &self.ghi.descriptor_sets[handle.0 as usize];
			if i == self.modulo_frame_index {
				return handle;
			}
			handle = descriptor_set.next.unwrap();
			i += 1;
		}
	}

	fn get_descriptor_set(&self, descriptor_set_handle: &DescriptorSetHandle) -> &DescriptorSet {
		&self.ghi.descriptor_sets[descriptor_set_handle.0 as usize]
	}

	fn consume_resources_current(&mut self, additional_transitions: &[graphics_hardware_interface::Consumption]) {
		let mut consumptions = Vec::with_capacity(32);

		let bound_pipeline_handle = self.bound_pipeline.expect("No bound pipeline");

		let pipeline = &self.ghi.pipelines[bound_pipeline_handle.0 as usize];

		for &((set_index, binding_index), (stages, access)) in &pipeline.resource_access {
			let set_handle = if let Some(&h) = self.bound_descriptor_set_handles.get(set_index as usize) { h.1 } else {
				println!("No bound descriptor set found for index {}", set_index);
				continue;
			};

			let resources = match self.ghi.descriptors.get(&set_handle).map(|d| d.get(&binding_index)) {
				Some(Some(b)) => b.values(),
				_ => {
					println!("Pipeline '{}' requires binding with '{}' index for set with '{}' index, but no such descriptor(s) exist.", bound_pipeline_handle.0, binding_index, set_index);
					continue;
				}
			};

			for idk in resources {
				let (layout, handle) = match idk {
					Descriptor::Buffer { buffer, .. } => {
						(graphics_hardware_interface::Layouts::General, Handle::Buffer(*buffer))
					}
					Descriptor::Image { layout, image } => {
						(*layout, Handle::Image(*image))
					}
					Descriptor::CombinedImageSampler { image, layout, .. } => {
						(*layout, Handle::Image(*image))
					}
				};

				consumptions.push(Consumption { handle, stages, access, layout, });
			}
		}

		consumptions.extend(additional_transitions.iter().map(|c|
			Consumption {
				handle: self.get_internal_handle(c.handle.clone()),
				stages: c.stages,
				access: c.access,
				layout: c.layout,
			}
		));

		unsafe { self.consume_resources(&consumptions) };
	}

	unsafe fn consume_resources(&mut self, consumptions: &[Consumption]) {
		if consumptions.is_empty() { return; } // Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).

		let mut image_memory_barriers = Vec::new();
		let mut buffer_memory_barriers = Vec::new();
		let mut memory_barriers = Vec::new();

		for consumption in consumptions {
			let format = match consumption.handle {
				Handle::Image(texture_handle) => {
					let image = self.get_image(texture_handle);
					Some(image.format_)
				}
				_ => { None }
			};

			let new_stage_mask = to_pipeline_stage_flags(consumption.stages, Some(consumption.layout), format);
			let new_access_mask = to_access_flags(consumption.access, consumption.stages, consumption.layout, format);

			let transition_state = TransitionState {
				stage: new_stage_mask,
				access: new_access_mask,
				layout: match consumption.handle {
					Handle::Image(image_handle) => {
						let image = self.get_image(image_handle);
						texture_format_and_resource_use_to_image_layout(image.format_, consumption.layout, Some(consumption.access))
					}
					_ => vk::ImageLayout::UNDEFINED
				}
			};

			if let Some(state) = self.states.get(&consumption.handle) {
				if &transition_state == state { continue; } // If current state is equal to new intended state, skip.
			}

			match consumption.handle {
				Handle::Image(handle) => {
					let image = self.get_image(handle);

					if image.image.is_null() { continue; }

					let new_layout = texture_format_and_resource_use_to_image_layout(image.format_, consumption.layout, Some(consumption.access));

					let image_memory_barrier = if let Some(barrier_source) = self.states.get(&consumption.handle) {
							vk::ImageMemoryBarrier2::default().old_layout(barrier_source.layout).src_stage_mask(barrier_source.stage).src_access_mask(barrier_source.access)
						} else {
							vk::ImageMemoryBarrier2::default().old_layout(vk::ImageLayout::UNDEFINED).src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty())
						}
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(new_layout)
						.dst_stage_mask(new_stage_mask)
						.dst_access_mask(new_access_mask)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(image.image)
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: if image.format != vk::Format::D32_SFLOAT { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
					;

					image_memory_barriers.push(image_memory_barrier);
				}
				Handle::Buffer(handle) => {
					let buffer = self.get_buffer(handle);

					if buffer.buffer.is_null() { continue; }

					// let buffer_memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
					// 	vk::BufferMemoryBarrier2::default().src_stage_mask(source.stage).src_access_mask(source.access).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					// } else {
					// 	vk::BufferMemoryBarrier2::default().src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty()).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					// }
					// .dst_stage_mask(new_stage_mask)
					// .dst_access_mask(new_access_mask)
					// .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					// .buffer(buffer.buffer)
					// .offset(0)
					// .size(vk::WHOLE_SIZE);

					// buffer_memory_barriers.push(buffer_memory_barrier);

					let memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::MemoryBarrier2::default().src_stage_mask(source.stage).src_access_mask(source.access)
					} else {
						vk::MemoryBarrier2::default().src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask);

					memory_barriers.push(memory_barrier);
				},
				Handle::TopLevelAccelerationStructure(_) | Handle::BottomLevelAccelerationStructure(_)=> {
					let memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::MemoryBarrier2::default().src_stage_mask(source.stage).src_access_mask(source.access)
					} else {
						vk::MemoryBarrier2::default().src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask);

					memory_barriers.push(memory_barrier);
				}
			};

			// Update current resource state, AFTER generating the barrier.
			self.states.insert(consumption.handle, transition_state);
		}

		if image_memory_barriers.is_empty() && buffer_memory_barriers.is_empty() && memory_barriers.is_empty() { return; } // consumptions may have had some elements but they may have been skipped.

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.buffer_memory_barriers(&buffer_memory_barriers)
			.memory_barriers(&memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION);

		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		let mut i = 0;
		let mut internal_buffer_handle = BufferHandle(handle.0);
		loop {
			let buffer = &self.ghi.buffers[internal_buffer_handle.0 as usize];
			if i == self.modulo_frame_index || buffer.next.is_none() {
				return internal_buffer_handle;
			}
			internal_buffer_handle = buffer.next.unwrap();
			i += 1;
		}
	}

	fn get_internal_handle(&self, handle: graphics_hardware_interface::Handle) -> Handle {
		match handle {
			graphics_hardware_interface::Handle::Image(handle) => Handle::Image(self.get_internal_image_handle(handle)),
			graphics_hardware_interface::Handle::Buffer(handle) => Handle::Buffer(self.get_internal_buffer_handle(handle)),
			graphics_hardware_interface::Handle::TopLevelAccelerationStructure(handle) => Handle::TopLevelAccelerationStructure(self.get_internal_top_level_acceleration_structure_handle(handle)),
			graphics_hardware_interface::Handle::BottomLevelAccelerationStructure(handle) => Handle::BottomLevelAccelerationStructure(self.get_internal_bottom_level_acceleration_structure_handle(handle)),
			_ => unimplemented!(),
		}
	}
}

impl graphics_hardware_interface::CommandBufferRecordable for VulkanCommandBufferRecording<'_> {
	fn begin(&mut self) {
		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.reset_command_pool(command_buffer.command_pool, vk::CommandPoolResetFlags::empty()).expect("No command pool reset") };

		let command_buffer_begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

		unsafe { self.ghi.device.begin_command_buffer(command_buffer.command_buffer, &command_buffer_begin_info).expect("No command buffer begin") };

		let mut pending_buffers = self.ghi.pending_buffers.clone();

		self.ghi.pending_buffers.clear();

		pending_buffers.sort();
		pending_buffers.dedup();

		self.start_region("Buffer Sync");

		let copy_buffers = pending_buffers.into_iter().map(|b| self.get_internal_buffer_handle(b)).collect::<Vec<_>>();

		unsafe {
			self.consume_resources(&copy_buffers.iter().map(|&b| {
				Consumption {
					handle: Handle::Buffer(b),
					stages: graphics_hardware_interface::Stages::TRANSFER,
					access: graphics_hardware_interface::AccessPolicies::WRITE,
					layout: graphics_hardware_interface::Layouts::General,
				}
			}).collect::<Vec<_>>());
		}

		let copy_buffers = copy_buffers.iter().map(|&b| self.get_buffer(b)).filter(|b| b.staging.is_some() && b.size != 0);

		// TODO: copy only buffers belonging to this frame
		for buffer in copy_buffers { // Copy all staging buffers to their respective buffers
			let vk_buffer = buffer.buffer;
			let staging_buffer_handle = buffer.staging.unwrap();
			let staging_buffer = self.ghi.buffers[staging_buffer_handle.0 as usize].buffer;

			let command_buffer = self.get_command_buffer();

			let regions = [vk::BufferCopy2KHR::default()
				.src_offset(0)
				.dst_offset(0)
				.size(buffer.size as u64)
			];

			let copy_buffer_info = vk::CopyBufferInfo2KHR::default()
				.src_buffer(staging_buffer)
				.dst_buffer(vk_buffer)
				.regions(&regions)
			;

			unsafe {
				self.ghi.device.cmd_copy_buffer2(command_buffer.command_buffer, &copy_buffer_info);
			}
		}

		self.end_region();

		self.stages |= vk::PipelineStageFlags2::TRANSFER;
	}

	fn start_render_pass(&mut self, extent: Extent, attachments: &[graphics_hardware_interface::AttachmentInformation]) -> &mut impl graphics_hardware_interface::RasterizationRenderPassMode {
		unsafe {
			self.consume_resources(&attachments.iter().map(|attachment|
				Consumption{
					handle: Handle::Image(self.get_internal_image_handle(attachment.image)),
					stages: graphics_hardware_interface::Stages::FRAGMENT,
					access: graphics_hardware_interface::AccessPolicies::WRITE,
					layout: attachment.layout,
				}
			).collect::<Vec<_>>());
		}

		let render_area = vk::Rect2D::default().offset(vk::Offset2D::default().x(0).y(0)).extent(vk::Extent2D::default().width(extent.width()).height(extent.height()));

		let color_attchments = attachments.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|attachment| {
			let image = self.get_image(self.get_internal_image_handle(attachment.image));
			vk::RenderingAttachmentInfo::default()
				.image_view(if let Some(index) = attachment.layer { image.image_views[index as usize] } else { image.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
		}).collect::<Vec<_>>();

		let depth_attachment = attachments.iter().find(|attachment| attachment.format == graphics_hardware_interface::Formats::Depth32).map(|attachment| {
			let image = self.get_image(self.get_internal_image_handle(attachment.image));
			vk::RenderingAttachmentInfo::default()
				.image_view(if let Some(index) = attachment.layer { image.image_views[index as usize] } else { image.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
		}).or(Some(vk::RenderingAttachmentInfo::default())).unwrap();

		let rendering_info = vk::RenderingInfoKHR::default().color_attachments(color_attchments.as_slice()).depth_attachment(&depth_attachment).render_area(render_area).layer_count(1);

		let viewports = [
			vk::Viewport {
				x: 0.0,
				y: 0.0,
				width: extent.width() as f32,
				height: (extent.height() as f32),
				min_depth: 0.0,
				max_depth: 1.0,
			}
		];

		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.cmd_set_scissor(command_buffer.command_buffer, 0, &[render_area]); }
		unsafe { self.ghi.device.cmd_set_viewport(command_buffer.command_buffer, 0, &viewports); }
		unsafe { self.ghi.device.cmd_begin_rendering(command_buffer.command_buffer, &rendering_info); }

		self.in_render_pass = true;

		self
	}

	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &graphics_hardware_interface::TopLevelAccelerationStructureBuild) {
		use graphics_hardware_interface::GraphicsHardwareInterface;

		let (acceleration_structure_handle, acceleration_structure) = self.get_top_level_acceleration_structure(acceleration_structure_build.acceleration_structure);

		let (as_geometries, offsets) = match acceleration_structure_build.description {
			graphics_hardware_interface::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, instance_count } => {
				(vec![
					vk::AccelerationStructureGeometryKHR::default()
						.geometry_type(vk::GeometryTypeKHR::INSTANCES)
						.geometry(vk::AccelerationStructureGeometryDataKHR{ instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
							.array_of_pointers(false)
							.data(vk::DeviceOrHostAddressConstKHR { device_address: self.ghi.get_buffer_address(instances_buffer), })
						})
						.flags(vk::GeometryFlagsKHR::OPAQUE)
				], vec![
					vk::AccelerationStructureBuildRangeInfoKHR::default()
						.primitive_count(instance_count)
						.primitive_offset(0)
						.first_vertex(0)
						.transform_offset(0)
				])
			}
		};

		let scratch_buffer_address = unsafe {
			let buffer = self.get_buffer(self.get_internal_buffer_handle(acceleration_structure_build.scratch_buffer.buffer));
			self.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + acceleration_structure_build.scratch_buffer.offset as u64
		};

		let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.dst_acceleration_structure(acceleration_structure.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch_buffer_address, })
		;

		self.states.insert(Handle::TopLevelAccelerationStructure(self.get_internal_top_level_acceleration_structure_handle(acceleration_structure_handle)), TransitionState {
			stage: vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
			access: vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
			layout: vk::ImageLayout::UNDEFINED,
		});

		let infos = vec![build_geometry_info];
		let build_range_infos = vec![offsets];
		let geometries = vec![as_geometries];

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let infos = infos.iter().zip(geometries.iter()).map(|(info, geos)| info.geometries(geos)).collect::<Vec<_>>();

		let build_range_infos = build_range_infos.iter().map(|build_range_info| build_range_info.as_slice()).collect::<Vec<_>>();

		self.stages |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR;

		unsafe {
			self.ghi.acceleration_structure.cmd_build_acceleration_structures(vk_command_buffer, &infos, &build_range_infos)
		}
	}

	fn build_bottom_level_acceleration_structures(&mut self, acceleration_structure_builds: &[graphics_hardware_interface::BottomLevelAccelerationStructureBuild]) {
		if acceleration_structure_builds.is_empty() { return; }

		fn visit(this: &mut VulkanCommandBufferRecording, acceleration_structure_builds: &[graphics_hardware_interface::BottomLevelAccelerationStructureBuild], mut infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR>, mut geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR>>, mut build_range_infos: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>>,) {
			if let Some(build) = acceleration_structure_builds.first() {
				let (acceleration_structure_handle, acceleration_structure) = this.get_bottom_level_acceleration_structure(build.acceleration_structure);

				let (as_geometries, offsets) = match &build.description {
					graphics_hardware_interface::BottomLevelAccelerationStructureBuildDescriptions::AABB { .. } => {
						(vec![], vec![])
					}
					graphics_hardware_interface::BottomLevelAccelerationStructureBuildDescriptions::Mesh { vertex_buffer, index_buffer, vertex_position_encoding, index_format, triangle_count, vertex_count } => {
						let vertex_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(vertex_buffer.buffer_offset.buffer));
							this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + vertex_buffer.buffer_offset.offset as u64
						};

						let index_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(index_buffer.buffer_offset.buffer));
							this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + index_buffer.buffer_offset.offset as u64
						};

						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR { device_address: vertex_data_address, })
							.index_data(vk::DeviceOrHostAddressConstKHR { device_address: index_data_address, })
							.max_vertex(vertex_count - 1)
							.vertex_format(match vertex_position_encoding {
								graphics_hardware_interface::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position encoding"),
							})
							.index_type(match index_format {
								graphics_hardware_interface::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								graphics_hardware_interface::DataTypes::U16 => vk::IndexType::UINT16,
								graphics_hardware_interface::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
							.vertex_stride(vertex_buffer.stride as vk::DeviceSize);

						let build_range_info = vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
							.primitive_count(*triangle_count)
							.primitive_offset(0)
							.first_vertex(0)
							.transform_offset(0)
						];

						(vec![vk::AccelerationStructureGeometryKHR::default()
							.flags(vk::GeometryFlagsKHR::OPAQUE)
							.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
							.geometry(vk::AccelerationStructureGeometryDataKHR{ triangles })],
						build_range_info)
					}
				};

				let scratch_buffer_address = unsafe {
					let buffer = this.get_buffer(this.get_internal_buffer_handle(build.scratch_buffer.buffer));
					this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + build.scratch_buffer.offset as u64
				};

				let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.dst_acceleration_structure(acceleration_structure.acceleration_structure)
					.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch_buffer_address, })
				;
				
				this.states.insert(Handle::BottomLevelAccelerationStructure(this.get_internal_bottom_level_acceleration_structure_handle(acceleration_structure_handle)), TransitionState {
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
					this.ghi.acceleration_structure.cmd_build_acceleration_structures(command_buffer.command_buffer, &infos, &build_range_infos)
				}
			}
		}

		visit(self, acceleration_structure_builds, Vec::new(), Vec::new(), Vec::new(),);

		self.stages |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR;
	}

	fn bind_shader(&self, _: graphics_hardware_interface::ShaderHandle) {
		panic!("Not implemented");
	}

	fn bind_compute_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundComputePipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::COMPUTE;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundRayTracingPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::RAY_TRACING_KHR, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::RAY_TRACING_KHR;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn blit_image(&mut self, source_image: crate::ImageHandle, source_layout: crate::Layouts, destination_image: crate::ImageHandle, destination_layout: crate::Layouts) {
		unsafe {
			self.consume_resources(&[
				Consumption {
					handle: Handle::Image(self.get_internal_image_handle(source_image)),
					stages: graphics_hardware_interface::Stages::TRANSFER,
					access: graphics_hardware_interface::AccessPolicies::READ,
					layout: source_layout,
				},
				Consumption {
					handle: Handle::Image(self.get_internal_image_handle(destination_image)),
					stages: graphics_hardware_interface::Stages::TRANSFER,
					access: graphics_hardware_interface::AccessPolicies::WRITE,
					layout: destination_layout,
				}
			]);
		}

		let command_buffer = self.get_command_buffer();
		let source_image = self.get_image(self.get_internal_image_handle(source_image));
		let destination_image = self.get_image(self.get_internal_image_handle(destination_image));
		unsafe {
			let blit = vk::ImageBlit2::default()
			.src_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.src_offsets([
				vk::Offset3D { x: 0, y: 0, z: 0 },
				vk::Offset3D { x: source_image.extent.width as i32, y: source_image.extent.height as i32, z: 1 },
			])
			.dst_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.dst_offsets([
				vk::Offset3D { x: 0, y: 0, z: 0 },
				vk::Offset3D { x: destination_image.extent.width as i32, y: destination_image.extent.height as i32, z: 1 },
			]);

			let blits = [blit];

			let blit_info = vk::BlitImageInfo2::default()
				.src_image(source_image.image).src_image_layout(texture_format_and_resource_use_to_image_layout(source_image.format_, source_layout, Some(crate::AccessPolicies::READ)))
				.dst_image(destination_image.image).dst_image_layout(texture_format_and_resource_use_to_image_layout(destination_image.format_, destination_layout, Some(crate::AccessPolicies::WRITE)))
				.regions(&blits)
				.filter(vk::Filter::LINEAR);
			self.ghi.device.cmd_blit_image2(command_buffer.command_buffer, &blit_info);
		}
	}

	fn write_to_push_constant(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, offset: u32, data: &[u8]) {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout;
		unsafe { self.ghi.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE, offset, data); }
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, pipeline_layout_handle: &crate::PipelineLayoutHandle, offset: u32, data: T) where [(); std::mem::size_of::<T>()]: Sized {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout;
		unsafe { self.ghi.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE, offset, std::slice::from_raw_parts(&data as *const T as *const u8, std::mem::size_of::<T>())); }	
	}

	fn clear_images(&mut self, textures: &[(graphics_hardware_interface::ImageHandle, graphics_hardware_interface::ClearValue)]) {
		unsafe { self.consume_resources(textures.iter().map(|(image_handle, _)| Consumption {
			handle: Handle::Image(self.get_internal_image_handle(*image_handle)),
			stages: graphics_hardware_interface::Stages::TRANSFER,
			access: graphics_hardware_interface::AccessPolicies::WRITE,
			layout: graphics_hardware_interface::Layouts::Transfer,
		}).collect::<Vec<_>>().as_slice()) };

		for (image_handle, clear_value) in textures {
			let image = self.get_image(self.get_internal_image_handle(*image_handle));

			if image.image.is_null() { continue; } // Skip unset textures
			
			if image.format_ != graphics_hardware_interface::Formats::Depth32 {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearColorValue{ float32: [0.0, 0.0, 0.0, 0.0] },
					graphics_hardware_interface::ClearValue::Color(color) => vk::ClearColorValue{ float32: [color.r, color.g, color.b, color.a] },
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearColorValue{ float32: [*depth, 0.0, 0.0, 0.0] },
					graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearColorValue{ uint32: [*r, *g, *b, *a] },
				};

				unsafe {
					self.ghi.device.cmd_clear_color_image(self.get_command_buffer().command_buffer, image.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear_value, &[vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::COLOR,
						base_mip_level: 0,
						level_count: vk::REMAINING_MIP_LEVELS,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					}]);
				}
			} else {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearDepthStencilValue{ depth: 0.0, stencil: 0 },
					graphics_hardware_interface::ClearValue::Color(_) => panic!("Color clear value for depth texture"),
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearDepthStencilValue{ depth: *depth, stencil: 0 },
					graphics_hardware_interface::ClearValue::Integer(_, _, _, _) => panic!("Integer clear value for depth texture"),
				};

				unsafe {
					self.ghi.device.cmd_clear_depth_stencil_image(self.get_command_buffer().command_buffer, image.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear_value, &[vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::DEPTH,
						base_mip_level: 0,
						level_count: vk::REMAINING_MIP_LEVELS,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					}]);
				}
			}
		}
	}

	unsafe fn consume_resources(&mut self, consumptions: &[graphics_hardware_interface::Consumption]) {
		let consumptions = consumptions.iter().map(|c| {
			Consumption {
				access: c.access,
				handle: self.get_internal_handle(c.handle.clone()),
				stages: c.stages,
				layout: c.layout,
			}
		}).collect::<Vec<_>>();

		self.consume_resources(consumptions.as_slice());
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		unsafe { self.consume_resources(&buffer_handles.iter().map(|buffer_handle|
			Consumption{
				handle: Handle::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}
		).collect::<Vec<_>>()) };

		for buffer_handle in buffer_handles {
			let internal_buffer_handle = self.get_internal_buffer_handle(*buffer_handle);
			let buffer = self.get_buffer(internal_buffer_handle);

			if buffer.buffer.is_null() { continue; }

			unsafe {
				self.ghi.device.cmd_fill_buffer(self.get_command_buffer().command_buffer, buffer.buffer, 0, vk::WHOLE_SIZE, 0);
			}

			self.states.insert(Handle::Buffer(internal_buffer_handle), TransitionState {
				stage: vk::PipelineStageFlags2::TRANSFER,
				access: vk::AccessFlags2::TRANSFER_WRITE,
				layout: vk::ImageLayout::UNDEFINED,
			});
		}
	}

	fn transfer_textures(&mut self, image_handles: &[graphics_hardware_interface::ImageHandle]) {
		unsafe { self.consume_resources(&image_handles.iter().map(|image_handle|
			Consumption{
				handle: Handle::Image(self.get_internal_image_handle(*image_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}
		).collect::<Vec<_>>()) };

		let command_buffer = self.get_command_buffer();

		for image_handle in image_handles {
			let image = self.get_image(self.get_internal_image_handle(*image_handle));

			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(0)
				.buffer_row_length(0)
				.buffer_image_height(0)
				.image_subresource(vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1)
				)
				.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
				.image_extent(vk::Extent3D::default().width(image.extent.width).height(image.extent.height).depth(image.extent.depth))];

			let buffer = self.get_buffer(image.staging_buffer.expect("No staging buffer"));

			// Copy to images from staging buffer
			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(buffer.buffer)
				.dst_image(image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.ghi.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
			}
		}

		self.stages |= vk::PipelineStageFlags2::TRANSFER;
	}

	fn write_image_data(&mut self, image_handle: graphics_hardware_interface::ImageHandle, data: &[graphics_hardware_interface::RGBAu8]) {
		let internal_image_handle = self.get_internal_image_handle(image_handle);

		unsafe { self.consume_resources(
			&[Consumption{
				handle: Handle::Image(self.get_internal_image_handle(image_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}]
		) };

		let texture = self.get_image(internal_image_handle);

		let staging_buffer_handle = texture.staging_buffer.expect("No staging buffer");

		let buffer = &self.ghi.buffers[staging_buffer_handle.0 as usize];

		let pointer = buffer.pointer;

		let subresource_layout = self.ghi.get_image_subresource_layout(&image_handle, 0);

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
			)
			.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
			.image_extent(vk::Extent3D::default().width(texture.extent.width).height(texture.extent.height).depth(texture.extent.depth))];

		// Copy to images from staging buffer
		let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
			.src_buffer(buffer.buffer)
			.dst_image(texture.image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&regions);

		let command_buffer = self.get_command_buffer();

		unsafe {
			self.ghi.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
		}

		unsafe { self.consume_resources(
			&[Consumption{
				handle: Handle::Image(internal_image_handle),
				stages: graphics_hardware_interface::Stages::FRAGMENT,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Read,
			}]
		) };
	}

	fn copy_to_swapchain(&mut self, source_image_handle: graphics_hardware_interface::ImageHandle, present_image_index: graphics_hardware_interface::PresentKey, swapchain_handle: graphics_hardware_interface::SwapchainHandle) {
		let source_image_internal_handle = self.get_internal_image_handle(source_image_handle);

		unsafe { self.consume_resources(&[
			Consumption {
				handle: Handle::Image(source_image_internal_handle),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Transfer,
			},
		]) };

		let source_texture = self.get_image(source_image_internal_handle);
		let swapchain = &self.ghi.swapchains[swapchain_handle.0 as usize];

		let swapchain_images = unsafe {
			self.ghi.swapchain.get_swapchain_images(swapchain.swapchain).expect("No swapchain images found.")
		};

		let swapchain_image = swapchain_images[present_image_index.0 as usize];

		// Transition source texture to transfer read layout and swapchain image to transfer write layout

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::UNDEFINED)
				.src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE) // This is needed to correctly synchronize presentation when submitting the command buffer.
				.src_access_mask(vk::AccessFlags2KHR::empty())
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.dst_stage_mask(vk::PipelineStageFlags2::BLIT)
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
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
		;

		unsafe {
			self.ghi.device.cmd_pipeline_barrier2(vk_command_buffer, &dependency_info);
		}

		// Copy texture to swapchain image

		let image_blits = [vk::ImageBlit2::default()
			.src_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
			.src_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1),
			])
			.dst_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
			.dst_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1),
			])
		];

		let copy_image_info = vk::BlitImageInfo2::default()
			.src_image(source_texture.image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(swapchain_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_blits)
		;

		self.stages |= vk::PipelineStageFlags2::BLIT;

		unsafe { self.ghi.device.cmd_blit_image2(vk_command_buffer, &copy_image_info); }

		// Transition swapchain image to present layout

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.src_stage_mask(vk::PipelineStageFlags2::BLIT)
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
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
		;

		unsafe {
			self.ghi.device.cmd_pipeline_barrier2(vk_command_buffer, &dependency_info);
		}

		self.stages |= vk::PipelineStageFlags2::BOTTOM_OF_PIPE; // This is needed to correctly synchronize presentation when submitting the command buffer.
	}

	fn end(&mut self) {
		let command_buffer = self.get_command_buffer();

		if self.in_render_pass {
			unsafe {
				self.ghi.device.cmd_end_render_pass(command_buffer.command_buffer);
			}
		}
		
		unsafe {
			self.ghi.device.end_command_buffer(command_buffer.command_buffer).expect("Failed to end command buffer.");
		}
	}

	fn bind_descriptor_sets(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut impl graphics_hardware_interface::CommandBufferRecordable {
		if sets.is_empty() { return self; }
		
		let pipeline_layout = &self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let s = sets.iter().map(|descriptor_set_handle| {
			let internal_descriptor_set_handle = self.get_internal_descriptor_set_handle(*descriptor_set_handle);
			let descriptor_set = self.get_descriptor_set(&internal_descriptor_set_handle);
			let index_in_layout = pipeline_layout.descriptor_set_template_indices.get(&descriptor_set.descriptor_set_layout).unwrap();
			(*index_in_layout, internal_descriptor_set_handle, descriptor_set.descriptor_set)
		}).collect::<Vec<_>>();

		let vulkan_pipeline_layout_handle = pipeline_layout.pipeline_layout;

		for (descriptor_set_index, descriptor_set_handle, _) in s {
			if (descriptor_set_index as usize) < self.bound_descriptor_set_handles.len() {
				self.bound_descriptor_set_handles[descriptor_set_index as usize] = (descriptor_set_index, descriptor_set_handle);
				self.bound_descriptor_set_handles.truncate(descriptor_set_index as usize + 1);
			} else {
				assert_eq!(descriptor_set_index as usize, self.bound_descriptor_set_handles.len());
				self.bound_descriptor_set_handles.push((descriptor_set_index, descriptor_set_handle));
			}
		}

		let command_buffer = self.get_command_buffer();

		let partitions = partition(&self.bound_descriptor_set_handles, |e| e.0 as usize);

		// Always rebind all descriptor sets set by the user as previously bound descriptor sets might have been invalidated by a pipeline layout change
		for (base_index, descriptor_sets) in partitions {
			let base_index = base_index as u32;

			let descriptor_sets = descriptor_sets.iter().map(|(_, descriptor_set)| self.get_descriptor_set(descriptor_set).descriptor_set).collect::<Vec<_>>();

			unsafe {
				for bp in [vk::PipelineBindPoint::GRAPHICS, vk::PipelineBindPoint::COMPUTE] { // TODO: do this for all needed bind points
					self.ghi.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, bp, vulkan_pipeline_layout_handle, base_index, &descriptor_sets, &[]);
				}

				if self.pipeline_bind_point == vk::PipelineBindPoint::RAY_TRACING_KHR {
					self.ghi.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, vk::PipelineBindPoint::RAY_TRACING_KHR, vulkan_pipeline_layout_handle, base_index, &descriptor_sets, &[]);
				}
			}
		}

		self
	}

	fn sync_textures(&mut self, image_handles: &[graphics_hardware_interface::ImageHandle]) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		unsafe {
			self.consume_resources(&image_handles.iter().map(|image_handle| Consumption {
				handle: Handle::Image(self.get_internal_image_handle(*image_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}).collect::<Vec<_>>());

			let buffer_handles = image_handles.iter().filter_map(|image_handle| self.get_image(self.get_internal_image_handle(*image_handle)).staging_buffer).collect::<Vec<_>>();

			self.consume_resources(&buffer_handles.iter().map(|buffer_handle| Consumption {
				handle: Handle::Buffer(*buffer_handle),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}).collect::<Vec<_>>());
		};

		let command_buffer = self.get_command_buffer();
		let command_buffer = command_buffer.command_buffer;

		for image_handle in image_handles {
			let image = self.get_image(self.get_internal_image_handle(*image_handle));
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(staging_buffer_handle) = image.staging_buffer {
				let staging_buffer = self.get_buffer(staging_buffer_handle);

				let regions = [vk::BufferImageCopy2KHR::default()
					.buffer_offset(0)
					.buffer_row_length(0)
					.buffer_image_height(0)
					.image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
					.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
					.image_extent(image.extent)
				];

				let copy_image_to_buffer_info = vk::CopyImageToBufferInfo2KHR::default()
					.src_image(image.image)
					.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
					.dst_buffer(staging_buffer.buffer)
					.regions(&regions)
				;

				unsafe {
					self.ghi.device.cmd_copy_image_to_buffer2(command_buffer, &copy_image_to_buffer_info);
				}
			}
		}

		let mut texture_copies = Vec::new();

		for image_handle in image_handles {
			let internal_image_handle = self.get_internal_image_handle(*image_handle);
			let image = self.get_image(internal_image_handle);
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(_) = image.staging_buffer {
				texture_copies.push(graphics_hardware_interface::TextureCopyHandle(internal_image_handle.0));
			}
		}

		texture_copies
	}

	fn execute(mut self, wait_for_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle], signal_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle], execution_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		self.end();

		let command_buffer = self.get_command_buffer();

		let command_buffers = [command_buffer.command_buffer];

		let command_buffer_infos = [
			vk::CommandBufferSubmitInfo::default().command_buffer(command_buffers[0])
		];

		// TODO: Take actual stage masks

		let wait_semaphores = wait_for_synchronizer_handles.iter().map(|wait_for| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.get_synchronizer(*wait_for).vk_semaphore)
				.stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE | vk::PipelineStageFlags2::TRANSFER)
		}).collect::<Vec<_>>();

		let signal_semaphores = signal_synchronizer_handles.iter().map(|signal| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.get_synchronizer(*signal).vk_semaphore)
				.stage_mask(self.stages)
		}).collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores)
		;

		let execution_completion_synchronizer = &self.get_synchronizer(execution_synchronizer_handle);

		unsafe { self.ghi.device.queue_submit2(self.ghi.queue, &[submit_info], execution_completion_synchronizer.fence).expect("Failed to submit command buffer."); }

		for (&k, v) in &self.states {
			self.ghi.states.insert(k, *v);
		}
	}

	fn start_region(&self, name: &str) {
		let command_buffer = self.get_command_buffer();

		let name = std::ffi::CString::new(name).unwrap();

		let marker_info = vk::DebugUtilsLabelEXT::default()
			.label_name(name.as_c_str());

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.ghi.debug_utils {
				// println!("Starting region: {}", name.to_str().unwrap());
				debug_utils.cmd_begin_debug_utils_label(command_buffer.command_buffer, &marker_info);
			}
		}
	}

	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
		self.start_region(name);
		f(self);
		self.end_region();
	}

	fn end_region(&self) {
		let command_buffer = self.get_command_buffer();

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.ghi.debug_utils {
				// println!("Ending region");
				debug_utils.cmd_end_debug_utils_label(command_buffer.command_buffer);
			}
		}	
	}
}

impl graphics_hardware_interface::RasterizationRenderPassMode for VulkanCommandBufferRecording<'_> {
	/// Binds a pipeline to the GPU.
	fn bind_raster_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundRasterizationPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::GRAPHICS;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[graphics_hardware_interface::BufferDescriptor]) {
		let command_buffer = self.get_command_buffer();

		let buffers = buffer_descriptors.iter().map(|buffer_descriptor| self.ghi.buffers[buffer_descriptor.buffer.0 as usize].buffer).collect::<Vec<_>>();
		let offsets = buffer_descriptors.iter().map(|buffer_descriptor| buffer_descriptor.offset).collect::<Vec<_>>();

		// TODO: implent slot splitting
		unsafe { self.ghi.device.cmd_bind_vertex_buffers(command_buffer.command_buffer, 0, &buffers, &offsets); }
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &graphics_hardware_interface::BufferDescriptor) {
		let command_buffer = self.get_command_buffer();

		let buffer = self.ghi.buffers[buffer_descriptor.buffer.0 as usize];

		unsafe { self.ghi.device.cmd_bind_index_buffer(command_buffer.command_buffer, buffer.buffer, buffer_descriptor.offset, vk::IndexType::UINT16); }
	}

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self) {
		let command_buffer = self.get_command_buffer();
		unsafe { self.ghi.device.cmd_end_rendering(command_buffer.command_buffer); }
		self.in_render_pass = false;
	}
}

impl graphics_hardware_interface::BoundRasterizationPipelineMode for VulkanCommandBufferRecording<'_> {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		let command_buffer = self.get_command_buffer();

		let mesh = &self.ghi.meshes[mesh_handle.0 as usize];

		let buffers = [mesh.buffer];
		let offsets = [0];

		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe { self.ghi.device.cmd_bind_vertex_buffers(command_buffer_handle, 0, &buffers, &offsets); }
		unsafe { self.ghi.device.cmd_bind_index_buffer(command_buffer_handle, mesh.buffer, index_data_offset, vk::IndexType::UINT16); }

		// self.consume_resources_current();

		unsafe { self.ghi.device.cmd_draw_indexed(command_buffer_handle, mesh.index_count, 1, 0, 0, 0); }
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		// self.consume_resources_current();

		self.stages |= vk::PipelineStageFlags2::MESH_SHADER_EXT;

		unsafe {
			self.ghi.mesh_shading.cmd_draw_mesh_tasks(command_buffer_handle, x, y, z);
		}
	}

	fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		// self.consume_resources_current();

		unsafe {
			self.ghi.device.cmd_draw_indexed(command_buffer_handle, index_count, instance_count, first_index, vertex_offset, first_instance);
		}
	}
}

impl graphics_hardware_interface::BoundComputePipelineMode for VulkanCommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		let (x, y, z) = dispatch.get_extent().as_tuple();

		self.consume_resources_current(&[]);

		self.stages |= vk::PipelineStageFlags2::COMPUTE_SHADER;

		unsafe {
			self.ghi.device.cmd_dispatch(command_buffer_handle, x, y, z);
		}
	}

	fn indirect_dispatch(&mut self, buffer_handle: &graphics_hardware_interface::BaseBufferHandle, entry_index: usize) {
		let buffer = self.ghi.buffers[buffer_handle.0 as usize];

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		self.stages |= vk::PipelineStageFlags2::COMPUTE_SHADER;

		self.consume_resources_current(&[
			graphics_hardware_interface::Consumption{
				handle: graphics_hardware_interface::Handle::Buffer(buffer_handle.clone()),
				stages: graphics_hardware_interface::Stages::COMPUTE,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Indirect,
			}
		]);

		unsafe {
			self.ghi.device.cmd_dispatch_indirect(command_buffer_handle, buffer.buffer, entry_index as u64 * (3 * 4));
		}
	}
}

impl graphics_hardware_interface::BoundRayTracingPipelineMode for VulkanCommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: graphics_hardware_interface::BindingTables, x: u32, y: u32, z: u32) {
		use graphics_hardware_interface::GraphicsHardwareInterface;

		let command_buffer = self.get_command_buffer();
		let comamand_buffer_handle = command_buffer.command_buffer;

		let make_strided_range = |range: graphics_hardware_interface::BufferStridedRange| -> vk::StridedDeviceAddressRegionKHR {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(self.ghi.get_buffer_address(range.buffer_offset.buffer) as vk::DeviceSize + range.buffer_offset.offset as vk::DeviceSize)
				.stride(range.stride as vk::DeviceSize)
				.size(range.size as vk::DeviceSize)
		};

		let raygen_shader_binding_tables = make_strided_range(binding_tables.raygen);
		let miss_shader_binding_tables = make_strided_range(binding_tables.miss);
		let hit_shader_binding_tables = make_strided_range(binding_tables.hit);
		let callable_shader_binding_tables = if let Some(binding_table) = binding_tables.callable { make_strided_range(binding_table) } else { vk::StridedDeviceAddressRegionKHR::default() };

		self.consume_resources_current(&[]);

		unsafe {
			self.ghi.ray_tracing_pipeline.cmd_trace_rays(comamand_buffer_handle, &raygen_shader_binding_tables, &miss_shader_binding_tables, &hit_shader_binding_tables, &callable_shader_binding_tables, x, y, z)
		}
	}
}

fn into_vk_image_usage_flags(uses: graphics_hardware_interface::Uses, format: graphics_hardware_interface::Formats) -> vk::ImageUsageFlags {
	vk::ImageUsageFlags::empty()
	|
	if uses.intersects(graphics_hardware_interface::Uses::Image) { vk::ImageUsageFlags::SAMPLED } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::Clear) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::Storage) { vk::ImageUsageFlags::STORAGE } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::RenderTarget) && format != graphics_hardware_interface::Formats::Depth32 { vk::ImageUsageFlags::COLOR_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::DepthStencil) || format == graphics_hardware_interface::Formats::Depth32 { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::TransferSource) { vk::ImageUsageFlags::TRANSFER_SRC } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::TransferDestination) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
}

struct Mesh {
	buffer: vk::Buffer,
	allocation: graphics_hardware_interface::AllocationHandle,
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
	/// The alignment the resources needs when bound to a memory region.
	alignment: usize,
	/// The memory flags that need used to create the resource.
	memory_flags: u32,
}

fn image_type_from_extent(extent: vk::Extent3D) -> Option<vk::ImageType> {
	match extent {
		vk::Extent3D { width: 1.., height: 1, depth: 1 } => { Some(vk::ImageType::TYPE_1D) }
		vk::Extent3D { width: 1.., height: 1.., depth: 1 } => { Some(vk::ImageType::TYPE_2D) }
		vk::Extent3D { width: 1.., height: 1.., depth: 1.. } => { Some(vk::ImageType::TYPE_3D) }
		_ => { None }
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use utils::RGBA;

	#[test]
	fn render_triangle() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::render_triangle(&mut ghi);
	}

	#[test]
	fn render_present() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::present(&mut ghi);
	}

	#[test]
	fn render_multiframe_present() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_present(&mut ghi); // BUG: can see graphical artifacts, most likely synchronization issue
	}

	#[test]
	fn render_multiframe() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_rendering(&mut ghi);
	}

	#[test]
	fn render_dynamic_data() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::dynamic_data(&mut ghi);
	}

	#[test]
	fn render_with_descriptor_sets() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::descriptor_sets(&mut ghi);
	}

	#[test]
	fn render_with_multiframe_resources() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::multiframe_resources(&mut ghi);
	}

	#[test]
	fn render_with_ray_tracing() {
		let mut ghi = VulkanGHI::new(graphics_hardware_interface::Features::new().validation(true).ray_tracing(true)).expect("Failed to create VulkanGHI.");
		graphics_hardware_interface::tests::ray_tracing(&mut ghi);
	}

	#[test]
	fn test_uses_to_vk_usage_flags() {
		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Vertex);
		assert!(value.intersects(vk::BufferUsageFlags::VERTEX_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Index);
		assert!(value.intersects(vk::BufferUsageFlags::INDEX_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Uniform);
		assert!(value.intersects(vk::BufferUsageFlags::UNIFORM_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Storage);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::TransferSource);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_SRC));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::TransferDestination);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_DST));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructure);
		assert!(value.intersects(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Indirect);
		assert!(value.intersects(vk::BufferUsageFlags::INDIRECT_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::ShaderBindingTable);
		assert!(value.intersects(vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructureBuildScratch);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructureBuild);
		assert!(value.intersects(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR));
	}

	#[test]
	fn test_to_clear_value() {
		let value = to_clear_value(graphics_hardware_interface::ClearValue::Color(RGBA { r: 0.0, g: 1.0, b: 2.0, a: 3.0 }));
		assert_eq!(unsafe { value.color.float32 }, [0.0, 1.0, 2.0, 3.0]);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Depth(0.0));
		assert_eq!(unsafe { value.depth_stencil.depth }, 0.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Depth(1.0));
		assert_eq!(unsafe { value.depth_stencil.depth }, 1.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Integer(1, 2, 3, 4));
		assert_eq!(unsafe { value.color.int32 }, [1, 2, 3, 4]);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::None);
		assert_eq!(unsafe { value.color.float32 }, [0.0, 0.0, 0.0, 0.0]);
		assert_eq!(unsafe { value.depth_stencil.depth }, 0.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);
	}

	#[test]
	fn test_to_load_operation() {
		let value = to_load_operation(true);
		assert_eq!(value, vk::AttachmentLoadOp::LOAD);

		let value = to_load_operation(false);
		assert_eq!(value, vk::AttachmentLoadOp::CLEAR);
	}

	#[test]
	fn test_to_store_operation() {
		let value = to_store_operation(true);
		assert_eq!(value, vk::AttachmentStoreOp::STORE);

		let value = to_store_operation(false);
		assert_eq!(value, vk::AttachmentStoreOp::DONT_CARE);
	}

	#[test]
	fn test_texture_format_and_resource_use_to_image_layout() {
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, Some(graphics_hardware_interface::AccessPolicies::READ));
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, Some(graphics_hardware_interface::AccessPolicies::WRITE));
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::Depth32, graphics_hardware_interface::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, Some(graphics_hardware_interface::AccessPolicies::READ));
		assert_eq!(value, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, Some(graphics_hardware_interface::AccessPolicies::WRITE));
		assert_eq!(value, vk::ImageLayout::TRANSFER_DST_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Present, None);
		assert_eq!(value, vk::ImageLayout::PRESENT_SRC_KHR);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::READ_ONLY_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::Depth32, graphics_hardware_interface::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::ImageLayout::GENERAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::ShaderBindingTable, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Indirect, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
	}

	#[test]
	fn test_to_format() {
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R32_UINT);
		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R32_SINT);
		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8G8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::BC5);
		assert_eq!(value, vk::Format::BC5_UNORM_BLOCK);
		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8G8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16B16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8G8B8A8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::BC7);
		assert_eq!(value, vk::Format::BC7_SRGB_BLOCK);
		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8G8B8A8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16A16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16A16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16B16A16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::BGRAu8);
		assert_eq!(value, vk::Format::B8G8R8A8_SRGB);

		let value = to_format(graphics_hardware_interface::Formats::RGBu10u10u11);
		assert_eq!(value, vk::Format::R16G16_S10_5_NV);

		let value = to_format(graphics_hardware_interface::Formats::Depth32);
		assert_eq!(value, vk::Format::D32_SFLOAT);
	}

	#[test]
	fn test_to_shader_stage_flags() {
		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Vertex);
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Fragment);
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Compute);
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Task);
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Mesh);
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::RayGen);
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::ClosestHit);
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::AnyHit);
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Intersection);
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Miss);
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Callable);
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);
	}

	#[test]
	fn test_to_pipeline_stage_flags() {
		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::NONE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::NONE);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::VERTEX, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::VERTEX_SHADER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::MESH, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::MESH_SHADER_EXT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::FRAGMENT_SHADER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, Some(graphics_hardware_interface::Layouts::RenderTarget), None);
		assert_eq!(value, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, None, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::COMPUTE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::COMPUTE_SHADER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::COMPUTE, Some(graphics_hardware_interface::Layouts::Indirect), None);
		assert_eq!(value, vk::PipelineStageFlags2::DRAW_INDIRECT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::TRANSFER, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TRANSFER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::PRESENTATION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TOP_OF_PIPE);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::RAYGEN, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::CLOSEST_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::ANY_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::INTERSECTION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::MISS, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::CALLABLE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR);
	}

	#[test]
	fn test_to_access_flags() {
		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::VERTEX, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::NONE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::TRANSFER, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::PRESENTATION, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::NONE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ);
		
		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::Read, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);
		
		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::Read, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::Indirect, None);
		assert_eq!(value, vk::AccessFlags2::INDIRECT_COMMAND_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::ShaderBindingTable, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::TRANSFER, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::General, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::General, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR);
	}

	#[test]
	fn stages_to_vk_shader_stage_flags() {
		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::VERTEX.into();
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::FRAGMENT.into();
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::COMPUTE.into();
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::MESH.into();
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::TASK.into();
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::RAYGEN.into();
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::CLOSEST_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::ANY_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::INTERSECTION.into();
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::MISS.into();
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::CALLABLE.into();
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::TRANSFER.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::PRESENTATION.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::NONE.into();
		assert_eq!(value, vk::ShaderStageFlags::default());
	}

	#[test]
	fn datatype_to_vk_format() {
		let value: vk::Format = graphics_hardware_interface::DataTypes::U8.into();
		assert_eq!(value, vk::Format::R8_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::U16.into();
		assert_eq!(value, vk::Format::R16_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::U32.into();
		assert_eq!(value, vk::Format::R32_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int.into();
		assert_eq!(value, vk::Format::R32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int2.into();
		assert_eq!(value, vk::Format::R32G32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int3.into();
		assert_eq!(value, vk::Format::R32G32B32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float.into();
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float2.into();
		assert_eq!(value, vk::Format::R32G32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float3.into();
		assert_eq!(value, vk::Format::R32G32B32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SFLOAT);
	}

	#[test]
	fn datatype_size() {
		let value = graphics_hardware_interface::DataTypes::U8.size();
		assert_eq!(value, 1);

		let value = graphics_hardware_interface::DataTypes::U16.size();
		assert_eq!(value, 2);

		let value = graphics_hardware_interface::DataTypes::U32.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Int.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Int2.size();
		assert_eq!(value, 8);

		let value = graphics_hardware_interface::DataTypes::Int3.size();
		assert_eq!(value, 12);

		let value = graphics_hardware_interface::DataTypes::Int4.size();
		assert_eq!(value, 16);

		let value = graphics_hardware_interface::DataTypes::Float.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Float2.size();
		assert_eq!(value, 8);

		let value = graphics_hardware_interface::DataTypes::Float3.size();
		assert_eq!(value, 12);

		let value = graphics_hardware_interface::DataTypes::Float4.size();
		assert_eq!(value, 16);
	}

	#[test]
	fn shader_types_to_stages() {
		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Vertex.into();
		assert_eq!(value, graphics_hardware_interface::Stages::VERTEX);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Fragment.into();
		assert_eq!(value, graphics_hardware_interface::Stages::FRAGMENT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Compute.into();
		assert_eq!(value, graphics_hardware_interface::Stages::COMPUTE);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Task.into();
		assert_eq!(value, graphics_hardware_interface::Stages::TASK);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Mesh.into();
		assert_eq!(value, graphics_hardware_interface::Stages::MESH);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::RayGen.into();
		assert_eq!(value, graphics_hardware_interface::Stages::RAYGEN);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::ClosestHit.into();
		assert_eq!(value, graphics_hardware_interface::Stages::CLOSEST_HIT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::AnyHit.into();
		assert_eq!(value, graphics_hardware_interface::Stages::ANY_HIT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Intersection.into();
		assert_eq!(value, graphics_hardware_interface::Stages::INTERSECTION);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Miss.into();
		assert_eq!(value, graphics_hardware_interface::Stages::MISS);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Callable.into();
		assert_eq!(value, graphics_hardware_interface::Stages::CALLABLE);
	}

	#[test]
	fn test_image_type_from_extent() {
		let value = image_type_from_extent(vk::Extent3D { width: 1, height: 1, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 1, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 2, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_2D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 2, depth: 2 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_3D);
	}
}