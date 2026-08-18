use super::*;

impl CommandBufferRecording<'_> {
	/// Retains every materialized argument buffer and descriptor allocation through native command completion.
	pub(super) fn retain_descriptor_resources(&self, layout: &PipelineLayout, materialization: &Materialization) {
		for (_, argument_buffer) in materialization.argument_buffers.iter() {
			self.command_buffer.retain_buffer(argument_buffer.clone());
		}
		for texture_view in materialization._texture_views.iter() {
			self.command_buffer.retain_texture(texture_view.clone());
		}

		for resource in &layout.resources {
			let Some(descriptors) = self.descriptors_at_slot(resource.descriptor.slot()) else {
				continue;
			};
			for descriptor in descriptors.values().copied() {
				match descriptor {
					Descriptor::Image { image, .. } => {
						self.command_buffer
							.retain_texture(self.device.images.resource(image).texture.clone());
					}
					Descriptor::CombinedImageSampler { image, sampler, .. } => {
						self.command_buffer
							.retain_texture(self.device.images.resource(image).texture.clone());
						self.command_buffer
							.retain_sampler(self.device.samplers[sampler.0 as usize].sampler.clone());
					}
					Descriptor::Buffer { buffer, .. } => {
						self.command_buffer
							.retain_buffer(self.device.buffers.resource(buffer).buffer.clone());
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy) = self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize] {
							self.command_buffer
								.retain_texture(self.device.images.resource(proxy).texture.clone());
						} else {
							self.command_buffer.retain_texture(self.drawable_texture(handle));
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						if let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() {
							let allocation = unsafe {
								Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(structure.clone())
							};
							self.command_buffer.retain_allocations(std::iter::once(allocation));
						}
					}
					Descriptor::Sampler { sampler } => {
						self.command_buffer
							.retain_sampler(self.device.samplers[sampler.0 as usize].sampler.clone());
					}
				}
			}
		}
	}

	/// Encodes one immutable argument buffer matching a shader stage's packed resource interface.
	pub(super) fn encode_stage_argument_buffer(
		&self,
		layout: &StageArgumentLayout,
		texture_views: &mut SmallVec<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 4]>,
	) -> Retained<ProtocolObject<dyn mtl::MTLBuffer>> {
		let argument_buffer = self
			.device
			.metal_device
			.newBufferWithLength_options(layout.encoded_length as _, mtl::MTLResourceOptions::StorageModeShared)
			.expect("Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.");
		unsafe {
			// Metal does not guarantee fresh buffer contents are zeroed. Null all unwritten array elements deterministically.
			std::ptr::write_bytes(argument_buffer.contents().as_ptr() as *mut u8, 0, layout.encoded_length);
			layout
				.argument_encoder
				.setArgumentBuffer_offset(Some(argument_buffer.as_ref()), 0);
		}

		for binding in &layout.bindings {
			let Some(descriptors) = self.descriptors_at_slot(binding.descriptor.slot()) else {
				continue;
			};

			for (&array_element, &descriptor) in descriptors {
				let argument_slot = binding.slot_for_array_element(array_element);
				match (argument_slot, descriptor) {
					(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
						let buffer = self.device.buffers.resource(buffer);
						layout
							.argument_encoder
							.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
					},
					(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, mip_level, .. }) => unsafe {
						let image = self.device.images.resource(image);
						let texture_view = descriptor_texture_view(&image.texture, image.format, mip_level);
						let texture = texture_view.as_ref().unwrap_or(&image.texture);
						layout.argument_encoder.setTexture_atIndex(Some(texture.as_ref()), slot as _);
						if let Some(texture_view) = texture_view {
							texture_views.push(texture_view);
						}
					},
					(DescriptorBindingSlot::Texture(slot), Descriptor::Swapchain { handle }) => unsafe {
						if let Some(proxy) = self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize] {
							let image = self.device.images.resource(proxy);
							layout
								.argument_encoder
								.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
						} else {
							let texture = self.drawable_texture(handle);
							layout.argument_encoder.setTexture_atIndex(Some(texture.as_ref()), slot as _);
						}
					},
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
					) => unsafe {
						let image = self.device.images.resource(image);
						let sampler_state = &self.device.samplers[sampler_handle.0 as usize];
						layout
							.argument_encoder
							.setTexture_atIndex(Some(image.texture.as_ref()), texture as _);
						layout
							.argument_encoder
							.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _);
					},
					(DescriptorBindingSlot::AccelerationStructure(slot), Descriptor::AccelerationStructure { handle }) => {
						if let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() {
							unsafe {
								layout
									.argument_encoder
									.setAccelerationStructure_atIndex(Some(structure.as_ref()), slot as _);
							}
						}
					}
					_ => unreachable!(
						"Validated Metal descriptor kind changed during materialization. The most likely cause is internal descriptor state corruption."
					),
				}
			}
		}

		argument_buffer
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
					resolved = self.device.descriptor_sets[resolved.0 as usize].next.expect(
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
				.map(|handle| self.device.descriptor_sets[handle.0 as usize].version),
		);
	}

	pub(super) fn descriptor_binding_matches(
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

	/// Returns immutable native argument-buffer snapshots, reusing them while every retained set version is unchanged.
	pub(super) fn materialize_argument_buffers(
		&self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> Materialization {
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let key = MaterializationKey {
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			sequence_index: self.sequence_index,
		};
		let versions = self.bound_descriptor_set_versions.clone();

		if let Some(materialization) = pipeline.materializations.borrow().get(&key) {
			if materialization.versions == versions {
				return materialization.clone();
			}
		}

		let layout = &self.device.pipeline_layouts[pipeline.layout.0 as usize];
		self.validate_bound_descriptor_sets(layout);
		let mut texture_views = SmallVec::new();
		let argument_buffers = Rc::new(
			layout
				.stage_argument_layouts
				.iter()
				.map(|stage_layout| {
					(
						stage_layout.stage,
						self.encode_stage_argument_buffer(stage_layout, &mut texture_views),
					)
				})
				.collect::<SmallVec<[_; 5]>>(),
		);
		let materialization = Materialization {
			versions,
			argument_buffers,
			_texture_views: Rc::new(texture_views),
		};
		pipeline.materializations.borrow_mut().insert(key, materialization.clone());
		materialization
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
			PipelineState::Compute(Some(compute_pipeline_state)) => compute_pipeline_state.clone(),
			PipelineState::Compute(None) => {
				panic!(
					"Metal compute pipeline has no MTLComputePipelineState. The most likely cause is shader creation failed."
				)
			}
			_ => panic!(
				"Cannot dispatch a non-compute Metal pipeline. The most likely cause is that a raster or ray tracing pipeline handle was passed to bind_compute_pipeline."
			),
		};
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
		let pipeline_state = pipeline.pipeline.clone();
		let depth_stencil_state = pipeline.depth_stencil_state.clone();
		let face_winding = pipeline.face_winding;
		let cull_mode = pipeline.cull_mode;
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that a draw was recorded outside start_render_pass.");

		encoder.setFrontFacingWinding(utils::winding(face_winding));
		encoder.setCullMode(utils::cull_mode(cull_mode));
		encoder.setDepthStencilState(depth_stencil_state.as_ref().map(|state| state.as_ref()));

		match &pipeline_state {
			PipelineState::Raster(Some(render_pipeline_state)) => {
				encoder.setRenderPipelineState(render_pipeline_state);
			}
			PipelineState::Raster(None) => panic!(
				"Metal raster pipeline has no MTLRenderPipelineState. The most likely cause is shader creation failed or SPIR-V was supplied to the Metal backend without translation to MSL or MTLB.",
			),
			_ => panic!(
				"Cannot draw with a non-raster Metal pipeline. The most likely cause is that a compute or ray tracing pipeline handle was passed to bind_raster_pipeline.",
			),
		}

		self.encoded_render_pipeline = Some(pipeline_handle);
	}

	/// Materializes and binds compute descriptors once per pipeline, set version, and native encoder.
	pub(super) fn apply_bound_compute_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a compute dispatch was recorded before bind_compute_pipeline.",
		);
		if self.descriptor_binding_matches(self.applied_compute_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		let pipeline_layout_handle = self.device.pipelines[pipeline_handle.0 as usize].layout;
		let materialization = self.materialize_argument_buffers(pipeline_handle);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			if stage.intersects(crate::Stages::COMPUTE) {
				self.set_stage_buffer_address(
					ArgumentTableStage::Compute,
					ARGUMENT_BUFFER_BINDING_BASE,
					argument_buffer.gpuAddress(),
				);
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.retain_descriptor_resources(pipeline_layout, &materialization);
		self.applied_compute_descriptor_binding = Some(AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
		});
	}

	/// Materializes and binds render descriptors once per pipeline, set version, and native encoder.
	pub(super) fn apply_bound_render_descriptors(&mut self) {
		self.refresh_bound_descriptor_set_versions();
		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that a draw was recorded before bind_raster_pipeline.");
		if self.descriptor_binding_matches(self.applied_render_descriptor_binding.as_ref(), pipeline_handle) {
			return;
		}

		let pipeline_layout_handle = self.device.pipelines[pipeline_handle.0 as usize].layout;
		let materialization = self.materialize_argument_buffers(pipeline_handle);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			let address = argument_buffer.gpuAddress();
			for (stages, table_stage) in [
				(crate::Stages::TASK, ArgumentTableStage::Object),
				(crate::Stages::MESH, ArgumentTableStage::Mesh),
				(crate::Stages::VERTEX, ArgumentTableStage::Vertex),
				(crate::Stages::FRAGMENT, ArgumentTableStage::Fragment),
			] {
				if stage.intersects(stages) {
					self.set_stage_buffer_address(table_stage, ARGUMENT_BUFFER_BINDING_BASE, address);
				}
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.retain_descriptor_resources(pipeline_layout, &materialization);
		self.applied_render_descriptor_binding = Some(AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
		});
	}

	/// Restores encoder-local compute state and makes prior copy writes device-visible before a dispatch.
	pub(super) fn prepare_compute_dispatch(&mut self) {
		let previous_phase = self.compute_encoder_phase;
		let encoder = self.ensure_compute_encoder().clone();
		match previous_phase {
			ComputeEncoderPhase::Transfer => encoder.barrierAfterEncoderStages_beforeEncoderStages_visibilityOptions(
				mtl::MTLStages::Blit,
				mtl::MTLStages::Dispatch,
				mtl::MTL4VisibilityOptions::Device,
			),
			ComputeEncoderPhase::Dispatch => encoder.barrierAfterEncoderStages_beforeEncoderStages_visibilityOptions(
				mtl::MTLStages::Dispatch,
				mtl::MTLStages::Dispatch,
				mtl::MTL4VisibilityOptions::Device,
			),
			ComputeEncoderPhase::None => {}
		}
		self.compute_encoder_phase = ComputeEncoderPhase::Dispatch;
		self.apply_bound_compute_pipeline();
		self.apply_bound_compute_descriptors();
	}

	/// Restores encoder-local render state immediately before a draw.
	pub(super) fn prepare_render_draw(&mut self) {
		self.apply_bound_render_pipeline();
		self.apply_bound_render_descriptors();
	}

	/// Encodes one render-pass clear for a compatible group of color and depth images.
	pub(super) fn encode_image_clear_batch(&mut self, images: &[(ImageHandle, graphics_hardware_interface::ClearValue)]) {
		let Some((first_handle, _)) = images.first() else {
			return;
		};
		let first_image = self.device.images.resource(*first_handle);
		let rpd = mtl::MTL4RenderPassDescriptor::new();
		if first_image.array_layers > 1 {
			rpd.setRenderTargetArrayLength(first_image.array_layers as _);
		}

		let mut color_index = 0;
		for (handle, clear_value) in images {
			let image = self.device.images.resource(*handle);
			self.command_buffer.retain_texture(image.texture.clone());
			if image.format.is_depth() {
				let attachment = rpd.depthAttachment();
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearDepth(utils::clear_depth(*clear_value));
			} else {
				let attachment = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				attachment.setTexture(Some(image.texture.as_ref()));
				attachment.setLoadAction(mtl::MTLLoadAction::Clear);
				attachment.setStoreAction(mtl::MTLStoreAction::Store);
				attachment.setClearColor(utils::clear_color(*clear_value));
				color_index += 1;
			}
		}

		let encoder = self.command_buffer.render_command_encoder(&rpd).expect(
				"Metal render command encoder creation failed. The most likely cause is that the command buffer could not start an image clear pass.",
		);
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			encoder.setLabel(Some(&self.next_encoder_block_label()));
			self.push_active_render_debug_regions(encoder.as_ref());
			for _ in 0..self.render_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.render_debug_region_depth = 0;
		}
		encoder.endEncoding();
	}
}
