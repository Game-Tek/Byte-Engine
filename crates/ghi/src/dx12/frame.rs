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

	pub fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: SwapchainHandle,
		uses: crate::Uses,
	) -> (PresentKey, crate::ImageHandle, crate::Formats, Extent) {
		let extent = self.device.swapchain_extent(swapchain_handle);
		let image_index = self.device.next_swapchain_image_index(swapchain_handle);
		let swapchain = &mut self.device.swapchains[swapchain_handle.0 as usize];
		let needs_new_proxy =
			swapchain.images[image_index as usize].is_none() || !swapchain.proxy_uses[image_index as usize].contains(uses);

		if needs_new_proxy {
			let image = self.device.build_image(
				crate::image::Builder::new(crate::Formats::BGRAu8, uses | crate::Uses::BlitSource)
					.extent(extent)
					.device_accesses(crate::DeviceAccesses::DeviceOnly)
					.use_case(crate::UseCases::DYNAMIC),
			);
			let swapchain = &mut self.device.swapchains[swapchain_handle.0 as usize];
			swapchain.images[image_index as usize] = Some(image);
			swapchain.proxy_uses[image_index as usize] = uses;
		}

		let present_key = PresentKey {
			image_index,
			sequence_index: self.frame_key.sequence_index,
			swapchain: swapchain_handle,
		};
		(
			present_key,
			self.device.swapchains[swapchain_handle.0 as usize].images[image_index as usize].unwrap(),
			crate::Formats::BGRAu8,
			extent,
		)
	}

	pub fn device(&mut self) -> &mut super::Device {
		self.device
	}
}
