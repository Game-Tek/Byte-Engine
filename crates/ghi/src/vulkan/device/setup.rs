use super::*;

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
		Ok(Self {
			device: inner.device.clone(),
			descriptor_heap_properties: inner.descriptor_heap_properties,
			inner: Some(inner),
			shaders: Vec::new(),
		})
	}

	pub(crate) fn detached_with_resources(
		device: ash::Device,
		descriptor_heap_properties: vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static>,
	) -> Self {
		Self {
			inner: None,
			device,
			descriptor_heap_properties,
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

		let physical_devices = unsafe { vk_instance.enumerate_physical_devices() }
			.or(Err("Failed to enumerate physical devices"))?
			.into_iter()
			.filter(|&physical_device| {
				settings.gpu.is_none_or(|gpu_name| {
					let properties = unsafe { vk_instance.get_physical_device_properties(physical_device) };
					properties.device_name_as_c_str().ok().and_then(|name| name.to_str().ok()) == Some(gpu_name)
				})
			})
			.collect::<Vec<_>>();

		if settings.gpu.is_some() && physical_devices.is_empty() {
			return Err("Failed to find physical device");
		}

		// Prefer the best-scoring suitable device. When none is suitable, report why the best-scoring one was rejected.
		let (physical_device, suitability) = physical_devices
			.into_iter()
			.map(|physical_device| {
				let suitability = Self::check_physical_device(vk_instance, physical_device, &settings);
				(physical_device, suitability)
			})
			.max_by_key(|&(physical_device, suitability)| {
				(suitability.is_ok(), Self::physical_device_score(vk_instance, physical_device))
			})
			.ok_or("Failed to choose a best physical device")?;
		suitability?;

		let queue_family_properties = unsafe { vk_instance.get_physical_device_queue_family_properties(physical_device) };

		let queue_family_indices = queues
			.iter()
			.map(|(selection, _)| {
				use crate::types::WorkloadTypes;

				let workloads = selection.r#type;
				if workloads.is_empty() {
					return Err(
						"Failed to find a compatible queue family. The requested queue selection did not include any workload type.",
					);
				}
				if workloads.intersects(WorkloadTypes::VIDEO) {
					return Err(
						"Failed to find a compatible queue family. Vulkan video queues are not exposed through this backend command-buffer path.",
					);
				}
				if workloads.intersects(WorkloadTypes::IO) {
					return Err(
						"Failed to find a compatible queue family. Vulkan IO queues are not exposed through this backend command-buffer path.",
					);
				}

				let compute_workloads = WorkloadTypes::COMPUTE | WorkloadTypes::RAY_TRACING;
				let required_queue_flags = flag_if(workloads.intersects(WorkloadTypes::RASTER), vk::QueueFlags::GRAPHICS)
					| flag_if(workloads.intersects(compute_workloads), vk::QueueFlags::COMPUTE)
					| flag_if(workloads.intersects(WorkloadTypes::TRANSFER), vk::QueueFlags::TRANSFER);

				queue_family_properties
					.iter()
					.enumerate()
					.filter(|(_, info)| info.queue_flags.contains(required_queue_flags))
					.min_by_key(|(_, info)| info.queue_flags.as_raw().count_ones())
					.map(|(index, _)| index as u32)
					.ok_or(
						"Failed to find a compatible queue family. The requested workload requires queue flags that no queue family exposes.",
					)
			})
			.collect::<Result<Vec<_>, _>>()?;

		// Requests that resolve to the same family share one Vulkan queue, created in order of first use.
		let mut queue_families = Vec::new();
		for &queue_family_index in &queue_family_indices {
			if !queue_families.contains(&queue_family_index) {
				queue_families.push(queue_family_index);
			}
		}

		let queue_create_infos = queue_families
			.iter()
			.map(|&queue_family_index| {
				vk::DeviceQueueCreateInfo::default()
					.queue_family_index(queue_family_index)
					.queue_priorities(&[1.0])
			})
			.collect::<Vec<_>>();

		let mut descriptor_heap_properties = vk::PhysicalDeviceDescriptorHeapPropertiesEXT::default();
		let mut physical_device_properties = vk::PhysicalDeviceProperties2::default().push(&mut descriptor_heap_properties);
		unsafe { vk_instance.get_physical_device_properties2(physical_device, &mut physical_device_properties) };

		let available_device_extensions = available_device_extensions(vk_instance, physical_device)?;
		let mut device_extension_names = required_device_extensions(&settings)
			.into_iter()
			.map(|(name, _)| name.as_ptr())
			.collect::<Vec<_>>();

		// Implementations that expose the portability subset require applications to enable it. ash only names this
		// provisional extension behind a feature flag, so it is spelled out here.
		const PORTABILITY_SUBSET: &std::ffi::CStr = c"VK_KHR_portability_subset";
		if has_extension(&available_device_extensions, PORTABILITY_SUBSET) {
			device_extension_names.push(PORTABILITY_SUBSET.as_ptr());
		}

		let mut features = DeviceFeatures::default();
		for (_, feature) in feature_requirements(&settings) {
			*feature(&mut features) = vk::TRUE;
		}
		let mut enabled_features = features.chain(&settings);

		// SAFETY: The chain links only live structures of `features`, and its last structure is writable.
		let device_create_info = unsafe { vk::DeviceCreateInfo::default().extend(&mut enabled_features) }
			.queue_create_infos(&queue_create_infos)
			.enabled_extension_names(&device_extension_names);

		let device = unsafe { vk_instance.create_device(physical_device, &device_create_info, None) }
			.map_err(|result| crate::vulkan::instance::creation_error(result, "Failed to create a device"))?;

		// Multiple GHI queue requests can resolve to the same Vulkan queue, so they must share one lock.
		// The context wraps each distinct queue in one mutex (see `Context::vk_queues`). This mutex is a temporary external
		// synchronization fix; prefer internally synchronized Vulkan queues when available.
		let vk_queues = queue_families
			.iter()
			.map(|&family| unsafe { device.get_device_queue(family, 0) })
			.collect::<Vec<_>>();
		let queues = queues
			.iter_mut()
			.zip(queue_family_indices)
			.enumerate()
			.map(|(index, ((_, queue_handle), queue_family_index))| {
				**queue_handle = Some(graphics_hardware_interface::QueueHandle(index as u64));
				let vk_queue_index = queue_families
					.iter()
					.position(|&family| family == queue_family_index)
					.unwrap();
				StoredQueue {
					vk_queue_index,
					queue_family_index,
				}
			})
			.collect();

		let supports_formatless_storage_write =
			|format| Self::format_supports_formatless_storage_write(vk_instance, physical_device, format);

		Ok(InnerDevice {
			debug_utils: settings
				.validation
				.then(|| ash::ext::debug_utils::Device::load(vk_instance, &device)),
			debug_data: super::DebugDataRef::new(&instance.debug_data),
			vk_queues,
			physical_device,
			swapchain: ash::khr::swapchain::Device::load(vk_instance, &device),
			surface: ash::khr::surface::Instance::load(vk_entry, vk_instance),
			acceleration_structure: ash::khr::acceleration_structure::Device::load(vk_instance, &device),
			ray_tracing_pipeline: ash::khr::ray_tracing_pipeline::Device::load(vk_instance, &device),
			mesh_shading: ash::ext::mesh_shader::Device::load(vk_instance, &device),
			descriptor_heap: ash::ext::descriptor_heap::Device::load(vk_instance, &device),
			descriptor_heap_properties,
			surface_capabilities: ash::khr::get_surface_capabilities2::Instance::load(vk_entry, vk_instance),
			wayland_surface: ash::khr::wayland_surface::Instance::load(vk_entry, vk_instance),
			memory_properties: unsafe { vk_instance.get_physical_device_memory_properties(physical_device) },
			queues,
			settings,
			swapchain_native_supports_formatless_storage_write: supports_formatless_storage_write(vk::Format::B8G8R8A8_SRGB),
			swapchain_proxy_supports_formatless_storage_write: supports_formatless_storage_write(vk::Format::B8G8R8A8_UNORM),
			device,
		})
	}
}

