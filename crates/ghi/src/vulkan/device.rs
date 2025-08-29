use std::{borrow::Cow, collections::VecDeque};

use ash::{vk::{self, Handle as _}};
use utils::{hash::{HashSet, HashSetExt}, sync::Mutex};
use ::utils::{hash::{HashMap, HashMapExt}, Extent};
use crate::{graphics_hardware_interface, image, raster_pipeline, render_debugger::RenderDebugger, sampler, vulkan::{queue::Queue, sampler::SamplerHandle, BufferCopy, Descriptor, DescriptorSetBindingHandle, DescriptorWrite, Descriptors, Handle, HandleLike, ImageCopy, ImageHandle, Next, Task, Tasks}, window, CommandBufferRecording, FrameKey, Instance, Size};

use super::{utils::{image_type_from_extent, into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format, to_shader_stage_flags, uses_to_vk_usage_flags}, AccelerationStructure, Allocation, Binding, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DebugCallbackData, DescriptorSet, DescriptorSetHandle, DescriptorSetLayout, Image, MemoryBackedResourceCreationResult, Mesh, Pipeline, PipelineLayout, Shader, Swapchain, Synchronizer, SynchronizerHandle, TransitionState, MAX_FRAMES_IN_FLIGHT};

pub struct Device {
	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,

	debug_data: *const DebugCallbackData,

	physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	pub(super) swapchain: ash::khr::swapchain::Device,
	surface: ash::khr::surface::Instance,
	pub(super) acceleration_structure: ash::khr::acceleration_structure::Device,
	pub(super) ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device,
	pub(super) mesh_shading: ash::ext::mesh_shader::Device,

	#[cfg(target_os = "linux")]
	pub(super) wayland_surface: ash::khr::wayland_surface::Instance,

	#[cfg(target_os = "windows")]
	pub(super) win32_surface: ash::khr::win32_surface::Instance,

	#[cfg(debug_assertions)]
	debugger: RenderDebugger,

	pub(super) frames: u8,

	pub(super) queues: Vec<Queue>,
	pub(super) buffers: Vec<Buffer>,
	pub(super) images: Vec<Image>,
	pub(super) allocations: Vec<Allocation>,
	pub(super) descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	pub(super) pipeline_layouts: Vec<PipelineLayout>,
	pub(super) bindings: Vec<Binding>,
	pub(super) descriptor_sets: Vec<DescriptorSet>,
	pub(super) meshes: Vec<Mesh>,
	pub(super) acceleration_structures: Vec<AccelerationStructure>,
	pub(super) shaders: Vec<Shader>,
	pub(super) pipelines: Vec<Pipeline>,
	pub(super) command_buffers: Vec<CommandBuffer>,
	pub(super) synchronizers: Vec<Synchronizer>,
	pub(super) swapchains: Vec<Swapchain>,

	resource_to_descriptor: HashMap<Handle, HashSet<(DescriptorSetBindingHandle, u32)>>,

	pub(super) descriptors: HashMap<DescriptorSetHandle, HashMap<u32, HashMap<u32, Descriptor>>>,
	descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32), HashSet<Handle>>,

	settings: graphics_hardware_interface::Features,

	pub(super) states: HashMap<Handle, TransitionState>,

	/// Tracks pending buffer synchronization operations.
	/// Buffer handle, pipeline stage
	pub(super) pending_buffer_syncs: Mutex<VecDeque<BufferHandle>>,
	pub(super) pending_image_syncs: Mutex<VecDeque<ImageHandle>>,

	memory_properties: vk::PhysicalDeviceMemoryProperties,

	#[cfg(debug_assertions)]
	/// Stores the debug names for resources.
	/// Used when inspecting resources from a rendering debugger such as RenderDoc.
	names: HashMap<graphics_hardware_interface::Handle, String>,

	tasks: VecDeque<Task>,
}

unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
	pub fn new(settings: graphics_hardware_interface::Features, instance: &Instance, queues: &mut [(graphics_hardware_interface::QueueSelection, &mut Option<graphics_hardware_interface::QueueHandle>)]) -> Result<Device, &'static str> {
		let vk_entry = &instance.entry;
		let vk_instance = &instance.instance;

		#[cfg(target_os = "linux")]
		let wayland_surface = ash::khr::wayland_surface::Instance::new(vk_entry, vk_instance);

		#[cfg(target_os = "windows")]
		let win32_surface = ash::khr::win32_surface::Instance::new(entry, instance);

		let flag_required_or_available = |feature: vk::Bool32, required: bool| {
			if required { feature != 0 } else { true }
		};

		let mut barycentric_required_features = vk::PhysicalDeviceFragmentShaderBarycentricFeaturesKHR::default()
			.fragment_shader_barycentric(false)
		;

		let mut physical_device_vulkan_11_required_features = vk::PhysicalDeviceVulkan11Features::default()
			.uniform_and_storage_buffer16_bit_access(true)
			.storage_buffer16_bit_access(true)
		;

		let mut physical_device_vulkan_12_required_features = vk::PhysicalDeviceVulkan12Features::default()
			.descriptor_indexing(true).descriptor_binding_partially_bound(true).runtime_descriptor_array(true).descriptor_binding_variable_descriptor_count(true)
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

		let mut physical_device_vulkan_13_required_features = vk::PhysicalDeviceVulkan13Features::default()
			.pipeline_creation_cache_control(true)
			.subgroup_size_control(true)
			.compute_full_subgroups(true)
			.synchronization2(true)
			.dynamic_rendering(true)
			.maintenance4(true)
		;

		let enabled_physical_device_required_features = vk::PhysicalDeviceFeatures::default()
			.shader_int16(true)
			.shader_int64(true)
			.shader_uniform_buffer_array_dynamic_indexing(true)
			.shader_storage_buffer_array_dynamic_indexing(true)
			.shader_storage_image_array_dynamic_indexing(true)
			.shader_storage_image_write_without_format(true)
			.texture_compression_bc(true)
			.geometry_shader(settings.geometry_shader)
		;

		let mut shader_atomic_float_required_features = vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT::default()
			.shader_buffer_float32_atomics(true)
			.shader_image_float32_atomics(true)
		;

		let mut physical_device_mesh_shading_required_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default()
			.task_shader(true)
			.mesh_shader(true)
		;

		let physical_devices = unsafe { vk_instance.enumerate_physical_devices().or(Err("Failed to enumerate physical devices"))? };

		let physical_device = if let Some(gpu_name) = settings.gpu {
			let physical_device = physical_devices.into_iter().find(|physical_device| {
				let properties = unsafe { vk_instance.get_physical_device_properties(*physical_device) };

				let name = properties.device_name_as_c_str();

				name.unwrap().to_str().unwrap() == gpu_name
			}).ok_or("Failed to find physical device")?;

			#[cfg(debug_assertions)]
			{
				let _ = unsafe { vk_instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		} else {
			let physical_device = physical_devices.into_iter().filter(|&physical_device| {
				let mut tools = [vk::PhysicalDeviceToolProperties::default(); 8];

				let tool_count = unsafe {
					vk_instance.get_physical_device_tool_properties_len(physical_device).unwrap()
				};

				unsafe {
					vk_instance.get_physical_device_tool_properties(physical_device, &mut tools[0..tool_count]).unwrap();
				};

				let mut vk_physical_device_memory_properties2 = vk::PhysicalDeviceMemoryProperties2::default();

				unsafe {
					vk_instance.get_physical_device_memory_properties2(physical_device, &mut vk_physical_device_memory_properties2);
				}

				for heap in &vk_physical_device_memory_properties2.memory_properties.memory_heaps[..vk_physical_device_memory_properties2.memory_properties.memory_heap_count as usize] {
					if heap.size == 0 {
						return false;
					}
				}

				let buffer_device_address_capture_replay = tools.iter().take(tool_count as usize).any(|tool| {
					let name = unsafe { std::ffi::CStr::from_ptr(tool.name.as_ptr()) };
					name.to_str().unwrap() == "RenderDoc"
				});

				let mut physical_device_mesh_shading_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
				let mut physical_device_vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default();
				let mut physical_device_barycentric_features = vk::PhysicalDeviceFragmentShaderBarycentricFeaturesKHR::default();
				let mut physical_device_features = vk::PhysicalDeviceFeatures2::default()
					.push_next(&mut physical_device_vulkan_12_features)
					.push_next(&mut physical_device_barycentric_features)
					.push_next(&mut physical_device_mesh_shading_features)
				;

				unsafe { vk_instance.get_physical_device_features2(physical_device, &mut physical_device_features) };

				let features = physical_device_features.features;

				features.sample_rate_shading != vk::FALSE &&
				flag_required_or_available(physical_device_vulkan_12_features.buffer_device_address_capture_replay, buffer_device_address_capture_replay) &&
				flag_required_or_available(physical_device_barycentric_features.fragment_shader_barycentric, barycentric_required_features.fragment_shader_barycentric != 0) &&
				features.shader_storage_image_array_dynamic_indexing != vk::FALSE &&
				features.shader_sampled_image_array_dynamic_indexing != vk::FALSE &&
				features.shader_storage_buffer_array_dynamic_indexing != vk::FALSE &&
				features.shader_uniform_buffer_array_dynamic_indexing != vk::FALSE &&
				features.shader_storage_image_write_without_format != vk::FALSE &&
				flag_required_or_available(features.geometry_shader, settings.geometry_shader) &&
				flag_required_or_available(physical_device_mesh_shading_features.mesh_shader, physical_device_mesh_shading_required_features.mesh_shader != 0) &&
				flag_required_or_available(physical_device_mesh_shading_features.task_shader, physical_device_mesh_shading_required_features.task_shader != 0)
			}).max_by_key(|physical_device| {
				let properties = unsafe { vk_instance.get_physical_device_properties(*physical_device) };

				let mut device_score = 0u64;

				device_score += match properties.device_type {
					vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
					vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
					vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
					vk::PhysicalDeviceType::CPU => 100,
					_ => 0,
				};

				device_score
			}).ok_or("Failed to choose a best physical device")?;

			#[cfg(debug_assertions)]
			{
				let _ = unsafe { vk_instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		};

		let queue_family_properties = unsafe { vk_instance.get_physical_device_queue_family_properties(physical_device) };

		let queue_create_infos = queues.iter().map(|(d, _)| {
			let mut queue_family_index = queue_family_properties
				.iter()
				.enumerate()
				.filter_map(|(index, info)| {
					let mask = match d.r#type {
						graphics_hardware_interface::CommandBufferType::COMPUTE => vk::QueueFlags::COMPUTE,
						graphics_hardware_interface::CommandBufferType::GRAPHICS => vk::QueueFlags::GRAPHICS,
						graphics_hardware_interface::CommandBufferType::TRANSFER => vk::QueueFlags::TRANSFER,
					};

					if info.queue_flags.contains(mask) {
						Some((index as u32, info.queue_flags.as_raw().count_ones()))
					} else {
						None
					}
				})
				.collect::<Vec<_>>()
			;

			queue_family_index.sort_by(|(_, a_bit_count), (_, b_bit_count)| {
				a_bit_count.cmp(b_bit_count)
			});

			let least_bits_queue_family_index = queue_family_index.first().unwrap().0;

			vk::DeviceQueueCreateInfo::default()
				.queue_family_index(least_bits_queue_family_index)
				.queue_priorities(&[1.0])
		}).collect::<Vec<_>>();

		let memory_properties = unsafe { vk_instance.get_physical_device_memory_properties(physical_device) };

		let available_device_extensions = unsafe { vk_instance.enumerate_device_extension_properties(physical_device) }.expect("Could not get supported device extensions");

		let is_device_extension_available = |name: &str| {
			available_device_extensions.iter().any(|extension| {
				unsafe { std::ffi::CStr::from_ptr(extension.extension_name.as_ptr()).to_str().unwrap() == name }
			})
		};

		let mut device_extension_names = Vec::new();

		device_extension_names.push(ash::khr::swapchain::NAME.as_ptr());

		if settings.ray_tracing {
			device_extension_names.push(ash::khr::acceleration_structure::NAME.as_ptr());
			device_extension_names.push(ash::khr::deferred_host_operations::NAME.as_ptr());
			device_extension_names.push(ash::khr::ray_tracing_pipeline::NAME.as_ptr());
			device_extension_names.push(ash::khr::ray_tracing_maintenance1::NAME.as_ptr());
		}

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

		device_extension_names.push(ash::ext::shader_atomic_float::NAME.as_ptr());

		let device_create_info = vk::DeviceCreateInfo::default();

		let device_create_info = if is_device_extension_available(ash::ext::mesh_shader::NAME.to_str().unwrap().as_str()) {
			device_extension_names.push(ash::ext::mesh_shader::NAME.as_ptr());
			device_create_info.push_next(&mut physical_device_mesh_shading_required_features)
		} else {
			return Err("Mesh shader extension not available");
		};

		let mut swapchain_maintenance_features = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default().swapchain_maintenance1(true);

		device_extension_names.push(ash::ext::swapchain_maintenance1::NAME.as_ptr());

		let device_create_info = device_create_info
			.push_next(&mut physical_device_vulkan_11_required_features)
			.push_next(&mut physical_device_vulkan_12_required_features)
			.push_next(&mut physical_device_vulkan_13_required_features)
			.push_next(&mut shader_atomic_float_required_features)
			.push_next(&mut barycentric_required_features)
			.push_next(&mut swapchain_maintenance_features)
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names)
			.enabled_features(&enabled_physical_device_required_features)
		;

		let device_create_info = if settings.ray_tracing {
			device_create_info
				.push_next(&mut physical_device_acceleration_structure_features)
				.push_next(&mut physical_device_ray_tracing_pipeline_features)
		} else {
			device_create_info
		};

		let device: ash::Device = unsafe { vk_instance.create_device(physical_device, &device_create_info, None).map_err(|e| {
				match e {
					vk::Result::ERROR_OUT_OF_HOST_MEMORY => "Out of host memory",
					vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => "Out of device memory",
					vk::Result::ERROR_INITIALIZATION_FAILED => "Initialization failed",
					vk::Result::ERROR_EXTENSION_NOT_PRESENT => "Extension not present",
					vk::Result::ERROR_FEATURE_NOT_PRESENT => "Feature not present",
					vk::Result::ERROR_TOO_MANY_OBJECTS => "Too many objects",
					vk::Result::ERROR_DEVICE_LOST => "Device lost",
					_ => "Failed to create a device"
				}
			})?
		};

		let queues = queues.iter_mut().zip(queue_create_infos.iter()).enumerate().map(|(index, ((_, queue_handle), create_info))| {
			let vk_queue = unsafe { device.get_device_queue(create_info.queue_family_index, 0) };

			**queue_handle = Some(graphics_hardware_interface::QueueHandle(index as u64));

			Queue {
				queue_family_index: create_info.queue_family_index,
				queue_index: 0,
				vk_queue,
			}
		}).collect::<Vec<_>>();

		let acceleration_structure = ash::khr::acceleration_structure::Device::new(&vk_instance, &device);
		let ray_tracing_pipeline = ash::khr::ray_tracing_pipeline::Device::new(&vk_instance, &device);

		let swapchain = ash::khr::swapchain::Device::new(&vk_instance, &device);
		let surface = ash::khr::surface::Instance::new(&vk_entry, &vk_instance);

		let mesh_shading = ash::ext::mesh_shader::Device::new(&vk_instance, &device);

		let debug_utils = if settings.validation {
			Some(ash::ext::debug_utils::Device::new(&vk_instance, &device))
		} else {
			None
		};

		Ok(Device {
			debug_utils,
			debug_data: instance.debug_data.as_ref() as *const DebugCallbackData,

			memory_properties,

			#[cfg(target_os = "linux")]
			wayland_surface,

			#[cfg(target_os = "windows")]
			win32_surface,

			physical_device,
			device,
			swapchain,
			surface,
			acceleration_structure,
			ray_tracing_pipeline,
			mesh_shading,

			#[cfg(debug_assertions)]
			debugger: RenderDebugger::new(),

			frames: 2, // Assuming double buffering

			queues,
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

			pending_buffer_syncs: Mutex::new(VecDeque::with_capacity(128)),
			pending_image_syncs: Mutex::new(VecDeque::with_capacity(128)),

			tasks: VecDeque::with_capacity(128),

			#[cfg(debug_assertions)]
			names: HashMap::with_capacity(4096),
		})
	}

	#[cfg(debug_assertions)]
	fn get_log_count(&self) -> u64 {
		use std::sync::atomic::Ordering;
		unsafe { &(*self.debug_data) }.error_count.load(Ordering::SeqCst)
	}

	pub(super) fn get_syncronizer_handles(&self, synchroizer_handle: graphics_hardware_interface::SynchronizerHandle) -> Vec<SynchronizerHandle> {
		let mut synchronizer_handles = Vec::with_capacity(3);
		let mut synchronizer_handle = Some(SynchronizerHandle(synchroizer_handle.0));
		while let Some(sh) = synchronizer_handle {
			synchronizer_handles.push(sh);
			synchronizer_handle = self.synchronizers[sh.0 as usize].next;
		}
		synchronizer_handles
	}

	fn create_vulkan_graphics_pipeline_create_info<'a, R>(&'a mut self, builder: raster_pipeline::Builder, after_build: impl FnOnce(&'a mut Self, raster_pipeline::Builder, vk::GraphicsPipelineCreateInfo) -> R) -> R {
		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
		;

		let pipeline_layout = &self.pipeline_layouts[builder.layout.0 as usize];

		let pipeline_create_info = pipeline_create_info.layout(pipeline_layout.pipeline_layout);

		let mut vertex_input_attribute_descriptions = vec![];

		let mut offset_per_binding = [0, 0, 0, 0, 0, 0, 0, 0]; // Assume 8 bindings max

		for (i, vertex_element) in builder.vertex_elements.iter().enumerate() {
			let ve = vk::VertexInputAttributeDescription::default()
				.binding(vertex_element.binding)
				.location(i as u32)
				.format(vertex_element.format.into())
				.offset(offset_per_binding[vertex_element.binding as usize]);

			vertex_input_attribute_descriptions.push(ve);

			offset_per_binding[vertex_element.binding as usize] += vertex_element.format.size() as u32;
		}

		let vertex_binding_descriptions = if let Some(max_binding) = builder.vertex_elements.iter().map(|ve| ve.binding).max() {
			let max_binding = max_binding as usize + 1;

			let mut vertex_binding_descriptions = Vec::with_capacity(max_binding);

			for i in 0..max_binding {
				vertex_binding_descriptions.push(
					vk::VertexInputBindingDescription::default()
					.binding(i as u32)
					.stride(offset_per_binding[i as usize])
					.input_rate(vk::VertexInputRate::VERTEX)
				)
			}

			vertex_binding_descriptions
		} else {
			Vec::new()
		};

		let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
			.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
			.vertex_binding_descriptions(&vertex_binding_descriptions);

		let pipeline_create_info = pipeline_create_info.vertex_input_state(&vertex_input_state);

		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);
		let mut entries = [vk::SpecializationMapEntry::default(); 32];
		let mut entry_count = 0;
		let specilization_info_count = 0;

		let stages = builder.shaders
			.iter()
			.map(|stage| {
				for entry in stage.specialization_map.iter() {
					specialization_entries_buffer.extend_from_slice(entry.get_data());

					entries[entry_count] = vk::SpecializationMapEntry::default()
						.constant_id(entry.get_constant_id())
						.size(entry.get_size())
						.offset(specialization_entries_buffer.len() as u32);

					entry_count += 1;
				}

				let shader = &self.shaders[stage.handle.0 as usize];

				assert!(specilization_info_count == 0);

				vk::PipelineShaderStageCreateInfo::default()
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		let pipeline_create_info = pipeline_create_info.stages(&stages);

		let pipeline_color_blend_attachments = builder.render_targets.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|_| {
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

		let color_attachement_formats: Vec<vk::Format> = builder.render_targets.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|a| to_format(a.format)).collect::<Vec<_>>();

		let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
			.logic_op_enable(false)
			.logic_op(vk::LogicOp::COPY)
			.attachments(&pipeline_color_blend_attachments)
			.blend_constants([0.0, 0.0, 0.0, 0.0]);

		let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
			.color_attachment_formats(&color_attachement_formats)
			.depth_attachment_format(vk::Format::UNDEFINED);

		let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

		let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
			.depth_test_enable(true)
			.depth_write_enable(true)
			.depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
			.depth_bounds_test_enable(false)
			.stencil_test_enable(false)
			.front(vk::StencilOpState::default())
			.back(vk::StencilOpState::default())
		;

		let pipeline_create_info = if let Some(_) = builder.render_targets.iter().find(|a| a.format == graphics_hardware_interface::Formats::Depth32) {
			rendering_info = rendering_info.depth_attachment_format(vk::Format::D32_SFLOAT);
			let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);
			let pipeline_create_info = pipeline_create_info.depth_stencil_state(&depth_stencil_state);
			pipeline_create_info
		} else {
			let pipeline_create_info = pipeline_create_info.push_next(&mut rendering_info);
			pipeline_create_info
		};

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false)
		;

		let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

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
			.cull_mode(vk::CullModeFlags::BACK)
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

		let pipeline_create_info = pipeline_create_info
			.viewport_state(&viewport_state)
			.dynamic_state(&dynamic_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state)
		;

		after_build(self, builder, pipeline_create_info)
	}

	fn create_vulkan_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_graphics_pipeline_create_info(builder, |this, builder, pipeline_create_info| {
			let pipeline_create_infos = [pipeline_create_info];

			let pipelines = unsafe { this.device.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None).expect("No pipeline") };

			let pipeline = pipelines[0];

			let handle = graphics_hardware_interface::PipelineHandle(this.pipelines.len() as u64);

			let resource_access: Vec<((u32, u32), (graphics_hardware_interface::Stages, graphics_hardware_interface::AccessPolicies))> = builder.shaders.iter().map(|s| {
				let shader = &this.shaders[s.handle.0 as usize];
				shader.shader_binding_descriptors.iter().map(|sbd| {
					((sbd.set, sbd.binding), (Into::<graphics_hardware_interface::Stages>::into(s.stage), sbd.access))
				})
			}).flatten().collect::<Vec<_>>();

			this.pipelines.push(Pipeline {
				pipeline,
				shader_handles: HashMap::new(),
				resource_access,
			});

			handle
		})
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
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	fn destroy_vulkan_buffer(&self, buffer: &graphics_hardware_interface::BaseBufferHandle) {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.destroy_buffer(buffer, None) };
	}

	fn get_vulkan_buffer_address(&self, buffer: &graphics_hardware_interface::BaseBufferHandle, _allocation: &graphics_hardware_interface::AllocationHandle) -> u64 {
		let buffer = self.buffers.get(buffer.0 as usize).expect("No buffer with that handle.").buffer.clone();
		unsafe { self.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)) }
	}

	fn create_vulkan_texture(&self, name: Option<&str>, extent: vk::Extent3D, format: graphics_hardware_interface::Formats, resource_uses: graphics_hardware_interface::Uses, mip_levels: u32, array_layers: u32) -> MemoryBackedResourceCreationResult<vk::Image> {
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

	pub(super) fn get_image_subresource_layout(&self, texture: &graphics_hardware_interface::ImageHandle, mip_level: u32) -> graphics_hardware_interface::ImageSubresourceLayout {
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
				let wayland_surface_create_info = vk::WaylandSurfaceCreateInfoKHR::default()
					.display(os_handles.display)
					.surface(os_handles.surface);

				unsafe { self.wayland_surface.create_wayland_surface(&wayland_surface_create_info, None).expect("No surface") }
			}
			#[cfg(target_os = "windows")]
			window::OSHandles::Win32(os_handles) => {
				let win32_surface = ash::khr::win32_surface::Instance::new(&self.entry, &self.instance);

				let win32_surface_create_info = vk::Win32SurfaceCreateInfoKHR::default()
					.hinstance(os_handles.hinstance.0 as isize)
					.hwnd(os_handles.hwnd.0 as isize);

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
		let memory_property_flags = {
			let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuRead) { vk::MemoryPropertyFlags::HOST_VISIBLE } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) { vk::MemoryPropertyFlags::HOST_COHERENT } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::GpuRead) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };
			memory_property_flags |= if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::GpuWrite) { vk::MemoryPropertyFlags::DEVICE_LOCAL } else { vk::MemoryPropertyFlags::empty() };

			memory_property_flags
		};

		let memory_properties = &self.memory_properties;

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

	/// Builds a buffer object with the given name, resource uses, size, Vulkan buffer usage flags, and device accesses.
	fn build_buffer_internal(&mut self, name: Option<&str>, resource_uses: crate::Uses, size: usize, vk_buffer_usage_flags: vk::BufferUsageFlags, device_accesses: graphics_hardware_interface::DeviceAccesses) -> Buffer {
		let buffer = if size != 0 {
			let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_buffer_usage_flags);
			let (allocation_handle, _) = self.create_allocation_internal(buffer_creation_result.size, buffer_creation_result.memory_flags.into(), device_accesses);
			let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);
			Buffer {
				next: None,
				staging: None,
				buffer: buffer_creation_result.resource,
				size,
				device_address,
				pointer,
				uses: resource_uses,
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
			}
		};

		buffer
	}

	/// Builds a buffer and returns its handle.
	fn create_buffer_internal(&mut self, name: Option<&str>, resource_uses: crate::Uses, size: usize, vk_buffer_usage_flags: vk::BufferUsageFlags, device_accesses: graphics_hardware_interface::DeviceAccesses) -> BufferHandle {
		let buffer = self.build_buffer_internal(name, resource_uses, size, vk_buffer_usage_flags, device_accesses);

		let buffer_handle = BufferHandle(self.buffers.len() as u64);

		self.buffers.push(buffer);

		buffer_handle
	}

	fn build_image_internal(&mut self, next: Option<ImageHandle>, name: Option<&str>, format: crate::Formats, device_accesses: crate::DeviceAccesses, array_layers: u32, size: usize, extent: vk::Extent3D, resource_uses: crate::Uses,) -> Image {
		let texture_creation_result = self.create_vulkan_texture(name, extent, format, resource_uses | graphics_hardware_interface::Uses::TransferSource, 1, array_layers);

		let m_device_accesses = if device_accesses.intersects(graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::CpuRead) {
			graphics_hardware_interface::DeviceAccesses::GpuRead | graphics_hardware_interface::DeviceAccesses::GpuWrite
		} else {
			device_accesses
		};

		let (allocation_handle, _) = self.create_allocation_internal(texture_creation_result.size, texture_creation_result.memory_flags.into(), m_device_accesses);

		let _ = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);

		let image_view = self.create_vulkan_image_view(name, &texture_creation_result.resource, format, 0, 0, array_layers);

		let staging_buffer = if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuRead) {
			let staging_buffer_handle = self.create_buffer_internal(name, crate::Uses::TransferDestination, size, vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::CpuRead);

			Some(staging_buffer_handle)
		} else if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) {
			let staging_buffer_handle = self.create_buffer_internal(name, crate::Uses::TransferSource, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::CpuWrite);

			Some(staging_buffer_handle)
		} else {
			None
		};

		let image_views = {
			let mut image_views = [vk::ImageView::null(); 8];

			for i in 0..array_layers {
				image_views[i as usize] = self.create_vulkan_image_view(name, &texture_creation_result.resource, format, 0, i, 1);
			}

			image_views
		};

		Image {
			next,
			size: texture_creation_result.size,
			staging_buffer,
			image: texture_creation_result.resource,
			image_view,
			image_views,
			extent,
			access: device_accesses,
			format: to_format(format),
			format_: format,
			uses: resource_uses,
			layers: array_layers,
		}
	}

	fn create_image_internal(&mut self, next: Option<ImageHandle>, name: Option<&str>, format: crate::Formats, device_accesses: crate::DeviceAccesses, array_layers: u32, size: usize, extent: vk::Extent3D, resource_uses: crate::Uses,) -> ImageHandle {
		let texture_handle = ImageHandle(self.images.len() as u64);

		let image = self.build_image_internal(next, name, format, device_accesses, array_layers, size, extent, resource_uses,);

		self.images.push(image);

		texture_handle
	}

	fn create_synchronizer_internal(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u64);

		self.synchronizers.push(Synchronizer {
			next: None,
			name: name.map(|name| name.to_string()),
			signaled,
			fence: self.create_vulkan_fence(signaled),
			semaphore: self.create_vulkan_semaphore(name, signaled),
		});

		synchronizer_handle
	}

	fn resize_buffer_internal(&mut self, buffer_handle: BufferHandle, size: usize) {
		let current_buffer = &self.buffers[buffer_handle.0 as usize];

		if current_buffer.size >= size {
			return;
		}

		assert!(current_buffer.staging.is_none(), "Cannot resize buffers with staging buffers");

		if current_buffer.size != 0 {
			let current_vk_buffer = current_buffer.buffer;

			self.tasks.push_back(Task::delete_vulkan_buffer(current_vk_buffer, None));
			self.tasks.push_back(Task::update_buffer_descriptor(buffer_handle, None));

			// todo!("copy data from old buffer to new buffer");
		}

		let new_buffer = self.build_buffer_internal(None, current_buffer.uses, size, uses_to_vk_usage_flags(current_buffer.uses) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::CpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);

		self.buffers[buffer_handle.0 as usize] = new_buffer;
	}

	fn write_internal(&mut self, writes: &[DescriptorWrite]) {
		if writes.is_empty() { return; }

		let mut buffers: StableVec<vk::DescriptorBufferInfo, 1024> = StableVec::new();
		let mut images: StableVec<vk::DescriptorImageInfo, 1024> = StableVec::new();
		let mut acceleration_structures: StableVec<vk::AccelerationStructureKHR, 1024> = StableVec::new();
		let mut acceleration_structures_writes: StableVec<vk::WriteDescriptorSetAccelerationStructureKHR, 1024> = StableVec::new();

		let writes = writes.iter().filter_map(|descriptor_set_write| {
			let binding_handle = descriptor_set_write.binding;
			let binding = binding_handle.access(&self.bindings);
			let descriptor_set_handle = binding.descriptor_set_handle;
			let descriptor_set = descriptor_set_handle.access(&self.descriptor_sets);

			let binding_index = binding.index;
			let descriptor_type = binding.descriptor_type;

			match descriptor_set_write.write {
				Descriptors::Buffer { handle, size } => {
					let buffer_handle = handle;
					let buffer = &self.buffers[buffer_handle.0 as usize];

					let res = if !buffer.buffer.is_null() {
						let e = buffers.append([
							vk::DescriptorBufferInfo::default().buffer(buffer.buffer).offset(0u64).range(match size {
								graphics_hardware_interface::Ranges::Size(size) => { size as u64 }
								graphics_hardware_interface::Ranges::Whole => { vk::WHOLE_SIZE }
							})
						]);

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.buffer_info(e)
						;

						Some(write_info)
					} else {
						None
					};

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Buffer{ size, buffer: buffer_handle });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Buffer(buffer_handle));
					self.resource_to_descriptor.entry(Handle::Buffer(buffer_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));

					res
				},
				Descriptors::Image{ handle, layout } => {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

					let image_handle = handle;

					let image = &self.images[image_handle.0 as usize];

					let res = if !image.image.is_null() && !image.image_view.is_null() {
						let e = images.append([vk::DescriptorImageInfo::default().image_layout(texture_format_and_resource_use_to_image_layout(image.format_, layout, None)).image_view(image.image_view)]);

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(&e)
						;

						Some(write_info)
					} else {
						None
					};

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Image{ layout, image: image_handle });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Image(image_handle));
					self.resource_to_descriptor.entry(Handle::Image(image_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));

					res
				},
				Descriptors::CombinedImageSampler{ image_handle, sampler_handle, layout, layer } => {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

					let image = &self.images[image_handle.0 as usize];

					let res = if !image.image.is_null() {
						let image_view = if let Some(layer) = layer { // If the descriptor asks for a subresource, we need to create a new image view
							image.image_views[layer as usize]
						} else {
							image.image_view
						};

						let e = images.append([vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, layout, None))
							.image_view(image_view)
							.sampler(vk::Sampler::from_raw(sampler_handle.0))
						]);

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(e)
						;

						Some(write_info)
					} else {
						None
					};

					self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::CombinedImageSampler{ image: image_handle, sampler: vk::Sampler::from_raw(sampler_handle.0), layout });
					self.descriptor_set_to_resource.entry((descriptor_set_handle, binding_index)).or_insert_with(HashSet::new).insert(Handle::Image(image_handle));
					self.resource_to_descriptor.entry(Handle::Image(image_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));

					res
				},
				Descriptors::Sampler{ handle } => {
					let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
					let sampler_handle = handle;
					let e = images.append([vk::DescriptorImageInfo::default().sampler(vk::Sampler::from_raw(sampler_handle.0))]);

					let write_info = vk::WriteDescriptorSet::default()
						.dst_set(descriptor_set.descriptor_set)
						.dst_binding(binding_index)
						.dst_array_element(descriptor_set_write.array_element)
						.descriptor_type(descriptor_type)
						.image_info(e)
					;

					// self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Sampler{ sampler: vk::Sampler::from_raw(sampler_handle.0) });
					// self.resource_to_descriptor.entry(Handle::Sampler(sampler_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));

					Some(write_info)
				}
				_ => None,
			}
		}).collect::<Vec<_>>();

		unsafe { self.device.update_descriptor_sets(&writes, &[]) };
	}

	fn process_tasks(&mut self, sequence_index: u8) {
		let mut descriptor_writes = Vec::with_capacity(32);

		self.tasks.retain(|e| {
			if let Some(e) = e.frame() {
				if e != sequence_index {
					return true;
				}
			}

			// Helps debug issues related to use after delete cases.
			let disable_deletions = true;

			match e.task() {
				Tasks::DeleteVulkanImage { handle, } => {
					if disable_deletions { return true; }
					unsafe { self.device.destroy_image(*handle, None); }
				}
				Tasks::DeleteVulkanImageView { handle, } => {
					if disable_deletions { return true; }
					unsafe { self.device.destroy_image_view(*handle, None); }
				}
				Tasks::DeleteVulkanBuffer { handle, } => {
					if disable_deletions { return true; }
					unsafe { self.device.destroy_buffer(*handle, None); }
				}
				Tasks::UpdateBufferDescriptors { handle } => {
					if let Some(e) = self.resource_to_descriptor.get(&(*handle).into()) {
						for (binding_handle, index) in e {
							let binding = binding_handle.access(&self.bindings);

							if let Some(descriptor) = self.descriptors.get(&binding.descriptor_set_handle).and_then(|d| d.get(&binding.index)).and_then(|d| d.get(&index)) {
								match descriptor {
									Descriptor::Buffer { size, .. } => {
										descriptor_writes.push(DescriptorWrite::new(Descriptors::Buffer { handle: *handle, size: *size }, *binding_handle).index(*index));
									}
									_ => {
										println!("Unexpected descriptor type for buffer handle {:#?}", handle);
									}
								}
							}
						}
					} else {
						println!("No binding found for buffer handle ({:#?}#{})", handle, sequence_index);
					}
				}
				Tasks::UpdateImageDescriptors { handle } => {
					if let Some(e) = self.resource_to_descriptor.get(&(*handle).into()) {
						for (binding_handle, index) in e {
							let binding = binding_handle.access(&self.bindings);

							if let Some(descriptor) = self.descriptors.get(&binding.descriptor_set_handle).and_then(|d| d.get(&binding.index)).and_then(|d| d.get(&index)) {
								match descriptor {
									Descriptor::Image { layout, .. } => {
										descriptor_writes.push(DescriptorWrite::new(Descriptors::Image { handle: *handle, layout: *layout }, *binding_handle).index(*index));
									}
									Descriptor::CombinedImageSampler { sampler, layout, .. } => {
										descriptor_writes.push(DescriptorWrite::new(Descriptors::CombinedImageSampler { image_handle: *handle, sampler_handle: SamplerHandle(sampler.as_raw()), layout: *layout, layer: None }, *binding_handle).index(*index));
									}
									_ => {
										println!("Unexpected descriptor type for image handle {:#?}", handle);
									}
								}
							}
						}
					} else {
						#[cfg(debug_assertions)]
						{
							let identifier = self.names.get(&graphics_hardware_interface::Handle::Image(graphics_hardware_interface::ImageHandle(handle.root(&self.images).0))).map(|s| s.clone()).unwrap_or_else(|| format!("{:#?}", handle));

							println!("No binding found for image ({}#{})", identifier, sequence_index);
						}
					}
				}
				Tasks::WriteDescriptor { binding_handle, descriptor } => {
					descriptor_writes.push(DescriptorWrite::new(*descriptor, *binding_handle));
				}
				Tasks::Other(f) => {
					f();
				}
			}

			false
		});

		self.write_internal(&descriptor_writes);
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		unsafe {
			self.device.device_wait_idle().expect("Failed to wait for device idle");

			self.command_buffers.iter().for_each(|command_buffer| {
				command_buffer.frames.iter().for_each(|command_buffer| {
					self.device.destroy_command_pool(command_buffer.command_pool, None);
				});
			});

			self.swapchains.iter().for_each(|swapchain| {
				self.swapchain.destroy_swapchain(swapchain.swapchain, None);
				self.surface.destroy_surface(swapchain.surface, None);
			});

			self.synchronizers.iter().for_each(|synchronizer| {
				self.device.destroy_semaphore(synchronizer.semaphore, None);
				self.device.destroy_fence(synchronizer.fence, None);
			});

			self.descriptor_sets_layouts.iter().for_each(|descriptor_set_layout| {
				self.device.destroy_descriptor_set_layout(descriptor_set_layout.descriptor_set_layout, None);
			});

			self.pipelines.iter().for_each(|pipeline| {
				self.device.destroy_pipeline(pipeline.pipeline, None);
			});

			self.meshes.iter().for_each(|mesh| {
				self.device.destroy_buffer(mesh.buffer, None);
			});

			self.buffers.iter().for_each(|buffer| {
				self.device.destroy_buffer(buffer.buffer, None);
			});

			self.images.iter().for_each(|image| {
				self.device.destroy_image(image.image, None);

				self.device.destroy_image_view(image.image_view, None);

				for vk_image_view in image.image_views {
					self.device.destroy_image_view(vk_image_view, None);
				}
			});

			self.shaders.iter().for_each(|shader| {
				self.device.destroy_shader_module(shader.shader, None);
			});

			self.pipeline_layouts.iter().for_each(|pipeline_layout| {
				self.device.destroy_pipeline_layout(pipeline_layout.pipeline_layout, None);
			});

			self.allocations.iter().for_each(|allocation| {
				self.device.free_memory(allocation.memory, None);
			});

			self.device.destroy_device(None);
		}
	}
}

