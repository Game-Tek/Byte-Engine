use super::super::*;

impl Context {
	/// Waits for one private synchronizer and returns every completed command to its queue.
	pub(crate) fn wait_for_private_synchronizer(&mut self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		if let Some(error) = self.synchronizers.resource_mut(synchronizer_handle).wait(&mut self.queues) {
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
		let frame_key = graphics_hardware_interface::FrameKey::new(index, self.frames);
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);
		let synchronizer_handle = synchronizer_for_sequence(&self.synchronizers, synchronizer_handle, frame_key.sequence_index);
		self.wait_for_private_synchronizer(synchronizer_handle);
		// The sequence's previous frame has completed, so its timestamps are final.
		self.resolve_counters(frame_key.sequence_index);
		self.retire_internal_uploads(frame_key.sequence_index);
		// Every command that read this sequence's upload pages has completed, so the pages can be rewound.
		self.upload_arenas[frame_key.sequence_index as usize].reset();
		self.process_tasks(frame_key.sequence_index);
		crate::queue::StartedFrame::new(
			super::super::Frame::new(self, frame_key, queue_handle, allocator),
			completed_frame,
		)
	}
}

impl Context {
	/// Reads the timestamps the completed frame on `sequence_index` wrote and publishes its counter durations.
	///
	/// Call it after the sequence's synchronizer was waited. Resolving copies the entries out of the sequence's
	/// opaque heap, and invalidating them afterwards makes a slot the next frame allocates but never writes read as
	/// zero, which counts as unwritten. Only this sequence's frames write that heap, and the last of them completed,
	/// so the GPU does not use it while it is invalidated; invalidating a heap frames in flight still write to zeroes
	/// entries they write later.
	pub(crate) fn resolve_counters(&mut self, sequence_index: u8) {
		let slots = self.counters.written_slots(sequence_index);
		let frequency = self.timestamp_frequency;
		let elapsed = |start, end| {
			crate::counters::duration_from_frequency(crate::counters::elapsed_ticks(start, end, u64::BITS), frequency)
		};
		if slots.is_empty() {
			self.counters.resolve(sequence_index, |_| None, elapsed);
			return;
		}
		let heap = &*self.counter_heaps[sequence_index as usize];
		let range = NSRange::new(crate::counters::heap_entry(slots.start).1 as usize, slots.len());
		// SAFETY: The range lies inside the sequence's heap, and the frame that wrote it completed before this call.
		let data = unsafe { heap.resolveCounterRange(range) };
		// SAFETY: The resolved data is a fresh immutable object that nothing mutates while the slice is read.
		let bytes = data.as_deref().map_or(&[][..], |data| unsafe { data.as_bytes_unchecked() });
		self.counters.resolve(
			sequence_index,
			|slot| {
				let offset = (slot - slots.start) as usize * std::mem::size_of::<u64>();
				let entry = bytes.get(offset..offset + std::mem::size_of::<u64>())?;
				let ticks = u64::from_ne_bytes(entry.try_into().ok()?);
				(ticks != 0).then_some(ticks)
			},
			elapsed,
		);
		// SAFETY: Only this sequence's frames write this heap, and the last of them completed, so the GPU does not use it.
		unsafe { heap.invalidateCounterRange(range) };
	}
}
