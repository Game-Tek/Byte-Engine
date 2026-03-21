use std::{borrow::Cow, num::NonZeroU32, u64};

use crate::{
	graphics_hardware_interface, image,
	render_debugger::RenderDebugger,
	sampler,
	utils::StableVec,
	vulkan::{
		queue::Queue, sampler::SamplerHandle, BufferCopy, BuildBuffer, Descriptor, DescriptorSetBindingHandle, DescriptorWrite,
		Descriptors, Frame, Handle, HandleLike, ImageCopy, ImageHandle, Task, Tasks, MAX_SWAPCHAIN_IMAGES,
	},
	vulkan::{CommandBufferRecording, Instance},
	window, FrameKey, Size,
};
use ash::vk::{self, Handle as _};
use smallvec::SmallVec;
use utils::hash::{HashSet, HashSetExt};
use utils::{
	hash::{HashMap, HashMapExt},
	Extent,
};

use super::{
	utils::{
		image_type_from_extent, into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format,
		to_shader_stage_flags, uses_to_vk_usage_flags,
	},
	AccelerationStructure, Allocation, Binding, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DebugCallbackData,
	DescriptorSet, DescriptorSetHandle, DescriptorSetLayout, Image, MemoryBackedResourceCreationResult, Mesh, Pipeline,
	PipelineLayout, PipelineLayoutKey, Shader, Swapchain, Synchronizer, SynchronizerHandle, TransitionState,
	MAX_FRAMES_IN_FLIGHT,
};

pub struct Device {
	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,

	debug_data: *const DebugCallbackData,

	pub(crate) physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	pub(super) swapchain: ash::khr::swapchain::Device,
	surface: ash::khr::surface::Instance,
	pub(super) acceleration_structure: ash::khr::acceleration_structure::Device,
	pub(super) ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device,
	pub(super) mesh_shading: ash::ext::mesh_shader::Device,
	pub(super) surface_capabilities: ash::khr::get_surface_capabilities2::Instance,

	#[cfg(target_os = "linux")]
	pub(super) wayland_surface: ash::khr::wayland_surface::Instance,

	#[cfg(target_os = "windows")]
	pub(super) win32_surface: ash::khr::win32_surface::Instance,

	#[cfg(target_os = "macos")]
	pub(super) macos_surface: ash::ext::metal_surface::Instance,

	#[cfg(debug_assertions)]
	debugger: RenderDebugger,

	pub(super) frames: u8,

	pub(super) queues: Vec<Queue>,
	pub(super) buffers: Vec<Buffer>,
	pub(super) images: Vec<Image>,
	pub(super) allocations: Vec<Allocation>,
	pub(super) descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	pub(super) pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
	pub(super) bindings: Vec<Binding>,
	pub(super) descriptor_pools: Vec<vk::DescriptorPool>,
	pub(super) descriptor_sets: Vec<DescriptorSet>,
	pub(super) meshes: Vec<Mesh>,
	pub(super) acceleration_structures: Vec<AccelerationStructure>,
	pub(super) shaders: Vec<Shader>,
	pub(super) pipelines: Vec<Pipeline>,
	pub(super) command_buffers: Vec<CommandBuffer>,
	pub(super) synchronizers: Vec<Synchronizer>,
	pub(super) swapchains: Vec<Swapchain>,

	/// Maps a resource to N descriptors that reference it.
	resource_to_descriptor: HashMap<Handle, HashSet<(DescriptorSetBindingHandle, u32)>>,

	pub(super) descriptors: HashMap<DescriptorSetHandle, HashMap<u32, HashMap<u32, Descriptor>>>,

	/// Maps a descriptor set and binding to N resources that it references.
	descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32), HashSet<Handle>>,

	pub settings: crate::device::Features,

	pub(super) states: HashMap<Handle, TransitionState>,

	/// Tracks pending buffer host to device, or device to host synchronization operations.
	pub(super) pending_buffer_syncs: HashSet<BufferHandle>,
	/// Tracks pending image host to device, or device to host synchronization operations.
	pub(super) pending_image_syncs: HashSet<ImageHandle>,

	/// Tracks all dynamic buffer master handles that use the persistent write mode.
	/// These buffers have their source buffer memcpy'd into the per-frame staging
	/// buffer every frame before GPU copies are issued.
	pub(super) persistent_write_dynamic_buffers: Vec<graphics_hardware_interface::BaseBufferHandle>,

	memory_properties: vk::PhysicalDeviceMemoryProperties,

	/// Stores the debug names for resources.
	/// Used when inspecting resources from a rendering debugger such as RenderDoc.
	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handle, String>,

	/// A queue of deferred tasks. Usually object deletions and resource updates.
	pub(crate) tasks: Vec<Task>,
}

