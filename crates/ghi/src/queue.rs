use crate::{CommandBufferHandle, FrameKey, PresentKey, SynchronizerHandle};

/// The `FrameRequest` struct identifies a frame that should be opened for queue execution.
#[derive(Clone, Copy)]
pub struct FrameRequest {
	pub index: u32,
	pub synchronizer: SynchronizerHandle,
}

/// The `StartedFrame` struct exists to pair an opened frame with the previous frame that became reusable.
pub struct StartedFrame<F> {
	pub frame: F,
	pub completed_frame: Option<FrameKey>,
}

impl<F> StartedFrame<F> {
	pub fn new(frame: F, completed_frame: Option<FrameKey>) -> Self {
		Self { frame, completed_frame }
	}
}

pub fn completed_frame_key(index: u32, frames_in_flight: u8) -> Option<FrameKey> {
	let frames_in_flight = frames_in_flight as u32;
	index.checked_sub(frames_in_flight).map(|frame_index| FrameKey {
		frame_index,
		sequence_index: (frame_index % frames_in_flight) as u8,
	})
}

/// The `QueueExecution` trait scopes command-buffer recordings created during one queue submission.
pub trait QueueExecution<'a> {
	type Frame: crate::frame::Frame<'a>;

	/// Returns the frame opened for this queue execution, if one was requested.
	fn frame(&mut self) -> Option<&mut Self::Frame>;

	/// Returns the previous frame that was completed before this execution began, if any.
	fn completed_frame(&self) -> Option<FrameKey>;

	/// Creates a command-buffer recording, passes it to `record`, and schedules it for submission.
	fn record<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record;
}

/// The `Queue` trait provides the queue-level entry points needed to build and submit graphics work.
pub trait Queue {
	type Frame<'a>: crate::frame::Frame<'a>;
	type Execution<'a>: QueueExecution<'a, Frame = Self::Frame<'a>>;

	/// Creates a command buffer which will execute commands on the provided queue.
	///
	/// Commands can be recorded onto it by starting a recording from a `Frame` or by calling `Device::create_command_buffer_recording` if the command buffer is not for performing per frame workloads.
	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle;

	/// Starts a new frame by waiting for these sequence frame's synchronizers.
	/// The returned frame allows safe access to the frame's resources and it's operations.
	fn start_frame<'a>(&'a mut self, index: u32, synchronizer_handle: SynchronizerHandle) -> StartedFrame<Self::Frame<'a>>;

	/// Opens the requested frame, lets the closure record submission work, and submits it on this queue.
	fn execute<'a, P>(
		&'a mut self,
		frame: Option<FrameRequest>,
		wait_for: &[SynchronizerHandle],
		synchronizer: crate::SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[PresentKey]>;
}
