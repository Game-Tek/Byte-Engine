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

		let copy = BufferCopy::new(staging_handle, 0, buffer_handle, 0, buffer.size);
		self.sync_buffers(std::iter::once(copy));
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
			sequence_index: frame_key.map_or(0, |frame_key| frame_key.sequence_index),
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
			active_render_extent: Extent::rectangle(0, 0),
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
		let begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

		unsafe {
			self.device
				.device
				.reset_command_pool(command_buffer.command_pool, vk::CommandPoolResetFlags::empty())
				.expect("No command pool reset");
			self.device
				.device
				.begin_command_buffer(command_buffer.command_buffer, &begin_info)
				.expect("No command buffer begin");
		}
	}

	pub(super) fn get_buffer(&self, buffer_handle: BufferHandle) -> &Buffer {
		self.device.buffers.resource(buffer_handle)
	}

	pub(super) fn get_image(&self, image_handle: ImageHandle) -> &Image {
		&self.device.images[image_handle.0 as usize]
	}

	pub(crate) fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.device.swapchains[swapchain_handle.0 as usize]
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
		unsafe {
			self.device
				.descriptor_heap
				.cmd_bind_resource_heap(command_buffer, &heaps.resource().bind_info());
			self.device
				.descriptor_heap
				.cmd_bind_sampler_heap(command_buffer, &heaps.sampler().bind_info());
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

		let snapshot = self.device.descriptor_materialization(materialization);
		let heap_offsets = [snapshot.resource_heap_offset, snapshot.sampler_heap_offset];
		let push_info = vk::PushDataInfoEXT::default()
			.offset(self.device.pipeline_layouts[layout_handle.0 as usize].heap_push_data_offset)
			.data(vk::HostAddressRangeConstEXT::default().address(::utils::as_byte_slice(&heap_offsets)));
		unsafe {
			self.device
				.descriptor_heap
				.cmd_push_data(self.get_command_buffer().command_buffer, &push_info);
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
					Descriptor::Image { image, layout, .. } | Descriptor::CombinedImageSampler { image, layout, .. } => {
						(Handles::Image(image), layout)
					}
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

	/// Gives an image-group member new contents. Returns `false` when `image` belongs to no group.
	///
	/// The member's next barrier starts from an undefined layout and waits for every earlier access to the members
	/// whose memory it reuses, since those accesses touched the same bytes.
	pub(super) fn initialize_group_member(&mut self, image: graphics_hardware_interface::BaseImageHandle) -> bool {
		let Some(overwritten) = self.device.image_groups.initialize(image) else {
			return false;
		};
		let handle = Handles::Image(self.get_internal_base_image_handle(image));
		let mut state = self.states.get(&handle).copied().unwrap_or(TransitionState::new(
			vk::PipelineStageFlags2::empty(),
			vk::AccessFlags2::empty(),
			vk::ImageLayout::UNDEFINED,
		));
		for other in overwritten {
			let other = Handles::Image(self.get_internal_base_image_handle(other));
			if let Some(other) = self.states.get(&other) {
				state.stage |= other.stage | other.last_write_stage;
				state.access |= other.access | other.last_write_access;
				state.last_write_stage |= other.last_write_stage;
				state.last_write_access |= other.last_write_access;
			}
		}
		state.layout = vk::ImageLayout::UNDEFINED;
		self.states.insert(handle, state);
		true
	}

	#[must_use]
	pub(super) fn consume_resources(&self, consumptions: impl IntoIterator<Item = Consumption>) -> TransitionStateUpdates {
		self.vulkan_consume_resources(consumptions.into_iter().map(|consumption| {
			let format = match consumption.handle {
				Handles::Image(image_handle) => {
					self.device.image_groups.assert_initialized(
						graphics_hardware_interface::BaseImageHandle(image_handle.0),
						|| {
							self.device.get_object_debug_name(
								graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle(
									image_handle.0,
								))
								.into(),
							)
						},
					);
					Some(self.get_image(image_handle).format_)
				}
				_ => None,
			};

			VulkanConsumption {
				handle: consumption.handle,
				stages: to_pipeline_stage_flags(consumption.stages, Some(consumption.layout), format),
				access: to_access_flags(consumption.access, consumption.stages, consumption.layout, format),
				layout: format.map_or(vk::ImageLayout::UNDEFINED, |format| {
					texture_format_and_resource_use_to_image_layout(format, consumption.layout, Some(consumption.access))
				}),
				range: None,
			}
		}))
	}

	/// Flags the passed resources as consumed and records the barriers they need.
	/// Consumptions are specified directly in Vulkan terms.
	#[must_use]
	pub(super) fn vulkan_consume_resources(
		&self,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> TransitionStateUpdates {
		let mut planned = Self::plan_vulkan_resource_transitions(
			&self.states,
			&self.buffer_states,
			consumptions,
			|handle| {
				let image = self.get_image(handle);
				Some((image.image, image.format))
			},
			|handle| Some(self.get_buffer(handle).buffer),
		);

		// Global barriers cover all memory, so one barrier with the union of masks orders everything the individual ones did.
		let folded_memory_barrier = planned.memory_barriers.iter().copied().reduce(|folded, barrier| {
			folded
				.src_stage_mask(folded.src_stage_mask | barrier.src_stage_mask)
				.src_access_mask(folded.src_access_mask | barrier.src_access_mask)
				.dst_stage_mask(folded.dst_stage_mask | barrier.dst_stage_mask)
				.dst_access_mask(folded.dst_access_mask | barrier.dst_access_mask)
		});
		let has_barriers =
			!planned.image_barriers.is_empty() || !planned.buffer_barriers.is_empty() || folded_memory_barrier.is_some();

		assert!(
			!self.active_rendering || !has_barriers,
			"Vulkan resource transition was requested inside active rendering. The most likely cause is that a resource changed after the first draw; end the render pass before recording work that needs a barrier.",
		);

		planned.updates.acquire_waits = self.chain_acquired_swapchain_images(&mut planned.image_barriers);

		// Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).
		if has_barriers {
			let dependency_info = vk::DependencyInfo::default()
				.image_memory_barriers(&planned.image_barriers)
				.buffer_memory_barriers(&planned.buffer_barriers)
				.memory_barriers(folded_memory_barrier.as_slice())
				.dependency_flags(vk::DependencyFlags::BY_REGION);
			unsafe {
				self.device
					.device
					.cmd_pipeline_barrier2(self.get_command_buffer().command_buffer, &dependency_info)
			};
		}

		planned.updates
	}

	/// Chains the first barrier on each freshly acquired swapchain image to the acquire semaphore wait.
	///
	/// The submission waits on the acquire semaphore at the returned first-use stages. A barrier's source scope must
	/// include the wait's stage for its layout transition to follow the presentation engine's release of the image.
	fn chain_acquired_swapchain_images(
		&self,
		image_barriers: &mut [vk::ImageMemoryBarrier2],
	) -> SmallVec<[(usize, vk::PipelineStageFlags2); 2]> {
		if self.frame_key.is_none() || image_barriers.is_empty() {
			return SmallVec::new();
		}

		let sequence_index = self.sequence_index as usize;
		self.device
			.swapchains
			.iter()
			.enumerate()
			.filter_map(|(swapchain_index, swapchain)| {
				let native_image = swapchain.native_images[swapchain.acquired_image_indices[sequence_index] as usize];
				let first_use_stage = Self::chain_barriers_to_acquire(image_barriers, self.get_image(native_image).image);
				(!first_use_stage.is_empty()).then_some((swapchain_index, first_use_stage))
			})
			.collect()
	}

	/// Sources each barrier on a freshly acquired image from its own destination stage and returns those stages.
	///
	/// Acquisition resets the image to an empty source state, so only its first barrier in a frame matches.
	pub(super) fn chain_barriers_to_acquire(
		image_barriers: &mut [vk::ImageMemoryBarrier2],
		acquired_image: vk::Image,
	) -> vk::PipelineStageFlags2 {
		let mut first_use_stage = vk::PipelineStageFlags2::NONE;
		for barrier in image_barriers
			.iter_mut()
			.filter(|barrier| barrier.image == acquired_image && barrier.src_stage_mask.is_empty())
		{
			barrier.src_stage_mask = barrier.dst_stage_mask;
			first_use_stage |= barrier.dst_stage_mask;
		}
		first_use_stage
	}

	/// Folds repeated whole-resource consumptions of one handle and layout into a single consumption.
	///
	/// Every consumption in a batch is planned against the state from before the batch. Planning a repeat separately,
	/// such as one per uploaded mip, would emit a second barrier whose old layout the first barrier already replaced.
	fn merge_repeated_consumptions(
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
	) -> SmallVec<[VulkanConsumption; 16]> {
		let mut merged = SmallVec::<[VulkanConsumption; 16]>::new();
		let mut indices = HashMap::<(Handles, vk::ImageLayout), usize>::default();

		for consumption in consumptions {
			if consumption.range.is_some() {
				merged.push(consumption);
				continue;
			}

			match indices.entry((consumption.handle, consumption.layout)) {
				std::collections::hash_map::Entry::Occupied(entry) => {
					let existing = &mut merged[*entry.get()];
					existing.stages |= consumption.stages;
					existing.access |= consumption.access;
				}
				std::collections::hash_map::Entry::Vacant(entry) => {
					entry.insert(merged.len());
					merged.push(consumption);
				}
			}
		}

		merged
	}

	pub(super) fn plan_vulkan_resource_transitions(
		states: &HashMap<Handles, TransitionState>,
		buffer_states: &HashMap<Handles, Vec<BufferTransitionState>>,
		consumptions: impl IntoIterator<Item = VulkanConsumption>,
		mut resolve_image: impl FnMut(ImageHandle) -> Option<(vk::Image, vk::Format)>,
		mut resolve_buffer: impl FnMut(BufferHandle) -> Option<vk::Buffer>,
	) -> PlannedTransitions {
		let mut planned = PlannedTransitions::default();

		for consumption in Self::merge_repeated_consumptions(consumptions) {
			let handle = consumption.handle;
			let source_state = states.get(&handle).copied();
			let mut transition_state = TransitionState::new(consumption.stages, consumption.access, consumption.layout);
			let mut recorded_state = transition_state;
			let mut read_after_read = false;

			if let Some(source_state) = source_state {
				transition_state = transition_state.inherit_last_write_from(source_state);
				recorded_state = transition_state;

				// Buffers have no layout, and their read-after-read coverage is decided per tracked range below.
				let is_buffer = matches!(handle, Handles::Buffer(_));
				read_after_read =
					source_state.reads_only(transition_state) && (is_buffer || source_state.layout == transition_state.layout);
				if read_after_read {
					recorded_state = source_state.merge_reads(transition_state);
					// Image layout transitions act as writes without write history, so only coverage can skip the barrier.
					if !is_buffer && source_state.covers(transition_state) {
						planned.updates.states.push((handle, recorded_state));
						continue;
					}
				}
			}

			let (src_stage, src_access, src_layout) = match source_state {
				// Earlier readers may not cover the new stages, so order against them and the last write.
				Some(source) if read_after_read => (
					source.stage | source.last_write_stage,
					source.access | source.last_write_access,
					source.layout,
				),
				Some(source) => (source.stage, source.access, source.layout),
				None => (
					vk::PipelineStageFlags2::empty(),
					vk::AccessFlags2::empty(),
					vk::ImageLayout::UNDEFINED,
				),
			};
			let (dst_stage, dst_access) = (transition_state.stage, transition_state.access);
			let buffer_barrier = move |src_stage, src_access, buffer, range: BufferRange| {
				vk::BufferMemoryBarrier2::default()
					.src_stage_mask(src_stage)
					.src_access_mask(src_access)
					.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.dst_stage_mask(dst_stage)
					.dst_access_mask(dst_access)
					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.buffer(buffer)
					.offset(range.offset)
					.size(range.size)
			};
			let range = consumption.range.unwrap_or(BufferRange::new(0, vk::WHOLE_SIZE));

			match handle {
				Handles::Image(image_handle) => {
					let Some((image, format)) = resolve_image(image_handle).filter(|(image, _)| !image.is_null()) else {
						continue;
					};

					planned.image_barriers.push(
						vk::ImageMemoryBarrier2::default()
							.old_layout(src_layout)
							.src_stage_mask(src_stage)
							.src_access_mask(src_access)
							.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
							.new_layout(transition_state.layout)
							.dst_stage_mask(dst_stage)
							.dst_access_mask(dst_access)
							.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
							.image(image)
							.subresource_range(
								vk::ImageSubresourceRange::default()
									.aspect_mask(image_aspect_mask(format))
									.level_count(vk::REMAINING_MIP_LEVELS)
									.layer_count(vk::REMAINING_ARRAY_LAYERS),
							),
					);
				}
				Handles::Buffer(buffer_handle) => {
					let Some(buffer) = resolve_buffer(buffer_handle).filter(|buffer| !buffer.is_null()) else {
						continue;
					};
					let overlapping_states = buffer_states
						.get(&handle)
						.into_iter()
						.flatten()
						.filter(|state| state.range.overlaps(range))
						.copied()
						.collect::<SmallVec<[_; 8]>>();

					// A read carries forward the pending writes of every range it overlaps. Without overlapping ranges it
					// keeps the last write it inherited from the whole-resource state.
					if !TransitionState::access_includes_write(transition_state.access) && !overlapping_states.is_empty() {
						transition_state.last_write_stage = vk::PipelineStageFlags2::empty();
						transition_state.last_write_access = vk::AccessFlags2::empty();
						for overlapping_state in &overlapping_states {
							transition_state.last_write_stage |= overlapping_state.state.last_write_stage;
							transition_state.last_write_access |= overlapping_state.state.last_write_access;
						}
					}

					for overlapping_state in &overlapping_states {
						let existing = overlapping_state.state;
						if existing.reads_only(transition_state)
							&& (existing.covers(transition_state) || !existing.has_write_history())
						{
							continue;
						}

						planned.buffer_barriers.push(buffer_barrier(
							existing.stage | existing.last_write_stage,
							existing.access | existing.last_write_access,
							buffer,
							overlapping_state.range.intersection(range),
						));
					}

					let handle_state_visible = source_state.is_some_and(|source_state| {
						read_after_read && (source_state.covers(transition_state) || !source_state.has_write_history())
					});
					if overlapping_states.is_empty() && consumption.range.is_none() && !handle_state_visible {
						planned
							.buffer_barriers
							.push(buffer_barrier(src_stage, src_access, buffer, range));
					}

					planned.update_buffer_state(handle, range, transition_state, buffer_states);
					if read_after_read {
						recorded_state.last_write_stage = transition_state.last_write_stage;
						recorded_state.last_write_access = transition_state.last_write_access;
					} else {
						recorded_state = transition_state;
					}
				}
				Handles::VkBuffer(buffer) => {
					planned
						.buffer_barriers
						.push(buffer_barrier(src_stage, src_access, buffer, range));
				}
				Handles::TopLevelAccelerationStructure(_) | Handles::BottomLevelAccelerationStructure(_) => {
					planned.memory_barriers.push(
						vk::MemoryBarrier2::default()
							.src_stage_mask(src_stage)
							.src_access_mask(src_access)
							.dst_stage_mask(dst_stage)
							.dst_access_mask(dst_access),
					);
				}
			}

			planned.updates.states.push((handle, recorded_state));
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

	pub(super) fn get_attachment_image_handle(
		&self,
		attachment: &graphics_hardware_interface::AttachmentInformation,
	) -> ImageHandle {
		match attachment.target {
			graphics_hardware_interface::ImageOrSwapchain::Image(handle) => self.get_internal_base_image_handle(handle),
			graphics_hardware_interface::ImageOrSwapchain::Swapchain(handle) => {
				let swapchain = self.get_swapchain(handle);
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

		let attachment_info = |attachment: &graphics_hardware_interface::AttachmentInformation| {
			vk::RenderingAttachmentInfo::default()
				.image_view(self.get_attachment_image_view(attachment))
				.image_layout(texture_format_and_resource_use_to_image_layout(
					self.get_attachment_format(attachment),
					attachment.layout,
					None,
				))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear_value()))
		};
		let render_area = vk::Rect2D::default().extent(vk::Extent2D {
			width: extent.width(),
			height: extent.height(),
		});
		let color_attachments = attachments
			.iter()
			.filter(|attachment| !self.get_attachment_format(attachment).is_depth())
			.map(|attachment| {
				let info = attachment_info(attachment);
				let image_extent = self.get_image(self.get_attachment_image_handle(attachment)).extent;
				if info.image_view.is_null() && image_extent.as_array() == [0; 3] {
					eprintln!("Creating a Vulkan render pass with an attachment that has no image view or extent. The image was most likely not resized before rendering.");
				}
				info
			})
			.collect::<Vec<_>>();
		let depth_attachment = attachments
			.iter()
			.find(|attachment| self.get_attachment_format(attachment).is_depth())
			.map(attachment_info)
			.unwrap_or_default();
		let rendering_info = vk::RenderingInfoKHR::default()
			.color_attachments(&color_attachments)
			.depth_attachment(&depth_attachment)
			.render_area(render_area)
			.layer_count(graphics_hardware_interface::AttachmentInformation::render_pass_layer_count(
				&attachments,
			));
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
		self.active_render_extent = extent;
	}

	pub(crate) fn get_presentable_swapchain_image_handle(
		&self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> ImageHandle {
		self.get_swapchain(present_key.swapchain).native_images[present_key.image_index as usize]
	}

	/// Performs a transfer-domain blit from the source image to the destination image, including the required layout
	/// transitions tracked through `self.states`.
	fn blit_image_to_image(&mut self, source_image_handle: ImageHandle, destination_image_handle: ImageHandle) {
		let source = self.get_image(source_image_handle);
		let (source_extent, source_vk_image) = (source.extent, source.image);
		let destination = self.get_image(destination_image_handle);
		let destination_vk_image = destination.image;
		let destination_extent = if destination.extent.as_array().contains(&0) {
			source_extent
		} else {
			destination.extent
		};

		if source_extent.width() == 0 || destination_extent.width() == 0 {
			return;
		}

		// Acquisition resets the native image to an undefined, empty state, so its barrier here is chained to the acquire wait.
		self.consume_resources([
			transfer_image_consumption(source_image_handle, crate::AccessPolicies::READ, crate::Layouts::Transfer),
			transfer_image_consumption(
				destination_image_handle,
				crate::AccessPolicies::WRITE,
				crate::Layouts::Transfer,
			),
		])
		.apply(self);

		let subresource = vk::ImageSubresourceLayers::default()
			.aspect_mask(vk::ImageAspectFlags::COLOR)
			.layer_count(1);
		let far_corner = |extent: Extent| vk::Offset3D {
			x: extent.width() as i32,
			y: extent.height().max(1) as i32,
			z: extent.depth().max(1) as i32,
		};
		let image_blits = [vk::ImageBlit2::default()
			.src_subresource(subresource)
			.src_offsets([vk::Offset3D::default(), far_corner(source_extent)])
			.dst_subresource(subresource)
			.dst_offsets([vk::Offset3D::default(), far_corner(destination_extent)])];
		let blit_image_info = vk::BlitImageInfo2::default()
			.src_image(source_vk_image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(destination_vk_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_blits);

		unsafe {
			self.device
				.device
				.cmd_blit_image2(self.get_command_buffer().command_buffer, &blit_image_info);
		}

		self.consume_resources([transfer_image_consumption(
			source_image_handle,
			crate::AccessPolicies::NONE,
			crate::Layouts::General,
		)])
		.apply(self);
	}

	pub fn handle_swapchain_proxies(&mut self, presentation_keys: &[graphics_hardware_interface::PresentKey]) {
		// When the swapchain uses proxies, resolve each user-facing proxy image into
		// the native presentable swapchain image before transitioning to present.
		for present_key in presentation_keys {
			let swapchain = self.get_swapchain(present_key.swapchain);
			let proxy_image = swapchain.images[present_key.image_index as usize];
			let native_image = swapchain.native_images[present_key.image_index as usize];

			if proxy_image != native_image {
				self.blit_image_to_image(proxy_image, native_image);
			}
		}

		let present_transitions = presentation_keys.iter().map(|present_key| Consumption {
			handle: Handles::Image(self.get_presentable_swapchain_image_handle(*present_key)),
			stages: crate::Stages::PRESENTATION,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Present,
		});

		self.consume_resources(present_transitions).apply(self);
	}

	/// Transitions all resources which were written to but not consumed by any later command.
	/// If this is skipped validation layers (correctly) complain about missing sync even though no "read" operation was performed.
	pub(crate) fn consume_last_resources(&mut self) {
		self.make_host_readable_writes_visible();

		let consumptions = self
			.states
			.iter()
			.filter(|(_, state)| state.access == vk::AccessFlags2::TRANSFER_WRITE)
			.map(|(handle, _)| Consumption {
				handle: *handle,
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::NONE,
				layout: crate::Layouts::General,
			});

		self.consume_resources(consumptions).apply(self);
	}

	/// Makes pending GPU writes to CPU-readable buffers visible to host reads through their mappings.
	///
	/// The fence the host waits on only makes device writes available; a barrier to the host stage makes them visible.
	fn make_host_readable_writes_visible(&self) {
		let (src_stage, src_access) = self
			.states
			.iter()
			.filter(|(handle, state)| {
				TransitionState::access_includes_write(state.access)
					&& matches!(handle, Handles::Buffer(buffer) if self.get_buffer(*buffer).access.contains(crate::DeviceAccesses::CpuRead))
			})
			.fold(
				(vk::PipelineStageFlags2::empty(), vk::AccessFlags2::empty()),
				|(stage, access), (_, state)| (stage | state.stage, access | state.access),
			);
		if src_access.is_empty() {
			return;
		}

		let barriers = [vk::MemoryBarrier2::default()
			.src_stage_mask(src_stage)
			.src_access_mask(src_access)
			.dst_stage_mask(vk::PipelineStageFlags2::HOST)
			.dst_access_mask(vk::AccessFlags2::HOST_READ)];
		unsafe {
			self.device.device.cmd_pipeline_barrier2(
				self.get_command_buffer().command_buffer,
				&vk::DependencyInfo::default().memory_barriers(&barriers),
			);
		}
	}

	pub fn end_recording(&self) {
		unsafe {
			self.device
				.device
				.end_command_buffer(self.get_command_buffer().command_buffer)
				.expect("Failed to end command buffer.");
		}
	}

	pub(crate) fn sync_buffers(&mut self, copy_buffers: impl Iterator<Item = BufferCopy> + Clone) {
		let consumption = |buffer, offset, size: usize, access| VulkanConsumption {
			handle: Handles::Buffer(buffer),
			stages: vk::PipelineStageFlags2::COPY,
			access,
			layout: vk::ImageLayout::UNDEFINED,
			range: Some(BufferRange::new(offset, size as vk::DeviceSize)),
		};
		let sources = copy_buffers
			.clone()
			.map(|copy| consumption(copy.src_buffer, copy.src_offset, copy.size, vk::AccessFlags2::TRANSFER_READ));
		let destinations = copy_buffers
			.clone()
			.map(|copy| consumption(copy.dst_buffer, copy.dst_offset, copy.size, vk::AccessFlags2::TRANSFER_WRITE));
		self.vulkan_consume_resources(sources.chain(destinations)).apply(self);

		let command_buffer = self.get_command_buffer().command_buffer;
		for copy in copy_buffers {
			let regions = [vk::BufferCopy2::default()
				.src_offset(copy.src_offset)
				.dst_offset(copy.dst_offset)
				.size(copy.size as u64)];
			let copy_buffer_info = vk::CopyBufferInfo2::default()
				.src_buffer(self.get_buffer(copy.src_buffer).buffer)
				.dst_buffer(self.get_buffer(copy.dst_buffer).buffer)
				.regions(&regions);

			unsafe { self.device.device.cmd_copy_buffer2(command_buffer, &copy_buffer_info) };
		}
	}

	/// Copies each image's staging buffer into the image and leaves it ready for fragment-shader reads.
	/// Uploads the whole staging buffer of each image.
	pub(crate) fn sync_textures(&mut self, copies: impl Iterator<Item = ImageCopy> + Clone) {
		self.vulkan_consume_resources(copies.clone().map(|copy| VulkanConsumption {
			handle: Handles::Image(copy.dst_texture),
			stages: vk::PipelineStageFlags2::TRANSFER,
			access: vk::AccessFlags2::TRANSFER_WRITE,
			layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			range: None,
		}))
		.apply(self);

		let command_buffer = self.get_command_buffer().command_buffer;
		for copy in copies.clone() {
			let image = self.get_image(copy.dst_texture);

			// The staging buffer holds tightly packed mip-0 payloads for every array layer, one after another.
			// A region upload reads its rectangle in place, so rows keep the full image's pitch.
			let origin = copy.region.map_or([0, 0], |region| region.offset);
			let extent = copy
				.region
				.map_or(image.extent, |region| Extent::rectangle(region.size[0], region.size[1]));
			let source_offset =
				(u64::from(origin[1]) * u64::from(image.extent.width()) + u64::from(origin[0])) * image.format_.size() as u64;
			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(source_offset)
				.buffer_row_length(if copy.region.is_some() { image.extent.width() } else { 0 })
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(image_aspect_mask(image.format))
						.layer_count(image.layers.map_or(1, std::num::NonZeroU32::get)),
				)
				.image_offset(vk::Offset3D::default().x(origin[0] as i32).y(origin[1] as i32))
				.image_extent(extent_into_vk_extent(extent))];
			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(image.staging_buffer.unwrap())
				.dst_image(image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.device
					.device
					.cmd_copy_buffer_to_image2(command_buffer, &buffer_image_copy);
			}
		}

		self.consume_resources(copies.map(|copy| Consumption {
			handle: Handles::Image(copy.dst_texture),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}))
		.apply(self);
	}
}

fn transfer_image_consumption(image: ImageHandle, access: crate::AccessPolicies, layout: crate::Layouts) -> Consumption {
	Consumption {
		handle: Handles::Image(image),
		stages: crate::Stages::TRANSFER,
		access,
		layout,
	}
}
