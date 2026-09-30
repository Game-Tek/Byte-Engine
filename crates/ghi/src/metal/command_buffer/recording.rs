use super::*;

impl<'a> CommandBufferRecording<'a> {
	/// Records a staging-to-buffer upload on this command buffer.
	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let buffer_handle = self.get_internal_buffer_handle(buffer_handle.into());
		self.sync_private_buffer(buffer_handle);
	}

	/// Records the upload of one frame-local buffer copy from its staging buffer, if it has one.
	pub(crate) fn sync_private_buffer(&mut self, buffer_handle: BufferHandle) {
		let buffer = self.device.buffers.resource(buffer_handle);

		let Some(staging_handle) = buffer.staging else {
			return;
		};

		let staging = self.device.buffers.resource(staging_handle);
		let staging_buffer = staging.buffer.clone();
		let destination_buffer = buffer.buffer.clone();
		let destination_size = buffer.size;
		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([
			synchronization::MetalResourceUse::buffer(
				staging_handle,
				0,
				destination_size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::READ,
			),
			synchronization::MetalResourceUse::buffer(
				buffer_handle,
				0,
				destination_size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			),
		]);

		// SAFETY: Both retained buffers expose `destination_size` bytes and are tracked for nonoverlapping transfer accesses.
		unsafe {
			transfer_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
				staging_buffer.as_ref(),
				0,
				destination_buffer.as_ref(),
				0,
				destination_size as _,
			);
		}
	}

	/// Records the upload of one frame-local image copy from its CPU staging bytes, or of one `region` of them.
	///
	/// Does nothing for images the CPU cannot access.
	pub(crate) fn sync_image(&mut self, image_handle: ImageHandle, region: Option<crate::image::Region>) {
		let image = self.device.images.resource(image_handle);
		let Some(staging) = image.staging.as_deref() else {
			return;
		};
		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([synchronization::MetalResourceUse::image(
			image_handle,
			Some(0),
			None,
			mtl::MTLStages::Blit,
			crate::AccessPolicies::WRITE,
		)]);
		let upload_buffer = encode_texture_upload(
			self.device.metal_device,
			self.commit.upload_arena,
			transfer_encoder.as_ref(),
			image.texture.as_ref(),
			image.description.format,
			image.description.extent,
			image.description.array_layers,
			staging,
			region,
		);
		// Hazard tracking never sees the upload page, so the command retains it here.
		self.command_buffer.retain_allocation(&*upload_buffer);
	}

	/// Copies each proxied swapchain's frame image into the drawable it presents.
	pub(crate) fn resolve_swapchain_proxies(
		&mut self,
		present_drawables: &[(
			graphics_hardware_interface::PresentKey,
			Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
		)],
	) {
		// The region names the shared compute encoder in capture tools, as "Compute: Present Resolve".
		self.start_region(|label| label.write_str("Present Resolve"));
		for (present_key, drawable) in present_drawables {
			let swapchain = &self.device.swapchains[present_key.swapchain.0 as usize];
			let (true, Some(drawable), Some(proxy)) = (
				swapchain.uses_proxy,
				drawable,
				swapchain.images[present_key.sequence_index as usize],
			) else {
				continue;
			};
			let source = self.device.images.resource(proxy).texture.clone();
			// The frame batch retains every presented drawable, so tracking the drawable write is enough here.
			let destination = drawable.texture();
			let transfer_encoder = self.ensure_compute_encoder().clone();
			self.consume_resources([
				synchronization::MetalResourceUse::image(proxy, None, None, mtl::MTLStages::Blit, crate::AccessPolicies::READ),
				synchronization::MetalResourceUse::drawable(
					destination.as_ref(),
					mtl::MTLStages::Blit,
					crate::AccessPolicies::WRITE,
				),
			]);
			// SAFETY: Source and drawable textures are retained and validated for the proxy resolve copy.
			unsafe {
				transfer_encoder.copyFromTexture_toTexture(source.as_ref(), destination.as_ref());
			}
		}
		self.end_region();
	}

	pub(crate) fn new(
		device: RecordingDevice<'a>,
		commit: RecordingCommit<'a>,
		mut command_buffer: queue::NativeCommand,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		autorelease_pool: Option<Retained<NSAutoreleasePool>>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let mut resource_tracker = std::mem::take(&mut commit.queue.resource_tracker);
		resource_tracker.begin_recording();
		// Shared argument tables are snapshotted by every command that binds them, so retain them up front.
		for table in commit.argument_tables.iter() {
			command_buffer.retain_object(&**table);
		}

		Self {
			device,
			commit,
			frame_key,
			sequence_index,
			command_buffer: NativeCommandSlot(Some(command_buffer)),
			#[cfg(debug_assertions)]
			debug_regions: Vec::new_in(allocator),
			drawables: Vec::new_in(allocator),
			bound_pipeline: None,
			bound_descriptor_set_roots: SmallVec::new(),
			bound_descriptor_sets: SmallVec::new(),
			bound_vertex_buffers: SmallVec::new(),
			render_vertex_buffers_dirty: false,
			encoded_vertex_buffer_count: 0,
			bound_index_buffer: None,
			push_constant_data: Vec::new_in(allocator),
			encoder: None,
			active_render_extent: Extent::rectangle(0, 0),
			next_encoder_id: 0,
			resource_tracker,
			active_render_attachment_uses: SmallVec::new(),
			texture_readbacks: SmallVec::new(),
			_autorelease_pool: autorelease_pool,
		}
	}

	/// Labels a new native encoder and mirrors every active logical debug region into it.
	///
	/// The label reads `<kind>: <region path> → <targets>`, so capture tools list what each encoder does and writes.
	/// A `None` target is a drawable. Returns how many regions it pushed, which the encoder pops before it ends.
	#[cfg(debug_assertions)]
	fn begin_encoder_debug_regions(
		&self,
		encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder>,
		kind: &str,
		targets: impl IntoIterator<Item = Option<ImageHandle>>,
	) -> usize {
		use std::fmt::Write as _;

		if !self.device.debug_labels {
			return 0;
		}
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		let _ = label.write_str(kind);
		for (index, region) in self.debug_regions.iter().enumerate() {
			let _ = label.write_str(if index == 0 { ": " } else { " › " });
			// Formatting the native string writes it in place, so no temporary String is allocated per region.
			let _ = write!(label, "{region}");
		}
		for (index, target) in targets.into_iter().enumerate() {
			let _ = label.write_str(if index == 0 { " → " } else { ", " });
			let name = match target {
				Some(handle) => self.device.images.resource(handle).name.as_deref().unwrap_or("Unnamed Image"),
				None => "Drawable",
			};
			let _ = label.write_str(name);
		}
		encoder.setLabel(Some(&NSString::from_str(label.as_str())));
		for region in &self.debug_regions {
			encoder.pushDebugGroup(region);
		}
		self.debug_regions.len()
	}

	/// Inserts a signpost that names the resources and accesses behind the barrier the tracker just planned.
	///
	/// The label reads `Barrier: <resource> (<earlier access> → <next access>), ...`.
	#[cfg(debug_assertions)]
	fn signpost_barrier_hazards(&self, encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder>) {
		use std::fmt::Write as _;

		let hazards = self.resource_tracker.hazards();
		if !self.device.debug_labels || hazards.is_empty() {
			return;
		}
		let access = |access: crate::AccessPolicies| {
			if access.contains(crate::AccessPolicies::READ | crate::AccessPolicies::WRITE) {
				"read-write"
			} else if access.intersects(crate::AccessPolicies::WRITE) {
				"write"
			} else {
				"read"
			}
		};
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		let _ = label.write_str("Barrier: ");
		for (index, hazard) in hazards.iter().enumerate() {
			if index > 0 {
				let _ = label.write_str(", ");
			}
			let name = match hazard.key {
				synchronization::MetalResourceKey::Buffer(handle) => self.device.buffers.resource(handle).name.as_deref(),
				synchronization::MetalResourceKey::Image(handle) => self.device.images.resource(handle).name.as_deref(),
				synchronization::MetalResourceKey::SwapchainDrawable(_) => Some("Drawable"),
				synchronization::MetalResourceKey::AccelerationStructure(_) => Some("Acceleration Structure"),
				synchronization::MetalResourceKey::GroupHeap(_) => Some("Image Group Heap"),
			};
			let _ = label.write_str(name.unwrap_or("Unnamed Resource"));
			if let synchronization::MetalResourceRegion::Texture { mip_level, layer } = hazard.region {
				if let Some(mip_level) = mip_level {
					let _ = write!(label, " mip {mip_level}");
				}
				if let Some(layer) = layer {
					let _ = write!(label, " layer {layer}");
				}
			}
			let _ = write!(label, " ({} → {})", access(hazard.previous), access(hazard.next));
		}
		encoder.insertDebugSignpost(&NSString::from_str(label.as_str()));
	}

	/// Makes `encoder` the recording's active encoder.
	///
	/// Call [`Self::end_encoder`] before creating the native encoder, since Metal allows one open encoder per command
	/// buffer. This is the only place encoder-local state is built, so every new encoder starts with no pipeline, no
	/// bound snapshot, and push constants to re-upload. A render encoder also starts with its vertex bindings
	/// unencoded. `kind` and `targets` label the encoder in capture tools; a `None` target is a drawable.
	pub(super) fn begin_encoder(
		&mut self,
		encoder: ActiveEncoder,
		_kind: &str,
		_targets: impl IntoIterator<Item = Option<ImageHandle>>,
	) {
		assert!(
			self.encoder.is_none(),
			"A Metal encoder is already open. The most likely cause is that a new encoder was created before end_encoder.",
		);
		#[cfg(debug_assertions)]
		let debug_region_depth = self.begin_encoder_debug_regions(encoder.common(), _kind, _targets);
		if matches!(encoder, ActiveEncoder::Render(_)) {
			self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
			self.encoded_vertex_buffer_count = 0;
		}
		self.encoder = Some(EncoderState {
			encoder,
			scope: self.allocate_encoder_scope(),
			pipeline: None,
			descriptors: None,
			push_constants_dirty: !self.push_constant_data.is_empty(),
			#[cfg(debug_assertions)]
			debug_region_depth,
		});
	}

	/// Ends the active encoder, if any, after balancing its mirrored debug regions.
	///
	/// A render encoder also records its attachment writes, so later commands order after them.
	pub(super) fn end_encoder(&mut self) {
		let Some(state) = self.encoder.take() else {
			return;
		};
		let encoder = state.encoder.common();
		#[cfg(debug_assertions)]
		for _ in 0..state.debug_region_depth {
			encoder.popDebugGroup();
		}
		encoder.endEncoding();
		if let ActiveEncoder::Render(_) = state.encoder {
			self.resource_tracker
				.record_final(state.scope, self.active_render_attachment_uses.drain(..));
		}
	}

	/// Records render-target writes after a draw so a later aliased access sees the dependency.
	pub(super) fn record_render_attachment_writes(&mut self) {
		let scope = self.encoder_state().scope;
		self.resource_tracker
			.record_final(scope, self.active_render_attachment_uses.iter().copied());
	}

	/// Returns the active encoder's local state.
	pub(super) fn encoder_state(&self) -> &EncoderState {
		self.encoder
			.as_ref()
			.expect("No active Metal encoder. The most likely cause is that a command was recorded after its encoder ended.")
	}

	/// Returns the active encoder's local state for updates.
	pub(super) fn encoder_state_mut(&mut self) -> &mut EncoderState {
		self.encoder
			.as_mut()
			.expect("No active Metal encoder. The most likely cause is that a command was recorded after its encoder ended.")
	}

	/// Returns the active render encoder, or panics naming `operation` when no render pass is open.
	pub(super) fn render_encoder(&self, operation: &str) -> &Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>> {
		match self.encoder.as_ref().map(|state| &state.encoder) {
			Some(ActiveEncoder::Render(encoder)) => encoder,
			_ => {
				panic!("No active render pass. The most likely cause is that {operation} was called outside start_render_pass.")
			}
		}
	}

	/// Retains acquired drawables that may be referenced directly while recording this frame.
	pub(crate) fn attach_drawables(
		&mut self,
		drawables: impl Iterator<
			Item = (
				graphics_hardware_interface::SwapchainHandle,
				Retained<ProtocolObject<dyn CAMetalDrawable>>,
			),
		>,
	) {
		for (handle, drawable) in drawables {
			self.command_buffer.retain_drawable(&drawable);
			self.drawables.push((handle, drawable));
		}
	}

	pub(crate) fn into_finished(mut self) -> FinishedCommandBuffer {
		self.end_encoder();
		self.publish_resource_states();

		FinishedCommandBuffer {
			queue_handle: self.commit.queue_handle,
			command_buffer: self.command_buffer.take(),
			texture_readbacks: std::mem::take(&mut self.texture_readbacks),
		}
	}

	pub(super) fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>> {
		if !matches!(
			self.encoder.as_ref().map(|state| &state.encoder),
			Some(ActiveEncoder::Compute(_))
		) {
			self.end_encoder();
			// One serial MTL4 compute encoder records both copy and dispatch commands. Phase transitions add explicit visibility.
			let encoder = self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			self.begin_encoder(ActiveEncoder::Compute(encoder), "Compute", []);
		}

		match self.encoder.as_ref().map(|state| &state.encoder) {
			Some(ActiveEncoder::Compute(encoder)) => encoder,
			_ => unreachable!("The compute encoder was started above."),
		}
	}

	/// Allocates one command-local identity for hazard tracking within a native encoder.
	fn allocate_encoder_scope(&mut self) -> synchronization::MetalEncoderScope {
		let id = self.next_encoder_id;
		self.next_encoder_id = self.next_encoder_id.checked_add(1).expect(
			"Metal encoder identity overflowed. The most likely cause is that one command recording created more than u32::MAX encoders.",
		);
		synchronization::MetalEncoderScope::Encoder(id)
	}

	/// Applies the dependencies one command needs on the active encoder and retains what it uses.
	///
	/// `descriptors` is the snapshot the command binds, whose uses are read in place. Every use in `additional_uses`
	/// has its native allocation retained here, so commands only retain objects hazard tracking never sees.
	pub(super) fn consume_resources_with_descriptors(
		&mut self,
		descriptors: Option<&mut AppliedDescriptorBinding>,
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let scope = self.encoder_state().scope;
		let Self {
			device,
			commit,
			command_buffer,
			resource_tracker,
			..
		} = self;
		let additional_uses = additional_uses
			.into_iter()
			.map(|resource_use| resource_use.in_group_memory(device.images))
			.collect::<SmallVec<[_; 8]>>();
		for resource_use in &additional_uses {
			retain_tracked_use(device, command_buffer, resource_use);
		}
		let descriptors = descriptors.map(|binding| (binding.snapshot.uses(commit.descriptor_sets), &mut binding.settled));
		// Only debug builds check members; descriptor members' memory was retained when their snapshot was applied.
		if cfg!(debug_assertions) {
			let descriptor_members = descriptors.iter().flat_map(|(uses, _)| uses.members());
			for member in descriptor_members.chain(additional_uses.iter().filter_map(|resource_use| resource_use.member)) {
				commit
					.image_groups
					.assert_initialized(graphics_hardware_interface::BaseImageHandle(member.0), || {
						device.images.resource(member).name.clone()
					});
			}
		}
		let barrier = match descriptors {
			Some((uses, settled)) => resource_tracker.consume_descriptors(scope, uses, settled, additional_uses),
			None => resource_tracker.consume(scope, additional_uses),
		};
		let encoder = self.encoder_state().encoder.common();
		#[cfg(debug_assertions)]
		self.signpost_barrier_hazards(encoder);
		barrier.encode(encoder);
	}

	/// Applies only the queue and encoder dependencies required by the resources one command consumes.
	pub(super) fn consume_resources(&mut self, uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.consume_resources_with_descriptors(None, uses);
	}

	/// Publishes this finalized recording's resource history to its queue.
	fn publish_resource_states(&mut self) {
		let recording = self.resource_tracker.finish_recording();
		self.command_buffer.set_tracked_recording(recording);
		self.commit.queue.resource_tracker = std::mem::take(&mut self.resource_tracker);
	}

	/// Returns the shared Metal 4 argument table for one stage, creating it on first use.
	pub(super) fn argument_table(&mut self, stage: ArgumentTableStage) -> Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>> {
		if let Some(table) = self.commit.argument_tables.get(stage) {
			return table.clone();
		}

		let descriptor = mtl::MTL4ArgumentTableDescriptor::new();
		descriptor.setMaxBufferBindCount(ARGUMENT_TABLE_BUFFER_COUNT);
		descriptor.setInitializeBindings(true);
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			descriptor.setLabel(Some(&NSString::from_str(stage.label())));
		}
		let table = self.device.metal_device.newArgumentTableWithDescriptor_error(&descriptor);
		let table = table.expect(
			"Metal 4 argument table creation failed. The most likely cause is that the device ran out of binding-table memory.",
		);
		self.command_buffer.retain_object(&*table);
		self.commit.argument_tables.insert(stage, table.clone());
		table
	}

	/// Updates one stage table and associates it with the active encoder before its next snapshot command.
	pub(super) fn set_stage_buffer_address(&mut self, stage: ArgumentTableStage, binding: u32, address: mtl::MTLGPUAddress) {
		assert!(
			(binding as usize) < ARGUMENT_TABLE_BUFFER_COUNT,
			"Metal argument-table buffer binding is out of range. The most likely cause is that a shader buffer index exceeded the fixed 17-buffer ABI. binding={binding}",
		);
		let table = self.argument_table(stage);
		// SAFETY: `binding` is checked against the fixed table size and `address` names a retained buffer.
		unsafe {
			table.setAddress_atIndex(address, binding as _);
		}

		match (stage, &self.encoder_state().encoder) {
			(ArgumentTableStage::Compute, ActiveEncoder::Compute(encoder)) => encoder.setArgumentTable(Some(table.as_ref())),
			(ArgumentTableStage::Compute, ActiveEncoder::Render(_)) => panic!(
				"No active Metal compute encoder. The most likely cause is that a compute table was updated outside dispatch preparation.",
			),
			(stage, ActiveEncoder::Render(encoder)) => encoder.setArgumentTable_atStages(table.as_ref(), stage.render_stage()),
			(_, ActiveEncoder::Compute(_)) => panic!(
				"No active Metal render encoder. The most likely cause is that a render table was updated outside a render pass.",
			),
		}
	}

	/// Uploads the current logical push state into an immutable range of the frame's upload arena.
	fn upload_push_constants(&mut self) -> mtl::MTLGPUAddress {
		let (buffer, offset) = self
			.commit
			.upload_arena
			.upload(self.device.metal_device, &self.push_constant_data);
		let address = buffer.gpuAddress().checked_add(offset as u64).expect(
			"Metal push upload GPU address overflowed. The most likely cause is an invalid buffer address or upload offset.",
		);
		self.command_buffer.retain_allocation(&**buffer);
		address
	}

	pub(super) fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	pub(super) fn get_internal_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device.images.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	/// Resolves an image or swapchain to the surface this frame's commands use.
	///
	/// `frame_offset` selects another frame's copy of a per-frame image; a swapchain only has this frame's surface.
	/// Returns `None` for an unknown handle, or for a direct swapchain whose drawable was not acquired.
	pub(super) fn surface(&self, target: ImageOrSwapchain, frame_offset: i32) -> Option<Surface> {
		match target {
			ImageOrSwapchain::Image(image) => {
				self.device.images.get_single(image)?;
				let frame_index = crate::frame_resources::frame_index_with_offset(
					self.sequence_index as usize,
					frame_offset,
					self.device.frames as usize,
				);
				Some(self.image_surface(self.device.images.nth_handle(image, frame_index)?))
			}
			ImageOrSwapchain::Swapchain(swapchain) => self.swapchain_surface(crate::swapchain::SwapchainHandle(swapchain.0)),
		}
	}

	/// Returns the surface a swapchain renders into this frame: its proxy image when it has one, else its drawable.
	///
	/// Both report the swapchain's uses. Returns `None` in the same cases as [`Self::surface`].
	pub(super) fn swapchain_surface(&self, handle: crate::swapchain::SwapchainHandle) -> Option<Surface> {
		let swapchain = self.device.swapchains.get(handle.0 as usize)?;
		// A proxy image reports the swapchain's uses, so both arms validate against the swapchain.
		Some(match swapchain.images[self.sequence_index as usize] {
			Some(proxy) => Surface {
				uses: swapchain.uses,
				..self.image_surface(proxy)
			},
			None => Surface {
				image: None,
				texture: self
					.drawables
					.iter()
					.find(|(swapchain, _)| swapchain.0 == handle.0)
					.map(|(_, drawable)| drawable.texture())?,
				// TODO: get the drawable's actual format.
				format: crate::Formats::BGRAu8,
				extent: swapchain.extent,
				array_layers: 1,
				uses: swapchain.uses,
			},
		})
	}

	/// Returns the surface of one frame-local image.
	fn image_surface(&self, handle: ImageHandle) -> Surface {
		let image = self.device.images.resource(handle);
		Surface {
			image: Some(handle),
			texture: image.texture.clone(),
			format: image.description.format,
			extent: image.description.extent,
			array_layers: image.description.array_layers,
			uses: image.description.uses,
		}
	}

	pub(super) fn descriptors_at_slot(&self, slot: crate::shader::ResourceSlot) -> Option<&HashMap<u32, Descriptor>> {
		descriptors_at_slot(self.commit.descriptor_sets, &self.bound_descriptor_sets, slot).map(|(_, descriptors)| descriptors)
	}

	/// Returns the frame-local handles of the bound descriptor sets.
	fn bound_descriptor_set_handles(&self) -> impl Iterator<Item = DescriptorSetHandle> + '_ {
		self.bound_descriptor_sets.iter().map(|(handle, _)| *handle)
	}

	pub(super) fn descriptor_matches_kind(descriptor: Descriptor, kind: crate::shader::ResourceKind) -> bool {
		match descriptor {
			Descriptor::Buffer { .. } => matches!(
				kind,
				crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer
			),
			Descriptor::Image { .. } | Descriptor::Swapchain { .. } => matches!(
				kind,
				crate::shader::ResourceKind::SampledImage
					| crate::shader::ResourceKind::StorageImage
					| crate::shader::ResourceKind::InputAttachment
			),
			Descriptor::CombinedImageSampler { .. } => kind == crate::shader::ResourceKind::CombinedImageSampler,
			Descriptor::Sampler { .. } => kind == crate::shader::ResourceKind::Sampler,
			Descriptor::AccelerationStructure { .. } => kind == crate::shader::ResourceKind::AccelerationStructure,
		}
	}

	/// Validates the retained set union against the active pipeline without requiring fixed arrays to be fully populated.
	///
	/// Only debug builds call it, because it scans every bound set against every pipeline resource.
	pub(super) fn validate_bound_descriptor_sets(&self, layout: &PipelineLayout) {
		for (left_index, left_handle) in self.bound_descriptor_set_handles().enumerate() {
			let left = self.commit.descriptor_sets.resource(left_handle);
			for right_handle in self.bound_descriptor_set_handles().skip(left_index + 1) {
				let right = self.commit.descriptor_sets.resource(right_handle);

				assert!(
					left.descriptors.keys().all(|slot| !right.descriptors.contains_key(slot)),
					"Overlapping retained descriptor sets. The most likely cause is that two bound sets write the same flat resource slot.",
				);
			}
		}

		for resource in &layout.resources {
			let descriptor = resource.descriptor;
			let range_start = descriptor.slot().index();
			let range_end = resource_range_end(descriptor);
			for set_handle in self.bound_descriptor_set_handles() {
				let descriptor_set = self.commit.descriptor_sets.resource(set_handle);

				assert!(
					descriptor_set
						.descriptors
						.keys()
						.all(|slot| resource_accepts_retained_slot_key(descriptor, *slot)),
					"Invalid retained descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array's base slot.",
				);
			}
			let owner_count = self
				.bound_descriptor_set_handles()
				.filter(|set_handle| {
					self.commit
						.descriptor_sets
						.resource(*set_handle)
						.descriptors
						.keys()
						.any(|slot| (range_start..range_end).contains(&slot.index()))
				})
				.count();

			assert!(
				owner_count <= 1,
				"Overlapping retained descriptor sets. The most likely cause is that two bound sets own slots within the same active shader resource range.",
			);

			let descriptors = self.descriptors_at_slot(descriptor.slot());
			if descriptor.count() == 1 {
				assert!(
					descriptors.is_some_and(|descriptors| descriptors.contains_key(&0)),
					"Missing retained descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					descriptor.slot().index(),
				);
			}

			if let Some(descriptors) = descriptors {
				for (&array_element, &value) in descriptors {
					assert!(
						array_element < descriptor.count(),
						"Descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(value, descriptor.kind()),
						"Descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
				}
			}
		}
	}

	/// Binds a pipeline for later commands and starts from zeroed push constants when the pipeline changes.
	pub(super) fn bind_pipeline(&mut self, pipeline_handle: graphics_hardware_interface::PipelineHandle) -> &mut Self {
		if self.bound_pipeline != Some(pipeline_handle) {
			self.bound_pipeline = Some(pipeline_handle);
			let push_constant_size = self.device.pipelines[pipeline_handle.0 as usize].layout.push_constant_size;
			self.push_constant_data.clear();
			self.push_constant_data.resize(push_constant_size, 0);
			if let Some(state) = &mut self.encoder {
				state.push_constants_dirty = push_constant_size > 0;
			}
		}
		self
	}

	/// Returns the buffer ranges consumed by the next ordinary vertex draw.
	pub(super) fn bound_vertex_resource_uses(&self) -> SmallVec<[synchronization::MetalResourceUse; 8]> {
		self.bound_vertex_buffers
			.iter()
			.map(|(buffer_handle, offset)| {
				let handle = self.get_internal_buffer_handle(*buffer_handle);
				let buffer = self.device.buffers.resource(handle);
				synchronization::MetalResourceUse::buffer(
					handle,
					*offset,
					buffer.size.saturating_sub(*offset),
					mtl::MTLStages::Vertex,
					crate::AccessPolicies::READ,
				)
			})
			.collect()
	}

	/// Applies changed logical vertex-buffer addresses once before the next ordinary draw.
	pub(super) fn apply_bound_vertex_buffers(&mut self) {
		if !self.render_vertex_buffers_dirty {
			return;
		}

		assert!(
			self.bound_vertex_buffers.len() <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal vertex buffers were bound. The most likely cause is that a vertex binding overlaps the reserved push-constant slot."
		);

		// The draw that follows tracks these buffers, which retains them.
		let addresses = self
			.bound_vertex_buffers
			.iter()
			.map(|&(buffer_handle, offset)| {
				let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
				buffer.gpu_address.checked_add(offset as u64).expect(
					"Metal vertex buffer address overflowed. The most likely cause is that the requested vertex offset exceeds the native buffer address range.",
				)
			})
			.collect::<SmallVec<[_; PUSH_CONSTANT_BINDING_INDEX as usize]>>();
		self.encode_vertex_addresses(&addresses);
		self.render_vertex_buffers_dirty = false;
	}

	/// Writes `addresses` into the leading vertex argument-table bindings and zeroes bindings an earlier draw left.
	pub(super) fn encode_vertex_addresses(&mut self, addresses: &[mtl::MTLGPUAddress]) {
		for (binding, &address) in addresses.iter().enumerate() {
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, address);
		}
		for binding in addresses.len()..self.encoded_vertex_buffer_count {
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, 0);
		}
		self.encoded_vertex_buffer_count = addresses.len();
	}

	/// Uploads changed push constants once before the next command and binds them for the stages it runs.
	pub(super) fn flush_push_constants(&mut self) {
		let state = self.encoder_state();
		if !state.push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pipeline_handle = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that push constants were flushed before binding a pipeline.");
		// Compute work reads one table. Draws read the fragment table and either the vertex table or the mesh tables.
		let mut stages = SmallVec::<[ArgumentTableStage; 3]>::new();
		match state.encoder {
			ActiveEncoder::Compute(_) => stages.push(ArgumentTableStage::Compute),
			ActiveEncoder::Render(_) => {
				let raster = self.device.pipelines[pipeline_handle.0 as usize].raster();
				if raster.mesh_threadgroup_size.is_some() {
					if raster.object_threadgroup_size.is_some() {
						stages.push(ArgumentTableStage::Object);
					}
					stages.push(ArgumentTableStage::Mesh);
				} else {
					stages.push(ArgumentTableStage::Vertex);
				}
				stages.push(ArgumentTableStage::Fragment);
			}
		}
		let address = self.upload_push_constants();
		for stage in stages {
			self.set_stage_buffer_address(stage, PUSH_CONSTANT_BINDING_INDEX, address);
		}
		self.encoder_state_mut().push_constants_dirty = false;
	}

	/// Ends and submits a non-frame recording as a one-command Metal 4 batch.
	pub(crate) fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.end_encoder();
		self.publish_resource_states();
		let synchronizer = context::synchronizer_for_sequence(self.commit.synchronizers, synchronizer, self.sequence_index);
		for handle in self.texture_readbacks.drain(..) {
			self.commit.texture_readbacks.mark_submitted(handle, Some(synchronizer));
		}

		let commands = SmallVec::<[queue::NativeCommand; 4]>::from_iter([self.command_buffer.take()]);
		let submitted = self.commit.queue.submit_batch(self.commit.queue_handle, commands);
		// The synchronizer owns the submitted batch until its completion message arrives.
		self.commit.synchronizers.resource_mut(synchronizer).signal(submitted);
	}
}

