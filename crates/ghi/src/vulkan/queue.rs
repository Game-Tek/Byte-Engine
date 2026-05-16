use ash::vk;
use utils::hash::HashMap;

use super::{context::Context, TransitionState};
use crate::frame::Frame as _;
use crate::vulkan::Frame;

/// The `Queue` struct provides owned access to a Vulkan queue through the GHI queue API.
pub struct Queue {
	pub(crate) device: std::ptr::NonNull<Context>,
	pub(crate) queue_handle: crate::QueueHandle,
	pub(crate) vk_queue: vk::Queue,
	pub(crate) queue_family_index: u32,
	pub(crate) _queue_index: u32,
}

unsafe impl Send for Queue {}

/// The `QueueReference` struct provides borrowed access to a Vulkan queue while a context remains mutably borrowed.
pub struct QueueReference<'a> {
	pub(crate) device: &'a mut Context,
	pub(crate) queue_handle: crate::QueueHandle,
}

/// The `Execution` struct gathers Vulkan command-buffer recordings before queue submission.
pub struct Execution<'a> {
	frame: Option<Frame<'a>>,
	completed_frame: Option<crate::FrameKey>,
	command_buffers: Vec<(crate::CommandBufferHandle, HashMap<super::Handles, TransitionState>)>,
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

impl crate::queue::Queue for Queue {
	type Frame<'a> = Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> crate::CommandBufferHandle {
		let queue_handle = self.queue_handle;
		unsafe { self.device.as_mut() }.create_command_buffer(name, queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u32,
		synchronizer_handle: crate::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Self::Frame<'a>> {
		unsafe { self.device.as_mut() }.start_frame(index, synchronizer_handle)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest>,
		wait_for: &[crate::SynchronizerHandle],
		synchronizer: crate::SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[crate::PresentKey]>,
	{
		let device = unsafe { self.device.as_mut() };
		for &wait_synchronizer in wait_for {
			device.wait_for_synchronizer(wait_synchronizer);
		}

		let frame = frame.map(|frame| device.start_frame(frame.index, frame.synchronizer));
		let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
		let frame = frame.map(|frame| frame.frame);
		let mut execution = Execution {
			frame,
			completed_frame,
			command_buffers: Vec::new(),
		};
		let present_keys = execute(&mut execution);
		let present_keys = present_keys.as_ref();

		let Some(mut frame) = execution.frame else {
			return;
		};
		let last_index = execution.command_buffers.len().saturating_sub(1);
		for (index, (command_buffer, states)) in execution.command_buffers.into_iter().enumerate() {
			let present_keys = if index == last_index { present_keys } else { &[] };
			frame.execute_submission(command_buffer, states, present_keys, synchronizer);
		}
	}
}

impl crate::queue::Queue for QueueReference<'_> {
	type Frame<'a> = Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> crate::CommandBufferHandle {
		self.device.create_command_buffer(name, self.queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u32,
		synchronizer_handle: crate::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Self::Frame<'a>> {
		self.device.start_frame(index, synchronizer_handle)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest>,
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

		let Some(mut frame) = execution.frame else {
			return;
		};
		let last_index = execution.command_buffers.len().saturating_sub(1);
		for (index, (command_buffer, states)) in execution.command_buffers.into_iter().enumerate() {
			let present_keys = if index == last_index { present_keys } else { &[] };
			frame.execute_submission(command_buffer, states, present_keys, synchronizer);
		}
	}
}
