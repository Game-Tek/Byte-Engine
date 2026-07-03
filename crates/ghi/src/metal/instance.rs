use objc2_metal::MTLDevice;

use super::*;

pub struct Instance {
	devices: Vec<Retained<ProtocolObject<dyn mtl::MTLDevice>>>,
	settings: crate::device::Features,
}

unsafe impl Send for Instance {}

impl Instance {
	pub fn new(settings: crate::device::Features) -> Result<Instance, &'static str> {
		let devices = mtl::MTLCopyAllDevices().to_vec();

		if devices.is_empty() {
			return Err("No Metal devices available. The most likely cause is that the system does not support Metal.");
		}

		Ok(Instance { devices, settings })
	}

	pub fn create_device(
		&mut self,
		settings: crate::device::Features,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<super::Device, &'static str> {
		let device = if let Some(preferred_name) = settings.gpu {
			let selected = self.devices.iter().find(|device| device.name().to_string() == preferred_name);

			match selected {
				Some(device) => device.clone(),
				None => {
					return Err(
						"Requested Metal device not found. The most likely cause is that the device name does not match any available GPU.",
					);
				}
			}
		} else {
			mtl::MTLCreateSystemDefaultDevice()
				.or_else(|| self.devices.first().cloned())
				.ok_or("Metal device creation failed. The most likely cause is that no compatible Metal device is available.")?
		};

		let merged_settings = crate::device::Features {
			validation: settings.validation || self.settings.validation,
			gpu_validation: settings.gpu_validation || self.settings.gpu_validation,
			api_dump: settings.api_dump || self.settings.api_dump,
			ray_tracing: settings.ray_tracing || self.settings.ray_tracing,
			debug_log_function: settings.debug_log_function.or(self.settings.debug_log_function),
			gpu: settings.gpu.or(self.settings.gpu),
			sparse: settings.sparse || self.settings.sparse,
			geometry_shader: settings.geometry_shader || self.settings.geometry_shader,
			mesh_shading: settings.mesh_shading || self.settings.mesh_shading,
		};

		super::Device::new(merged_settings, device, queues)
	}
}
