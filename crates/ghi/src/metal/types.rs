use crate::{PrivateHandles, buffer::BufferHandle, graphics_hardware_interface, image::ImageHandle, sampler::SamplerHandle};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Descriptor {
	Image {
		image: ImageHandle,
		layout: crate::Layouts,
		mip_level: Option<u32>,
	},
	CombinedImageSampler {
		image: ImageHandle,
		sampler: SamplerHandle,
		layout: crate::Layouts,
	},
	Buffer {
		buffer: BufferHandle,
		size: graphics_hardware_interface::Ranges,
	},
	Sampler {
		sampler: SamplerHandle,
	},
	Swapchain {
		handle: crate::swapchain::SwapchainHandle,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
}

impl Descriptor {
	pub(crate) fn tracked_resource(self) -> Option<PrivateHandles> {
		match self {
			Descriptor::Buffer { buffer, .. } => Some(PrivateHandles::Buffer(buffer)),
			Descriptor::Image { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::CombinedImageSampler { image, .. } => Some(PrivateHandles::Image(image)),
			Descriptor::Sampler { .. } => None,
			Descriptor::Swapchain { handle } => Some(PrivateHandles::Swapchain(handle)),
			Descriptor::AccelerationStructure { .. } => None,
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TopLevelAccelerationStructureHandle(pub(crate) u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BottomLevelAccelerationStructureHandle(pub(crate) u64);

pub(crate) const MAX_FRAMES_IN_FLIGHT: usize = 3;
pub(crate) const MAX_SWAPCHAIN_IMAGES: usize = 8;
