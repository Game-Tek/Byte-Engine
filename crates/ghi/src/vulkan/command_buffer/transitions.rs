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

#[derive(Clone, Copy)]
pub(crate) struct ImageCopy {
	pub _src_texture: ImageHandle,
	pub _src_offset: vk::DeviceSize,
	pub dst_texture: ImageHandle,
	pub _dst_offset: vk::DeviceSize,
	pub _size: usize,
}

impl ImageCopy {
	pub fn new(
		src_texture: ImageHandle,
		src_offset: vk::DeviceSize,
		dst_texture: ImageHandle,
		dst_offset: vk::DeviceSize,
		size: usize,
	) -> Self {
		Self {
			_src_texture: src_texture,
			_src_offset: src_offset,
			dst_texture,
			_dst_offset: dst_offset,
			_size: size,
		}
	}
}

pub(super) fn buffer_row_length(format: crate::Formats, source_bytes_per_row: usize) -> u32 {
	match format {
		crate::Formats::BC5 | crate::Formats::BC7 | crate::Formats::BC7SRGB => ((source_bytes_per_row / 16) * 4) as u32,
		_ => (source_bytes_per_row / format.size()) as u32,
	}
}

pub(super) fn buffer_image_height(format: crate::Formats, source_row_count: usize) -> u32 {
	match format {
		crate::Formats::BC5 | crate::Formats::BC7 | crate::Formats::BC7SRGB => (source_row_count * 4) as u32,
		_ => source_row_count as u32,
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedImageBarrier {
	pub(super) old_layout: vk::ImageLayout,
	pub(super) src_stage: vk::PipelineStageFlags2,
	pub(super) src_access: vk::AccessFlags2,
	pub(super) new_layout: vk::ImageLayout,
	pub(super) dst_stage: vk::PipelineStageFlags2,
	pub(super) dst_access: vk::AccessFlags2,
	pub(super) image: vk::Image,
	pub(super) aspect_mask: vk::ImageAspectFlags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedBufferBarrier {
	pub(super) src_stage: vk::PipelineStageFlags2,
	pub(super) src_access: vk::AccessFlags2,
	pub(super) dst_stage: vk::PipelineStageFlags2,
	pub(super) dst_access: vk::AccessFlags2,
	pub(super) buffer: vk::Buffer,
	pub(super) offset: vk::DeviceSize,
	pub(super) size: vk::DeviceSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedMemoryBarrier {
	pub(super) src_stage: vk::PipelineStageFlags2,
	pub(super) src_access: vk::AccessFlags2,
	pub(super) dst_stage: vk::PipelineStageFlags2,
	pub(super) dst_access: vk::AccessFlags2,
}

/// The `TransitionStateUpdates` struct carries planner state changes without allocating a boxed callback.
pub(super) struct TransitionStateUpdates {
	pub(super) states: SmallVec<[(Handles, TransitionState); 64]>,
	pub(super) buffer_states: SmallVec<[(Handles, Vec<BufferTransitionState>); 16]>,
}

impl TransitionStateUpdates {
	pub(super) fn apply(self, recording: &mut CommandBufferRecording<'_>) {
		for (handle, state) in self.states {
			recording.states.insert(handle, state);
		}
		for (handle, states) in self.buffer_states {
			recording.buffer_states.insert(handle, states);
		}
	}
}

#[derive(Default)]
pub(super) struct PlannedTransitions {
	pub(super) image_barriers: Vec<PlannedImageBarrier>,
	pub(super) buffer_barriers: Vec<PlannedBufferBarrier>,
	pub(super) memory_barriers: Vec<PlannedMemoryBarrier>,
	pub(super) state_updates: SmallVec<[(Handles, TransitionState); 64]>,
	pub(super) buffer_state_updates: SmallVec<[(Handles, Vec<BufferTransitionState>); 16]>,
}

impl PlannedTransitions {
	pub(super) fn update_buffer_state(
		&mut self,
		handle: Handles,
		range: BufferRange,
		state: TransitionState,
		buffer_states: &HashMap<Handles, Vec<BufferTransitionState>>,
	) {
		let mut states = self
			.buffer_state_updates
			.iter()
			.find_map(|(updated_handle, states)| (*updated_handle == handle).then(|| states.clone()))
			.or_else(|| buffer_states.get(&handle).cloned())
			.unwrap_or_default();

		states.retain(|existing| !existing.range.overlaps(range));
		states.push(BufferTransitionState { range, state });

		if let Some((_, updated_states)) = self
			.buffer_state_updates
			.iter_mut()
			.find(|(updated_handle, _)| *updated_handle == handle)
		{
			*updated_states = states;
		} else {
			self.buffer_state_updates.push((handle, states));
		}
	}
}
