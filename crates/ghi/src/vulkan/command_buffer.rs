use ash::vk::{self, Handle as _};
use smallvec::SmallVec;
use utils::{hash::HashMap, partition, Extent};

use super::{
	utils::{
		extent_into_vk_extent, texture_format_and_resource_use_to_image_layout, to_access_flags, to_clear_value,
		to_load_operation, to_pipeline_stage_flags, to_store_operation,
	},
	AccelerationStructure, BottomLevelAccelerationStructureHandle, Buffer, BufferHandle, BufferRange, BufferTransitionState,
	CommandBufferInternal, Consumption, Context, Descriptor, DescriptorSet, Handles, Image, ImageHandle, Swapchain,
	Synchronizer, TopLevelAccelerationStructureHandle, TransitionState, VulkanConsumption,
};
use crate::{descriptors::DescriptorSetHandle, graphics_hardware_interface, utils::StableVec, FrameKey, HandleLike as _, Size};

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

	bound_pipeline_layout: Option<crate::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<(u32, DescriptorSetHandle)>,
	bound_descriptor_sets_in_recording: Vec<DescriptorSetHandle>,
	active_rendering: bool,
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

impl CommandBufferRecording<'_> {
	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	pub(crate) fn new(
		device: &'_ mut Context,
		command_buffer: graphics_hardware_interface::CommandBufferHandle,
		frame_key: Option<FrameKey>,
	) -> CommandBufferRecording<'_> {
		let command_buffer = CommandBufferRecording {
			pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
			command_buffer,
			frame_key,
			sequence_index: frame_key.map(|f| f.sequence_index).unwrap_or(0),
			states: device.states.clone(),
			buffer_states: device.buffer_states.clone(),

			bound_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_set_handles: Vec::new(),
			bound_descriptor_sets_in_recording: Vec::new(),
			active_rendering: false,

			device,
		};

		command_buffer.begin();

		command_buffer
	}

	pub(crate) fn into_submission(
		mut self,
		presentation_keys: &[graphics_hardware_interface::PresentKey],
	) -> (
		graphics_hardware_interface::CommandBufferHandle,
		HashMap<Handles, TransitionState>,
		HashMap<Handles, Vec<BufferTransitionState>>,
	) {
		self.handle_swapchain_proxies(presentation_keys);
		self.consume_last_resources();
		self.end_recording();

		(self.command_buffer, self.states, self.buffer_states)
	}

	fn begin(&self) {
		let command_buffer = self.get_command_buffer();

		unsafe {
			self.device
				.device
				.reset_command_pool(command_buffer.command_pool, vk::CommandPoolResetFlags::empty())
				.expect("No command pool reset")
		};

		let command_buffer_begin_info =
			vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

		unsafe {
			self.device
				.device
				.begin_command_buffer(command_buffer.command_buffer, &command_buffer_begin_info)
				.expect("No command buffer begin")
		};
	}

	fn get_buffer(&self, buffer_handle: BufferHandle) -> &Buffer {
		self.device.buffers.resource(buffer_handle)
	}

	fn get_image(&self, image_handle: ImageHandle) -> &Image {
		&self.device.images[image_handle.0 as usize]
	}

	pub(crate) fn get_synchronizer(
		&self,
		syncronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> &Synchronizer {
		&self.device.synchronizers
			[self.device.get_syncronizer_handles(syncronizer_handle)[self.sequence_index as usize].0 as usize]
	}

	pub(crate) fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.device.swapchains[swapchain_handle.0 as usize]
	}

	fn get_internal_top_level_acceleration_structure_handle(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
	) -> TopLevelAccelerationStructureHandle {
		TopLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	fn get_top_level_acceleration_structure(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
	) -> (
		graphics_hardware_interface::TopLevelAccelerationStructureHandle,
		&AccelerationStructure,
	) {
		(
			acceleration_structure_handle,
			&self.device.acceleration_structures[acceleration_structure_handle.0 as usize],
		)
	}

	fn get_internal_bottom_level_acceleration_structure_handle(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) -> BottomLevelAccelerationStructureHandle {
		BottomLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	fn get_bottom_level_acceleration_structure(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) -> (
		graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
		&AccelerationStructure,
	) {
		(
			acceleration_structure_handle,
			&self.device.acceleration_structures[acceleration_structure_handle.0 as usize],
		)
	}

	pub(crate) fn get_command_buffer(&self) -> &CommandBufferInternal {
		&self.device.command_buffers[self.command_buffer.0 as usize].frames[self.sequence_index as usize]
	}

	fn get_internal_descriptor_set_handle(
		&self,
		descriptor_set_handle: graphics_hardware_interface::DescriptorSetHandle,
	) -> DescriptorSetHandle {
		let handles = DescriptorSetHandle(descriptor_set_handle.0).get_all(&self.device.descriptor_sets);
		handles[self.sequence_index as usize]
	}

	fn get_descriptor_set(&self, descriptor_set_handle: &DescriptorSetHandle) -> &DescriptorSet {
		&self.device.descriptor_sets[descriptor_set_handle.0 as usize]
	}

	/// Refreshes image descriptors for one internal descriptor set before it is used by a command buffer.
	/// TODO: replace with a better fitted solution
	fn refresh_image_descriptors_for_set(&mut self, descriptor_set_handle: DescriptorSetHandle) {
		let Some(bindings) = self.device.descriptors.get(&descriptor_set_handle) else {
			return;
		};

		let mut images: StableVec<vk::DescriptorImageInfo, 1024> = StableVec::new();
		let mut writes = Vec::new();

		for (binding_index, array_elements) in bindings {
			let Some(binding) = self
				.device
				.bindings
				.iter()
				.find(|binding| binding.descriptor_set_handle == descriptor_set_handle && binding.index == *binding_index)
			else {
				continue;
			};
			let descriptor_set = &self.device.descriptor_sets[descriptor_set_handle.0 as usize];

			for (array_element, descriptor) in array_elements {
				match descriptor {
					Descriptor::Image { image, layout } => {
						let image_resource = &self.device.images[image.0 as usize];
						let image_view = if !image_resource.full_image_view.is_null() {
							image_resource.full_image_view
						} else {
							image_resource.image_views[0]
						};

						if image_resource.image.is_null() || image_view.is_null() {
							continue;
						}

						let image_info = images.append([vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(
								image_resource.format_,
								*layout,
								None,
							))
							.image_view(image_view)]);

						writes.push(
							vk::WriteDescriptorSet::default()
								.dst_set(descriptor_set.descriptor_set)
								.dst_binding(*binding_index)
								.dst_array_element(*array_element)
								.descriptor_type(binding.descriptor_type)
								.image_info(&image_info),
						);
					}
					Descriptor::CombinedImageSampler { image, sampler, layout } => {
						let image_resource = &self.device.images[image.0 as usize];
						let image_view = if !image_resource.full_image_view.is_null() {
							image_resource.full_image_view
						} else {
							image_resource.image_views[0]
						};

						if image_resource.image.is_null() || image_view.is_null() {
							continue;
						}

						let image_info = images.append([vk::DescriptorImageInfo::default()
							.image_layout(texture_format_and_resource_use_to_image_layout(
								image_resource.format_,
								*layout,
								None,
							))
							.image_view(image_view)
							.sampler(*sampler)]);

						writes.push(
							vk::WriteDescriptorSet::default()
								.dst_set(descriptor_set.descriptor_set)
								.dst_binding(*binding_index)
								.dst_array_element(*array_element)
								.descriptor_type(binding.descriptor_type)
								.image_info(&image_info),
						);
					}
					_ => {}
				}
			}
		}

		if !writes.is_empty() {
			unsafe { self.device.device.update_descriptor_sets(&writes, &[]) };
		}
	}

	#[must_use]
	fn consume_resources_current(
		&self,
		additional_transitions: impl IntoIterator<Item = Consumption>,
	) -> Box<dyn FnOnce(&mut Self) -> ()> {
		let mut consumptions = Vec::with_capacity(32);

		let bound_pipeline_handle = self.bound_pipeline.expect("No bound pipeline");

		let pipeline = &self.device.pipelines[bound_pipeline_handle.0 as usize];

		for &((set_index, binding_index), (stages, access)) in &pipeline.resource_access {
			let set_handle = if let Some(&h) = self.bound_descriptor_set_handles.get(set_index as usize) {
				h.1
			} else {
				continue;
			};

			let resources = match self.device.descriptors.get(&set_handle).map(|d| d.get(&binding_index)) {
				Some(Some(b)) => b.values(),
				_ => {
					continue;
				}
			};

			for idk in resources {
				let (layout, handle) = match idk {
					Descriptor::Buffer { buffer, .. } => (crate::Layouts::General, Handles::Buffer(*buffer)),
					Descriptor::Image { layout, image } => (*layout, Handles::Image(*image)),
					Descriptor::CombinedImageSampler { image, layout, .. } => (*layout, Handles::Image(*image)),
					Descriptor::Swapchain { handle } => {
						let swapchain = &self.device.swapchains[handle.0 as usize];
						let image_index = swapchain.acquired_image_indices[self.sequence_index as usize] as usize;
						(crate::Layouts::General, Handles::Image(swapchain.images[image_index]))
					}
				};

				consumptions.push(Consumption {
					handle,
					stages,
					access,
					layout,
				});
			}
		}

		consumptions.extend(additional_transitions.into_iter().map(|c| Consumption {
			handle: c.handle,
			stages: c.stages,
			access: c.access,
			layout: c.layout,
		}));

		self.consume_resources(consumptions)
	}

	#[must_use]
	fn consume_resources(&self, consumptions: impl IntoIterator<Item = Consumption>) -> Box<dyn FnOnce(&mut Self) -> ()> {
		// Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).

		let consumptions = consumptions.into_iter().map(|consumption| {
			let format = match consumption.handle {
				Handles::Image(texture_handle) => {
					let image = self.get_image(texture_handle);
					Some(image.format_)
				}
				_ => None,
			};

			let stages = to_pipeline_stage_flags(consumption.stages, Some(consumption.layout), format);
			let access = to_access_flags(consumption.access, consumption.stages, consumption.layout, format);

			let layout = match consumption.handle {
				Handles::Image(image_handle) => {
					let image = self.get_image(image_handle);
					texture_format_and_resource_use_to_image_layout(image.format_, consumption.layout, Some(consumption.access))
				}
				_ => vk::ImageLayout::UNDEFINED,
			};

			VulkanConsumption {
				handle: consumption.handle,
				stages,
				access,
				layout,
				range: None,
			}
		});

		self.vulkan_consume_resources(consumptions)
	}

	/// Flags the passed resources as consumed.
	/// Consumptions are specified directly in Vulkan terms.
	#[must_use]
	fn vulkan_consume_resources(
		&self,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> Box<dyn FnOnce(&mut Self) -> ()> {
		Self::vulkan_consume_resources_impl(self.device, self, &self.states, consumptions)
	}

	#[must_use]
	fn vulkan_consume_resources_impl(
		device: &Context,
		command_buffer: &CommandBufferRecording,
		states: &HashMap<Handles, TransitionState>,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> Box<dyn FnOnce(&mut Self) -> ()> {
		let planned = Self::plan_vulkan_resource_transitions(
			states,
			&command_buffer.buffer_states,
			consumptions,
			|handle| {
				let image = command_buffer.get_image(handle);
				Some((image.image, image.format))
			},
			|handle| {
				let buffer = command_buffer.get_buffer(handle);
				Some(buffer.buffer)
			},
		);

		let active_rendering = command_buffer.active_rendering;

		if active_rendering {
			if planned
				.image_barriers
				.iter()
				.any(|barrier| barrier.old_layout != barrier.new_layout)
			{
				eprintln!(
					"Unable to transition image layout inside an active render pass. The most likely cause is that a graphics draw samples or stores an image that was not transitioned before vkCmdBeginRendering."
				);
			}

			let new_states = planned.state_updates;
			let buffer_state_updates = planned.buffer_state_updates;

			// Dynamic rendering without dynamicRenderingLocalRead cannot call vkCmdPipelineBarrier2 at
			// all while rendering is active. The state cache is still advanced so later passes do not
			// repeatedly try to emit the same invalid in-render-pass barrier.
			return Box::new(move |s: &mut Self| {
				for (handle, state) in new_states {
					s.states.insert(handle, state);
				}
				for (handle, states) in buffer_state_updates {
					s.buffer_states.insert(handle, states);
				}
			});
		}

		let folded_memory_barriers = planned.memory_barriers;

		let image_memory_barriers = if active_rendering {
			Vec::new()
		} else {
			planned
				.image_barriers
				.iter()
				.map(|barrier| {
					vk::ImageMemoryBarrier2::default()
						.old_layout(barrier.old_layout)
						.src_stage_mask(barrier.src_stage)
						.src_access_mask(barrier.src_access)
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(barrier.new_layout)
						.dst_stage_mask(barrier.dst_stage)
						.dst_access_mask(barrier.dst_access)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(barrier.image)
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: barrier.aspect_mask,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
				})
				.collect::<Vec<_>>()
		};

		let buffer_memory_barriers = if active_rendering {
			Vec::new()
		} else {
			planned
				.buffer_barriers
				.iter()
				.map(|barrier| {
					vk::BufferMemoryBarrier2::default()
						.src_stage_mask(barrier.src_stage)
						.src_access_mask(barrier.src_access)
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.dst_stage_mask(barrier.dst_stage)
						.dst_access_mask(barrier.dst_access)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.buffer(barrier.buffer)
						.offset(barrier.offset)
						.size(barrier.size)
				})
				.collect::<Vec<_>>()
		};

		let memory_barriers = folded_memory_barriers
			.iter()
			.map(|barrier| {
				vk::MemoryBarrier2::default()
					.src_stage_mask(barrier.src_stage)
					.src_access_mask(barrier.src_access)
					.dst_stage_mask(barrier.dst_stage)
					.dst_access_mask(barrier.dst_access)
			})
			.collect::<Vec<_>>();

		let new_states = planned.state_updates;
		let buffer_state_updates = planned.buffer_state_updates;

		let ret = move |s: &mut Self| {
			for (handle, state) in new_states {
				s.states.insert(handle, state);
			}
			for (handle, states) in buffer_state_updates {
				s.buffer_states.insert(handle, states);
			}
		};

		if image_memory_barriers.is_empty() && buffer_memory_barriers.is_empty() && memory_barriers.is_empty() {
			return Box::new(ret);
		} // Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.buffer_memory_barriers(&buffer_memory_barriers)
			.memory_barriers(&memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION);

		let command_buffer = command_buffer.get_command_buffer();

		unsafe {
			device
				.device
				.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info)
		};

		Box::new(ret)
	}

	fn plan_vulkan_resource_transitions(
		states: &HashMap<Handles, TransitionState>,
		buffer_states: &HashMap<Handles, Vec<BufferTransitionState>>,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
		mut resolve_image: impl FnMut(ImageHandle) -> Option<(vk::Image, vk::Format)>,
		mut resolve_buffer: impl FnMut(BufferHandle) -> Option<vk::Buffer>,
	) -> PlannedTransitions {
		let mut planned = PlannedTransitions::default();

		for consumption in consumptions {
			let source_state = states.get(&consumption.handle).copied();
			let mut transition_state = TransitionState::new(consumption.stages, consumption.access, consumption.layout);

			if let Some(source_state) = source_state {
				transition_state = transition_state.inherit_last_write_from(source_state);

				if !matches!(consumption.handle, Handles::Buffer(_))
					&& source_state == transition_state
					&& !TransitionState::access_includes_write(transition_state.access)
				{
					continue;
				}
			}

			let (mut src_stage, mut src_access, src_layout) = if let Some(source_state) = source_state {
				(source_state.stage, source_state.access, source_state.layout)
			} else {
				(
					vk::PipelineStageFlags2::empty(),
					vk::AccessFlags2::empty(),
					vk::ImageLayout::UNDEFINED,
				)
			};

			match consumption.handle {
				Handles::Image(handle) => {
					let Some((image, format)) = resolve_image(handle) else {
						continue;
					};

					if image.is_null() {
						continue;
					}

					planned.image_barriers.push(PlannedImageBarrier {
						old_layout: src_layout,
						src_stage,
						src_access,
						new_layout: transition_state.layout,
						dst_stage: transition_state.stage,
						dst_access: transition_state.access,
						image,
						aspect_mask: if format != vk::Format::D32_SFLOAT {
							vk::ImageAspectFlags::COLOR
						} else {
							vk::ImageAspectFlags::DEPTH
						},
					});
				}
				Handles::Buffer(handle) => {
					let Some(buffer) = resolve_buffer(handle) else {
						continue;
					};

					if buffer.is_null() {
						continue;
					}

					let range = consumption.range.unwrap_or(BufferRange::new(0, vk::WHOLE_SIZE));
					let overlapping_states = buffer_states
						.get(&consumption.handle)
						.into_iter()
						.flatten()
						.filter(|state| state.range.overlaps(range))
						.copied()
						.collect::<Vec<_>>();

					if !TransitionState::access_includes_write(transition_state.access) {
						transition_state.last_write_stage = vk::PipelineStageFlags2::empty();
						transition_state.last_write_access = vk::AccessFlags2::empty();

						for overlapping_state in &overlapping_states {
							transition_state.last_write_stage |= overlapping_state.state.last_write_stage;
							transition_state.last_write_access |= overlapping_state.state.last_write_access;
						}

						if overlapping_states.is_empty() {
							if let Some(source_state) = source_state {
								transition_state = transition_state.inherit_last_write_from(source_state);
							}
						}
					}

					for overlapping_state in &overlapping_states {
						let mut range_src_stage = overlapping_state.state.stage;
						let mut range_src_access = overlapping_state.state.access;

						if TransitionState::access_includes_write(transition_state.access) {
							range_src_stage |= overlapping_state.state.last_write_stage;
							range_src_access |= overlapping_state.state.last_write_access;
						}

						planned.buffer_barriers.push(PlannedBufferBarrier {
							src_stage: range_src_stage,
							src_access: range_src_access,
							dst_stage: transition_state.stage,
							dst_access: transition_state.access,
							buffer,
							offset: range.offset,
							size: range.size,
						});
					}

					if overlapping_states.is_empty() && consumption.range.is_none() {
						planned.buffer_barriers.push(PlannedBufferBarrier {
							src_stage,
							src_access,
							dst_stage: transition_state.stage,
							dst_access: transition_state.access,
							buffer,
							offset: 0,
							size: vk::WHOLE_SIZE,
						});
					}

					planned.update_buffer_state(consumption.handle, range, transition_state, buffer_states);
				}
				Handles::VkBuffer(buffer) => {
					planned.buffer_barriers.push(PlannedBufferBarrier {
						src_stage,
						src_access,
						dst_stage: transition_state.stage,
						dst_access: transition_state.access,
						buffer,
						offset: consumption.range.map(|range| range.offset).unwrap_or(0),
						size: consumption.range.map(|range| range.size).unwrap_or(vk::WHOLE_SIZE),
					});
				}
				Handles::TopLevelAccelerationStructure(_) | Handles::BottomLevelAccelerationStructure(_) => {
					planned.memory_barriers.push(PlannedMemoryBarrier {
						src_stage,
						src_access,
						dst_stage: transition_state.stage,
						dst_access: transition_state.access,
					});
				}
				_ => {}
			}

			planned.state_updates.push((consumption.handle, transition_state));
		}

		planned
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> ImageHandle {
		if let Some(swapchain) = self
			.device
			.swapchains
			.iter()
			.find(|swapchain| swapchain.images[0].0 == handle.0 .0 || swapchain.native_images[0].0 == handle.0 .0)
		{
			return swapchain.images[swapchain.acquired_image_indices[self.sequence_index as usize] as usize];
		}

		let handles = ImageHandle(handle.0 .0).get_all(&self.device.images);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	fn get_internal_base_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.get_internal_image_handle(graphics_hardware_interface::ImageHandle(handle))
	}

	fn get_attachment_image_handle(&self, attachment: &graphics_hardware_interface::AttachmentInformation) -> ImageHandle {
		match attachment.target {
			graphics_hardware_interface::ImageOrSwapchain::Image(handle) => self.get_internal_base_image_handle(handle),
			graphics_hardware_interface::ImageOrSwapchain::Swapchain(handle) => {
				let swapchain = &self.device.swapchains[handle.0 as usize];
				swapchain.images[swapchain.acquired_image_indices[self.sequence_index as usize] as usize]
			}
		}
	}

	fn get_attachment_format(&self, attachment: &graphics_hardware_interface::AttachmentInformation) -> crate::Formats {
		attachment
			.format
			.unwrap_or_else(|| self.get_image(self.get_attachment_image_handle(attachment)).format_)
	}

	fn get_internal_handle(&self, handle: graphics_hardware_interface::Handles) -> Handles {
		match handle {
			graphics_hardware_interface::Handles::Image(handle) => {
				Handles::Image(self.get_internal_image_handle(handle.into()))
			}
			graphics_hardware_interface::Handles::Buffer(handle) => Handles::Buffer(self.get_internal_buffer_handle(handle)),
			graphics_hardware_interface::Handles::TopLevelAccelerationStructure(handle) => {
				Handles::TopLevelAccelerationStructure(self.get_internal_top_level_acceleration_structure_handle(handle))
			}
			graphics_hardware_interface::Handles::BottomLevelAccelerationStructure(handle) => {
				Handles::BottomLevelAccelerationStructure(self.get_internal_bottom_level_acceleration_structure_handle(handle))
			}
			_ => unimplemented!(),
		}
	}

	pub(crate) fn get_presentable_swapchain_image_handle(
		&self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> ImageHandle {
		let swapchain = self.get_swapchain(present_key.swapchain);
		swapchain.native_images[present_key.image_index as usize]
	}

	fn blit_image_to_image(&mut self, source_image_handle: ImageHandle, destination_image_handle: ImageHandle) {
		// Performs a transfer-domain blit from source image to destination image,
		// including the required layout transitions tracked through `self.states`.
		let (source_extent, source_vk_image) = {
			let image = self.get_image(source_image_handle);
			(image.extent, image.image)
		};
		let (destination_extent_raw, destination_vk_image) = {
			let image = self.get_image(destination_image_handle);
			(image.extent, image.image)
		};

		let destination_extent = if destination_extent_raw.width() == 0
			|| destination_extent_raw.height() == 0
			|| destination_extent_raw.depth() == 0
		{
			source_extent
		} else {
			destination_extent_raw
		};

		if source_extent.width() == 0 || destination_extent.width() == 0 {
			return;
		}

		self.states.insert(
			Handles::Image(destination_image_handle),
			TransitionState::new(
				vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
					| vk::PipelineStageFlags2::BLIT
					| vk::PipelineStageFlags2::TRANSFER,
				vk::AccessFlags2::NONE,
				vk::ImageLayout::UNDEFINED,
			),
		);

		self.consume_resources([
			Consumption {
				handle: Handles::Image(source_image_handle),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			},
			Consumption {
				handle: Handles::Image(destination_image_handle),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			},
		])(self);

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let image_blits = [vk::ImageBlit2::default()
			.src_subresource(
				vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1),
			)
			.src_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default()
					.x(source_extent.width() as i32)
					.y(source_extent.height().max(1) as i32)
					.z(source_extent.depth().max(1) as i32),
			])
			.dst_subresource(
				vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1),
			)
			.dst_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default()
					.x(destination_extent.width() as i32)
					.y(destination_extent.height().max(1) as i32)
					.z(destination_extent.depth().max(1) as i32),
			])];

		let copy_image_info = vk::BlitImageInfo2::default()
			.src_image(source_vk_image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(destination_vk_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_blits);

		unsafe {
			self.device.device.cmd_blit_image2(vk_command_buffer, &copy_image_info);
		}

		self.consume_resources([Consumption {
			handle: Handles::Image(source_image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::NONE,
			layout: crate::Layouts::General,
		}])(self);
	}

	pub fn handle_swapchain_proxies(&mut self, presentation_keys: &[graphics_hardware_interface::PresentKey]) {
		let proxy_copies = presentation_keys
			.iter()
			.filter_map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let proxy_image = swapchain.images[present_key.image_index as usize];
				let native_image = swapchain.native_images[present_key.image_index as usize];

				if proxy_image == native_image {
					return None;
				}

				Some((proxy_image, native_image))
			})
			.collect::<SmallVec<[(ImageHandle, ImageHandle); 8]>>();

		// When the swapchain uses proxies, resolve each user-facing proxy image into
		// the native presentable swapchain image before transitioning to present.
		for (proxy_image_handle, native_image_handle) in proxy_copies {
			self.blit_image_to_image(proxy_image_handle, native_image_handle);
		}

		let present_transitions = presentation_keys.iter().map(|present_key| {
			let swapchain_image_handle = self.get_presentable_swapchain_image_handle(*present_key);

			Consumption {
				handle: Handles::Image(swapchain_image_handle),
				stages: crate::Stages::PRESENTATION,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Present,
			}
		});

		self.consume_resources(present_transitions)(self);
	}

	// Transition all resources which where written to but not consumed by any previous command
	// If this is skipped validation layers (correctly) complain about missing sync even though no "read" operation was performed, except for the following commands
	pub(crate) fn consume_last_resources(&mut self) {
		let consumptions = self.states.iter().filter_map(|(handle, ts)| match ts.access {
			vk::AccessFlags2::TRANSFER_WRITE => Some(Consumption {
				access: crate::AccessPolicies::NONE,
				layout: crate::Layouts::General,
				stages: crate::Stages::TRANSFER,
				handle: *handle,
			}),
			_ => None,
		});

		self.consume_resources(consumptions)(self);
	}

	pub fn end_recording(&self) {
		let command_buffer = self.get_command_buffer().command_buffer;

		unsafe {
			self.device
				.device
				.end_command_buffer(command_buffer)
				.expect("Failed to end command buffer.");
		}
	}

	pub(crate) fn sync_buffers(&mut self, copy_buffers: impl Iterator<Item = BufferCopy> + Clone) {
		let source_consumptions = copy_buffers.clone().map(|e| VulkanConsumption {
			handle: Handles::Buffer(e.src_buffer),
			stages: vk::PipelineStageFlags2::COPY,
			access: vk::AccessFlags2::TRANSFER_READ,
			layout: vk::ImageLayout::UNDEFINED,
			range: Some(BufferRange::new(e.src_offset, e.size as vk::DeviceSize)),
		});
		let destination_consumptions = copy_buffers.clone().map(|e| VulkanConsumption {
			handle: Handles::Buffer(e.dst_buffer),
			stages: vk::PipelineStageFlags2::COPY,
			access: vk::AccessFlags2::TRANSFER_WRITE,
			layout: vk::ImageLayout::UNDEFINED,
			range: Some(BufferRange::new(e.dst_offset, e.size as vk::DeviceSize)),
		});
		self.vulkan_consume_resources(source_consumptions.chain(destination_consumptions))(self);

		for e in copy_buffers {
			// Copy all staging buffers to their respective buffers
			let src_buffer = self.get_buffer(e.src_buffer);
			let dst_buffer = self.get_buffer(e.dst_buffer);

			let src_vk_buffer = src_buffer.buffer;
			let dst_vk_buffer = dst_buffer.buffer;

			let command_buffer = self.get_command_buffer();

			let regions = [vk::BufferCopy2KHR::default()
				.src_offset(e.src_offset)
				.dst_offset(e.dst_offset)
				.size(e.size as u64)];

			let copy_buffer_info = vk::CopyBufferInfo2KHR::default()
				.src_buffer(src_vk_buffer)
				.dst_buffer(dst_vk_buffer)
				.regions(&regions);

			unsafe {
				self.device
					.device
					.cmd_copy_buffer2(command_buffer.command_buffer, &copy_buffer_info);
			}
		}
	}

	pub(crate) fn sync_textures(&mut self, copy_textures: impl Iterator<Item = ImageCopy> + Clone) {
		let copied_textures = copy_textures.clone();

		self.vulkan_consume_resources(copy_textures.clone().map(|e| VulkanConsumption {
			handle: Handles::Image(e.dst_texture),
			stages: vk::PipelineStageFlags2::TRANSFER,
			access: vk::AccessFlags2::TRANSFER_WRITE,
			layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			range: None,
		}))(self);

		let command_buffer = self.get_command_buffer();

		for copy_texture in copied_textures {
			let image = self.get_image(copy_texture.dst_texture);

			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(0)
				.buffer_row_length(0)
				.buffer_image_height(0)
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(vk::ImageAspectFlags::COLOR)
						.mip_level(0)
						.base_array_layer(0)
						.layer_count(1),
				)
				.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
				.image_extent(extent_into_vk_extent(image.extent))];

			let buffer = image.staging_buffer.unwrap();

			// Copy to images from staging buffer
			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(buffer)
				.dst_image(image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.device
					.device
					.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
			}
		}

		self.consume_resources(copy_textures.map(|e| Consumption {
			handle: Handles::Image(e.dst_texture),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}))(self);
	}
}

