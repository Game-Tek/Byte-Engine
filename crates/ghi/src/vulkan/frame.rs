use ash::vk::{self, Handle as _};
use utils::Extent;

use crate::{graphics_hardware_interface, vulkan::{BufferCopy, BufferHandle, HandleLike as _, ImageCopy, ImageHandle, Tasks}, CommandBufferRecording, Device, FrameKey};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut Device,
}

impl <'a> Frame<'a> {
	pub fn new(device: &'a mut Device, frame_key: FrameKey) -> Self {
		Self {
			frame_key,
			device,
		}
	}
}

impl crate::frame::Frame for Frame<'_> {
	fn acquire_swapchain_image(&mut self, swapchain_handle: crate::SwapchainHandle) -> (crate::PresentKey, utils::Extent) {
		let swapchains = &self.device.swapchains;
		let synchronizers = &self.device.synchronizers;

		let swapchain = &swapchains[swapchain_handle.0 as usize];

		let s = swapchain.images.iter().filter(|e| !e.is_null()).count() as u64;
		let m = swapchain.min_image_count as u64;

		let frame_key = self.frame_key;

		let swapchain_frame_synchronizer = swapchain.acquire_synchronizers[frame_key.sequence_index as usize].access(synchronizers);

		let semaphore = swapchain_frame_synchronizer.semaphore;

		// Use our own waiting technique if only one image (s - m == 0) can be acquired at a time, since
		let use_vulkan_timeout = s - m != 0;

		let acquire_info = vk::AcquireNextImageInfoKHR::default()
			.swapchain(swapchain.swapchain)
			.timeout(if use_vulkan_timeout { u64::MAX } else { 0 })
			.semaphore(semaphore)
			.device_mask(1)
			.fence(swapchain_frame_synchronizer.fence)
		;

		let mut vk_surface_present_mode = vk::SurfacePresentModeEXT::default().present_mode(swapchain.vk_present_mode);

		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
    		.push_next(&mut vk_surface_present_mode)
			.surface(swapchain.surface)
		;

		let mut vk_present_modes = [swapchain.vk_present_mode];

		let mut vk_surface_present_mode_compatibility = vk::SurfacePresentModeCompatibilityEXT::default().present_modes(&mut vk_present_modes);

		let mut vk_surface_capabilities = vk::SurfaceCapabilities2KHR::default()
			.push_next(&mut vk_surface_present_mode_compatibility)
		;

		unsafe { self.device.surface_capabilities.get_physical_device_surface_capabilities2(self.device.physical_device, &vk_surface_info, &mut vk_surface_capabilities).expect("No surface capabilities") };

		let vk_surface_capabilities = vk_surface_capabilities.surface_capabilities;

		let device = &self.device.device;

		unsafe {
			let _ = device.wait_for_fences(&[swapchain_frame_synchronizer.fence], true, u64::MAX);
			let _ = device.reset_fences(&[swapchain_frame_synchronizer.fence]);
		}

		let swapchain_functions = &self.device.swapchain;

		let acquisition_result = if !use_vulkan_timeout {
			loop {
				let acquisition_result = unsafe { swapchain_functions.acquire_next_image2(&acquire_info) };

				match acquisition_result {
					Ok(_) => break acquisition_result,
					Err(vk::Result::NOT_READY) => std::thread::sleep(std::time::Duration::from_millis(1)),
					_ => panic!("Failed to acquire next image"),
				}
			}
		} else {
			unsafe { swapchain_functions.acquire_next_image2(&acquire_info) }
		};

		let (index, _) = if let Ok((index, is_suboptimal)) = acquisition_result {
			if !is_suboptimal {
				(index, graphics_hardware_interface::SwapchainStates::Ok)
			} else {
				(index, graphics_hardware_interface::SwapchainStates::Suboptimal)
			}
		} else {
			(0, graphics_hardware_interface::SwapchainStates::Invalid)
		};

		let extent = if vk_surface_capabilities.current_extent.width != u32::MAX && vk_surface_capabilities.current_extent.height != u32::MAX {
			Extent::rectangle(vk_surface_capabilities.current_extent.width, vk_surface_capabilities.current_extent.height)
		} else {
			Extent::rectangle(swapchain.extent.width, swapchain.extent.height)
		};

		(graphics_hardware_interface::PresentKey {
			image_index: index as u8,
			sequence_index: frame_key.sequence_index,
			swapchain: swapchain_handle,
		}, extent)
	}

	fn resize_image(&mut self, image_handle: crate::ImageHandle, extent: Extent) {
		let image_handles = ImageHandle(image_handle.0).get_all(&self.device.images);

		let current_frame = self.frame_key.sequence_index;

		let handle = image_handles[current_frame as usize];

		self.device.resize_image_internal(handle, extent, current_frame);

		self.device.add_task_to_all_other_frames(Tasks::ResizeImage { handle, extent }, current_frame);
	}

	fn create_command_buffer_recording<'a>(&'a mut self, command_buffer_handle: crate::CommandBufferHandle) -> super::CommandBufferRecording<'a> {
		let frame_key = self.frame_key;

		// Update descriptors before creating command buffer
		self.device.process_tasks(frame_key.sequence_index);

		let pending_buffer_syncs = &self.device.pending_buffer_syncs;
		let buffers = &self.device.buffers;

		let mut pending_buffers = pending_buffer_syncs.lock();

		let buffer_copies = pending_buffers.drain(..).map(|e| {
			let dst_buffer_handle = e;

			let dst_buffer = &buffers[dst_buffer_handle.0 as usize];
			let src_buffer_handle = dst_buffer.staging.unwrap();

			BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size)
		}).collect();

		drop(pending_buffers);

		let pending_image_syncs = &self.device.pending_image_syncs;
		let images = &self.device.images;

		let mut pending_images = pending_image_syncs.lock();

		let image_copies = pending_images.drain(..).map(|e| {
			let dst_image_handle = e;

			let dst_image = &images[dst_image_handle.0 as usize];

			ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
		}).collect();

		drop(pending_images);

		let recording = CommandBufferRecording::new(self.device, command_buffer_handle, buffer_copies, image_copies, frame_key.into());

		recording
	}

	fn get_mut_dynamic_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: crate::DynamicBufferHandle<T>) -> &'a mut T {
		let buffers = &self.device.buffers;
		let frame_key = self.frame_key;

		let handles = BufferHandle(buffer_handle.0).get_all(buffers);

		let handle = handles[frame_key.sequence_index as usize];

		self.device.pending_buffer_syncs.lock().push_back(handle);

		let buffer = handle.access(buffers);
		let buffer = buffer.staging.unwrap().access(buffers);

		unsafe {
			std::mem::transmute(buffer.pointer)
		}
	}

	fn device(&mut self) -> &mut Device {
    	self.device
	}
}
