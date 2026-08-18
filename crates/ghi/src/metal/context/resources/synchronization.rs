use super::super::*;

impl Context {
	pub fn create_synchronizer(
		&mut self,
		_name: Option<&str>,
		signaled: bool,
	) -> graphics_hardware_interface::SynchronizerHandle {
		let (master, mut previous) = self.synchronizers.add(synchronizer::Synchronizer::new(signaled));

		for _ in 1..self.frames {
			let handle = self
				.synchronizers
				.add_with_master(synchronizer::Synchronizer::new(signaled), master);
			self.synchronizers.set_next(previous, Some(handle));
			previous = handle;
		}

		master
	}

	pub fn reset_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_index as u8);
			self.synchronizers.resource(synchronizer_handle).reset();
		}
	}

	pub fn wait_for_synchronizer(&self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_index as u8);
			self.synchronizers.resource(synchronizer_handle).wait();
		}
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> crate::queue::StartedFrame<super::super::Frame<'a>> {
		let frame_key = graphics_hardware_interface::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);
		let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_key.sequence_index);
		self.synchronizers.resource(synchronizer_handle).wait();
		self.retire_internal_uploads(frame_key.sequence_index);
		self.process_tasks(frame_key.sequence_index);
		crate::queue::StartedFrame::new(
			super::super::Frame::new_for_queue(self, frame_key, queue_handle),
			completed_frame,
		)
	}

	pub fn start_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn end_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn wait(&self) {
		for synchronizer in self.synchronizers.iter() {
			synchronizer.wait();
		}
	}
}
