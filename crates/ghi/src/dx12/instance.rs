use windows::Win32::Graphics::Direct3D12::{D3D12GetDebugInterface, ID3D12Debug};

use crate::graphics_hardware_interface;

pub struct Instance {
	debug: Option<ID3D12Debug>,
}

impl Instance {
	/// Creates a DX12 instance and optionally enables the debug layer.
	pub fn new(settings: graphics_hardware_interface::Features) -> Result<Self, &'static str> {
		let debug = if settings.validation {
			let mut debug: Option<ID3D12Debug> = None;
			unsafe { D3D12GetDebugInterface(&mut debug) }.map_err(|_| {
				"Failed to acquire the D3D12 debug interface. The most likely cause is that the Graphics Tools optional feature is not installed."
			})?;
			let debug = debug.ok_or(
				"Failed to acquire the D3D12 debug interface. The most likely cause is that the debug interface was not returned by the API.",
			)?;
			unsafe {
				debug.EnableDebugLayer();
			}
			Some(debug)
		} else {
			None
		};

		Ok(Self { debug })
	}

	/// Creates a DX12 device and the requested queues.
	pub fn create_device(
		&mut self,
		settings: graphics_hardware_interface::Features,
		queues: &mut [(graphics_hardware_interface::QueueSelection, &mut Option<graphics_hardware_interface::QueueHandle>)],
	) -> Result<crate::dx12::Device, &'static str> {
		crate::dx12::Device::new(settings, queues)
	}
}
