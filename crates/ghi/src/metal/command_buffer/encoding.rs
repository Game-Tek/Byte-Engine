use super::*;

/// Retains every materialized argument buffer and descriptor allocation through native command completion.
///
/// Takes split borrows so a cached materialization can stay inside the context while the command retains it.
fn retain_descriptor_resources(
	device: &RecordingDevice<'_>,
	descriptor_sets: &[DescriptorSet],
	command_buffer: &mut NativeCommandSlot,
	bound_descriptor_set_handles: &[DescriptorSetHandle],
	sequence_index: u8,
	layout: &PipelineLayout,
	materialization: &Materialization,
) {
	for (_, argument_buffer, _) in materialization.argument_buffers.iter() {
		command_buffer.retain_allocation(argument_buffer.clone());
	}
	for texture_view in materialization._texture_views.iter() {
		command_buffer.retain_allocation(texture_view.clone());
	}

	for resource in &layout.resources {
		let Some(descriptors) = bound_descriptor_set_handles.iter().find_map(|set_handle| {
			descriptor_sets[set_handle.0 as usize]
				.descriptors
				.get(&resource.descriptor.slot())
		}) else {
			continue;
		};
		for descriptor in descriptors.values().copied() {
			match descriptor {
				Descriptor::Image { image, .. } => {
					command_buffer.retain_allocation(device.images.resource(image).texture.clone());
				}
				Descriptor::CombinedImageSampler { image, sampler, .. } => {
					command_buffer.retain_allocation(device.images.resource(image).texture.clone());
					command_buffer.retain_object(device.samplers[sampler.0 as usize].sampler.clone());
				}
				Descriptor::Buffer { buffer, .. } => {
					command_buffer.retain_allocation(device.buffers.resource(buffer).buffer.clone());
				}
				// Acquired drawables are retained when they are attached to the recording, so only proxies remain.
				Descriptor::Swapchain { handle } => {
					if let Some(proxy) = device.swapchains[handle.0 as usize].images[sequence_index as usize] {
						command_buffer.retain_allocation(device.images.resource(proxy).texture.clone());
					}
				}
				Descriptor::AccelerationStructure { handle } => {
					command_buffer.retain_allocation(device.acceleration_structures[handle.0 as usize].structure.clone());
				}
				Descriptor::Sampler { sampler } => {
					command_buffer.retain_object(device.samplers[sampler.0 as usize].sampler.clone());
				}
			}
		}
	}
}

