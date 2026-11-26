use super::*;

impl Device {
	/// Discards every unsubmitted side effect owned by one command-buffer recording.
	pub(crate) fn discard_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) {
		let index = command_buffer_handle.index() as usize;
		if !matches!(
			self.command_buffers[index].lifecycle,
			CommandBufferLifecycle::Recording | CommandBufferLifecycle::Scheduled
		) {
			return;
		}
		self.abandon_texture_readbacks_for_command_buffer(command_buffer_handle);
		self.rollback_command_buffer_resource_states(command_buffer_handle);
		self.abandon_recorded_builds(command_buffer_handle);
		self.command_buffers[index].lifecycle = CommandBufferLifecycle::Idle;
	}

	/// Rejects command buffers that cannot join one queue execution before recording mutates their native state.
	pub(crate) fn validate_command_buffer_for_execution(
		&self,
		command_buffer_handle: CommandBufferHandle,
		queue_handle: QueueHandle,
	) {
		let command_buffer = &self.command_buffers[command_buffer_handle.index() as usize];
		assert_eq!(
			command_buffer.queue_handle, queue_handle,
			"Invalid DX12 execution queue. The most likely cause is that the command buffer was created by a different queue."
		);
		assert!(
			matches!(
				command_buffer.lifecycle,
				CommandBufferLifecycle::Idle | CommandBufferLifecycle::Submitted
			),
			"DX12 command buffer is already recording or scheduled. The most likely cause is that one execution recorded the same handle more than once."
		);
	}

	/// Starts recording speculative resource states for one command buffer.
	pub(crate) fn begin_command_buffer_state_transaction(&mut self, command_buffer_handle: CommandBufferHandle) {
		debug_assert!(
			self.active_command_buffer.is_none() || self.active_command_buffer == Some(command_buffer_handle),
			"Only one DX12 command buffer can own mutable recording state at a time."
		);
		self.active_command_buffer = Some(command_buffer_handle);
		self.unordered_writes.clear();
	}

	/// Stops routing transitions to a command buffer while preserving them for submission.
	pub(crate) fn finish_command_buffer_state_transaction(&mut self, command_buffer_handle: CommandBufferHandle) {
		if self.active_command_buffer == Some(command_buffer_handle) {
			self.active_command_buffer = None;
		}
	}

	/// Commits resource states after the command list has entered the native queue.
	pub(crate) fn commit_command_buffer_resource_states(&mut self, command_buffer_handle: CommandBufferHandle) {
		let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
		command_buffer.original_buffer_states.clear();
		command_buffer.original_image_states.clear();
		self.finish_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Restores resource states when recorded barriers will never execute.
	pub(crate) fn rollback_command_buffer_resource_states(&mut self, command_buffer_handle: CommandBufferHandle) {
		/// Puts each saved state back, or forgets the key when it had none.
		fn restore<S>(states: &mut HashMap<usize, S>, originals: impl IntoIterator<Item = (usize, Option<S>)>) {
			for (key, state) in originals {
				match state {
					Some(state) => states.insert(key, state),
					None => states.remove(&key),
				};
			}
		}

		let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
		command_buffer.present_resources.clear();
		restore(&mut self.buffer_states, command_buffer.original_buffer_states.drain(..));
		restore(&mut self.image_states, command_buffer.original_image_states.drain(..));
		self.finish_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Saves the committed state of `key` in `states` into `originals` before the active command buffer changes it for
	/// the first time.
	fn remember_state<S: Copy>(states: &HashMap<usize, S>, originals: &mut SmallVec<[(usize, Option<S>); 16]>, key: usize) {
		if originals.iter().all(|(recorded, _)| *recorded != key) {
			originals.push((key, states.get(&key).copied()));
		}
	}

	/// Saves the committed buffer state before the active command buffer changes it for the first time.
	fn remember_buffer_state(&mut self, key: usize) {
		if let Some(handle) = self.active_command_buffer {
			let originals = &mut self.command_buffers[handle.index() as usize].original_buffer_states;
			Self::remember_state(&self.buffer_states, originals, key);
		}
	}

	/// Saves the committed image state before the active command buffer changes it for the first time.
	fn remember_image_state(&mut self, key: usize) {
		if let Some(handle) = self.active_command_buffer {
			let originals = &mut self.command_buffers[handle.index() as usize].original_image_states;
			Self::remember_state(&self.image_states, originals, key);
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
			// Leaving the undefined layout discards the contents, which also initializes a texture placed in a heap.
			Flags: if before.layout == D3D12_BARRIER_LAYOUT_UNDEFINED {
				D3D12_TEXTURE_BARRIER_FLAG_DISCARD
			} else {
				D3D12_TEXTURE_BARRIER_FLAG_NONE
			},
		}
	}

	/// Gives an image new contents. Returns `false` when `image` belongs to no image group.
	///
	/// An image whose memory nothing else wrote since its last use keeps its state, so its next barrier waits for that
	/// use. Otherwise, as for an image-group member whose memory other members reused, the next barrier moves the image
	/// out of the undefined layout, discarding its contents, after deactivating those members: each deactivation flushes
	/// the member's writes and holds back the image's first use until the member's work finishes. D3D12 global barriers
	/// can't do this, because accesses such as depth writes belong to one layout.
	pub(crate) fn initialize_image_contents(&mut self, image: crate::BaseImageHandle, sequence_index: u8) -> bool {
		let overwritten = self.image_groups.initialize(image);
		let member = overwritten.is_some();
		let Some(resource) = self.ensure_image_resource_for_sequence(image, sequence_index) else {
			return member;
		};
		let key = Self::native_resource_key(&resource);
		let is_active = |state: &TextureBarrierState| state.access != D3D12_BARRIER_ACCESS_NO_ACCESS;
		let reused = overwritten
			.into_iter()
			.flatten()
			.filter_map(|other| self.images.get(other.index() as usize)?.resource.clone())
			.filter(|other| {
				let other_key = Self::native_resource_key(other);
				other_key != key && self.image_states.get(&other_key).is_some_and(is_active)
			})
			.collect::<SmallVec<[ID3D12Resource; 4]>>();
		let own = self.image_states.get(&key).copied();

		if !own.is_some_and(|state| is_active(&state)) {
			// D3D12 pairs the undefined layout with no access. The image keeps the synchronization its own deactivation
			// left it, since a barrier after one that ended in no synchronization must start from none.
			let sync = own.map_or(D3D12_BARRIER_SYNC_NONE, |state| state.sync);
			self.remember_image_state(key);
			self.image_states.insert(
				key,
				TextureBarrierState::new(sync, D3D12_BARRIER_ACCESS_NO_ACCESS, D3D12_BARRIER_LAYOUT_UNDEFINED),
			);
		}
		if reused.is_empty() {
			self.image_deactivations.remove(&key);
		} else {
			self.image_deactivations.insert(key, reused);
		}
		member
	}

	/// Appends a swapchain texture transition while preserving its PRESENT boundary between submissions.
	pub(crate) fn transition_swapchain_texture_into(
		&mut self,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
		barriers: &mut EnhancedBarrierBatch,
	) -> bool {
		if let Some(command_buffer) = self.active_command_buffer {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		let key = Self::native_resource_key(resource);
		let before = self.image_states.get(&key).copied().unwrap_or(TextureBarrierState::PRESENT);
		if before == after {
			return false;
		}
		barriers.texture.push(Self::texture_barrier(resource, before, after));
		self.remember_image_state(key);
		self.image_states.insert(key, after);
		true
	}

	/// Uses native resource identity so dynamic frame allocations keep independent state histories.
	pub(crate) fn native_resource_key(resource: &ID3D12Resource) -> usize {
		resource.as_raw() as usize
	}

	/// Notes one transition of `key` in the open unordered group and returns whether the group already transitioned
	/// it, so the transition needs no UAV barrier. Always `false` outside a group.
	fn unordered_group_covers(&mut self, key: usize) -> bool {
		let Some(group) = &mut self.unordered_group else {
			return false;
		};
		if group.contains(&key) {
			return true;
		}
		group.push(key);
		false
	}

	/// Decides whether a use of `key` in unordered access, whose state doesn't change, needs a UAV barrier, and notes the
	/// use. `writes` tells whether the use writes. Only a write on either side is a hazard: before a read, a barrier
	/// makes the earlier write visible, and before a write, it keeps earlier reads from seeing the new data.
	fn unordered_use_needs_barrier(&mut self, key: usize, writes: bool) -> bool {
		let ordered_by_group = self.unordered_group_covers(key);
		let wrote = self.unordered_writes.get(&key).copied().unwrap_or(true);
		let barrier = !ordered_by_group && (wrote || writes);
		// A barrier orders everything before it, so only this use remains unordered after one.
		self.unordered_writes
			.insert(key, if barrier { writes } else { wrote || writes });
		barrier
	}

	pub(crate) fn transition_tracked_buffer(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		resource: &ID3D12Resource,
		after: BufferBarrierState,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		self.transition_tracked_buffer_into(resource, after, true, &mut barriers);
		barriers.submit(command_list);
	}

	/// Appends a tracked buffer transition to a caller-owned synchronization batch. `writes` tells whether the use
	/// may write the buffer, which decides whether a repeated unordered-access use needs a UAV barrier.
	pub(crate) fn transition_tracked_buffer_into(
		&mut self,
		resource: &ID3D12Resource,
		after: BufferBarrierState,
		writes: bool,
		barriers: &mut EnhancedBarrierBatch,
	) {
		if let Some(command_buffer) = self.active_command_buffer {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		let key = Self::native_resource_key(resource);
		let before = self.buffer_states.get(&key).copied().unwrap_or(BufferBarrierState::COMMON);
		if before == after {
			if after.access == D3D12_BARRIER_ACCESS_UNORDERED_ACCESS && self.unordered_use_needs_barrier(key, writes) {
				barriers.buffer.push(Self::buffer_barrier(resource, before, after));
			}
			return;
		}
		// The transition orders the buffer's earlier work, so only this use remains unordered.
		self.unordered_group_covers(key);
		self.unordered_writes.insert(key, writes);
		barriers.buffer.push(Self::buffer_barrier(resource, before, after));
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
		self.transition_tracked_image_into(image, resource, after, true, &mut barriers);
		barriers.submit(command_list);
	}

	/// Appends a tracked texture transition to a caller-owned synchronization batch. `writes` tells whether the use
	/// may write the image, which decides whether a repeated unordered-access use needs a UAV barrier.
	pub(crate) fn transition_tracked_image_into(
		&mut self,
		image: crate::BaseImageHandle,
		resource: &ID3D12Resource,
		after: TextureBarrierState,
		writes: bool,
		barriers: &mut EnhancedBarrierBatch,
	) {
		self.image_groups.assert_initialized(image, || None);
		debug_assert!(
			after.access.0 & D3D12_BARRIER_ACCESS_UNORDERED_ACCESS.0 == 0
				// SAFETY: The resource is live for the call.
				|| unsafe { resource.GetDesc() }.Flags.contains(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
			"DX12 image {image:?} moves into unordered access, which it was created without. The most likely cause is that a storage view or a clear names an image built without storage usage. Build the image with `Uses::Storage`.",
		);
		if let Some(command_buffer) = self.active_command_buffer {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		let key = Self::native_resource_key(resource);
		for other in self.image_deactivations.remove(&key).into_iter().flatten() {
			let other_key = Self::native_resource_key(&other);
			let Some(before) = self.image_states.get(&other_key).copied() else {
				continue;
			};
			// Ending in no access flushes the member's writes, and ending at the image's first synchronization makes that
			// use wait for the member's work. D3D12 takes the undefined layout for such a barrier, which suits memory the
			// image now overwrites.
			// A UAV clear's synchronization only pairs with unordered access, so a deactivation waits for every stage.
			let sync = if after.sync.0 & D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW.0 != 0 {
				D3D12_BARRIER_SYNC_ALL
			} else {
				after.sync
			};
			let deactivated = TextureBarrierState::new(sync, D3D12_BARRIER_ACCESS_NO_ACCESS, D3D12_BARRIER_LAYOUT_UNDEFINED);
			if before == deactivated {
				continue;
			}
			if let Some(command_buffer) = self.active_command_buffer {
				self.retain_command_buffer_resource(command_buffer, &other);
			}
			barriers.texture.push(Self::texture_barrier(&other, before, deactivated));
			self.remember_image_state(other_key);
			self.image_states.insert(other_key, deactivated);
		}
		let before = self.image_states.get(&key).copied().unwrap_or(TextureBarrierState::COMMON);
		if before == after {
			// A merged state, such as a sampled and storage alias, still writes through its unordered access.
			if after.access.0 & D3D12_BARRIER_ACCESS_UNORDERED_ACCESS.0 != 0 && self.unordered_use_needs_barrier(key, writes) {
				barriers.texture.push(Self::texture_barrier(resource, before, after));
			}
			return;
		}
		// The transition orders the image's earlier work, so only this use remains unordered.
		self.unordered_group_covers(key);
		self.unordered_writes.insert(key, writes);
		barriers.texture.push(Self::texture_barrier(resource, before, after));
		self.remember_image_state(key);
		self.image_states.insert(key, after);
	}
}
