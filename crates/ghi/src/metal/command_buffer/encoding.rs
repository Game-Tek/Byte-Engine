use super::recording::{descriptors_at_slot, retain_image};
use super::*;

/// Retains every materialized argument buffer and descriptor allocation through native command completion.
///
/// Takes split borrows so a cached materialization can stay inside the context while the command retains it. A slot
/// this command already retained at the same set version is skipped, so re-applying the same sets after a pipeline
/// switch does not walk large bindless arrays again.
fn retain_descriptor_resources(
	device: &RecordingDevice<'_>,
	descriptor_sets: &context::DescriptorSets,
	command_buffer: &mut queue::NativeCommand,
	bound_descriptor_sets: &[(DescriptorSetHandle, u64)],
	sequence_index: u8,
	layout: &PipelineLayout,
	materialization: &Materialization,
) {
	for (_, argument_buffer, _) in &materialization.argument_buffers {
		command_buffer.retain_allocation(&**argument_buffer);
	}
	for texture_view in &materialization._texture_views {
		command_buffer.retain_allocation(&**texture_view);
	}

	for resource in &layout.resources {
		let slot = resource.descriptor.slot();
		let Some((set_handle, descriptors)) = descriptors_at_slot(descriptor_sets, bound_descriptor_sets, slot) else {
			continue;
		};
		if !command_buffer.retain_descriptor_slot(set_handle.0, descriptor_sets.resource(set_handle).version, slot) {
			continue;
		}
		for descriptor in descriptors.values().copied() {
			match descriptor {
				Descriptor::Image { image, .. } => retain_image(device, command_buffer, image),
				Descriptor::CombinedImageSampler { image, sampler, .. } => {
					retain_image(device, command_buffer, image);
					command_buffer.retain_object(&*device.samplers[sampler.0 as usize].sampler);
				}
				Descriptor::Buffer { buffer, .. } => {
					command_buffer.retain_allocation(&*device.buffers.resource(buffer).buffer);
				}
				// Acquired drawables are retained when they are attached to the recording, so only proxies remain.
				Descriptor::Swapchain { handle } => {
					if let Some(proxy) = device.swapchains[handle.0 as usize].images[sequence_index as usize] {
						retain_image(device, command_buffer, proxy);
					}
				}
				Descriptor::AccelerationStructure { handle } => {
					command_buffer.retain_allocation(&*device.acceleration_structures[handle.0 as usize].structure);
				}
				Descriptor::Sampler { sampler } => {
					command_buffer.retain_object(&*device.samplers[sampler.0 as usize].sampler);
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
						let texture = self.swapchain_surface(handle).expect(MISSING_SURFACE).texture;
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
			self.bound_descriptor_sets.clear();

			for &descriptor_set_handle in sets {
				let resolved = self
					.commit
					.descriptor_sets
					.nth_handle(descriptor_set_handle, self.sequence_index as usize)
					.expect(
						"Missing frame-local Metal descriptor set. The most likely cause is that the set handle came from another context.",
					);
				// The version is read before each command, since writes can follow the bind.
				self.bound_descriptor_sets.push((resolved, 0));
			}
		}
	}

	/// Refreshes retained-set versions so writes made after a logical bind are visible before execution.
	fn refresh_bound_descriptor_set_versions(&mut self) {
		for (handle, version) in &mut self.bound_descriptor_sets {
			*version = self.commit.descriptor_sets.resource(*handle).version;
		}
	}

	/// Returns whether `key` names the pipeline and the bound sets at their current versions.
	fn binding_is_current(&self, key: &DescriptorBindingKey, pipeline: graphics_hardware_interface::PipelineHandle) -> bool {
		key.pipeline == pipeline && key.sets == self.bound_descriptor_sets
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
		let owner = self.bound_descriptor_sets.first().map(|(handle, _)| *handle);
		let existing = owner.and_then(|owner| {
			let snapshots = &self.commit.descriptor_sets.resource(owner).argument_buffers;
			// A snapshot for the same pipeline and sets is replaced in place when only the versions differ.
			let index = snapshots.iter().position(|snapshot| {
				snapshot.key.pipeline == pipeline_handle
					&& snapshot
						.key
						.sets
						.iter()
						.map(|(handle, _)| handle)
						.eq(self.bound_descriptor_sets.iter().map(|(handle, _)| handle))
			})?;
			Some((index, self.binding_is_current(&snapshots[index].key, pipeline_handle)))
		});

		// Either the owner set's retained snapshot, or a transient one encoded into the frame arena.
		let source: Result<(DescriptorSetHandle, usize), Materialization> = match (owner, existing) {
			(Some(owner), Some((index, true))) => Ok((owner, index)),
			(Some(owner), existing) if !self.bound_descriptors_reference_swapchain(layout) => {
				let snapshot = self.materialize_argument_buffers(pipeline_handle, false);
				let snapshots = &mut self.commit.descriptor_sets.resource_mut(owner).argument_buffers;
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
			bound_descriptor_sets,
			sequence_index,
			..
		} = self;
		let snapshot = match &source {
			Ok((owner, index)) => &commit.descriptor_sets.resource(*owner).argument_buffers[*index],
			Err(snapshot) => snapshot,
		};
		retain_descriptor_resources(
			device,
			commit.descriptor_sets,
			command_buffer,
			bound_descriptor_sets,
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
			key: snapshot.key.clone(),
			// A transient snapshot's buffers are retained by the command now, so only its uses need to outlive it.
			snapshot: match source {
				Ok((owner, index)) => AppliedSnapshot::Retained { owner, index },
				Err(snapshot) => AppliedSnapshot::Transient(snapshot.resource_uses),
			},
			settled: None,
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
					Descriptor::Swapchain { handle } => self
						.swapchain_surface(handle)
						.expect(MISSING_SURFACE)
						.resource_use(None, None, stages, access),
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
		// Validation scans every bound set against every resource, and transient snapshots materialize every frame,
		// so only debug builds pay for it.
		if cfg!(debug_assertions) {
			self.validate_bound_descriptor_sets(layout);
		}
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
			key: DescriptorBindingKey {
				pipeline: pipeline_handle,
				sets: self.bound_descriptor_sets.clone(),
			},
			argument_buffers,
			resource_uses: self.descriptor_resource_uses(layout),
			_texture_views: texture_views,
		}
	}

	/// Sets the bound pipeline's native state on the active encoder when the encoder does not hold it yet.
	fn apply_bound_pipeline(&mut self) {
		let Self {
			device,
			command_buffer,
			bound_pipeline,
			encoder,
			..
		} = self;
		let pipeline_handle = bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a draw or dispatch was recorded before binding a pipeline.",
		);
		let state = encoder.as_mut().expect(
			"No active Metal encoder. The most likely cause is that a draw or dispatch was recorded after its encoder ended.",
		);
		if state.pipeline == Some(pipeline_handle) {
			return;
		}

		match (&state.encoder, &device.pipelines[pipeline_handle.0 as usize].pipeline) {
			(
				ActiveEncoder::Compute(encoder),
				PipelineState::Compute {
					state: pipeline_state, ..
				},
			) => {
				command_buffer.retain_allocation(&**pipeline_state);
				encoder.setComputePipelineState(pipeline_state);
			}
			(ActiveEncoder::Render(encoder), PipelineState::Raster(raster)) => {
				command_buffer.retain_allocation(&*raster.state);
				encoder.setFrontFacingWinding(utils::winding(raster.face_winding));
				encoder.setCullMode(utils::cull_mode(raster.cull_mode));
				encoder.setTriangleFillMode(utils::fill_mode(raster.fill_mode));
				encoder.setDepthStencilState(raster.depth_stencil_state.as_deref());
				encoder.setRenderPipelineState(&raster.state);
			}
			(ActiveEncoder::Compute(_), PipelineState::Raster(_)) => panic!(
				"Cannot dispatch a raster Metal pipeline. The most likely cause is that a raster pipeline handle was passed to bind_compute_pipeline."
			),
			(ActiveEncoder::Render(_), PipelineState::Compute { .. }) => panic!(
				"Cannot draw with a non-raster Metal pipeline. The most likely cause is that a compute or ray tracing pipeline handle was passed to bind_raster_pipeline.",
			),
		}
		state.pipeline = Some(pipeline_handle);
	}

	/// Materializes and binds descriptors once per pipeline, set version, and native encoder.
	fn apply_bound_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a draw or dispatch was recorded before binding a pipeline.",
		);
		let state = self.encoder_state();
		if state
			.descriptors
			.as_ref()
			.is_some_and(|applied| self.binding_is_current(&applied.key, pipeline_handle))
		{
			return;
		}

		let compute = matches!(state.encoder, ActiveEncoder::Compute(_));
		let applied = self.apply_argument_buffers(pipeline_handle, |recording, stage, address| {
			if compute {
				// A ray-tracing pipeline runs only its ray-generation function on Metal, so the dispatch binds that
				// stage's argument buffer and leaves the hit and miss stages, which have no Metal function, unbound. A
				// compute pipeline's layouts only hold the compute stage, so one filter serves both kinds.
				if stage.intersects(crate::Stages::COMPUTE | crate::Stages::RAYGEN) {
					recording.set_stage_buffer_address(ArgumentTableStage::Compute, ARGUMENT_BUFFER_BINDING_BASE, address);
				}
				return;
			}
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
		self.encoder_state_mut().descriptors = Some(applied);
	}

	/// Restores encoder-local state and synchronizes only the resources the next draw or dispatch consumes.
	fn prepare_command(&mut self, additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.apply_bound_pipeline();
		self.apply_bound_descriptors();
		let mut binding = self.encoder_state_mut().descriptors.take().expect(
			"Metal descriptors are missing. The most likely cause is that descriptor application did not retain its materialization.",
		);
		self.consume_resources_with_descriptors(Some(&mut binding), additional_uses);
		self.encoder_state_mut().descriptors = Some(binding);
		self.flush_push_constants();
	}

	/// Prepares the next dispatch on the compute encoder, starting one when needed.
	pub(super) fn prepare_dispatch(&mut self, additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.ensure_compute_encoder();
		self.prepare_command(additional_uses);
	}

	/// Prepares the next draw in the open render pass; `operation` names the draw call if no pass is open.
	pub(super) fn prepare_draw(
		&mut self,
		operation: &str,
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		self.render_encoder(operation);
		self.prepare_command(additional_uses);
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
		self.begin_encoder(
			ActiveEncoder::Render(encoder),
			"Clear",
			images.iter().map(|(handle, _)| Some(*handle)),
		);
		self.consume_resources(images.iter().map(|(handle, _)| {
			synchronization::MetalResourceUse::image(
				*handle,
				Some(0),
				None,
				mtl::MTLStages::Fragment,
				crate::AccessPolicies::WRITE,
			)
		}));
		self.end_encoder();
	}
}
