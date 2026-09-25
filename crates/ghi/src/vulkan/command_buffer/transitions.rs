use super::*;

#[derive(Clone, Copy)]
pub(crate) struct BufferCopy {
	pub src_buffer: BufferHandle,
	pub src_offset: vk::DeviceSize,
	pub dst_buffer: BufferHandle,
	pub dst_offset: vk::DeviceSize,
	pub size: usize,
}

impl BufferCopy {
	pub fn new(
		src_buffer: BufferHandle,
		src_offset: vk::DeviceSize,
		dst_buffer: BufferHandle,
		dst_offset: vk::DeviceSize,
		size: usize,
	) -> Self {
		Self {
			src_buffer,
			src_offset,
			dst_buffer,
			dst_offset,
			size,
		}
	}
}

/// Width of BC compression blocks in texels.
const BC_BLOCK_EXTENT: usize = 4;

/// Converts a source row pitch into Vulkan's `bufferRowLength`, which counts texels even for block-compressed formats.
pub(super) fn buffer_row_length(format: crate::Formats, source_bytes_per_row: usize) -> u32 {
	match format.bc_bytes_per_block() {
		Some(bytes_per_block) => (source_bytes_per_row / bytes_per_block as usize * BC_BLOCK_EXTENT) as u32,
		None => (source_bytes_per_row / format.size()) as u32,
	}
}

/// Converts a source row count into Vulkan's `bufferImageHeight`, which counts texel rows rather than block rows.
pub(super) fn buffer_image_height(format: crate::Formats, source_row_count: usize) -> u32 {
	match format.bc_bytes_per_block() {
		Some(_) => (source_row_count * BC_BLOCK_EXTENT) as u32,
		None => source_row_count as u32,
	}
}

/// The `TransitionStateUpdates` struct carries planner state changes without allocating a boxed callback.
#[derive(Default)]
pub(super) struct TransitionStateUpdates {
	pub(super) states: SmallVec<[(Handles, TransitionState); 64]>,
	pub(super) buffer_states: SmallVec<[(Handles, Vec<BufferTransitionState>); 16]>,
	/// Swapchain indices and the stages at which their acquired image is first used.
	pub(super) acquire_waits: SmallVec<[(usize, vk::PipelineStageFlags2); 2]>,
}

impl TransitionStateUpdates {
	pub(super) fn apply(self, recording: &mut CommandBufferRecording<'_>) {
		recording.states.extend(self.states);
		recording.buffer_states.extend(self.buffer_states);
		let sequence_index = recording.sequence_index as usize;
		for (swapchain_index, stage) in self.acquire_waits {
			recording.device.swapchains[swapchain_index].acquire_wait_stages[sequence_index] |= stage;
		}
	}
}

/// The `PlannedTransitions` struct holds the barriers a batch of consumptions needs and the states it leaves behind.
#[derive(Default)]
pub(super) struct PlannedTransitions {
	pub(super) image_barriers: Vec<vk::ImageMemoryBarrier2<'static>>,
	pub(super) buffer_barriers: Vec<vk::BufferMemoryBarrier2<'static>>,
	pub(super) memory_barriers: Vec<vk::MemoryBarrier2<'static>>,
	pub(super) updates: TransitionStateUpdates,
}

impl PlannedTransitions {
	pub(super) fn update_buffer_state(
		&mut self,
		handle: Handles,
		range: BufferRange,
		state: TransitionState,
		buffer_states: &HashMap<Handles, Vec<BufferTransitionState>>,
	) {
		let updates = &mut self.updates.buffer_states;
		let index = updates.iter().position(|(updated_handle, _)| *updated_handle == handle);
		let existing_states = match index {
			Some(index) => updates[index].1.as_slice(),
			None => buffer_states.get(&handle).map_or(&[][..], Vec::as_slice),
		};
		let span = |start, end, state| BufferTransitionState {
			range: BufferRange::from_bounds(start, end),
			state,
		};

		// Tracked ranges are disjoint; split the ones the new range touches so untouched bytes keep their pending state.
		let mut states = Vec::with_capacity(existing_states.len() + 2);
		let mut touched = SmallVec::<[BufferRange; 8]>::new();
		for existing in existing_states {
			if !existing.range.overlaps(range) {
				states.push(*existing);
				continue;
			}

			if existing.range.offset < range.offset {
				states.push(span(existing.range.offset, range.offset, existing.state));
			}
			if existing.range.end() > range.end() {
				states.push(span(range.end(), existing.range.end(), existing.state));
			}

			let overlap = existing.range.intersection(range);
			let overlap_state = if existing.state.reads_only(state) {
				existing.state.merge_reads(state)
			} else {
				state.inherit_last_write_from(existing.state)
			};
			states.push(BufferTransitionState {
				range: overlap,
				state: overlap_state,
			});
			touched.push(overlap);
		}

		touched.sort_unstable_by_key(|range| range.offset);
		let mut cursor = range.offset;
		for touched_range in touched {
			if touched_range.offset > cursor {
				states.push(span(cursor, touched_range.offset, state));
			}
			cursor = cursor.max(touched_range.end());
		}
		if cursor < range.end() {
			states.push(span(cursor, range.end(), state));
		}

		states.sort_unstable_by_key(|state| state.range.offset);
		states.dedup_by(|next, previous| {
			let adjacent = previous.range.end() == next.range.offset && previous.state == next.state;
			if adjacent {
				previous.range = BufferRange::from_bounds(previous.range.offset, next.range.end());
			}
			adjacent
		});

		match index {
			Some(index) => updates[index].1 = states,
			None => updates.push((handle, states)),
		}
	}
}