impl CommandBufferRecording<'_> {
	/// Encodes one immutable argument buffer matching a shader stage's packed resource interface.
	///
	/// The caller provides `encoded_length` writable bytes at `offset` in `argument_buffer`.
	pub(super) fn encode_stage_argument_buffer(
		&self,
		layout: &StageArgumentLayout,
		argument_buffer: &ProtocolObject<dyn mtl::MTLBuffer>,
		offset: usize,
		texture_views: &mut SmallVec<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 4]>,
	) {
		// SAFETY: The caller's range starts at `offset` and exposes `encoded_length` writable bytes.
		unsafe {
			std::ptr::write_bytes(
				argument_buffer.contents().as_ptr().cast::<u8>().add(offset),
				0,
				layout.encoded_length,
			)
		};
		// SAFETY: The range was sized by this encoder's required length and starts at an aligned upload offset.
		unsafe {
			layout
				.argument_encoder
				.setArgumentBuffer_offset(Some(argument_buffer), offset)
		};

		for binding in &layout.bindings {
			let Some(descriptors) = self.descriptors_at_slot(binding.descriptor.slot()) else {
				continue;
			};

			for (&array_element, &descriptor) in descriptors {
				let argument_slot = binding.slot_for_array_element(array_element);
				match (argument_slot, descriptor) {
					// SAFETY: The materialized slot was produced by this argument encoder's reflection layout.
					(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
						let buffer = self.device.buffers.resource(buffer);
						layout
							.argument_encoder
							.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
					},
					// SAFETY: The materialized slot was produced by this argument encoder's reflection layout.
					(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, mip_level, .. }) => unsafe {
						let image = self.device.images.resource(image);
						let texture_view =
							mip_level.map(|mip_level| texture_view_2d(&image.texture, image.description.format, mip_level, 0));
						let texture = texture_view.as_ref().unwrap_or(&image.texture);
						layout.argument_encoder.setTexture_atIndex(Some(texture.as_ref()), slot as _);
						if let Some(texture_view) = texture_view {
							texture_views.push(texture_view);
						}
					},
					(DescriptorBindingSlot::Texture(slot), Descriptor::Swapchain { handle }) => {
						let texture = match self.swapchain_proxy(handle) {
							Some(proxy) => self.device.images.resource(proxy).texture.clone(),
							None => self.drawable_texture(handle),
						};
						// SAFETY: The materialized slot was produced by this argument encoder's reflection layout.
						unsafe { layout.argument_encoder.setTexture_atIndex(Some(&texture), slot as _) };
					}
					// SAFETY: The materialized slot was produced by this argument encoder's reflection layout.
					(DescriptorBindingSlot::Sampler(slot), Descriptor::Sampler { sampler }) => unsafe {
						let sampler = &self.device.samplers[sampler.0 as usize];
						layout
							.argument_encoder
							.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
					},
					(
						DescriptorBindingSlot::CombinedImageSampler { texture, sampler },
						Descriptor::CombinedImageSampler {
							image,
							sampler: sampler_handle,
							..
						},
					) => {
						let image = self.device.images.resource(image);
						let sampler_state = &self.device.samplers[sampler_handle.0 as usize];
						// SAFETY: The texture slot was produced by this argument encoder's reflection layout.
						unsafe {
							layout
								.argument_encoder
								.setTexture_atIndex(Some(image.texture.as_ref()), texture as _)
						};
						// SAFETY: The sampler slot was produced by this argument encoder's reflection layout.
						unsafe {
							layout
								.argument_encoder
								.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _)
						};
					}
					(DescriptorBindingSlot::AccelerationStructure(slot), Descriptor::AccelerationStructure { handle }) => {
						let structure = &self.device.acceleration_structures[handle.0 as usize].structure;
						// SAFETY: The materialized slot was produced by this argument encoder's reflection layout.
						unsafe {
							layout
								.argument_encoder
								.setAccelerationStructure_atIndex(Some(structure.as_ref()), slot as _);
						}
					}
					_ => unreachable!(
						"Validated Metal descriptor kind changed during materialization. The most likely cause is internal descriptor state corruption."
					),
				}
			}
		}
	}

	/// Resolves logical descriptor-set roots to the frame-local handles used by this recording.
	pub(super) fn update_bound_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) {
		if self.bound_descriptor_set_roots.as_slice() != sets {
			self.bound_descriptor_set_roots.clear();
			self.bound_descriptor_set_roots.extend_from_slice(sets);
			self.bound_descriptor_set_handles.clear();

			for descriptor_set_handle in sets {
				let mut resolved = DescriptorSetHandle(descriptor_set_handle.0);
				for _ in 0..self.sequence_index {
					resolved = self.commit.descriptor_sets[resolved.0 as usize].next.expect(
						"Missing frame-local Metal descriptor set. The most likely cause is that the retained set chain is shorter than the frame count.",
					);
				}
				self.bound_descriptor_set_handles.push(resolved);
			}
		}
	}

	/// Refreshes retained-set versions so writes made after a logical bind are visible before execution.
	pub(super) fn refresh_bound_descriptor_set_versions(&mut self) {
		self.bound_descriptor_set_versions.clear();
		self.bound_descriptor_set_versions.extend(
			self.bound_descriptor_set_handles
				.iter()
				.map(|handle| self.commit.descriptor_sets[handle.0 as usize].version),
		);
	}

	fn descriptor_binding_is_current(
		&self,
		applied: Option<&AppliedDescriptorBinding>,
		pipeline: graphics_hardware_interface::PipelineHandle,
	) -> bool {
		applied.is_some_and(|applied| {
			applied.pipeline == pipeline
				&& applied.descriptor_sets.as_slice() == self.bound_descriptor_set_handles.as_slice()
				&& applied.versions.as_slice() == self.bound_descriptor_set_versions.as_slice()
		})
	}

	/// Returns whether any bound descriptor references a swapchain, whose native texture can change per frame.
	fn bound_descriptors_reference_swapchain(&self, layout: &PipelineLayout) -> bool {
		layout.resources.iter().any(|resource| {
			self.descriptors_at_slot(resource.descriptor.slot())
				.is_some_and(|descriptors| {
					descriptors
						.values()
						.any(|descriptor| matches!(descriptor, Descriptor::Swapchain { .. }))
				})
		})
	}

	/// Applies the argument-buffer snapshot for the bound sets, encoding one only when no valid snapshot exists.
	///
	/// The first bound set retains the snapshot; it stays valid while every bound
	/// set keeps its version, so unchanged bindings cost one scan per encoder
	/// instead of a buffer allocation and encode. A snapshot that binds a
	/// swapchain is transient because the drawable changes per frame.
	fn apply_argument_buffers(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		mut bind: impl FnMut(&mut Self, crate::Stages, mtl::MTLGPUAddress),
	) -> AppliedDescriptorBinding {
		let layout = &self.device.pipelines[pipeline_handle.0 as usize].layout;
		let owner = self.bound_descriptor_set_handles.first().map(|handle| handle.0 as usize);
		let existing = owner.and_then(|owner| {
			let snapshots = &self.commit.descriptor_sets[owner].argument_buffers;
			let index = snapshots.iter().position(|snapshot| {
				snapshot.pipeline == pipeline_handle && snapshot.descriptor_sets == self.bound_descriptor_set_handles
			})?;
			Some((index, snapshots[index].versions == self.bound_descriptor_set_versions))
		});

		// Either the owner set's retained snapshot, or a transient one encoded into the frame arena.
		let source: Result<(usize, usize), Materialization> = match (owner, existing) {
			(Some(owner), Some((index, true))) => Ok((owner, index)),
			(Some(owner), existing) if !self.bound_descriptors_reference_swapchain(layout) => {
				let snapshot = self.materialize_argument_buffers(pipeline_handle, false);
				let snapshots = &mut self.commit.descriptor_sets[owner].argument_buffers;
				let index = match existing {
					Some((index, _)) => {
						snapshots[index] = snapshot;
						index
					}
					None => {
						snapshots.push(snapshot);
						snapshots.len() - 1
					}
				};
				Ok((owner, index))
			}
			_ => Err(self.materialize_argument_buffers(pipeline_handle, true)),
		};

		let Self {
			device,
			commit,
			command_buffer,
			bound_descriptor_set_handles,
			sequence_index,
			..
		} = self;
		let snapshot = match &source {
			Ok((owner, index)) => &commit.descriptor_sets[*owner].argument_buffers[*index],
			Err(snapshot) => snapshot,
		};
		retain_descriptor_resources(
			device,
			commit.descriptor_sets,
			command_buffer,
			bound_descriptor_set_handles,
			*sequence_index,
			layout,
			snapshot,
		);
		let addresses = snapshot
			.argument_buffers
			.iter()
			.map(|(stage, buffer, offset)| {
				let address = buffer
					.gpuAddress()
					.checked_add(*offset as u64)
					.expect("Metal argument buffer GPU address overflowed. The most likely cause is an invalid upload offset.");
				(*stage, address)
			})
			.collect::<SmallVec<[_; 5]>>();
		let applied = AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: bound_descriptor_set_handles.clone(),
			versions: snapshot.versions.clone(),
			resource_uses: snapshot.resource_uses.clone(),
		};
		for (stage, address) in addresses {
			bind(self, stage, address);
		}
		applied
	}

	/// Resolves the resource accesses represented by one immutable descriptor materialization.
	fn descriptor_resource_uses(&self, layout: &PipelineLayout) -> synchronization::DescriptorUses {
		let mut uses = SmallVec::new();
		for resource in &layout.resources {
			let Some(descriptors) = self.descriptors_at_slot(resource.descriptor.slot()) else {
				continue;
			};
			let stages = synchronization::to_metal_stages(resource.stages);
			let access = resource.descriptor.access();
			for descriptor in descriptors.values().copied() {
				let resource_use = match descriptor {
					Descriptor::Buffer { buffer, size } => {
						let buffer_size = self.device.buffers.resource(buffer).size;
						let size = match size {
							crate::Ranges::Size(size) => size.min(buffer_size),
							crate::Ranges::Whole => buffer_size,
						};
						synchronization::MetalResourceUse::buffer(buffer, 0, size, stages, access)
					}
					Descriptor::Image { image, mip_level, .. } => {
						synchronization::MetalResourceUse::image(image, mip_level, None, stages, access)
					}
					Descriptor::CombinedImageSampler { image, .. } => {
						synchronization::MetalResourceUse::image(image, None, None, stages, access)
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy) = self.swapchain_proxy(handle) {
							synchronization::MetalResourceUse::image(proxy, None, None, stages, access)
						} else {
							let drawable = self.drawable_texture(handle);
							synchronization::MetalResourceUse::drawable(drawable.as_ref(), stages, access)
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						synchronization::MetalResourceUse::acceleration_structure(handle.0 as usize, stages, access)
					}
					Descriptor::Sampler { .. } => continue,
				};
				uses.push(resource_use.in_group_memory(self.device.images));
			}
		}
		synchronization::DescriptorUses::new(uses)
	}

	/// Encodes the argument-buffer snapshot for the bound sets.
	///
	/// A retained snapshot gets its own buffers because it outlives the frame. A
	/// transient snapshot borrows ranges from the frame's upload arena, which is
	/// rewound once the frame's commands complete, so no buffer is created for it.
	pub(super) fn materialize_argument_buffers(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		transient: bool,
	) -> Materialization {
		let layout = &self.device.pipelines[pipeline_handle.0 as usize].layout;
		self.validate_bound_descriptor_sets(layout);
		let mut argument_buffers = layout
			.stage_argument_layouts
			.iter()
			.map(|stage_layout| {
				let (buffer, offset) = if transient {
					debug_assert!(
						stage_layout.argument_encoder.alignment() <= UPLOAD_ALIGNMENT,
						"Metal argument encoder alignment exceeds the upload arena alignment. The most likely cause is a device requiring more than 256-byte argument buffer alignment.",
					);
					let (buffer, offset) = self
						.commit
						.upload_arena
						.allocate(self.device.metal_device, stage_layout.encoded_length.max(1));
					(buffer.clone(), offset)
				} else {
					let buffer = self
						.device
						.metal_device
						.newBufferWithLength_options(
							stage_layout.encoded_length as _,
							mtl::MTLResourceOptions::StorageModeShared,
						)
						.expect(
							"Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.",
						);
					#[cfg(debug_assertions)]
					if self.device.debug_labels {
						buffer.setLabel(Some(&NSString::from_str("Argument Buffer")));
					}
					(buffer, 0)
				};
				(stage_layout.stage, buffer, offset)
			})
			.collect::<SmallVec<[_; 5]>>();
		let mut texture_views = SmallVec::new();
		for (stage_layout, (_, buffer, offset)) in layout.stage_argument_layouts.iter().zip(argument_buffers.iter_mut()) {
			self.encode_stage_argument_buffer(stage_layout, buffer, *offset, &mut texture_views);
		}
		Materialization {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
			argument_buffers,
			resource_uses: self.descriptor_resource_uses(layout),
			_texture_views: texture_views,
		}
	}

	/// Applies the logical compute pipeline to the current native encoder when required.
	pub(super) fn apply_bound_compute_pipeline(&mut self) {
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a compute dispatch was recorded before bind_compute_pipeline.",
		);
		if self.encoded_compute_pipeline == Some(pipeline_handle) {
			return;
		}

		let compute_pipeline_state = match &self.device.pipelines[pipeline_handle.0 as usize].pipeline {
			PipelineState::Compute(compute_pipeline_state) | PipelineState::RayTracing(compute_pipeline_state) => {
				compute_pipeline_state.clone()
			}
			PipelineState::Raster(_) => panic!(
				"Cannot dispatch a raster Metal pipeline. The most likely cause is that a raster pipeline handle was passed to bind_compute_pipeline."
			),
		};
		self.command_buffer.retain_allocation(compute_pipeline_state.clone());
		self.ensure_compute_encoder()
			.setComputePipelineState(compute_pipeline_state.as_ref());
		self.encoded_compute_pipeline = Some(pipeline_handle);
	}

	/// Applies the logical render pipeline to the active render pass when required.
	pub(super) fn apply_bound_render_pipeline(&mut self) {
		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that a draw was recorded before bind_raster_pipeline.");
		if self.encoded_render_pipeline == Some(pipeline_handle) {
			return;
		}

		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let render_pipeline_state = match &pipeline.pipeline {
			PipelineState::Raster(render_pipeline_state) => render_pipeline_state.clone(),
			_ => panic!(
				"Cannot draw with a non-raster Metal pipeline. The most likely cause is that a compute or ray tracing pipeline handle was passed to bind_raster_pipeline.",
			),
		};
		let depth_stencil_state = pipeline.depth_stencil_state.clone();
		let face_winding = pipeline.face_winding;
		let cull_mode = pipeline.cull_mode;
		let fill_mode = pipeline.fill_mode;
		self.command_buffer.retain_allocation(render_pipeline_state.clone());
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that a draw was recorded outside start_render_pass.");

		encoder.setFrontFacingWinding(utils::winding(face_winding));
		encoder.setCullMode(utils::cull_mode(cull_mode));
		encoder.setTriangleFillMode(utils::fill_mode(fill_mode));
		encoder.setDepthStencilState(depth_stencil_state.as_ref().map(|state| state.as_ref()));
		encoder.setRenderPipelineState(render_pipeline_state.as_ref());

		self.encoded_render_pipeline = Some(pipeline_handle);
	}

	/// Materializes and binds compute descriptors once per pipeline, set version, and native encoder.
	pub(super) fn apply_bound_compute_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a compute dispatch was recorded before bind_compute_pipeline.",
		);
		if self.descriptor_binding_is_current(self.applied_compute_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		// A ray-tracing pipeline runs only its ray-generation function on Metal, so the dispatch binds that stage's
		// argument buffer and leaves the hit and miss stages, which have no Metal function, unbound.
		let dispatched_stage = match &self.device.pipelines[pipeline_handle.0 as usize].pipeline {
			PipelineState::RayTracing(_) => crate::Stages::RAYGEN,
			_ => crate::Stages::COMPUTE,
		};
		let applied = self.apply_argument_buffers(pipeline_handle, |recording, stage, address| {
			if stage.intersects(dispatched_stage) {
				recording.set_stage_buffer_address(ArgumentTableStage::Compute, ARGUMENT_BUFFER_BINDING_BASE, address);
			}
		});
		self.applied_compute_descriptor_binding = Some(applied);
	}

	/// Materializes and binds render descriptors once per pipeline, set version, and native encoder.
	pub(super) fn apply_bound_render_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that a draw was recorded before bind_raster_pipeline.");
		if self.descriptor_binding_is_current(self.applied_render_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		let applied = self.apply_argument_buffers(pipeline_handle, |recording, stage, address| {
			for (stages, table_stage) in [
				(crate::Stages::TASK, ArgumentTableStage::Object),
				(crate::Stages::MESH, ArgumentTableStage::Mesh),
				(crate::Stages::VERTEX, ArgumentTableStage::Vertex),
				(crate::Stages::FRAGMENT, ArgumentTableStage::Fragment),
			] {
				if stage.intersects(stages) {
					recording.set_stage_buffer_address(table_stage, ARGUMENT_BUFFER_BINDING_BASE, address);
				}
			}
		});
		self.applied_render_descriptor_binding = Some(applied);
	}

	/// Restores encoder-local compute state and synchronizes only resources consumed by the next dispatch.
	pub(super) fn prepare_compute_dispatch(
		&mut self,
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		self.ensure_compute_encoder();
		self.apply_bound_compute_pipeline();
		self.apply_bound_compute_descriptors();
		let mut binding = self.applied_compute_descriptor_binding.take().expect(
			"Metal compute descriptors are missing. The most likely cause is that descriptor application did not retain its materialization.",
		);
		self.consume_resources_with_descriptors(&mut binding.resource_uses, additional_uses);
		self.applied_compute_descriptor_binding = Some(binding);
	}

	/// Restores encoder-local render state and synchronizes only resources consumed by the next draw.
	pub(super) fn prepare_render_draw(&mut self, additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.apply_bound_render_pipeline();
		self.apply_bound_render_descriptors();
		let mut binding = self.applied_render_descriptor_binding.take().expect(
			"Metal render descriptors are missing. The most likely cause is that descriptor application did not retain its materialization.",
		);
		self.consume_resources_with_descriptors(&mut binding.resource_uses, additional_uses);
		self.applied_render_descriptor_binding = Some(binding);
	}

	/// Encodes one render-pass clear for a compatible group of color and depth images.
	pub(super) fn encode_image_clear_batch(&mut self, images: &[(ImageHandle, graphics_hardware_interface::ClearValue)]) {
		let Some((first_handle, _)) = images.first() else {
			return;
		};
		let first_image = self.device.images.resource(*first_handle);
		let rpd = mtl::MTL4RenderPassDescriptor::new();
		if first_image.description.array_layers > 1 {
			rpd.setRenderTargetArrayLength(first_image.description.array_layers as _);
		}

		let mut color_index = 0;
		for (handle, clear_value) in images {
			let image = self.device.images.resource(*handle);
			self.command_buffer.retain_allocation(image.texture.clone());
			if image.description.format.is_depth() {
				let attachment = rpd.depthAttachment();
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearDepth(utils::clear_depth(*clear_value));
			} else {
				// SAFETY: `color_index` counts only non-depth attachments and stays within the render-pass descriptor array.
				let attachment = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearColor(utils::clear_color(*clear_value));
				color_index += 1;
			}
		}

		let encoder = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
			"Metal render command encoder creation failed. The most likely cause is that the command buffer could not start an image clear pass.",
		);
		#[cfg(debug_assertions)]
		{
			self.render_debug_region_depth =
				self.begin_encoder_debug_regions(&*encoder, "Clear", images.iter().map(|(handle, _)| Some(*handle)));
		}
		self.active_encoder_scope = Some(self.allocate_encoder_scope());
		self.active_render_encoder = Some(encoder);
		self.consume_resources(images.iter().map(|(handle, _)| {
			synchronization::MetalResourceUse::image(
				*handle,
				Some(0),
				None,
				mtl::MTLStages::Fragment,
				crate::AccessPolicies::WRITE,
			)
		}));
		self.end_render_encoder();
	}
}
