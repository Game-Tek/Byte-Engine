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

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use vulkan::Instance;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use vulkan::Device;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use vulkan::Frame;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use vulkan::CommandBufferRecording;

pub mod buffer;
pub mod command_buffer;
pub mod device;
pub mod frame;
pub mod image;
pub mod raster_pipeline;
pub mod sampler;

mod utils;