impl graphics_hardware_interface::Device for Device {
	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		if self.frames == frames { return; }

		if frames > MAX_FRAMES_IN_FLIGHT as u8 {
			panic!("Cannot set frames in flight to more than {}", MAX_FRAMES_IN_FLIGHT);
		}

		todo!("Update swapchain synchronizers");

		let current_frames = self.frames;
		let target_frames = frames;
		let delta_frames = target_frames as i8 - current_frames as i8;

		if delta_frames > 0 {
			let to_extend = self.images.iter().filter_map(|image| {
				let next = image.next?;

				let mut handle = next;

				while let Some(h) = self.images[handle.0 as usize].next {
					handle = h;
				}

				handle.into()
			}).collect::<Vec<_>>();

			for image_handle in to_extend {
				let current_image = &self.images[image_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name: Option<&str> = None;

				#[cfg(not(debug_assertions))]
				let name = None;

				let next = current_image.next;
				let format = current_image.format_;
				let access = current_image.access;
				let array_layers = current_image.layers;
				let size = current_image.size;
				let extent = current_image.extent;
				let resource_uses = current_image.uses;

				let new_image = self.create_image_internal(next, name, format, access, array_layers, size, extent, resource_uses);

				let current_image = &mut self.images[image_handle.0 as usize];
				current_image.next = Some(new_image);
			}

			let to_extend = self.synchronizers.iter().filter_map(|synchronizer| {
				let next = synchronizer.next?;

				let mut handle = next;

				while let Some(h) = self.synchronizers[handle.0 as usize].next {
					handle = h;
				}

				handle.into()
			}).collect::<Vec<_>>();

			for synchronizer_handle in to_extend {
				let current_synchronizer = &self.synchronizers[synchronizer_handle.0 as usize];

				let name = current_synchronizer.name.clone();
				let name = name.as_ref().map(|s| s.as_str());
				let signaled = current_synchronizer.signaled;

				let new_synchronizer = self.create_synchronizer_internal(name, signaled);

				let current_synchronizer = &mut self.synchronizers[synchronizer_handle.0 as usize];
				current_synchronizer.next = Some(new_synchronizer);
			}

			for command_buffer in &mut self.command_buffers {
				let queue = &self.queues[command_buffer.queue_handle.0 as usize];
				let command_pool_create_info = vk::CommandPoolCreateInfo::default().queue_family_index(queue.queue_family_index);

				let command_pool = unsafe { self.device.create_command_pool(&command_pool_create_info, None).expect("No command pool") };

				let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
					.command_pool(command_pool)
					.level(vk::CommandBufferLevel::PRIMARY)
					.command_buffer_count(1)
				;

				let command_buffers = unsafe { self.device.allocate_command_buffers(&command_buffer_allocate_info).expect("No command buffer") };

				let vk_command_buffer = command_buffers[0];

				// self.set_name(vk_command_buffer, name);

				command_buffer.frames.push(CommandBufferInternal { vk_queue: queue.vk_queue, command_pool, command_buffer: vk_command_buffer, });
			}

		} else {
			unimplemented!()
		}

		self.frames = target_frames;
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
			vertex_count,
			index_count,
			vertex_size: vertex_layout.size(),
		});

		mesh_handle
	}

	/// Creates a shader.
	fn create_shader(&mut self, name: Option<&str>, shader_source_type: graphics_hardware_interface::ShaderSource, stage: graphics_hardware_interface::ShaderTypes, shader_binding_descriptors: &[graphics_hardware_interface::ShaderBindingDescriptor],) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
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
		let bindings = bindings.iter().map(|binding| {
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

			assert_ne!(binding.descriptor_count, 0, "Descriptor count must be greater than 0.");

			let _ = if let Some(inmutable_samplers) = &binding.immutable_samplers {
				inmutable_samplers.iter().map(|sampler| vk::Sampler::from_raw(sampler.0)).collect::<Vec<_>>()
			} else {
				Vec::new()
			};

			b
		}).collect::<Vec<_>>();

		let binding_flags = bindings.iter().map(|binding| {
			if binding.descriptor_count > 1 {
				vk::DescriptorBindingFlags::PARTIALLY_BOUND
			} else {
				vk::DescriptorBindingFlags::empty()
			}
		}).collect::<Vec<_>>();

		let mut dslbfci = vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);

		let descriptor_set_layout_create_info = vk::DescriptorSetLayoutCreateInfo::default().push_next(&mut dslbfci).bindings(&bindings);

		let descriptor_set_layout = unsafe { self.device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None).expect("No descriptor set layout") };

		self.set_name(descriptor_set_layout, name);

		let handle = graphics_hardware_interface::DescriptorSetTemplateHandle(self.descriptor_sets_layouts.len() as u64);

		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings: bindings.iter().map(|binding| {
				(binding.descriptor_type, binding.descriptor_count)
			}).collect::<Vec<_>>(),
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

		let offset = constructor.frame_offset.unwrap_or(0) as isize;

		let descriptor_set_handles = DescriptorSetHandle(descriptor_set.0).get_all(&self.descriptor_sets);

		let mut next = None;

		for (i, &descriptor_set_handle) in descriptor_set_handles.iter().enumerate() {
			let binding_handle = DescriptorSetBindingHandle(self.bindings.len() as u64);

			let created_binding = Binding {
				next,
				descriptor_set_handle,
				descriptor_type,
				count: binding.descriptor_count,
				index: binding.binding,
			};

			self.bindings.push(created_binding);

			next = Some(binding_handle);

			let index = i as isize + offset;

			let descriptor = match constructor.descriptor {
				crate::Descriptor::Buffer { handle, size } => {
					let handles = BufferHandle(handle.0).get_all(&self.buffers);

					Descriptors::Buffer { handle: handles[index.rem_euclid(handles.len() as isize) as usize], size }
				}
				crate::Descriptor::Image { handle, layout } => {
					let handles = ImageHandle(handle.0).get_all(&self.images);

					Descriptors::Image { handle: handles[index.rem_euclid(handles.len() as isize) as usize], layout }
				}
				crate::Descriptor::CombinedImageSampler { image_handle, sampler_handle, layout, layer } => {
					let image_handles = ImageHandle(image_handle.0).get_all(&self.images);
					let sampler_handles = vec![SamplerHandle(sampler_handle.0); 3];

					Descriptors::CombinedImageSampler {
						image_handle: image_handles[index.rem_euclid(image_handles.len() as isize) as usize],
						sampler_handle: sampler_handles[index.rem_euclid(sampler_handles.len() as isize) as usize],
						layout,
						layer,
					}
				}
				crate::Descriptor::Sampler(e) => {
					let sampler_handles = vec![SamplerHandle(e.0); 3];

					Descriptors::Sampler { handle: sampler_handles[index.rem_euclid(sampler_handles.len() as isize) as usize] }
				}
				crate::Descriptor::CombinedImageSamplerArray => {
					Descriptors::CombinedImageSamplerArray {}
				},
				_ => panic!("Unsupported descriptor type")
			};

			self.tasks.push_back(Task::write_descriptor(binding_handle, descriptor, Some(i as u8)));
		}

		graphics_hardware_interface::DescriptorSetBindingHandle(next.expect("No next binding").0)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>, descriptor_set_layout_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle) -> graphics_hardware_interface::DescriptorSetHandle {
		let pool_sizes = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize].bindings.iter().map(|(descriptor_type, descriptor_count)| {
			vk::DescriptorPoolSize::default().ty(*descriptor_type).descriptor_count(descriptor_count * self.frames as u32)
		}).collect::<Vec<_>>();

		let descriptor_pool_create_info = vk::DescriptorPoolCreateInfo::default()
			.max_sets(MAX_FRAMES_IN_FLIGHT as _)
			.pool_sizes(&pool_sizes)
		;

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
		let writes = descriptor_set_writes.iter().filter_map(|descriptor_set_write| {
			let binding_handles = DescriptorSetBindingHandle(descriptor_set_write.binding_handle.0).get_all(&self.bindings);

			// assert!(descriptor_set_write.array_element < binding.count, "Binding index out of range.");

			match descriptor_set_write.descriptor {
				graphics_hardware_interface::Descriptor::Buffer { handle, size } => {
					let buffer_handles = BufferHandle(handle.0).get_all(&self.buffers);

					let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

					for (i, &binding_handle) in binding_handles.iter().enumerate() {
						let offset = descriptor_set_write.frame_offset.unwrap_or(0);

						let buffer_handle = buffer_handles[((i as i32 - offset) % buffer_handles.len() as i32) as usize];

						writes.push(DescriptorWrite::new(Descriptors::Buffer { handle: buffer_handle, size: size }, binding_handle));
					}

					Some(writes)
				},
				graphics_hardware_interface::Descriptor::Image{ handle, layout } => {
					let image_handles = ImageHandle(handle.0).get_all(&self.images);
					let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

					for (i, &binding_handle) in binding_handles.iter().enumerate() {
						let offset = descriptor_set_write.frame_offset.unwrap_or(0);

						let image_handle = image_handles[((i as i32 - offset) % image_handles.len() as i32) as usize];

						writes.push(DescriptorWrite::new(Descriptors::Image { handle: image_handle, layout }, binding_handle));
					}

					Some(writes)
				},
				graphics_hardware_interface::Descriptor::CombinedImageSampler{ image_handle, sampler_handle, layout, layer } => {
					let image_handles = ImageHandle(image_handle.0).get_all(&self.images);
					let sampler_handle = SamplerHandle(sampler_handle.0);

					let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

					for (i, &binding_handle) in binding_handles.iter().enumerate() {
						let offset = descriptor_set_write.frame_offset.unwrap_or(0);

						let image_handle = image_handles[((i as i32 - offset) % image_handles.len() as i32) as usize];

						writes.push(DescriptorWrite::new(Descriptors::CombinedImageSampler { image_handle, layout, sampler_handle, layer }, binding_handle));
					}

					Some(writes)
				},
				graphics_hardware_interface::Descriptor::Sampler(handle) => {
					let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

					let sampler_handle = SamplerHandle(handle.0);

					for (_, &binding_handle) in binding_handles.iter().enumerate() {

						writes.push(DescriptorWrite::new(Descriptors::Sampler { handle: sampler_handle }, binding_handle));
					}

					Some(writes)
				},
				_ => unimplemented!(),
			}
		}).flatten().collect::<Vec<_>>();

		self.write_internal(&writes);
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

	fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_pipeline(builder)
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
			resource_access,
		});

		handle
	}

	fn create_command_buffer(&mut self, name: Option<&str>, queue_handle: graphics_hardware_interface::QueueHandle) -> graphics_hardware_interface::CommandBufferHandle {
		let command_buffer_handle = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

		let queue = &self.queues[queue_handle.0 as usize];

		let command_buffers = (0..self.frames).map(|_| {
			let _ = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

			let command_pool_create_info = vk::CommandPoolCreateInfo::default().queue_family_index(queue.queue_family_index);

			let command_pool = unsafe { self.device.create_command_pool(&command_pool_create_info, None).expect("No command pool") };

			let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
				.command_pool(command_pool)
				.level(vk::CommandBufferLevel::PRIMARY)
				.command_buffer_count(1)
			;

			let command_buffers = unsafe { self.device.allocate_command_buffers(&command_buffer_allocate_info).expect("No command buffer") };

			let command_buffer = command_buffers[0];

			self.set_name(command_buffer, name);

			CommandBufferInternal { vk_queue: queue.vk_queue, command_pool, command_buffer, }
		}).collect::<Vec<_>>();

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			frames: command_buffers,
		});

		command_buffer_handle
	}

	fn create_command_buffer_recording(&mut self, command_buffer_handle: graphics_hardware_interface::CommandBufferHandle, frame_key: Option<FrameKey>) -> crate::CommandBufferRecording {
		use graphics_hardware_interface::CommandBufferRecordable;

		let mut pending_buffers = self.pending_buffer_syncs.lock();

		let buffer_copies = pending_buffers.drain(..).map(|e| {
			let dst_buffer_handle = e;

			let dst_buffer = &self.buffers[dst_buffer_handle.0 as usize];

			let src_buffer_handle = dst_buffer.staging.unwrap();

			BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size)
		}).collect();

		drop(pending_buffers);

		let mut pending_images = self.pending_image_syncs.lock();

		let image_copies = pending_images.drain(..).map(|e| {
			let dst_image_handle = e;

			let dst_image = &self.images[dst_image_handle.0 as usize];

			ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
		}).collect();

		drop(pending_images);

		self.process_tasks(frame_key.map(|e| e.sequence_index).unwrap_or(0));

		let mut recording = CommandBufferRecording::new(self, command_buffer_handle, buffer_copies, image_copies, frame_key);

		recording.begin();

		recording
	}

	fn create_buffer<T: Copy>(&mut self, name: Option<&str>, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses,) -> graphics_hardware_interface::BufferHandle<T> {
		let mut uses = uses_to_vk_usage_flags(resource_uses);
		let size = std::mem::size_of::<T>();

		if !self.settings.ray_tracing {
			// Remove acc struct build flag
			uses &= !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
		}

		let buffer_handle = graphics_hardware_interface::BufferHandle::<T>(self.buffers.len() as u64, std::marker::PhantomData::<T>{});

		let handle = self.create_buffer_internal(name, resource_uses, size, uses | vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::GpuWrite);

		let staging_buffer_handle_option = if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) {
			let buffer_handle = self.create_buffer_internal(name, resource_uses, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::CpuWrite);

			Some(buffer_handle)
		} else {
			None
		};

		if let Some(staging_buffer_handle) = staging_buffer_handle_option {
			self.buffers[handle.0 as usize].staging = Some(staging_buffer_handle);
		}

		return buffer_handle;
	}

	fn create_dynamic_buffer<T: Copy>(&mut self, name: Option<&str>, resource_uses: crate::Uses, device_accesses: crate::DeviceAccesses) -> crate::DynamicBufferHandle<T> {
		let buffer_count = self.frames;

		let mut uses = uses_to_vk_usage_flags(resource_uses);
		let size = std::mem::size_of::<T>();

		if !self.settings.ray_tracing {
			// Remove acc struct build flag
			uses &= !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
		}

		let buffer_handle = graphics_hardware_interface::DynamicBufferHandle::<T>(self.buffers.len() as u64, std::marker::PhantomData::<T>{});

		let mut previous: Option<BufferHandle> = None;

		for _ in 0..buffer_count {
			let handle = self.create_buffer_internal(name, resource_uses, size, uses | vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::GpuWrite);

			let staging_buffer_handle_option = if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) {
				let buffer_handle = self.create_buffer_internal(name, resource_uses, size, vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS, graphics_hardware_interface::DeviceAccesses::CpuWrite);

				Some(buffer_handle)
			} else {
				None
			};

			if let Some(staging_buffer_handle) = staging_buffer_handle_option {
				self.buffers[handle.0 as usize].staging = Some(staging_buffer_handle);
			}

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

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe {
			std::mem::transmute(buffer.pointer)
		}
	}

	fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'a mut T {
		let handle = BufferHandle(buffer_handle.0);

		let buffer = self.buffers[handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];

		self.pending_buffer_syncs.lock().push_back(handle);

		unsafe {
			std::mem::transmute(buffer.pointer)
		}
	}

	fn get_mut_dynamic_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: crate::DynamicBufferHandle<T>, frame_key: FrameKey) -> &'a mut T {
		let handles = BufferHandle(buffer_handle.0).get_all(&self.buffers);

		let handle = handles[frame_key.sequence_index as usize];

		self.pending_buffer_syncs.lock().push_back(handle);

		let buffer = handle.access(&self.buffers);
		let buffer = buffer.staging.unwrap().access(&self.buffers);

		unsafe {
			std::mem::transmute(buffer.pointer)
		}
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let texture = &self.images[texture_handle.0 as usize];
		let buffer  = &self.buffers[texture.staging_buffer.unwrap().0 as usize];

		unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, texture.size)
		}
	}

	fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handles = ImageHandle(image_handle.0).get_all(&self.images);

		let handle = handles[0];

		let texture = handle.access(&self.images);
		let buffer = texture.staging_buffer.unwrap().access(&self.buffers);

		let slice = unsafe {
			std::slice::from_raw_parts_mut(buffer.pointer, texture.size)
		};

		f(slice);

		self.pending_image_syncs.lock().push_back(handle);
	}

	fn create_image(&mut self, name: Option<&str>, extent: Extent, format: graphics_hardware_interface::Formats, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses, use_case: graphics_hardware_interface::UseCases, array_layers: u32) -> graphics_hardware_interface::ImageHandle {
		let size = (extent.width() * extent.height() * extent.depth()) as usize * format.size();

		let mut next: Option<ImageHandle> = None;

		let extent = vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth());

		for _ in 0..(match use_case { graphics_hardware_interface::UseCases::DYNAMIC => { self.frames } graphics_hardware_interface::UseCases::STATIC => { 1 }}) {
			let resource_uses = resource_uses | if device_accesses.contains(graphics_hardware_interface::DeviceAccesses::CpuWrite) { graphics_hardware_interface::Uses::TransferDestination } else { graphics_hardware_interface::Uses::empty() };

			let texture_handle = if extent.width != 0 && extent.height != 0 && extent.depth != 0 {
				self.create_image_internal(next, name, format, device_accesses, array_layers, size, extent, resource_uses,)
			} else {
				let texture_handle = ImageHandle(self.images.len() as u64);

				self.images.push(Image {
					next,
					size: 0,
					staging_buffer: None,
					image: vk::Image::null(),
					image_view: vk::ImageView::null(),
					image_views: [vk::ImageView::null(); 8],
					extent,
					access: device_accesses,
					format: to_format(format),
					format_: format,
					uses: resource_uses,
					layers: array_layers,
				});

				texture_handle
			};

			next = Some(texture_handle);
		}

		let handle = graphics_hardware_interface::ImageHandle(next.unwrap().0);

		#[cfg(debug_assertions)] {
			if let Some(name) = name {
				self.names.insert(graphics_hardware_interface::Handle::Image(handle), name.to_string());
			}
		}

		handle
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
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
		let _ = size_info.build_scratch_size as usize;

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
		let _ = size_info.build_scratch_size as usize;

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

		let buffer = self.buffers[sbt_buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];

		(unsafe { std::slice::from_raw_parts_mut(buffer.pointer, buffer.size) })[sbt_record_offset..sbt_record_offset + 32].copy_from_slice(shader_handles.get(&shader_handle).unwrap());
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::ImageHandle, extent: Extent) {
		let image_handles = ImageHandle(image_handle.0).get_all(&self.images);

		for (index, handle) in image_handles.iter().enumerate() {
			#[cfg(debug_assertions)]
			let Image { format_, staging_buffer, .. } = self.images[handle.0 as usize];

			let size = (extent.width() * extent.height() * extent.depth()) as usize * format_.size();

			if let Some(staging_buffer_handle) = staging_buffer {
				self.resize_buffer_internal(staging_buffer_handle, size);
			}

			let Image { image: vk_image, image_view, format_, uses, layers, .. } = self.images[handle.0 as usize];

			self.tasks.push_back(Task::delete_vulkan_image_view(image_view, index as u8));
			self.tasks.push_back(Task::delete_vulkan_image(vk_image, index as u8));
			self.tasks.push_back(Task::update_image_descriptor(*handle, Some(index as u8)));

			// TODO: release memory/allocation

			#[cfg(debug_assertions)]
			let name = self.names.get(&graphics_hardware_interface::Handle::Image(image_handle)).as_ref().map(|s| s.as_str());

			#[cfg(not(debug_assertions))]
			let name = None;

			let r = self.create_vulkan_texture(name, vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth()), format_, uses | graphics_hardware_interface::Uses::TransferSource, 1, layers);

			let (allocation_handle, _) = self.create_allocation_internal(r.size, r.memory_flags.into(), graphics_hardware_interface::DeviceAccesses::GpuWrite | graphics_hardware_interface::DeviceAccesses::GpuRead);

			let (_, _) = self.bind_vulkan_texture_memory(&r, allocation_handle, 0);

			let image_view = self.create_vulkan_image_view(None, &r.resource, format_, 0, 0, layers);

			let image = &mut self.images[handle.0 as usize];
			image.size = size;
			image.extent = vk::Extent3D::default().width(extent.width()).height(extent.height()).depth(extent.depth());
			image.image_view = image_view;
			image.image = r.resource;

			if let Some(state) = self.states.get_mut(&Handle::Image(*handle)) {
				state.layout = vk::ImageLayout::UNDEFINED;
			}
		}
	}

	fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let buffer_handle = BufferHandle(buffer_handle.0);

		self.resize_buffer_internal(buffer_handle, size);
	}

	fn bind_to_window(&mut self, window_os_handles: &window::OSHandles, presentation_mode: graphics_hardware_interface::PresentationModes, fallback_extent: Extent) -> graphics_hardware_interface::SwapchainHandle {
		let surface = self.create_vulkan_surface(window_os_handles);

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, surface).expect("No surface capabilities") };

		let min_image_count = surface_capabilities.min_image_count;

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

		let presentation_modes = [presentation_mode];

		let mut present_modes_create_info = vk::SwapchainPresentModesCreateInfoEXT::default()
    		.present_modes(&presentation_modes)
		;

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
    		.push_next(&mut present_modes_create_info)
			.flags(vk::SwapchainCreateFlagsKHR::DEFERRED_MEMORY_ALLOCATION_EXT)
			.surface(surface)
			.min_image_count(min_image_count)
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

		let mut acquire_synchronizers = [SynchronizerHandle(!0u64); MAX_FRAMES_IN_FLIGHT];

		for i in 0..self.frames {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Acquire Sync"), true);
			acquire_synchronizers[i as usize] = synchronizer;
		}

		let mut submit_synchronizers = [SynchronizerHandle(!0u64); 8];

		for i in 0..min_image_count {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Submit Sync"), true);
			submit_synchronizers[i as usize] = synchronizer;
		}

		self.swapchains.push(Swapchain {
			surface,
			swapchain,
			acquire_synchronizers,
			submit_synchronizers,
			extent,
			sync_stage: vk::PipelineStageFlags2::TRANSFER,
		});

		swapchain_handle
	}

	fn get_image_data(&self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &[u8] {
		let image = &self.images[texture_copy_handle.0 as usize];
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
				let synchronizer_handle = self.create_synchronizer_internal(name, signaled);

				if let Some(pr) = previous {
					self.synchronizers[pr.0 as usize].next = Some(synchronizer_handle);
				}

				previous = Some(synchronizer_handle);
			}
		}

		synchronizer_handle
	}

	fn start_frame(&mut self, index: u32, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> FrameKey {
		let frame_index = index;
		let sequence_index = (index % self.frames as u32) as u8;

		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = &self.synchronizers[synchronizer_handles[sequence_index as usize].0 as usize];

		let per_cycle_wait_ms = 1;
		let wait_warning_time_threshold = 8;
		let mut timeout_count = 0;

		loop {
			match unsafe { self.device.wait_for_fences(&[synchronizer.fence], true, per_cycle_wait_ms * 1000000) } {
				Ok(_) => break,
				Err(vk::Result::TIMEOUT) => {
					if timeout_count * per_cycle_wait_ms >= wait_warning_time_threshold && timeout_count % 500 == 0 {
						println!("Stuck waiting for fences for {} ms at frame {index}. There is a potential issue with synchronization.", per_cycle_wait_ms * timeout_count);
					}
					timeout_count += 1;
					continue;
				},
				Err(_) => panic!("Failed to wait for fence"),
			}
		}

		unsafe { self.device.reset_fences(&[synchronizer.fence]).expect("No fence reset"); }

		// self.process_tasks(sequence_index);

		FrameKey { frame_index, sequence_index }
	}

	fn acquire_swapchain_image(&mut self, frame_key: FrameKey, swapchain_handle: graphics_hardware_interface::SwapchainHandle,) -> (graphics_hardware_interface::PresentKey, Extent) {
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];

		let swapchain_frame_synchronizer = swapchain.acquire_synchronizers[frame_key.sequence_index as usize].access(&self.synchronizers);

		let semaphore = swapchain_frame_synchronizer.semaphore;

		let acquisition_result = unsafe { self.swapchain.acquire_next_image(swapchain.swapchain, 0, semaphore, vk::Fence::default()) };

		let (index, _) = if let Ok((index, is_suboptimal)) = acquisition_result {
			if !is_suboptimal {
				(index, graphics_hardware_interface::SwapchainStates::Ok)
			} else {
				(index, graphics_hardware_interface::SwapchainStates::Suboptimal)
			}
		} else {
			(0, graphics_hardware_interface::SwapchainStates::Invalid)
		};

		let surface_capabilities = unsafe { self.surface.get_physical_device_surface_capabilities(self.physical_device, swapchain.surface).expect("No surface capabilities") };

		let extent = if surface_capabilities.current_extent.width != u32::MAX && surface_capabilities.current_extent.height != u32::MAX {
			Extent::rectangle(surface_capabilities.current_extent.width, surface_capabilities.current_extent.height)
		} else {
			Extent::rectangle(swapchain.extent.width, swapchain.extent.height)
		};

		(graphics_hardware_interface::PresentKey {
			image_index: index as u8,
			sequence_index: frame_key.sequence_index,
			swapchain: swapchain_handle,
		}, extent)
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

	fn wait(&self) {
    	unsafe { self.device.device_wait_idle().unwrap(); }
	}
}

