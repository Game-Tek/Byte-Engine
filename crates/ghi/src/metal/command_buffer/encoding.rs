use super::*;

impl CommandBufferRecording<'_> {
	/// Returns whether `key` names the pipeline and the bound sets at their current versions.
	fn binding_is_current(&self, key: &DescriptorBindingKey, pipeline: graphics_hardware_interface::PipelineHandle) -> bool {
		key.pipeline == pipeline && key.sets == self.bound_descriptor_sets
	}

	/// Returns, for each descriptor set the pipeline declares, the bound set created from the same layout.
	///
	/// # Panics
	///
	/// Panics when no bound set was created from a layout the pipeline declares.
	fn resolve_pipeline_sets(&self, layout: &PipelineLayout) -> SmallVec<[DescriptorSetHandle; 4]> {
		layout
			.sets
			.iter()
			.map(|set| {
				self.bound_descriptor_sets
					.iter()
					.map(|(handle, _)| *handle)
					.find(|handle| {
						self.commit.descriptor_sets.resource(*handle).layout.identity() == set.layout.identity()
					})
					.unwrap_or_else(|| {
						panic!(
							"Missing descriptor set for layout `{}`. The most likely cause is that no set created from this layout was bound for the pipeline, or that a shader was built from another version of the layout.",
							set.layout.name(),
						)
					})
			})
			.collect()
	}

	/// Returns the GPU address of an argument buffer that holds `set_handle` as it is now, and keeps the buffer until
	/// the command completes.
	///
	/// A set keeps the buffer of its last encoding, so binding an unchanged set costs nothing, whichever pipeline
	/// reads it. A changed set is copied over that buffer when no command reads it any more, and into a new buffer
	/// otherwise.
	///
	/// The set keeps the resources it names alive and resident itself, see [`DescriptorSet`], so the command retains
	/// none of them. Samplers live as long as the context.
	fn descriptor_set_argument_buffer(&mut self, set_handle: DescriptorSetHandle) -> mtl::MTLGPUAddress {
		let Self {
			device,
			commit,
			command_buffer,
			..
		} = self;
		let descriptor_set = commit.descriptor_sets.resource_mut(set_handle);
		let version = descriptor_set.version;
		let words = &descriptor_set.words;
		// Copies the set's entries into `buffer`, which must have room for all of them and no reader.
		let copy_words = |buffer: &ProtocolObject<dyn mtl::MTLBuffer>| {
			// SAFETY: The buffer was allocated with room for every entry, and no command reads it.
			unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), buffer.contents().as_ptr().cast::<u64>(), words.len()) };
		};

		// Commands hold the buffers they read, so a buffer nothing else holds is read by no command.
		match descriptor_set
			.argument_buffer
			.as_mut()
			.map(|snapshot| (snapshot.version, std::sync::Arc::get_mut(snapshot)))
		{
			Some((current, _)) if current == version => {}
			Some((_, Some(snapshot))) => {
				copy_words(&snapshot.buffer);
				snapshot.version = version;
			}
			_ => {
				let length = std::mem::size_of_val(words.as_slice()).max(std::mem::size_of::<u64>());
				let buffer = device
					.metal_device
					.newBufferWithLength_options(length, mtl::MTLResourceOptions::StorageModeShared)
					.expect(
						"Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.",
					);
				#[cfg(debug_assertions)]
				if device.debug_labels {
					buffer.setLabel(Some(&NSString::from_str(descriptor_set.layout.name())));
				}
				copy_words(&buffer);
				descriptor_set.argument_buffer = Some(std::sync::Arc::new(ArgumentBufferSnapshot {
					version,
					address: buffer.gpuAddress(),
					buffer,
				}));
				descriptor_set.argument_buffer_reader = None;
			}
		}
		let snapshot = descriptor_set
			.argument_buffer
			.as_ref()
			.expect("Metal argument buffer is missing. The most likely cause is that its encoding did not store it.");
		// Binding the set again in this recording, as each pipeline that reads it does, retains nothing.
		let reader = Some((commit.queue_handle, command_buffer.serial()));
		if descriptor_set.argument_buffer_reader != reader {
			command_buffer.retain_argument_buffer(snapshot);
			descriptor_set.argument_buffer_reader = reader;
		}
		snapshot.address
	}

	/// Binds the descriptor sets the pipeline declares to the active encoder's stage tables, and returns the binding
	/// the encoder now references.
	///
	/// Each pipeline set is matched to the bound set created from its layout, so the order sets were bound in does
	/// not matter, and a bound set the pipeline does not declare is left alone.
	fn apply_argument_buffers(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> AppliedDescriptorBinding {
		// The pipelines outlive the recording, so the layout does not borrow it.
		let pipelines = self.device.pipelines;
		let layout = &pipelines[pipeline_handle.0 as usize].layout;
		let resolved = self.resolve_pipeline_sets(layout);
		// Validation scans every resource the pipeline uses, so only debug builds pay for it.
		if cfg!(debug_assertions) {
			self.validate_bound_descriptor_sets(layout, &resolved);
		}

		let compute = matches!(self.encoder_state().encoder, ActiveEncoder::Compute(_));
		let mut tables = SmallVec::<[u32; 4]>::new();
		for (set, &set_handle) in layout.sets.iter().zip(&resolved) {
			let address = self.descriptor_set_argument_buffer(set_handle);
			for &(stages, binding) in &set.bindings {
				if compute {
					// A ray tracing pipeline binds its sets for its ray generation kernel, which hands their addresses
					// to the closest-hit functions it calls, so every set binding of either kind names a stage here.
					if stages.intersects(crate::Stages::COMPUTE | crate::Stages::RAYGEN) {
						self.set_stage_buffer_address(ArgumentTableStage::Compute, binding, address);
					}
					continue;
				}
				for (stage, table_stage) in [
					(crate::Stages::TASK, ArgumentTableStage::Object),
					(crate::Stages::MESH, ArgumentTableStage::Mesh),
					(crate::Stages::VERTEX, ArgumentTableStage::Vertex),
					(crate::Stages::FRAGMENT, ArgumentTableStage::Fragment),
				] {
					if stages.intersects(stage) {
						self.set_stage_buffer_address(table_stage, binding, address);
					}
				}
			}
			tables.push(self.track_set_uses(set, set_handle));
		}

		// Sets that reach the same memory, with a write among them, are planned as one table.
		let tracked = &self.encoder_state().tables;
		let conflict = tables.iter().enumerate().any(|(index, &table)| {
			tables[index + 1..]
				.iter()
				.any(|&other| tracked[table as usize].uses.conflicts_with(&tracked[other as usize].uses))
		});
		let merged = conflict.then(|| {
			synchronization::TrackedTable::new(std::sync::Arc::new(synchronization::DescriptorUses::merged(
				tables.iter().map(|&table| &*tracked[table as usize].uses),
			)))
		});

		AppliedDescriptorBinding {
			key: DescriptorBindingKey {
				pipeline: pipeline_handle,
				sets: self.bound_descriptor_sets.clone(),
			},
			tables,
			merged,
		}
	}

	/// Returns the index, among the tables the active encoder tracks, of the resource uses of the bound set
	/// `set_handle` as the pipeline set `set` uses it.
	///
	/// Pipelines that use a set the same way share its uses, so the encoder plans the set's reads once however many
	/// of them bind it.
	fn track_set_uses(&mut self, set: &PipelineSet, set_handle: DescriptorSetHandle) -> u32 {
		let uses = self.set_uses(set, set_handle);
		let tables = &mut self.encoder_state_mut().tables;
		let index = tables
			.iter()
			.position(|table| std::sync::Arc::ptr_eq(&table.uses, &uses))
			.unwrap_or_else(|| {
				tables.push(synchronization::TrackedTable::new(uses));
				tables.len() - 1
			});
		index as u32
	}

	/// Returns the resource uses of the bound set `set_handle` as the pipeline set `set` uses it.
	///
	/// The set keeps the uses of each way a pipeline uses it while it keeps its version, and builds them again after
	/// a write.
	fn set_uses(
		&mut self,
		set: &PipelineSet,
		set_handle: DescriptorSetHandle,
	) -> std::sync::Arc<synchronization::DescriptorUses> {
		let descriptor_set = self.commit.descriptor_sets.resource_mut(set_handle);
		let version = descriptor_set.version;
		let retained = descriptor_set.uses.iter().position(|uses| uses.usage == set.usage);
		let storage = match retained {
			Some(index) if descriptor_set.uses[index].version == version => return descriptor_set.uses[index].uses.clone(),
			// The uses of the version before are built again in their own storage once no encoder tracks them.
			Some(index) => std::sync::Arc::get_mut(&mut descriptor_set.uses[index].uses)
				.map(synchronization::DescriptorUses::take_storage)
				.unwrap_or_default(),
			None => SmallVec::new(),
		};
		let uses = std::sync::Arc::new(self.resolve_set_uses(set, set_handle, storage));
		let entry = SetUses {
			usage: set.usage,
			version,
			uses: uses.clone(),
		};
		let retained_uses = &mut self.commit.descriptor_sets.resource_mut(set_handle).uses;
		match retained {
			Some(index) => retained_uses[index] = entry,
			None => retained_uses.push(entry),
		}
		uses
	}

	/// Resolves the resource accesses of a pipeline that uses the bound set `set_handle` as `set` describes.
	///
	/// Only the resources the pipeline's shaders use are accesses, with the access the layout declares for them. The
	/// table is built in `uses`, whose contents are discarded.
	fn resolve_set_uses(
		&self,
		set: &PipelineSet,
		set_handle: DescriptorSetHandle,
		mut uses: SmallVec<[synchronization::MetalResourceUse; 16]>,
	) -> synchronization::DescriptorUses {
		let descriptor_set = self.commit.descriptor_sets.resource(set_handle);
		uses.clear();
		for resource in &set.resources {
			let stages = synchronization::to_metal_stages(resource.stages);
			let access = resource.descriptor.access();
			for descriptor in descriptor_set
				.resource_descriptors(resource.resource_index)
				.iter()
				.flatten()
				.copied()
			{
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
						let image = self.swapchain_surface(handle).expect(MISSING_SURFACE).image;
						synchronization::MetalResourceUse::image(image, None, None, stages, access)
					}
					Descriptor::TopLevelAccelerationStructure { handle, copy } => {
						synchronization::MetalResourceUse::top_level(handle, copy, stages, access)
					}
					Descriptor::Sampler { .. } => continue,
				};
				let resource_use = resource_use.in_group_memory(self.device.images);
				// A texture table often names one placeholder in a run of elements, which is one access.
				if uses.last() != Some(&resource_use) {
					uses.push(resource_use);
				}
			}
		}
		synchronization::DescriptorUses::new(uses)
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
			// A ray tracing pipeline is its ray generation kernel; its trace binds the closest hits the kernel calls.
			(
				ActiveEncoder::Compute(encoder),
				PipelineState::Compute {
					state: pipeline_state, ..
				}
				| PipelineState::RayTracing {
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
			(ActiveEncoder::Render(_), PipelineState::Compute { .. } | PipelineState::RayTracing { .. }) => panic!(
				"Cannot draw with a non-raster Metal pipeline. The most likely cause is that a compute or ray tracing pipeline handle was passed to bind_raster_pipeline.",
			),
		}
		state.pipeline = Some(pipeline_handle);
	}

	/// Binds descriptor sets once per pipeline, set version, and native encoder.
	fn apply_bound_descriptors(&mut self) {
		// Refreshing the retained-set versions makes writes made after a logical bind visible before execution.
		for (handle, version) in &mut self.bound_descriptor_sets {
			*version = self.commit.descriptor_sets.resource(*handle).version;
		}
		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that a draw or dispatch was recorded before binding a pipeline.",
		);
		if self
			.encoder_state()
			.descriptors
			.as_ref()
			.is_some_and(|applied| self.binding_is_current(&applied.key, pipeline_handle))
		{
			return;
		}

		let applied = self.apply_argument_buffers(pipeline_handle);
		self.encoder_state_mut().descriptors = Some(applied);
	}

	/// Restores encoder-local state and synchronizes only the resources the next draw or dispatch consumes.
	fn prepare_command(&mut self, additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.apply_bound_pipeline();
		self.apply_bound_descriptors();
		self.consume_resources_with_descriptors(true, additional_uses);
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
			let attachment: Retained<mtl::MTLRenderPassAttachmentDescriptor> = if image.description.format.is_depth() {
				let depth = rpd.depthAttachment();
				depth.setClearDepth(utils::clear_depth(*clear_value));
				Retained::into_super(depth)
			} else {
				// SAFETY: `color_index` counts only non-depth attachments and stays within the render-pass descriptor array.
				let color = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				color.setClearColor(utils::clear_color(*clear_value));
				color_index += 1;
				Retained::into_super(color)
			};
			attachment.setTexture(Some(image.texture.as_ref()));
			attachment.setLoadAction(mtl::MTLLoadAction::Clear);
			attachment.setStoreAction(mtl::MTLStoreAction::Store);
		}

		let encoder = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
			"Metal render command encoder creation failed. The most likely cause is that the command buffer could not start an image clear pass.",
		);
		self.begin_encoder(
			ActiveEncoder::Render(encoder),
			"Clear",
			images.iter().map(|(handle, _)| *handle),
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
