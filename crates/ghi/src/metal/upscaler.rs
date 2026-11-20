//! MetalFX temporal scalers behind [`crate::upscaler`].
//!
//! The backend records into Metal 4 command buffers, so it builds `MTL4FXTemporalScaler`s with the context's Metal 4
//! compiler. Textures live in untracked heaps, so each scaler carries a fence: the recording updates it after the
//! commands that wrote the scaler's inputs, and MetalFX waits for it before its own work; see
//! [`crate::metal::CommandBufferRecording`]'s `upscale`.

use std::ops::RangeInclusive;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal as mtl;
use objc2_metal::MTLDevice as _;
use objc2_metal_fx as fx;

use super::utils::to_pixel_format;
use crate::upscaler::{Builder, UpscalerError};

/// The `Upscaler` struct keeps one MetalFX temporal scaler with the fence that orders it after the commands that wrote
/// its inputs, so a recording can encode it like any other command.
pub(crate) struct Upscaler {
	pub(crate) scaler: Retained<ProtocolObject<dyn fx::MTL4FXTemporalScaler>>,
	pub(crate) fence: Retained<ProtocolObject<dyn mtl::MTLFence>>,
	pub(crate) name: String,
	/// Whether the scaler reads a reactive mask, which `upscale` must then bind.
	pub(crate) reactive: bool,
}

// SAFETY: Like the context's other Metal objects, a scaler and a fence may move between threads; the context that
// owns them is used by one thread at a time.
unsafe impl Send for Upscaler {}

/// Returns the input-to-output scales MetalFX's Metal 4 temporal scaler takes on `device`, or `None` when it can't run
/// there.
pub(crate) fn scale_range(device: &ProtocolObject<dyn mtl::MTLDevice>) -> Option<RangeInclusive<f32>> {
	// SAFETY: The class methods only query the device.
	unsafe {
		if !fx::MTLFXTemporalScalerDescriptor::supportsMetal4FX(device) {
			return None;
		}
		// MetalFX states its limits as output-over-input factors, so the smallest input scale is the inverse of the
		// largest factor.
		let max_factor = fx::MTLFXTemporalScalerDescriptor::supportedInputContentMaxScaleForDevice(device);
		let min_factor = fx::MTLFXTemporalScalerDescriptor::supportedInputContentMinScaleForDevice(device);
		Some(1.0 / max_factor.max(1.0)..=1.0 / min_factor.max(1.0))
	}
}

/// Builds a MetalFX temporal scaler for the images `builder` describes, with `compiler` building its pipelines.
pub(crate) fn build(
	device: &ProtocolObject<dyn mtl::MTLDevice>,
	compiler: &ProtocolObject<dyn mtl::MTL4Compiler>,
	builder: Builder,
) -> Result<Upscaler, UpscalerError> {
	if scale_range(device).is_none() {
		return Err(UpscalerError::Unsupported);
	}
	let name = builder.name.unwrap_or_default().to_string();
	let rejected = || UpscalerError::Rejected { name: name.clone() };
	// SAFETY: The descriptor is a fresh object configured before the scaler is created from it, and the device and
	// compiler outlive the call.
	let scaler = unsafe {
		let descriptor = fx::MTLFXTemporalScalerDescriptor::new();
		descriptor.setColorTextureFormat(to_pixel_format(builder.color_format));
		descriptor.setDepthTextureFormat(to_pixel_format(builder.depth_format));
		descriptor.setMotionTextureFormat(to_pixel_format(builder.motion_format));
		descriptor.setOutputTextureFormat(to_pixel_format(builder.output_format));
		descriptor.setInputWidth(builder.input_extent.width() as usize);
		descriptor.setInputHeight(builder.input_extent.height() as usize);
		descriptor.setOutputWidth(builder.output_extent.width() as usize);
		descriptor.setOutputHeight(builder.output_extent.height() as usize);
		// The engine passes its own exposure through `Inputs::pre_exposure`.
		descriptor.setAutoExposureEnabled(false);
		if let Some(format) = builder.reactive_format {
			descriptor.setReactiveMaskTextureEnabled(true);
			descriptor.setReactiveMaskTextureFormat(to_pixel_format(format));
		}
		descriptor.newTemporalScalerWithDevice_compiler(device, compiler)
	}
	.ok_or_else(rejected)?;
	// SAFETY: Setting a plain flag on a scaler nothing encodes yet.
	unsafe {
		use fx::MTLFXTemporalScalerBase as _;
		scaler.setDepthReversed(builder.depth_reversed);
	}
	let fence = device.newFence().ok_or_else(rejected)?;
	Ok(Upscaler {
		scaler,
		fence,
		name,
		reactive: builder.reactive_format.is_some(),
	})
}
