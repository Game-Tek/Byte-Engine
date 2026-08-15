//! The `factory` module exposes detached Vulkan resource types for public API consumers.

/// The `Factory` type alias provides detached Vulkan resource creation from a cloned logical device.
pub type Factory = crate::vulkan::Device;

pub use crate::vulkan::device::{ComputePipeline, FactoryImage, FactorySampler, RasterPipeline};

/// The `Image` type alias preserves the detached image name used by backend-specific factory paths.
pub type Image = FactoryImage;

/// The `Sampler` type alias preserves the detached sampler name used by backend-specific factory paths.
pub type Sampler = FactorySampler;