impl crate::command_buffer::CommandBufferRecording for CommandBufferRecording<'_> {
	fn frame_key(&self) -> FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn transfer_textures(
		&mut self,
		image_handles: &[graphics_hardware_interface::BaseImageHandle],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		self.consume_resources(image_handles.iter().map(|image_handle| Consumption {
			handle: Handles::Image(self.get_internal_base_image_handle(*image_handle)),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Transfer,
		}))(self);

		let buffer_handles = image_handles.iter().filter_map(|image_handle| {
			self.get_image(self.get_internal_base_image_handle(*image_handle))
				.staging_buffer
		});

		self.vulkan_consume_resources(buffer_handles.map(|buffer_handle| VulkanConsumption {
			handle: Handles::VkBuffer(buffer_handle),
			stages: vk::PipelineStageFlags2::TRANSFER,
			access: vk::AccessFlags2::TRANSFER_WRITE,
			layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			range: None,
		}))(self);

		let command_buffer = self.get_command_buffer();
		let command_buffer = command_buffer.command_buffer;

		for image_handle in image_handles {
			let image = self.get_image(self.get_internal_base_image_handle(*image_handle));
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(staging_buffer_handle) = image.staging_buffer {
				let regions = [vk::BufferImageCopy2KHR::default()
					.buffer_offset(0)
					.buffer_row_length(0)
					.buffer_image_height(0)
					.image_subresource(
						vk::ImageSubresourceLayers::default()
							.aspect_mask(vk::ImageAspectFlags::COLOR)
							.mip_level(0)
							.base_array_layer(0)
							.layer_count(1),
					)
					.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
					.image_extent(extent_into_vk_extent(image.extent))];

				let copy_image_to_buffer_info = vk::CopyImageToBufferInfo2KHR::default()
					.src_image(image.image)
					.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
					.dst_buffer(staging_buffer_handle)
					.regions(&regions);

				unsafe {
					self.device
						.device
						.cmd_copy_image_to_buffer2(command_buffer, &copy_image_to_buffer_info);
				}
			}
		}

		let mut texture_copies = Vec::new();

		for image_handle in image_handles {
			let internal_image_handle = self.get_internal_base_image_handle(*image_handle);
			let image = self.get_image(internal_image_handle);
			if let Some(_) = image.staging_buffer {
				texture_copies.push(graphics_hardware_interface::TextureCopyHandle(internal_image_handle.0));
			}
		}

		texture_copies
	}

	fn copy_images_to_buffer(&mut self, _copies: &[crate::ImageBufferCopyDescriptor]) {
		panic!(
			"Vulkan image-to-buffer copy is not implemented. The most likely cause is that this backend has not been wired for arbitrary texture readback buffers."
		);
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl crate::command_buffer::RasterizationRenderPassMode {
		self.consume_resources(attachments.iter().map(|attachment| Consumption {
			handle: Handles::Image(self.get_attachment_image_handle(attachment)),
			stages: crate::Stages::FRAGMENT,
			access: if attachment.load {
				crate::AccessPolicies::READ_WRITE
			} else {
				crate::AccessPolicies::WRITE
			},
			layout: attachment.layout,
		}))(self);

		let render_area = vk::Rect2D::default()
			.offset(vk::Offset2D::default().x(0).y(0))
			.extent(vk::Extent2D::default().width(extent.width()).height(extent.height()));

		let color_attchments = attachments
			.iter()
			.filter(|a| self.get_attachment_format(a) != crate::Formats::Depth32)
			.map(|attachment| {
				let image = self.get_image(self.get_attachment_image_handle(attachment));
				let format = self.get_attachment_format(attachment);
				let image_view = image.image_views[attachment.layer.unwrap_or(0) as usize];

				if image_view.is_null() && image.extent.width() == 0 && image.extent.height() == 0 && image.extent.depth() == 0 {
					eprintln!("Creating a render pass with a color attachment from an image that has no image view and no extent. Image was likely created with extent 0 and resize was not called prior to rendering.");
				}

				vk::RenderingAttachmentInfo::default()
					.image_view(image_view)
					.image_layout(texture_format_and_resource_use_to_image_layout(format, attachment.layout, None))
					.load_op(to_load_operation(attachment.load))
					.store_op(to_store_operation(attachment.store))
					.clear_value(to_clear_value(attachment.clear))
			})
			.collect::<Vec<_>>();

		let depth_attachment = attachments
			.iter()
			.find(|attachment| self.get_attachment_format(attachment) == crate::Formats::Depth32)
			.map(|attachment| {
				let image = self.get_image(self.get_attachment_image_handle(attachment));
				let format = self.get_attachment_format(attachment);
				let image_view = image.image_views[attachment.layer.unwrap_or(0) as usize];

				vk::RenderingAttachmentInfo::default()
					.image_view(image_view)
					.image_layout(texture_format_and_resource_use_to_image_layout(
						format,
						attachment.layout,
						None,
					))
					.load_op(to_load_operation(attachment.load))
					.store_op(to_store_operation(attachment.store))
					.clear_value(to_clear_value(attachment.clear))
			})
			.or(Some(vk::RenderingAttachmentInfo::default()))
			.unwrap();

		let rendering_info = vk::RenderingInfoKHR::default()
			.color_attachments(color_attchments.as_slice())
			.depth_attachment(&depth_attachment)
			.render_area(render_area)
			.layer_count(1);

		let viewports = [vk::Viewport {
			x: 0.0,
			y: (extent.height() as f32),
			width: extent.width() as f32,
			height: -(extent.height() as f32),
			min_depth: 0.0,
			max_depth: 1.0,
		}];

		let command_buffer = self.get_command_buffer();

		unsafe {
			self.device
				.device
				.cmd_set_scissor(command_buffer.command_buffer, 0, &[render_area]);
		}
		unsafe {
			self.device
				.device
				.cmd_set_viewport(command_buffer.command_buffer, 0, &viewports);
		}
		unsafe {
			self.device
				.device
				.cmd_begin_rendering(command_buffer.command_buffer, &rendering_info);
		}

		self.active_rendering = true;

		self
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		let (acceleration_structure_handle, acceleration_structure) =
			self.get_top_level_acceleration_structure(acceleration_structure_build.acceleration_structure);

		let (as_geometries, offsets) = match acceleration_structure_build.description {
			crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
				instances_buffer,
				instance_count,
			} => (
				vec![vk::AccelerationStructureGeometryKHR::default()
					.geometry_type(vk::GeometryTypeKHR::INSTANCES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
							.array_of_pointers(false)
							.data(vk::DeviceOrHostAddressConstKHR {
								device_address: self.device.get_buffer_address(instances_buffer),
							}),
					})
					.flags(vk::GeometryFlagsKHR::OPAQUE)],
				vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
					.primitive_count(instance_count)
					.primitive_offset(0)
					.first_vertex(0)
					.transform_offset(0)],
			),
		};

		let scratch_buffer_address = unsafe {
			let buffer = self.get_buffer(self.get_internal_buffer_handle(acceleration_structure_build.scratch_buffer.buffer));
			self.device
				.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
				+ acceleration_structure_build.scratch_buffer.offset as u64
		};

		let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.dst_acceleration_structure(acceleration_structure.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR {
				device_address: scratch_buffer_address,
			});

		self.states.insert(
			Handles::TopLevelAccelerationStructure(
				self.get_internal_top_level_acceleration_structure_handle(acceleration_structure_handle),
			),
			TransitionState::new(
				vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
				vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
				vk::ImageLayout::UNDEFINED,
			),
		);

		let infos = vec![build_geometry_info];
		let build_range_infos = vec![offsets];
		let geometries = vec![as_geometries];

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let infos = infos
			.iter()
			.zip(geometries.iter())
			.map(|(info, geos)| info.geometries(geos))
			.collect::<Vec<_>>();

		let build_range_infos = build_range_infos
			.iter()
			.map(|build_range_info| build_range_info.as_slice())
			.collect::<Vec<_>>();

		unsafe {
			self.device
				.acceleration_structure
				.cmd_build_acceleration_structures(vk_command_buffer, &infos, &build_range_infos)
		}
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		if acceleration_structure_builds.is_empty() {
			return;
		}

		fn visit(
			this: &mut CommandBufferRecording,
			acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
			mut infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR>,
			mut geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR>>,
			mut build_range_infos: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>>,
		) {
			if let Some(build) = acceleration_structure_builds.first() {
				let (acceleration_structure_handle, acceleration_structure) =
					this.get_bottom_level_acceleration_structure(build.acceleration_structure);

				let (as_geometries, offsets) = match &build.description {
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB { .. } => (vec![], vec![]),
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer,
						index_buffer,
						vertex_position_encoding,
						index_format,
						triangle_count,
						vertex_count,
					} => {
						let vertex_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(vertex_buffer.buffer_offset.buffer));
							this.device
								.device
								.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
								+ vertex_buffer.buffer_offset.offset as u64
						};

						let index_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(index_buffer.buffer_offset.buffer));
							this.device
								.device
								.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
								+ index_buffer.buffer_offset.offset as u64
						};

						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR {
								device_address: vertex_data_address,
							})
							.index_data(vk::DeviceOrHostAddressConstKHR {
								device_address: index_data_address,
							})
							.max_vertex(vertex_count - 1)
							.vertex_format(match vertex_position_encoding {
								crate::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position encoding"),
							})
							.index_type(match index_format {
								crate::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								crate::DataTypes::U16 => vk::IndexType::UINT16,
								crate::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
							.vertex_stride(vertex_buffer.stride as vk::DeviceSize);

						let build_range_info = vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
							.primitive_count(*triangle_count)
							.primitive_offset(0)
							.first_vertex(0)
							.transform_offset(0)];

						(
							vec![vk::AccelerationStructureGeometryKHR::default()
								.flags(vk::GeometryFlagsKHR::OPAQUE)
								.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
								.geometry(vk::AccelerationStructureGeometryDataKHR { triangles })],
							build_range_info,
						)
					}
				};

				let scratch_buffer_address = unsafe {
					let buffer = this.get_buffer(this.get_internal_buffer_handle(build.scratch_buffer.buffer));
					this.device
						.device
						.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
						+ build.scratch_buffer.offset as u64
				};

				let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.dst_acceleration_structure(acceleration_structure.acceleration_structure)
					.scratch_data(vk::DeviceOrHostAddressKHR {
						device_address: scratch_buffer_address,
					});

				this.states.insert(
					Handles::BottomLevelAccelerationStructure(
						this.get_internal_bottom_level_acceleration_structure_handle(acceleration_structure_handle),
					),
					TransitionState::new(
						vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
						vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
						vk::ImageLayout::UNDEFINED,
					),
				);

				infos.push(build_geometry_info);
				build_range_infos.push(offsets);
				geometries.push(as_geometries);

				visit(
					this,
					&acceleration_structure_builds[1..],
					infos,
					geometries,
					build_range_infos,
				);
			} else {
				let command_buffer = this.get_command_buffer();

				let infos = infos
					.iter()
					.zip(geometries.iter())
					.map(|(info, geos)| info.geometries(geos))
					.collect::<Vec<_>>();

				let build_range_infos = build_range_infos
					.iter()
					.map(|build_range_info| build_range_info.as_slice())
					.collect::<Vec<_>>();

				unsafe {
					this.device.acceleration_structure.cmd_build_acceleration_structures(
						command_buffer.command_buffer,
						&infos,
						&build_range_infos,
					)
				}
			}
		}

		visit(self, acceleration_structure_builds, Vec::new(), Vec::new(), Vec::new());
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		destination_layout: crate::Layouts,
	) {
		self.consume_resources([
			Consumption {
				handle: Handles::Image(self.get_internal_base_image_handle(source_image)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: source_layout,
			},
			Consumption {
				handle: Handles::Image(self.get_internal_base_image_handle(destination_image)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: destination_layout,
			},
		])(self);

		let command_buffer = self.get_command_buffer();
		let source_image = self.get_image(self.get_internal_base_image_handle(source_image));
		let destination_image = self.get_image(self.get_internal_base_image_handle(destination_image));
		unsafe {
			let blit = vk::ImageBlit2::default()
				.src_subresource(vk::ImageSubresourceLayers {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					mip_level: 0,
					base_array_layer: 0,
					layer_count: 1,
				})
				.src_offsets([
					vk::Offset3D { x: 0, y: 0, z: 0 },
					vk::Offset3D {
						x: source_image.extent.width() as i32,
						y: source_image.extent.height() as i32,
						z: 1,
					},
				])
				.dst_subresource(vk::ImageSubresourceLayers {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					mip_level: 0,
					base_array_layer: 0,
					layer_count: 1,
				})
				.dst_offsets([
					vk::Offset3D { x: 0, y: 0, z: 0 },
					vk::Offset3D {
						x: destination_image.extent.width() as i32,
						y: destination_image.extent.height() as i32,
						z: 1,
					},
				]);

			let blits = [blit];

			let blit_info = vk::BlitImageInfo2::default()
				.src_image(source_image.image)
				.src_image_layout(texture_format_and_resource_use_to_image_layout(
					source_image.format_,
					source_layout,
					Some(crate::AccessPolicies::READ),
				))
				.dst_image(destination_image.image)
				.dst_image_layout(texture_format_and_resource_use_to_image_layout(
					destination_image.format_,
					destination_layout,
					Some(crate::AccessPolicies::WRITE),
				))
				.regions(&blits)
				.filter(vk::Filter::LINEAR);
			self.device.device.cmd_blit_image2(command_buffer.command_buffer, &blit_info);
		}
	}

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		self.consume_resources(textures.iter().map(|(image_handle, _)| Consumption {
			handle: Handles::Image(self.get_internal_base_image_handle(*image_handle)),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}))(self);

		for (image_handle, clear_value) in textures {
			let image = self.get_image(self.get_internal_base_image_handle(*image_handle));

			if image.image.is_null() {
				continue;
			} // Skip unset textures

			if image.format_ != crate::Formats::Depth32 {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearColorValue {
						float32: [0.0, 0.0, 0.0, 0.0],
					},
					graphics_hardware_interface::ClearValue::Color(color) => vk::ClearColorValue {
						float32: [color.r, color.g, color.b, color.a],
					},
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearColorValue {
						float32: [*depth, 0.0, 0.0, 0.0],
					},
					graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearColorValue {
						uint32: [*r, *g, *b, *a],
					},
				};

				unsafe {
					self.device.device.cmd_clear_color_image(
						self.get_command_buffer().command_buffer,
						image.image,
						vk::ImageLayout::TRANSFER_DST_OPTIMAL,
						&clear_value,
						&[vk::ImageSubresourceRange {
							aspect_mask: vk::ImageAspectFlags::COLOR,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						}],
					);
				}
			} else {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearDepthStencilValue { depth: 0.0, stencil: 0 },
					graphics_hardware_interface::ClearValue::Color(_) => {
						panic!("Color clear value for depth texture")
					}
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearDepthStencilValue {
						depth: *depth,
						stencil: 0,
					},
					graphics_hardware_interface::ClearValue::Integer(..) => {
						panic!("Integer clear value for depth texture")
					}
				};

				unsafe {
					self.device.device.cmd_clear_depth_stencil_image(
						self.get_command_buffer().command_buffer,
						image.image,
						vk::ImageLayout::TRANSFER_DST_OPTIMAL,
						&clear_value,
						&[vk::ImageSubresourceRange {
							aspect_mask: vk::ImageAspectFlags::DEPTH,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						}],
					);
				}
			}
		}
	}

	fn copy_buffers(&mut self, copies: &[crate::BufferCopyDescriptor]) {
		let copies = copies
			.iter()
			.map(|copy| {
				BufferCopy::new(
					self.get_internal_buffer_handle(copy.source_buffer),
					copy.source_offset as vk::DeviceSize,
					self.get_internal_buffer_handle(copy.destination_buffer),
					copy.destination_offset as vk::DeviceSize,
					copy.size,
				)
			})
			.collect::<Vec<_>>();
		self.sync_buffers(copies.into_iter());
	}

	fn copy_buffer_to_images(&mut self, copies: &[crate::BufferImageCopyDescriptor]) {
		let consumptions = copies
			.iter()
			.flat_map(|copy| {
				[
					Consumption {
						handle: Handles::Buffer(self.get_internal_buffer_handle(copy.source_buffer)),
						stages: crate::Stages::TRANSFER,
						access: crate::AccessPolicies::READ,
						layout: crate::Layouts::Transfer,
					},
					Consumption {
						handle: Handles::Image(self.get_internal_base_image_handle(copy.destination_image)),
						stages: crate::Stages::TRANSFER,
						access: crate::AccessPolicies::WRITE,
						layout: crate::Layouts::Transfer,
					},
				]
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions)(self);

		let command_buffer = self.get_command_buffer().command_buffer;

		for copy in copies {
			let source_buffer_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_image_handle = self.get_internal_base_image_handle(copy.destination_image);
			let source_buffer = self.get_buffer(source_buffer_handle);
			let destination_image = self.get_image(destination_image_handle);
			let source_row_count = copy.source_bytes_per_image / copy.source_bytes_per_row;

			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(copy.source_offset as _)
				.buffer_row_length(buffer_row_length(destination_image.format_, copy.source_bytes_per_row))
				.buffer_image_height(buffer_image_height(destination_image.format_, source_row_count))
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(vk::ImageAspectFlags::COLOR)
						.mip_level(0)
						.base_array_layer(0)
						.layer_count(destination_image.layers.map(|layers| layers.get()).unwrap_or(1)),
				)
				.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
				.image_extent(extent_into_vk_extent(destination_image.extent))];

			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(source_buffer.buffer)
				.dst_image(destination_image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.device
					.device
					.cmd_copy_buffer_to_image2(command_buffer, &buffer_image_copy);
			}
		}

		self.consume_resources(copies.iter().map(|copy| Consumption {
			handle: Handles::Image(self.get_internal_base_image_handle(copy.destination_image)),
			stages: crate::Stages::COMPUTE | crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}))(self);
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		self.consume_resources(buffer_handles.iter().map(|buffer_handle| Consumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}))(self);

		for buffer_handle in buffer_handles {
			let internal_buffer_handle = self.get_internal_buffer_handle(*buffer_handle);
			let buffer = self.get_buffer(internal_buffer_handle);

			if buffer.buffer.is_null() {
				continue;
			}

			unsafe {
				self.device.device.cmd_fill_buffer(
					self.get_command_buffer().command_buffer,
					buffer.buffer,
					0,
					vk::WHOLE_SIZE,
					0,
				);
			}

			self.states.insert(
				Handles::Buffer(internal_buffer_handle),
				TransitionState::new(
					vk::PipelineStageFlags2::TRANSFER,
					vk::AccessFlags2::TRANSFER_WRITE,
					vk::ImageLayout::UNDEFINED,
				),
			);
		}
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let internal_image_handle = self.get_internal_base_image_handle(image_handle);

		self.consume_resources([Consumption {
			handle: Handles::Image(internal_image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}])(self);

		let texture = self.get_image(internal_image_handle);

		let buffer = texture.staging_buffer.unwrap();
		let pointer = texture.pointer.unwrap();

		let subresource_layout = self
			.device
			.get_image_subresource_layout(&graphics_hardware_interface::ImageHandle(image_handle), 0);

		if pointer.is_null() {
			for i in data.len()
				..texture.extent.width() as usize
					* texture.extent.height().max(1) as usize
					* texture.extent.depth().max(1) as usize
			{
				unsafe {
					std::ptr::write(pointer.offset(i as isize), if i % 4 == 0 { 255 } else { 0 });
				}
			}
		} else {
			let pointer = unsafe { pointer.offset(subresource_layout.offset as isize) };

			for i in 0..texture.extent.height() {
				let pointer = unsafe { pointer.offset(subresource_layout.row_pitch as isize * i as isize) };

				unsafe {
					std::ptr::copy_nonoverlapping(
						(data.as_ptr().add(i as usize * texture.extent.width() as usize)) as *mut u8,
						pointer,
						texture.extent.width() as usize * 4,
					);
				}
			}
		}

		let regions = [vk::BufferImageCopy2::default()
			.buffer_offset(0)
			.buffer_row_length(0)
			.buffer_image_height(0)
			.image_subresource(
				vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1),
			)
			.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
			.image_extent(extent_into_vk_extent(texture.extent))];

		// Copy to images from staging buffer
		let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
			.src_buffer(buffer)
			.dst_image(texture.image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&regions);

		let command_buffer = self.get_command_buffer();

		unsafe {
			self.device
				.device
				.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
		}

		self.consume_resources([Consumption {
			handle: Handles::Image(internal_image_handle),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}])(self);
	}

	fn execute(mut self, synchronizer: crate::SynchronizerHandle) {
		self.consume_last_resources();
		self.end_recording();

		let command_buffer = self.get_command_buffer();
		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.command_buffer)];
		let submit_info = vk::SubmitInfo2::default().command_buffer_infos(&command_buffer_infos);
		let synchronizer_handle = self.device.get_syncronizer_handles(synchronizer)[self.sequence_index as usize];
		let synchronizer = &self.device.synchronizers[synchronizer_handle.0 as usize];

		unsafe {
			self.device
				.device
				.reset_fences(&[synchronizer.fence])
				.expect("Failed to reset Vulkan command buffer synchronizer. The most likely cause is that the fence is invalid or already in use.");
			let vk_queue = command_buffer
				.vk_queue
				.lock()
				.expect("Failed to lock Vulkan queue for command-buffer submission. The most likely cause is that another thread panicked while holding the queue lock.");
			self.device
				.device
				.queue_submit2(*vk_queue, &[submit_info], synchronizer.fence)
				.expect("Failed to submit Vulkan command buffer. The most likely cause is that the command buffer was not recorded for this queue.");
		}

		for (handle, state) in self.states {
			self.device.states.insert(handle, state);
		}
		for (handle, states) in self.buffer_states {
			self.device.buffer_states.insert(handle, states);
		}
	}
}

