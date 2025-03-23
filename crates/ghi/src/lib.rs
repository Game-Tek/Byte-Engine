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
pub mod glsl;
pub mod render_debugger;

pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub use crate::vulkan::VulkanCommandBufferRecording as CommandBufferRecording;

pub mod image;
pub mod sampler;

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

pub fn compile_glsl<'a>(name: &'a str, source: &'a str) -> Result<Box<[u8]>, String> {
	let compiler = shaderc::Compiler::new().unwrap();
	let mut options = shaderc::CompileOptions::new().unwrap();

	options.set_optimization_level(shaderc::OptimizationLevel::Performance);
	options.set_target_env(shaderc::TargetEnv::Vulkan, (1 << 22) | (3 << 12));

	if cfg!(debug_assertions) {
		options.set_generate_debug_info();
	}

	options.set_target_spirv(shaderc::SpirvVersion::V1_6);
	options.set_invert_y(true);

	let binary = compiler.compile_into_spirv(&source, shaderc::ShaderKind::InferFromSource, name, "main", Some(&options));

	match binary {
		Ok(binary) => Ok(binary.as_binary_u8().into()),
		Err(error) => {
			Err(glsl::pretty_print(&glsl::process_glslc_error(name, source, &error.to_string())))
		}
	}
}
