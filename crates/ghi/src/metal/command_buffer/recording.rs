use super::*;

impl<'a> CommandBufferRecording<'a> {
	/// Records the upload of one frame-local buffer copy from its staging buffer, if it has one.
	pub(crate) fn sync_private_buffer(&mut self, buffer_handle: BufferHandle) {
		let buffer = self.device.buffers.resource(buffer_handle);

		let Some(staging_handle) = buffer.staging else {
			return;
		};

		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([
			synchronization::MetalResourceUse::buffer(
				staging_handle,
				0,
				buffer.size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::READ,
			),
			synchronization::MetalResourceUse::buffer(
				buffer_handle,
				0,
				buffer.size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			),
		]);

		// SAFETY: Both retained buffers expose `buffer.size` bytes and are tracked for nonoverlapping transfer accesses.
		unsafe {
			transfer_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
				&self.device.buffers.resource(staging_handle).buffer,
				0,
				&buffer.buffer,
				0,
				buffer.size as _,
			);
		}
	}

	/// Records the upload of one frame-local image copy from its CPU staging bytes, or of one `region` of them.
	///
	/// Does nothing for images the CPU cannot access.
	pub(crate) fn sync_image(&mut self, image_handle: ImageHandle, region: Option<crate::image::Region>) {
		if let Some(staging) = self.device.images.resource(image_handle).staging.as_deref() {
			self.upload_texture(image_handle, staging, region);
		}
	}

	/// Copies compact CPU texture data into an aligned upload range and records the blits into one frame-local image.
	///
	/// `bytes` holds every array layer of the image; `region`, when present, selects the rectangle of each layer to
	/// copy. The upload range snapshots `bytes` now, and the tracked blits write the image in command order.
	pub(super) fn upload_texture(&mut self, image_handle: ImageHandle, bytes: &[u8], region: Option<crate::image::Region>) {
		let image = self.device.images.resource(image_handle);
		let ImageDescription {
			format,
			extent,
			array_layers,
			..
		} = image.description;
		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([synchronization::MetalResourceUse::image(
			image_handle,
			Some(0),
			None,
			mtl::MTLStages::Blit,
			crate::AccessPolicies::WRITE,
		)]);

		let (source_row_pitch, _, source_image_pitch) = utils::texture_upload_layout(format, extent);
		if let Some(region) = region {
			region.validate(extent, format, array_layers);
		}
		let copy_extent = region.map_or(extent, |region| Extent::rectangle(region.size[0], region.size[1]));
		let origin = region.map_or([0, 0], |region| region.offset);
		let source_start = origin[1] as usize * source_row_pitch + origin[0] as usize * crate::types::Size::size(&format);
		let (bytes_per_row, row_count, _) = utils::texture_upload_layout(format, copy_extent);
		let expected_size = source_image_pitch.checked_mul(array_layers as usize).expect(
			"Metal texture upload size overflowed. The most likely cause is an invalid array layer count or image extent.",
		);

		assert!(
			bytes.len() >= expected_size,
			"Metal texture upload data is too small. The most likely cause is that the source payload does not contain every image layer. staging_len={}, expected_size={expected_size}",
			bytes.len(),
		);
		if format.bc_bytes_per_block().is_some() {
			assert_eq!(
				bytes.len(),
				expected_size,
				"Metal compressed texture staging size mismatch. The most likely cause is that CPU staging was not packed as one compact BC image per slice. format={format:?}, extent={extent:?}, array_layers={array_layers}, staging_len={}, expected_size={expected_size}",
				bytes.len()
			);
		}

		let (aligned_bytes_per_row, aligned_bytes_per_image) = utils::texture_copy_pitches(bytes_per_row, row_count);
		let upload_size = aligned_bytes_per_image.checked_mul(array_layers as usize).expect(
			"Metal texture upload buffer size overflowed. The most likely cause is an invalid array layer count or image pitch.",
		);
		let (upload_page, upload_offset) = self.commit.upload_arena.allocate(self.device.metal_device, upload_size);
		// SAFETY: The arena range starts at `upload_offset` and spans `upload_size` writable bytes.
		let destination = unsafe { upload_page.contents.add(upload_offset) };
		let mut source_size = utils::mtl_size(copy_extent);
		source_size.depth = 1;
		let destination_origin = mtl::MTLOrigin {
			x: origin[0] as _,
			y: origin[1] as _,
			z: 0,
		};

		// The CPU copies all land before the command is submitted, so each blit can follow its slice's copy.
		for slice in 0..array_layers as usize {
			let source_offset = slice * source_image_pitch;
			let source_bytes = &bytes[source_offset..source_offset + source_image_pitch];
			// SAFETY: The size checks above keep every source row of the region inside this slice, the upload allocation
			// covers every padded row of every layer, and caller bytes never alias an upload page.
			unsafe {
				utils::copy_rows(
					source_bytes.as_ptr().add(source_start),
					source_row_pitch,
					destination.add(slice * aligned_bytes_per_image),
					aligned_bytes_per_row,
					bytes_per_row,
					row_count,
				);
			}
			// SAFETY: The upload buffer layout and destination slice range were validated while the image was built.
			unsafe {
				transfer_encoder.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
					&upload_page.buffer,
					(upload_offset + slice * aligned_bytes_per_image) as _,
					aligned_bytes_per_row as _,
					aligned_bytes_per_image as _,
					source_size,
					&image.texture,
					slice,
					0,
					destination_origin,
				);
			}
		}
		// Hazard tracking never sees the upload page, so the command retains it here.
		self.command_buffer.retain_allocation(&*upload_page.buffer);
	}

	/// Copies each presented swapchain's image for this frame into the drawable that presents it.
	pub(crate) fn resolve_swapchain_images(
		&mut self,
		present_drawables: &[(
			graphics_hardware_interface::PresentKey,
			Retained<ProtocolObject<dyn CAMetalDrawable>>,
		)],
	) {
		// The region names the shared compute encoder in capture tools, as "Compute: Present Resolve".
		self.start_region(|label| label.write_str("Present Resolve"));
		for (present_key, drawable) in present_drawables {
			let Some(image) =
				self.device.swapchains[present_key.swapchain.0 as usize].images[present_key.sequence_index as usize]
			else {
				continue;
			};
			let source = &self.device.images.resource(image).texture;
			// The resolve command retains every presented drawable, so tracking the drawable write is enough here.
			let destination = drawable.texture();
			let transfer_encoder = self.ensure_compute_encoder().clone();
			self.consume_resources([
				synchronization::MetalResourceUse::image(image, None, None, mtl::MTLStages::Blit, crate::AccessPolicies::READ),
				synchronization::MetalResourceUse::drawable(
					destination.as_ref(),
					mtl::MTLStages::Blit,
					crate::AccessPolicies::WRITE,
				),
			]);
			// SAFETY: Source and drawable textures are retained, and both have the extent the swapchain was acquired at.
			unsafe {
				transfer_encoder.copyFromTexture_toTexture(source, &destination);
			}
		}
		self.end_region();
	}

	/// Starts a recording on `queue_handle` without submitting pending uploads first.
	///
	/// [`context::Context::create_command_buffer_recording`] submits pending uploads and then starts caller recordings
	/// here. Internal work that is itself part of an upload or a presentation starts here directly, so it shares the
	/// hazard tracking and copy code of every other recording.
	pub(crate) fn new(
		context: &'a mut context::Context,
		queue_handle: graphics_hardware_interface::QueueHandle,
		label: Option<&str>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Self {
		// SAFETY: Detached recordings create and drain the pool on their owning thread.
		let autorelease_pool = frame_key.is_none().then(|| unsafe { NSAutoreleasePool::new() });
		// A frame records into its retained arena, and a recording outside any frame into the transient arena.
		let arena_index = frame_key.map_or(context.frames as usize, |key| key.sequence_index as usize);
		// Acquire one reusable native command from the selected queue's context-local pool.
		let mut command_buffer = context
			.queues
			.get_mut(queue_handle.0 as usize)
			.expect("Metal command queue is missing. The most likely cause is that the queue handle came from another context.")
			.acquire_native_command(label, context.settings.debug_labels);

		let device = RecordingDevice {
			metal_device: context.device.as_ref(),
			buffers: &context.buffers,
			images: &context.images,
			samplers: &context.samplers,
			acceleration_structures: &context.acceleration_structures,
			meshes: &context.meshes,
			pipelines: &context.pipelines,
			swapchains: &context.swapchains,
			frames: context.frames,
			debug_labels: context.settings.debug_labels,
		};
		let commit = RecordingCommit {
			queue_handle,
			queue: &mut context.queues[queue_handle.0 as usize],
			synchronizers: &mut context.synchronizers,
			texture_readbacks: &mut context.texture_readbacks,
			descriptor_sets: &mut context.descriptor_sets,
			upload_arena: &mut context.upload_arenas[arena_index],
			argument_tables: &mut context.argument_tables,
			image_groups: &mut context.image_groups,
		};

		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let mut resource_tracker = std::mem::take(&mut commit.queue.resource_tracker);
		resource_tracker.begin_recording();
		// Shared argument tables are snapshotted by every command that binds them, so retain them up front.
		for table in commit.argument_tables.iter().flatten() {
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
	/// Returns how many regions it pushed, which the encoder pops before it ends.
	#[cfg(debug_assertions)]
	fn begin_encoder_debug_regions(
		&self,
		encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder>,
		kind: &str,
		targets: impl IntoIterator<Item = ImageHandle>,
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
			let name = self.device.images.resource(target).name.as_deref().unwrap_or("Unnamed Image");
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
	/// unencoded. `kind` and `targets` label the encoder in capture tools.
	pub(super) fn begin_encoder(
		&mut self,
		encoder: ActiveEncoder,
		_kind: &str,
		_targets: impl IntoIterator<Item = ImageHandle>,
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
		// Each encoder gets its own hazard-tracking identity within the recording.
		let id = self.next_encoder_id;
		self.next_encoder_id = id.checked_add(1).expect(
			"Metal encoder identity overflowed. The most likely cause is that one command recording created more than u32::MAX encoders.",
		);
		self.encoder = Some(EncoderState {
			encoder,
			scope: synchronization::MetalEncoderScope::Encoder(id),
			pipeline: None,
			descriptors: None,
			bound_argument_tables: 0,
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
				.record_final(state.scope, &self.active_render_attachment_uses);
			self.active_render_attachment_uses.clear();
		}
	}

	/// Records render-target writes after a draw so a later aliased access sees the dependency.
	pub(super) fn record_render_attachment_writes(&mut self) {
		let scope = self.encoder_state().scope;
		self.resource_tracker.record_final(scope, &self.active_render_attachment_uses);
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

	/// Applies the dependencies one command needs on the active encoder and retains what it uses.
	///
	/// With `bind_descriptors`, the command also binds the active encoder's applied snapshot, whose uses are read in
	/// place. Every use in `additional_uses` has its native allocation retained here, so commands only retain objects
	/// hazard tracking never sees.
	pub(super) fn consume_resources_with_descriptors(
		&mut self,
		bind_descriptors: bool,
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let scope = self.encoder_state().scope;
		let Self {
			device,
			commit,
			command_buffer,
			resource_tracker,
			encoder,
			..
		} = self;
		let additional_uses = additional_uses
			.into_iter()
			.map(|resource_use| resource_use.in_group_memory(device.images))
			.collect::<SmallVec<[_; 8]>>();
		for resource_use in &additional_uses {
			retain_tracked_use(device, command_buffer, resource_use);
		}
		// The applied binding stays in the encoder state and is borrowed in place.
		let descriptors = if bind_descriptors {
			let binding = encoder.as_mut().and_then(|state| state.descriptors.as_mut()).expect(
				"Metal descriptors are missing. The most likely cause is that descriptor application did not retain its materialization.",
			);
			Some((binding.snapshot.uses(commit.descriptor_sets), &mut binding.settled))
		} else {
			None
		};
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
		self.consume_resources_with_descriptors(false, uses);
	}

	/// Publishes this finalized recording's resource history to its queue.
	pub(super) fn publish_resource_states(&mut self) {
		let recording = self.resource_tracker.finish_recording();
		self.command_buffer.set_tracked_recording(recording);
		self.commit.queue.resource_tracker = std::mem::take(&mut self.resource_tracker);
	}

	/// Updates one stage table and associates it with the active encoder before its next snapshot command.
	///
	/// Each stage's shared table is created on first use. An encoder keeps the table it was given, and draws and
	/// dispatches snapshot table contents when they are encoded, so each encoder is given each stage's table once.
	pub(super) fn set_stage_buffer_address(&mut self, stage: ArgumentTableStage, binding: u32, address: mtl::MTLGPUAddress) {
		assert!(
			(binding as usize) < ARGUMENT_TABLE_BUFFER_COUNT,
			"Metal argument-table buffer binding is out of range. The most likely cause is that a shader buffer index exceeded the fixed 17-buffer ABI. binding={binding}",
		);
		let Self {
			device,
			commit,
			command_buffer,
			encoder,
			..
		} = self;
		let table = commit.argument_tables[stage as usize].get_or_insert_with(|| {
			let descriptor = mtl::MTL4ArgumentTableDescriptor::new();
			descriptor.setMaxBufferBindCount(ARGUMENT_TABLE_BUFFER_COUNT);
			descriptor.setInitializeBindings(true);
			#[cfg(debug_assertions)]
			if device.debug_labels {
				descriptor.setLabel(Some(&NSString::from_str(stage.label())));
			}
			let table = device.metal_device.newArgumentTableWithDescriptor_error(&descriptor).expect(
				"Metal 4 argument table creation failed. The most likely cause is that the device ran out of binding-table memory.",
			);
			command_buffer.retain_object(&*table);
			table
		});
		// SAFETY: `binding` is checked against the fixed table size and `address` names a retained buffer.
		unsafe {
			table.setAddress_atIndex(address, binding as _);
		}

		let state = encoder
			.as_mut()
			.expect("No active Metal encoder. The most likely cause is that a command was recorded after its encoder ended.");
		let stage_bit = 1 << stage as u8;
		if state.bound_argument_tables & stage_bit != 0 {
			return;
		}
		match (stage, &state.encoder) {
			(ArgumentTableStage::Compute, ActiveEncoder::Compute(encoder)) => encoder.setArgumentTable(Some(table.as_ref())),
			(ArgumentTableStage::Compute, ActiveEncoder::Render(_)) => panic!(
				"No active Metal compute encoder. The most likely cause is that a compute table was updated outside dispatch preparation.",
			),
			(stage, ActiveEncoder::Render(encoder)) => encoder.setArgumentTable_atStages(table.as_ref(), stage.render_stage()),
			(_, ActiveEncoder::Compute(_)) => panic!(
				"No active Metal render encoder. The most likely cause is that a render table was updated outside a render pass.",
			),
		}
		state.bound_argument_tables |= stage_bit;
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
	/// Returns `None` for an unknown handle, or for a swapchain this frame sequence has not acquired at a nonzero extent.
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

	/// Returns the surface a swapchain renders into this frame, which is its image for the frame sequence.
	///
	/// The surface reports the swapchain's uses. Returns `None` in the same cases as [`Self::surface`].
	pub(super) fn swapchain_surface(&self, handle: crate::swapchain::SwapchainHandle) -> Option<Surface> {
		let swapchain = self.device.swapchains.get(handle.0 as usize)?;
		let image = swapchain.images[self.sequence_index as usize]?;
		Some(Surface {
			uses: swapchain.uses,
			..self.image_surface(image)
		})
	}

	/// Returns the surface of one frame-local image.
	fn image_surface(&self, handle: ImageHandle) -> Surface {
		let image = self.device.images.resource(handle);
		Surface {
			image: handle,
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
		// The logical push state is copied into an immutable range of the frame's upload arena.
		let (page, offset) = self
			.commit
			.upload_arena
			.upload(self.device.metal_device, &self.push_constant_data);
		let address = page.gpu_address.checked_add(offset as u64).expect(
			"Metal push upload GPU address overflowed. The most likely cause is an invalid buffer address or upload offset.",
		);
		self.command_buffer.retain_allocation(&*page.buffer);
		for stage in stages {
			self.set_stage_buffer_address(stage, PUSH_CONSTANT_BINDING_INDEX, address);
		}
		self.encoder_state_mut().push_constants_dirty = false;
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
/// Drawables need nothing here: only the present resolve writes one, and its command retains the drawable.
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