struct StableVec<T: Default, const N: usize> {
	data: [T; N],
	pos: usize,
}

impl <T: Default + Copy, const N: usize> StableVec<T, N> {
	pub fn new() -> Self {
		StableVec {
			data: [T::default(); N],
			pos: 0,
		}
	}

	pub fn push(&mut self, value: T) -> &'static T {
		assert!(self.pos < N, "StableVec is full");
		let pos = self.pos;
		self.data[pos] = value;
		self.pos += 1;
		unsafe { std::mem::transmute(&self.data[pos]) } // SAFETY: this is not correct
	}

	pub fn push_mut(&mut self, value: T) -> &'static mut T {
		assert!(self.pos < N, "StableVec is full");
		let pos = self.pos;
		self.data[pos] = value;
		self.pos += 1;
		unsafe { std::mem::transmute(&mut self.data[pos]) } // SAFETY: this is not correct
	}

	pub fn append<const M: usize>(&mut self, array: [T; M]) -> &'static [T] {
		assert!(self.pos + M <= N, "StableVec is full");
		let start = self.pos;
		let end = start + M;
		self.data[start..end].copy_from_slice(&array);
		self.pos += M;
		unsafe { std::mem::transmute(&self.data[start..end]) } // SAFETY: this is not correct
	}
}
