use super::*;

impl CommandBufferRecording<'_> {
	pub fn get_mut_buffer_slice<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	/// Records a staging-to-buffer upload on this command buffer.
	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into());
		let buffer = self.device.buffers.resource(buffer_handle);
		let Some(staging_handle) = buffer.staging else {
			return;
		};

		self.sync_buffers(std::iter::once(BufferCopy::new(
			staging_handle,
			0,
			buffer_handle,
			0,
			buffer.size,
		)));
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
			current_descriptor_materialization: None,
			descriptor_materialization_dirty: false,
			descriptor_resources_initialized: false,
			descriptor_heaps_bound: false,
			pending_rendering: None,
			active_rendering: false,
			texture_readbacks: SmallVec::new(),
			readbacks_finalized: false,

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
		SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
	) {
		self.handle_swapchain_proxies(presentation_keys);
		self.consume_last_resources();
		self.end_recording();
		self.readbacks_finalized = true;

		(
			self.command_buffer,
			std::mem::take(&mut self.states),
			std::mem::take(&mut self.buffer_states),
			std::mem::take(&mut self.texture_readbacks),
		)
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

	pub(super) fn get_buffer(&self, buffer_handle: BufferHandle) -> &Buffer {
		self.device.buffers.resource(buffer_handle)
	}

	pub(super) fn get_image(&self, image_handle: ImageHandle) -> &Image {
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

	pub(super) fn get_internal_top_level_acceleration_structure_handle(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
	) -> TopLevelAccelerationStructureHandle {
		TopLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	pub(super) fn get_top_level_acceleration_structure(
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

	pub(super) fn get_internal_bottom_level_acceleration_structure_handle(
		&self,
		acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) -> BottomLevelAccelerationStructureHandle {
		BottomLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	pub(super) fn get_bottom_level_acceleration_structure(
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

	/// Binds the context's long-lived heaps once for this command buffer.
	fn bind_descriptor_heaps_once(&mut self) {
		if self.descriptor_heaps_bound {
			return;
		}

		let command_buffer = self.get_command_buffer().command_buffer;
		let heaps = self.device.descriptor_heaps.as_ref().expect(
			"Missing Vulkan descriptor heaps. The most likely cause is that command recording started on an incompletely initialized context.",
		);
		let resource_bind_info = heaps.resource().bind_info();
		let sampler_bind_info = heaps.sampler().bind_info();
		unsafe {
			self.device
				.descriptor_heap
				.cmd_bind_resource_heap(command_buffer, &resource_bind_info);
			self.device
				.descriptor_heap
				.cmd_bind_sampler_heap(command_buffer, &sampler_bind_info);
		}
		self.descriptor_heaps_bound = true;
	}

	/// Materializes the retained flat-set union only after its pipeline layout or backing resources change.
	fn ensure_descriptor_materialization(&mut self) -> Option<DescriptorMaterializationHandle> {
		let layout_handle = self.bound_pipeline_layout.expect(
			"No Vulkan pipeline layout is active. The most likely cause is that a draw or dispatch was recorded before binding a pipeline.",
		);
		if self.device.pipeline_layouts[layout_handle.0 as usize].resources.is_empty() {
			self.current_descriptor_materialization = None;
			self.descriptor_materialization_dirty = false;
			return None;
		}
		if !self.descriptor_materialization_dirty {
			return self.current_descriptor_materialization;
		}

		self.frame_key.expect(
			"Vulkan descriptor heaps require a frame-owned command buffer. The most likely cause is that descriptor sets were bound on a context-level transfer recording that has no retirement fence.",
		);
		let materialization =
			self.device
				.materialize_descriptor_sets(layout_handle, &self.bound_descriptor_set_handles, self.sequence_index);
		self.bind_descriptor_heaps_once();

		let layout = &self.device.pipeline_layouts[layout_handle.0 as usize];
		let snapshot = self.device.descriptor_materialization(materialization);
		let heap_offsets = [snapshot.resource_heap_offset, snapshot.sampler_heap_offset];
		// SAFETY: heap_offsets is plain u32 data and remains alive for the duration of vkCmdPushDataEXT.
		let bytes =
			unsafe { std::slice::from_raw_parts(heap_offsets.as_ptr().cast::<u8>(), std::mem::size_of_val(&heap_offsets)) };
		let push_info = vk::PushDataInfoEXT::default()
			.offset(layout.heap_push_data_offset)
			.data(vk::HostAddressRangeConstEXT::default().address(bytes));
		let command_buffer = self.get_command_buffer().command_buffer;
		unsafe {
			self.device.descriptor_heap.cmd_push_data(command_buffer, &push_info);
		}

		self.current_descriptor_materialization = Some(materialization);
		self.descriptor_materialization_dirty = false;
		Some(materialization)
	}

	#[must_use]
	pub(super) fn consume_resources_current(
		&mut self,
		additional_transitions: impl IntoIterator<Item = Consumption>,
	) -> TransitionStateUpdates {
		let mut consumptions = SmallVec::<[Consumption; 128]>::new();
		let include_read_only = !self.descriptor_resources_initialized;
		if let Some(materialization) = self.ensure_descriptor_materialization() {
			for resource in &self.device.descriptor_materialization(materialization).resources {
				let writes = resource.access.intersects(crate::AccessPolicies::WRITE);

				assert!(
					!self.active_rendering || !writes,
					"Writable Vulkan descriptors cannot be reused by multiple draws in one render pass. The most likely cause is that a storage resource needs a barrier; split the draws into separate render passes.",
				);
				if !include_read_only && !writes {
					continue;
				}
				let (handle, layout) = match resource.descriptor {
					Descriptor::Buffer { buffer, .. } => (Handles::Buffer(buffer), crate::Layouts::General),
					Descriptor::Image { image, layout, .. } => (Handles::Image(image), layout),
					Descriptor::CombinedImageSampler { image, layout, .. } => (Handles::Image(image), layout),
					Descriptor::AccelerationStructure { handle } => {
						(Handles::TopLevelAccelerationStructure(handle), crate::Layouts::General)
					}
					Descriptor::Sampler { .. } => continue,
				};
				consumptions.push(Consumption {
					handle,
					stages: resource.stages,
					access: resource.access,
					layout,
				});
			}
		}
		self.descriptor_resources_initialized = true;
		consumptions.extend(additional_transitions);
		self.consume_resources(consumptions)
	}

	#[must_use]
	pub(super) fn consume_resources(&self, consumptions: impl IntoIterator<Item = Consumption>) -> TransitionStateUpdates {
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
	pub(super) fn vulkan_consume_resources(
		&self,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> TransitionStateUpdates {
		Self::vulkan_consume_resources_impl(self.device, self, &self.states, consumptions)
	}

	#[must_use]
	fn vulkan_consume_resources_impl(
		device: &Context,
		command_buffer: &CommandBufferRecording,
		states: &HashMap<Handles, TransitionState>,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> TransitionStateUpdates {
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
			assert!(
				planned.image_barriers.is_empty() && planned.buffer_barriers.is_empty() && planned.memory_barriers.is_empty(),
				"Vulkan resource transition was requested inside active rendering. The most likely cause is that a resource changed after the first draw; end the render pass before recording work that needs a barrier.",
			);

			return TransitionStateUpdates {
				states: planned.state_updates,
				buffer_states: planned.buffer_state_updates,
			};
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

		let updates = TransitionStateUpdates {
			states: planned.state_updates,
			buffer_states: planned.buffer_state_updates,
		};

		if image_memory_barriers.is_empty() && buffer_memory_barriers.is_empty() && memory_barriers.is_empty() {
			return updates;
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

		updates
	}

	pub(super) fn plan_vulkan_resource_transitions(
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

				let read_after_read = !TransitionState::access_includes_write(source_state.access)
					&& !TransitionState::access_includes_write(transition_state.access)
					&& source_state.layout == transition_state.layout;
				if read_after_read {
					transition_state.stage |= source_state.stage;
					transition_state.access |= source_state.access;
					if let Handles::Buffer(_) = consumption.handle {
						let range = consumption.range.unwrap_or(BufferRange::new(0, vk::WHOLE_SIZE));
						planned.update_buffer_state(consumption.handle, range, transition_state, buffer_states);
					}
					planned.state_updates.push((consumption.handle, transition_state));
					continue;
				}
			}

			let (src_stage, src_access, src_layout) = if let Some(source_state) = source_state {
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

	pub(super) fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	pub(super) fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> ImageHandle {
		if let Some(swapchain) = self
			.device
			.swapchains
			.iter()
			.find(|swapchain| swapchain.images[0].0 == handle.0.0 || swapchain.native_images[0].0 == handle.0.0)
		{
			return swapchain.images[swapchain.acquired_image_indices[self.sequence_index as usize] as usize];
		}

		let handles = ImageHandle(handle.0.0).get_all(&self.device.images);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	pub(super) fn get_internal_base_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.get_internal_image_handle(graphics_hardware_interface::ImageHandle(handle))
	}

	/// Resolves an image-or-swapchain source to the image selected for this recording.
	pub(super) fn get_image_or_swapchain_handle(&self, source: graphics_hardware_interface::ImageOrSwapchain) -> ImageHandle {
		match source {
			graphics_hardware_interface::ImageOrSwapchain::Image(handle) => self.get_internal_base_image_handle(handle),
			graphics_hardware_interface::ImageOrSwapchain::Swapchain(handle) => {
				let swapchain = &self.device.swapchains[handle.0 as usize];
				swapchain.images[swapchain.acquired_image_indices[self.sequence_index as usize] as usize]
			}
		}
	}

	pub(super) fn get_attachment_image_handle(
		&self,
		attachment: &graphics_hardware_interface::AttachmentInformation,
	) -> ImageHandle {
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

	/// Selects the native image view declared by one render-pass attachment.
	pub(super) fn get_attachment_image_view(
		&self,
		attachment: &graphics_hardware_interface::AttachmentInformation,
	) -> vk::ImageView {
		let image = self.get_image(self.get_attachment_image_handle(attachment));
		let image_layer_count = image.layers.map_or(1, |layer_count| layer_count.get());
		let requested_layer_count = attachment.layer_count.map_or(1, std::num::NonZeroU32::get);

		assert!(
			requested_layer_count <= image_layer_count,
			"Invalid Vulkan attachment layer count. The most likely cause is that the render pass requested more layers than the image provides."
		);
		assert!(
			attachment.layer.is_none_or(|layer| layer < image_layer_count),
			"Invalid Vulkan attachment layer. The most likely cause is that the render pass requested an array layer outside the image."
		);
		if attachment.layer_count.is_some() {
			assert!(
				attachment.layer.is_none(),
				"Invalid layered Vulkan attachment. The most likely cause is that the attachment selects both one layer and a layered range."
			);
			assert!(
				image.layers.is_some(),
				"Invalid layered Vulkan attachment image. The most likely cause is that layered rendering targeted a non-array image."
			);
			image.full_image_view
		} else {
			*image.image_views.get(attachment.layer.unwrap_or(0) as usize).expect(
				"Vulkan attachment layer is unavailable. The most likely cause is that the selected layer exceeds the image array size.",
			)
		}
	}

	/// Begins deferred dynamic rendering only after descriptor-backed resources have been transitioned.
	pub(super) fn begin_rendering_if_needed(&mut self) {
		if self.active_rendering {
			return;
		}
		let Some((extent, attachments)) = self.pending_rendering.take() else {
			return;
		};

		let render_area = vk::Rect2D::default()
			.offset(vk::Offset2D::default().x(0).y(0))
			.extent(vk::Extent2D::default().width(extent.width()).height(extent.height()));
		let color_attachments = attachments
			.iter()
			.filter(|attachment| !self.get_attachment_format(attachment).is_depth())
			.map(|attachment| {
				let image = self.get_image(self.get_attachment_image_handle(attachment));
				let format = self.get_attachment_format(attachment);
				let image_view = self.get_attachment_image_view(attachment);
				if image_view.is_null() && image.extent.width() == 0 && image.extent.height() == 0 && image.extent.depth() == 0 {
					eprintln!("Creating a Vulkan render pass with an attachment that has no image view or extent. The image was most likely not resized before rendering.");
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
			.find(|attachment| self.get_attachment_format(attachment).is_depth())
			.map(|attachment| {
				let format = self.get_attachment_format(attachment);
				vk::RenderingAttachmentInfo::default()
					.image_view(self.get_attachment_image_view(attachment))
					.image_layout(texture_format_and_resource_use_to_image_layout(
						format,
						attachment.layout,
						None,
					))
					.load_op(to_load_operation(attachment.load))
					.store_op(to_store_operation(attachment.store))
					.clear_value(to_clear_value(attachment.clear))
			})
			.unwrap_or_default();
		let layer_count = graphics_hardware_interface::AttachmentInformation::render_pass_layer_count(&attachments);
		let rendering_info = vk::RenderingInfoKHR::default()
			.color_attachments(&color_attachments)
			.depth_attachment(&depth_attachment)
			.render_area(render_area)
			.layer_count(layer_count);
		let viewports = [vk::Viewport {
			x: 0.0,
			y: extent.height() as f32,
			width: extent.width() as f32,
			height: -(extent.height() as f32),
			min_depth: 0.0,
			max_depth: 1.0,
		}];
		let command_buffer = self.get_command_buffer().command_buffer;
		unsafe {
			self.device.device.cmd_set_scissor(command_buffer, 0, &[render_area]);
			self.device.device.cmd_set_viewport(command_buffer, 0, &viewports);
			self.device.device.cmd_begin_rendering(command_buffer, &rendering_info);
		}
		self.active_rendering = true;
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
		])
		.apply(self);

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
		}])
		.apply(self);
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

		self.consume_resources(present_transitions).apply(self);
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

		self.consume_resources(consumptions).apply(self);
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
		self.vulkan_consume_resources(source_consumptions.chain(destination_consumptions))
			.apply(self);

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
		}))
		.apply(self);

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
		}))
		.apply(self);
	}
}