impl Device {
	pub fn new(
		settings: crate::device::Features,
		instance: &Instance,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Device, &'static str> {
		let vk_entry = &instance.entry;
		let vk_instance = &instance.instance;

		#[cfg(target_os = "linux")]
		let wayland_surface = ash::khr::wayland_surface::Instance::new(vk_entry, vk_instance);

		#[cfg(target_os = "windows")]
		let win32_surface = ash::khr::win32_surface::Instance::new(vk_entry, vk_instance);

		#[cfg(target_os = "macos")]
		let macos_surface = ash::ext::metal_surface::Instance::new(vk_entry, vk_instance);

		let surface_capabilities = ash::khr::get_surface_capabilities2::Instance::new(vk_entry, vk_instance);

		let flag_required_or_available = |feature: vk::Bool32, required: bool| {
			if required {
				feature != 0
			} else {
				true
			}
		};

		let mut barycentric_required_features =
			vk::PhysicalDeviceFragmentShaderBarycentricFeaturesKHR::default().fragment_shader_barycentric(false);

		let mut physical_device_vulkan_11_required_features = vk::PhysicalDeviceVulkan11Features::default()
			.uniform_and_storage_buffer16_bit_access(true)
			.storage_buffer16_bit_access(true);

		let mut physical_device_vulkan_12_required_features = vk::PhysicalDeviceVulkan12Features::default()
			.descriptor_indexing(true)
			.descriptor_binding_partially_bound(true)
			.runtime_descriptor_array(true)
			.descriptor_binding_variable_descriptor_count(true)
			.shader_sampled_image_array_non_uniform_indexing(true)
			.shader_storage_image_array_non_uniform_indexing(true)
			.scalar_block_layout(true)
			.buffer_device_address(true)
			.separate_depth_stencil_layouts(true)
			.shader_float16(true)
			.shader_int8(true)
			.storage_buffer8_bit_access(true)
			.uniform_and_storage_buffer8_bit_access(true)
			.vulkan_memory_model(true)
			.vulkan_memory_model_device_scope(true)
			.timeline_semaphore(true);

		let mut physical_device_vulkan_13_required_features = vk::PhysicalDeviceVulkan13Features::default()
			.pipeline_creation_cache_control(true)
			.subgroup_size_control(true)
			.compute_full_subgroups(true)
			.synchronization2(true)
			.dynamic_rendering(true)
			.maintenance4(true);

		let enabled_physical_device_required_features = vk::PhysicalDeviceFeatures::default()
			.shader_int16(true)
			.shader_int64(true)
			.shader_uniform_buffer_array_dynamic_indexing(true)
			.shader_storage_buffer_array_dynamic_indexing(true)
			.shader_storage_image_array_dynamic_indexing(true)
			.shader_storage_image_write_without_format(true)
			.texture_compression_bc(true)
			.geometry_shader(settings.geometry_shader)
			.shader_storage_image_write_without_format(true);

		let mut shader_atomic_float_required_features =
			vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT::default().shader_buffer_float32_atomics(true);

		let mut physical_device_mesh_shading_required_features = vk::PhysicalDeviceMeshShaderFeaturesEXT::default()
			.task_shader(settings.mesh_shading)
			.mesh_shader(settings.mesh_shading);

		let physical_devices = unsafe {
			vk_instance
				.enumerate_physical_devices()
				.or(Err("Failed to enumerate physical devices"))?
		};

		let physical_device = if let Some(gpu_name) = settings.gpu {
			let physical_device = physical_devices
				.into_iter()
				.find(|physical_device| {
					let properties = unsafe { vk_instance.get_physical_device_properties(*physical_device) };

					let name = properties.device_name_as_c_str();

					name.unwrap().to_str().unwrap() == gpu_name
				})
				.ok_or("Failed to find physical device")?;

			#[cfg(debug_assertions)]
			{
				let _ = unsafe { vk_instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		} else {
			let physical_device = physical_devices
				.into_iter()
				.filter(|&physical_device| {
					let mut tools = [vk::PhysicalDeviceToolProperties::default(); 8];

					let tool_count = unsafe { vk_instance.get_physical_device_tool_properties_len(physical_device).unwrap() };

					unsafe {
						vk_instance
							.get_physical_device_tool_properties(physical_device, &mut tools[0..tool_count])
							.unwrap();
					};

					let mut vk_physical_device_memory_properties2 = vk::PhysicalDeviceMemoryProperties2::default();

					unsafe {
						vk_instance.get_physical_device_memory_properties2(
							physical_device,
							&mut vk_physical_device_memory_properties2,
						);
					}

					for heap in &vk_physical_device_memory_properties2.memory_properties.memory_heaps
						[..vk_physical_device_memory_properties2.memory_properties.memory_heap_count as usize]
					{
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
					let mut physical_device_barycentric_features =
						vk::PhysicalDeviceFragmentShaderBarycentricFeaturesKHR::default();
					let mut physical_device_features = vk::PhysicalDeviceFeatures2::default()
						.push_next(&mut physical_device_vulkan_12_features)
						.push_next(&mut physical_device_barycentric_features)
						.push_next(&mut physical_device_mesh_shading_features);

					unsafe { vk_instance.get_physical_device_features2(physical_device, &mut physical_device_features) };

					let features = physical_device_features.features;

					let feature_validation = [
						(features.sample_rate_shading != vk::FALSE, "Sample Rate Shading"),
						(
							flag_required_or_available(
								physical_device_vulkan_12_features.buffer_device_address_capture_replay,
								buffer_device_address_capture_replay,
							),
							"Buffer Device Address Capture Replay",
						),
						(
							flag_required_or_available(
								physical_device_barycentric_features.fragment_shader_barycentric,
								barycentric_required_features.fragment_shader_barycentric != 0,
							),
							"Fragment Shader Barycentric",
						),
						(
							features.shader_storage_image_array_dynamic_indexing != vk::FALSE,
							"Shader Storage Image Array Dynamic Indexing",
						),
						(
							features.shader_sampled_image_array_dynamic_indexing != vk::FALSE,
							"Shader Sampled Image Array Dynamic Indexing",
						),
						(
							features.shader_storage_buffer_array_dynamic_indexing != vk::FALSE,
							"Shader Storage Buffer Array Dynamic Indexing",
						),
						(
							features.shader_uniform_buffer_array_dynamic_indexing != vk::FALSE,
							"Shader Uniform Buffer Array Dynamic Indexing",
						),
						(
							features.shader_storage_image_write_without_format != vk::FALSE,
							"Shader Storage Image Write Without Format",
						),
						(
							flag_required_or_available(features.geometry_shader, settings.geometry_shader),
							"Geometry Shader",
						),
						(
							flag_required_or_available(
								physical_device_mesh_shading_features.mesh_shader,
								physical_device_mesh_shading_required_features.mesh_shader != 0,
							),
							"Mesh Shader",
						),
						(
							flag_required_or_available(
								physical_device_mesh_shading_features.task_shader,
								physical_device_mesh_shading_required_features.task_shader != 0,
							),
							"Task Shader",
						),
					];

					let all_features_available = feature_validation.iter().all(|(available, _)| *available);

					all_features_available
				})
				.max_by_key(|physical_device| {
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
				})
				.ok_or("Failed to choose a best physical device")?;

			#[cfg(debug_assertions)]
			{
				let _ = unsafe { vk_instance.get_physical_device_properties(physical_device) };
			}

			physical_device
		};

		let queue_family_properties = unsafe { vk_instance.get_physical_device_queue_family_properties(physical_device) };

		let queue_create_infos = queues
			.iter()
			.map(|(d, _)| {
				let mut queue_family_index = queue_family_properties
					.iter()
					.enumerate()
					.filter_map(|(index, info)| {
						let mask = match d.r#type {
							crate::command_buffer::CommandBufferType::COMPUTE => vk::QueueFlags::COMPUTE,
							crate::command_buffer::CommandBufferType::GRAPHICS => vk::QueueFlags::GRAPHICS,
							crate::command_buffer::CommandBufferType::TRANSFER => vk::QueueFlags::TRANSFER,
						};

						if info.queue_flags.contains(mask) {
							Some((index as u32, info.queue_flags.as_raw().count_ones()))
						} else {
							None
						}
					})
					.collect::<Vec<_>>();

				queue_family_index.sort_by(|(_, a_bit_count), (_, b_bit_count)| a_bit_count.cmp(b_bit_count));

				let least_bits_queue_family_index = queue_family_index.first().unwrap().0;

				vk::DeviceQueueCreateInfo::default()
					.queue_family_index(least_bits_queue_family_index)
					.queue_priorities(&[1.0])
			})
			.collect::<Vec<_>>();

		let memory_properties = unsafe { vk_instance.get_physical_device_memory_properties(physical_device) };

		let available_device_extensions = unsafe { vk_instance.enumerate_device_extension_properties(physical_device) }
			.expect("Could not get supported device extensions");

		let is_device_extension_available = |name: &str| {
			available_device_extensions.iter().any(|extension| unsafe {
				std::ffi::CStr::from_ptr(extension.extension_name.as_ptr()).to_str().unwrap() == name
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

		#[cfg(target_os = "macos")]
		{
			device_extension_names.push(ash::khr::portability_subset::NAME.as_ptr());
		}

		let (mut physical_device_acceleration_structure_features, mut physical_device_ray_tracing_pipeline_features) =
			if settings.ray_tracing {
				let physical_device_acceleration_structure_features =
					vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default().acceleration_structure(true);

				let physical_device_ray_tracing_pipeline_features = vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default()
					.ray_tracing_pipeline(true)
					.ray_traversal_primitive_culling(true);

				(
					physical_device_acceleration_structure_features,
					physical_device_ray_tracing_pipeline_features,
				)
			} else {
				(
					vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default(),
					vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default(),
				)
			};

		device_extension_names.push(ash::ext::shader_atomic_float::NAME.as_ptr());

		let device_create_info = vk::DeviceCreateInfo::default();

		let device_create_info = if settings.mesh_shading {
			if is_device_extension_available(ash::ext::mesh_shader::NAME.to_str().unwrap().as_str()) {
				device_extension_names.push(ash::ext::mesh_shader::NAME.as_ptr());
				device_create_info.push_next(&mut physical_device_mesh_shading_required_features)
			} else {
				return Err("Mesh shader extension not available");
			}
		} else {
			device_create_info
		};

		let mut swapchain_maintenance_features =
			vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default().swapchain_maintenance1(true);

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
			.enabled_features(&enabled_physical_device_required_features);

		let device_create_info = if settings.ray_tracing {
			device_create_info
				.push_next(&mut physical_device_acceleration_structure_features)
				.push_next(&mut physical_device_ray_tracing_pipeline_features)
		} else {
			device_create_info
		};

		let device: ash::Device = unsafe {
			vk_instance
				.create_device(physical_device, &device_create_info, None)
				.map_err(|e| match e {
					vk::Result::ERROR_OUT_OF_HOST_MEMORY => "Out of host memory",
					vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => "Out of device memory",
					vk::Result::ERROR_INITIALIZATION_FAILED => "Initialization failed",
					vk::Result::ERROR_EXTENSION_NOT_PRESENT => "Extension not present",
					vk::Result::ERROR_FEATURE_NOT_PRESENT => "Feature not present",
					vk::Result::ERROR_TOO_MANY_OBJECTS => "Too many objects",
					vk::Result::ERROR_DEVICE_LOST => "Device lost",
					_ => "Failed to create a device",
				})?
		};

		let queues = queues
			.iter_mut()
			.zip(queue_create_infos.iter())
			.enumerate()
			.map(|(index, ((_, queue_handle), create_info))| {
				let vk_queue = unsafe { device.get_device_queue(create_info.queue_family_index, 0) };

				**queue_handle = Some(graphics_hardware_interface::QueueHandle(index as u64));

				Queue {
					queue_family_index: create_info.queue_family_index,
					_queue_index: 0,
					vk_queue,
				}
			})
			.collect::<Vec<_>>();

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

		let frames = 2u8;

		Ok(Device {
			debug_utils,
			debug_data: instance.debug_data.as_ref() as *const DebugCallbackData,

			memory_properties,

			#[cfg(target_os = "linux")]
			wayland_surface,

			#[cfg(target_os = "windows")]
			win32_surface,

			#[cfg(target_os = "macos")]
			macos_surface,

			surface_capabilities,

			physical_device,
			device,
			swapchain,
			surface,
			acceleration_structure,
			ray_tracing_pipeline,
			mesh_shading,

			#[cfg(debug_assertions)]
			debugger: RenderDebugger::new(),

			frames, // Assuming double buffering

			queues,
			allocations: Vec::new(),
			buffers: Vec::with_capacity(1024),
			images: Vec::with_capacity(512),
			descriptor_sets_layouts: Vec::with_capacity(128),
			pipeline_layouts: Vec::with_capacity(64),
			pipeline_layout_indices: HashMap::with_capacity(64),
			bindings: Vec::with_capacity(1024),
			descriptor_pools: Vec::with_capacity(512),
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

			pending_buffer_syncs: HashSet::with_capacity(128),
			pending_image_syncs: HashSet::with_capacity(128),

			persistent_write_dynamic_buffers: Vec::with_capacity(64),

			tasks: Vec::with_capacity(1024),

			#[cfg(debug_assertions)]
			names: HashMap::with_capacity(4096),
		})
	}

	#[cfg(debug_assertions)]
	fn get_log_count(&self) -> u64 {
		use std::sync::atomic::Ordering;
		unsafe { &(*self.debug_data) }.error_count.load(Ordering::SeqCst)
	}

	pub(super) fn get_syncronizer_handles(
		&self,
		synchroizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> SmallVec<[SynchronizerHandle; MAX_FRAMES_IN_FLIGHT]> {
		SynchronizerHandle(synchroizer_handle.0).get_all(&self.synchronizers)
	}

	fn create_vulkan_graphics_pipeline_create_info<'a, R>(
		&'a mut self,
		builder: crate::pipelines::raster::Builder,
		after_build: impl FnOnce(&'a mut Self, crate::pipelines::raster::Builder, vk::GraphicsPipelineCreateInfo) -> R,
	) -> R {
		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
		;

		let pipeline_layout_handle = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

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
						.input_rate(vk::VertexInputRate::VERTEX),
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

		let stages = builder
			.shaders
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

		let pipeline_color_blend_attachments = builder
			.render_targets
			.iter()
			.filter(|a| a.format != crate::Formats::Depth32)
			.map(|attachment| {
				let blend_state =
					vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);

				match attachment.blend {
					crate::pipelines::raster::BlendMode::None => blend_state
						.blend_enable(false)
						.src_color_blend_factor(vk::BlendFactor::ONE)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ZERO)
						.dst_alpha_blend_factor(vk::BlendFactor::ZERO)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
					crate::pipelines::raster::BlendMode::Alpha => blend_state
						.blend_enable(true)
						.src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
				}
			})
			.collect::<Vec<_>>();

		let color_attachement_formats: Vec<vk::Format> = builder
			.render_targets
			.iter()
			.filter(|a| a.format != crate::Formats::Depth32)
			.map(|a| to_format(a.format))
			.collect::<Vec<_>>();

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
			.back(vk::StencilOpState::default());

		let pipeline_create_info = if let Some(_) = builder.render_targets.iter().find(|a| a.format == crate::Formats::Depth32)
		{
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
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

		let viewports = [vk::Viewport::default()
			.x(0.0)
			.y(9.0)
			.width(16.0)
			.height(9.0)
			.min_depth(0.0)
			.max_depth(1.0)];

		let scissors = [vk::Rect2D::default()
			.offset(vk::Offset2D { x: 0, y: 0 })
			.extent(vk::Extent2D { width: 16, height: 9 })];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors);

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
			.alpha_to_one_enable(false);

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info
			.viewport_state(&viewport_state)
			.dynamic_state(&dynamic_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state);

		after_build(self, builder, pipeline_create_info)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		descriptor_set_layout_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let key = PipelineLayoutKey {
			descriptor_set_templates: descriptor_set_layout_handles.to_vec(),
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		let push_constant_stages =
			vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE;

		let push_constant_stages = push_constant_stages
			| if self.settings.mesh_shading {
				vk::ShaderStageFlags::MESH_EXT
			} else {
				vk::ShaderStageFlags::empty()
			};

		let push_constant_ranges = push_constant_ranges
			.iter()
			.map(|push_constant_range| {
				vk::PushConstantRange::default()
					.size(push_constant_range.size)
					.offset(push_constant_range.offset)
					.stage_flags(push_constant_stages)
			})
			.collect::<Vec<_>>();
		let set_layouts = descriptor_set_layout_handles
			.iter()
			.map(|set_layout| self.descriptor_sets_layouts[set_layout.0 as usize].descriptor_set_layout)
			.collect::<Vec<_>>();

		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&set_layouts)
			.push_constant_ranges(&push_constant_ranges);

		let pipeline_layout = unsafe {
			self.device
				.create_pipeline_layout(&pipeline_layout_create_info, None)
				.expect("No pipeline layout")
		};

		let handle = graphics_hardware_interface::PipelineLayoutHandle(self.pipeline_layouts.len() as u64);

		self.pipeline_layouts.push(PipelineLayout {
			pipeline_layout,
			descriptor_set_template_indices: descriptor_set_layout_handles
				.iter()
				.enumerate()
				.map(|(i, handle)| (*handle, i as u32))
				.collect(),
		});
		self.pipeline_layout_indices.insert(key, handle);

		handle
	}

	fn create_vulkan_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_graphics_pipeline_create_info(builder, |this, builder, pipeline_create_info| {
			let pipeline_layout_handle = this.get_or_create_pipeline_layout(
				builder.descriptor_set_templates.as_ref(),
				builder.push_constant_ranges.as_ref(),
			);
			let pipeline_create_infos = [pipeline_create_info];

			let pipelines = unsafe {
				this.device
					.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None)
					.expect("No pipeline")
			};

			let pipeline = pipelines[0];

			let handle = graphics_hardware_interface::PipelineHandle(this.pipelines.len() as u64);

			let resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))> = builder
				.shaders
				.iter()
				.map(|s| {
					let shader = &this.shaders[s.handle.0 as usize];
					shader
						.shader_binding_descriptors
						.iter()
						.map(|sbd| ((sbd.set, sbd.binding), (Into::<crate::Stages>::into(s.stage), sbd.access)))
				})
				.flatten()
				.collect::<Vec<_>>();

			this.pipelines.push(Pipeline {
				pipeline,
				layout: pipeline_layout_handle,
				shader_handles: HashMap::new(),
				resource_access,
			});

			handle
		})
	}

