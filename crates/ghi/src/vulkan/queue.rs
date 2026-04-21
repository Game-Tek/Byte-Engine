use ash::vk;
use utils::hash::HashMap;

use crate::frame::Frame as _;
use crate::vulkan::{CommandBufferRecording, Frame};

use super::TransitionState;

pub struct Queue {
	pub(crate) vk_queue: vk::Queue,
	pub(crate) queue_family_index: u32,
	pub(crate) _queue_index: u32,
}

/// The `Execution` struct gathers Vulkan command-buffer recordings before queue submission.
pub struct Execution<'a> {
	frame: Option<Frame<'a>>,
	command_buffers: Vec<(crate::CommandBufferHandle, HashMap<crate::PrivateHandles, TransitionState>)>,
}

impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
	type Frame = Frame<'a>;

	fn frame(&mut self) -> Option<&mut Self::Frame> {
		self.frame.as_mut()
	}

	fn record<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		let frame = self.frame.as_mut().expect(
			"Frame is required to record a frame command buffer. The most likely cause is that Queue::execute was called with None and the closure tried to record frame work.",
		);
		let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
		record(&mut command_buffer);
		self.command_buffers.push(command_buffer.into_submission());
	}
}

impl crate::queue::Queue for Queue {
	type Frame<'a> = Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest>,
		_wait_for: &[crate::SynchronizerHandle],
		synchronizer: crate::SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[crate::PresentKey]>,
	{
		let frame = match frame {
			Some(_) => panic!(
				"Vulkan queue execution cannot open a frame from the queue yet. The most likely cause is that the Vulkan queue wrapper does not currently own device access."
			),
			None => None,
		};
		let mut execution = Execution {
			frame,
			command_buffers: Vec::new(),
		};
		let present_keys = execute(&mut execution);

		let Some(mut frame) = execution.frame else {
			return;
		};
		let last_index = execution.command_buffers.len().saturating_sub(1);
		for (index, (command_buffer, states)) in execution.command_buffers.into_iter().enumerate() {
			let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
			frame.execute_submission(command_buffer, states, present_keys, synchronizer);
		}
	}
}
