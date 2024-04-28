//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![feature(generic_const_exprs)]
#![feature(pointer_is_aligned_to)]

pub mod window;
#[cfg(target_os = "linux")]
pub mod x11_window;
#[cfg(target_os = "linux")]
pub mod wayland_window;

pub mod graphics_hardware_interface;
pub mod shader_compilation;
pub mod vulkan;
pub mod render_debugger;

pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub fn create(settings: graphics_hardware_interface::Features) -> GHI {
	GHI(vulkan::VulkanGHI::new(settings).expect("Failed to create VulkanGHI"))
}

// pub enum GHI {
// 	Vulkan(vulkan_ghi::VulkanGHI),
// }

pub struct GHI(pub vulkan::VulkanGHI);

impl std::ops::Deref for GHI {
	type Target = vulkan::VulkanGHI;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl std::ops::DerefMut for GHI {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.0
	}
}

pub struct CBR<'a>(pub vulkan::VulkanCommandBufferRecording<'a>);

impl<'a> std::ops::Deref for CBR<'a> {
	type Target = vulkan::VulkanCommandBufferRecording<'a>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'a> std::ops::DerefMut for CBR<'a> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.0
	}
}