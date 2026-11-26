use ash::vk::{self, TaggedStructure as _};
use utils::Extent;

use super::{command_buffer::CommandBufferRecording, context::Context};
use crate::{
	FrameKey, HandleLike as _,
	context::Context as _,
	graphics_hardware_interface,
	vulkan::{ImageHandle, Tasks},
};

pub struct Frame<'a> {
	pub(crate) frame_key: FrameKey,
	pub(crate) device: &'a mut Context,
}

impl Frame<'_> {
	pub(crate) fn execute_submission(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		texture_readbacks: smallvec::SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: Option<graphics_hardware_interface::SynchronizerHandle>,
	) {
		let command_buffer = self.device.command_buffers[command_buffer_handle.index() as usize].frames
			[self.frame_key.sequence_index as usize]
			.clone();
		// The submission signals the frame sequence's own synchronizer.
		let synchronizer = synchronizer
			.map(|synchronizer| self.device.get_syncronizer_handles(synchronizer)[self.frame_key.sequence_index as usize]);
		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.command_buffer)];

		let wait_semaphores = present_keys
			.iter()
			.map(|present_key| {
				let swapchain = &self.device.swapchains[present_key.swapchain.index() as usize];
				let semaphore = swapchain.acquire_synchronizers[present_key.sequence_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;
				// Waiting only at the image's first-use stage lets earlier work in the submission run before acquisition;
				// that first barrier's source scope is the same stage, so its layout transition still follows the wait.
				let first_use_stage = swapchain.acquire_wait_stages[present_key.sequence_index as usize];
				let stage_mask = if first_use_stage.is_empty() {
					vk::PipelineStageFlags2::ALL_COMMANDS
				} else {
					first_use_stage
				};
				vk::SemaphoreSubmitInfo::default().semaphore(semaphore).stage_mask(stage_mask)
			})
			.collect::<Vec<_>>();
		// ALL_COMMANDS orders the signal after the pre-present layout transition whatever stage wrote last,
		// as the Khronos swapchain synchronization example allows.
		let signal_semaphores = present_keys
			.iter()
			.map(|present_key| {
				let semaphore = self.device.swapchains[present_key.swapchain.index() as usize].submit_synchronizers
					[present_key.image_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;
				vk::SemaphoreSubmitInfo::default()
					.semaphore(semaphore)
					.stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
			})
			.collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores);
		let execution_completion_fence = synchronizer.map_or(vk::Fence::null(), |handle| {
			self.device.synchronizers[handle.index() as usize].fence
		});
		let vk_queue = self.device.device.vk_queues[command_buffer.vk_queue_index]
			.lock()
			.expect("Failed to lock Vulkan queue for frame submission. The most likely cause is that another thread panicked while holding the queue lock.");
		unsafe {
			self.device
				.queue_submit2(*vk_queue, &[submit_info], execution_completion_fence)
				.expect("Failed to submit command buffer.");
		}
		for handle in texture_readbacks {
			self.device.texture_readbacks.mark_submitted(handle, synchronizer);
		}

		// Binary semaphores are consumed by one wait, so each present waits only on its own image's render semaphore.
		for (presentation, signal) in present_keys.iter().zip(&signal_semaphores) {
			let wait_semaphores = [signal.semaphore];
			let swapchain = &mut self.device.swapchains[presentation.swapchain.index() as usize];
			let swapchains = [swapchain.swapchain];
			let image_indices = [presentation.image_index as u32];
			// Identifiers rise by one per present, so acquisition can wait for the present the frames in flight back.
			if swapchain.feedback.waits_for_presents {
				swapchain.present_id += 1;
			}
			let present_ids = [swapchain.present_id];
			let mut present_id = vk::PresentId2KHR::default().present_ids(&present_ids);
			// The present asks when its first pixel goes out; acquisition reads the answer.
			let timing_infos = swapchain.present_time_domain.map(|(_, time_domain)| {
				[vk::PresentTimingInfoEXT::default()
					.time_domain_id(time_domain)
					.present_stage_queries(vk::PresentStageFlagsEXT::IMAGE_FIRST_PIXEL_OUT)]
			});
			let mut present_timings =
				vk::PresentTimingsInfoEXT::default().timing_infos(timing_infos.as_ref().map_or(&[], |infos| infos));
			let mut present_info = vk::PresentInfoKHR::default()
				.swapchains(&swapchains)
				.wait_semaphores(&wait_semaphores)
				.image_indices(&image_indices);
			if swapchain.feedback.waits_for_presents {
				present_info = present_info.push(&mut present_id);
			}
			if timing_infos.is_some() {
				present_info = present_info.push(&mut present_timings);
			}

			match unsafe { self.device.swapchain.queue_present(*vk_queue, &present_info) } {
				Ok(false) => {}
				Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
					self.device.swapchains[presentation.swapchain.index() as usize].needs_recreation = true;
				}
				Err(error) => panic!(
					"Failed to present a Vulkan swapchain image ({error:?}). The most likely cause is that the surface or the device was lost."
				),
			}
		}
		// The queue lock borrows the context, so release it before marking the fence as pending.
		drop(vk_queue);
		if let Some(handle) = synchronizer {
			self.device.synchronizers[handle.index() as usize].armed = true;
		}
	}

	pub(crate) fn complete_without_submissions(
		&mut self,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) {
		let index =
			self.device.get_syncronizer_handles(synchronizer_handle)[self.frame_key.sequence_index as usize].index() as usize;
		let fence = self.device.synchronizers[index].fence;
		{
			let queue = self.device.device.vk_queues[self.device.queues[0].vk_queue_index]
				.lock()
				.expect("Failed to lock Vulkan queue for empty frame submission. The most likely cause is that another thread panicked while holding the queue lock.");
			unsafe {
				self.device
					.queue_submit2(*queue, &[vk::SubmitInfo2::default()], fence)
					.expect(
						"Failed to submit empty Vulkan frame. The most likely cause is that the completion fence is invalid.",
					);
			}
		}
		self.device.synchronizers[index].armed = true;
	}

	fn create_command_buffer_recording_internal(
		&mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		include_implicit_sync: bool,
	) -> CommandBufferRecording<'_> {
		let sequence_index = self.frame_key.sequence_index;
		// Update descriptors before creating command buffer.
		self.device.process_tasks(sequence_index);

		// Recordings without implicit work leave the pending zero fills to the frame's main recording, which every
		// command of the frame follows.
		let work = if include_implicit_sync {
			self.device.take_implicit_work()
		} else {
			Default::default()
		};

		CommandBufferRecording::new(self.device, command_buffer_handle, self.frame_key.into(), work)
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> crate::FrameKey {
		self.frame_key
	}

	fn map<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		let buffer = self.device.buffers.get_single(buffer_handle.into()).expect(
			"Missing Vulkan buffer. The most likely cause is that the buffer handle came from another context. Use the buffer only with the context that built it.",
		);
		crate::buffer::assert_frame_mappable(buffer.host_access, buffer.copies);
		let pointer = self
			.device
			.typed_buffer_pointer::<T>(buffer_handle, self.frame_key.sequence_index);
		// SAFETY: The pointer addresses initialized POD storage in coherent memory, and starting this frame waited for
		// the last frame that used its sequence's copy, so the CPU has it to itself.
		unsafe { &mut *pointer }
	}

	fn read<T: ?Sized + crate::buffer::BufferContents>(&self, buffer_handle: crate::BufferHandle<T>) -> Option<&T> {
		let buffer = self.device.buffers.get_single(buffer_handle.into()).expect(
			"Missing Vulkan buffer. The most likely cause is that the buffer handle came from another context. Use the buffer only with the context that built it.",
		);
		crate::buffer::assert_frame_readable(buffer.host_access, buffer.copies);
		// The frame that completed when this one started is the one before it in this frame's sequence.
		if self.frame_key.frame_index < u64::from(self.device.frames) {
			return None;
		}
		let pointer = self
			.device
			.typed_buffer_pointer::<T>(buffer_handle, self.frame_key.sequence_index);
		// SAFETY: That frame completed, its recording made its writes visible to host reads, and this frame's
		// commands, the next to use its copy, haven't run.
		Some(unsafe { &*pointer })
	}

	fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: crate::BufferHandle<T>,
		length: usize,
	) {
		crate::context::Context::resize_buffer(&mut *self.device, buffer_handle, length);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	/// Acquires a swapchain image from inside the started frame. The sequence fence was already waited by `start_frame`.
	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: crate::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		self.device
			.acquire_swapchain_image_for_sequence(self.frame_key.sequence_index, swapchain_handle)
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		self.resize_image_storage(image_handle, extent, None);
	}

	fn resize_image_layers(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		array_layers: std::num::NonZeroU32,
	) {
		let image = &self.device.images[image_handle.index() as usize];
		crate::image::assert_resizable_layers(image.cube_array_compatible, array_layers);
		self.resize_image_storage(image_handle, extent, Some(array_layers));
	}

	fn place_image_group(&mut self, group: graphics_hardware_interface::ImageGroupHandle, members: &[crate::ImageGroupMember]) {
		self.device.place_image_group(group, members);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.create_command_buffer_recording_internal(command_buffer_handle, true)
	}

	fn create_command_buffer_recording_without_implicit_sync<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.create_command_buffer_recording_internal(command_buffer_handle, false)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}

