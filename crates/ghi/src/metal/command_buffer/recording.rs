use super::*;

impl PushUploadArena<'_> {
	/// Copies one push-constant state into a unique aligned range and returns its GPU address.
	fn upload(
		&mut self,
		device: &ProtocolObject<dyn mtl::MTLDevice>,
		command: &mut queue::NativeCommand,
		bytes: &[u8],
	) -> mtl::MTLGPUAddress {
		assert!(
			!bytes.is_empty(),
			"Empty Metal push upload. The most likely cause is that a zero-sized push-constant layout was marked dirty."
		);

		let current_offset = self
			.pages
			.last()
			.and_then(|page| push_upload_offset(page.cursor, bytes.len(), page.buffer.length()));
		if current_offset.is_none() {
			let capacity =
				bytes.len().checked_add(PUSH_UPLOAD_ALIGNMENT - 1).expect(
					"Metal push upload size overflowed. The most likely cause is an invalid push-constant layout size.",
				) & !(PUSH_UPLOAD_ALIGNMENT - 1);
			let capacity = capacity.max(PUSH_UPLOAD_PAGE_SIZE);
			let buffer = device
				.newBufferWithLength_options(capacity, mtl::MTLResourceOptions::StorageModeShared)
				.expect(
					"Metal push upload allocation failed. The most likely cause is that the device is out of shared memory.",
				);
			command.retain_buffer(buffer.clone());
			self.pages.push(PushUploadPage { buffer, cursor: 0 });
		}

		let page = self.pages.last_mut().expect(
			"Missing Metal push upload page. The most likely cause is that page allocation did not update the command-local arena.",
		);
		let offset = push_upload_offset(page.cursor, bytes.len(), page.buffer.length()).expect(
			"Metal push upload range does not fit. The most likely cause is that the newly allocated page is smaller than the push-constant state.",
		);
		// SAFETY: `offset` was computed against this page's capacity and leaves `bytes.len()` writable bytes.
		let destination = unsafe { page.buffer.contents().as_ptr().cast::<u8>().add(offset) };
		// SAFETY: Caller bytes and the freshly allocated upload page do not overlap.
		unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
		page.cursor = offset + bytes.len();
		page.buffer.gpuAddress().checked_add(offset as u64).expect(
			"Metal push upload GPU address overflowed. The most likely cause is an invalid buffer address or upload offset.",
		)
	}
}

impl<'a> CommandBufferRecording<'a> {
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

