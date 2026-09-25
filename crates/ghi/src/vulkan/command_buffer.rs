use ash::vk::{self, Handle as _};
use smallvec::SmallVec;
use utils::{Extent, hash::HashMap};

use super::{
	AccelerationStructure, BottomLevelAccelerationStructureHandle, Buffer, BufferHandle, BufferRange, BufferTransitionState,
	CommandBufferInternal, Consumption, Context, Descriptor, DescriptorMaterializationHandle, Handles, Image, ImageHandle,
	Swapchain, TextureReadbackStorage, TopLevelAccelerationStructureHandle, TransitionState, VulkanConsumption,
	utils::{
		extent_into_vk_extent, image_aspect_mask, texture_format_and_resource_use_to_image_layout, to_access_flags,
		to_clear_value, to_load_operation, to_pipeline_stage_flags, to_store_operation,
	},
};
use crate::{FrameKey, HandleLike as _, Size, graphics_hardware_interface};

/// The `CommandBufferReference` struct creates recordings for one Vulkan command buffer through a borrowed context.
pub struct CommandBufferReference<'a> {
	pub(crate) device: &'a mut Context,
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
}

impl crate::command_buffer::CommandBuffer for CommandBufferReference<'_> {
	fn create_command_buffer_recording(
		&mut self,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		self.device.create_command_buffer_recording(self.command_buffer_handle)
	}
}

/// The `CommandBufferRecording` struct exists to encode Vulkan commands for one GHI command-buffer recording.
pub struct CommandBufferRecording<'a> {
	device: &'a mut Context,
	command_buffer: graphics_hardware_interface::CommandBufferHandle,
	frame_key: Option<FrameKey>,
	sequence_index: u8,
	pub(crate) states: HashMap<Handles, TransitionState>,
	pub(crate) buffer_states: HashMap<Handles, Vec<BufferTransitionState>>,
	pipeline_bind_point: vk::PipelineBindPoint,

	bound_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<graphics_hardware_interface::DescriptorSetHandle>,
	current_descriptor_materialization: Option<DescriptorMaterializationHandle>,
	descriptor_materialization_dirty: bool,
	descriptor_resources_initialized: bool,
	descriptor_heaps_bound: bool,
	pending_rendering: Option<(Extent, SmallVec<[graphics_hardware_interface::AttachmentInformation; 8]>)>,
	active_rendering: bool,
	/// Extent of the render pass being recorded; scissors are clamped to it.
	active_render_extent: Extent,
	texture_readbacks: SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
	readbacks_finalized: bool,
}

impl Drop for CommandBufferRecording<'_> {
	fn drop(&mut self) {
		if !self.readbacks_finalized {
			for handle in std::mem::take(&mut self.texture_readbacks) {
				self.device.cancel_texture_readback(handle);
			}
		}
	}
}

mod operations;
mod recording;
mod transitions;

pub(crate) use transitions::{BufferCopy, ImageCopy};
use transitions::{PlannedTransitions, TransitionStateUpdates, buffer_image_height, buffer_row_length};

#[cfg(test)]
mod tests {
	use super::*;

	type BufferStates = HashMap<Handles, Vec<BufferTransitionState>>;

	fn transition(stage: vk::PipelineStageFlags2, access: vk::AccessFlags2, layout: vk::ImageLayout) -> TransitionState {
		TransitionState::new(stage, access, layout)
	}

	fn buffer_transition(stage: vk::PipelineStageFlags2, access: vk::AccessFlags2) -> TransitionState {
		transition(stage, access, vk::ImageLayout::UNDEFINED)
	}

	/// Builds a state that is read after a transfer write it still has to order against.
	fn read_after_write(stage: vk::PipelineStageFlags2, access: vk::AccessFlags2, layout: vk::ImageLayout) -> TransitionState {
		let mut state = transition(stage, access, layout);
		state.last_write_stage = vk::PipelineStageFlags2::TRANSFER;
		state.last_write_access = vk::AccessFlags2::TRANSFER_WRITE;
		state
	}

	fn assert_visible_state_eq(actual: TransitionState, expected: TransitionState) {
		assert_eq!(
			(actual.stage, actual.access, actual.layout),
			(expected.stage, expected.access, expected.layout)
		);
	}

	fn consumption(handle: Handles, state: TransitionState) -> VulkanConsumption {
		VulkanConsumption {
			handle,
			stages: state.stage,
			access: state.access,
			layout: state.layout,
			range: None,
		}
	}

