use super::*;

impl<'a> CommandBufferRecording<'a> {
	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		let buffer = self.device.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.device.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { &mut *(buffer.pointer as *mut T) }
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
		let blit_encoder = self.ensure_blit_encoder().clone();

		unsafe {
			blit_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
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
		commit: Option<RecordingCommit<'a>>,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		drawables: SmallVec<
			[(
				graphics_hardware_interface::SwapchainHandle,
				Retained<ProtocolObject<dyn CAMetalDrawable>>,
			); 4],
		>,
		autorelease_pool: Option<Retained<NSAutoreleasePool>>,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);

		Self {
			device,
			commit,
			command_buffer_handle,
			frame_key,
			sequence_index,
			command_buffer,
			#[cfg(debug_assertions)]
			debug_regions: SmallVec::new(),
			#[cfg(debug_assertions)]
			compute_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			render_debug_region_depth: 0,
			#[cfg(debug_assertions)]
			blit_debug_region_depth: 0,
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
			push_constant_data: SmallVec::new(),
			compute_push_constants_dirty: false,
			render_push_constants_dirty: false,
			active_compute_encoder: None,
			active_render_encoder: None,
			active_blit_encoder: None,
			encoded_compute_pipeline: None,
			encoded_render_pipeline: None,
			applied_compute_descriptor_binding: None,
			applied_render_descriptor_binding: None,
			compute_resident_bindings: SmallVec::new(),
			render_resident_bindings: SmallVec::new(),
			_autorelease_pool: autorelease_pool,
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	pub(super) fn push_active_compute_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLComputeCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.compute_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	pub(super) fn push_active_render_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLRenderCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.render_debug_region_depth = self.debug_regions.len();
		}
	}

	/// Mirrors every active logical region into a newly created native encoder.
	#[cfg(debug_assertions)]
	pub(super) fn push_active_blit_debug_regions(&mut self, encoder: &ProtocolObject<dyn mtl::MTLBlitCommandEncoder>) {
		if self.device.debug_labels {
			for region in &self.debug_regions {
				encoder.pushDebugGroup(region);
			}
			self.blit_debug_region_depth = self.debug_regions.len();
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
		self.encoded_compute_pipeline = None;
		self.applied_compute_descriptor_binding = None;
		self.compute_resident_bindings.clear();
		self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
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
		self.encoded_render_pipeline = None;
		self.applied_render_descriptor_binding = None;
		self.render_resident_bindings.clear();
		self.render_push_constants_dirty = !self.push_constant_data.is_empty();
		self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
		self.encoded_vertex_buffer_count = 0;
	}

	/// Ends the active blit encoder and balances its mirrored debug regions.
	pub(super) fn end_blit_encoder(&mut self) {
		let Some(encoder) = self.active_blit_encoder.take() else {
			return;
		};
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			for _ in 0..self.blit_debug_region_depth {
				encoder.popDebugGroup();
			}
			self.blit_debug_region_depth = 0;
		}
		encoder.endEncoding();
	}

	pub(super) fn ensure_blit_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLBlitCommandEncoder>> {
		self.end_compute_encoder();
		self.end_render_encoder();

		if self.active_blit_encoder.is_none() {
			let encoder = self.command_buffer.blitCommandEncoder().expect(
				"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
			);
			#[cfg(debug_assertions)]
			if self.device.debug_labels {
				encoder.setLabel(Some(&self.next_encoder_block_label()));
				self.push_active_blit_debug_regions(encoder.as_ref());
			}
			self.active_blit_encoder = Some(encoder);
		}

		self.active_blit_encoder.as_ref().unwrap()
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
		self.drawables.extend(drawables);
	}

	pub(crate) fn into_finished(mut self) -> FinishedCommandBuffer<'static> {
		self.end_render_encoder();
		self.end_compute_encoder();
		self.end_blit_encoder();

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer,
			_marker: std::marker::PhantomData,
		}
	}

	pub(super) fn ensure_compute_encoder(&mut self) -> &Retained<ProtocolObject<dyn mtl::MTLComputeCommandEncoder>> {
		self.end_render_encoder();
		self.end_blit_encoder();

		if self.active_compute_encoder.is_none() {
			// The ordinary Metal compute encoder is serial. Its dispatch order supplies inter-dispatch dependencies;
			// Metal explicitly ignores memoryBarrier calls unless a concurrent encoder is requested.
			let encoder = self.command_buffer.computeCommandEncoder().expect(
				"Metal compute command encoder creation failed. The most likely cause is that the command buffer could not start a compute pass.",
			);
			#[cfg(debug_assertions)]
			if self.device.debug_labels {
				encoder.setLabel(Some(&self.next_encoder_block_label()));
				self.push_active_compute_debug_regions(encoder.as_ref());
			}
			self.active_compute_encoder = Some(encoder);
			self.encoded_compute_pipeline = None;
			self.applied_compute_descriptor_binding = None;
			self.compute_resident_bindings.clear();
			self.compute_push_constants_dirty = !self.push_constant_data.is_empty();
		}

		self.active_compute_encoder.as_ref().unwrap()
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

	/// Applies changed logical vertex-buffer bindings once before the next ordinary draw.
	pub(super) fn apply_bound_vertex_buffers(&mut self) {
		if !self.render_vertex_buffers_dirty {
			return;
		}
		let Some(encoder) = self.active_render_encoder.as_ref() else {
			return;
		};

		let mut buffers = SmallVec::<[*const ProtocolObject<dyn mtl::MTLBuffer>; 8]>::new();
		let mut offsets = SmallVec::<[usize; 8]>::new();
		for (buffer_handle, offset) in self.bound_vertex_buffers.iter().copied() {
			let buffer = &self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
			buffers.push(buffer.buffer.as_ref());
			offsets.push(offset);
		}

		if !buffers.is_empty() {
			let buffers = NonNull::new(buffers.as_mut_ptr()).expect("A non-empty Metal vertex buffer list had a null pointer.");
			let offsets =
				NonNull::new(offsets.as_mut_ptr()).expect("A non-empty Metal vertex buffer offset list had a null pointer.");
			unsafe {
				encoder.setVertexBuffers_offsets_withRange(buffers, offsets, NSRange::new(0, self.bound_vertex_buffers.len()));
			}
		}
		for index in self.bound_vertex_buffers.len()..self.encoded_vertex_buffer_count {
			unsafe {
				encoder.setVertexBuffer_offset_atIndex(None, 0, index);
			}
		}
		self.encoded_vertex_buffer_count = self.bound_vertex_buffers.len();
		self.render_vertex_buffers_dirty = false;
	}

	/// Uploads changed push constants once before the next render command.
	pub(super) fn flush_render_push_constants(&mut self) {
		if !self.render_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pointer = NonNull::new(self.push_constant_data.as_ptr() as *mut std::ffi::c_void)
			.expect("Push constant data pointer was null. The most likely cause is an empty push constant buffer upload.");

		if let Some(encoder) = self.active_render_encoder.as_ref() {
			unsafe {
				encoder.setObjectBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setMeshBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setVertexBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
				encoder.setFragmentBytes_length_atIndex(
					pointer,
					self.push_constant_data.len() as _,
					PUSH_CONSTANT_BINDING_INDEX as _,
				);
			}
		}
		self.render_push_constants_dirty = false;
	}

	/// Uploads changed push constants once before the next compute dispatch.
	pub(super) fn flush_compute_push_constants(&mut self) {
		if !self.compute_push_constants_dirty || self.push_constant_data.is_empty() {
			return;
		}

		let pointer = NonNull::new(self.push_constant_data.as_ptr() as *mut std::ffi::c_void)
			.expect("Push constant data pointer was null. The most likely cause is an empty push constant buffer upload.");
		let push_constant_size = self.push_constant_data.len();
		unsafe {
			self.ensure_compute_encoder().setBytes_length_atIndex(
				pointer,
				push_constant_size as _,
				PUSH_CONSTANT_BINDING_INDEX as _,
			);
		}
		self.compute_push_constants_dirty = false;
	}

	pub(super) fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.end_compute_encoder();
		self.end_render_encoder();
		self.end_blit_encoder();

		if let Some(commit) = self.commit.as_mut() {
			let synchronizer = commit.synchronizer_for_sequence(synchronizer, self.sequence_index);
			// Retain the command buffer until a GHI wait observes completion.
			commit
				.synchronizers
				.resource(synchronizer)
				.signal_workload(self.command_buffer.clone());
		}

		device::submit_metal_command_buffer(self.command_buffer.as_ref());
	}
}
