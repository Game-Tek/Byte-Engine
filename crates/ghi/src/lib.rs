//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![feature(generic_const_exprs)]
#![feature(str_as_str)]
#![feature(pointer_is_aligned_to)]

pub mod window;
#[cfg(target_os = "linux")]
pub mod x11_window;
#[cfg(target_os = "linux")]
pub mod wayland_window;
#[cfg(target_os = "windows")]
pub mod win32_window;

pub mod graphics_hardware_interface;
pub mod vulkan;
pub mod render_debugger;

pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

#[cfg(target_os = "linux")]
pub use vulkan::Device as Device;
#[cfg(target_os = "linux")]
pub use vulkan::CommandBufferRecording as CommandBufferRecording;

pub mod image;
pub mod sampler;
pub mod raster_pipeline;

pub fn create(settings: graphics_hardware_interface::Features) -> Device {
	vulkan::Device::new(settings).expect("Failed to create VulkanGHI")
}