use utils::hash::HashMap;

use super::{context::Context, BufferTransitionState, Handles, TransitionState};
use crate::frame::Frame as _;
use crate::vulkan::Frame;

/// The `Queue` struct provides borrowed Vulkan submission while its context remains borrowed.
pub struct Queue<'a> {
	pub(crate) device: &'a mut Context,
	pub(crate) queue_handle: crate::QueueHandle,
}

/// The `Execution` struct gathers Vulkan command-buffer recordings before queue submission.
pub struct Execution<'a> {
	frame: Option<Frame<'a>>,
	completed_frame: Option<crate::FrameKey>,
	command_buffers: Vec<(
		crate::CommandBufferHandle,
		HashMap<Handles, TransitionState>,
		HashMap<Handles, Vec<BufferTransitionState>>,
		smallvec::SmallVec<[crate::TextureCopyHandle; 4]>,
	)>,
}

impl Drop for Execution<'_> {
	fn drop(&mut self) {
		let Some(frame) = self.frame.as_mut() else {
			return;
		};
		for (_, _, _, readbacks) in &self.command_buffers {
			for &handle in readbacks {
				frame.device_mut().cancel_texture_readback(handle);
			}
		}
	}
}

impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
	type Frame = Frame<'a>;

	fn frame(&mut self) -> Option<&mut Self::Frame> {
		self.frame.as_mut()
	}

	fn completed_frame(&self) -> Option<crate::FrameKey> {
		self.completed_frame
	}

	fn record<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		self.record_with_present_keys(command_buffer_handle, &[], record);
	}

	fn record_with_present_keys<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		present_keys: &[crate::PresentKey],
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		let frame = self.frame.as_mut().expect(
			"Frame is required to record a frame command buffer. The most likely cause is that Queue::execute was called with None and the closure tried to record frame work.",
		);
		let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
		record(&mut command_buffer);
		self.command_buffers.push(command_buffer.into_submission(present_keys));
	}
}

impl crate::queue::Queue for Queue<'_> {
	type Frame<'a> = Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> crate::CommandBufferHandle {
		self.device.create_command_buffer(name, self.queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: crate::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Self::Frame<'a>> {
		self.device.start_frame(index, synchronizer_handle)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest<'a>>,
		wait_for: &[crate::SynchronizerHandle],
		synchronizer: crate::SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[crate::PresentKey]>,
	{
		for &wait_synchronizer in wait_for {
			self.device.wait_for_synchronizer(wait_synchronizer);
		}

		let frame = frame.map(|frame| self.device.start_frame(frame.index, frame.synchronizer));
		let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
		let frame = frame.map(|frame| frame.frame);
		let mut execution = Execution {
			frame,
			completed_frame,
			command_buffers: Vec::new(),
		};
		let present_keys = execute(&mut execution);
		let present_keys = present_keys.as_ref();

		let Some(mut frame) = execution.frame.take() else {
			return;
		};
		let command_buffers = std::mem::take(&mut execution.command_buffers);
		let last_index = command_buffers.len().saturating_sub(1);
		if command_buffers.is_empty() {
			frame.complete_without_submissions(synchronizer);
			return;
		}
		for (index, (command_buffer, states, buffer_states, texture_readbacks)) in command_buffers.into_iter().enumerate() {
			let present_keys = if index == last_index { present_keys } else { &[] };
			let completion_synchronizer = (index == last_index).then_some(synchronizer);
			frame.execute_submission(
				command_buffer,
				states,
				buffer_states,
				texture_readbacks,
				present_keys,
				completion_synchronizer,
			);
		}
	}
}