impl crate::command_buffer::CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundComputePipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		unsafe {
			self.device.device.cmd_bind_pipeline(
				command_buffer.command_buffer,
				vk::PipelineBindPoint::COMPUTE,
				pipeline.pipeline,
			);
		}

		self.pipeline_bind_point = vk::PipelineBindPoint::COMPUTE;
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(pipeline.layout);

		self
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundRayTracingPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		unsafe {
			self.device.device.cmd_bind_pipeline(
				command_buffer.command_buffer,
				vk::PipelineBindPoint::RAY_TRACING_KHR,
				pipeline.pipeline,
			);
		}

		self.pipeline_bind_point = vk::PipelineBindPoint::RAY_TRACING_KHR;
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(pipeline.layout);

		self
	}

	fn start_region(&self, write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		let command_buffer = self.get_command_buffer();
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");

		// Vulkan requires a null-terminated label that remains alive for the duration of the call.
		label.null_terminate();
		let name = std::ffi::CStr::from_bytes_with_nul(label.as_bytes())
			.expect("Invalid debug label. The label most likely contains an interior null byte.");
		let marker_info = vk::DebugUtilsLabelEXT::default().label_name(name);

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.device.debug_utils {
				debug_utils.cmd_begin_debug_utils_label(command_buffer.command_buffer, &marker_info);
			}
		}
	}

	fn region(
		&mut self,
		write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result,
		f: impl FnOnce(&mut Self),
	) {
		self.start_region(write_label);
		f(self);
		self.end_region();
	}

	fn end_region(&self) {
		let command_buffer = self.get_command_buffer();

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.device.debug_utils {
				debug_utils.cmd_end_debug_utils_label(command_buffer.command_buffer);
			}
		}
	}
}

