use ash::vk::{self};
use utils::Extent;

use crate::{
	device::Device as _,
	graphics_hardware_interface,
	vulkan::{BufferCopy, BufferHandle, Handle, HandleLike as _, ImageCopy, ImageHandle, Swapchain, Synchronizer, Tasks},
	CommandBufferRecording, Device, FrameKey,
};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut Device,
	acquired_swapchains: Vec<crate::PresentKey>,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut Device, frame_key: FrameKey) -> Self {
		Self {
			frame_key,
			device,
			acquired_swapchains: Vec::new(),
		}
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'f>
		= CommandBufferRecording<'f>
	where
		Self: 'f;

	fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn acquire_swapchain_image(&mut self, swapchain_handle: crate::SwapchainHandle) -> (crate::PresentKey, utils::Extent) {
		let swapchains = &self.device.swapchains;
		let synchronizers = &self.device.synchronizers;

		let swapchain = &swapchains[swapchain_handle.0 as usize];

		let s = swapchain.max_image_count as u64;
		let m = swapchain.min_image_count as u64;

		let frame_key = self.frame_key;

		let swapchain_frame_synchronizer =
			swapchain.acquire_synchronizers[frame_key.sequence_index as usize].access(synchronizers);

		let semaphore = swapchain_frame_synchronizer.semaphore;

		// Use our own waiting technique if only one image (s - m == 0) can be acquired at a time, since
		let use_vulkan_timeout = s - m != 0;

		let acquire_info = vk::AcquireNextImageInfoKHR::default()
			.swapchain(swapchain.swapchain)
			.timeout(if use_vulkan_timeout { u64::MAX } else { 0 })
			.semaphore(semaphore)
			.device_mask(1)
			.fence(swapchain_frame_synchronizer.fence);

		let mut vk_surface_present_mode = vk::SurfacePresentModeEXT::default().present_mode(swapchain.vk_present_mode);

		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
			.push_next(&mut vk_surface_present_mode)
			.surface(swapchain.surface);

		let mut vk_present_modes = [swapchain.vk_present_mode];

		let mut vk_surface_present_mode_compatibility =
			vk::SurfacePresentModeCompatibilityEXT::default().present_modes(&mut vk_present_modes);

		let mut vk_surface_capabilities =
			vk::SurfaceCapabilities2KHR::default().push_next(&mut vk_surface_present_mode_compatibility);

		unsafe {
			self.device
				.surface_capabilities
				.get_physical_device_surface_capabilities2(
					self.device.physical_device,
					&vk_surface_info,
					&mut vk_surface_capabilities,
				)
				.expect("No surface capabilities")
		};

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

		let (index, swapchain_state) = if let Ok((index, is_suboptimal)) = acquisition_result {
			if !is_suboptimal {
				(index, graphics_hardware_interface::SwapchainStates::Ok)
			} else {
				(index, graphics_hardware_interface::SwapchainStates::Suboptimal)
			}
		} else {
			(0, graphics_hardware_interface::SwapchainStates::Invalid)
		};

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: index as u8,
			sequence_index: frame_key.sequence_index,
			swapchain: swapchain_handle,
		};

		if swapchain_state != graphics_hardware_interface::SwapchainStates::Invalid
			&& !self.acquired_swapchains.contains(&present_key)
		{
			self.acquired_swapchains.push(present_key);
		}

		let extent = if vk_surface_capabilities.current_extent.width != u32::MAX
			&& vk_surface_capabilities.current_extent.height != u32::MAX
		{
			Extent::rectangle(
				vk_surface_capabilities.current_extent.width,
				vk_surface_capabilities.current_extent.height,
			)
		} else {
			Extent::rectangle(swapchain.extent.width, swapchain.extent.height)
		};

		(present_key, extent)
	}

	fn resize_image(&mut self, image_handle: crate::ImageHandle, extent: Extent) {
		let image_handles = ImageHandle(image_handle.0).get_all(&self.device.images);

		let current_frame = self.frame_key.sequence_index;

		let handle = image_handles[current_frame as usize];

		self.device.resize_image_internal(handle, extent, current_frame);

		self.device
			.add_task_to_all_other_frames(Tasks::ResizeImage { handle, extent }, current_frame);
	}

	fn create_command_buffer_recording(&mut self, command_buffer_handle: crate::CommandBufferHandle) -> Self::CBR<'_> {
		let frame_key = self.frame_key;

		// Update descriptors before creating command buffer
		self.device.process_tasks(frame_key.sequence_index);

		// When PERSISTENT_WRITE is enabled, memcpy from each dynamic buffer's
		// persistent source buffer into the current frame's staging buffer, then
		// enqueue the staging→GPU copy. This ensures every frame gets the latest
		// data even if the CPU didn't write this frame.
		if super::buffer::PERSISTENT_WRITE {
			let buffers = &self.device.buffers;

			for master_handle in &self.device.persistent_write_dynamic_buffers {
				let all_handles = BufferHandle(master_handle.0).get_all(buffers);
				let frame_buffer_handle = all_handles[frame_key.sequence_index as usize];
				let frame_buffer = frame_buffer_handle.access(buffers);

				let source_handle = frame_buffer
					.source
					.expect("Persistent write dynamic buffer must have a source");
				let staging_handle = frame_buffer
					.staging
					.expect("Persistent write dynamic buffer must have per-frame staging");

				let source_buffer = source_handle.access(buffers);
				let staging_buffer = staging_handle.access(buffers);
				let size = frame_buffer.size;

				// CPU-side memcpy: source → per-frame staging
				unsafe {
					std::ptr::copy_nonoverlapping(source_buffer.pointer, staging_buffer.pointer, size);
				}

				// Enqueue staging → GPU copy
				self.device.pending_buffer_syncs.insert(frame_buffer_handle);
			}
		}

		let pending_buffers = &mut self.device.pending_buffer_syncs;
		let buffers = &self.device.buffers;

		let buffer_copies: Vec<_> = pending_buffers
			.drain()
			.map(|e| {
				let dst_buffer_handle = e;

				let dst_buffer = &buffers[dst_buffer_handle.0 as usize];
				let src_buffer_handle = dst_buffer.staging.unwrap();

				BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size)
			})
			.collect();

		let pending_images = &mut self.device.pending_image_syncs;
		let images = &self.device.images;

		let image_copies: Vec<_> = pending_images
			.drain()
			.map(|e| {
				let dst_image_handle = e;

				let dst_image = &images[dst_image_handle.0 as usize];

				ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
			})
			.collect();

		let mut recording = CommandBufferRecording::new(self.device, command_buffer_handle, frame_key.into());

		recording.sync_buffers(buffer_copies.iter().copied());
		recording.sync_textures(image_copies.iter().copied());

		recording
	}

	fn get_mut_dynamic_buffer_slice<T: Copy>(&mut self, buffer_handle: crate::DynamicBufferHandle<T>) -> &mut T {
		let buffers = &self.device.buffers;
		let frame_key = self.frame_key;

		let handles = BufferHandle(buffer_handle.0).get_all(buffers);
		let handle = handles[frame_key.sequence_index as usize];
		let buffer = handle.access(buffers);

		if super::buffer::PERSISTENT_WRITE {
			if let Some(source_handle) = buffer.source {
				// Return the persistent source buffer's pointer. The user writes
				// here and every frame the data is automatically memcpy'd to per-frame
				// staging and then GPU-copied. No need to push to pending_buffer_syncs.
				let source_buffer = source_handle.access(buffers);
				return unsafe { std::mem::transmute(source_buffer.pointer) };
			}
		}

		// Fallback: original behavior for non-persistent-write buffers
		self.device.pending_buffer_syncs.insert(handle);

		let staging_buffer = buffer.staging.unwrap().access(buffers);

		unsafe { std::mem::transmute(staging_buffer.pointer) }
	}

	fn execute<'s, 'f>(
		&mut self,
		cbr: <Self::CBR<'f> as crate::command_buffer::CommandBufferRecording>::Result<'s>,
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) where
		Self: 'f, {
		let (command_buffer_handle, states, present_keys) = cbr;

		let command_buffer = self.device.command_buffers[0].frames[0];

		let command_buffers = [command_buffer.command_buffer];

		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffers[0])];

		let wait_for_synchronizer_handles: [graphics_hardware_interface::SynchronizerHandle; 0] = [];

		let wait_semaphores = wait_for_synchronizer_handles
			.iter()
			.map(|&synchronizer| {
				vk::SemaphoreSubmitInfo::default()
					.semaphore(self.get_synchronizer(synchronizer).semaphore)
					.stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE | vk::PipelineStageFlags2::TRANSFER)
			})
			.chain(present_keys.iter().map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let semaphore = swapchain.acquire_synchronizers[present_key.sequence_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;

				vk::SemaphoreSubmitInfo::default()
					.semaphore(semaphore)
					.stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
			}))
			.collect::<Vec<_>>();

		let signal_synchronizer_handles: [graphics_hardware_interface::SynchronizerHandle; 0] = [];

		let signal_semaphores = signal_synchronizer_handles
			.iter()
			.map(|&synchronizer| {
				vk::SemaphoreSubmitInfo::default()
					.semaphore(self.get_synchronizer(synchronizer).semaphore)
					.stage_mask(vk::PipelineStageFlags2::empty())
			})
			.chain(present_keys.iter().map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let presentable_image_handle = self.get_presentable_swapchain_image_handle(*present_key);
				let wait_stage = states
					.get(&Handle::Image(presentable_image_handle))
					.map(|state| state.stage)
					.unwrap_or(vk::PipelineStageFlags2::ALL_COMMANDS);

				vk::SemaphoreSubmitInfo::default()
					.semaphore(
						swapchain.submit_synchronizers[present_key.image_index as usize]
							.access(&self.device.synchronizers)
							.semaphore,
					)
					.stage_mask(wait_stage)
			}))
			.collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores);

		let execution_completion_synchronizer = &self.get_synchronizer(synchronizer);

		let vk_queue = command_buffer.vk_queue;

		unsafe {
			self.device
				.device
				.queue_submit2(vk_queue, &[submit_info], execution_completion_synchronizer.fence)
				.expect("Failed to submit command buffer.");
		}

		for presentation in present_keys {
			let swapchain = self.get_swapchain(presentation.swapchain);

			let wait_semaphores = signal_synchronizer_handles
				.iter()
				.map(|synchronizer| self.get_synchronizer(*synchronizer).semaphore)
				.chain(present_keys.iter().map(|present_key| {
					self.get_swapchain(present_key.swapchain).submit_synchronizers[present_key.image_index as usize]
						.access(&self.device.synchronizers)
						.semaphore
				}))
				.collect::<Vec<_>>();

			let swapchains = [swapchain.swapchain];
			let image_indices = [presentation.image_index as u32];

			let mut results = [vk::Result::default()];

			let present_info = vk::PresentInfoKHR::default()
				.results(&mut results)
				.swapchains(&swapchains)
				.wait_semaphores(&wait_semaphores)
				.image_indices(&image_indices);

			let _ = unsafe {
				self.device
					.swapchain
					.queue_present(vk_queue, &present_info)
					.expect("No present")
			};

			if !results.iter().all(|result| *result == vk::Result::SUCCESS) {
				dbg!("Some error occurred during presentation");
			}
		}

		for (k, v) in states {
			self.device.states.insert(k, v);
		}
	}
}

impl<'a> Frame<'a> {
	pub(crate) fn get_synchronizer(
		&self,
		syncronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> &Synchronizer {
		&self.device.synchronizers
			[self.device.get_syncronizer_handles(syncronizer_handle)[self.frame_key.sequence_index as usize].0 as usize]
	}

	pub(crate) fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.device.swapchains[swapchain_handle.0 as usize]
	}

	pub(crate) fn get_presentable_swapchain_image_handle(
		&self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> ImageHandle {
		let swapchain = self.get_swapchain(present_key.swapchain);
		swapchain.native_images[present_key.image_index as usize]
	}
}
