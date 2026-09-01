use super::*;

impl Device {
	pub(crate) unsafe fn transition_resource(
		command_list: &ID3D12GraphicsCommandList,
		resource: &ID3D12Resource,
		before: D3D12_RESOURCE_STATES,
		after: D3D12_RESOURCE_STATES,
	) {
		unsafe {
			let barrier = Self::transition_resource_barrier(resource, before, after);
			Self::submit_resource_barriers(command_list, &[barrier]);
		}
	}

	/// Creates a transition barrier so callers can submit independent resource transitions together.
	pub(crate) fn transition_resource_barrier(
		resource: &ID3D12Resource,
		before: D3D12_RESOURCE_STATES,
		after: D3D12_RESOURCE_STATES,
	) -> D3D12_RESOURCE_BARRIER {
		D3D12_RESOURCE_BARRIER {
			Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
			Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
			Anonymous: D3D12_RESOURCE_BARRIER_0 {
				Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
					pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
					Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
					StateBefore: before,
					StateAfter: after,
				}),
			},
		}
	}

	/// Submits one native call for a group of barriers that share a synchronization boundary.
	pub(crate) unsafe fn submit_resource_barriers(
		command_list: &ID3D12GraphicsCommandList,
		barriers: &[D3D12_RESOURCE_BARRIER],
	) {
		unsafe {
			if !barriers.is_empty() {
				command_list.ResourceBarrier(barriers);
			}
		}
	}

	pub(crate) unsafe fn unordered_access_barrier(command_list: &ID3D12GraphicsCommandList, resource: &ID3D12Resource) {
		unsafe {
			let barrier = Self::unordered_access_resource_barrier(resource);
			Self::submit_resource_barriers(command_list, &[barrier]);
		}
	}

	/// Creates a resource-specific UAV barrier for a caller-owned synchronization batch.
	pub(crate) fn unordered_access_resource_barrier(resource: &ID3D12Resource) -> D3D12_RESOURCE_BARRIER {
		D3D12_RESOURCE_BARRIER {
			Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
			Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
			Anonymous: D3D12_RESOURCE_BARRIER_0 {
				UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
					pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
				}),
			},
		}
	}

	pub(crate) unsafe fn unordered_access_barrier_all(command_list: &ID3D12GraphicsCommandList) {
		unsafe {
			let barrier = D3D12_RESOURCE_BARRIER {
				Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
				Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
				Anonymous: D3D12_RESOURCE_BARRIER_0 {
					UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
						pResource: std::mem::ManuallyDrop::new(None),
					}),
				},
			};
			command_list.ResourceBarrier(&[barrier]);
		}
	}

	/// Uses native resource identity so dynamic frame allocations keep independent state histories.
	pub(crate) fn native_resource_key(resource: &ID3D12Resource) -> usize {
		resource.as_raw() as usize
	}

	pub(crate) fn initial_buffer_resource_state(heap_kind: BufferHeapKind) -> D3D12_RESOURCE_STATES {
		match heap_kind {
			BufferHeapKind::Upload => D3D12_RESOURCE_STATE_GENERIC_READ,
			BufferHeapKind::Readback => D3D12_RESOURCE_STATE_COPY_DEST,
			BufferHeapKind::Default => D3D12_RESOURCE_STATE_COMMON,
		}
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

	pub(crate) unsafe fn transition_tracked_buffer(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		_buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		unsafe {
			let mut barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
			self.transition_tracked_buffer_into(_buffer, resource, after, &mut barriers);
			Self::submit_resource_barriers(command_list, &barriers);
		}
	}

	/// Appends a tracked buffer transition to a caller-owned synchronization batch.
	pub(crate) unsafe fn transition_tracked_buffer_into(
		&mut self,
		_buffer: BaseBufferHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
		barriers: &mut SmallVec<[D3D12_RESOURCE_BARRIER; 32]>,
	) {
		let key = Self::native_resource_key(resource);
		let heap_kind = self
			.buffer_heap_kind_for_resource(_buffer, resource)
			.unwrap_or(BufferHeapKind::Default);
		if heap_kind != BufferHeapKind::Default {
			self.buffer_states
				.entry(key)
				.or_insert_with(|| Self::initial_buffer_resource_state(heap_kind));
			return;
		}
		let before = self
			.buffer_states
			.get(&key)
			.copied()
			.unwrap_or_else(|| Self::initial_buffer_resource_state(heap_kind));
		if before == after {
			if after == D3D12_RESOURCE_STATE_UNORDERED_ACCESS {
				barriers.push(Self::unordered_access_resource_barrier(resource));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push(Self::transition_resource_barrier(resource, before, after));
		self.buffer_states.insert(key, after);
	}

	pub(crate) unsafe fn transition_tracked_image(
		&mut self,
		command_list: &ID3D12GraphicsCommandList,
		_image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
	) {
		unsafe {
			let mut barriers = SmallVec::<[D3D12_RESOURCE_BARRIER; 32]>::new();
			self.transition_tracked_image_into(_image, resource, after, &mut barriers);
			Self::submit_resource_barriers(command_list, &barriers);
		}
	}

	/// Appends a tracked image transition to a caller-owned synchronization batch.
	pub(crate) unsafe fn transition_tracked_image_into(
		&mut self,
		_image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: D3D12_RESOURCE_STATES,
		barriers: &mut SmallVec<[D3D12_RESOURCE_BARRIER; 32]>,
	) {
		let key = Self::native_resource_key(resource);
		let before = self.image_states.get(&key).copied().unwrap_or(D3D12_RESOURCE_STATE_COMMON);
		if before == after {
			if after == D3D12_RESOURCE_STATE_UNORDERED_ACCESS {
				barriers.push(Self::unordered_access_resource_barrier(resource));
				self.uav_barrier_count += 1;
			}
			return;
		}
		barriers.push(Self::transition_resource_barrier(resource, before, after));
		self.image_states.insert(key, after);
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
