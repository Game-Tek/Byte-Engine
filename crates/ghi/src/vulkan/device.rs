use std::{borrow::Cow, cell::RefCell};

use ash::vk::{self, Handle as _};
use utils::hash::{HashSet, HashSetExt};
use ::utils::{hash::{HashMap, HashMapExt}, Extent};
use crate::{graphics_hardware_interface, image, raster_pipeline, render_debugger::RenderDebugger, sampler, vulkan::{Descriptor, DescriptorSetBindingHandle, Handle, ImageHandle}, window, CommandBufferRecording, FrameKey, Size};

use super::{utils::{image_type_from_extent, into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format, to_pipeline_stage_flags, to_shader_stage_flags, uses_to_vk_usage_flags}, AccelerationStructure, Allocation, Binding, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DebugCallbackData, DescriptorSet, DescriptorSetHandle, DescriptorSetLayout, Image, MemoryBackedResourceCreationResult, Mesh, Pipeline, PipelineLayout, Shader, Swapchain, Synchronizer, SynchronizerHandle, TransitionState, MAX_FRAMES_IN_FLIGHT};

pub struct Device {
	entry: ash::Entry,
	instance: ash::Instance,

	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,
	debug_utils_messenger: Option<vk::DebugUtilsMessengerEXT>,
	debug_data: Box<DebugCallbackData>,

	physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	queue_family_index: u32,
	pub(super) queue: vk::Queue,
	pub(super) swapchain: ash::khr::swapchain::Device,
	surface: ash::khr::surface::Instance,
	pub(super) acceleration_structure: ash::khr::acceleration_structure::Device,
	pub(super) ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device,
	pub(super) mesh_shading: ash::ext::mesh_shader::Device,

	#[cfg(debug_assertions)]
	debugger: RenderDebugger,

	pub(super) frames: u8,

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

	pub(super) buffer_writes_queue: RefCell<HashMap<graphics_hardware_interface::BaseBufferHandle, u32>>,

	pub(super) pending_images: Vec<graphics_hardware_interface::ImageHandle>,
}

impl Device {
	pub fn new(settings: graphics_hardware_interface::Features) -> Result<Device, &'static str> {
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

		let instance = unsafe { entry.create_instance(&instance_create_info, None).or(Err("Failed to create instance"))? };

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

