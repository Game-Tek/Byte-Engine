//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![feature(generic_const_exprs)]
#![feature(pointer_is_aligned)]
#![feature(pointer_is_aligned_to)]

pub mod window;
pub mod graphics_hardware_interface;
pub mod shader_compilation;
pub mod vulkan_ghi;
pub mod render_debugger;

// Enable if on linux
#[cfg(target_os = "linux")]
pub mod wayland_window;

pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub fn create() -> impl GraphicsHardwareInterface {
	vulkan_ghi::VulkanGHI::new(&graphics_hardware_interface::Features::new().validation(false)).expect("Failed to create VulkanGHI")
}