use super::context::{Device, Execution};
use crate::{CommandBufferHandle, PresentKey, QueueHandle, SynchronizerHandle};

/// The `Queue` struct provides borrowed DX12 queue submission through the shared GHI queue API.
pub struct Queue<'a> {
	pub(crate) device: &'a mut Device,
	pub(crate) queue_handle: QueueHandle,
}

impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
	type Frame = super::Frame<'a>;

	fn frame(&mut self) -> Option<&mut Self::Frame> {
		self.frame.as_mut()
	}

	fn completed_frame(&self) -> Option<crate::FrameKey> {
		self.completed_frame
	}

	fn record<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		self.record_with_present_keys(command_buffer_handle, &[], record);
	}

	fn record_with_present_keys<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
		present_keys: &[PresentKey],
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		let frame = self.frame.as_mut().expect(
			"Frame is required to record a DX12 frame command buffer. The most likely cause is that Queue::execute was called without a frame request.",
		);
		let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
		record(&mut command_buffer);
		// Present keys are recorded after user commands so swapchain proxy images written by compute passes
		// are copied to the native backbuffer before the command list is submitted.
		command_buffer.record_present_preparation(present_keys);
		command_buffer.finish_for_submission();
		self.command_buffers.push(command_buffer_handle);
	}
}

impl crate::queue::Queue for Queue<'_> {
	type Frame<'a> = super::Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle {
		self.device.create_command_buffer(name, self.queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: SynchronizerHandle,
	) -> crate::queue::StartedFrame<Self::Frame<'a>> {
		let frames = self.device.frames;
		crate::queue::StartedFrame::new(
			self.device.start_frame(index, synchronizer_handle),
			crate::queue::completed_frame_key(index, frames),
		)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest<'a>>,
		_wait_for: &[SynchronizerHandle],
		_synchronizer: SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[PresentKey]>,
	{
		let mut device_pointer = std::ptr::NonNull::from(&mut *self.device);
		for &wait_synchronizer in _wait_for {
			self.device.wait_for_synchronizer(wait_synchronizer);
		}
		let frames = self.device.frames;
		let frame_sequence_index = frame.as_ref().map(|frame| (frame.index % u64::from(frames)) as u8);
		let frame = frame.map(|frame| {
			crate::queue::StartedFrame::new(
				self.device.start_frame(frame.index, frame.synchronizer),
				crate::queue::completed_frame_key(frame.index, frames),
			)
		});
		let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
		let frame = frame.map(|frame| frame.frame);
		let mut execution = Execution {
			frame,
			completed_frame,
			command_buffers: smallvec::SmallVec::new(),
		};
		let present_keys = execute(&mut execution);
		let should_complete_empty_frame = execution.frame.is_some() && execution.command_buffers.is_empty();
		let command_buffers = std::mem::take(&mut execution.command_buffers);
		drop(execution);
		if command_buffers.is_empty() {
			if should_complete_empty_frame {
				if let Some(sequence_index) = frame_sequence_index {
					unsafe {
						device_pointer
							.as_mut()
							.complete_synchronizer_for_sequence_from_cpu(_synchronizer, sequence_index);
					}
				}
			}
			return;
		}
		for command_buffer in command_buffers {
			let submitted = unsafe { device_pointer.as_mut().submit_command_buffer(command_buffer, _synchronizer) };
			if !submitted {
				unsafe {
					let device = device_pointer.as_mut();
					device.abandon_texture_readbacks_for_command_buffer(command_buffer);
					device.requeue_recorded_texture_syncs_for_command_buffer(command_buffer);
					device.rollback_command_buffer_resource_states(command_buffer);
				}
			}
		}
		for present_key in present_keys.as_ref() {
			unsafe {
				device_pointer.as_mut().present_swapchain(*present_key);
			}
		}
	}
}