	fn create_vulkan_buffer(
		&self,
		name: Option<&str>,
		size: usize,
		usage: vk::BufferUsageFlags,
	) -> MemoryBackedResourceCreationResult<vk::Buffer> {
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

	fn create_vulkan_texture(
		&self,
		name: Option<&str>,
		extent: vk::Extent3D,
		format: crate::Formats,
		resource_uses: crate::Uses,
		mip_levels: u32,
		array_layers: Option<NonZeroU32>,
	) -> MemoryBackedResourceCreationResult<vk::Image> {
		let image_create_info = vk::ImageCreateInfo::default()
			.image_type(image_type_from_extent(extent).expect("Failed to get VkImageType from extent"))
			.format(to_format(format))
			.extent(extent)
			.mip_levels(mip_levels)
			.array_layers(array_layers.map(|e| e.get()).unwrap_or(1))
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(vk::ImageTiling::OPTIMAL)
			.usage(into_vk_image_usage_flags(resource_uses, format))
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED);

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };

		let memory_requirements = unsafe { self.device.get_image_memory_requirements(image) };

		self.set_name(image, name);

		MemoryBackedResourceCreationResult {
			resource: image.to_owned(),
			size: memory_requirements.size as usize,
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	fn create_vulkan_sampler(
		&self,
		min_mag_filter: vk::Filter,
		reduction_mode: vk::SamplerReductionMode,
		mip_map_filter: vk::SamplerMipmapMode,
		address_mode: vk::SamplerAddressMode,
		anisotropy: Option<f32>,
		min_lod: f32,
		max_lod: f32,
	) -> vk::Sampler {
		let mut vk_sampler_reduction_mode_create_info =
			vk::SamplerReductionModeCreateInfo::default().reduction_mode(reduction_mode);

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
			.unnormalized_coordinates(false);

		let sampler = unsafe { self.device.create_sampler(&sampler_create_info, None).expect("No sampler") };

		sampler
	}

	pub(super) fn get_image_subresource_layout(
		&self,
		texture: &graphics_hardware_interface::ImageHandle,
		mip_level: u32,
	) -> graphics_hardware_interface::ImageSubresourceLayout {
		let image_subresource = vk::ImageSubresource {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			mip_level,
			array_layer: 0,
		};

		let texture = self.images.get(texture.0 as usize).expect("No texture with that handle.");

		if true
		/* TILING_OPTIMAL */
		{
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: 0,
				size: texture.size,
				row_pitch: texture.extent.width() as usize * texture.format_.size(),
				array_pitch: texture.extent.width() as usize * texture.extent.height() as usize * texture.format_.size(),
				depth_pitch: texture.extent.width() as usize
					* texture.extent.height() as usize
					* texture.extent.depth() as usize
					* texture.format_.size(),
			}
		} else {
			let image_subresource_layout =
				unsafe { self.device.get_image_subresource_layout(texture.image, image_subresource) };
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: image_subresource_layout.offset as usize,
				size: image_subresource_layout.size as usize,
				row_pitch: image_subresource_layout.row_pitch as usize,
				array_pitch: image_subresource_layout.array_pitch as usize,
				depth_pitch: image_subresource_layout.depth_pitch as usize,
			}
		}
	}

	fn bind_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> (u64, *mut u8) {
		let buffer = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_buffer_memory(buffer, allocation.memory, offset as u64)
				.expect("No buffer memory binding")
		};
		unsafe {
			(
				self.device
					.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)),
				allocation.pointer.add(offset),
			)
		}
	}

	fn bind_host_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> *mut u8 {
		let buffer = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_buffer_memory(buffer, allocation.memory, offset as u64)
				.expect("No buffer memory binding")
		};
		unsafe { allocation.pointer.add(offset) }
	}

	fn bind_vulkan_texture_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Image>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> (u64, *mut u8) {
		let image = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_image_memory(image, allocation.memory, offset as u64)
				.expect("No image memory binding")
		};
		(0, unsafe { allocation.pointer.add(offset) })
	}

	fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
		let fence_create_info = vk::FenceCreateInfo::default().flags(
			vk::FenceCreateFlags::empty()
				| if signaled {
					vk::FenceCreateFlags::SIGNALED
				} else {
					vk::FenceCreateFlags::empty()
				},
		);
		unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") }
	}

	fn set_name<T: vk::Handle>(&self, handle: T, name: Option<&str>) {
		if let Some(name) = name {
			let name = std::ffi::CString::new(name).unwrap();
			let name = name.as_c_str();
			#[cfg(debug_assertions)]
			unsafe {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils
						.set_debug_utils_object_name(
							&vk::DebugUtilsObjectNameInfoEXT::default()
								.object_handle(handle)
								.object_name(name),
						)
						.ok();
					// Ignore errors, if the name can't be set, it's not a big deal.
				}
			}
		}
	}

	fn create_vulkan_semaphore(&self, name: Option<&str>, _: bool) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default();
		let handle = unsafe {
			self.device
				.create_semaphore(&semaphore_create_info, None)
				.expect("No semaphore")
		};

		self.set_name(handle, name);

		handle
	}

	fn create_vulkan_image_view(
		&self,
		name: Option<&str>,
		texture: &vk::Image,
		format: crate::Formats,
		_mip_levels: u32,
		base_layer: u32,
		layer_count: Option<NonZeroU32>,
	) -> vk::ImageView {
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(if layer_count.is_none() {
				vk::ImageViewType::TYPE_2D
			} else {
				vk::ImageViewType::TYPE_2D_ARRAY
			})
			.format(to_format(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if format != crate::Formats::Depth32 {
					vk::ImageAspectFlags::COLOR
				} else {
					vk::ImageAspectFlags::DEPTH
				},
				base_mip_level: 0,
				level_count: 1,
				base_array_layer: base_layer,
				layer_count: layer_count.map(|e| e.get()).unwrap_or(1),
			});

		let vk_image_view = unsafe {
			self.device
				.create_image_view(&image_view_create_info, None)
				.expect("No image view")
		};

		self.set_name(vk_image_view, name);

		vk_image_view
	}

	/// Creates swapchain-backed image wrappers chained across frames and returns the root handle.
	fn create_swapchain_image(
		&mut self,
		vk_image: vk::Image,
		format: crate::Formats,
		uses: crate::Uses,
		previous: Option<ImageHandle>,
	) -> ImageHandle {
		let root_handle = ImageHandle(self.images.len() as u64);
		let root_image = {
			let image_view = self.create_vulkan_image_view(None, &vk_image, format, 0, 0, None);

			let mut image_views = [vk::ImageView::null(); 8];
			image_views[0] = image_view;

			Image {
				next: None,
				size: 0,
				staging_buffer: None,
				pointer: None,
				image: vk_image,
				image_views,
				extent: Extent::cube(0, 0, 0),
				access: crate::DeviceAccesses::DeviceOnly,
				format: to_format(format),
				format_: format,
				uses,
				layers: None,
				owns_image: false,
			}
		};

		if let Some(previous) = previous {
			self.images[previous.0 as usize].next = Some(root_handle);
		}

		self.images.push(root_image);

		root_handle
	}

	fn create_vulkan_surface(&self, window_os_handles: &window::Handles) -> vk::SurfaceKHR {
		let surface = {
			#[cfg(target_os = "linux")]
			{
				let wayland_surface_create_info = vk::WaylandSurfaceCreateInfoKHR::default()
					.display(window_os_handles.display)
					.surface(window_os_handles.surface);

				unsafe {
					self.wayland_surface
						.create_wayland_surface(&wayland_surface_create_info, None)
						.expect("No surface")
				}
			}
			#[cfg(target_os = "windows")]
			{
				let win32_surface_create_info = vk::Win32SurfaceCreateInfoKHR::default()
					.hinstance(window_os_handles.hinstance.0 as isize)
					.hwnd(window_os_handles.hwnd.0 as isize);

				unsafe {
					self.win32_surface
						.create_win32_surface(&win32_surface_create_info, None)
						.expect("No surface")
				}
			}
			#[cfg(target_os = "macos")]
			{
				let metal_layer = objc2_quartz_core::CAMetalLayer::new();

				let view = &window_os_handles.view;
				let logical_size = view.frame().size;
				let drawable_size = view.convertSizeToBacking(logical_size);
				let scale_factor = if logical_size.width > 0.0 {
					(drawable_size.width / logical_size.width).max(1.0)
				} else if logical_size.height > 0.0 {
					(drawable_size.height / logical_size.height).max(1.0)
				} else {
					1.0
				};

				view.setWantsLayer(true);
				view.setLayer(Some(&metal_layer));
				metal_layer.setContentsScale(scale_factor);
				metal_layer.setDrawableSize(drawable_size);

				let macos_surface_create_info =
					vk::MetalSurfaceCreateInfoEXT::default().layer(objc2::rc::Retained::as_ptr(&metal_layer) as _);

				unsafe {
					self.macos_surface
						.create_metal_surface(&macos_surface_create_info, None)
						.expect("No surface")
				}
			}
		};

		let surface_capabilities = unsafe {
			self.surface
				.get_physical_device_surface_capabilities(self.physical_device, surface)
				.expect("No surface capabilities")
		};

		let surface_format = unsafe {
			self.surface
				.get_physical_device_surface_formats(self.physical_device, surface)
				.expect("No surface formats")
		};

		let _: vk::SurfaceFormatKHR = surface_format
			.iter()
			.find(|format| {
				format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
			})
			.expect("No surface format")
			.to_owned();

		let surface_present_modes = unsafe {
			self.surface
				.get_physical_device_surface_present_modes(self.physical_device, surface)
				.expect("No surface present modes")
		};

		let _: vk::PresentModeKHR = surface_present_modes
			.iter()
			.find(|present_mode| **present_mode == vk::PresentModeKHR::FIFO)
			.expect("No surface present mode")
			.to_owned();

		let _surface_resolution = surface_capabilities.current_extent;

		surface
	}

	/// Allocates memory from the device.
	fn create_allocation_internal(
		&mut self,
		size: usize,
		memory_bits: Option<u32>,
		device_accesses: crate::DeviceAccesses,
	) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		let memory_property_flags = {
			let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::CpuRead) {
				vk::MemoryPropertyFlags::HOST_VISIBLE
			} else {
				vk::MemoryPropertyFlags::empty()
			};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::CpuWrite) {
				vk::MemoryPropertyFlags::HOST_COHERENT
			} else {
				vk::MemoryPropertyFlags::empty()
			};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::GpuRead) {
				vk::MemoryPropertyFlags::DEVICE_LOCAL
			} else {
				vk::MemoryPropertyFlags::empty()
			};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::GpuWrite) {
				vk::MemoryPropertyFlags::DEVICE_LOCAL
			} else {
				vk::MemoryPropertyFlags::empty()
			};

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

		let mut memory_allocate_flags_info =
			vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);

		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(memory_type_index)
			.push_next(&mut memory_allocate_flags_info);

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

		let mut mapped_memory = None;

		if device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite) {
			mapped_memory = Some(unsafe {
				self.device
					.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty())
					.expect("No mapped memory") as *mut u8
			});
		}

		let allocation_handle = graphics_hardware_interface::AllocationHandle(self.allocations.len() as u64);

		self.allocations.push(Allocation {
			memory,
			pointer: mapped_memory.unwrap_or(std::ptr::null_mut()),
		});

		(allocation_handle, mapped_memory)
	}

	/// Builds a buffer object with the given name, resource uses, size, Vulkan buffer usage flags, and device accesses.
	fn build_buffer_internal(
		&mut self,
		next: Option<BufferHandle>,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		device_accesses: crate::DeviceAccesses,
	) -> Buffer {
		if size == 0 {
			return Buffer {
				next,
				staging: None,
				source: None,
				buffer: vk::Buffer::null(),
				size: 0,
				device_address: 0,
				pointer: std::ptr::null_mut(),
				uses: resource_uses,
				access: device_accesses,
			};
		}

		let vk_usage_flags = uses_to_vk_usage_flags(resource_uses);

		// Remove acceleration structure usage flags if ray tracing is disabled (causes validation errors)
		let vk_usage_flags = if !self.settings.ray_tracing {
			vk_usage_flags & !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
		} else {
			vk_usage_flags
		};

		// Add shader device address usage flag as all buffers are guaranteed to be accessible by addressing
		let vk_usage_flags = vk_usage_flags | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;

		let vk_usage_flags = vk_usage_flags
			| if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::empty()
			} | if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			vk::BufferUsageFlags::TRANSFER_SRC
		} else {
			vk::BufferUsageFlags::empty()
		};

		let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_usage_flags);
		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			device_accesses & !(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite),
		);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let staging = if device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite) {
			let buffer_handle = BufferHandle(self.buffers.len() as u64);

			let vk_usage_flags = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::empty()
			} | if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
				vk::BufferUsageFlags::TRANSFER_SRC
			} else {
				vk::BufferUsageFlags::empty()
			} | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;

			let device_access = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				crate::DeviceAccesses::GpuWrite | crate::DeviceAccesses::CpuRead
			} else {
				crate::DeviceAccesses::empty()
			} | if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
				crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::CpuWrite
			} else {
				crate::DeviceAccesses::empty()
			};

			let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_usage_flags);
			let (allocation_handle, _) = self.create_allocation_internal(
				buffer_creation_result.size,
				buffer_creation_result.memory_flags.into(),
				device_access,
			);
			let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			let staging_buffer = Buffer {
				next,
				staging: None,
				source: None,
				buffer: buffer_creation_result.resource,
				size,
				device_address,
				pointer,
				uses: resource_uses,
				access: device_accesses,
			};

			self.buffers.push(staging_buffer);

			Some(buffer_handle)
		} else {
			None
		};

		Buffer {
			next,
			staging,
			source: None,
			buffer: buffer_creation_result.resource,
			size,
			device_address,
			pointer,
			uses: resource_uses,
			access: device_accesses,
		}
	}

	/// Builds a buffer and returns its handle.
	fn create_buffer_internal(
		&mut self,
		next: Option<BufferHandle>,
		previous: Option<BufferHandle>,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		device_accesses: crate::DeviceAccesses,
	) -> BufferHandle {
		let buffer = self.build_buffer_internal(next, name, resource_uses, size, device_accesses);

		let buffer_handle = BufferHandle(self.buffers.len() as u64);

		if let Some(previous) = previous {
			self.buffers[previous.0 as usize].next = Some(buffer_handle);
		}

		self.buffers.push(buffer);

		buffer_handle
	}

	/// Creates a CPU-visible staging buffer (TRANSFER_SRC) for use as a per-frame
	/// staging buffer in the persistent write mode. Returns its handle.
	fn create_staging_buffer(&mut self, name: Option<&str>, size: usize) -> BufferHandle {
		let vk_usage_flags = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
		let device_access = crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::CpuWrite;

		let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_usage_flags);
		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			device_access,
		);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let handle = BufferHandle(self.buffers.len() as u64);

		self.buffers.push(Buffer {
			next: None,
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size,
			device_address,
			pointer,
			uses: crate::Uses::empty(),
			access: device_access,
		});

		handle
	}

	fn build_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: crate::DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		extent: Extent,
		resource_uses: crate::Uses,
	) -> Image {
		let size = extent.width() as usize * extent.height() as usize * extent.depth() as usize * format.size();

		if size == 0 {
			return Image {
				next,
				size: 0,
				staging_buffer: None,
				pointer: None,
				image: vk::Image::null(),
				image_views: [vk::ImageView::null(); 8],
				extent,
				access: device_accesses,
				format: to_format(format),
				format_: format,
				uses: resource_uses,
				layers: array_layers,
				owns_image: true,
			};
		}

		let vk_extent = vk::Extent3D {
			width: extent.width(),
			height: extent.height(),
			depth: extent.depth(),
		};

		let transfer_uses = (if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			crate::Uses::TransferSource
		} else {
			crate::Uses::empty()
		}) | (if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
			crate::Uses::TransferDestination
		} else {
			crate::Uses::empty()
		});

		let texture_creation_result =
			self.create_vulkan_texture(name, vk_extent, format, resource_uses | transfer_uses, 1, array_layers);

		let m_device_accesses = if device_accesses.intersects(crate::DeviceAccesses::HostOnly) {
			crate::DeviceAccesses::DeviceOnly
		} else {
			device_accesses
		};

		let (allocation_handle, _) = self.create_allocation_internal(
			texture_creation_result.size,
			texture_creation_result.memory_flags.into(),
			m_device_accesses,
		);

		let _ = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);

		let (staging_buffer, pointer) = if device_accesses.intersects(crate::DeviceAccesses::HostOnly) {
			let vk_buffer_usage_flags = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::TRANSFER_SRC
			};

			let device_accesses = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				crate::DeviceAccesses::DeviceToHost
			} else {
				crate::DeviceAccesses::HostToDevice
			};

			let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_buffer_usage_flags);
			let (allocation_handle, _) = self.create_allocation_internal(
				buffer_creation_result.size,
				buffer_creation_result.memory_flags.into(),
				device_accesses,
			);
			let pointer = self.bind_host_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			(Some(buffer_creation_result.resource), Some(pointer))
		} else {
			(None, None)
		};

		let image_views = {
			let mut image_views = [vk::ImageView::null(); 8];

			if let Some(l) = array_layers.map(|e| e.get()) {
				for i in 0..l {
					image_views[i as usize] = self.create_vulkan_image_view(
						name,
						&texture_creation_result.resource,
						format,
						0,
						i,
						NonZeroU32::new(1),
					);
				}
			} else {
				image_views[0] = self.create_vulkan_image_view(name, &texture_creation_result.resource, format, 0, 0, None);
			}

			image_views
		};

		Image {
			next,
			size,
			staging_buffer,
			pointer,
			image: texture_creation_result.resource,
			image_views,
			extent,
			access: device_accesses,
			format: to_format(format),
			format_: format,
			uses: resource_uses,
			layers: array_layers,
			owns_image: true,
		}
	}

	fn create_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		previous: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: crate::DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		extent: Extent,
		resource_uses: crate::Uses,
	) -> ImageHandle {
		let texture_handle = ImageHandle(self.images.len() as u64);

		let image = self.build_image_internal(next, name, format, device_accesses, array_layers, extent, resource_uses);

		if let Some(previous) = previous {
			self.images[previous.0 as usize].next = Some(texture_handle);
		}

		self.images.push(image);

		texture_handle
	}

	fn create_synchronizer_internal(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u64);

		self.synchronizers.push(Synchronizer {
			next: None,
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

			self.tasks.push(Task::delete_vulkan_buffer(current_vk_buffer, None));
			self.tasks.push(Task::update_buffer_descriptor(buffer_handle, None));

			// todo!("copy data from old buffer to new buffer");
		}

		let new_buffer = self.build_buffer_internal(
			None,
			None,
			current_buffer.uses,
			size,
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		self.buffers[buffer_handle.0 as usize] = new_buffer;
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent, sequence_index: u8) {
		let image = image_handle.access(&self.images);

		if !image.owns_image {
			return;
		}

		if image.extent == extent {
			// Requested extent matches current extent, no resize needed
			return;
		}

		if let Some(staging_buffer_handle) = image.staging_buffer {
			self.tasks
				.push(Task::delete_vulkan_buffer(staging_buffer_handle, Some(sequence_index)));
		}

		for image_view in image.image_views {
			if !image_view.is_null() {
				self.tasks.push(Task::delete_vulkan_image_view(image_view, sequence_index));
			}
		}

		self.tasks.push(Task::delete_vulkan_image(image.image, sequence_index));

		// TODO: release memory/allocation

		#[cfg(debug_assertions)]
		let name = self
			.names
			.get(&graphics_hardware_interface::ImageHandle(image_handle.root(&self.images).0).into())
			.map(|s| s.clone());

		#[cfg(not(debug_assertions))]
		let name: Option<String> = None;

		let new_image = self.build_image_internal(
			image.next,
			name.as_ref().map(|e| e.as_str()),
			image.format_,
			image.access,
			image.layers,
			extent,
			image.uses,
		);

		self.images[image_handle.0 as usize] = new_image;

		if let Some(state) = self.states.get_mut(&image_handle.into()) {
			state.layout = vk::ImageLayout::UNDEFINED;
		}

		self.update_image_bindings(image_handle);
	}

	/// Add the task to all frames
	pub(crate) fn add_task_to_all_frames(&mut self, tasks: Tasks) {
		for i in 0..self.frames {
			self.tasks.push(Task::new(tasks, Some(i)));
		}
	}

	/// Add the task to all other frames but the current frame.
	pub(crate) fn add_task_to_all_other_frames(&mut self, tasks: Tasks, current_frame: u8) {
		for i in 1..self.frames {
			// Skip current frame
			let i = current_frame + i; // Offset by current frame
			let i = i.rem_euclid(self.frames); // Wrap around frames
			self.tasks.push(Task::new(tasks, Some(i)));
		}
	}

	#[must_use]
	fn produce_writes(&self, writes: impl IntoIterator<Item = DescriptorWrite>) -> SmallVec<[WriteResult; 128]> {
		let mut buffers: StableVec<vk::DescriptorBufferInfo, 1024> = StableVec::new();
		let mut images: StableVec<vk::DescriptorImageInfo, 1024> = StableVec::new();

		let mut write_results = SmallVec::<[WriteResult; 128]>::new();

		let writes = writes
			.into_iter()
			.filter_map(|descriptor_set_write| {
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
							let e = buffers.append([vk::DescriptorBufferInfo::default()
								.buffer(buffer.buffer)
								.offset(0u64)
								.range(match size {
									graphics_hardware_interface::Ranges::Size(size) => size as u64,
									graphics_hardware_interface::Ranges::Whole => vk::WHOLE_SIZE,
								})]);

							let write_info = vk::WriteDescriptorSet::default()
								.dst_set(descriptor_set.descriptor_set)
								.dst_binding(binding_index)
								.dst_array_element(descriptor_set_write.array_element)
								.descriptor_type(descriptor_type)
								.buffer_info(e);

							Some(write_info)
						} else {
							None
						};

						write_results.push(WriteResult {
							array_element: descriptor_set_write.array_element,
							binding_handle,
							descriptor_set_handle,
							binding_index,
							descriptor: Descriptor::Buffer {
								size,
								buffer: buffer_handle,
							},
						});

						res
					}
					Descriptors::Image { handle, layout } => {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

						let image_handle = handle;

						let image = &self.images[image_handle.0 as usize];
						let image_view = image.image_views[0];
						let format = image.format_;
						let image = image.image;

						let res = if !image.is_null() && !image_view.is_null() {
							let e = images.append([vk::DescriptorImageInfo::default()
								.image_layout(texture_format_and_resource_use_to_image_layout(format, layout, None))
								.image_view(image_view)]);

							let write_info = vk::WriteDescriptorSet::default()
								.dst_set(descriptor_set.descriptor_set)
								.dst_binding(binding_index)
								.dst_array_element(descriptor_set_write.array_element)
								.descriptor_type(descriptor_type)
								.image_info(&e);

							Some(write_info)
						} else {
							None
						};

						write_results.push(WriteResult {
							array_element: descriptor_set_write.array_element,
							binding_handle,
							descriptor_set_handle,
							binding_index,
							descriptor: Descriptor::Image {
								layout,
								image: image_handle,
							},
						});

						res
					}
					Descriptors::CombinedImageSampler {
						image_handle,
						sampler_handle,
						layout,
						layer,
					} => {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];

						let image = &self.images[image_handle.0 as usize];

						let res = if !image.image.is_null() {
							let image_view = if let Some(layer) = layer {
								// If the descriptor asks for a subresource, we need to create a new image view
								image.image_views[layer as usize]
							} else {
								image.image_views[0]
							};

							let e = images.append([vk::DescriptorImageInfo::default()
								.image_layout(texture_format_and_resource_use_to_image_layout(image.format_, layout, None))
								.image_view(image_view)
								.sampler(vk::Sampler::from_raw(sampler_handle.0))]);

							let write_info = vk::WriteDescriptorSet::default()
								.dst_set(descriptor_set.descriptor_set)
								.dst_binding(binding_index)
								.dst_array_element(descriptor_set_write.array_element)
								.descriptor_type(descriptor_type)
								.image_info(e);

							Some(write_info)
						} else {
							None
						};

						write_results.push(WriteResult {
							array_element: descriptor_set_write.array_element,
							binding_handle,
							descriptor_set_handle,
							binding_index,
							descriptor: Descriptor::CombinedImageSampler {
								image: image_handle,
								sampler: vk::Sampler::from_raw(sampler_handle.0),
								layout,
							},
						});

						res
					}
					Descriptors::Sampler { handle } => {
						let descriptor_set = &self.descriptor_sets[descriptor_set_handle.0 as usize];
						let sampler_handle = handle;
						let e = images
							.append([vk::DescriptorImageInfo::default().sampler(vk::Sampler::from_raw(sampler_handle.0))]);

						let write_info = vk::WriteDescriptorSet::default()
							.dst_set(descriptor_set.descriptor_set)
							.dst_binding(binding_index)
							.dst_array_element(descriptor_set_write.array_element)
							.descriptor_type(descriptor_type)
							.image_info(e);

						// self.descriptors.entry(descriptor_set_handle).or_insert_with(HashMap::new).entry(binding_index).or_insert_with(HashMap::new).insert(descriptor_set_write.array_element, Descriptor::Sampler{ sampler: vk::Sampler::from_raw(sampler_handle.0) });
						// self.resource_to_descriptor.entry(Handle::Sampler(sampler_handle)).or_insert_with(HashSet::new).insert((binding_handle, descriptor_set_write.array_element));

						Some(write_info)
					}
				}
			})
			.collect::<SmallVec<[vk::WriteDescriptorSet; 128]>>();

		unsafe { self.device.update_descriptor_sets(&writes, &[]) };

		write_results
	}

	fn process_write_results(&mut self, writes: SmallVec<[WriteResult; 128]>) {
		for write in writes {
			let descriptor_set_handle = write.descriptor_set_handle;
			let binding_index = write.binding_index;
			let array_element = write.array_element;
			let binding_handle = write.binding_handle;

			match write.descriptor {
				Descriptor::Buffer { buffer, size } => {
					self.descriptors
						.entry(descriptor_set_handle)
						.or_insert_with(HashMap::new)
						.entry(binding_index)
						.or_insert_with(HashMap::new)
						.insert(array_element, Descriptor::Buffer { size, buffer });
					self.descriptor_set_to_resource
						.entry((descriptor_set_handle, binding_index))
						.or_insert_with(HashSet::new)
						.insert(Handle::Buffer(buffer));
					self.resource_to_descriptor
						.entry(Handle::Buffer(buffer))
						.or_insert_with(HashSet::new)
						.insert((binding_handle, array_element));
				}
				Descriptor::Image { image, layout } => {
					self.descriptors
						.entry(descriptor_set_handle)
						.or_insert_with(HashMap::new)
						.entry(binding_index)
						.or_insert_with(HashMap::new)
						.insert(array_element, Descriptor::Image { image, layout });
					self.descriptor_set_to_resource
						.entry((descriptor_set_handle, binding_index))
						.or_insert_with(HashSet::new)
						.insert(Handle::Image(image));
					self.resource_to_descriptor
						.entry(Handle::Image(image))
						.or_insert_with(HashSet::new)
						.insert((binding_handle, array_element));
				}
				Descriptor::CombinedImageSampler { image, sampler, layout } => {
					self.descriptors
						.entry(descriptor_set_handle)
						.or_insert_with(HashMap::new)
						.entry(binding_index)
						.or_insert_with(HashMap::new)
						.insert(array_element, Descriptor::CombinedImageSampler { image, sampler, layout });
					self.descriptor_set_to_resource
						.entry((descriptor_set_handle, binding_index))
						.or_insert_with(HashSet::new)
						.insert(Handle::Image(image));
					self.resource_to_descriptor
						.entry(Handle::Image(image))
						.or_insert_with(HashSet::new)
						.insert((binding_handle, array_element));
				}
			}
		}
	}

	fn write_internal(&mut self, writes: impl IntoIterator<Item = DescriptorWrite>) {
		let writes = self.produce_writes(writes);
		self.process_write_results(writes);
	}

	pub(crate) fn add_descriptor_writes_for_update_buffer_descriptors(
		&self,
		handle: BufferHandle,
		descriptor_writes: &mut impl Extend<DescriptorWrite>,
	) {
		if let Some(e) = self.resource_to_descriptor.get(&handle.into()) {
			for (binding_handle, index) in e {
				let binding = binding_handle.access(&self.bindings);

				if let Some(descriptor) = self
					.descriptors
					.get(&binding.descriptor_set_handle)
					.and_then(|d| d.get(&binding.index))
					.and_then(|d| d.get(&index))
				{
					match descriptor {
						Descriptor::Buffer { size, .. } => {
							descriptor_writes.extend_one(
								DescriptorWrite::new(Descriptors::Buffer { handle, size: *size }, *binding_handle)
									.index(*index),
							);
						}
						_ => {
							println!("Unexpected descriptor type for buffer handle {:#?}", handle);
						}
					}
				}
			}
		}
	}

	pub(crate) fn add_descriptor_writes_for_update_image_descriptors(
		&self,
		handle: ImageHandle,
		descriptor_writes: &mut impl Extend<DescriptorWrite>,
	) {
		if let Some(e) = self.resource_to_descriptor.get(&handle.into()) {
			for (binding_handle, index) in e {
				let binding = binding_handle.access(&self.bindings);

				if let Some(descriptor) = self
					.descriptors
					.get(&binding.descriptor_set_handle)
					.and_then(|d| d.get(&binding.index))
					.and_then(|d| d.get(&index))
				{
					match descriptor {
						Descriptor::Image { layout, .. } => {
							descriptor_writes.extend_one(
								DescriptorWrite::new(Descriptors::Image { handle, layout: *layout }, *binding_handle)
									.index(*index),
							);
						}
						Descriptor::CombinedImageSampler { sampler, layout, .. } => {
							descriptor_writes.extend_one(
								DescriptorWrite::new(
									Descriptors::CombinedImageSampler {
										image_handle: handle,
										sampler_handle: SamplerHandle(sampler.as_raw()),
										layout: *layout,
										layer: None,
									},
									*binding_handle,
								)
								.index(*index),
							);
						}
						_ => {
							println!("Unexpected descriptor type for image handle {:#?}", handle);
						}
					}
				}
			}
		}
	}

	pub(crate) fn update_image_bindings(&mut self, handle: ImageHandle) {
		let mut writes = SmallVec::<[DescriptorWrite; 8]>::new();
		self.add_descriptor_writes_for_update_image_descriptors(handle, &mut writes);
		self.write_internal(writes);
	}

	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let mut descriptor_writes = Vec::with_capacity(32);

		let mut tasks = self.tasks.split_off(0);

		// TODO: optimize consecutive tasks such as two resize tasks

		tasks.retain(|e| {
			if let Some(e) = e.frame() {
				if e != sequence_index {
					return true;
				}
			}

			// Helps debug issues related to use after delete cases.
			let disable_deletions = false;

			match e.task() {
				Tasks::DeleteVulkanImage { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_image(*handle, None);
					}
				}
				Tasks::DeleteVulkanImageView { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_image_view(*handle, None);
					}
				}
				Tasks::DeleteVulkanBuffer { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_buffer(*handle, None);
					}
				}
				Tasks::UpdateBufferDescriptors { handle } => {
					self.add_descriptor_writes_for_update_buffer_descriptors(*handle, &mut descriptor_writes);
				}
				Tasks::UpdateDescriptor { descriptor_write } => {
					let binding_handles = DescriptorSetBindingHandle(descriptor_write.binding_handle.0).get_all(&self.bindings);

					if binding_handles.is_empty() {
						return false;
					}

					let binding = binding_handles[(sequence_index as usize).rem_euclid(binding_handles.len())];
					let frame_offset = descriptor_write.frame_offset.unwrap_or(0);

					let new_descriptor_write = match descriptor_write.descriptor {
						crate::descriptors::WriteData::Buffer { handle, size } => {
							let handles = BufferHandle(handle.0).get_all(&self.buffers);
							let index = (sequence_index as i32 - frame_offset).rem_euclid(handles.len() as i32) as usize;
							let handle = handles[index];
							Some(DescriptorWrite::new(Descriptors::Buffer { handle, size }, binding))
						}
						crate::descriptors::WriteData::Image { handle, layout } => {
							let handles = ImageHandle(handle.0).get_all(&self.images);
							let index = (sequence_index as i32 - frame_offset).rem_euclid(handles.len() as i32) as usize;
							let handle = handles[index];
							Some(DescriptorWrite::new(Descriptors::Image { handle, layout }, binding))
						}
						crate::descriptors::WriteData::CombinedImageSampler {
							image_handle,
							sampler_handle,
							layout,
							layer,
						} => {
							let image_handles = ImageHandle(image_handle.0).get_all(&self.images);
							let index = (sequence_index as i32 - frame_offset).rem_euclid(image_handles.len() as i32) as usize;
							let image_handle = image_handles[index];
							Some(DescriptorWrite::new(
								Descriptors::CombinedImageSampler {
									image_handle,
									sampler_handle: SamplerHandle(sampler_handle.0),
									layout,
									layer,
								},
								binding,
							))
						}
						crate::descriptors::WriteData::Sampler(sampler_handle) => Some(DescriptorWrite::new(
							Descriptors::Sampler {
								handle: SamplerHandle(sampler_handle.0),
							},
							binding,
						)),
						_ => None,
					};

					if let Some(write) = new_descriptor_write {
						descriptor_writes.push(write.index(descriptor_write.array_element));
					}
				}
				Tasks::BuildImage(builder) => {
					#[cfg(debug_assertions)]
					let name = self
						.names
						.get(&graphics_hardware_interface::Handle::Image(builder.master))
						.map(|e| e.clone());

					#[cfg(not(debug_assertions))]
					let name: Option<String> = None;

					let previous_image = builder.previous.access(&self.images);

					self.create_image_internal(
						None,
						Some(builder.previous),
						name.as_ref().map(|e| e.as_str()),
						previous_image.format_,
						previous_image.access,
						previous_image.layers,
						previous_image.extent,
						previous_image.uses,
					);
				}
				Tasks::BuildBuffer(builder) => {
					#[cfg(debug_assertions)]
					let name = self
						.names
						.get(&graphics_hardware_interface::Handle::Buffer(builder.master))
						.map(|e| e.clone());

					#[cfg(not(debug_assertions))]
					let name: Option<String> = None;

					let previous_buffer = builder.previous.access(&self.buffers);

					let new_buffer_handle = self.create_buffer_internal(
						None,
						Some(builder.previous),
						name.as_ref().map(|e| e.as_str()),
						previous_buffer.uses,
						previous_buffer.size,
						previous_buffer.access,
					);

					// When PERSISTENT_WRITE is enabled and this buffer has a source,
					// create a per-frame staging buffer and point the new buffer's
					// staging and source fields accordingly.
					if let Some(source_handle) = builder.source {
						let size = self.buffers[new_buffer_handle.0 as usize].size;
						let per_frame_staging = self.create_staging_buffer(name.as_ref().map(|e| e.as_str()), size);
						self.buffers[new_buffer_handle.0 as usize].staging = Some(per_frame_staging);
						self.buffers[new_buffer_handle.0 as usize].source = Some(source_handle);
					}
				}
				Tasks::ResizeImage { handle, extent } => {
					let root_handle = handle.root(&self.images);
					let handles = root_handle.get_all(&self.images);
					let handle = handles[(sequence_index as usize).rem_euclid(handles.len())];
					self.resize_image_internal(handle, *extent, sequence_index);
				}
			}

			false
		});

		self.write_internal(descriptor_writes);

		self.tasks = tasks;
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

			self.synchronizers.iter().for_each(|synchronizer| {
				self.device.destroy_semaphore(synchronizer.semaphore, None);
				self.device.destroy_fence(synchronizer.fence, None);
			});

			self.descriptor_sets_layouts.iter().for_each(|descriptor_set_layout| {
				self.device
					.destroy_descriptor_set_layout(descriptor_set_layout.descriptor_set_layout, None);
			});

			self.descriptor_pools.iter().for_each(|descriptor_pool| {
				self.device.destroy_descriptor_pool(*descriptor_pool, None);
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
				for vk_image_view in image.image_views {
					self.device.destroy_image_view(vk_image_view, None);
				}
			});

			self.swapchains.iter().for_each(|swapchain| {
				self.swapchain.destroy_swapchain(swapchain.swapchain, None);
				self.surface.destroy_surface(swapchain.surface, None);
			});

			self.images.iter().for_each(|image| {
				if image.owns_image {
					self.device.destroy_image(image.image, None);
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

impl crate::device::Device for Device {
	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		if self.frames == frames {
			return;
		}

		if frames > MAX_FRAMES_IN_FLIGHT as u8 {
			panic!("Cannot set frames in flight to more than {}", MAX_FRAMES_IN_FLIGHT);
		}

		let current_frames = self.frames;
		let target_frames = frames;
		let delta_frames = target_frames as i8 - current_frames as i8;

		if delta_frames > 0 {
			let to_extend = self
				.images
				.iter()
				.filter_map(|image| {
					let next = image.next?;

					let mut handle = next;

					while let Some(h) = self.images[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

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
				let extent = current_image.extent;
				let resource_uses = current_image.uses;

				let new_image =
					self.create_image_internal(next, None, name, format, access, array_layers, extent, resource_uses);

				let current_image = &mut self.images[image_handle.0 as usize];
				current_image.next = Some(new_image);
			}

			let to_extend = self
				.synchronizers
				.iter()
				.filter_map(|synchronizer| {
					let next = synchronizer.next?;

					let mut handle = next;

					while let Some(h) = self.synchronizers[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

			for synchronizer_handle in to_extend {
				let current_synchronizer = &self.synchronizers[synchronizer_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name_owned = self
					.names
					.get(
						&graphics_hardware_interface::SynchronizerHandle(synchronizer_handle.root(&self.synchronizers).0)
							.into(),
					)
					.cloned();

				#[cfg(not(debug_assertions))]
				let name_owned: Option<String> = None;

				let name = name_owned.as_deref();
				let signaled = current_synchronizer.signaled;

				let new_synchronizer = self.create_synchronizer_internal(name, signaled);

				let current_synchronizer = &mut self.synchronizers[synchronizer_handle.0 as usize];
				current_synchronizer.next = Some(new_synchronizer);
			}

			for command_buffer in &mut self.command_buffers {
				let queue = &self.queues[command_buffer.queue_handle.0 as usize];
				let command_pool_create_info =
					vk::CommandPoolCreateInfo::default().queue_family_index(queue.queue_family_index);

				let command_pool = unsafe {
					self.device
						.create_command_pool(&command_pool_create_info, None)
						.expect("No command pool")
				};

				let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
					.command_pool(command_pool)
					.level(vk::CommandBufferLevel::PRIMARY)
					.command_buffer_count(1);

				let command_buffers = unsafe {
					self.device
						.allocate_command_buffers(&command_buffer_allocate_info)
						.expect("No command buffer")
				};

				let vk_command_buffer = command_buffers[0];

				// self.set_name(vk_command_buffer, name);

				command_buffer.frames.push(CommandBufferInternal {
					vk_queue: queue.vk_queue,
					command_pool,
					command_buffer: vk_command_buffer,
				});
			}
		} else {
			unimplemented!()
		}

		self.frames = target_frames;
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		let writes = descriptor_set_writes
			.iter()
			.filter_map(|descriptor_set_write| {
				let binding_handles = DescriptorSetBindingHandle(descriptor_set_write.binding_handle.0).get_all(&self.bindings);

				// assert!(descriptor_set_write.array_element < binding.count, "Binding index out of range.");

				match descriptor_set_write.descriptor {
					crate::descriptors::WriteData::Buffer { handle, size } => {
						let buffer_handles = BufferHandle(handle.0).get_all(&self.buffers);

						let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

						for (i, &binding_handle) in binding_handles.iter().enumerate() {
							let offset = descriptor_set_write.frame_offset.unwrap_or(0);

							let buffer_handle =
								buffer_handles[(i as i32 - offset).rem_euclid(buffer_handles.len() as i32) as usize];

							writes.push(
								DescriptorWrite::new(
									Descriptors::Buffer {
										handle: buffer_handle,
										size,
									},
									binding_handle,
								)
								.index(descriptor_set_write.array_element),
							);
						}

						Some(writes)
					}
					crate::descriptors::WriteData::Image { handle, layout } => {
						let image_handles = ImageHandle(handle.0).get_all(&self.images);
						let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

						for (i, &binding_handle) in binding_handles.iter().enumerate() {
							let offset = descriptor_set_write.frame_offset.unwrap_or(0);

							let image_handle =
								image_handles[(i as i32 - offset).rem_euclid(image_handles.len() as i32) as usize];

							writes.push(
								DescriptorWrite::new(
									Descriptors::Image {
										handle: image_handle,
										layout,
									},
									binding_handle,
								)
								.index(descriptor_set_write.array_element),
							);
						}

						Some(writes)
					}
					crate::descriptors::WriteData::CombinedImageSampler {
						image_handle,
						sampler_handle,
						layout,
						layer,
					} => {
						let image_handles = ImageHandle(image_handle.0).get_all(&self.images);
						let sampler_handle = SamplerHandle(sampler_handle.0);

						let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

						for (i, &binding_handle) in binding_handles.iter().enumerate() {
							let offset = descriptor_set_write.frame_offset.unwrap_or(0);

							let image_handle =
								image_handles[(i as i32 - offset).rem_euclid(image_handles.len() as i32) as usize];

							writes.push(
								DescriptorWrite::new(
									Descriptors::CombinedImageSampler {
										image_handle,
										layout,
										sampler_handle,
										layer,
									},
									binding_handle,
								)
								.index(descriptor_set_write.array_element),
							);
						}

						Some(writes)
					}
					crate::descriptors::WriteData::Sampler(handle) => {
						let mut writes = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

						let sampler_handle = SamplerHandle(handle.0);

						for (_, &binding_handle) in binding_handles.iter().enumerate() {
							writes.push(
								DescriptorWrite::new(Descriptors::Sampler { handle: sampler_handle }, binding_handle)
									.index(descriptor_set_write.array_element),
							);
						}

						Some(writes)
					}
					_ => unimplemented!(),
				}
			})
			.flatten();

		let writes = self.produce_writes(writes);
		self.process_write_results(writes);
	}

	fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> crate::vulkan::CommandBufferRecording<'_> {
		let pending_buffers = &mut self.pending_buffer_syncs;

		let buffer_copies: Vec<BufferCopy> = pending_buffers
			.drain()
			.map(|e| {
				let dst_buffer_handle = e;

				let dst_buffer = &self.buffers[dst_buffer_handle.0 as usize];

				let src_buffer_handle = dst_buffer.staging.unwrap();

				BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size)
			})
			.collect();

		let pending_images = &mut self.pending_image_syncs;

		let image_copies: Vec<ImageCopy> = pending_images
			.drain()
			.map(|e| {
				let dst_image_handle = e;

				let dst_image = &self.images[dst_image_handle.0 as usize];

				ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
			})
			.collect();

		let mut recording = CommandBufferRecording::new(self, command_buffer_handle, None);

		recording.sync_buffers(buffer_copies.iter().copied());
		recording.sync_textures(image_copies.iter().copied());

		recording
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers[buffer_handle.0 as usize].device_address
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers[buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];
		unsafe { std::mem::transmute(buffer.pointer) }
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		let handle = BufferHandle(buffer_handle.0);

		let buffer = self.buffers[handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];

		unsafe { std::mem::transmute(buffer.pointer) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		let buffer_handle = buffer_handle.into();
		let handle = BufferHandle(buffer_handle.0);

		self.pending_buffer_syncs.insert(handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let texture = &self.images[texture_handle.0 as usize];
		let size = texture.size;
		assert!(
			texture.staging_buffer.is_some(),
			"Attempted to map an image without a staging buffer. The most likely cause is that the image was created without CPU-visible access but is being written from the CPU."
		);
		let pointer = texture.pointer.expect(
			"Attempted to map an image without a CPU-visible pointer. The most likely cause is that image resize or creation did not rebuild the host-visible staging allocation."
		);
		assert!(
			size > 0,
			"Attempted to map a zero-sized image. The most likely cause is that the image was used before receiving a valid extent."
		);

		unsafe { std::slice::from_raw_parts_mut(pointer, size) }
	}

	fn sync_texture(&mut self, image_handle: crate::ImageHandle) {
		let image_handle = ImageHandle(image_handle.0);
		let image = &self.images[image_handle.0 as usize];
		assert!(
			image.staging_buffer.is_some(),
			"Attempted to sync an image without a staging buffer. The most likely cause is that CPU-side image uploads are being requested for a GPU-only image."
		);

		self.pending_image_syncs.insert(image_handle);
	}

	fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handles = ImageHandle(image_handle.0).get_all(&self.images);

		let handle = handles[0];

		let texture = handle.access(&self.images);

		let pointer = texture.pointer.unwrap();
		let size = texture.size;

		let slice = unsafe { std::slice::from_raw_parts_mut(pointer, size) };

		f(slice);

		self.pending_image_syncs.insert(handle);
	}

	fn write_instance(
		&mut self,
		instances_buffer: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		let buffer = self.acceleration_structures[acceleration_structure.0 as usize].buffer;

		let address = unsafe {
			self.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
		};

		let instance = vk::AccelerationStructureInstanceKHR {
			transform: vk::TransformMatrixKHR {
				matrix: [
					transform[0][0],
					transform[0][1],
					transform[0][2],
					transform[0][3],
					transform[1][0],
					transform[1][1],
					transform[1][2],
					transform[1][3],
					transform[2][0],
					transform[2][1],
					transform[2][2],
					transform[2][3],
				],
			},
			instance_custom_index_and_mask: vk::Packed24_8::new(custom_index as u32, mask),
			instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
				sbt_record_offset as u32,
				vk::GeometryInstanceFlagsKHR::FORCE_OPAQUE.as_raw() as u8,
			),
			acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: address },
		};

		let instance_buffer = &mut self.buffers[instances_buffer.0 as usize];

		let instance_buffer_slice = unsafe {
			std::slice::from_raw_parts_mut(
				instance_buffer.pointer as *mut vk::AccelerationStructureInstanceKHR,
				instance_buffer.size / std::mem::size_of::<vk::AccelerationStructureInstanceKHR>(),
			)
		};

		instance_buffer_slice[instance_index] = instance;
	}

	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let shader_handles = pipeline.shader_handles.clone();

		let buffer = self.buffers[sbt_buffer_handle.0 as usize];
		let buffer = self.buffers[buffer.staging.unwrap().0 as usize];

		(unsafe { std::slice::from_raw_parts_mut(buffer.pointer, buffer.size) })[sbt_record_offset..sbt_record_offset + 32]
			.copy_from_slice(shader_handles.get(&shader_handle).unwrap());
	}

	fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let buffer_handle = BufferHandle(buffer_handle.0);

		self.resize_buffer_internal(buffer_handle, size);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let vk_surface = self.create_vulkan_surface(window_os_handles);

		let vk_present_mode = match presentation_mode {
			graphics_hardware_interface::PresentationModes::FIFO => vk::PresentModeKHR::FIFO,
			graphics_hardware_interface::PresentationModes::Inmediate => vk::PresentModeKHR::IMMEDIATE,
			graphics_hardware_interface::PresentationModes::Mailbox => vk::PresentModeKHR::MAILBOX,
		};

		let mut vk_surface_present_mode = vk::SurfacePresentModeEXT::default().present_mode(vk_present_mode);

		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
			.push_next(&mut vk_surface_present_mode)
			.surface(vk_surface);

		let mut vk_presentation_modes = [vk::PresentModeKHR::default(); 8];

		let mut vk_surface_present_mode_compatibility =
			vk::SurfacePresentModeCompatibilityEXT::default().present_modes(&mut vk_presentation_modes);

		let mut vk_surface_capabilities =
			vk::SurfaceCapabilities2KHR::default().push_next(&mut vk_surface_present_mode_compatibility);

		unsafe {
			self.surface_capabilities
				.get_physical_device_surface_capabilities2(self.physical_device, &vk_surface_info, &mut vk_surface_capabilities)
				.expect("No surface capabilities")
		};

		let vk_surface_capabilities = vk_surface_capabilities.surface_capabilities;

		let min_image_count = vk_surface_capabilities.min_image_count;
		let max_image_count = vk_surface_capabilities.max_image_count;

		let extent = if vk_surface_capabilities.current_extent.width != u32::MAX
			&& vk_surface_capabilities.current_extent.height != u32::MAX
		{
			vk_surface_capabilities.current_extent
		} else {
			vk::Extent2D::default()
				.width(fallback_extent.width())
				.height(fallback_extent.height())
		};

		let presentation_modes = [vk_present_mode];

		let mut present_modes_create_info =
			vk::SwapchainPresentModesCreateInfoEXT::default().present_modes(&presentation_modes);

		let requested_image_count = if max_image_count != 0 {
			max_image_count.max(min_image_count)
		} else {
			(min_image_count * 2).min(MAX_SWAPCHAIN_IMAGES as u32)
		};

		let format = crate::Formats::BGRAsRGB;

		let requested_image_usage = into_vk_image_usage_flags(uses, format);
		let supported_image_usage = vk_surface_capabilities.supported_usage_flags;
		let uses_proxy_images = !supported_image_usage.contains(requested_image_usage);

		let native_image_usage = if uses_proxy_images {
			let fallback_usage = vk::ImageUsageFlags::TRANSFER_DST;

			if !supported_image_usage.contains(fallback_usage) {
				panic!(
					"Failed to create swapchain fallback copy path. The most likely cause is that the surface does not support transfer destination usage for swapchain images."
				);
			}

			fallback_usage
		} else {
			requested_image_usage
		};

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.push_next(&mut present_modes_create_info)
			.flags(vk::SwapchainCreateFlagsKHR::DEFERRED_MEMORY_ALLOCATION_EXT)
			.surface(vk_surface)
			.min_image_count(requested_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(extent)
			.image_usage(native_image_usage)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(vk_surface_capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(vk_present_mode)
			.image_array_layers(1)
			.clipped(true);

		let vk_swapchain = unsafe {
			self.swapchain
				.create_swapchain(&swapchain_create_info, None)
				.expect("No swapchain")
		};

		let swapchain_handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		let mut acquire_synchronizers = [SynchronizerHandle(!0u64); MAX_FRAMES_IN_FLIGHT];

		for i in 0..self.frames {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Acquire Sync"), true);
			acquire_synchronizers[i as usize] = synchronizer;
		}

		let vk_images = unsafe {
			self.swapchain
				.get_swapchain_images(vk_swapchain)
				.expect("No swapchain images found.")
		};
		let image_count = vk_images.len() as u32;

		let mut submit_synchronizers = [SynchronizerHandle(!0u64); MAX_SWAPCHAIN_IMAGES];

		for i in 0..image_count {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Submit Sync"), true);
			submit_synchronizers[i as usize] = synchronizer;
		}

		let mut native_images = [ImageHandle(!0u64); MAX_SWAPCHAIN_IMAGES];
		let native_resource_uses = uses
			| if native_image_usage.contains(vk::ImageUsageFlags::TRANSFER_DST) {
				crate::Uses::TransferDestination
			} else {
				crate::Uses::empty()
			};

		for (i, vk_image) in vk_images.iter().enumerate() {
			let previous = if i > 0 { Some(native_images[i - 1]) } else { None };
			native_images[i] = self.create_swapchain_image(*vk_image, crate::Formats::BGRAsRGB, native_resource_uses, previous);
		}

		let mut images = native_images;

		if uses_proxy_images {
			let proxy_extent = Extent::rectangle(extent.width, extent.height);
			let proxy_uses = uses | crate::Uses::TransferSource | crate::Uses::TransferDestination;

			for i in 0..image_count as usize {
				let previous = if i > 0 { Some(images[i - 1]) } else { None };
				images[i] = self.create_image_internal(
					None,
					previous,
					Some("Swapchain Proxy Image"),
					crate::Formats::BGRAu8,
					crate::DeviceAccesses::DeviceOnly,
					None,
					proxy_extent,
					proxy_uses,
				);
			}
		}

		self.swapchains.push(Swapchain {
			surface: vk_surface,
			swapchain: vk_swapchain,
			acquire_synchronizers,
			submit_synchronizers,
			extent,
			images,
			native_images,
			uses_proxy_images,
			min_image_count,
			max_image_count: image_count,
			vk_present_mode,
		});

		swapchain_handle
	}

	fn get_swapchain_image(
		&self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> graphics_hardware_interface::ImageHandle {
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		graphics_hardware_interface::ImageHandle(swapchain.images[0].0)
	}

	fn get_image_data<'a>(&'a self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		let image = &self.images[texture_copy_handle.0 as usize];

		let pointer = image.pointer.unwrap();
		let size = image.size;

		if pointer.is_null() {
			panic!("Texture data was requested but texture has no memory associated.");
		}

		let slice = unsafe { std::slice::from_raw_parts::<'a, u8>(pointer, size) };

		slice
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u32,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> Frame<'a> {
		let frame_index = index;
		let sequence_index = (index % self.frames as u32) as u8;

		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = &self.synchronizers[synchronizer_handles[sequence_index as usize].0 as usize];

		let per_cycle_wait_ms = 1;
		let wait_warning_time_threshold = 8;
		let mut timeout_count = 0;

		loop {
			match unsafe {
				self.device
					.wait_for_fences(&[synchronizer.fence], true, per_cycle_wait_ms * 1000000)
			} {
				Ok(_) => break,
				Err(vk::Result::TIMEOUT) => {
					if timeout_count * per_cycle_wait_ms >= wait_warning_time_threshold && timeout_count % 500 == 0 {
						println!(
							"Stuck waiting for fences for {} ms at frame {index}. There is a potential issue with synchronization.",
							per_cycle_wait_ms * timeout_count
						);
					}
					timeout_count += 1;
					continue;
				}
				Err(_) => panic!("Failed to wait for fence"),
			}
		}

		unsafe {
			self.device.reset_fences(&[synchronizer.fence]).expect("No fence reset");
		}

		let frame_key = FrameKey {
			frame_index,
			sequence_index,
		};

		// Build lazy resources before the frame may need them
		self.process_tasks(frame_key.sequence_index);

		Frame::new(self, frame_key)
	}

	#[inline]
	fn start_frame_capture(&mut self) {
		#[cfg(debug_assertions)]
		self.debugger.start_frame_capture();
	}

	#[inline]
	fn end_frame_capture(&mut self) {
		#[cfg(debug_assertions)]
		self.debugger.end_frame_capture();
	}

	fn wait(&self) {
		unsafe {
			self.device.device_wait_idle().unwrap();
		}
	}
}

impl crate::device::DeviceCreate for Device {
	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		self.create_allocation_internal(size, None, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(
			None,
			buffer_size,
			vk::BufferUsageFlags::VERTEX_BUFFER
				| vk::BufferUsageFlags::INDEX_BUFFER
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, pointer) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

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
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
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
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
		});

		self.set_name(shader_module, name);

		Ok(handle)
	}

	fn create_descriptor_set_template(
		&mut self,
		name: Option<&str>,
		bindings: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
	) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		let bindings = bindings
			.iter()
			.map(|binding| {
				let b = vk::DescriptorSetLayoutBinding::default()
					.binding(binding.binding)
					.descriptor_type(match binding.descriptor_type {
						crate::descriptors::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
						crate::descriptors::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
						crate::descriptors::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
						crate::descriptors::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
						crate::descriptors::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
						crate::descriptors::DescriptorType::InputAttachment => vk::DescriptorType::INPUT_ATTACHMENT,
						crate::descriptors::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
						crate::descriptors::DescriptorType::AccelerationStructure => {
							vk::DescriptorType::ACCELERATION_STRUCTURE_KHR
						}
					})
					.descriptor_count(binding.descriptor_count)
					.stage_flags(binding.stages.into());

				assert_ne!(binding.descriptor_count, 0, "Descriptor count must be greater than 0.");

				let _ = if let Some(inmutable_samplers) = &binding.immutable_samplers {
					inmutable_samplers
						.iter()
						.map(|sampler| vk::Sampler::from_raw(sampler.0))
						.collect::<Vec<_>>()
				} else {
					Vec::new()
				};

				b
			})
			.collect::<Vec<_>>();

		let binding_flags = bindings
			.iter()
			.map(|binding| {
				if binding.descriptor_count > 1 {
					vk::DescriptorBindingFlags::PARTIALLY_BOUND
				} else {
					vk::DescriptorBindingFlags::empty()
				}
			})
			.collect::<Vec<_>>();

		let mut dslbfci = vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&binding_flags);

		let descriptor_set_layout_create_info = vk::DescriptorSetLayoutCreateInfo::default()
			.push_next(&mut dslbfci)
			.bindings(&bindings);

		let descriptor_set_layout = unsafe {
			self.device
				.create_descriptor_set_layout(&descriptor_set_layout_create_info, None)
				.expect("No descriptor set layout")
		};

		self.set_name(descriptor_set_layout, name);

		let handle = graphics_hardware_interface::DescriptorSetTemplateHandle(self.descriptor_sets_layouts.len() as u64);

		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings: bindings
				.iter()
				.map(|binding| (binding.descriptor_type, binding.descriptor_count))
				.collect::<Vec<_>>(),
			descriptor_set_layout,
		});

		handle
	}

	fn create_descriptor_binding(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		constructor: graphics_hardware_interface::BindingConstructor,
	) -> graphics_hardware_interface::DescriptorSetBindingHandle {
		let binding = constructor.descriptor_set_binding_template;
		let descriptor = constructor.descriptor;
		let frame_offset = constructor.frame_offset.map(i32::from);
		let array_element = constructor.array_element();

		let descriptor_type = match binding.descriptor_type {
			crate::descriptors::DescriptorType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
			crate::descriptors::DescriptorType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
			crate::descriptors::DescriptorType::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
			crate::descriptors::DescriptorType::CombinedImageSampler => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
			crate::descriptors::DescriptorType::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
			crate::descriptors::DescriptorType::InputAttachment => vk::DescriptorType::INPUT_ATTACHMENT,
			crate::descriptors::DescriptorType::Sampler => vk::DescriptorType::SAMPLER,
			crate::descriptors::DescriptorType::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
		};

		let descriptor_set_handles = DescriptorSetHandle(descriptor_set.0).get_all(&self.descriptor_sets);

		let mut next = None;

		for descriptor_set_handle in descriptor_set_handles.iter().rev() {
			let binding_handle = DescriptorSetBindingHandle(self.bindings.len() as u64);

			let created_binding = Binding {
				next,
				descriptor_set_handle: *descriptor_set_handle,
				descriptor_type,
				_count: binding.descriptor_count,
				index: binding.binding,
			};

			self.bindings.push(created_binding);

			next = Some(binding_handle);
		}

		let handle = graphics_hardware_interface::DescriptorSetBindingHandle(next.expect("No next binding").0);

		let mut descriptor_write = crate::descriptors::Write::new(handle, descriptor);
		descriptor_write.array_element = array_element;
		descriptor_write.frame_offset = frame_offset;

		self.add_task_to_all_frames(Tasks::UpdateDescriptor { descriptor_write });

		handle
	}

	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		descriptor_set_layout_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		let pool_sizes = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize]
			.bindings
			.iter()
			.map(|(descriptor_type, descriptor_count)| {
				vk::DescriptorPoolSize::default()
					.ty(*descriptor_type)
					.descriptor_count(descriptor_count * self.frames as u32)
			})
			.collect::<Vec<_>>();

		let descriptor_pool_create_info = vk::DescriptorPoolCreateInfo::default()
			.max_sets(self.frames as _)
			.pool_sizes(&pool_sizes);

		let descriptor_pool = unsafe {
			self.device
				.create_descriptor_pool(&descriptor_pool_create_info, None)
				.expect("No descriptor pool")
		};
		self.descriptor_pools.push(descriptor_pool);

		let descriptor_set_layout = self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize].descriptor_set_layout;

		let descriptor_set_layouts = vec![descriptor_set_layout; self.frames as usize];

		let descriptor_set_allocate_info = vk::DescriptorSetAllocateInfo::default()
			.descriptor_pool(descriptor_pool)
			.set_layouts(&descriptor_set_layouts);

		let descriptor_sets = unsafe {
			self.device
				.allocate_descriptor_sets(&descriptor_set_allocate_info)
				.expect("No descriptor set")
		};

		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<DescriptorSetHandle> = None;

		for descriptor_set in descriptor_sets {
			let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);

			self.descriptor_sets.push(DescriptorSet {
				next: None,
				descriptor_set,
				descriptor_set_layout: *descriptor_set_layout_handle,
			});

			if let Some(previous_handle) = previous_handle {
				self.descriptor_sets[previous_handle.0 as usize].next = Some(handle);
			}

			self.set_name(descriptor_set, name);

			previous_handle = Some(handle);
		}

		handle
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_pipeline(builder)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_parameter = builder.shader;
		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);

		let mut specialization_map_entries = Vec::with_capacity(48);

		for specialization_map_entry in shader_parameter.specialization_map {
			// TODO: accumulate offset
			match specialization_map_entry.get_type().as_str() {
				"vec2f" => {
					for i in 0..2 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec3f" => {
					for i in 0..3 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec4f" => {
					for i in 0..4 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
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

		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.stage(
				vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					.specialization_info(&specialization_info),
			)
			.layout(pipeline_layout.pipeline_layout)];

		let pipeline_handle = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("No compute pipeline")[0]
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		let resource_access = shader
			.shader_binding_descriptors
			.iter()
			.map(|descriptor| {
				(
					(descriptor.set, descriptor.binding),
					(crate::Stages::COMPUTE, descriptor.access),
				)
			})
			.collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: HashMap::new(),
			resource_access,
		});

		handle
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline_layout_handle = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let shaders = builder.shaders;
		let mut groups = Vec::with_capacity(1024);

		let stages = shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];

				vk::PipelineShaderStageCreateInfo::default()
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		for (i, shader) in shaders.iter().enumerate() {
			match shader.stage {
				crate::ShaderTypes::RayGen | crate::ShaderTypes::Miss | crate::ShaderTypes::Callable => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
							.general_shader(i as u32)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::ClosestHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(i as u32)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::AnyHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(i as u32)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::Intersection => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(i as u32),
					);
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
			let pipeline = self
				.ray_tracing_pipeline
				.create_ray_tracing_pipelines(
					vk::DeferredOperationKHR::null(),
					vk::PipelineCache::null(),
					&[create_info],
					None,
				)
				.expect("No ray tracing pipeline")[0];
			let handle_buffer = self
				.ray_tracing_pipeline
				.get_ray_tracing_shader_group_handles(pipeline, 0, groups.len() as u32, 32 * groups.len())
				.expect("Could not get ray tracing shader group handles");

			for (i, shader) in shaders.iter().enumerate() {
				let mut h = [0u8; 32];
				h.copy_from_slice(&handle_buffer[i * 32..(i + 1) * 32]);

				handles.insert(*shader.handle, h);
			}

			pipeline
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		let resource_access = shaders
			.iter()
			.map(|shader| {
				let shader = &self.shaders[shader.handle.0 as usize];

				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| ((descriptor.set, descriptor.binding), (shader.stage, descriptor.access)))
					.collect::<Vec<_>>()
			})
			.flatten()
			.collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: handles,
			resource_access,
		});

		handle
	}

	fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		let command_buffer_handle = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

		let queue = &self.queues[queue_handle.0 as usize];

		let command_buffers = (0..self.frames)
			.map(|_| {
				let _ = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

				let command_pool_create_info = vk::CommandPoolCreateInfo::default()
					.flags(vk::CommandPoolCreateFlags::TRANSIENT)
					.queue_family_index(queue.queue_family_index);

				let command_pool = unsafe {
					self.device
						.create_command_pool(&command_pool_create_info, None)
						.expect("No command pool")
				};

				let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
					.command_pool(command_pool)
					.level(vk::CommandBufferLevel::PRIMARY)
					.command_buffer_count(1);

				let command_buffers = unsafe {
					self.device
						.allocate_command_buffers(&command_buffer_allocate_info)
						.expect("No command buffer")
				};

				let command_buffer = command_buffers[0];

				self.set_name(command_buffer, name);

				CommandBufferInternal {
					vk_queue: queue.vk_queue,
					command_pool,
					command_buffer,
				}
			})
			.collect::<Vec<_>>();

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			frames: command_buffers,
		});

		command_buffer_handle
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
		let root_image_handle = self.create_image_internal(
			None,
			None,
			builder.name,
			builder.format,
			builder.device_accesses,
			builder.array_layers,
			builder.extent,
			builder.resource_uses,
		);
		let handle = graphics_hardware_interface::ImageHandle(root_image_handle.0);

		let instances = match builder.use_case {
			crate::UseCases::DYNAMIC => self.frames,
			crate::UseCases::STATIC => 1,
		};

		let mut previous = root_image_handle;
		for _ in 1..instances {
			previous = self.create_image_internal(
				None,
				Some(previous),
				builder.name,
				builder.format,
				builder.device_accesses,
				builder.array_layers,
				builder.extent,
				builder.resource_uses,
			);
		}

		#[cfg(debug_assertions)]
		{
			if let Some(name) = builder.name {
				self.names
					.insert(graphics_hardware_interface::Handle::Image(handle), name.to_string());
			}
		}

		handle
	}

	fn build_sampler(&mut self, builder: sampler::Builder) -> crate::SamplerHandle {
		let filtering_mode = match builder.filtering_mode {
			crate::FilteringModes::Closest => vk::Filter::NEAREST,
			crate::FilteringModes::Linear => vk::Filter::LINEAR,
		};

		let mip_map_filter = match builder.mip_map_mode {
			crate::FilteringModes::Closest => vk::SamplerMipmapMode::NEAREST,
			crate::FilteringModes::Linear => vk::SamplerMipmapMode::LINEAR,
		};

		let address_mode = match builder.addressing_mode {
			crate::SamplerAddressingModes::Repeat => vk::SamplerAddressMode::REPEAT,
			crate::SamplerAddressingModes::Mirror => vk::SamplerAddressMode::MIRRORED_REPEAT,
			crate::SamplerAddressingModes::Clamp => vk::SamplerAddressMode::CLAMP_TO_EDGE,
			crate::SamplerAddressingModes::Border { .. } => vk::SamplerAddressMode::CLAMP_TO_BORDER,
		};

		let reduction_mode = match builder.reduction_mode {
			crate::SamplingReductionModes::WeightedAverage => vk::SamplerReductionMode::WEIGHTED_AVERAGE,
			crate::SamplingReductionModes::Min => vk::SamplerReductionMode::MIN,
			crate::SamplingReductionModes::Max => vk::SamplerReductionMode::MAX,
		};

		graphics_hardware_interface::SamplerHandle(
			self.create_vulkan_sampler(
				filtering_mode,
				reduction_mode,
				mip_map_filter,
				address_mode,
				builder.anisotropy,
				builder.min_lod,
				builder.max_lod,
			)
			.as_raw(),
		)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

		let buffer_creation_result = self.create_vulkan_buffer(
			name,
			size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let buffer_handle = graphics_hardware_interface::BaseBufferHandle(self.buffers.len() as u64);

		self.buffers.push(Buffer {
			next: None,
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size: buffer_creation_result.size,
			device_address: address,
			pointer,
			uses: crate::Uses::empty(),
			access: crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		});

		buffer_handle
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
			});

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				&[max_instance_count],
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) =
			self.create_allocation_internal(buffer.size, buffer.memory_flags.into(), crate::DeviceAccesses::GpuWrite);

		let (..) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);

		let handle =
			graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer.resource,
			});

			self.set_name(handle, name);
		}

		handle
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let (geometry, primitive_count) = match &description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				vertex_position_encoding,
				triangle_count,
				index_format,
			} => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_format(match vertex_position_encoding {
								crate::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position format"),
							})
							.max_vertex(*vertex_count - 1)
							.index_type(match index_format {
								crate::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								crate::DataTypes::U16 => vk::IndexType::UINT16,
								crate::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							}),
					}),
				*triangle_count,
			),
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::AABBS)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default(),
					}),
				*transform_count,
			),
		};

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				&[primitive_count],
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer_descriptor = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_descriptor.size,
			buffer_descriptor.memory_flags.into(),
			crate::DeviceAccesses::GpuWrite,
		);

		let (..) = self.bind_vulkan_buffer_memory(&buffer_descriptor, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer_descriptor.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

		let handle =
			graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer_descriptor.resource,
			});
		}

		handle
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::BufferHandle::<T>(buffer_handle.0, std::marker::PhantomData::<T> {});

		return handle;
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(buffer_handle.0, std::marker::PhantomData::<T> {});

		if super::buffer::PERSISTENT_WRITE && builder.device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
			// The master buffer's existing staging buffer becomes the shared, persistent
			// CPU-writable source buffer. We create a new per-frame staging buffer for
			// frame 0 and store the source handle on the master buffer.

			let source_handle = self.buffers[buffer_handle.0 as usize]
				.staging
				.expect("CpuWrite dynamic buffer must have a staging buffer");

			// Create a new per-frame staging buffer for frame 0
			let frame0_staging = self.create_staging_buffer(builder.name, size);

			// Reassign: the master's staging now points to the new per-frame staging,
			// and source points to the original (persistent) CPU-writable buffer.
			self.buffers[buffer_handle.0 as usize].staging = Some(frame0_staging);
			self.buffers[buffer_handle.0 as usize].source = Some(source_handle);

			// Track this dynamic buffer for automatic per-frame memcpy
			self.persistent_write_dynamic_buffers.push(handle.into());

			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: Some(source_handle),
					}),
					Some(i),
				));
			}
		} else {
			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: None,
					}),
					Some(i),
				));
			}
		}

		handle
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));

		crate::DynamicImageHandle(handle.0)
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
}

struct WriteResult {
	descriptor_set_handle: DescriptorSetHandle,
	binding_index: u32,
	array_element: u32,
	descriptor: Descriptor,
	binding_handle: DescriptorSetBindingHandle,
}