impl crate::command_buffer::RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl crate::command_buffer::BoundRasterizationPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		unsafe {
			self.device.device.cmd_bind_pipeline(
				command_buffer.command_buffer,
				vk::PipelineBindPoint::GRAPHICS,
				pipeline.pipeline,
			);
		}

		self.pipeline_bind_point = vk::PipelineBindPoint::GRAPHICS;
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(pipeline.layout);

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		let consumptions = buffer_descriptors.iter().map(|buffer_descriptor| VulkanConsumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
			stages: vk::PipelineStageFlags2::VERTEX_INPUT,
			access: vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
			layout: vk::ImageLayout::UNDEFINED,
			range: None,
		});

		self.vulkan_consume_resources(consumptions)(self);

		let command_buffer = self.get_command_buffer();

		let buffers = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| {
				self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer))
					.buffer
			})
			.collect::<Vec<_>>();
		let offsets = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| buffer_descriptor.offset)
			.collect::<Vec<_>>();

		// TODO: implent slot splitting
		unsafe {
			self.device.device.cmd_bind_vertex_buffers(
				command_buffer.command_buffer,
				0,
				&buffers,
				&offsets.iter().map(|&e| e as _).collect::<Vec<_>>(),
			);
		}
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		self.vulkan_consume_resources([VulkanConsumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
			stages: vk::PipelineStageFlags2::INDEX_INPUT,
			access: vk::AccessFlags2::INDEX_READ,
			layout: vk::ImageLayout::UNDEFINED,
			range: None,
		}])(self);

		let command_buffer = self.get_command_buffer();

		let buffer = self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer));
		let index_type = match buffer_descriptor.index_type {
			Some(crate::DataTypes::U16) => vk::IndexType::UINT16,
			Some(crate::DataTypes::U32) => vk::IndexType::UINT32,
			Some(_) => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
			None => panic!(
				"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32)."
			),
		};

		unsafe {
			self.device.device.cmd_bind_index_buffer(
				command_buffer.command_buffer,
				buffer.buffer,
				buffer_descriptor.offset as _,
				index_type,
			);
		}
	}

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self) {
		let command_buffer = self.get_command_buffer();
		unsafe {
			self.device.device.cmd_end_rendering(command_buffer.command_buffer);
		}
		self.active_rendering = false;
	}
}

