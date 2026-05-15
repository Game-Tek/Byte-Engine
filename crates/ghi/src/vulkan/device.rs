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

	memory_properties: vk::PhysicalDeviceMemoryProperties,
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

		// Build all requested queue family indices
		let queue_family_indices = queues
			.iter()
			.map(|(d, _)| {
				if d.r#type.is_empty() {
					return Err(
						"Failed to find a compatible queue family. The requested queue selection did not include any workload type.",
					);
				}

				if d.r#type.intersects(crate::types::WorkloadTypes::VIDEO) {
					return Err(
						"Failed to find a compatible queue family. Vulkan video queues are not exposed through this backend command-buffer path.",
					);
				}

				if d.r#type.intersects(crate::types::WorkloadTypes::IO) {
					return Err(
						"Failed to find a compatible queue family. Vulkan IO queues are not exposed through this backend command-buffer path.",
					);
				}

				let required_queue_flags = if d.r#type.intersects(crate::types::WorkloadTypes::RASTER) {
					vk::QueueFlags::GRAPHICS
				} else {
					vk::QueueFlags::empty()
				} | if d
					.r#type
					.intersects(crate::types::WorkloadTypes::COMPUTE | crate::types::WorkloadTypes::RAY_TRACING)
				{
					vk::QueueFlags::COMPUTE
				} else {
					vk::QueueFlags::empty()
				} | if d.r#type.intersects(crate::types::WorkloadTypes::TRANSFER) {
					vk::QueueFlags::TRANSFER
				} else {
					vk::QueueFlags::empty()
				};

				let queue_family_index = queue_family_properties
					.iter()
					.enumerate()
					.filter(|(_, info)| info.queue_flags.contains(required_queue_flags))
					.min_by_key(|(_, info)| info.queue_flags.as_raw().count_ones())
					.map(|(index, _)| index as u32)
					.ok_or(
						"Failed to find a compatible queue family. The requested workload requires queue flags that no queue family exposes.",
					)?;

				Ok(queue_family_index)
			})
			.collect::<Result<Vec<_>, _>>()?;

		// Fold duplicate queue family indices into a single queue create info per family
		let queue_create_infos =
			queue_family_indices
				.iter()
				.copied()
				.fold(Vec::new(), |mut queue_create_infos, queue_family_index| {
					if !queue_create_infos
						.iter()
						.any(|create_info: &vk::DeviceQueueCreateInfo<'_>| create_info.queue_family_index == queue_family_index)
					{
						queue_create_infos.push(
							vk::DeviceQueueCreateInfo::default()
								.queue_family_index(queue_family_index)
								.queue_priorities(&[1.0]),
						);
					}

					queue_create_infos
				});

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
			.zip(queue_family_indices.iter().copied())
			.enumerate()
			.map(|(index, ((_, queue_handle), queue_family_index))| {
				let vk_queue = unsafe { device.get_device_queue(queue_family_index, 0) };

				**queue_handle = Some(graphics_hardware_interface::QueueHandle(index as u64));

				StoredQueue {
					vk_queue,
					queue_family_index,
					_queue_index: 0,
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

		let swapchain_native_supports_formatless_storage_write =
			Self::format_supports_formatless_storage_write(&vk_instance, physical_device, vk::Format::B8G8R8A8_SRGB);
		let swapchain_proxy_supports_formatless_storage_write =
			Self::format_supports_formatless_storage_write(&vk_instance, physical_device, vk::Format::B8G8R8A8_UNORM);

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
		})
	}

	#[cfg(debug_assertions)]
	fn get_log_count(&self) -> u64 {
		use std::sync::atomic::Ordering;
		unsafe { &(*self.debug_data) }.error_count.load(Ordering::SeqCst)
	}
}

impl Drop for Device {
	fn drop(&mut self) {
		unsafe {
			self.device.device_wait_idle().expect("Failed to wait for device idle");

			self.device.destroy_device(None);
		}
	}
}

impl crate::device::Device for Device {
	type Context = Context;

	#[cfg(debug_assertions)]
	fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}

	fn create_context(self) -> Result<Self::Context, &'static str> {
		Context::new(self)
	}
}

impl Device {
	#[inline]
	pub(crate) fn start_frame_capture(&mut self) {
		#[cfg(debug_assertions)]
		self.debugger.start_frame_capture();
	}

	#[inline]
	pub(crate) fn end_frame_capture(&mut self) {
		#[cfg(debug_assertions)]
		self.debugger.end_frame_capture();
	}

	pub(crate) fn wait(&self) {
		unsafe {
			self.device.device_wait_idle().unwrap();
		}
	}
}

use std::{borrow::Cow, num::NonZeroU32, u64};

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
	DescriptorSet, DescriptorSetLayout, Image, MemoryBackedResourceCreationResult, Mesh, Pipeline, PipelineLayout,
	PipelineLayoutKey, Shader, Swapchain, Synchronizer, TransitionState, MAX_FRAMES_IN_FLIGHT,
};
use crate::vulkan::utils::extent_into_vk_extent;
use crate::vulkan::StoredQueue;
use crate::PrivateHandles as Handles;
use crate::{
	binding::DescriptorSetBindingHandle,
	descriptors::DescriptorSetHandle,
	graphics_hardware_interface, image,
	render_debugger::RenderDebugger,
	sampler::{self, SamplerHandle},
	synchronizer::SynchronizerHandle,
	utils::StableVec,
	vulkan::{
		queue::Queue, BufferCopy, BuildBuffer, CommandBufferRecording, Context, Descriptor, DescriptorWrite, Descriptors,
		Frame, ImageCopy, ImageHandle, Instance, Task, Tasks, MAX_SWAPCHAIN_IMAGES,
	},
	window, FrameKey, HandleLike, MasterHandle as _, PrivateHandles, ResourceCollection, Size,
};
