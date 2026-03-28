use crate::{
	graphics_hardware_interface::ImageHandleLike, BaseBufferHandle, DescriptorSet, DescriptorSetBindingHandle, HandleLike,
	ImageHandle, Layouts, Next, Ranges, SamplerHandle, SwapchainHandle, TopLevelAccelerationStructureHandle,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WriteData {
	Buffer {
		handle: BaseBufferHandle,
		size: Ranges,
	},
	Image {
		handle: ImageHandle,
		layout: Layouts,
	},
	CombinedImageSampler {
		image_handle: ImageHandle,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		layer: Option<u32>,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
	Swapchain(SwapchainHandle),
	Sampler(SamplerHandle),
	StaticSamplers,
	CombinedImageSamplerArray,
}

/// Stores the information of a descriptor set write.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Write {
	pub(super) binding_handle: DescriptorSetBindingHandle,
	/// The index of the array element to write to in the binding(if the binding is an array).
	pub(super) array_element: u32,
	/// Information describing the descriptor.
	pub(super) descriptor: WriteData,
	pub(super) frame_offset: Option<i32>,
}

impl Write {
	pub fn new(binding_handle: DescriptorSetBindingHandle, descriptor: WriteData) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor,
			frame_offset: None,
		}
	}

	pub fn buffer(binding_handle: DescriptorSetBindingHandle, buffer_handle: BaseBufferHandle) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::Buffer {
				handle: buffer_handle,
				size: Ranges::Whole,
			},
			frame_offset: None,
		}
	}

	pub fn image(binding_handle: DescriptorSetBindingHandle, image_handle: impl ImageHandleLike, layout: Layouts) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::Image {
				handle: image_handle.into_image_handle(),
				layout,
			},
			frame_offset: None,
		}
	}

	pub fn image_with_frame(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl ImageHandleLike,
		layout: Layouts,
		frame_offset: i32,
	) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::Image {
				handle: image_handle.into_image_handle(),
				layout,
			},
			frame_offset: Some(frame_offset),
		}
	}

	pub fn sampler(binding_handle: DescriptorSetBindingHandle, sampler_handle: SamplerHandle) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::Sampler(sampler_handle),
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl ImageHandleLike,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::CombinedImageSampler {
				image_handle: image_handle.into_image_handle(),
				sampler_handle,
				layout,
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn combined_image_sampler_array(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: ImageHandle,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		index: u32,
	) -> Write {
		Write {
			binding_handle,
			array_element: index,
			descriptor: WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer: None,
			},
			frame_offset: None,
		}
	}

	pub fn acceleration_structure(
		binding_handle: DescriptorSetBindingHandle,
		acceleration_structure_handle: TopLevelAccelerationStructureHandle,
	) -> Write {
		Write {
			binding_handle,
			array_element: 0,
			descriptor: WriteData::AccelerationStructure {
				handle: acceleration_structure_handle,
			},
			frame_offset: None,
		}
	}
}

#[derive(Clone, Copy)]
/// Enumerates the available descriptor types.
pub enum DescriptorType {
	/// A uniform buffer.
	UniformBuffer,
	/// A storage buffer.
	StorageBuffer,
	/// An image.
	SampledImage,
	/// A combined image sampler.
	CombinedImageSampler,
	/// A storage image.
	StorageImage,
	/// An input attachment.
	InputAttachment,
	/// A sampler.
	Sampler,
	/// An acceleration structure.
	AccelerationStructure,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DescriptorSetHandle(pub(crate) u64);

impl Next for DescriptorSet {
	type Handle = DescriptorSetHandle;

	fn next(&self) -> Option<DescriptorSetHandle> {
		self.next
	}
}

impl HandleLike for DescriptorSetHandle {
	type Item = DescriptorSet;

	fn build(value: u64) -> Self {
		DescriptorSetHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a DescriptorSet {
		&collection[self.0 as usize]
	}
}
