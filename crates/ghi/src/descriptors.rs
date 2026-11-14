#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum WriteData {
	/// A buffer descriptor always spans the whole buffer.
	Buffer {
		handle: BaseBufferHandle,
	},
	Image {
		handle: BaseImageHandle,
		layout: Layouts,
		mip_level: Option<u32>,
		/// The one layer of a layered image, such as a cube face, that a 2D shader resource sees.
		array_layer: Option<u32>,
	},
	CombinedImageSampler {
		image_handle: BaseImageHandle,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	},
	AccelerationStructure {
		handle: TopLevelAccelerationStructureHandle,
	},
	Swapchain(SwapchainHandle),
	Sampler(SamplerHandle),
}

impl WriteData {
	pub(crate) fn buffer(handle: BaseBufferHandle) -> Self {
		Self::Buffer { handle }
	}

	pub(crate) fn image(handle: impl Into<BaseImageHandle>, layout: Layouts) -> Self {
		Self::Image {
			handle: handle.into(),
			layout,
			mip_level: None,
			array_layer: None,
		}
	}

	pub(crate) fn image_mip(
		handle: impl Into<BaseImageHandle>,
		layout: Layouts,
		mip_level: u32,
		array_layer: Option<u32>,
	) -> Self {
		Self::Image {
			handle: handle.into(),
			layout,
			mip_level: Some(mip_level),
			array_layer,
		}
	}

	pub(crate) fn combined_image_sampler(
		image_handle: impl Into<BaseImageHandle>,
		sampler_handle: SamplerHandle,
		layout: Layouts,
	) -> Self {
		Self::CombinedImageSampler {
			image_handle: image_handle.into(),
			sampler_handle,
			layout,
		}
	}

	pub(crate) fn acceleration_structure(handle: TopLevelAccelerationStructureHandle) -> Self {
		Self::AccelerationStructure { handle }
	}

	/// Returns whether this write can fill a slot that a layout declares as a resource of `kind`.
	///
	/// Backends check retained writes against the layout of the pipeline that binds them.
	#[cfg_attr(
		target_os = "macos",
		allow(dead_code, reason = "Metal checks its resolved descriptors instead.")
	)]
	pub(crate) fn matches_kind(self, kind: ResourceKind) -> bool {
		match self {
			Self::Buffer { .. } => matches!(kind, ResourceKind::UniformBuffer | ResourceKind::StorageBuffer),
			Self::Image { .. } | Self::Swapchain(_) => matches!(
				kind,
				ResourceKind::SampledImage | ResourceKind::StorageImage | ResourceKind::InputAttachment
			),
			Self::CombinedImageSampler { .. } => kind == ResourceKind::CombinedImageSampler,
			Self::Sampler(_) => kind == ResourceKind::Sampler,
			Self::AccelerationStructure { .. } => kind == ResourceKind::AccelerationStructure,
		}
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

	/// Binds one image mip as a storage or sampled image resource.
	pub fn image_mip(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
		mip_level: u32,
	) -> Self {
		Self::new(
			descriptor_set,
			slot,
			WriteData::image_mip(image_handle, layout, mip_level, None),
		)
	}

	/// Binds mip `mip_level` of layer `layer` of a layered image, such as one level of one cube face, to a 2D shader
	/// resource, so a compute shader writes that face's level as a GPU-generated environment fills its cube.
	pub fn image_layer_mip(
		descriptor_set: PublicDescriptorSetHandle,
		slot: ResourceSlot,
		image_handle: impl Into<BaseImageHandle>,
		layout: Layouts,
		layer: u32,
		mip_level: u32,
	) -> Self {
		Self::new(
			descriptor_set,
			slot,
			WriteData::image_mip(image_handle, layout, mip_level, Some(layer)),
		)
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
			WriteData::combined_image_sampler(image_handle, sampler_handle, layout),
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

	/// Names, in each frame, the copy of the frame `frame_offset` frames away. Use `-1` for the previous frame.
	///
	/// Use it to read history, such as the image the previous frame rendered, from a resource with
	/// [`crate::Copies::PerFrame`]. The magnitude of the offset must be below the resource's copy count;
	/// [`crate::context::Context::write`] panics otherwise.
	pub fn with_frame_offset(mut self, frame_offset: i32) -> Self {
		self.frame_offset = Some(frame_offset);
		self
	}
}

/// Checks that a frame offset names a copy of a resource that has `copies` copies: its magnitude is below the copy
/// count.
///
/// Backends call this when they resolve a descriptor write or a transfer with a frame offset.
pub(crate) fn assert_frame_offset(copies: usize, frame_offset: i32) {
	assert!(
		(frame_offset.unsigned_abs() as usize) < copies,
		"Frame offset {frame_offset} names a copy the resource doesn't have. The most likely cause is a resource with {copies} copies, such as one built without per-frame copies. Build it with `copies(Copies::PerFrame)`, or use an offset whose magnitude is below {copies}."
	);
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash)]
	pub(crate) struct DescriptorSetHandle;
}

impl crate::PrivateHandle for DescriptorSetHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		DescriptorSetHandle::index(*self)
	}
}

// Metal keeps its frame-local set chains in a `ResourceCollection`, so only the other backends link sets themselves.
#[cfg(not(target_os = "macos"))]
impl Next for DescriptorSet {
	type Handle = DescriptorSetHandle;

	fn next(&self) -> Option<DescriptorSetHandle> {
		self.next
	}
}

#[cfg(not(target_os = "macos"))]
impl HandleLike for DescriptorSetHandle {
	type Item = DescriptorSet;

	fn build(value: u64) -> Self {
		DescriptorSetHandle::from_index(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a DescriptorSet {
		&collection[self.index() as usize]
	}
}

use crate::{
	BaseBufferHandle, BaseImageHandle, DescriptorSetHandle as PublicDescriptorSetHandle, Layouts, SamplerHandle,
	SwapchainHandle, TopLevelAccelerationStructureHandle,
	shader::{ResourceKind, ResourceSlot},
};
#[cfg(not(target_os = "macos"))]
use crate::{DescriptorSet, HandleLike, Next};
