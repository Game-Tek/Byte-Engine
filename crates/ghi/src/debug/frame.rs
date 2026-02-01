use utils::Extent;
use crate::{CommandBufferHandle, DynamicBufferHandle, FrameKey, ImageHandle, PresentKey, SwapchainHandle};

pub struct Frame<'a> {
	device: &'a mut super::Device,
	frame_key: FrameKey,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut super::Device, frame_key: FrameKey) -> Self {
		Self { device, frame_key }
	}

	pub fn get_mut_dynamic_buffer_slice<'b, T: Copy>(&'b self, _buffer_handle: DynamicBufferHandle<T>) -> &'b mut T {
		todo!("Handle true allocations");
	}

	pub fn resize_image(&mut self, _image_handle: ImageHandle, _extent: Extent) {
	}

	pub fn create_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) -> super::CommandBufferRecording {
		super::CommandBufferRecording::new(self.device, command_buffer_handle, Vec::new(), Vec::new(), Some(self.frame_key))
	}

	pub fn acquire_swapchain_image(&mut self, _swapchain_handle: SwapchainHandle) -> (PresentKey, Extent) {
		(PresentKey {
			image_index: 0,
			sequence_index: 0,
			swapchain: SwapchainHandle(0),
		}, Extent::rectangle(0, 0))
	}

	pub fn device(&mut self) -> &mut super::Device {
		self.device
	}
}
