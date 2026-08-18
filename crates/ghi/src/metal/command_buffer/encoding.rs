use super::*;

impl CommandBufferRecording<'_> {
	pub(super) fn render_stages(stages: crate::Stages) -> mtl::MTLRenderStages {
		let mut render_stages = mtl::MTLRenderStages(0);

		if stages.intersects(crate::Stages::VERTEX) {
			render_stages |= mtl::MTLRenderStages::Vertex;
		}

		if stages.intersects(crate::Stages::FRAGMENT) {
			render_stages |= mtl::MTLRenderStages::Fragment;
		}

		if stages.intersects(crate::Stages::TASK) {
			render_stages |= mtl::MTLRenderStages::Object;
		}

		if stages.intersects(crate::Stages::MESH) {
			render_stages |= mtl::MTLRenderStages::Mesh;
		}

		if render_stages.is_empty() {
			mtl::MTLRenderStages(
				mtl::MTLRenderStages::Vertex.0
					| mtl::MTLRenderStages::Fragment.0
					| mtl::MTLRenderStages::Object.0
					| mtl::MTLRenderStages::Mesh.0,
			)
		} else {
			render_stages
		}
	}

	pub(super) fn metal_resource_usage(access: crate::AccessPolicies) -> mtl::MTLResourceUsage {
		let mut usage = mtl::MTLResourceUsage(0);
		if access.intersects(crate::AccessPolicies::READ) {
			usage |= mtl::MTLResourceUsage::Read;
		}
		if access.intersects(crate::AccessPolicies::WRITE) {
			usage |= mtl::MTLResourceUsage::Write;
		}
		usage
	}

	/// Returns the residency usage that still needs to be declared for one compute descriptor slot.
	pub(super) fn update_compute_binding_residency(
		&mut self,
		set_handle: DescriptorSetHandle,
		version: u64,
		slot: crate::shader::ResourceSlot,
		usage: mtl::MTLResourceUsage,
	) -> Option<mtl::MTLResourceUsage> {
		if let Some((_, (resident_version, resident_usage))) = self
			.compute_resident_bindings
			.iter_mut()
			.find(|(key, _)| *key == (set_handle, slot))
		{
			if *resident_version != version {
				*resident_version = version;
				*resident_usage = usage;
				return Some(usage);
			}

			let combined = mtl::MTLResourceUsage(resident_usage.0 | usage.0);
			if combined.0 == resident_usage.0 {
				return None;
			}
			*resident_usage = combined;
			return Some(combined);
		}

		self.compute_resident_bindings.push(((set_handle, slot), (version, usage)));
		Some(usage)
	}

	/// Returns the residency usage and stages that still need to be declared for one render descriptor slot.
	pub(super) fn update_render_binding_residency(
		&mut self,
		set_handle: DescriptorSetHandle,
		version: u64,
		slot: crate::shader::ResourceSlot,
		usage: mtl::MTLResourceUsage,
		stages: mtl::MTLRenderStages,
	) -> Option<(mtl::MTLResourceUsage, mtl::MTLRenderStages)> {
		if let Some((_, (resident_version, resident_usage, resident_stages))) = self
			.render_resident_bindings
			.iter_mut()
			.find(|(key, _)| *key == (set_handle, slot))
		{
			if *resident_version != version {
				*resident_version = version;
				*resident_usage = usage;
				*resident_stages = stages;
				return Some((usage, stages));
			}

			let combined_usage = mtl::MTLResourceUsage(resident_usage.0 | usage.0);
			let combined_stages = mtl::MTLRenderStages(resident_stages.0 | stages.0);
			if combined_usage.0 == resident_usage.0 && combined_stages.0 == resident_stages.0 {
				return None;
			}
			*resident_usage = combined_usage;
			*resident_stages = combined_stages;
			return Some((combined_usage, combined_stages));
		}

		self.render_resident_bindings
			.push(((set_handle, slot), (version, usage, stages)));
		Some((usage, stages))
	}

	/// Makes the resources referenced by the flat pipeline interface resident for a render encoder.
	pub(super) fn make_render_descriptor_resources_resident(
		&mut self,
		encoder: &ProtocolObject<dyn mtl::MTLRenderCommandEncoder>,
		layout: &PipelineLayout,
	) {
		struct UsageBatch {
			usage: mtl::MTLResourceUsage,
			stages: mtl::MTLRenderStages,
			resources: SmallVec<[NonNull<ProtocolObject<dyn mtl::MTLResource>>; 32]>,
		}

		let mut batches = SmallVec::<[UsageBatch; 8]>::new();
		let mut retained_drawable_textures = SmallVec::<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 1]>::new();

		for resource in &layout.resources {
			let slot = resource.descriptor.slot();
			let Some((set_handle, _)) = self.descriptors_at_slot_with_owner(slot) else {
				continue;
			};
			let version = self.device.descriptor_sets[set_handle.0 as usize].version;
			let usage = Self::metal_resource_usage(resource.descriptor.access());
			let stages = Self::render_stages(resource.stages);
			let Some((usage, stages)) = self.update_render_binding_residency(set_handle, version, slot, usage, stages) else {
				continue;
			};
			let descriptors = self
				.descriptors_at_slot(slot)
				.expect("A Metal descriptor slot disappeared while its residency declaration was being recorded.");

			let batch_index = batches.iter().position(|b| b.usage.0 == usage.0 && b.stages.0 == stages.0);
			let batch = match batch_index {
				Some(index) => &mut batches[index],
				None => {
					batches.push(UsageBatch {
						usage,
						stages,
						resources: SmallVec::new(),
					});
					batches.last_mut().unwrap()
				}
			};

			for descriptor in descriptors.values().copied() {
				let native_resource = match descriptor {
					Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
						let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(image).texture;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
						NonNull::from(resource)
					}
					Descriptor::Buffer { buffer, .. } => {
						let buf: &ProtocolObject<dyn mtl::MTLBuffer> = &self.device.buffers.resource(buffer).buffer;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(buf);
						NonNull::from(resource)
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy_handle) =
							self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
						{
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(proxy_handle).texture;
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						} else {
							retained_drawable_textures.push(self.drawable_texture(handle));
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = retained_drawable_textures.last().unwrap().as_ref();
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() else {
							continue;
						};
						let structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure> = structure.as_ref();
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(structure);
						NonNull::from(resource)
					}
					Descriptor::Sampler { .. } => continue,
				};
				batch.resources.push(native_resource);
			}
		}

		for batch in &batches {
			if !batch.resources.is_empty() {
				let resources = NonNull::new(batch.resources.as_ptr() as *mut _)
					.expect("A non-empty Metal render residency list had a null pointer.");
				unsafe {
					encoder.useResources_count_usage_stages(resources, batch.resources.len(), batch.usage, batch.stages);
				}
			}
		}
	}

	/// Makes the resources referenced by the flat pipeline interface resident for a compute encoder.
	pub(super) fn make_compute_descriptor_resources_resident(
		&mut self,
		encoder: &ProtocolObject<dyn mtl::MTLComputeCommandEncoder>,
		layout: &PipelineLayout,
	) {
		struct UsageBatch {
			usage: mtl::MTLResourceUsage,
			resources: SmallVec<[NonNull<ProtocolObject<dyn mtl::MTLResource>>; 32]>,
		}

		let mut batches = SmallVec::<[UsageBatch; 4]>::new();
		let mut retained_drawable_textures = SmallVec::<[Retained<ProtocolObject<dyn mtl::MTLTexture>>; 1]>::new();

		for resource in &layout.resources {
			let slot = resource.descriptor.slot();
			let Some((set_handle, _)) = self.descriptors_at_slot_with_owner(slot) else {
				continue;
			};
			let version = self.device.descriptor_sets[set_handle.0 as usize].version;
			let usage = Self::metal_resource_usage(resource.descriptor.access());
			let Some(usage) = self.update_compute_binding_residency(set_handle, version, slot, usage) else {
				continue;
			};
			let descriptors = self
				.descriptors_at_slot(slot)
				.expect("A Metal descriptor slot disappeared while its residency declaration was being recorded.");

			let batch_index = batches.iter().position(|b| b.usage.0 == usage.0);
			let batch = match batch_index {
				Some(index) => &mut batches[index],
				None => {
					batches.push(UsageBatch {
						usage,
						resources: SmallVec::new(),
					});
					batches.last_mut().unwrap()
				}
			};

			for descriptor in descriptors.values().copied() {
				let native_resource = match descriptor {
					Descriptor::Image { image, .. } | Descriptor::CombinedImageSampler { image, .. } => {
						let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(image).texture;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
						NonNull::from(resource)
					}
					Descriptor::Buffer { buffer, .. } => {
						let buf: &ProtocolObject<dyn mtl::MTLBuffer> = &self.device.buffers.resource(buffer).buffer;
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(buf);
						NonNull::from(resource)
					}
					Descriptor::Swapchain { handle } => {
						if let Some(proxy_handle) =
							self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
						{
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = &self.device.images.resource(proxy_handle).texture;
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						} else {
							retained_drawable_textures.push(self.drawable_texture(handle));
							let tex: &ProtocolObject<dyn mtl::MTLTexture> = retained_drawable_textures.last().unwrap().as_ref();
							let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(tex);
							NonNull::from(resource)
						}
					}
					Descriptor::AccelerationStructure { handle } => {
						let Some(structure) = self.device.acceleration_structures[handle.0 as usize].structure.as_ref() else {
							continue;
						};
						let structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure> = structure.as_ref();
						let resource: &ProtocolObject<dyn mtl::MTLResource> = ProtocolObject::from_ref(structure);
						NonNull::from(resource)
					}
					Descriptor::Sampler { .. } => continue,
				};
				batch.resources.push(native_resource);
			}
		}

		for batch in &batches {
			if !batch.resources.is_empty() {
				let resources = NonNull::new(batch.resources.as_ptr() as *mut _)
					.expect("A non-empty Metal compute residency list had a null pointer.");
				unsafe {
					encoder.useResources_count_usage(resources, batch.resources.len(), batch.usage);
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
		let encoder = self.active_compute_encoder.clone().expect(
			"No active compute encoder. The most likely cause is that compute descriptors were prepared before a dispatch.",
		);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			if stage.intersects(crate::Stages::COMPUTE) {
				unsafe {
					encoder.setBuffer_offset_atIndex(Some(argument_buffer.as_ref()), 0, ARGUMENT_BUFFER_BINDING_BASE as _);
				}
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.make_compute_descriptor_resources_resident(encoder.as_ref(), pipeline_layout);
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
		let encoder = self.active_render_encoder.clone().expect(
			"No active render pass. The most likely cause is that render descriptors were prepared before start_render_pass.",
		);

		for (stage, argument_buffer) in materialization.argument_buffers.iter() {
			unsafe {
				if stage.intersects(crate::Stages::TASK) {
					encoder.setObjectBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
				if stage.intersects(crate::Stages::MESH) {
					encoder.setMeshBuffer_offset_atIndex(Some(argument_buffer.as_ref()), 0, ARGUMENT_BUFFER_BINDING_BASE as _);
				}
				if stage.intersects(crate::Stages::VERTEX) {
					encoder.setVertexBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
				if stage.intersects(crate::Stages::FRAGMENT) {
					encoder.setFragmentBuffer_offset_atIndex(
						Some(argument_buffer.as_ref()),
						0,
						ARGUMENT_BUFFER_BINDING_BASE as _,
					);
				}
			}
		}
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		self.make_render_descriptor_resources_resident(encoder.as_ref(), pipeline_layout);
		self.applied_render_descriptor_binding = Some(AppliedDescriptorBinding {
			pipeline: pipeline_handle,
			descriptor_sets: self.bound_descriptor_set_handles.clone(),
			versions: self.bound_descriptor_set_versions.clone(),
		});
	}

	/// Restores encoder-local compute state immediately before a dispatch.
	pub(super) fn prepare_compute_dispatch(&mut self) {
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
		let rpd = mtl::MTLRenderPassDescriptor::new();
		if first_image.array_layers > 1 {
			rpd.setRenderTargetArrayLength(first_image.array_layers as _);
		}

		let mut color_index = 0;
		for (handle, clear_value) in images {
			let image = self.device.images.resource(*handle);
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

		let encoder = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
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
