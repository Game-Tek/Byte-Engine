/// The `Device` struct carries the selected Vulkan device until a rendering context is created.
pub struct Device {
	pub inner: Option<InnerDevice>,
	device: ash::Device,
	descriptor_set_layouts: Vec<DescriptorSetLayout>,
	shaders: Vec<crate::vulkan::Shader>,
}

// Vulkan device handles are thread-safe, and detached resource creation uses `Device` with no `InnerDevice`.
unsafe impl Send for Device {}

impl Device {
	pub fn new(
		settings: crate::device::Features,
		instance: &Instance,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Self, &'static str> {
		let inner = InnerDevice::new(settings, instance, queues)?;
		let device = inner.device.clone();

		Ok(Self {
			inner: Some(inner),
			device,
			descriptor_set_layouts: Vec::new(),
			shaders: Vec::new(),
		})
	}

	pub(crate) fn detached(device: ash::Device) -> Self {
		Self {
			inner: None,
			device,
			descriptor_set_layouts: Vec::new(),
			shaders: Vec::new(),
		}
	}

	pub(crate) fn detached_with_resources(device: ash::Device, descriptor_set_layouts: Vec<DescriptorSetLayout>) -> Self {
		Self {
			inner: None,
			device,
			descriptor_set_layouts,
			shaders: Vec::with_capacity(64),
		}
	}
}

