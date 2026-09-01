use super::*;

impl Device {
	/// Starts recording speculative resource states for one command buffer.
	pub(crate) fn begin_command_buffer_state_transaction(&mut self, command_buffer_handle: CommandBufferHandle) {
		debug_assert!(
			self.active_command_buffer.is_none() || self.active_command_buffer == Some(command_buffer_handle),
			"Only one DX12 command buffer can own mutable recording state at a time."
		);
		self.active_command_buffer = Some(command_buffer_handle);
	}

	/// Stops routing transitions to a command buffer while preserving them for submission.
	pub(crate) fn finish_command_buffer_state_transaction(&mut self, command_buffer_handle: CommandBufferHandle) {
		if self.active_command_buffer == Some(command_buffer_handle) {
			self.active_command_buffer = None;
		}
	}

	/// Commits resource states after the command list has entered the native queue.
	pub(crate) fn commit_command_buffer_resource_states(&mut self, command_buffer_handle: CommandBufferHandle) {
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.original_buffer_states.clear();
			command_buffer.original_image_states.clear();
		}
		self.finish_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Restores resource states when recorded barriers will never execute.
	pub(crate) fn rollback_command_buffer_resource_states(&mut self, command_buffer_handle: CommandBufferHandle) {
		self.present_transitions.remove(&command_buffer_handle);
		let (buffer_states, image_states) = self
			.command_buffers
			.get_mut(command_buffer_handle.0 as usize)
			.map(|command_buffer| {
				(
					std::mem::take(&mut command_buffer.original_buffer_states),
					std::mem::take(&mut command_buffer.original_image_states),
				)
			})
			.unwrap_or_default();
		for (key, state) in buffer_states {
			if let Some(state) = state {
				self.buffer_states.insert(key, state);
			} else {
				self.buffer_states.remove(&key);
			}
		}
		for (key, state) in image_states {
			if let Some(state) = state {
				self.image_states.insert(key, state);
			} else {
				self.image_states.remove(&key);
			}
		}
		self.finish_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Saves the committed buffer state before the active command buffer changes it for the first time.
	fn remember_buffer_state(&mut self, key: usize) {
		let Some(command_buffer_handle) = self.active_command_buffer else {
			return;
		};
		let original = self.buffer_states.get(&key).copied();
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		if command_buffer
			.original_buffer_states
			.iter()
			.all(|(recorded, _)| *recorded != key)
		{
			command_buffer.original_buffer_states.push((key, original));
		}
	}

	/// Saves the committed image state before the active command buffer changes it for the first time.
	fn remember_image_state(&mut self, key: usize) {
		let Some(command_buffer_handle) = self.active_command_buffer else {
			return;
		};
		let original = self.image_states.get(&key).copied();
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		if command_buffer
			.original_image_states
			.iter()
			.all(|(recorded, _)| *recorded != key)
		{
			command_buffer.original_image_states.push((key, original));
		}
	}

	/// Creates a whole-buffer enhanced barrier for one access transition.
	pub(crate) fn buffer_barrier(
		resource: &ID3D12Resource,
		before: BufferBarrierState,
		after: BufferBarrierState,
	) -> D3D12_BUFFER_BARRIER {
		D3D12_BUFFER_BARRIER {
			SyncBefore: before.sync,
			SyncAfter: after.sync,
			AccessBefore: before.access,
			AccessAfter: after.access,
			pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
			Offset: 0,
			Size: u64::MAX,
		}
	}

	/// Creates an all-subresource enhanced texture barrier for one access and layout transition.
	pub(crate) fn texture_barrier(
		resource: &ID3D12Resource,
		before: TextureBarrierState,
		after: TextureBarrierState,
	) -> D3D12_TEXTURE_BARRIER {
		D3D12_TEXTURE_BARRIER {
			SyncBefore: before.sync,
			SyncAfter: after.sync,
			AccessBefore: before.access,
			AccessAfter: after.access,
			LayoutBefore: before.layout,
			LayoutAfter: after.layout,
			pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
			Subresources: D3D12_BARRIER_SUBRESOURCE_RANGE {
				IndexOrFirstMipLevel: u32::MAX,
				..Default::default()
			},
			Flags: D3D12_TEXTURE_BARRIER_FLAG_NONE,
		}
	}

	/// Appends a swapchain texture transition while preserving its PRESENT boundary between submissions.
	pub(crate) fn transition_swapchain_texture_into(
		&mut self,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
		barriers: &mut EnhancedBarrierBatch,
	) {
		let key = Self::native_resource_key(resource);
		let before = self.image_states.get(&key).copied().unwrap_or(TextureBarrierState::PRESENT);
		if before == after {
			return;
		}
		barriers.push_texture(Self::texture_barrier(resource, before, after));
		self.remember_image_state(key);
		self.image_states.insert(key, after);
	}

	/// Records one tracked swapchain texture transition immediately.
	pub(crate) fn transition_swapchain_texture(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		self.transition_swapchain_texture_into(resource, after, &mut barriers);
		barriers.submit(command_list);
	}

	/// Submits one native call for a group of barriers that share a synchronization boundary.
	pub(crate) fn submit_resource_barriers(command_list: &ID3D12GraphicsCommandList7, barriers: &EnhancedBarrierBatch) {
		barriers.submit(command_list);
	}

	/// Uses native resource identity so dynamic frame allocations keep independent state histories.
	pub(crate) fn native_resource_key(resource: &ID3D12Resource) -> usize {
		resource.as_raw() as usize
	}

	/// Returns the enhanced access contract established when a buffer is created.
	pub(crate) fn initial_buffer_barrier_state(_heap_kind: BufferHeapKind) -> BufferBarrierState {
		// Enhanced barriers treat every buffer as COMMON at command-list boundaries, including upload and readback heaps.
		BufferBarrierState::COMMON
	}

	pub(crate) fn buffer_heap_kind_for_resource(
		&self,
		buffer_handle: BaseBufferHandle,
		resource: &ID3D12Resource,
	) -> Option<BufferHeapKind> {
		let key = Self::native_resource_key(resource);
		let buffer = self.buffer(buffer_handle)?;
		if buffer
			.resource
			.as_ref()
			.is_some_and(|resource| Self::native_resource_key(resource) == key)
		{
			return Some(buffer.heap_kind);
		}
		buffer.frame_resources.as_ref().and_then(|frame_resources| {
			frame_resources.iter().flatten().find_map(|frame_resource| {
				frame_resource
					.resource
					.as_ref()
					.is_some_and(|resource| Self::native_resource_key(resource) == key)
					.then_some(frame_resource.heap_kind)
			})
		})
	}

	pub(crate) fn transition_tracked_buffer(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: BufferBarrierState,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		self.transition_tracked_buffer_into(buffer, resource, after, &mut barriers);
		Self::submit_resource_barriers(command_list, &barriers);
	}

	/// Appends a tracked buffer transition to a caller-owned synchronization batch.
	pub(crate) fn transition_tracked_buffer_into(
		&mut self,
		buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: BufferBarrierState,
		barriers: &mut EnhancedBarrierBatch,
	) {
		let heap_kind = self
			.buffer_heap_kind_for_resource(buffer, resource)
			.unwrap_or(BufferHeapKind::Default);
		if heap_kind != BufferHeapKind::Default {
			return;
		}
		self.transition_buffer_resource_into(resource, after, barriers);
	}

	/// Appends a transition for a default-heap buffer that is not represented by a public buffer handle.
	pub(crate) fn transition_buffer_resource_into(
		&mut self,
		resource: &ID3D12Resource,
		after: BufferBarrierState,
		barriers: &mut EnhancedBarrierBatch,
	) {
		let key = Self::native_resource_key(resource);
		let before = self.buffer_states.get(&key).copied().unwrap_or(BufferBarrierState::COMMON);
		if before == after {
			if after.access == D3D12_BARRIER_ACCESS_UNORDERED_ACCESS {
				barriers.push_buffer(Self::buffer_barrier(resource, before, after));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push_buffer(Self::buffer_barrier(resource, before, after));
		self.remember_buffer_state(key);
		self.buffer_states.insert(key, after);
	}

	pub(crate) fn transition_tracked_image(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		self.transition_tracked_image_into(image, resource, after, &mut barriers);
		Self::submit_resource_barriers(command_list, &barriers);
	}

	/// Appends a tracked texture transition to a caller-owned synchronization batch.
	pub(crate) fn transition_tracked_image_into(
		&mut self,
		_image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
		barriers: &mut EnhancedBarrierBatch,
	) {
		let key = Self::native_resource_key(resource);
		let before = self.image_states.get(&key).copied().unwrap_or(TextureBarrierState::COMMON);
		if before == after {
			if after.access == D3D12_BARRIER_ACCESS_UNORDERED_ACCESS {
				barriers.push_texture(Self::texture_barrier(resource, before, after));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push_texture(Self::texture_barrier(resource, before, after));
		self.remember_image_state(key);
		self.image_states.insert(key, after);
	}

	/// Prepares a dedicated acceleration-structure buffer for a native build write.
	pub(crate) fn transition_acceleration_structure_for_build(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		resource: &ID3D12Resource,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		self.transition_buffer_resource_into(resource, BufferBarrierState::ACCELERATION_STRUCTURE_WRITE, &mut barriers);
		barriers.submit(command_list);
	}

	/// Makes a completed acceleration-structure build visible to subsequent builds and ray dispatches.
	pub(crate) fn complete_acceleration_structure_build(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		resource: &ID3D12Resource,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		// Build scratch and destination writes share the cache contract used by a legacy null UAV barrier.
		let raytracing_uav_sync = D3D12_BARRIER_SYNC_ALL_SHADING
			| D3D12_BARRIER_SYNC_BUILD_RAYTRACING_ACCELERATION_STRUCTURE
			| D3D12_BARRIER_SYNC_COPY_RAYTRACING_ACCELERATION_STRUCTURE
			| D3D12_BARRIER_SYNC_EMIT_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO;
		let raytracing_uav_access = D3D12_BARRIER_ACCESS_UNORDERED_ACCESS
			| D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_WRITE
			| D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_READ;
		barriers.push_global(D3D12_GLOBAL_BARRIER {
			SyncBefore: raytracing_uav_sync,
			SyncAfter: raytracing_uav_sync,
			AccessBefore: raytracing_uav_access,
			AccessAfter: raytracing_uav_access,
		});
		barriers.submit(command_list);
		let key = Self::native_resource_key(resource);
		self.remember_buffer_state(key);
		self.buffer_states
			.insert(key, BufferBarrierState::ACCELERATION_STRUCTURE_READ);
	}

	pub(crate) fn align_up(value: usize, alignment: usize) -> usize {
		value.div_ceil(alignment) * alignment
	}

	pub(crate) fn buffer_range_for_sequence(
		&self,
		buffer_handle: BaseBufferHandle,
		offset: usize,
		size: usize,
		sequence_index: u8,
	) -> Vec<u8> {
		let Some((data, buffer_size)) = self.buffer_storage_parts_for_sequence(buffer_handle, sequence_index) else {
			return Vec::new();
		};
		let end = offset.saturating_add(size);
		if end > buffer_size {
			panic!(
				"Failed to read DX12 buffer data. The most likely cause is that the requested range is outside the buffer allocation."
			);
		}
		if size == 0 {
			return Vec::new();
		}

		unsafe { std::slice::from_raw_parts(data.add(offset), size).to_vec() }
	}
}