		let staging = self.device.buffers.resource(staging_handle);
		let staging_buffer = staging.buffer.clone();
		let destination_buffer = buffer.buffer.clone();
		let destination_size = buffer.size;
		self.command_buffer.retain_buffer(staging_buffer.clone());
		self.command_buffer.retain_buffer(destination_buffer.clone());
		let transfer_encoder = self.prepare_transfer().clone();
		self.consume_compute_resources([
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

	pub(crate) fn new(
		device: RecordingDevice<'a>,
		commit: RecordingCommit<'a>,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		mut command_buffer: queue::NativeCommand,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		drawables: Vec<
			(
				graphics_hardware_interface::SwapchainHandle,
				Retained<ProtocolObject<dyn CAMetalDrawable>>,
			),
			&'a dyn std::alloc::Allocator,
		>,
		autorelease_pool: Option<Retained<NSAutoreleasePool>>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let mut resource_tracker = std::mem::take(&mut commit.queue.resource_tracker);
		resource_tracker.begin_recording();
		for (_, drawable) in &drawables {
			command_buffer.retain_drawable(drawable.clone());
		}

		Self {
			device,
			commit,
			command_buffer_handle,
			frame_key,
			sequence_index,
			command_buffer: NativeCommandSlot(Some(command_buffer)),
			#[cfg(debug_assertions)]
			debug_regions: Vec::new_in(allocator),
			#[cfg(debug_assertions)]
			compute_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			render_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			encoder_block_index: 0,
			drawables,
			active_pipeline_layout: None,
			bound_pipeline: None,
			bound_descriptor_set_roots: SmallVec::new(),
			bound_descriptor_set_handles: SmallVec::new(),
			bound_descriptor_set_versions: SmallVec::new(),
			bound_vertex_buffers: SmallVec::new(),
			render_vertex_buffers_dirty: false,
			encoded_vertex_buffer_count: 0,
			bound_index_buffer: None,
			push_constant_data: Vec::new_in(allocator),
			compute_push_constants_dirty: false,
			render_push_constants_dirty: false,
			active_compute_encoder: None,
			active_render_encoder: None,
			active_encoder_scope: None,
			next_encoder_id: 0,
			resource_tracker,
			argument_tables: CommandArgumentTables::default(),
			push_upload_arena: PushUploadArena::new_in(allocator),
			encoded_compute_pipeline: None,
			encoded_render_pipeline: None,
			applied_compute_descriptor_binding: None,
			applied_render_descriptor_binding: None,
			active_render_attachment_uses: SmallVec::new(),
			texture_readbacks: SmallVec::new(),
			readbacks_finalized: false,
			_autorelease_pool: autorelease_pool,
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	pub(super) fn push_active_compute_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.compute_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	pub(super) fn push_active_render_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.render_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Returns the next type-independent encoder label for this command buffer.
	#[cfg(debug_assertions)]
	pub(super) fn next_encoder_block_label(&mut self) -> Retained<NSString> {
		use std::fmt::Write as _;

		self.encoder_block_index += 1;
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		write!(label, "Block {}", self.encoder_block_index)
			.expect("Invalid encoder block label. The most likely cause is that the debug label writer rejected an integer.");
		NSString::from_str(label.as_str())
	}

	/// Ends the active compute encoder and resets state that is native-encoder-local.
	pub(super) fn end_compute_encoder(&mut self) {
		let Some(encoder) = self.active_compute_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.compute_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.compute_debug_region_depth = 0;
		}
		encoder.endEncoding();
		self.active_encoder_scope = None;
		self.encoded_compute_pipeline = None;
		self.applied_compute_descriptor_binding = None;
		self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
	}

	/// Records render-target writes after a draw so a later aliased access sees the dependency.
	pub(super) fn record_render_attachment_writes(&mut self) {
		let scope = self.active_encoder_scope.expect(
			"Metal render resource finalization failed. The most likely cause is that attachment writes were recorded without an active encoder.",
		);
		self.resource_tracker
			.record_final(scope, self.active_render_attachment_uses.iter().copied());
	}

	/// Ends the active render encoder and balances its mirrored debug regions.
	pub(super) fn end_render_encoder(&mut self) {
		let Some(encoder) = self.active_render_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.render_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.render_debug_region_depth = 0;
		}
		encoder.endEncoding();
		self.record_render_attachment_writes();
		self.active_render_attachment_uses.clear();
		self.active_encoder_scope = None;
		self.encoded_render_pipeline = None;
		self.applied_render_descriptor_binding = None;
		self.render_push_constants_dirty = !self.push_constant_data.is_empty();
		self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
		self.encoded_vertex_buffer_count = 0;
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
			self.command_buffer.retain_drawable(drawable.clone());
			self.drawables.push((handle, drawable));
		}
	}

	pub(crate) fn into_finished(mut self) -> FinishedCommandBuffer<'static> {
		self.end_render_encoder();
		self.end_compute_encoder();
		self.publish_resource_states();
		self.readbacks_finalized = true;

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer.take(),
			texture_readbacks: std::mem::take(&mut self.texture_readbacks),
			_marker: std::marker::PhantomData,
		}
	}

	pub(super) fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>> {
		self.end_render_encoder();

		if self.active_compute_encoder.is_none() {
			// One serial MTL4 compute encoder records both copy and dispatch commands. Phase transitions add explicit visibility.
			let encoder = self.command_buffer.compute_command_encoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			#[cfg(debug_assertions)]
			if self.device.debug_labels {
				encoder.setLabel(Some(&self.next_encoder_block_label()));
				self.push_active_compute_debug_regions(encoder.as_ref());
			}
			self.active_compute_encoder = Some(encoder);
			self.active_encoder_scope = Some(self.allocate_encoder_scope());
			self.encoded_compute_pipeline = None;
			self.applied_compute_descriptor_binding = None;
			self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
		}

		self.active_compute_encoder.as_ref().unwrap()
	}

	/// Prepares the combined Metal 4 compute encoder for transfer commands.
	pub(super) fn prepare_transfer(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>> {
		self.ensure_compute_encoder()
	}

	/// Allocates one command-local identity for hazard tracking within a native encoder.
	pub(super) fn allocate_encoder_scope(&mut self) -> synchronization::MetalEncoderScope {
		let id = self.next_encoder_id;
		self.next_encoder_id = self.next_encoder_id.checked_add(1).expect(
			"Metal encoder identity overflowed. The most likely cause is that one command recording created more than u32::MAX encoders.",
		);
		synchronization::MetalEncoderScope::Encoder(id)
	}

	/// Applies dependencies for one compute command without copying its immutable descriptor-use table.
	pub(super) fn consume_compute_resources_with_descriptors(
		&mut self,
		descriptor_uses: &[synchronization::MetalResourceUse],
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let scope = self.active_encoder_scope.expect(
			"Metal compute resource tracking failed. The most likely cause is that a command consumed resources without an active compute encoder.",
		);
		let barrier = self
			.resource_tracker
			.consume_preconsolidated(scope, descriptor_uses, additional_uses);
		let encoder = self.active_compute_encoder.as_ref().expect(
			"Metal compute resource tracking failed. The most likely cause is that the active encoder was ended before its resource barrier.",
		);
		barrier.encode_compute(encoder.as_ref());
	}

	/// Applies only the queue and encoder dependencies required by the resources one compute command consumes.
	pub(super) fn consume_compute_resources(&mut self, uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.consume_compute_resources_with_descriptors(&[], uses);
	}

	/// Applies dependencies for one render command without copying its immutable descriptor-use table.
	pub(super) fn consume_render_resources_with_descriptors(
		&mut self,
		descriptor_uses: &[synchronization::MetalResourceUse],
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let scope = self.active_encoder_scope.expect(
			"Metal render resource tracking failed. The most likely cause is that a command consumed resources without an active render encoder.",
		);
		let barrier = self
			.resource_tracker
			.consume_preconsolidated(scope, descriptor_uses, additional_uses);
		let encoder = self.active_render_encoder.as_ref().expect(
			"Metal render resource tracking failed. The most likely cause is that the active encoder was ended before its resource barrier.",
		);
		barrier.encode_render(encoder.as_ref());
	}

	/// Applies only the queue and encoder dependencies required by the resources one render command consumes.
	pub(super) fn consume_render_resources(&mut self, uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.consume_render_resources_with_descriptors(&[], uses);
	}

	/// Publishes this finalized recording's resource history to its queue.
	fn publish_resource_states(&mut self) {
		self.resource_tracker.finish_recording();
		self.commit.queue.resource_tracker = std::mem::take(&mut self.resource_tracker);
	}

	/// Creates one initialized Metal 4 argument table when a shader stage first needs bindings.
	pub(super) fn argument_table(&mut self, stage: ArgumentTableStage) -> Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>> {
		if let Some(table) = self.argument_tables.get(stage) {
			return table.clone();
		}

		let descriptor = mtl::MTL4ArgumentTableDescriptor::new();
		descriptor.setMaxBufferBindCount(ARGUMENT_TABLE_BUFFER_COUNT);
		descriptor.setInitializeBindings(true);
		let table = self.device.metal_device.newArgumentTableWithDescriptor_error(&descriptor);
		let table = table.expect(
			"Metal 4 argument table creation failed. The most likely cause is that the device ran out of binding-table memory.",
		);
		self.command_buffer.retain_argument_table(table.clone());
		self.argument_tables.insert(stage, table.clone());
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

		match stage {
			ArgumentTableStage::Compute => self
				.active_compute_encoder
				.as_ref()
				.expect(
					"No active Metal compute encoder. The most likely cause is that a compute table was updated outside dispatch preparation.",
				)
				.setArgumentTable(Some(table.as_ref())),
			stage => self
				.active_render_encoder
				.as_ref()
				.expect(
					"No active Metal render encoder. The most likely cause is that a render table was updated outside a render pass.",
				)
				.setArgumentTable_atStages(table.as_ref(), stage.render_stage()),
		}
	}

	/// Uploads the current logical push state into an immutable command-local range.
	fn upload_push_constants(&mut self) -> mtl::MTLGPUAddress {
		self.push_upload_arena
			.upload(self.device.metal_device, &mut self.command_buffer, &self.push_constant_data)
	}

	pub(super) fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	pub(super) fn get_internal_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device.images.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	/// Returns the acquired drawable texture for a direct swapchain.
	pub(super) fn drawable_texture(
		&self,
		handle: crate::swapchain::SwapchainHandle,
	) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
		self.drawables
			.iter()
			.find(|(swapchain, _)| swapchain.0 == handle.0)
			.map(|(_, drawable)| drawable.texture())
			.expect(
				"Missing Metal drawable. The most likely cause is that a direct swapchain was used before its frame image was acquired.",
			)
	}

	pub(super) fn descriptors_at_slot(&self, slot: crate::shader::ResourceSlot) -> Option<&HashMap<u32, Descriptor>> {
		self.descriptors_at_slot_with_owner(slot).map(|(_, descriptors)| descriptors)
	}

	pub(super) fn descriptors_at_slot_with_owner(
		&self,
		slot: crate::shader::ResourceSlot,
	) -> Option<(DescriptorSetHandle, &HashMap<u32, Descriptor>)> {
		self.bound_descriptor_set_handles.iter().find_map(|set_handle| {
			self.device.descriptor_sets[set_handle.0 as usize]
				.descriptors
				.get(&slot)
				.map(|descriptors| (*set_handle, descriptors))
		})
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
	pub(super) fn validate_bound_descriptor_sets(&self, layout: &PipelineLayout) {
		for (left_index, left_handle) in self.bound_descriptor_set_handles.iter().enumerate() {
			let left = &self.device.descriptor_sets[left_handle.0 as usize];
			for right_handle in self.bound_descriptor_set_handles.iter().skip(left_index + 1) {
				let right = &self.device.descriptor_sets[right_handle.0 as usize];

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
			for set_handle in &self.bound_descriptor_set_handles {
				let descriptor_set = &self.device.descriptor_sets[set_handle.0 as usize];

				assert!(
					descriptor_set
						.descriptors
						.keys()
						.all(|slot| resource_accepts_retained_slot_key(descriptor, *slot)),
					"Invalid retained descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array's base slot.",
				);
			}
			let owner_count = self
				.bound_descriptor_set_handles
				.iter()
				.filter(|set_handle| {
					self.device.descriptor_sets[set_handle.0 as usize]
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

	pub(super) fn resize_push_constants_for_layout(
		&mut self,
		pipeline_layout: graphics_hardware_interface::PipelineLayoutHandle,
	) {
		let push_constant_size = self.device.pipeline_layouts[pipeline_layout.0 as usize].push_constant_size;
		self.push_constant_data.clear();
		self.push_constant_data.resize(push_constant_size, 0);
		self.compute_push_constants_dirty = push_constant_size > 0;
		self.render_push_constants_dirty = push_constant_size > 0;
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

		for binding in 0..self.bound_vertex_buffers.len() {
			let (buffer_handle, offset) = self.bound_vertex_buffers[binding];
			let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
			let address = buffer.gpu_address.checked_add(offset as u64).expect(
				"Metal vertex buffer address overflowed. The most likely cause is that the requested vertex offset exceeds the native buffer address range.",
			);
			self.command_buffer.retain_buffer(buffer.buffer.clone());
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, address);
		}
		for binding in self.bound_vertex_buffers.len()..self.encoded_vertex_buffer_count {
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, 0);
		}
		self.encoded_vertex_buffer_count = self.bound_vertex_buffers.len();
		self.render_vertex_buffers_dirty = false;
	}

	/// Uploads changed push constants once before the next render command.
	pub(super) fn flush_render_push_constants(&mut self) {
		if !self.render_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pipeline_handle = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that render push constants were flushed before binding a pipeline.",
		);
		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];
		let uses_mesh = pipeline.mesh_threadgroup_size.is_some();
		let uses_object = pipeline.object_threadgroup_size.is_some();
		let address = self.upload_push_constants();
		if uses_mesh {
			if uses_object {
				self.set_stage_buffer_address(ArgumentTableStage::Object, PUSH_CONSTANT_BINDING_INDEX, address);
			}
			self.set_stage_buffer_address(ArgumentTableStage::Mesh, PUSH_CONSTANT_BINDING_INDEX, address);
		} else {
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, PUSH_CONSTANT_BINDING_INDEX, address);
		}
		self.set_stage_buffer_address(ArgumentTableStage::Fragment, PUSH_CONSTANT_BINDING_INDEX, address);
		self.render_push_constants_dirty = false;
	}

	/// Uploads changed push constants once before the next compute dispatch.
	pub(super) fn flush_compute_push_constants(&mut self) {
		if !self.compute_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let address = self.upload_push_constants();
		self.set_stage_buffer_address(ArgumentTableStage::Compute, PUSH_CONSTANT_BINDING_INDEX, address);
		self.compute_push_constants_dirty = false;
	}

	/// Ends and submits a non-frame recording as a one-command Metal 4 batch.
	pub(super) fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.end_compute_encoder();
		self.end_render_encoder();
		self.publish_resource_states();
		for handle in &self.texture_readbacks {
			self.commit.texture_readbacks.mark_submitted(*handle);
		}
		self.readbacks_finalized = true;

		let synchronizer = self.commit.synchronizer_for_sequence(synchronizer, self.sequence_index);
		let commands = SmallVec::<[queue::NativeCommand; 4]>::from_iter([self.command_buffer.take()]);
		let submitted = self.commit.queue.submit_batch(self.commit.queue_handle, commands);
		// The synchronizer owns the submitted batch until its completion message arrives.
		self.commit.synchronizers.resource_mut(synchronizer).signal(submitted);
	}
}
