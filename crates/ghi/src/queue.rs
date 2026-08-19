/// The `Queue` trait provides the queue-level entry points needed to build and submit graphics work.
pub trait Queue {
	type Frame<'a>: crate::frame::Frame<'a>;
	type Execution<'a>: QueueExecution<'a, Frame = Self::Frame<'a>>;

	/// Creates a command buffer for this queue.
	///
	/// Record per-frame work through [`crate::Frame`]. Use the device recording API
	/// for work that does not belong to a frame.
	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle;

	/// Starts a frame after waiting for its sequence synchronizers.
	///
	/// The returned frame provides safe access to its resources and operations.
	fn start_frame<'a>(
		&'a mut self,
		frame_identity: u64,
		synchronizer_handle: SynchronizerHandle,
	) -> StartedFrame<Self::Frame<'a>>;

	/// Opens the requested frame, lets the closure record submission work, and submits it on this queue.
	fn execute<'a, P>(
		&'a mut self,
		frame: Option<FrameRequest<'a>>,
		wait_for: &[SynchronizerHandle],
		synchronizer: crate::SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[PresentKey]>;
}

/// The `FrameRequest` struct identifies a frame and the allocator for its temporary host objects.
#[derive(Clone, Copy)]
pub struct FrameRequest<'a> {
	/// The monotonically increasing identity of this submission frame.
	pub index: u64,
	pub synchronizer: SynchronizerHandle,
	pub(crate) allocator: &'a dyn std::alloc::Allocator,
}

impl FrameRequest<'static> {
	/// Creates a frame request that uses the global allocator.
	pub fn new(index: u64, synchronizer: SynchronizerHandle) -> Self {
		Self::new_in(index, synchronizer, &std::alloc::Global)
	}
}

impl<'a> FrameRequest<'a> {
	/// Creates a frame request that uses `allocator` for frame-owned host memory.
	pub fn new_in(index: u64, synchronizer: SynchronizerHandle, allocator: &'a dyn std::alloc::Allocator) -> Self {
		Self {
			index,
			synchronizer,
			allocator,
		}
	}
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

/// Returns the completed frame after its GPU sequence becomes reusable.
///
/// `None` is returned only during startup, before every sequence has been submitted once.
pub fn completed_frame_key(frame_identity: u64, frames_in_flight: u8) -> Option<FrameKey> {
	let frames_in_flight = u64::from(frames_in_flight);
	frame_identity.checked_sub(frames_in_flight).map(|frame_identity| FrameKey {
		frame_index: frame_identity,
		sequence_index: (frame_identity % frames_in_flight) as u8,
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

	/// Creates a command-buffer recording that also performs presentation preparation before submission.
	fn record_with_present_keys<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
		_present_keys: &[PresentKey],
		record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
	) where
		Self::Frame: 'record,
	{
		self.record(command_buffer_handle, record);
	}
}

#[cfg(test)]
mod tests {
	use super::completed_frame_key;

	#[test]
	fn completed_frame_is_absent_only_while_sequences_are_first_submitted() {
		assert_eq!(completed_frame_key(0, 2), None);
		assert_eq!(completed_frame_key(1, 2), None);

		let completed_frame = completed_frame_key(2, 2).unwrap();

		assert_eq!(completed_frame.frame_index, 0);
		assert_eq!(completed_frame.sequence_index, 0);
	}

	#[test]
	fn completed_frame_identity_survives_the_former_u32_rollover() {
		let frame_after_u32_max = u64::from(u32::MAX) + 1;
		let completed_frame = completed_frame_key(frame_after_u32_max, 2).unwrap();

		assert_eq!(completed_frame.frame_index, u64::from(u32::MAX) - 1);
		assert_eq!(completed_frame.sequence_index, 0);
	}
}

use crate::{CommandBufferHandle, FrameKey, PresentKey, SynchronizerHandle};
