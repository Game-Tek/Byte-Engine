use crate::{
	BaseBufferHandle, BaseImageHandle, DescriptorSet, DescriptorSetBindingHandle, HandleLike, Layouts, Next, Ranges,
	SamplerHandle, SwapchainHandle, TopLevelAccelerationStructureHandle,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WriteData {
	Buffer {
		handle: BaseBufferHandle,
		size: Ranges,
	},
	Image {
		handle: BaseImageHandle,
		layout: Layouts,
	},
	CombinedImageSampler {
		image_handle: BaseImageHandle,
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

impl WriteData {
	pub(crate) fn buffer(handle: BaseBufferHandle) -> Self {
		Self::Buffer {
			handle,
			size: Ranges::Whole,
		}
	}

	pub(crate) fn image(handle: impl Into<BaseImageHandle>, layout: Layouts) -> Self {
		Self::Image {
			handle: handle.into(),
			layout,
		}
	}

	pub(crate) fn combined_image_sampler(
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		layer: Option<u32>,
	) -> Self {
		Self::CombinedImageSampler {
			image_handle: image_handle.into(),
			sampler_handle,
			layout,
			layer,
		}
	}

	pub(crate) fn acceleration_structure(handle: TopLevelAccelerationStructureHandle) -> Self {
		Self::AccelerationStructure { handle }
	}
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
		Self::new(binding_handle, WriteData::buffer(buffer_handle))
	}

	pub fn image(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
	) -> Write {
		Self::new(binding_handle, WriteData::image(image_handle, layout))
	}

	pub fn image_with_frame(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
		frame_offset: i32,
	) -> Write {
		Self::image(binding_handle, image_handle, layout).with_frame_offset(frame_offset)
	}

	pub fn sampler(binding_handle: DescriptorSetBindingHandle, sampler_handle: SamplerHandle) -> Write {
		Self::new(binding_handle, WriteData::Sampler(sampler_handle))
	}

	pub fn combined_image_sampler(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Write {
		Self::new(
			binding_handle,
			WriteData::combined_image_sampler(image_handle, sampler_handle, layout, None),
		)
	}

	pub fn combined_image_sampler_with_frame(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		frame_offset: i32,
	) -> Write {
		Self::combined_image_sampler(binding_handle, image_handle, sampler_handle, layout).with_frame_offset(frame_offset)
	}

	pub fn combined_image_sampler_array(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		index: u32,
	) -> Write {
		Self::combined_image_sampler(binding_handle, image_handle, sampler_handle, layout).with_array_element(index)
	}

	pub fn combined_image_sampler_array_with_frame(
		binding_handle: DescriptorSetBindingHandle,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		index: u32,
		frame_offset: i32,
	) -> Write {
		Self::combined_image_sampler(binding_handle, image_handle, sampler_handle, layout)
			.with_array_element(index)
			.with_frame_offset(frame_offset)
	}

	pub fn acceleration_structure(
		binding_handle: DescriptorSetBindingHandle,
		acceleration_structure_handle: TopLevelAccelerationStructureHandle,
	) -> Write {
		Self::new(
			binding_handle,
			WriteData::acceleration_structure(acceleration_structure_handle),
		)
	}

	fn with_array_element(mut self, array_element: u32) -> Self {
		self.array_element = array_element;
		self
	}

	fn with_frame_offset(mut self, frame_offset: i32) -> Self {
		self.frame_offset = Some(frame_offset);
		self
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