			let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).or(Err("Failed to enable debug utils messanger"))? };

			Some(debug_utils_messenger)
		} else {
			None
		};

		let physical_devices = unsafe { instance.enumerate_physical_devices().or(Err("Failed to enumerate physical devices"))? };

		let physical_device = if let Some(gpu_name) = settings.gpu {
			let physical_device = physical_devices.into_iter().find(|physical_device| {
				let properties = unsafe { instance.get_physical_device_properties(*physical_device) };

				let name = properties.device_name_as_c_str();

				name.unwrap().to_str().unwrap() == gpu_name
			}).ok_or("Failed to find physical device")?;

			#[cfg(debug_assertions)]
			{
				let properties = unsafe { instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		} else {
			let physical_device = physical_devices.into_iter().filter(|physical_device| {
				let mut tools = [vk::PhysicalDeviceToolProperties::default(); 8];

				let tool_count = unsafe {
					instance.get_physical_device_tool_properties_len(*physical_device).unwrap()
				};

				unsafe {
					instance.get_physical_device_tool_properties(*physical_device, &mut tools[0..tool_count])
				};

				let buffer_device_address_capture_replay = tools.iter().take(tool_count as usize).any(|tool| {
					let name = unsafe { std::ffi::CStr::from_ptr(tool.name.as_ptr()) };
					name.to_str().unwrap() == "RenderDoc"
				});

				let mut physical_device_vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default();
				let mut physical_device_features = vk::PhysicalDeviceFeatures2::default().push_next(&mut physical_device_vulkan_12_features);
	
				unsafe { instance.get_physical_device_features2(*physical_device, &mut physical_device_features) };
	
				let features = physical_device_features.features;
	
				if features.sample_rate_shading == vk::FALSE { return false; }
				if physical_device_vulkan_12_features.buffer_device_address_capture_replay != buffer_device_address_capture_replay as vk::Bool32 { return false; }

				features.shader_storage_image_array_dynamic_indexing != vk::FALSE &&
				features.shader_sampled_image_array_dynamic_indexing != vk::FALSE &&
				features.shader_storage_buffer_array_dynamic_indexing != vk::FALSE &&
				features.shader_uniform_buffer_array_dynamic_indexing != vk::FALSE &&
				features.shader_storage_image_write_without_format != vk::FALSE &&
				features.geometry_shader == settings.geometry_shader as vk::Bool32
			}).max_by_key(|physical_device| {
				let properties = unsafe { instance.get_physical_device_properties(*physical_device) };
	
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
				let properties = unsafe { instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		};

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

		let available_device_extensions = unsafe { instance.enumerate_device_extension_properties(physical_device) }.expect("Could not get supported device extensions");

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
			.geometry_shader(settings.geometry_shader)
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

		device_extension_names.push(ash::ext::shader_atomic_float::NAME.as_ptr());

		let device_create_info = vk::DeviceCreateInfo::default();

		let device_create_info = if is_device_extension_available(ash::ext::mesh_shader::NAME.to_str().unwrap().as_str()) {
			device_extension_names.push(ash::ext::mesh_shader::NAME.as_ptr());
			device_create_info.push_next(&mut physical_device_mesh_shading_features)
		} else {
			return Err("Mesh shader extension not available");
		};

		let device_create_info = device_create_info
			.push_next(&mut physical_device_vulkan_11_features)
			.push_next(&mut physical_device_vulkan_12_features)
			.push_next(&mut physical_device_vulkan_13_features)
			// .push_next(&mut shader_atomic_float_features)
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

		let device: ash::Device = unsafe { instance.create_device(physical_device, &device_create_info, None).map_err(|e| {
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

		Ok(Device {
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

			buffer_writes_queue: RefCell::new(HashMap::with_capacity(128)),
		})
	}

	#[cfg(debug_assertions)]
	fn get_log_count(&self) -> u64 { self.debug_data.error_count }

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

		let max_binding = builder.vertex_elements.iter().map(|ve| ve.binding).max().unwrap() + 1;

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
				shaders: Vec::new(),
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

impl graphics_hardware_interface::Device for Device {
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
		fn m(rs: &mut Device, bindings: &[graphics_hardware_interface::DescriptorSetBindingTemplate], layout_bindings: &mut Vec<vk::DescriptorSetLayoutBinding>, map: &mut Vec<(vk::DescriptorType, u32)>) -> vk::DescriptorSetLayout {
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

	fn create_command_buffer_recording(&mut self, command_buffer_handle: graphics_hardware_interface::CommandBufferHandle, frame_key: Option<FrameKey>) -> crate::CommandBufferRecording {
		use graphics_hardware_interface::CommandBufferRecordable;
		let pending_images = self.pending_images.clone();
		self.pending_images.clear();
		let mut recording = CommandBufferRecording::new(self, command_buffer_handle, frame_key);
		recording.begin();
		recording.transfer_textures(&pending_images);
		recording
	}

	fn create_buffer<T: Copy>(&mut self, name: Option<&str>, resource_uses: graphics_hardware_interface::Uses, device_accesses: graphics_hardware_interface::DeviceAccesses, use_case: graphics_hardware_interface::UseCases) -> graphics_hardware_interface::BufferHandle<T> {
		let buffer_count = match use_case {
			graphics_hardware_interface::UseCases::STATIC => 1,
			graphics_hardware_interface::UseCases::DYNAMIC => self.frames,
		};

		let mut uses = uses_to_vk_usage_flags(resource_uses);
		let size = std::mem::size_of::<T>();

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

		let buffer_handle = graphics_hardware_interface::BufferHandle::<T>(self.buffers.len() as u64, std::marker::PhantomData::<T>{});

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

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe {
			std::mem::transmute(buffer.pointer)
		}
	}

	fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'a mut T {
		let mut buffer_writes = self.buffer_writes_queue.borrow_mut();
		let mut entry = buffer_writes.entry(buffer_handle.into()).insert_entry(0);
		*entry.get_mut() = 0;

		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe {
			std::mem::transmute(buffer.pointer)
		}
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

		let mut buffer_writes = self.buffer_writes_queue.borrow_mut();
		let mut entry = buffer_writes.entry(sbt_buffer_handle.into()).insert_entry(0);

		*entry.get_mut() = 0;

		let buffer = self.buffers[sbt_buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];

		(unsafe { std::slice::from_raw_parts_mut(buffer.pointer, buffer.size) })[sbt_record_offset..sbt_record_offset + 32].copy_from_slice(shader_handles.get(&shader_handle).unwrap());
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

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
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

		let mut semaphores = [vk::Semaphore::null(); MAX_FRAMES_IN_FLIGHT];

		for i in 0..self.frames as usize {
			semaphores[i] = unsafe { self.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).expect("No semaphore") };
			self.set_name(semaphores[i], format!("Swapchain semaphore ({i})").as_str().into());
		}

		self.swapchains.push(Swapchain {
			surface,
			surface_present_mode: presentation_mode,
			swapchain,
			semaphores,
			extent,
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

	fn start_frame(&self, index: u32) -> FrameKey {
		FrameKey { frame_index: index, sequence_index: (index % self.frames as u32) as u8 }
	}

	fn acquire_swapchain_image(&mut self, frame_key: FrameKey, swapchain_handle: graphics_hardware_interface::SwapchainHandle,) -> (graphics_hardware_interface::PresentKey, Extent) {
		let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];

		let semaphore = swapchain.semaphores[frame_key.sequence_index as usize];

		let timeout = if false {
			std::time::Duration::from_secs(5).as_micros() as u64
		} else {
			u64::MAX
		};

		let acquisition_result = unsafe { self.swapchain.acquire_next_image(swapchain.swapchain, timeout, semaphore, vk::Fence::null()) };

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

		// if swapchain_state == graphics_hardware_interface::SwapchainStates::Suboptimal || swapchain_state == graphics_hardware_interface::SwapchainStates::Invalid {

		// 	unsafe { // TODO: consider deadlock https://vulkan-tutorial.com/Drawing_a_triangle/Swap_chain_recreation
		// 		self.device.device_wait_idle().unwrap();

		// 		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
		// 			.surface(swapchain.surface)
		// 			.min_image_count(surface_capabilities.min_image_count)
		// 			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
		// 			.image_format(vk::Format::B8G8R8A8_SRGB)
		// 			.image_extent(vk::Extent2D::default().width(1920).height(1080))
		// 			.image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
		// 			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
		// 			.pre_transform(surface_capabilities.current_transform)
		// 			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
		// 			.present_mode(swapchain.surface_present_mode)
		// 			.image_array_layers(1)
		// 			.clipped(true)
		// 		;

		// 		let new_swapchain = self.swapchain.create_swapchain(&swapchain_create_info, None).expect("No swapchain");

		// 		self.swapchain.destroy_swapchain(swapchain.swapchain, None);

		// 		swapchain.swapchain = new_swapchain;
		// 	}
		// }

		let extent = if surface_capabilities.current_extent.width != u32::MAX && surface_capabilities.current_extent.height != u32::MAX {
			Extent::rectangle(surface_capabilities.current_extent.width, surface_capabilities.current_extent.height)
		} else {
			Extent::rectangle(swapchain.extent.width, swapchain.extent.height)
		};

		(graphics_hardware_interface::PresentKey{
			image_index: index as u8,
			sequence_index: frame_key.sequence_index,
			swapchain: swapchain_handle,
		}, extent)
	}

	fn wait(&self, frame_key: FrameKey, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = self.synchronizers[synchronizer_handles[frame_key.sequence_index as usize].0 as usize];
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