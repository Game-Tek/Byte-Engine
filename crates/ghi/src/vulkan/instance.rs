use std::{
	ffi::{CStr, c_char},
	sync::atomic::{AtomicU64, Ordering},
};

use ash::vk::{self, TaggedStructure as _};

use crate::{graphics_hardware_interface, vulkan::DebugCallbackData};

pub struct Instance {
	pub(crate) instance: ash::Instance,
	pub(crate) entry: ash::Entry,

	/// Boxed so the debug messenger's `pUserData` pointer and every [`DebugDataRef`](super::device::DebugDataRef) keep a stable address.
	pub(crate) debug_data: Box<DebugCallbackData>,

	debug_messenger: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
}

impl Instance {
	pub fn new(settings: crate::device::Features) -> Result<Instance, &'static str> {
		let entry = ash::Entry::linked();

		let available_layers = unsafe { entry.enumerate_instance_layer_properties().unwrap() };
		let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None).unwrap() };
		let has_extension = |name: &CStr| {
			available_extensions
				.iter()
				.any(|extension| extension.extension_name_as_c_str() == Ok(name))
		};

		let application_info = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 4, 0));

		let layer_names = enable_names(
			[
				(
					settings.validation,
					c"VK_LAYER_KHRONOS_validation",
					"VK_LAYER_KHRONOS_validation is not available",
				),
				(
					settings.api_dump,
					c"VK_LAYER_LUNARG_api_dump",
					"VK_LAYER_LUNARG_api_dump is not available",
				),
			],
			|name| available_layers.iter().any(|layer| layer.layer_name_as_c_str() == Ok(name)),
		)?;

		let mut extension_names = enable_names(
			[
				(true, ash::khr::surface::NAME, "VK_KHR_surface extension is not available"),
				(
					true,
					ash::khr::get_surface_capabilities2::NAME,
					"VK_KHR_get_surface_capabilities2 extension is not available",
				),
				(
					true,
					ash::ext::surface_maintenance1::NAME,
					"VK_EXT_surface_maintenance1 extension is not available",
				),
				(
					settings.validation,
					ash::ext::debug_utils::NAME,
					"VK_EXT_debug_utils extension is not available",
				),
			],
			has_extension,
		)?;

		// Unlike the extensions above, Wayland surface support is optional.
		if has_extension(ash::khr::wayland_surface::NAME) {
			extension_names.push(ash::khr::wayland_surface::NAME.as_ptr());
		}

		let mut enabled_validation_features = vec![
			vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION,
			vk::ValidationFeatureEnableEXT::BEST_PRACTICES,
		];
		if settings.gpu_validation {
			enabled_validation_features.push(vk::ValidationFeatureEnableEXT::GPU_ASSISTED);
		}
		let mut validation_features =
			vk::ValidationFeaturesEXT::default().enabled_validation_features(&enabled_validation_features);

		let mut instance_create_info = vk::InstanceCreateInfo::default()
			.application_info(&application_info)
			.enabled_layer_names(&layer_names)
			.enabled_extension_names(&extension_names);
		if settings.validation {
			instance_create_info = instance_create_info.push(&mut validation_features);
		}

		let instance = unsafe { entry.create_instance(&instance_create_info, None) }
			.map_err(|result| creation_error(result, "Unknown error"))?;

		let debug_data = Box::new(DebugCallbackData {
			error_count: AtomicU64::new(0),
			error_log_function: settings.debug_log_function.unwrap_or(|message| println!("{message}")),
		});

		let debug_messenger = if settings.validation {
			let debug_utils = ash::ext::debug_utils::Instance::load(&entry, &instance);

			let debug_utils_create_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
				.message_severity(
					vk::DebugUtilsMessageSeverityFlagsEXT::INFO
						| vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
						| vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
				)
				.message_type(
					vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
						| vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
						| vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
				)
				.pfn_user_callback(Some(vulkan_debug_utils_callback))
				.user_data(std::ptr::from_ref::<DebugCallbackData>(&*debug_data).cast_mut().cast());

			let messenger = unsafe { debug_utils.create_debug_utils_messenger(&debug_utils_create_info, None) }
				.or(Err("Failed to enable debug utils messanger"))?;
			Some((debug_utils, messenger))
		} else {
			None
		};

		Ok(Instance {
			instance,
			entry,
			debug_data,
			debug_messenger,
		})
	}

	pub fn create_device(
		&mut self,
		settings: crate::device::Features,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<crate::vulkan::Device, &'static str> {
		crate::vulkan::Device::new(settings, self, queues)
	}
}

impl Drop for Instance {
	fn drop(&mut self) {
		unsafe {
			if let Some((debug_utils, messenger)) = &self.debug_messenger {
				debug_utils.destroy_debug_utils_messenger(*messenger, None);
			}

			self.instance.destroy_instance(None);
		}
	}
}

/// Collects the requested layer or extension names, failing with the error of the first one that is unavailable.
fn enable_names<const N: usize>(
	names: [(bool, &'static CStr, &'static str); N],
	is_available: impl Fn(&CStr) -> bool,
) -> Result<Vec<*const c_char>, &'static str> {
	names
		.into_iter()
		.filter(|&(requested, ..)| requested)
		.map(|(_, name, error)| if is_available(name) { Ok(name.as_ptr()) } else { Err(error) })
		.collect()
}

/// Describes a failed instance or device creation, falling back to `fallback` for unexpected results.
pub(super) fn creation_error(result: vk::Result, fallback: &'static str) -> &'static str {
	match result {
		vk::Result::ERROR_OUT_OF_HOST_MEMORY => "Out of host memory",
		vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => "Out of device memory",
		vk::Result::ERROR_INITIALIZATION_FAILED => "Initialization failed",
		vk::Result::ERROR_LAYER_NOT_PRESENT => "Layer not present",
		vk::Result::ERROR_EXTENSION_NOT_PRESENT => "Extension not present",
		vk::Result::ERROR_FEATURE_NOT_PRESENT => "Feature not present",
		vk::Result::ERROR_INCOMPATIBLE_DRIVER => "Incompatible driver",
		vk::Result::ERROR_TOO_MANY_OBJECTS => "Too many objects",
		vk::Result::ERROR_DEVICE_LOST => "Device lost",
		vk::Result::ERROR_VALIDATION_FAILED_EXT => "Validation failed",
		_ => fallback,
	}
}

unsafe extern "system" fn vulkan_debug_utils_callback(
	message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
	_message_type: vk::DebugUtilsMessageTypeFlagsEXT,
	p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
	p_user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
	// SAFETY: Vulkan keeps the callback data and its null-terminated message valid for this invocation. The instance
	// owns the boxed user data and destroys the messenger before dropping it. Callbacks may run concurrently, so only borrow the
	// atomic callback state immutably.
	let (Some(message), Some(user_data)) = (unsafe {
		(
			p_callback_data
				.as_ref()
				.and_then(|callback_data| callback_data.message_as_c_str()),
			p_user_data.cast::<DebugCallbackData>().as_ref(),
		)
	}) else {
		return vk::FALSE;
	};
	let Ok(message) = message.to_str() else {
		return vk::FALSE;
	};

	match message_severity {
		vk::DebugUtilsMessageSeverityFlagsEXT::WARNING => println!("{message}"),
		vk::DebugUtilsMessageSeverityFlagsEXT::ERROR => {
			(user_data.error_log_function)(message);
			user_data.error_count.fetch_add(10, Ordering::SeqCst);
		}
		_ => {}
	}

	vk::FALSE
}
