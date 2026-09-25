use super::*;

impl<'a> CommandBufferRecording<'a> {
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
		self.command_buffer.retain_allocation(staging_buffer.clone());
		self.command_buffer.retain_allocation(destination_buffer.clone());
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

	pub(crate) fn new(
		device: RecordingDevice<'a>,
		commit: RecordingCommit<'a>,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
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
			command_buffer.retain_object(table.clone());
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
			drawables: Vec::new_in(allocator),
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
			active_render_extent: Extent::rectangle(0, 0),
			active_encoder_scope: None,
			next_encoder_id: 0,
			resource_tracker,
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

	/// Labels a new native encoder and mirrors every active logical debug region into it.
	///
	/// The label reads `<kind>: <region path> → <targets>`, so capture tools list what each encoder does and writes.
	/// A `None` target is a drawable. Returns how many regions it pushed, which the encoder pops before it ends.
	#[cfg(debug_assertions)]
	pub(super) fn begin_encoder_debug_regions<E: objc2::Message + ?Sized>(
		&mut self,
		encoder: &E,
		kind: &str,
		targets: impl IntoIterator<Item = Option<ImageHandle>>,
	) -> usize
	where
		dyn mtl::MTL4CommandEncoder: objc2::runtime::ImplementedBy<E>,
	{
		if !self.device.debug_labels {
			return 0;
		}
		let encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder> = ProtocolObject::from_ref(encoder);
		let mut label = crate::command_buffer::DebugLabelWriter::new();
		let _ = label.write_str(kind);
		for (index, region) in self.debug_regions.iter().enumerate() {
			let _ = label.write_str(if index == 0 { ": " } else { " › " });
			let _ = label.write_str(&region.to_string());
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
	fn signpost_barrier_hazards<E: objc2::Message + ?Sized>(&self, encoder: &E)
	where
		dyn mtl::MTL4CommandEncoder: objc2::runtime::ImplementedBy<E>,
	{
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
		let encoder: &ProtocolObject<dyn mtl::MTL4CommandEncoder> = ProtocolObject::from_ref(encoder);
		encoder.insertDebugSignpost(&NSString::from_str(label.as_str()));
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
			let encoder = self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			#[cfg(debug_assertions)]
			{
				self.compute_debug_region_depth = self.begin_encoder_debug_regions(&*encoder, "Compute", []);
			}
			self.active_compute_encoder = Some(encoder);
			self.active_encoder_scope = Some(self.allocate_encoder_scope());
			self.encoded_compute_pipeline = None;
			self.applied_compute_descriptor_binding = None;
			self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
		}

		self.active_compute_encoder.as_ref().unwrap()
	}

	/// Allocates one command-local identity for hazard tracking within a native encoder.
	pub(super) fn allocate_encoder_scope(&mut self) -> synchronization::MetalEncoderScope {
		let id = self.next_encoder_id;
		self.next_encoder_id = self.next_encoder_id.checked_add(1).expect(
			"Metal encoder identity overflowed. The most likely cause is that one command recording created more than u32::MAX encoders.",
		);
		synchronization::MetalEncoderScope::Encoder(id)
	}

	/// Applies the dependencies one command needs on the active encoder without copying its descriptor-use table.
	pub(super) fn consume_resources_with_descriptors(
		&mut self,
		descriptor_uses: &mut synchronization::DescriptorUses,
		additional_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let scope = self.active_encoder_scope.expect(
			"Metal resource tracking failed. The most likely cause is that a command consumed resources without an active encoder.",
		);
		let barrier = self
			.resource_tracker
			.consume_descriptors(scope, descriptor_uses, additional_uses);
		// Starting either encoder ends the other, so at most one is active.
		match (&self.active_compute_encoder, &self.active_render_encoder) {
			(Some(encoder), _) => {
				#[cfg(debug_assertions)]
				self.signpost_barrier_hazards(&**encoder);
				barrier.encode(&**encoder)
			}
			(None, Some(encoder)) => {
				#[cfg(debug_assertions)]
				self.signpost_barrier_hazards(&**encoder);
				barrier.encode(&**encoder)
			}
			(None, None) => unreachable!(
				"Metal resource tracking failed. The most likely cause is that the active encoder was ended before its resource barrier."
			),
		}
	}

	/// Applies only the queue and encoder dependencies required by the resources one command consumes.
	pub(super) fn consume_resources(&mut self, uses: impl IntoIterator<Item = synchronization::MetalResourceUse>) {
		self.consume_resources_with_descriptors(&mut synchronization::DescriptorUses::default(), uses);
	}

	/// Publishes this finalized recording's resource history to its queue.
	fn publish_resource_states(&mut self) {
		self.resource_tracker.finish_recording();
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
		self.command_buffer.retain_object(table.clone());
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

	/// Uploads the current logical push state into an immutable range of the frame's upload arena.
	fn upload_push_constants(&mut self) -> mtl::MTLGPUAddress {
		let (buffer, offset) = self
			.commit
			.upload_arena
			.upload(self.device.metal_device, &self.push_constant_data);
		let address = buffer.gpuAddress().checked_add(offset as u64).expect(
			"Metal push upload GPU address overflowed. The most likely cause is an invalid buffer address or upload offset.",
		);
		self.command_buffer.retain_allocation(buffer.clone());
		address
	}

	pub(super) fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		self.device.buffers.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	pub(super) fn get_internal_image_handle(&self, handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device.images.nth_handle(handle, self.sequence_index as _).unwrap()
	}

	/// Returns the proxy image a swapchain renders into this frame, or `None` when it renders to its drawable.
	pub(super) fn swapchain_proxy(&self, handle: crate::swapchain::SwapchainHandle) -> Option<ImageHandle> {
		self.device.swapchains[handle.0 as usize].images[self.sequence_index as usize]
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
		self.bound_descriptor_set_handles
			.iter()
			.find_map(|set_handle| self.commit.descriptor_sets[set_handle.0 as usize].descriptors.get(&slot))
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
			let left = &self.commit.descriptor_sets[left_handle.0 as usize];
			for right_handle in self.bound_descriptor_set_handles.iter().skip(left_index + 1) {
				let right = &self.commit.descriptor_sets[right_handle.0 as usize];

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
				let descriptor_set = &self.commit.descriptor_sets[set_handle.0 as usize];

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
					self.commit.descriptor_sets[set_handle.0 as usize]
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
			self.compute_push_constants_dirty = push_constant_size > 0;
			self.render_push_constants_dirty = push_constant_size > 0;
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

		for binding in 0..self.bound_vertex_buffers.len() {
			let (buffer_handle, offset) = self.bound_vertex_buffers[binding];
			let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
			let address = buffer.gpu_address.checked_add(offset as u64).expect(
				"Metal vertex buffer address overflowed. The most likely cause is that the requested vertex offset exceeds the native buffer address range.",
			);
			self.command_buffer.retain_allocation(buffer.buffer.clone());
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
