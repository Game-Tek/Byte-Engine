use super::context::{Device, Execution};
use crate::{CommandBufferHandle, PresentKey, QueueHandle, SynchronizerHandle};

/// The `Queue` struct provides borrowed DX12 queue submission through the shared GHI queue API.
pub struct Queue<'a> {
	pub(crate) device: &'a mut Device,
	pub(crate) queue_handle: QueueHandle,
}

impl Execution<'_> {
	/// Completes one queue execution after the recording closure releases its command-buffer borrows.
	fn finish(mut self, synchronizer: SynchronizerHandle, sequence_index: Option<u8>, present_keys: &[PresentKey]) {
		let frame = self.frame.as_mut().expect(
			"Frame is required to finish a recorded DX12 execution. The most likely cause is that a frameless execution accepted a command buffer.",
		);
		let device = frame.device_mut();
		let sequence_index = sequence_index.expect(
			"Missing DX12 frame sequence. The most likely cause is that frame execution finalization lost its frame request.",
		);
		device.validate_present_keys(self.queue_handle, sequence_index, present_keys);
		device.validate_present_preparation(&self.prepared_present_keys, present_keys);
		// Keep the handles until the whole batch enters the native queue so unwinding restores journals in reverse order.
		let readbacks = device.execute_command_buffers(self.queue_handle, &self.command_buffers, sequence_index);
		for &present_key in present_keys {
			device.present_swapchain(present_key);
		}
		device.complete_command_buffer_execution(
			self.queue_handle,
			&self.command_buffers,
			synchronizer,
			sequence_index,
			readbacks,
		);
		device.complete_present_submission(!present_keys.is_empty());
		self.command_buffers.clear();
		self.prepared_present_keys.clear();
	}
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
		assert!(
			!self.command_buffers.contains(&command_buffer_handle),
			"DX12 command buffer is already scheduled. The most likely cause is that one execution recorded the same handle more than once."
		);
		for &present_key in present_keys {
			assert!(
				!self.prepared_present_keys.contains(&present_key),
				"Duplicate DX12 present key. The most likely cause is that one execution scheduled the same acquired image more than once."
			);
			assert_eq!(
				present_keys.iter().filter(|candidate| **candidate == present_key).count(),
				1,
				"Duplicate DX12 present key. The most likely cause is that one recording listed the same acquired image more than once."
			);
		}
		let frame = self.frame.as_mut().expect(
			"Frame is required to record a DX12 frame command buffer. The most likely cause is that Queue::execute was called without a frame request.",
		);
		let sequence_index = crate::frame::Frame::key(frame).sequence_index;
		frame
			.device_mut()
			.validate_present_keys(self.queue_handle, sequence_index, present_keys);
		frame
			.device_mut()
			.validate_command_buffer_for_execution(command_buffer_handle, self.queue_handle);
		self.prepared_present_keys.extend_from_slice(present_keys);
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
		self.device.validate_queue_submission_state();
		if let Some(frame) = frame.as_ref() {
			assert_eq!(
				frame.synchronizer, _synchronizer,
				"Invalid DX12 frame synchronizer. The most likely cause is that the execution would signal a different fence than the one used to retire this frame sequence."
			);
		}
		for &wait_synchronizer in _wait_for {
			self.device.queue_wait_for_synchronizer(self.queue_handle, wait_synchronizer);
		}
		match frame {
			Some(frame) => {
				let frames = self.device.frames;
				let sequence_index = (frame.index % u64::from(frames)) as u8;
				let started_frame = crate::queue::StartedFrame::new(
					self.device.start_frame(frame.index, frame.synchronizer),
					crate::queue::completed_frame_key(frame.index, frames),
				);
				let mut execution = Execution {
					frame: Some(started_frame.frame),
					completed_frame: started_frame.completed_frame,
					command_buffers: smallvec::SmallVec::new(),
					prepared_present_keys: smallvec::SmallVec::new(),
					queue_handle: self.queue_handle,
				};
				let present_keys = execute(&mut execution);
				execution.finish(_synchronizer, Some(sequence_index), present_keys.as_ref());
			}
			None => {
				let mut execution = Execution {
					frame: None,
					completed_frame: None,
					command_buffers: smallvec::SmallVec::new(),
					prepared_present_keys: smallvec::SmallVec::new(),
					queue_handle: self.queue_handle,
				};
				let present_keys = execute(&mut execution);
				assert!(
					execution.command_buffers.is_empty(),
					"Frameless DX12 execution recorded a command buffer. The most likely cause is that the recording API bypassed its frame requirement."
				);
				self.device.validate_present_keys(self.queue_handle, 0, present_keys.as_ref());
				self.device
					.validate_present_preparation(&execution.prepared_present_keys, present_keys.as_ref());
				drop(execution);
				let readbacks = self.device.execute_command_buffers(self.queue_handle, &[], 0);
				for &present_key in present_keys.as_ref() {
					self.device.present_swapchain(present_key);
				}
				self.device
					.complete_command_buffer_execution(self.queue_handle, &[], _synchronizer, 0, readbacks);
				self.device.complete_present_submission(!present_keys.as_ref().is_empty());
			}
		}
	}
}