impl InnerDevice {
	pub fn new(
		settings: crate::device::Features,
		instance: &Instance,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Self, &'static str> {
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
								// .flags(vk::DeviceQueueCreateFlags::from_raw(0x00000004)) // VK_DEVICE_QUEUE_CREATE_INTERNALLY_SYNCHRONIZED_BIT_KHR
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

		let _physical_device_features = unsafe { vk_instance.get_physical_device_features(physical_device) };

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

		// Multiple GHI queue requests can resolve to the same Vulkan queue, so they must share one lock.
		// This mutex is a temporary external synchronization fix; prefer internally synchronized Vulkan queues when available.
		let mut shared_queues = Vec::<(u32, std::sync::Arc<std::sync::Mutex<vk::Queue>>)>::new();
		let queues = queues
			.iter_mut()
			.zip(queue_family_indices.iter().copied())
			.enumerate()
			.map(|(index, ((_, queue_handle), queue_family_index))| {
				let vk_queue = if let Some((_, vk_queue)) = shared_queues
					.iter()
					.find(|(stored_queue_family_index, _)| *stored_queue_family_index == queue_family_index)
				{
					vk_queue.clone()
				} else {
					let vk_queue = std::sync::Arc::new(std::sync::Mutex::new(unsafe {
						device.get_device_queue(queue_family_index, 0)
					}));
					shared_queues.push((queue_family_index, vk_queue.clone()));
					vk_queue
				};

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

		Ok(InnerDevice {
			debug_utils,
			debug_data: instance.debug_data.as_ref() as *const DebugCallbackData,

			memory_properties,
			queues,
			settings,
			swapchain_native_supports_formatless_storage_write,
			swapchain_proxy_supports_formatless_storage_write,

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
			// #[cfg(debug_assertions)]
			// debugger: RenderDebugger::new(),
		})
	}

	fn format_supports_formatless_storage_write(
		vk_instance: &ash::Instance,
		physical_device: vk::PhysicalDevice,
		format: vk::Format,
	) -> bool {
		let mut format_properties_3 = vk::FormatProperties3::default();
		let mut format_properties_2 = vk::FormatProperties2::default().push_next(&mut format_properties_3);

		unsafe {
			vk_instance.get_physical_device_format_properties2(physical_device, format, &mut format_properties_2);
		}

		format_properties_3
			.optimal_tiling_features
			.contains(vk::FormatFeatureFlags2::STORAGE_IMAGE | vk::FormatFeatureFlags2::STORAGE_WRITE_WITHOUT_FORMAT)
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

	pub fn build_swapchain(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: crate::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> (
		vk::SurfaceKHR,
		vk::PresentModeKHR,
		u32,
		vk::Extent2D,
		crate::Formats,
		crate::Formats,
		vk::ImageUsageFlags,
		bool,
		vk::ImageUsageFlags,
		vk::SwapchainKHR,
	) {
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
		let proxy_format = crate::Formats::BGRAu8;

		let requested_image_usage = into_vk_image_usage_flags(uses, format);
		let supported_image_usage = vk_surface_capabilities.supported_usage_flags;
		let uses_proxy_images = self.swapchain_needs_proxy(supported_image_usage, requested_image_usage, uses);

		let native_image_usage = if uses_proxy_images {
			self.validate_swapchain_proxy_format(uses);

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
		(
			vk_surface,
			vk_present_mode,
			min_image_count,
			extent,
			format,
			proxy_format,
			supported_image_usage,
			uses_proxy_images,
			native_image_usage,
			vk_swapchain,
		)
	}

	fn swapchain_needs_proxy(
		&self,
		supported_usage_flags: vk::ImageUsageFlags,
		requested_usage_flags: vk::ImageUsageFlags,
		uses: crate::Uses,
	) -> bool {
		!supported_usage_flags.contains(requested_usage_flags)
			|| uses.contains(crate::Uses::Storage) && !self.swapchain_native_supports_formatless_storage_write
	}

	fn validate_swapchain_proxy_format(&self, uses: crate::Uses) {
		if uses.contains(crate::Uses::Storage) && !self.swapchain_proxy_supports_formatless_storage_write {
			panic!(
				"Failed to create swapchain storage proxy image. The most likely cause is that the selected Vulkan device does not support storage writes without format for the swapchain proxy format."
			);
		}
	}

	#[cfg(any(debug_assertions, test))]
	fn get_log_count(&self) -> u64 {
		use std::sync::atomic::Ordering;
		unsafe { &(*self.debug_data) }.error_count.load(Ordering::SeqCst)
	}

	#[cfg(any(debug_assertions, test))]
	pub(crate) fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}
}

#[derive(Clone)]
pub struct InnerDevice {
	pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,

	debug_data: *const DebugCallbackData,

	pub(crate) physical_device: vk::PhysicalDevice,
	pub(super) device: ash::Device,
	pub(super) swapchain: ash::khr::swapchain::Device,
	pub(super) surface: ash::khr::surface::Instance,
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

	// #[cfg(debug_assertions)]
	// debugger: RenderDebugger, // DISABLED FOR NOW
	pub(super) memory_properties: vk::PhysicalDeviceMemoryProperties,
	pub(super) queues: Vec<StoredQueue>,
	pub(super) settings: crate::device::Features,
	pub(super) swapchain_native_supports_formatless_storage_write: bool,
	pub(super) swapchain_proxy_supports_formatless_storage_write: bool,
}

// TODO: re-implement when we use a Box
// impl Drop for InnerDevice {
// 	fn drop(&mut self) {
// 		unsafe {
// 			self.device.device_wait_idle().expect("Failed to wait for device idle");
// 			self.device.destroy_device(None);
// 		}
// 	}
// }

impl std::ops::Deref for InnerDevice {
	type Target = ash::Device;

	fn deref(&self) -> &Self::Target {
		&self.device
	}
}

impl crate::device::Device for Device {
	type Context = Context;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type Image = FactoryImage;
	type Sampler = FactorySampler;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		self.inner.as_ref().is_some_and(InnerDevice::has_errors)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Context::new(&self)
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<crate::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
				if spirv.as_ptr().is_aligned_to(std::mem::align_of::<u32>()) {
					Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
				} else {
					let mut words = Vec::with_capacity(spirv.len() / 4);
					for chunk in spirv.chunks_exact(4) {
						words.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
					}
					Cow::Owned(words)
				}
			}
			crate::shader::Sources::DXIL(_)
			| crate::shader::Sources::HLSL { .. }
			| crate::shader::Sources::MTL { .. }
			| crate::shader::Sources::MTLB { .. } => return Err(()),
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);
		let shader_module = unsafe {
			self.device
				.create_shader_module(&shader_module_create_info, None)
				.map_err(|_| ())?
		};
		let handle = crate::ShaderHandle(self.shaders.len() as u64);

		self.shaders.push(crate::vulkan::Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
		});

		Ok(handle)
	}

	fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		RasterPipeline
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		self.create_compute_pipeline_with_resources(builder, &self.descriptor_set_layouts, &self.shaders)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> Self::Image {
		FactoryImage {
			name: crate::debug_name(builder.name),
			extent: builder.extent,
			format: builder.format,
			resource_uses: builder.resource_uses,
			device_accesses: builder.device_accesses,
			use_case: builder.use_case,
			array_layers: builder.array_layers,
		}
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> Self::Sampler {
		FactorySampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		}
	}
}

impl InnerDevice {
	#[inline]
	pub(crate) fn start_frame_capture(&mut self) {
		// #[cfg(debug_assertions)]
		// self.debugger.start_frame_capture();
	}

