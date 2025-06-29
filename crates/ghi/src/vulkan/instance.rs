use ash::vk::{self};
use crate::graphics_hardware_interface;

pub struct Instance {
	instance: ash::Instance,
	entry: ash::Entry,
}

unsafe impl Send for Instance {}

impl Instance {
	pub fn new(settings: graphics_hardware_interface::Features) -> Result<Instance, &'static str> {
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

		if is_instance_extension_available(ash::khr::get_surface_capabilities2::NAME.to_str().unwrap()) {
			extension_names.push(ash::khr::get_surface_capabilities2::NAME.as_ptr());
		}

		if is_instance_extension_available(ash::ext::surface_maintenance1::NAME.to_str().unwrap()) {
			extension_names.push(ash::ext::surface_maintenance1::NAME.as_ptr());
		}

		if is_instance_extension_available(ash::ext::swapchain_maintenance1::NAME.to_str().unwrap()) {
			extension_names.push(ash::ext::swapchain_maintenance1::NAME.as_ptr());
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

		Ok(Instance {
			instance,
			entry,
		})
	}

	pub fn create_device(&mut self, settings: graphics_hardware_interface::Features) -> Result<crate::vulkan::Device, &'static str> {
		crate::vulkan::Device::new(settings, &self.entry, &self.instance)
	}
}

impl Drop for Instance {
	fn drop(&mut self) {
		unsafe {
			self.instance.destroy_instance(None);
		}
	}
}
