use crate::{
	shader::ResourceSlot, BaseBufferHandle, BaseImageHandle, DescriptorSet, DescriptorSetHandle as PublicDescriptorSetHandle,
	HandleLike, Layouts, Next, Ranges, SamplerHandle, SwapchainHandle, TopLevelAccelerationStructureHandle,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum WriteData {
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

/// The `DescriptorWrite` struct records one retained resource update at a flat shader slot.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct DescriptorWrite {
	pub(crate) descriptor_set: PublicDescriptorSetHandle,
	pub(crate) slot: ResourceSlot,
	/// The index of the array element to update when the resource is an array.
	pub(crate) array_element: u32,
	pub(crate) descriptor: WriteData,
	pub(crate) frame_offset: Option<i32>,
}

impl DescriptorWrite {
	pub(crate) fn new(descriptor_set: PublicDescriptorSetHandle, slot: ResourceSlot, descriptor: WriteData) -> Self {
		Self {
			descriptor_set,
			slot,
			array_element: 0,
			descriptor,
			frame_offset: None,
		}
	}

	pub fn buffer(descriptor_set: PublicDescriptorSetHandle, slot: ResourceSlot, buffer_handle: BaseBufferHandle) -> Self {
		Self::new(descriptor_set, slot, WriteData::buffer(buffer_handle))
	}

	pub fn image(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
	) -> Self {
		Self::new(descriptor_set, slot, WriteData::image(image_handle, layout))
	}

	pub fn image_with_frame(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
		frame_offset: i32,
	) -> Self {
		Self::image(descriptor_set, slot, image_handle, layout).with_frame_offset(frame_offset)
	}

	pub fn sampler(descriptor_set: PublicDescriptorSetHandle, slot: ResourceSlot, sampler_handle: SamplerHandle) -> Self {
		Self::new(descriptor_set, slot, WriteData::Sampler(sampler_handle))
	}

	pub fn swapchain(descriptor_set: PublicDescriptorSetHandle, slot: ResourceSlot, swapchain_handle: SwapchainHandle) -> Self {
		Self::new(descriptor_set, slot, WriteData::Swapchain(swapchain_handle))
	}

	pub fn combined_image_sampler(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Self {
		Self::new(
			descriptor_set,
			slot,
			WriteData::combined_image_sampler(image_handle, sampler_handle, layout, None),
		)
	}

	pub fn combined_image_sampler_with_frame(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		frame_offset: i32,
	) -> Self {
		Self::combined_image_sampler(descriptor_set, slot, image_handle, sampler_handle, layout).with_frame_offset(frame_offset)
	}

	pub fn combined_image_sampler_layer(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		layer: u32,
	) -> Self {
		Self::new(
			descriptor_set,
			slot,
			WriteData::combined_image_sampler(image_handle, sampler_handle, layout, Some(layer)),
		)
	}

	pub fn combined_image_sampler_array(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		index: u32,
	) -> Self {
		Self::combined_image_sampler(descriptor_set, slot, image_handle, sampler_handle, layout).with_array_element(index)
	}

	pub fn combined_image_sampler_array_with_frame(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
		index: u32,
		frame_offset: i32,
	) -> Self {
		Self::combined_image_sampler(descriptor_set, slot, image_handle, sampler_handle, layout)
			.with_array_element(index)
			.with_frame_offset(frame_offset)
	}

	pub fn acceleration_structure(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		acceleration_structure_handle: TopLevelAccelerationStructureHandle,
	) -> Self {
		Self::new(
			descriptor_set,
			slot,
			WriteData::acceleration_structure(acceleration_structure_handle),
		)
	}

	pub fn with_array_element(mut self, array_element: u32) -> Self {
		self.array_element = array_element;
		self
	}

	pub fn with_frame_offset(mut self, frame_offset: i32) -> Self {
		self.frame_offset = Some(frame_offset);
		self
	}
}

#[derive(Clone, Copy)]
/// Legacy descriptor categories retained internally for the pending Vulkan and DX12 migrations.
pub(crate) enum DescriptorType {
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