impl Frame<'_> {
	/// Resizes the current image now and the other sequences' copies when their frames start.
	///
	/// `array_layers` replaces the layer count, or keeps it when `None`.
	fn resize_image_storage(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		array_layers: Option<std::num::NonZeroU32>,
	) {
		self.device.image_groups.assert_resizable(image_handle);
		let current_frame = self.frame_key.sequence_index;
		let image_handles = ImageHandle::from_index(image_handle.index()).get_all(&self.device.images);
		// Every earlier resize queued its size for the other copies after resizing this one, so matching copies
		// mean no resize toward a different size is still pending.
		if image_handles.iter().all(|handle| {
			let image = &self.device.images[handle.index() as usize];
			image.extent == extent && array_layers.is_none_or(|layers| image.layers == Some(layers))
		}) {
			return;
		}
		let handle = image_handles[(current_frame as usize).rem_euclid(image_handles.len())];

		// Replaced storage is destroyed only once in-flight frames finish, so even a shared static image resizes now.
		self.device.resize_image_internal(handle, extent, array_layers);

		// Other sequences' copies may still be in flight, so they are rebuilt when their own frame starts.
		for offset in 1..image_handles.len() as u8 {
			let task = Tasks::ResizeImage {
				handle,
				extent,
				array_layers,
			};
			let frame = (current_frame + offset) % self.device.frames;
			self.device.tasks.push(crate::vulkan::Task::new(task, Some(frame)));
		}
	}
}
