use super::context::{Device, Execution};
use crate::{CommandBufferHandle, PresentKey, QueueHandle, SynchronizerHandle};

/// The `Queue` struct exists to expose DX12 queue submission through the shared GHI queue API.
pub struct Queue {
	pub(crate) device: std::ptr::NonNull<Device>,
	pub(crate) queue_handle: QueueHandle,
}

unsafe impl Send for Queue {}

/// The `QueueReference` struct exists to expose borrowed DX12 queue submission through the shared GHI queue API.
pub struct QueueReference<'a> {
	pub(crate) device: &'a mut Device,
	pub(crate) queue_handle: QueueHandle,
}

impl Queue {
	fn device_mut(&mut self) -> &mut Device {
		unsafe { self.device.as_mut() }
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
		let frame = self.frame.as_mut().expect(
			"Frame is required to record a DX12 frame command buffer. The most likely cause is that Queue::execute was called without a frame request.",
		);
		let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
		record(&mut command_buffer);
		// Present keys are recorded after user commands so swapchain proxy images written by compute passes
		// are copied to the native backbuffer before the command list is submitted.
		command_buffer.record_present_preparation(present_keys);
		self.command_buffers.push(command_buffer_handle);
	}
}

impl crate::queue::Queue for Queue {
	type Frame<'a> = super::Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle {
		let queue_handle = self.queue_handle;
		self.device_mut().create_command_buffer(name, queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u32,
		synchronizer_handle: SynchronizerHandle,
	) -> crate::queue::StartedFrame<Self::Frame<'a>> {
		let frames = self.device_mut().frames;
		crate::queue::StartedFrame::new(
			self.device_mut().start_frame(index, synchronizer_handle),
			crate::queue::completed_frame_key(index, frames),
		)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest>,
		_wait_for: &[SynchronizerHandle],
		_synchronizer: SynchronizerHandle,
		execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
	) where
		P: AsRef<[PresentKey]>,
	{
		let mut device_pointer = self.device;
		let device = self.device_mut();
		for &wait_synchronizer in _wait_for {
			device.wait_for_synchronizer(wait_synchronizer);
		}
		let frame = frame.map(|frame| {
			let frames = device.frames;
			crate::queue::StartedFrame::new(
				device.start_frame(frame.index, frame.synchronizer),
				crate::queue::completed_frame_key(frame.index, frames),
			)
		});
		let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
		let frame = frame.map(|frame| frame.frame);
		let mut execution = Execution {
			frame,
			completed_frame,
			command_buffers: Vec::new(),
		};
		let present_keys = execute(&mut execution);
		let present_keys: Vec<PresentKey> = present_keys.as_ref().to_vec();
		let command_buffers = std::mem::take(&mut execution.command_buffers);
		drop(execution);
		for command_buffer in command_buffers {
			unsafe {
				device_pointer.as_mut().submit_command_buffer(command_buffer, _synchronizer);
			}
		}
		for present_key in &present_keys {
			unsafe {
				device_pointer.as_mut().present_swapchain(*present_key);
			}
		}
	}
}

impl crate::queue::Queue for QueueReference<'_> {
	type Frame<'a> = super::Frame<'a>;
	type Execution<'a> = Execution<'a>;

	fn create_command_buffer(&mut self, name: Option<&str>) -> CommandBufferHandle {
		self.device.create_command_buffer(name, self.queue_handle)
	}

	fn start_frame<'a>(
		&'a mut self,
		index: u32,
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
		frame: Option<crate::queue::FrameRequest>,
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
			command_buffers: Vec::new(),
		};
		let present_keys = execute(&mut execution);
		let present_keys: Vec<PresentKey> = present_keys.as_ref().to_vec();
		let command_buffers = std::mem::take(&mut execution.command_buffers);
		drop(execution);
		for command_buffer in command_buffers {
			unsafe {
				device_pointer.as_mut().submit_command_buffer(command_buffer, _synchronizer);
			}
		}
		for present_key in &present_keys {
			unsafe {
				device_pointer.as_mut().present_swapchain(*present_key);
			}
		}
	}
}
