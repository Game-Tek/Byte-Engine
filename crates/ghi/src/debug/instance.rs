use crate::graphics_hardware_interface;

pub struct Instance {}

impl Instance {
	pub fn new(_settings: crate::device::Features) -> Result<Instance, &'static str> {
		Ok(Instance {})
	}

	pub fn create_device(
		&mut self,
		_settings: crate::device::Features,
		_queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<super::Device, &'static str> {
		Ok(super::Device::new())
	}
}
