use utils::Extent;

use crate::{CommandBufferHandle, DynamicBufferHandle, FrameKey, ImageHandle, PresentKey, SwapchainHandle};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut super::Device,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut super::Device, frame_key: FrameKey) -> Self {
		Self { frame_key, device }
	}
}

impl Frame<'_> {
	pub fn get_mut_dynamic_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: DynamicBufferHandle<T>) -> &'a mut T {
		self.device.dynamic_buffer_slice_mut(buffer_handle)
	}

	pub fn resize_image(&mut self, image_handle: ImageHandle, extent: Extent) {
		self.device.resize_image_internal(image_handle, extent);
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.device.create_command_buffer_recording(command_buffer_handle)
	}

	pub fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> (PresentKey, Extent) {
		let extent = self.device.swapchain_extent(swapchain_handle);
		let image_index = self.device.next_swapchain_image_index(swapchain_handle);
		let present_key = PresentKey {
			image_index,
			sequence_index: self.frame_key.sequence_index,
			swapchain: swapchain_handle,
		};
		self.device.swapchains[swapchain_handle.0 as usize].acquired_image_indices[self.frame_key.sequence_index as usize] =
			image_index;
		(present_key, extent)
	}

	pub fn device(&mut self) -> &mut super::Device {
		self.device
	}
}
