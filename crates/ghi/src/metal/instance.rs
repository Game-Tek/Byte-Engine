use objc2_metal::MTLDevice;

use super::*;

pub struct Instance {
	devices: Vec<Retained<ProtocolObject<dyn mtl::MTLDevice>>>,
}

// SAFETY: The instance owns retained Metal device objects, which Metal documents as safe to transfer between threads.
unsafe impl Send for Instance {}

impl Instance {
	/// Lists the Metal devices. Metal needs no instance-wide setup, so only [`Self::create_device`] reads the features.
	pub fn new(_features: crate::device::Features) -> Result<Instance, &'static str> {
		let devices = mtl::MTLCopyAllDevices().to_vec();

		if devices.is_empty() {
			return Err("No Metal devices available. The most likely cause is that the system does not support Metal.");
		}

		Ok(Instance { devices })
	}

	/// Creates a Metal device on the GPU `settings` names, or on the system default GPU.
	pub fn create_device(
		&mut self,
		settings: crate::device::Features,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<super::Device, &'static str> {
		let device = match settings.gpu {
			Some(preferred_name) => self
				.devices
				.iter()
				.find(|device| device.name().to_string() == preferred_name)
				.cloned()
				.ok_or(
					"Requested Metal device not found. The most likely cause is that the device name does not match any available GPU.",
				)?,
			None => mtl::MTLCreateSystemDefaultDevice()
				.or_else(|| self.devices.first().cloned())
				.ok_or(
					"Metal device creation failed. The most likely cause is that no compatible Metal device is available.",
				)?,
		};

		super::Device::new(settings, device, queues)
	}
}