impl crate::command_buffer::BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let pipeline_layout_handle = self.bound_pipeline_layout.unwrap();
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = self.device.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout;

		let push_constant_stages =
			vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE;

		let push_constant_stages = push_constant_stages
			| if self.device.settings.mesh_shading {
				vk::ShaderStageFlags::MESH_EXT
			} else {
				vk::ShaderStageFlags::empty()
			};

		unsafe {
			self.device.device.cmd_push_constants(
				command_buffer.command_buffer,
				pipeline_layout,
				push_constant_stages,
				offset,
				std::slice::from_raw_parts(&data as *const T as *const u8, std::mem::size_of::<T>()),
			);
		}
	}

	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		if sets.is_empty() {
			return self;
		}

		let pipeline_layout_handle = self.bound_pipeline_layout.unwrap();

		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let s: SmallVec<[(u32, DescriptorSetHandle, vk::DescriptorSet); 16]> = sets
			.iter()
			.map(|descriptor_set_handle| {
				let internal_descriptor_set_handle = self.get_internal_descriptor_set_handle(*descriptor_set_handle);
				let descriptor_set = self.get_descriptor_set(&internal_descriptor_set_handle);
				let index_in_layout = pipeline_layout
					.descriptor_set_template_indices
					.get(&descriptor_set.descriptor_set_layout)
					.expect("Descriptor set layout not found in pipeline layout. You're likely trying to bind a descriptor set that is not compatible with the currently bound pipeline layout, which means you forgot to add this set to the layout or you bound the wrong layout");
				(
					*index_in_layout,
					internal_descriptor_set_handle,
					descriptor_set.descriptor_set,
				)
			})
			.collect();

		let vulkan_pipeline_layout_handle = pipeline_layout.pipeline_layout;

		for &(descriptor_set_index, descriptor_set_handle, _) in &s {
			if !self.bound_descriptor_sets_in_recording.contains(&descriptor_set_handle) {
				// Updating a descriptor set after it has been bound invalidates this command buffer unless
				// the set layout was created with UPDATE_AFTER_BIND. Keep this legacy refresh limited to
				// the first use of each set in this recording.
				self.refresh_image_descriptors_for_set(descriptor_set_handle);
				self.bound_descriptor_sets_in_recording.push(descriptor_set_handle);
			}

			if (descriptor_set_index as usize) < self.bound_descriptor_set_handles.len() {
				self.bound_descriptor_set_handles[descriptor_set_index as usize] =
					(descriptor_set_index, descriptor_set_handle);
				self.bound_descriptor_set_handles.truncate(descriptor_set_index as usize + 1);
			} else {
				assert_eq!(descriptor_set_index as usize, self.bound_descriptor_set_handles.len());
				self.bound_descriptor_set_handles
					.push((descriptor_set_index, descriptor_set_handle));
			}
		}

		let command_buffer = self.get_command_buffer();

		let partitions = partition(&self.bound_descriptor_set_handles, |e| e.0 as usize);

		// Always rebind all descriptor sets set by the user as previously bound descriptor sets might have been invalidated by a pipeline layout change.
		// Descriptor bindings are scoped to the active bind point. Binding compute-only descriptor sets to graphics leaves
		// storage/read image descriptors visible to later draws and makes Vulkan validate resources the graphics pipeline does
		// not actually use.
		for (base_index, descriptor_sets) in partitions {
			let base_index = base_index as u32;

			let descriptor_sets = descriptor_sets
				.iter()
				.map(|(_, descriptor_set)| self.get_descriptor_set(descriptor_set).descriptor_set)
				.collect::<Vec<_>>();

			unsafe {
				self.device.device.cmd_bind_descriptor_sets(
					command_buffer.command_buffer,
					self.pipeline_bind_point,
					vulkan_pipeline_layout_handle,
					base_index,
					&descriptor_sets,
					&[],
				);
			}
		}

		self
	}
}