impl InnerDevice {
	/// Checks every capability the device must offer, returning the first one that is missing.
	fn check_physical_device(
		vk_instance: &ash::Instance,
		physical_device: vk::PhysicalDevice,
		settings: &crate::device::Features,
	) -> Result<(), &'static str> {
		let properties = unsafe { vk_instance.get_physical_device_properties(physical_device) };
		if properties.api_version < vk::API_VERSION_1_4 {
			return Err(
				"Vulkan 1.4 is unavailable. The most likely cause is that the GPU driver predates Vulkan 1.4, which descriptor heaps require.",
			);
		}

		let memory_properties = unsafe { vk_instance.get_physical_device_memory_properties(physical_device) };
		if memory_properties.memory_heaps[..memory_properties.memory_heap_count as usize]
			.iter()
			.any(|heap| heap.size == 0)
		{
			return Err("Vulkan device reports an empty memory heap. The most likely cause is a misconfigured virtual GPU.");
		}

		let available_extensions = available_device_extensions(vk_instance, physical_device)?;
		if let Some((_, error)) = required_device_extensions(settings)
			.into_iter()
			.find(|(name, _)| !has_extension(&available_extensions, name))
		{
			return Err(error);
		}

		// Only structures of available extensions may be queried, so features are checked after extensions.
		let mut available = DeviceFeatures::default();
		let mut chain = available.chain(settings);
		unsafe { vk_instance.get_physical_device_features2(physical_device, &mut chain) };
		available.core = chain.features;
		if let Some((error, _)) = feature_requirements(settings)
			.into_iter()
			.find(|(_, feature)| *feature(&mut available) == vk::FALSE)
		{
			return Err(error);
		}