	#[inline]
	pub(crate) fn end_frame_capture(&mut self) {
		// #[cfg(debug_assertions)]
		// self.debugger.end_frame_capture();
	}

	pub(crate) fn wait(&self) {
		unsafe {
			self.device.device_wait_idle().unwrap();
		}
	}

	/// Creates a Vulkan buffer and reports the memory requirements needed to bind it.
	pub(super) fn create_vulkan_buffer(
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

	/// Creates a Vulkan image and reports the memory requirements needed to bind it.
	pub(super) fn create_vulkan_texture(
		&self,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		mip_levels: u32,
		array_layers: Option<NonZeroU32>,
	) -> MemoryBackedResourceCreationResult<vk::Image> {
		let image_create_info = vk::ImageCreateInfo::default()
			.image_type(image_type_from_extent(extent).expect("Failed to get VkImageType from extent"))
			.format(to_format(format))
			.extent(extent_into_vk_extent(extent))
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

	/// Creates a Vulkan sampler from the resolved sampler builder parameters.
	pub(super) fn create_vulkan_sampler(
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

		unsafe { self.device.create_sampler(&sampler_create_info, None).expect("No sampler") }
	}

	/// Creates a Vulkan fence with the requested initial signal state.
	pub(super) fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
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

	/// Assigns a Vulkan debug name when debug utilities are available.
	pub(super) fn set_name<T: vk::Handle>(&self, handle: T, name: Option<&str>) {
		#[cfg(debug_assertions)]
		if let Some(name) = name {
			let name = std::ffi::CString::new(name).unwrap();
			let name = name.as_c_str();
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

	/// Creates a Vulkan semaphore and assigns its debug name.
	pub(super) fn create_vulkan_semaphore(&self, name: Option<&str>, _: bool) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default();
		let handle = unsafe {
			self.device
				.create_semaphore(&semaphore_create_info, None)
				.expect("No semaphore")
		};

		self.set_name(handle, name);

		handle
	}

	/// Creates a Vulkan image view for images with view-capable usage flags.
	pub(super) fn create_vulkan_image_view(
		&self,
		name: Option<&str>,
		texture: &vk::Image,
		format: crate::Formats,
		usage: vk::ImageUsageFlags,
		_mip_levels: u32,
		base_layer: u32,
		layer_count: Option<NonZeroU32>,
	) -> vk::ImageView {
		if !Self::image_usage_allows_views(usage) {
			return vk::ImageView::null();
		}

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

	pub(super) fn image_usage_allows_views(usage: vk::ImageUsageFlags) -> bool {
		usage.intersects(
			vk::ImageUsageFlags::SAMPLED
				| vk::ImageUsageFlags::STORAGE
				| vk::ImageUsageFlags::COLOR_ATTACHMENT
				| vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
				| vk::ImageUsageFlags::TRANSIENT_ATTACHMENT
				| vk::ImageUsageFlags::INPUT_ATTACHMENT
				| vk::ImageUsageFlags::FRAGMENT_SHADING_RATE_ATTACHMENT_KHR
				| vk::ImageUsageFlags::FRAGMENT_DENSITY_MAP_EXT,
		)
	}
}

/// The `ComputePipeline` struct carries a Vulkan compute pipeline before it has a public GHI handle.
pub struct ComputePipeline {
	pub(crate) pipeline: vk::Pipeline,
	pub(crate) layout: crate::vulkan::PipelineLayout,
	pub(crate) shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	pub(crate) resource_access: Vec<((u32, u32), (crate::Stages, crate::AccessPolicies))>,
}

unsafe impl Send for ComputePipeline {}

/// The `RasterPipeline` struct marks detached Vulkan raster pipelines for future support.
pub struct RasterPipeline;

/// The `FactoryImage` struct carries Vulkan image parameters until a context interns them.
pub struct FactoryImage {
	pub(crate) name: Option<String>,
	pub(crate) extent: Extent,
	pub(crate) format: crate::Formats,
	pub(crate) resource_uses: crate::Uses,
	pub(crate) device_accesses: crate::DeviceAccesses,
	pub(crate) use_case: crate::UseCases,
	pub(crate) array_layers: Option<NonZeroU32>,
}

/// The `FactorySampler` struct carries Vulkan sampler parameters until a context interns them.
pub struct FactorySampler {
	pub(crate) filtering_mode: crate::FilteringModes,
	pub(crate) reduction_mode: crate::SamplingReductionModes,
	pub(crate) mip_map_mode: crate::FilteringModes,
	pub(crate) addressing_mode: crate::SamplerAddressingModes,
	pub(crate) anisotropy: Option<f32>,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

impl Device {
	/// Creates a detached compute pipeline using context-owned shader modules and descriptor set layouts without storing them on the detached device.
	pub(crate) fn create_compute_pipeline_with_resources(
		&self,
		builder: crate::pipelines::compute::Builder,
		descriptor_set_layouts: &[DescriptorSetLayout],
		shaders: &[crate::vulkan::Shader],
	) -> ComputePipeline {
		let layout = self.build_pipeline_layout(
			builder.descriptor_set_templates,
			builder.push_constant_ranges,
			descriptor_set_layouts,
		);
		let shader_parameter = builder.shader;
		let shader = &shaders[shader_parameter.handle.0 as usize];
		let (specialization_entries_buffer, specialization_map_entries) =
			build_specialization_entries(shader_parameter.specialization_map);
		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.stage(
				vk::PipelineShaderStageCreateInfo::default()
					.stage(vk::ShaderStageFlags::COMPUTE)
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
					.specialization_info(&specialization_info),
			)
			.layout(layout.pipeline_layout)];
		let pipeline = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("Vulkan compute pipeline creation failed. The most likely cause is that shader specialization or pipeline layout creation failed.")[0]
		};
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
		let mut shader_handles = HashMap::default();
		shader_handles.insert(*shader_parameter.handle, [0; 32]);

		ComputePipeline {
			pipeline,
			layout,
			shader_handles,
			resource_access,
		}
	}

	fn build_pipeline_layout(
		&self,
		descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
		stored_descriptor_set_layouts: &[DescriptorSetLayout],
	) -> crate::vulkan::PipelineLayout {
		// Resolve template handles against the caller-provided layouts so the factory device stays stateless.
		let descriptor_set_layouts = descriptor_set_template_handles
			.iter()
			.map(|handle| stored_descriptor_set_layouts[handle.0 as usize].descriptor_set_layout)
			.collect::<Vec<_>>();
		let default_push_constant_range;
		let push_constant_ranges = if push_constant_ranges.is_empty() {
			default_push_constant_range = [crate::pipelines::PushConstantRange::new(0, 128)];
			default_push_constant_range.as_slice()
		} else {
			push_constant_ranges
		};
		let push_constant_stages = vk::ShaderStageFlags::VERTEX
			| vk::ShaderStageFlags::FRAGMENT
			| vk::ShaderStageFlags::COMPUTE
			| vk::ShaderStageFlags::MESH_EXT;
		let push_constant_ranges_vk = push_constant_ranges
			.iter()
			.map(|range| {
				vk::PushConstantRange::default()
					.stage_flags(push_constant_stages)
					.offset(range.offset)
					.size(range.size)
			})
			.collect::<Vec<_>>();
		let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default()
			.set_layouts(&descriptor_set_layouts)
			.push_constant_ranges(&push_constant_ranges_vk);
		let pipeline_layout = unsafe {
			self.device
				.create_pipeline_layout(&pipeline_layout_create_info, None)
				.expect("Vulkan detached pipeline layout creation failed. The most likely cause is that a descriptor set template handle was invalid.")
		};
		let descriptor_set_template_indices = descriptor_set_template_handles
			.iter()
			.enumerate()
			.map(|(index, handle)| (*handle, index as u32))
			.collect();

		crate::vulkan::PipelineLayout {
			pipeline_layout,
			descriptor_set_template_indices,
		}
	}
}

fn build_specialization_entries(
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
	let mut data = Vec::<u8>::with_capacity(256);
	let mut entries = Vec::with_capacity(48);

	for specialization_map_entry in specialization_map {
		let scalar_count = match specialization_map_entry.get_type().as_str() {
			"bool" | "u32" | "f32" => 1,
			"vec2f" => 2,
			"vec3f" => 3,
			"vec4f" => 4,
			_ => panic!("Unsupported Vulkan specialization constant type. The most likely cause is that the Vulkan backend was not updated for a new specialization entry type."),
		};
		let offset = data.len() as u32;
		for i in 0..scalar_count {
			entries.push(
				vk::SpecializationMapEntry::default()
					.constant_id(specialization_map_entry.get_constant_id() + i)
					.offset(offset + i * 4)
					.size(4),
			);
		}
		data.extend_from_slice(specialization_map_entry.get_data());
	}

	(data, entries)
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