impl crate::command_buffer::BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		// Raster pipelines can read descriptor-backed resources in vertex, mesh, and fragment stages.
		// Transition them before issuing the draw so transfer uploads are visible to shader reads.
		self.consume_resources_current([])(self);

		let command_buffer = self.get_command_buffer();

		let mesh = &self.device.meshes[mesh_handle.0 as usize];

		let buffers = [mesh.buffer];
		let offsets = [0];

		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device
				.device
				.cmd_bind_vertex_buffers(command_buffer_handle, 0, &buffers, &offsets);
		}
		unsafe {
			self.device.device.cmd_bind_index_buffer(
				command_buffer_handle,
				mesh.buffer,
				index_data_offset,
				vk::IndexType::UINT16,
			);
		}

		unsafe {
			self.device
				.device
				.cmd_draw_indexed(command_buffer_handle, mesh.index_count, 1, 0, 0, 0);
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		// Mesh shaders in the visibility pipeline read descriptor-backed storage buffers populated by
		// transfer uploads. Without this transition, Vulkan can execute the mesh read before those
		// transfer writes are available even though the descriptor set itself is correctly bound.
		self.consume_resources_current([])(self);

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.mesh_shading.cmd_draw_mesh_tasks(command_buffer_handle, x, y, z);
		}
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		// Draw calls use the currently bound pipeline descriptors just like compute dispatches do.
		self.consume_resources_current([])(self);

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.device.cmd_draw(
				command_buffer_handle,
				vertex_count,
				instance_count,
				first_vertex,
				first_instance,
			);
		}
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		// Draw calls use the currently bound pipeline descriptors just like compute dispatches do.
		self.consume_resources_current([])(self);

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.device.device.cmd_draw_indexed(
				command_buffer_handle,
				index_count,
				instance_count,
				first_index,
				vertex_offset,
				first_instance,
			);
		}
	}
}

