use std::sync::atomic::{AtomicU64, Ordering};

use ash::vk::{self};
use crate::{graphics_hardware_interface, vulkan::DebugCallbackData};

pub struct Instance {
	pub(crate) instance: ash::Instance,
	pub(crate) entry: ash::Entry,

	pub(crate) debug_data: Box<DebugCallbackData>,

	debug_utils: Option<ash::ext::debug_utils::Instance>,
	debug_utils_messenger: Option<vk::DebugUtilsMessengerEXT>,
}

unsafe impl Send for Instance {}

impl Instance {
	pub fn new(settings: graphics_hardware_interface::Features) -> Result<Instance, &'static str> {
		let entry = ash::Entry::linked();

		let available_instance_layers = unsafe { entry.enumerate_instance_layer_properties().unwrap() };
		let available_instance_extensions = unsafe { entry.enumerate_instance_extension_properties(None).unwrap() };

		let is_instance_layer_available = |name: &str| {
			available_instance_layers.iter().any(|layer| {
				unsafe { std::ffi::CStr::from_ptr(layer.layer_name.as_ptr()).to_str().unwrap() == name }
			})
		};

		let is_instance_extension_available = |name: &str| {
			available_instance_extensions.iter().any(|extension| {
				unsafe { std::ffi::CStr::from_ptr(extension.extension_name.as_ptr()).to_str().unwrap() == name }
			})
		};

		let application_info = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 3, 0));

		let mut layer_names = Vec::new();

		if settings.validation {
			if is_instance_layer_available("VK_LAYER_KHRONOS_validation") {
				layer_names.push(std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap().as_ptr());
			} else {
				println!("Warning: VK_LAYER_KHRONOS_validation is not available");
			}
		}

		if settings.api_dump {
			if is_instance_layer_available("VK_LAYER_LUNARG_api_dump") {
				layer_names.push(std::ffi::CStr::from_bytes_with_nul(b"VK_LAYER_LUNARG_api_dump\0").unwrap().as_ptr());
			} else {
				println!("Warning: VK_LAYER_LUNARG_api_dump is not available");
			}
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

		let mut debug_data = Box::new(DebugCallbackData {
			error_count: AtomicU64::new(0),
			error_log_function: settings.debug_log_function.unwrap_or(|message| { println!("{}", message); }),
		});

		let (debug_utils, debug_utils_messenger) = if settings.validation {
			let debug_utils = ash::ext::debug_utils::Instance::new(&entry, &instance);

			let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
				.message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::INFO | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,)
				.message_type(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,)
				.pfn_user_callback(Some(vulkan_debug_utils_callback))
				.user_data(debug_data.as_mut() as *mut DebugCallbackData as *mut std::ffi::c_void)
			;

			let debug_utils_messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None).or(Err("Failed to enable debug utils messanger"))? };

			(Some(debug_utils), Some(debug_utils_messenger))
		} else {
			(None, None)
		};

		Ok(Instance {
			instance,
			entry,

			debug_utils,
			debug_utils_messenger,

			debug_data,
		})
	}

	pub fn create_device(&mut self, settings: graphics_hardware_interface::Features) -> Result<crate::vulkan::Device, &'static str> {
		crate::vulkan::Device::new(settings, &self)
	}
}

impl Drop for Instance {
	fn drop(&mut self) {
		unsafe {
			if let Some(debug_utils) = &self.debug_utils {
				if let Some(messenger) = self.debug_utils_messenger {
					debug_utils.destroy_debug_utils_messenger(messenger, None);
				}
			}

			self.instance.destroy_instance(None);
		}
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
			user_data.error_count.fetch_add(10, Ordering::SeqCst);
		}
		_ => {}
	}

	vk::FALSE
}