		let mut subgroup_properties = vk::PhysicalDeviceSubgroupProperties::default();
		let mut subgroup_device_properties = vk::PhysicalDeviceProperties2::default().push(&mut subgroup_properties);
		unsafe { vk_instance.get_physical_device_properties2(physical_device, &mut subgroup_device_properties) };
		let required_subgroup_operations = vk::SubgroupFeatureFlags::BASIC | vk::SubgroupFeatureFlags::BALLOT;
		if !subgroup_properties.supported_stages.contains(vk::ShaderStageFlags::COMPUTE)
			|| !subgroup_properties
				.supported_operations
				.contains(required_subgroup_operations)
			|| !(1..=128).contains(&subgroup_properties.subgroup_size)
		{
			return Err(
				"Vulkan compute subgroups with ballot support are unavailable. The most likely cause is that the selected GPU or driver does not support the required Material Count subgroup operations.",
			);
		}

		Ok(())
	}

	fn physical_device_score(vk_instance: &ash::Instance, physical_device: vk::PhysicalDevice) -> u64 {
		match unsafe { vk_instance.get_physical_device_properties(physical_device) }.device_type {
			vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
			vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
			vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
			vk::PhysicalDeviceType::CPU => 100,
			_ => 0,
		}
	}

	fn format_supports_formatless_storage_write(
		vk_instance: &ash::Instance,
		physical_device: vk::PhysicalDevice,
		format: vk::Format,
	) -> bool {
		let mut format_properties_3 = vk::FormatProperties3::default();
		let mut format_properties_2 = vk::FormatProperties2::default().push(&mut format_properties_3);
		unsafe { vk_instance.get_physical_device_format_properties2(physical_device, format, &mut format_properties_2) };
		format_properties_3
			.optimal_tiling_features
			.contains(vk::FormatFeatureFlags2::STORAGE_IMAGE | vk::FormatFeatureFlags2::STORAGE_WRITE_WITHOUT_FORMAT)
	}
}

fn available_device_extensions(
	vk_instance: &ash::Instance,
	physical_device: vk::PhysicalDevice,
) -> Result<Vec<vk::ExtensionProperties>, &'static str> {
	unsafe { vk_instance.enumerate_device_extension_properties(physical_device) }.map_err(
		|_| "Failed to enumerate Vulkan device extensions. The most likely cause is that the GPU driver ran out of host memory.",
	)
}

fn has_extension(available_extensions: &[vk::ExtensionProperties], name: &std::ffi::CStr) -> bool {
	available_extensions
		.iter()
		.any(|extension| extension.extension_name_as_c_str() == Ok(name))
}

