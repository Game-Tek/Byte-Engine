//! The `factory` module exposes detached GPU resource types for public cross-platform consumers.

#[cfg(target_os = "windows")]
pub use crate::dx12::factory::*;
#[cfg(target_os = "macos")]
pub use crate::metal::factory::*;
#[cfg(target_os = "linux")]
pub use crate::vulkan::{ComputePipeline, Factory, FactoryImage, FactorySampler, RasterPipeline};
