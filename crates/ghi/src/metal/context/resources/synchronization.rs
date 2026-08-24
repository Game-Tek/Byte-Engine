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
			self.wait_for_private_synchronizer(synchronizer_handle);
			self.synchronizers.resource_mut(synchronizer_handle).reset();
		}
	}

	pub fn wait_for_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_index as u8);
			self.wait_for_private_synchronizer(synchronizer_handle);
		}
	}

	/// Waits for one private synchronizer and returns every completed command to its queue.
	pub(crate) fn wait_for_private_synchronizer(&mut self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		let (completed, error) = self.synchronizers.resource_mut(synchronizer_handle).wait();
		for (queue_handle, commands) in completed {
			self.queues[queue_handle.0 as usize].recycle(commands);
		}
		if let Some(error) = error {
			panic!("{error}");
		}
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		queue_handle: graphics_hardware_interface::QueueHandle,
		allocator: &'a dyn std::alloc::Allocator,
	) -> crate::queue::StartedFrame<super::super::Frame<'a>> {
		let frame_key = graphics_hardware_interface::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);
		let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_key.sequence_index);
		self.wait_for_private_synchronizer(synchronizer_handle);
		self.retire_internal_uploads(frame_key.sequence_index);
		self.process_tasks(frame_key.sequence_index);
		crate::queue::StartedFrame::new(
			super::super::Frame::new_for_queue(self, frame_key, queue_handle, allocator),
			completed_frame,
		)
	}

	pub fn start_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn end_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn wait(&mut self) {
		let mut completed =
			SmallVec::<[(graphics_hardware_interface::QueueHandle, SmallVec<[queue::NativeCommand; 4]>); 8]>::new();
		let mut first_error = None;
		for synchronizer in self.synchronizers.iter_mut() {
			let (workloads, error) = synchronizer.wait();
			completed.extend(workloads);
			if let Some(error) = error {
				first_error.get_or_insert(error);
			}
		}
		for (queue_handle, commands) in completed {
			self.queues[queue_handle.0 as usize].recycle(commands);
		}
		if let Some(error) = first_error {
			panic!("{error}");
		}
	}
}