impl crate::command_buffer::BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		let (x, y, z) = dispatch.get_extent().as_tuple();

		self.consume_resources_current([])(self);

		unsafe {
			self.device.device.cmd_dispatch(command_buffer_handle, x, y, z);
		}
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<[[u32; 4]; N]>,
		entry_index: usize,
	) {
		let buffer = self.get_buffer(self.get_internal_buffer_handle(buffer_handle.into())).buffer;

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		self.consume_resources_current([Consumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(buffer_handle.clone().into())),
			stages: crate::Stages::COMPUTE,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Indirect,
		}])(self);

		unsafe {
			self.device.device.cmd_dispatch_indirect(
				command_buffer_handle,
				buffer,
				entry_index as u64 * std::mem::size_of::<[u32; 4]>() as u64,
			);
		}
	}
}

impl crate::command_buffer::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: crate::rt::BindingTables, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();
		let comamand_buffer_handle = command_buffer.command_buffer;

		let make_strided_range = |range: crate::BufferStridedRange| -> vk::StridedDeviceAddressRegionKHR {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(
					self.device.get_buffer_address(range.buffer_offset.buffer) as vk::DeviceSize
						+ range.buffer_offset.offset as vk::DeviceSize,
				)
				.stride(range.stride as vk::DeviceSize)
				.size(range.size as vk::DeviceSize)
		};

		let raygen_shader_binding_tables = make_strided_range(binding_tables.raygen);
		let miss_shader_binding_tables = make_strided_range(binding_tables.miss);
		let hit_shader_binding_tables = make_strided_range(binding_tables.hit);
		let callable_shader_binding_tables = if let Some(binding_table) = binding_tables.callable {
			make_strided_range(binding_table)
		} else {
			vk::StridedDeviceAddressRegionKHR::default()
		};

		self.consume_resources_current([])(self);

		unsafe {
			self.device.ray_tracing_pipeline.cmd_trace_rays(
				comamand_buffer_handle,
				&raygen_shader_binding_tables,
				&miss_shader_binding_tables,
				&hit_shader_binding_tables,
				&callable_shader_binding_tables,
				x,
				y,
				z,
			)
		}
	}
}

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

