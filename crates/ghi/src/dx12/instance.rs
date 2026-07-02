use windows::core::Interface as _;
use windows::Win32::Graphics::Direct3D12::{
	D3D12GetDebugInterface, ID3D12Debug, ID3D12Debug3, D3D12_GPU_BASED_VALIDATION_FLAGS_NONE,
};

use crate::{device::Features, QueueHandle, QueueSelection};

pub struct Instance {
	debug: Option<ID3D12Debug>,
}

impl Instance {
	/// Creates a DX12 instance and optionally enables the debug layer.
	pub fn new(settings: Features) -> Result<Self, &'static str> {
		let debug = if settings.validation {
			let mut debug: Option<ID3D12Debug> = None;
			unsafe { D3D12GetDebugInterface(&mut debug) }
				.map_err(|_| "Failed to acquire the D3D12 debug interface. The most likely cause is that the Graphics Tools optional feature is not installed.")?;
			let debug = debug.ok_or("Failed to acquire the D3D12 debug interface. The most likely cause is that the debug interface was not returned by the API.")?;
			unsafe {
				debug.EnableDebugLayer();
				if settings.gpu_validation {
					let debug3 = debug.cast::<ID3D12Debug3>().map_err(|_| {
						"Failed to acquire the D3D12 debug3 interface. The most likely cause is that GPU-based validation is not supported by the installed D3D12 debug layer."
					})?;
					debug3.SetEnableGPUBasedValidation(true);
					debug3.SetEnableSynchronizedCommandQueueValidation(true);
					debug3.SetGPUBasedValidationFlags(D3D12_GPU_BASED_VALIDATION_FLAGS_NONE);
				}
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
		settings: Features,
		queues: &mut [(QueueSelection, &mut Option<QueueHandle>)],
	) -> Result<crate::dx12::Device, &'static str> {
		crate::dx12::Device::new(settings, queues)
	}
}
