//! DX12 resource metadata for native resource-IO requests.

use super::super::*;

impl Device {
	/// Returns the native device used to create DirectStorage queues and completion fences.
	pub(crate) fn resource_io_native_device(&self) -> &ID3D12Device {
		&self.device
	}

	/// Resolves a static buffer and its current DirectStorage compatibility without changing resource state.
	pub(crate) fn resource_io_buffer_destination(
		&self,
		buffer_handle: BaseBufferHandle,
	) -> Option<ResourceIoBufferDestination> {
		let (index, dynamic) = Self::buffer_index(buffer_handle);
		if dynamic {
			return None;
		}

		let buffer = self.buffers.get(index)?;
		let resource = buffer.resource.clone()?;
		let common_state = self
			.buffer_states
			.get(&Self::native_resource_key(&resource))
			.copied()
			.unwrap_or_else(|| Self::initial_buffer_barrier_state(buffer.heap_kind))
			== BufferBarrierState::COMMON;

		Some(ResourceIoBufferDestination {
			resource,
			size: buffer.size,
			common_state,
			direct_storage_compatible: buffer.heap_kind == BufferHeapKind::Default,
		})
	}

	/// Resolves a static image and its current DirectStorage compatibility without changing resource state.
	pub(crate) fn resource_io_image_destination(
		&self,
		image_handle: crate::BaseImageHandle,
	) -> Option<ResourceIoImageDestination> {
		let image = self.images.get(image_handle.0 as usize)?;
		if image.frame_resources.is_some() {
			return None;
		}

		let resource = image.resource.clone()?;
		let common_state = self
			.image_states
			.get(&Self::native_resource_key(&resource))
			.copied()
			.unwrap_or(TextureBarrierState::COMMON)
			== TextureBarrierState::COMMON;

		Some(ResourceIoImageDestination {
			resource,
			extent: image.extent,
			format: image.format,
			array_layers: image.array_layers,
			mip_levels: image.mip_levels,
			common_state,
		})
	}
}