fn buffer_row_length(format: crate::Formats, source_bytes_per_row: usize) -> u32 {
	match format {
		crate::Formats::BC5 | crate::Formats::BC7 | crate::Formats::BC7SRGB => ((source_bytes_per_row / 16) * 4) as u32,
		_ => (source_bytes_per_row / format.size()) as u32,
	}
}

fn buffer_image_height(format: crate::Formats, source_row_count: usize) -> u32 {
	match format {
		crate::Formats::BC5 | crate::Formats::BC7 | crate::Formats::BC7SRGB => (source_row_count * 4) as u32,
		_ => source_row_count as u32,
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlannedImageBarrier {
	old_layout: vk::ImageLayout,
	src_stage: vk::PipelineStageFlags2,
	src_access: vk::AccessFlags2,
	new_layout: vk::ImageLayout,
	dst_stage: vk::PipelineStageFlags2,
	dst_access: vk::AccessFlags2,
	image: vk::Image,
	aspect_mask: vk::ImageAspectFlags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlannedBufferBarrier {
	src_stage: vk::PipelineStageFlags2,
	src_access: vk::AccessFlags2,
	dst_stage: vk::PipelineStageFlags2,
	dst_access: vk::AccessFlags2,
	buffer: vk::Buffer,
	offset: vk::DeviceSize,
	size: vk::DeviceSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlannedMemoryBarrier {
	src_stage: vk::PipelineStageFlags2,
	src_access: vk::AccessFlags2,
	dst_stage: vk::PipelineStageFlags2,
	dst_access: vk::AccessFlags2,
}

#[derive(Default)]
struct PlannedTransitions {
	image_barriers: Vec<PlannedImageBarrier>,
	buffer_barriers: Vec<PlannedBufferBarrier>,
	memory_barriers: Vec<PlannedMemoryBarrier>,
	state_updates: SmallVec<[(Handles, TransitionState); 64]>,
	buffer_state_updates: SmallVec<[(Handles, Vec<BufferTransitionState>); 16]>,
}

impl PlannedTransitions {
	fn update_buffer_state(
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

#[cfg(test)]
mod tests {
	use ash::vk::Handle as _;

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
