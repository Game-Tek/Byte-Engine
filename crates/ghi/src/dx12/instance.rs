use crate::{QueueHandle, QueueSelection, device::Features};

/// The `Instance` struct provides the entry point for creating an independently configured DX12 device.
pub struct Instance;

impl Instance {
	/// Creates a DX12 instance that defers runtime and validation configuration until device creation.
	pub fn new(_settings: Features) -> Result<Self, &'static str> {
		// The device factory owns configuration state, so Device::new enables validation on that factory.
		Ok(Self)
	}

	/// Creates a DX12 device and the requested queues.
	pub fn create_device(
		&mut self,
		settings: Features,
		queues: &mut [(QueueSelection, &mut Option<QueueHandle>)],
	) -> Result<crate::dx12::Device, &'static str> {
		crate::dx12::Device::new(settings, queues)
	}
}