macro_rules! extension {
	($name:expr, $label:literal) => {
		(
			$name,
			concat!(
				"Vulkan device extension ",
				$label,
				" is unavailable. The most likely cause is that the GPU driver does not support it."
			),
		)
	};
}

/// Every device extension the backend enables for `settings`, with the error reported when it is missing.
fn required_device_extensions(settings: &crate::device::Features) -> Vec<(&'static std::ffi::CStr, &'static str)> {
	let mut extensions = vec![
		extension!(ash::khr::swapchain::NAME, "VK_KHR_swapchain"),
		extension!(ash::ext::swapchain_maintenance1::NAME, "VK_EXT_swapchain_maintenance1"),
		extension!(ash::ext::descriptor_heap::NAME, "VK_EXT_descriptor_heap"),
		extension!(ash::ext::shader_atomic_float::NAME, "VK_EXT_shader_atomic_float"),
	];

	if settings.mesh_shading {
		extensions.push(extension!(ash::ext::mesh_shader::NAME, "VK_EXT_mesh_shader"));
	}

	if settings.ray_tracing {
		extensions.extend([
			extension!(ash::khr::acceleration_structure::NAME, "VK_KHR_acceleration_structure"),
			extension!(ash::khr::deferred_host_operations::NAME, "VK_KHR_deferred_host_operations"),
			extension!(ash::khr::ray_tracing_pipeline::NAME, "VK_KHR_ray_tracing_pipeline"),
			extension!(ash::khr::ray_tracing_maintenance1::NAME, "VK_KHR_ray_tracing_maintenance1"),
		]);
	}

	extensions
}

/// The feature structures the backend enables, shared by the support query and device creation.
#[derive(Default)]
struct DeviceFeatures {
	core: vk::PhysicalDeviceFeatures,
	vulkan_11: vk::PhysicalDeviceVulkan11Features<'static>,
	vulkan_12: vk::PhysicalDeviceVulkan12Features<'static>,
	vulkan_13: vk::PhysicalDeviceVulkan13Features<'static>,
	descriptor_heap: vk::PhysicalDeviceDescriptorHeapFeaturesEXT<'static>,
	swapchain_maintenance1: vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT<'static>,
	shader_atomic_float: vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT<'static>,
	mesh_shader: vk::PhysicalDeviceMeshShaderFeaturesEXT<'static>,
	acceleration_structure: vk::PhysicalDeviceAccelerationStructureFeaturesKHR<'static>,
	ray_tracing_pipeline: vk::PhysicalDeviceRayTracingPipelineFeaturesKHR<'static>,
}

impl DeviceFeatures {
	/// Links the structures into one chain. Structures of extensions that `settings` leaves disabled stay out of it.
	fn chain(&mut self, settings: &crate::device::Features) -> vk::PhysicalDeviceFeatures2<'_> {
		let mut chain = vk::PhysicalDeviceFeatures2::default()
			.features(self.core)
			.push(&mut self.vulkan_11)
			.push(&mut self.vulkan_12)
			.push(&mut self.vulkan_13)
			.push(&mut self.descriptor_heap)
			.push(&mut self.swapchain_maintenance1)
			.push(&mut self.shader_atomic_float);

		if settings.mesh_shading {
			chain = chain.push(&mut self.mesh_shader);
		}

		if settings.ray_tracing {
			chain = chain
				.push(&mut self.acceleration_structure)
				.push(&mut self.ray_tracing_pipeline);
		}

		chain
	}
}

type FeatureField = fn(&mut DeviceFeatures) -> &mut vk::Bool32;

macro_rules! feature {
	($($field:ident).+) => {{
		fn field(features: &mut DeviceFeatures) -> &mut vk::Bool32 {
			&mut features.$($field).+
		}

		(
			concat!(
				"Vulkan device feature ",
				stringify!($($field).+),
				" is unavailable. The most likely cause is that the GPU or driver does not support it."
			),
			field as FeatureField,
		)
	}};
}