/// Returns the first bound set that writes `slot`, and that set's descriptors there.
pub(super) fn descriptors_at_slot<'s>(
	descriptor_sets: &'s context::DescriptorSets,
	bound_descriptor_sets: &[(DescriptorSetHandle, u64)],
	slot: crate::shader::ResourceSlot,
) -> Option<(DescriptorSetHandle, &'s HashMap<u32, Descriptor>)> {
	bound_descriptor_sets.iter().find_map(|&(set_handle, _)| {
		descriptor_sets
			.resource(set_handle)
			.descriptors
			.get(&slot)
			.map(|descriptors| (set_handle, descriptors))
	})
}

/// Retains the native allocation behind one tracked use until the command completes.
///
/// Drawables need nothing here: the recording retains each acquired drawable when it attaches it.
fn retain_tracked_use(
	device: &RecordingDevice<'_>,
	command_buffer: &mut queue::NativeCommand,
	resource_use: &synchronization::MetalResourceUse,
) {
	match resource_use.key {
		synchronization::MetalResourceKey::Buffer(handle) => {
			command_buffer.retain_allocation(&*device.buffers.resource(handle).buffer);
		}
		synchronization::MetalResourceKey::Image(handle) => {
			command_buffer.retain_allocation(&*device.images.resource(handle).texture);
		}
		synchronization::MetalResourceKey::GroupHeap(_) => {
			let member = resource_use.member.expect(
				"Metal image-group use has no member. The most likely cause is that a heap use was built outside in_group_memory.",
			);
			retain_image(device, command_buffer, member);
		}
		synchronization::MetalResourceKey::AccelerationStructure(index) => {
			command_buffer.retain_allocation(&*device.acceleration_structures[index].structure);
		}
		synchronization::MetalResourceKey::SwapchainDrawable(_) => {}
	}
}

/// Retains an image's texture and, for an image-group member, the heap that holds its memory.
pub(super) fn retain_image(device: &RecordingDevice<'_>, command_buffer: &mut queue::NativeCommand, handle: ImageHandle) {
	let image = device.images.resource(handle);
	command_buffer.retain_allocation(&*image.texture);
	if let Some(slot) = &image.slot {
		command_buffer.retain_allocation(&*slot.heap);
	}
}