	fn ranged_consumption(handle: Handles, state: TransitionState, range: BufferRange) -> VulkanConsumption {
		VulkanConsumption {
			range: Some(range),
			..consumption(handle, state)
		}
	}

	fn buffer_states(handle: Handles, ranges: &[(BufferRange, TransitionState)]) -> BufferStates {
		let ranges = ranges.iter().map(|&(range, state)| BufferTransitionState { range, state });
		HashMap::from_iter([(handle, ranges.collect())])
	}

	/// Plans `consumptions` against the given whole-resource and ranged buffer states.
	/// Lookups without a provided result panic, which also checks that the planner does not resolve unrelated handles.
	fn plan<const N: usize>(
		states: &[(Handles, TransitionState)],
		buffer_states: &BufferStates,
		consumptions: [VulkanConsumption; N],
		image: Option<(vk::Image, vk::Format)>,
		buffer: Option<vk::Buffer>,
	) -> PlannedTransitions {
		CommandBufferRecording::plan_vulkan_resource_transitions(
			&states.iter().copied().collect(),
			buffer_states,
			consumptions,
			|_| Some(image.expect("Unexpected image lookup.")),
			|_| Some(buffer.expect("Unexpected buffer lookup.")),
		)
	}

	#[test]
	fn planner_barriers_equal_write_states() {
		let handle = Handles::Buffer(BufferHandle(1));
		let write = buffer_transition(vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE);

		let planned = plan(
			&[(handle, write)],
			&BufferStates::default(),
			[consumption(handle, write)],
			None,
			Some(vk::Buffer::from_raw(13)),
		);

		assert!(planned.image_barriers.is_empty() && planned.memory_barriers.is_empty());
		assert_eq!(planned.buffer_barriers.len(), 1);
		assert_eq!(planned.updates.states.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert_eq!((barrier.src_stage_mask, barrier.src_access_mask), (write.stage, write.access));
		assert_eq!((barrier.dst_stage_mask, barrier.dst_access_mask), (write.stage, write.access));
	}

	#[test]
	fn planner_merges_read_after_read_buffer_state_without_a_barrier() {
		let handle = Handles::Buffer(BufferHandle(11));
		let fragment_read = buffer_transition(vk::PipelineStageFlags2::FRAGMENT_SHADER, vk::AccessFlags2::SHADER_READ);
		let compute_read = buffer_transition(vk::PipelineStageFlags2::COMPUTE_SHADER, vk::AccessFlags2::SHADER_READ);

		let planned = plan(
			&[(handle, fragment_read)],
			&BufferStates::default(),
			[consumption(handle, compute_read)],
			None,
			Some(vk::Buffer::from_raw(12)),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert_eq!(planned.updates.states.len(), 1);
		assert!(
			planned.updates.states[0]
				.1
				.stage
				.contains(fragment_read.stage | compute_read.stage)
		);
	}

	#[test]
	fn planner_barriers_read_after_read_for_stages_the_last_write_barrier_missed() {
		let handle = Handles::Buffer(BufferHandle(20));
		let fragment_read = read_after_write(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::UNDEFINED,
		);

		let planned = plan(
			&[(handle, fragment_read)],
			&BufferStates::default(),
			[consumption(
				handle,
				buffer_transition(vk::PipelineStageFlags2::VERTEX_SHADER, vk::AccessFlags2::SHADER_READ),
			)],
			None,
			Some(vk::Buffer::from_raw(20)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert!(barrier.src_stage_mask.contains(vk::PipelineStageFlags2::TRANSFER));
		assert!(barrier.src_access_mask.contains(vk::AccessFlags2::TRANSFER_WRITE));
		assert_eq!(barrier.dst_stage_mask, vk::PipelineStageFlags2::VERTEX_SHADER);
		let state = planned.updates.states[0].1;
		assert!(
			state
				.stage
				.contains(vk::PipelineStageFlags2::FRAGMENT_SHADER | vk::PipelineStageFlags2::VERTEX_SHADER)
		);
	}

	#[test]
	fn planner_skips_read_after_read_already_visible_to_the_new_stage() {
		let handle = Handles::Image(ImageHandle(21));
		let sampled = read_after_write(
			vk::PipelineStageFlags2::FRAGMENT_SHADER | vk::PipelineStageFlags2::COMPUTE_SHADER,
			vk::AccessFlags2::SHADER_SAMPLED_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);
		let compute_sample = transition(
			vk::PipelineStageFlags2::COMPUTE_SHADER,
			vk::AccessFlags2::SHADER_SAMPLED_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);

		let planned = plan(
			&[(handle, sampled)],
			&BufferStates::default(),
			[consumption(handle, compute_sample)],
			Some((vk::Image::from_raw(21), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert!(planned.image_barriers.is_empty());
		assert_eq!(planned.updates.states.len(), 1);
	}

	#[test]
	fn planner_barriers_first_reader_after_a_consumed_transfer_write() {
		let handle = Handles::Image(ImageHandle(22));
		// consume_last_resources leaves pending uploads as a TRANSFER/NONE state that only remembers the write.
		let consumed_upload = read_after_write(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::NONE,
			vk::ImageLayout::GENERAL,
		);
		let fragment_read = transition(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::GENERAL,
		);

		let planned = plan(
			&[(handle, consumed_upload)],
			&BufferStates::default(),
			[consumption(handle, fragment_read)],
			Some((vk::Image::from_raw(22), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		let barrier = planned.image_barriers[0];
		assert_eq!(
			(barrier.old_layout, barrier.new_layout),
			(vk::ImageLayout::GENERAL, vk::ImageLayout::GENERAL)
		);
		assert!(barrier.src_access_mask.contains(vk::AccessFlags2::TRANSFER_WRITE));
		assert_eq!(barrier.dst_stage_mask, vk::PipelineStageFlags2::FRAGMENT_SHADER);
	}

	#[test]
	fn planner_barriers_image_read_after_read_for_uncovered_stages_without_write_history() {
		let handle = Handles::Image(ImageHandle(23));
		// The layout transition into this state is itself a write the new stage must be ordered after.
		let fragment_sample = transition(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_SAMPLED_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);
		let vertex_sample = transition(
			vk::PipelineStageFlags2::VERTEX_SHADER,
			vk::AccessFlags2::SHADER_SAMPLED_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);

		let planned = plan(
			&[(handle, fragment_sample)],
			&BufferStates::default(),
			[consumption(handle, vertex_sample)],
			Some((vk::Image::from_raw(23), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		assert!(planned.image_barriers[0].src_stage_mask.contains(fragment_sample.stage));
	}

	#[test]
	fn planner_keeps_pending_writes_for_untouched_parts_of_a_split_buffer_range() {
		let handle = Handles::Buffer(BufferHandle(24));
		let buffer = Some(vk::Buffer::from_raw(24));
		let copy_write = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);
		let compute_read = buffer_transition(vk::PipelineStageFlags2::COMPUTE_SHADER, vk::AccessFlags2::SHADER_READ);
		let mut tracked = buffer_states(handle, &[(BufferRange::new(0, 128), copy_write)]);

		let first = plan(
			&[],
			&tracked,
			[ranged_consumption(handle, compute_read, BufferRange::new(64, 64))],
			None,
			buffer,
		);
		assert_eq!(first.buffer_barriers.len(), 1);
		assert_eq!((first.buffer_barriers[0].offset, first.buffer_barriers[0].size), (64, 64));
		tracked.extend(first.updates.buffer_states);

		let second = plan(
			&[],
			&tracked,
			[ranged_consumption(handle, compute_read, BufferRange::new(0, 64))],
			None,
			buffer,
		);

		assert_eq!(second.buffer_barriers.len(), 1);
		let barrier = second.buffer_barriers[0];
		assert_eq!((barrier.offset, barrier.size), (0, 64));
		assert!(barrier.src_access_mask.contains(vk::AccessFlags2::TRANSFER_WRITE));
	}

	#[test]
	fn planner_clips_buffer_barriers_to_each_tracked_range() {
		let handle = Handles::Buffer(BufferHandle(25));
		let write = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);
		let tracked = buffer_states(
			handle,
			&[
				(BufferRange::new(0, 64), write),
				(BufferRange::new(64, vk::WHOLE_SIZE), write),
			],
		);
		let vertex_read = buffer_transition(vk::PipelineStageFlags2::VERTEX_INPUT, vk::AccessFlags2::VERTEX_ATTRIBUTE_READ);

		let planned = plan(
			&[],
			&tracked,
			[consumption(handle, vertex_read)],
			None,
			Some(vk::Buffer::from_raw(25)),
		);

		let ranges = planned
			.buffer_barriers
			.iter()
			.map(|barrier| (barrier.offset, barrier.size))
			.collect::<Vec<_>>();
		assert_eq!(ranges, vec![(0, 64), (64, vk::WHOLE_SIZE)]);
		let (_, states) = &planned.updates.buffer_states[0];
		assert_eq!(states.len(), 1, "identical adjacent read states should coalesce");
		assert!(states[0].range == BufferRange::new(0, vk::WHOLE_SIZE));
	}

	#[test]
	fn acquired_swapchain_image_barrier_is_sourced_from_its_first_use_stage() {
		let handle = Handles::Image(ImageHandle(26));
		let acquired = vk::Image::from_raw(26);
		let acquired_state = transition(
			vk::PipelineStageFlags2::NONE,
			vk::AccessFlags2::NONE,
			vk::ImageLayout::UNDEFINED,
		);
		let compute_write = transition(
			vk::PipelineStageFlags2::COMPUTE_SHADER,
			vk::AccessFlags2::SHADER_STORAGE_WRITE,
			vk::ImageLayout::GENERAL,
		);

		let mut planned = plan(
			&[(handle, acquired_state)],
			&BufferStates::default(),
			[consumption(handle, compute_write)],
			Some((acquired, vk::Format::B8G8R8A8_UNORM)),
			None,
		);

		let first_use_stage = CommandBufferRecording::chain_barriers_to_acquire(&mut planned.image_barriers, acquired);

		assert_eq!(first_use_stage, vk::PipelineStageFlags2::COMPUTE_SHADER);
		let barrier = planned.image_barriers[0];
		assert_eq!(barrier.src_stage_mask, vk::PipelineStageFlags2::COMPUTE_SHADER);
		assert_eq!(
			(barrier.old_layout, barrier.new_layout),
			(vk::ImageLayout::UNDEFINED, vk::ImageLayout::GENERAL)
		);
	}

	#[test]
	fn barriers_on_other_images_are_not_chained_to_acquire() {
		let mut barriers = [vk::ImageMemoryBarrier2::default()
			.old_layout(vk::ImageLayout::UNDEFINED)
			.new_layout(vk::ImageLayout::GENERAL)
			.dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
			.dst_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
			.image(vk::Image::from_raw(27))];

		let first_use_stage = CommandBufferRecording::chain_barriers_to_acquire(&mut barriers, vk::Image::from_raw(28));

		assert!(first_use_stage.is_empty());
		assert!(barriers[0].src_stage_mask.is_empty());
	}

	#[test]
	fn planner_skips_non_overlapping_buffer_ranges() {
		let handle = Handles::Buffer(BufferHandle(12));
		let copy_write = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);

		let planned = plan(
			&[],
			&buffer_states(handle, &[(BufferRange::new(0, 64), copy_write)]),
			[ranged_consumption(handle, copy_write, BufferRange::new(128, 64))],
			None,
			Some(vk::Buffer::from_raw(14)),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert_eq!(planned.updates.buffer_states.len(), 1);
	}

	#[test]
	fn planner_barriers_overlapping_buffer_ranges() {
		let handle = Handles::Buffer(BufferHandle(13));
		let copy_write = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);

		let planned = plan(
			&[],
			&buffer_states(handle, &[(BufferRange::new(0, 128), copy_write)]),
			[ranged_consumption(handle, copy_write, BufferRange::new(64, 64))],
			None,
			Some(vk::Buffer::from_raw(15)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(copy_write.stage, copy_write.access)
		);
		assert_eq!((barrier.offset, barrier.size), (64, 64));
	}

	#[test]
	fn planner_includes_last_buffer_write_when_read_state_transitions_to_write() {
		let handle = Handles::Buffer(BufferHandle(14));
		let copy_write = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);
		let mut read_state = buffer_transition(vk::PipelineStageFlags2::COMPUTE_SHADER, vk::AccessFlags2::SHADER_READ);
		read_state.last_write_stage = copy_write.stage;
		read_state.last_write_access = copy_write.access;
		let range = BufferRange::new(64, 64);

		let planned = plan(
			&[],
			&buffer_states(handle, &[(range, read_state)]),
			[ranged_consumption(handle, copy_write, range)],
			None,
			Some(vk::Buffer::from_raw(16)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert!(barrier.src_stage_mask.contains(read_state.stage | copy_write.stage));
		assert!(barrier.src_access_mask.contains(read_state.access | copy_write.access));
		assert_eq!(
			(barrier.dst_stage_mask, barrier.dst_access_mask),
			(copy_write.stage, copy_write.access)
		);
	}

	#[test]
	fn planner_uses_previous_image_state_when_present() {
		let handle = Handles::Image(ImageHandle(2));
		let previous = transition(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_WRITE,
			vk::ImageLayout::TRANSFER_DST_OPTIMAL,
		);
		let destination = transition(
			vk::PipelineStageFlags2::COMPUTE_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::GENERAL,
		);

		let planned = plan(
			&[(handle, previous)],
			&BufferStates::default(),
			[consumption(handle, destination)],
			Some((vk::Image::from_raw(77), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		let barrier = planned.image_barriers[0];
		assert_eq!(
			(barrier.old_layout, barrier.src_stage_mask, barrier.src_access_mask),
			(previous.layout, previous.stage, previous.access)
		);
		assert_eq!(
			(barrier.new_layout, barrier.dst_stage_mask, barrier.dst_access_mask),
			(destination.layout, destination.stage, destination.access)
		);
		assert_eq!(barrier.image, vk::Image::from_raw(77));
		assert_eq!(barrier.subresource_range.aspect_mask, vk::ImageAspectFlags::COLOR);
		assert_eq!(planned.updates.states.len(), 1);
		let (updated_handle, updated_state) = planned.updates.states[0];
		assert!(updated_handle == handle);
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_uses_default_source_when_state_is_missing() {
		let destination = transition(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);

		let planned = plan(
			&[],
			&BufferStates::default(),
			[consumption(Handles::Image(ImageHandle(3)), destination)],
			Some((vk::Image::from_raw(88), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		let barrier = planned.image_barriers[0];
		assert_eq!(barrier.old_layout, vk::ImageLayout::UNDEFINED);
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(vk::PipelineStageFlags2::empty(), vk::AccessFlags2::empty())
		);
	}

	#[test]
	fn planner_selects_depth_aspect_for_depth_images() {
		let depth_write = transition(
			vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS,
			vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
			vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
		);

		for format in [vk::Format::D32_SFLOAT, vk::Format::D16_UNORM] {
			let planned = plan(
				&[],
				&BufferStates::default(),
				[consumption(Handles::Image(ImageHandle(4)), depth_write)],
				Some((vk::Image::from_raw(99), format)),
				None,
			);

			assert_eq!(planned.image_barriers.len(), 1);
			assert_eq!(
				planned.image_barriers[0].subresource_range.aspect_mask,
				vk::ImageAspectFlags::DEPTH
			);
		}
	}

	#[test]
	fn planner_merges_repeated_image_consumptions_into_one_barrier() {
		// Uploading several mips of one image consumes it once per mip in the same batch.
		let handle = Handles::Image(ImageHandle(31));
		let sampled = transition(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);
		let upload = consumption(
			handle,
			transition(
				vk::PipelineStageFlags2::TRANSFER,
				vk::AccessFlags2::TRANSFER_WRITE,
				vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			),
		);

		let planned = plan(
			&[(handle, sampled)],
			&BufferStates::default(),
			[upload.clone(), upload.clone(), upload],
			Some((vk::Image::from_raw(31), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		assert_eq!(planned.image_barriers[0].old_layout, sampled.layout);
		assert_eq!(planned.updates.states.len(), 1);
	}

	#[test]
	fn compressed_copy_rows_count_texels() {
		// 64 texels wide is 16 blocks of 16 bytes; 8 block rows are 32 texel rows.
		for format in [
			crate::Formats::BC5,
			crate::Formats::BC5SNORM,
			crate::Formats::BC7,
			crate::Formats::BC7SRGB,
		] {
			assert_eq!(buffer_row_length(format, 256), 64);
			assert_eq!(buffer_image_height(format, 8), 32);
		}
	}

	#[test]
	fn planner_skips_null_image_and_does_not_update_state() {
		let image_write = transition(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_WRITE,
			vk::ImageLayout::TRANSFER_DST_OPTIMAL,
		);

		let planned = plan(
			&[],
			&BufferStates::default(),
			[consumption(Handles::Image(ImageHandle(5)), image_write)],
			Some((vk::Image::null(), vk::Format::R8G8B8A8_UNORM)),
			None,
		);

		assert!(planned.image_barriers.is_empty());
		assert!(planned.updates.states.is_empty());
	}

	#[test]
	fn planner_builds_buffer_barrier_from_previous_state() {
		let handle = Handles::Buffer(BufferHandle(6));
		let previous = buffer_transition(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE);
		let destination = buffer_transition(vk::PipelineStageFlags2::VERTEX_INPUT, vk::AccessFlags2::VERTEX_ATTRIBUTE_READ);

		let planned = plan(
			&[(handle, previous)],
			&BufferStates::default(),
			[consumption(handle, destination)],
			None,
			Some(vk::Buffer::from_raw(111)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(previous.stage, previous.access)
		);
		assert_eq!(
			(barrier.dst_stage_mask, barrier.dst_access_mask),
			(destination.stage, destination.access)
		);
		assert_eq!(barrier.buffer, vk::Buffer::from_raw(111));
		assert_eq!(planned.updates.states.len(), 1);
		assert_visible_state_eq(planned.updates.states[0].1, destination);
	}

	#[test]
	fn planner_skips_null_buffer_and_does_not_update_state() {
		let buffer_write = buffer_transition(vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE);

		let planned = plan(
			&[],
			&BufferStates::default(),
			[consumption(Handles::Buffer(BufferHandle(7)), buffer_write)],
			None,
			Some(vk::Buffer::null()),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert!(planned.updates.states.is_empty());
	}

	#[test]
	fn planner_handles_vk_buffer_without_buffer_lookup() {
		let handle = Handles::VkBuffer(vk::Buffer::from_raw(222));
		let destination = buffer_transition(vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_READ);

		let planned = plan(&[], &BufferStates::default(), [consumption(handle, destination)], None, None);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(vk::PipelineStageFlags2::empty(), vk::AccessFlags2::empty())
		);
		assert_eq!(barrier.buffer, vk::Buffer::from_raw(222));
		assert_eq!(planned.updates.states.len(), 1);
		let (updated_handle, updated_state) = planned.updates.states[0];
		assert!(updated_handle == handle);
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_builds_memory_barrier_for_acceleration_structures() {
		let handle = Handles::TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle(8));
		let previous = buffer_transition(
			vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
			vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
		);
		let destination = buffer_transition(
			vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR,
			vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR,
		);

		let planned = plan(
			&[(handle, previous)],
			&BufferStates::default(),
			[consumption(handle, destination)],
			None,
			None,
		);

		assert_eq!(planned.memory_barriers.len(), 1);
		let barrier = planned.memory_barriers[0];
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(previous.stage, previous.access)
		);
		assert_eq!(
			(barrier.dst_stage_mask, barrier.dst_access_mask),
			(destination.stage, destination.access)
		);
		assert_eq!(planned.updates.states.len(), 1);
		assert_visible_state_eq(planned.updates.states[0].1, destination);
	}

	#[test]
	fn planner_merges_duplicate_consumptions_of_one_buffer() {
		// A mesh buffer read as vertices and indices in one batch needs one barrier, and later writers must wait for both reads.
		let handle = Handles::Buffer(BufferHandle(10));
		let source = buffer_transition(vk::PipelineStageFlags2::TRANSFER, vk::AccessFlags2::TRANSFER_WRITE);
		let vertex_read = buffer_transition(vk::PipelineStageFlags2::VERTEX_INPUT, vk::AccessFlags2::VERTEX_ATTRIBUTE_READ);
		let index_read = buffer_transition(vk::PipelineStageFlags2::INDEX_INPUT, vk::AccessFlags2::INDEX_READ);

		let planned = plan(
			&[(handle, source)],
			&BufferStates::default(),
			[consumption(handle, vertex_read), consumption(handle, index_read)],
			None,
			Some(vk::Buffer::from_raw(333)),
		);

		let merged = buffer_transition(vertex_read.stage | index_read.stage, vertex_read.access | index_read.access);
		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];
		assert_eq!(
			(barrier.src_stage_mask, barrier.src_access_mask),
			(source.stage, source.access)
		);
		assert_eq!(
			(barrier.dst_stage_mask, barrier.dst_access_mask),
			(merged.stage, merged.access)
		);
		assert_eq!(planned.updates.states.len(), 1);
		assert_visible_state_eq(planned.updates.states[0].1, merged);
	}
}