/// Every device feature the backend enables for `settings`, with the error reported when it is unsupported.
fn feature_requirements(settings: &crate::device::Features) -> Vec<(&'static str, FeatureField)> {
	let mut features = vec![
		feature!(core.shader_int16),
		feature!(core.shader_int64),
		feature!(core.shader_uniform_buffer_array_dynamic_indexing),
		feature!(core.shader_sampled_image_array_dynamic_indexing),
		feature!(core.shader_storage_buffer_array_dynamic_indexing),
		feature!(core.shader_storage_image_array_dynamic_indexing),
		feature!(core.shader_storage_image_read_without_format),
		feature!(core.shader_storage_image_write_without_format),
		feature!(core.texture_compression_bc),
		feature!(core.fill_mode_non_solid),
		feature!(vulkan_11.storage_buffer16_bit_access),
		feature!(vulkan_11.uniform_and_storage_buffer16_bit_access),
		feature!(vulkan_12.descriptor_indexing),
		feature!(vulkan_12.descriptor_binding_partially_bound),
		feature!(vulkan_12.descriptor_binding_variable_descriptor_count),
		feature!(vulkan_12.runtime_descriptor_array),
		feature!(vulkan_12.shader_sampled_image_array_non_uniform_indexing),
		feature!(vulkan_12.shader_storage_image_array_non_uniform_indexing),
		feature!(vulkan_12.scalar_block_layout),
		feature!(vulkan_12.buffer_device_address),
		feature!(vulkan_12.separate_depth_stencil_layouts),
		feature!(vulkan_12.shader_float16),
		feature!(vulkan_12.shader_int8),
		feature!(vulkan_12.storage_buffer8_bit_access),
		feature!(vulkan_12.uniform_and_storage_buffer8_bit_access),
		feature!(vulkan_12.vulkan_memory_model),
		feature!(vulkan_12.vulkan_memory_model_device_scope),
		feature!(vulkan_12.timeline_semaphore),
		feature!(vulkan_13.pipeline_creation_cache_control),
		feature!(vulkan_13.subgroup_size_control),
		feature!(vulkan_13.compute_full_subgroups),
		feature!(vulkan_13.synchronization2),
		feature!(vulkan_13.dynamic_rendering),
		feature!(vulkan_13.maintenance4),
		feature!(descriptor_heap.descriptor_heap),
		feature!(swapchain_maintenance1.swapchain_maintenance1),
		feature!(shader_atomic_float.shader_buffer_float32_atomics),
	];

	if settings.geometry_shader {
		features.push(feature!(core.geometry_shader));
	}

	if settings.mesh_shading {
		features.extend([feature!(mesh_shader.task_shader), feature!(mesh_shader.mesh_shader)]);
	}

	if settings.ray_tracing {
		features.extend([
			feature!(acceleration_structure.acceleration_structure),
			feature!(ray_tracing_pipeline.ray_tracing_pipeline),
			feature!(ray_tracing_pipeline.ray_traversal_primitive_culling),
		]);
	}

	features
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_requirement_names_a_distinct_feature() {
		let settings = crate::device::Features::new()
			.mesh_shading(true)
			.ray_tracing(true)
			.geometry_shader(true);
		let requirements = feature_requirements(&settings);
		let mut features = DeviceFeatures::default();

		// A field that is already enabled when its entry is reached appears in the table twice.
		for (error, field) in &requirements {
			assert!(*field(&mut features) == vk::FALSE, "{error} is listed twice");
			*field(&mut features) = vk::TRUE;
		}
	}

	#[test]
	fn optional_extensions_follow_settings() {
		let names = |settings: crate::device::Features| {
			required_device_extensions(&settings)
				.into_iter()
				.map(|(name, _)| name)
				.collect::<Vec<_>>()
		};

		let minimal = names(crate::device::Features::new().mesh_shading(false));
		assert!(!minimal.contains(&ash::ext::mesh_shader::NAME));
		assert!(!minimal.contains(&ash::khr::ray_tracing_pipeline::NAME));

		let full = names(crate::device::Features::new().mesh_shading(true).ray_tracing(true));
		assert!(full.contains(&ash::ext::mesh_shader::NAME));
		assert!(full.contains(&ash::khr::acceleration_structure::NAME));
	}
}
