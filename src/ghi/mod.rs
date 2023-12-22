//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

pub mod graphics_hardware_interface;

pub mod shader_compilation;

pub mod vulkan_ghi;

pub use crate::ghi::graphics_hardware_interface::*;

pub fn create() -> impl GraphicsHardwareInterface {
	vulkan_ghi::VulkanGHI::new(&self::vulkan_ghi::Settings{ validation: true, ray_tracing: false })
}