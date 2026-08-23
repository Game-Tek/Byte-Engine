use ash::vk::{self, Handle as _};
use smallvec::SmallVec;
use utils::{hash::HashMap, Extent};

use super::{
	utils::{
		extent_into_vk_extent, texture_format_and_resource_use_to_image_layout, to_access_flags, to_clear_value,
		to_load_operation, to_pipeline_stage_flags, to_store_operation,
	},
	AccelerationStructure, BottomLevelAccelerationStructureHandle, Buffer, BufferHandle, BufferRange, BufferTransitionState,
	CommandBufferInternal, Consumption, Context, Descriptor, DescriptorMaterializationHandle, Handles, Image, ImageHandle,
	Swapchain, Synchronizer, TextureReadbackStorage, TopLevelAccelerationStructureHandle, TransitionState, VulkanConsumption,
};
use crate::{graphics_hardware_interface, FrameKey, HandleLike as _, Size};

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

pub struct VulkanCommandBuffer<'a> {
	pub(crate) device: &'a mut Context,
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
}

impl crate::command_buffer::CommandBuffer for VulkanCommandBuffer<'_> {
	fn create_command_buffer_recording(
		&mut self,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		Context::create_command_buffer_recording(self.device, self.command_buffer_handle)
	}
}

mod operations;
mod recording;
mod transitions;

use transitions::{
	buffer_image_height, buffer_row_length, PlannedBufferBarrier, PlannedImageBarrier, PlannedMemoryBarrier,
	PlannedTransitions, TransitionStateUpdates,
};
pub(crate) use transitions::{BufferCopy, ImageCopy};

mod tests {
	use super::*;

	fn transition(stage: vk::PipelineStageFlags2, access: vk::AccessFlags2, layout: vk::ImageLayout) -> TransitionState {
		TransitionState::new(stage, access, layout)
	}

	fn assert_visible_state_eq(actual: TransitionState, expected: TransitionState) {
		assert!(actual.stage == expected.stage);
		assert!(actual.access == expected.access);
		assert!(actual.layout == expected.layout);
	}

	fn consumption(
		handle: Handles,
		stage: vk::PipelineStageFlags2,
		access: vk::AccessFlags2,
		layout: vk::ImageLayout,
	) -> VulkanConsumption {
		VulkanConsumption {
			handle,
			stages: stage,
			access,
			layout,
			range: None,
		}
	}

	fn ranged_consumption(
		handle: Handles,
		stage: vk::PipelineStageFlags2,
		access: vk::AccessFlags2,
		range: BufferRange,
	) -> VulkanConsumption {
		VulkanConsumption {
			handle,
			stages: stage,
			access,
			layout: vk::ImageLayout::UNDEFINED,
			range: Some(range),
		}
	}

	#[test]
	fn planner_barriers_equal_write_states() {
		let handle = Handles::Buffer(BufferHandle(1));
		let current = transition(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_WRITE,
			vk::ImageLayout::UNDEFINED,
		);
		let mut states = HashMap::default();
		states.insert(handle, current);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[consumption(
				handle,
				vk::PipelineStageFlags2::TRANSFER,
				vk::AccessFlags2::TRANSFER_WRITE,
				vk::ImageLayout::UNDEFINED,
			)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(13)),
		);

		assert!(planned.image_barriers.is_empty());
		assert_eq!(planned.buffer_barriers.len(), 1);
		assert!(planned.memory_barriers.is_empty());
		assert_eq!(planned.state_updates.len(), 1);

		let barrier = planned.buffer_barriers[0];

