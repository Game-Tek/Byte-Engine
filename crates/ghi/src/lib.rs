//! The G.H.I. module (graphics hardware interface) is responsible for abstracting the access to the graphics hardware.

#![feature(generic_const_exprs)]
#![feature(str_as_str)]
#![feature(pointer_is_aligned_to)]
#![feature(extend_one)]

pub mod window;

pub mod graphics_hardware_interface;
pub mod render_debugger;

pub mod debug;

#[cfg(target_os = "windows")]
pub mod dx12;
#[cfg(all(target_os = "macos", feature = "metal"))]
pub mod metal;
pub mod vulkan;

pub use crate::graphics_hardware_interface::*;
pub use crate::window::*;

pub mod implementation {
	pub const USES_METAL: bool = cfg!(all(target_os = "macos", feature = "metal"));

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Instance;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Device;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::Frame;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::CommandBufferRecording;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	pub use metal::queue::Queue;

	#[cfg(all(target_os = "macos", feature = "metal"))]
	use crate::metal;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Instance;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Device;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::Frame;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::CommandBufferRecording;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	pub use vulkan::queue::Queue;

	#[cfg(any(
		target_os = "linux",
		target_os = "windows",
		all(target_os = "macos", not(feature = "metal"))
	))]
	use crate::vulkan;
}

pub mod buffer;
pub mod command_buffer;
pub mod descriptors;
pub mod device;
pub mod frame;
pub mod image;
pub mod pipelines;
pub mod queue;
pub mod rt;
pub mod sampler;
pub mod shader;

pub mod types;
mod utils;

pub use device::Device;
pub use frame::Frame;
pub use queue::Queue;

pub use pipelines::ShaderParameter;

pub use types::*;