		assert!(barrier.src_stage == vk::PipelineStageFlags2::TRANSFER);
		assert!(barrier.src_access == vk::AccessFlags2::TRANSFER_WRITE);
		assert!(barrier.dst_stage == vk::PipelineStageFlags2::TRANSFER);
		assert!(barrier.dst_access == vk::AccessFlags2::TRANSFER_WRITE);
	}

	#[test]
	fn planner_merges_read_after_read_buffer_state_without_a_barrier() {
		let handle = Handles::Buffer(BufferHandle(11));
		let mut states = HashMap::default();
		states.insert(
			handle,
			transition(
				vk::PipelineStageFlags2::FRAGMENT_SHADER,
				vk::AccessFlags2::SHADER_READ,
				vk::ImageLayout::UNDEFINED,
			),
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[consumption(
				handle,
				vk::PipelineStageFlags2::COMPUTE_SHADER,
				vk::AccessFlags2::SHADER_READ,
				vk::ImageLayout::UNDEFINED,
			)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(12)),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert_eq!(planned.state_updates.len(), 1);
		let state = planned.state_updates[0].1;

		assert!(state.stage.contains(vk::PipelineStageFlags2::FRAGMENT_SHADER));
		assert!(state.stage.contains(vk::PipelineStageFlags2::COMPUTE_SHADER));
	}

	#[test]
	fn planner_skips_non_overlapping_buffer_ranges() {
		let handle = Handles::Buffer(BufferHandle(12));
		let mut buffer_states = HashMap::default();
		buffer_states.insert(
			handle,
			vec![BufferTransitionState {
				range: BufferRange::new(0, 64),
				state: transition(
					vk::PipelineStageFlags2::COPY,
					vk::AccessFlags2::TRANSFER_WRITE,
					vk::ImageLayout::UNDEFINED,
				),
			}],
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&buffer_states,
			[ranged_consumption(
				handle,
				vk::PipelineStageFlags2::COPY,
				vk::AccessFlags2::TRANSFER_WRITE,
				BufferRange::new(128, 64),
			)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(14)),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert_eq!(planned.buffer_state_updates.len(), 1);
	}

	#[test]
	fn planner_barriers_overlapping_buffer_ranges() {
		let handle = Handles::Buffer(BufferHandle(13));
		let mut buffer_states = HashMap::default();
		buffer_states.insert(
			handle,
			vec![BufferTransitionState {
				range: BufferRange::new(0, 128),
				state: transition(
					vk::PipelineStageFlags2::COPY,
					vk::AccessFlags2::TRANSFER_WRITE,
					vk::ImageLayout::UNDEFINED,
				),
			}],
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&buffer_states,
			[ranged_consumption(
				handle,
				vk::PipelineStageFlags2::COPY,
				vk::AccessFlags2::TRANSFER_WRITE,
				BufferRange::new(64, 64),
			)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(15)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];

		assert!(barrier.src_stage == vk::PipelineStageFlags2::COPY);
		assert!(barrier.src_access == vk::AccessFlags2::TRANSFER_WRITE);
		assert!(barrier.offset == 64);
		assert!(barrier.size == 64);
	}

	#[test]
	fn planner_includes_last_buffer_write_when_read_state_transitions_to_write() {
		let handle = Handles::Buffer(BufferHandle(14));
		let mut read_state = transition(
			vk::PipelineStageFlags2::COMPUTE_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::UNDEFINED,
		);
		read_state.last_write_stage = vk::PipelineStageFlags2::COPY;
		read_state.last_write_access = vk::AccessFlags2::TRANSFER_WRITE;

		let mut buffer_states = HashMap::default();
		buffer_states.insert(
			handle,
			vec![BufferTransitionState {
				range: BufferRange::new(64, 64),
				state: read_state,
			}],
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&buffer_states,
			[ranged_consumption(
				handle,
				vk::PipelineStageFlags2::COPY,
				vk::AccessFlags2::TRANSFER_WRITE,
				BufferRange::new(64, 64),
			)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(16)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];

		assert!(barrier.src_stage.contains(vk::PipelineStageFlags2::COMPUTE_SHADER));
		assert!(barrier.src_stage.contains(vk::PipelineStageFlags2::COPY));
		assert!(barrier.src_access.contains(vk::AccessFlags2::SHADER_READ));
		assert!(barrier.src_access.contains(vk::AccessFlags2::TRANSFER_WRITE));
		assert!(barrier.dst_stage == vk::PipelineStageFlags2::COPY);
		assert!(barrier.dst_access == vk::AccessFlags2::TRANSFER_WRITE);
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
		let mut states = HashMap::default();
		states.insert(handle, previous);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| Some((vk::Image::from_raw(77), vk::Format::R8G8B8A8_UNORM)),
			|_| None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		let barrier = planned.image_barriers[0];

		assert!(barrier.old_layout == previous.layout);
		assert!(barrier.src_stage == previous.stage);
		assert!(barrier.src_access == previous.access);
		assert!(barrier.new_layout == destination.layout);
		assert!(barrier.dst_stage == destination.stage);
		assert!(barrier.dst_access == destination.access);
		assert!(barrier.image == vk::Image::from_raw(77));
		assert!(barrier.aspect_mask == vk::ImageAspectFlags::COLOR);
		assert_eq!(planned.state_updates.len(), 1);
		let (updated_handle, updated_state) = planned.state_updates[0];

		assert!(updated_handle == handle);
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_uses_default_source_when_state_is_missing() {
		let handle = Handles::Image(ImageHandle(3));
		let destination = transition(
			vk::PipelineStageFlags2::FRAGMENT_SHADER,
			vk::AccessFlags2::SHADER_READ,
			vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| Some((vk::Image::from_raw(88), vk::Format::R8G8B8A8_UNORM)),
			|_| None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		let barrier = planned.image_barriers[0];

		assert!(barrier.old_layout == vk::ImageLayout::UNDEFINED);
		assert!(barrier.src_stage == vk::PipelineStageFlags2::empty());
		assert!(barrier.src_access == vk::AccessFlags2::empty());
	}

	#[test]
	fn planner_selects_depth_aspect_for_d32_images() {
		let handle = Handles::Image(ImageHandle(4));
		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(
				handle,
				vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS,
				vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
				vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
			)],
			|_| Some((vk::Image::from_raw(99), vk::Format::D32_SFLOAT)),
			|_| None,
		);

		assert_eq!(planned.image_barriers.len(), 1);
		assert!(planned.image_barriers[0].aspect_mask == vk::ImageAspectFlags::DEPTH);
	}

	#[test]
	fn planner_skips_null_image_and_does_not_update_state() {
		let handle = Handles::Image(ImageHandle(5));
		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(
				handle,
				vk::PipelineStageFlags2::TRANSFER,
				vk::AccessFlags2::TRANSFER_WRITE,
				vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			)],
			|_| Some((vk::Image::null(), vk::Format::R8G8B8A8_UNORM)),
			|_| None,
		);

		assert!(planned.image_barriers.is_empty());
		assert!(planned.state_updates.is_empty());
	}

	#[test]
	fn planner_builds_buffer_barrier_from_previous_state() {
		let handle = Handles::Buffer(BufferHandle(6));
		let previous = transition(
			vk::PipelineStageFlags2::COPY,
			vk::AccessFlags2::TRANSFER_WRITE,
			vk::ImageLayout::UNDEFINED,
		);
		let destination = transition(
			vk::PipelineStageFlags2::VERTEX_INPUT,
			vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
			vk::ImageLayout::UNDEFINED,
		);
		let mut states = HashMap::default();
		states.insert(handle, previous);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| None,
			|_| Some(vk::Buffer::from_raw(111)),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];

		assert!(barrier.src_stage == previous.stage);
		assert!(barrier.src_access == previous.access);
		assert!(barrier.dst_stage == destination.stage);
		assert!(barrier.dst_access == destination.access);
		assert!(barrier.buffer == vk::Buffer::from_raw(111));
		assert_eq!(planned.state_updates.len(), 1);
		let (_, updated_state) = planned.state_updates[0];
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_skips_null_buffer_and_does_not_update_state() {
		let handle = Handles::Buffer(BufferHandle(7));
		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(
				handle,
				vk::PipelineStageFlags2::TRANSFER,
				vk::AccessFlags2::TRANSFER_WRITE,
				vk::ImageLayout::UNDEFINED,
			)],
			|_| None,
			|_| Some(vk::Buffer::null()),
		);

		assert!(planned.buffer_barriers.is_empty());
		assert!(planned.state_updates.is_empty());
	}

	#[test]
	fn planner_handles_vk_buffer_without_buffer_lookup() {
		let handle = Handles::VkBuffer(vk::Buffer::from_raw(222));
		let destination = transition(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_READ,
			vk::ImageLayout::UNDEFINED,
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| None,
			|_| panic!("buffer lookup must not be called for Handle::VkBuffer"),
		);

		assert_eq!(planned.buffer_barriers.len(), 1);
		let barrier = planned.buffer_barriers[0];

		assert!(barrier.src_stage == vk::PipelineStageFlags2::empty());
		assert!(barrier.src_access == vk::AccessFlags2::empty());
		assert!(barrier.buffer == vk::Buffer::from_raw(222));
		assert_eq!(planned.state_updates.len(), 1);
		let (updated_handle, updated_state) = planned.state_updates[0];

		assert!(updated_handle == handle);
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_builds_memory_barrier_for_acceleration_structures() {
		let handle = Handles::TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle(8));
		let previous = transition(
			vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
			vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
			vk::ImageLayout::UNDEFINED,
		);
		let destination = transition(
			vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR,
			vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR,
			vk::ImageLayout::UNDEFINED,
		);
		let mut states = HashMap::default();
		states.insert(handle, previous);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| None,
			|_| None,
		);

		assert_eq!(planned.memory_barriers.len(), 1);
		let barrier = planned.memory_barriers[0];

		assert!(barrier.src_stage == previous.stage);
		assert!(barrier.src_access == previous.access);
		assert!(barrier.dst_stage == destination.stage);
		assert!(barrier.dst_access == destination.access);
		assert_eq!(planned.state_updates.len(), 1);
		let (_, updated_state) = planned.state_updates[0];
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_updates_state_without_barrier_for_non_memory_handles() {
		let handle = Handles::Synchronizer(crate::synchronizer::SynchronizerHandle(9));
		let destination = transition(
			vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
			vk::AccessFlags2::empty(),
			vk::ImageLayout::UNDEFINED,
		);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&HashMap::default(),
			&HashMap::default(),
			[consumption(handle, destination.stage, destination.access, destination.layout)],
			|_| panic!("image lookup must not be called for synchronizers"),
			|_| panic!("buffer lookup must not be called for synchronizers"),
		);

		assert!(planned.image_barriers.is_empty());
		assert!(planned.buffer_barriers.is_empty());
		assert!(planned.memory_barriers.is_empty());
		assert_eq!(planned.state_updates.len(), 1);

		let (updated_handle, updated_state) = planned.state_updates[0];

		assert!(updated_handle == handle);
		assert_visible_state_eq(updated_state, destination);
	}

	#[test]
	fn planner_uses_original_state_for_each_duplicate_consumption() {
		let handle = Handles::Buffer(BufferHandle(10));
		let source = transition(
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_WRITE,
			vk::ImageLayout::UNDEFINED,
		);
		let first = transition(
			vk::PipelineStageFlags2::VERTEX_INPUT,
			vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
			vk::ImageLayout::UNDEFINED,
		);
		let second = transition(
			vk::PipelineStageFlags2::INDEX_INPUT,
			vk::AccessFlags2::INDEX_READ,
			vk::ImageLayout::UNDEFINED,
		);
		let mut states = HashMap::default();
		states.insert(handle, source);

		let planned = CommandBufferRecording::plan_vulkan_resource_transitions(
			&states,
			&HashMap::default(),
			[
				consumption(handle, first.stage, first.access, first.layout),
				consumption(handle, second.stage, second.access, second.layout),
			],
			|_| None,
			|_| Some(vk::Buffer::from_raw(333)),
		);

		assert_eq!(planned.buffer_barriers.len(), 2);
		let first_barrier = planned.buffer_barriers[0];
		let second_barrier = planned.buffer_barriers[1];

		assert!(first_barrier.src_stage == source.stage);
		assert!(first_barrier.src_access == source.access);
		assert!(second_barrier.src_stage == source.stage);
		assert!(second_barrier.src_access == source.access);
		assert_eq!(planned.state_updates.len(), 2);
		let (_, first_state) = planned.state_updates[0];
		let (_, second_state) = planned.state_updates[1];
		assert_visible_state_eq(first_state, first);
		assert_visible_state_eq(second_state, second);
	}
}
