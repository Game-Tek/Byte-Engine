use super::super::*;

impl Device {
	/// Submits one directly recorded command buffer through the same ordered batch path used by queue executions.
	pub(crate) fn submit_command_buffer(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		synchronizer_handle: SynchronizerHandle,
	) {
		let command_buffer = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		let (queue_handle, sequence_index) = (command_buffer.queue_handle, command_buffer.sequence_index);
		if let Some(frame_synchronizer) = command_buffer.frame_synchronizer {
			assert_eq!(
				frame_synchronizer, synchronizer_handle,
				"Invalid DX12 frame synchronizer. The most likely cause is that a frame command buffer would signal a different fence than the one used to retire its sequence."
			);
		}
		self.submit_execution(
			queue_handle,
			std::slice::from_ref(&command_buffer_handle),
			&[],
			synchronizer_handle,
			sequence_index,
			&[],
		);
	}

	/// Validates the present keys, submits the command buffers as one native batch, presents, and signals the
	/// execution's terminal fence, in that order.
	///
	/// `prepared_present_keys` lists the keys whose proxy copies the command buffers recorded, and `present_keys` the
	/// keys to present.
	pub(crate) fn submit_execution(
		&mut self,
		queue_handle: QueueHandle,
		command_buffer_handles: &[CommandBufferHandle],
		prepared_present_keys: &[PresentKey],
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
		present_keys: &[PresentKey],
	) {
		self.validate_present_keys(queue_handle, sequence_index, present_keys);
		self.validate_present_preparation(prepared_present_keys, present_keys);
		let readbacks = self.execute_command_buffers(queue_handle, command_buffer_handles, sequence_index);
		for &present_key in present_keys {
			self.present_swapchain(present_key);
		}
		self.complete_command_buffer_execution(
			queue_handle,
			command_buffer_handles,
			synchronizer_handle,
			sequence_index,
			readbacks,
		);
		self.complete_present_submission(!present_keys.is_empty());
	}

	/// Closes and submits one execution as a single native queue batch before presentation and its terminal signal.
	pub(crate) fn execute_command_buffers(
		&mut self,
		queue_handle: QueueHandle,
		command_buffer_handles: &[CommandBufferHandle],
		sequence_index: u8,
	) -> SmallVec<[TextureCopyHandle; 4]> {
		// Validate the entire execution before closing any list so a rejected batch leaves every recording recoverable.
		for &handle in command_buffer_handles {
			let command_buffer = self.command_buffers.get(handle.0 as usize).expect(
				"Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.",
			);
			assert_eq!(
				command_buffer.queue_handle, queue_handle,
				"Invalid DX12 execution queue. The most likely cause is that the command buffer was created by a different queue."
			);
			assert_eq!(
				command_buffer.sequence_index, sequence_index,
				"Invalid DX12 execution sequence. The most likely cause is that the execution combined command buffers from different frames."
			);
			assert_eq!(
				command_buffer.lifecycle,
				CommandBufferLifecycle::Scheduled,
				"DX12 command buffer is not scheduled. The most likely cause is that the recording was abandoned or submitted more than once."
			);
			assert!(
				command_buffer.command_list.is_some(),
				"Missing DX12 command list. The most likely cause is that command-buffer creation failed."
			);
		}

		let mut native_lists = SmallVec::<[Option<ID3D12CommandList>; 4]>::new();
		let mut readback_candidates = SmallVec::<[TextureCopyHandle; 4]>::new();
		let mut submitted_readbacks = SmallVec::<[TextureCopyHandle; 4]>::new();
		for &handle in command_buffer_handles {
			let command_buffer_index = handle.0 as usize;
			let command_list = self.command_buffers[command_buffer_index].command_list.clone().unwrap();
			if self.command_buffers[command_buffer_index].is_open {
				if unsafe { command_list.Close() }.is_err() {
					// A list whose Close call fails can never be reset. Keep its native references and block later submissions.
					self.command_buffers[command_buffer_index].lifecycle = CommandBufferLifecycle::Poisoned;
					self.drain_debug_messages();
					panic!(
						"Failed to close a DX12 command list. The most likely cause is that command list recording failed or the command list was already closed."
					);
				}
				self.command_buffers[command_buffer_index].is_open = false;
			}
			// Closing a list leaves its recorded-work flag unchanged.
			if self.command_buffers[command_buffer_index].recorded_work {
				native_lists.push(Some(command_list.cast::<ID3D12CommandList>().expect(
					"Failed to cast a DX12 graphics command list for execution. The most likely cause is an incompatible command list object.",
				)));
				readback_candidates.extend_from_slice(&self.command_buffers[command_buffer_index].recorded_readbacks);
			} else {
				debug_assert!(self.command_buffers[command_buffer_index].recorded_readbacks.is_empty());
				self.counters.empty_command_list_skip_count += 1;
			}
		}

		if !native_lists.is_empty() {
			let queue = self
				.queues
				.get(queue_handle.0 as usize)
				.expect("Invalid DX12 queue handle. The most likely cause is that the handle came from another device.");
			// SAFETY: All command-list interfaces stay alive through this synchronous queue call.
			unsafe { queue.queue.ExecuteCommandLists(&native_lists) };
			// Until a terminal signal succeeds, keep submitted resources permanently retained if unwinding interrupts finalization.
			for &handle in command_buffer_handles {
				self.command_buffers[handle.0 as usize].lifecycle = CommandBufferLifecycle::Poisoned;
			}
			self.counters.native_command_list_execute_count = self
				.counters
				.native_command_list_execute_count
				.saturating_add(native_lists.len());
			for handle in readback_candidates {
				// DX12 tracks readback completion through `completion`, so the shared state carries no synchronizer.
				if self.texture_readbacks.mark_submitted(handle, None) {
					submitted_readbacks.push(handle);
				}
			}
		}

		for &handle in command_buffer_handles {
			self.command_buffers[handle.0 as usize].recorded_readbacks.clear();
			self.command_buffers[handle.0 as usize].recorded_texture_syncs.clear();
			self.commit_command_buffer_resource_states(handle);
		}
		submitted_readbacks
	}

	/// Appends the execution's one terminal queue signal after presentation and publishes its exact completion value.
	pub(crate) fn complete_command_buffer_execution(
		&mut self,
		queue_handle: QueueHandle,
		command_buffer_handles: &[CommandBufferHandle],
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
		submitted_readbacks: SmallVec<[TextureCopyHandle; 4]>,
	) {
		let completion = match self.signal_synchronizer_for_sequence(queue_handle, synchronizer_handle, sequence_index) {
			Some(Ok(completion)) => completion,
			Some(Err(_)) => panic!(
				"Failed to signal a DX12 fence. The most likely cause is that the queue or fence was invalid or the device was removed."
			),
			None => panic!(
				"Invalid DX12 synchronizer sequence. The most likely cause is that the synchronizer was not created for the active frame count."
			),
		};
		for &handle in command_buffer_handles {
			let command_buffer = &mut self.command_buffers[handle.0 as usize];
			command_buffer.last_submission = Some((synchronizer_handle, sequence_index));
			command_buffer.lifecycle = CommandBufferLifecycle::Submitted;
		}
		for handle in submitted_readbacks {
			if let Some(readback) = self.texture_readbacks.get_mut(handle) {
				readback.completion = Some(completion);
			}
		}
	}

	/// Records terminal swapchain transitions before later command buffers derive their opening layouts.
	pub(crate) fn finish_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) {
		assert_eq!(
			self.command_buffers
				.get(command_buffer_handle.0 as usize)
				.map(|command_buffer| command_buffer.lifecycle),
			Some(CommandBufferLifecycle::Recording),
			"DX12 command buffer is not recording. The most likely cause is that the recording was finished more than once."
		);
		if let Some(command_list) = self.command_list(command_buffer_handle).cloned() {
			self.transition_present_resources(command_buffer_handle, &command_list);
		}
		self.finish_command_buffer_state_transaction(command_buffer_handle);
		self.command_buffers[command_buffer_handle.0 as usize].lifecycle = CommandBufferLifecycle::Scheduled;
	}

	pub(crate) fn record_present_preparation(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		present_keys: &[PresentKey],
	) {
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};

		let mut copies = SmallVec::<[(ImageHandle, ID3D12Resource, ID3D12Resource); 2]>::new();
		let mut copy_barriers = EnhancedBarrierBatch::default();
		for present_key in present_keys {
			let Some((source_image, proxy_uses)) = self
				.swapchains
				.get(present_key.swapchain.0 as usize)
				.and_then(|swapchain| swapchain.image.map(|image| (image, swapchain.proxy_uses)))
			else {
				continue;
			};
			if !proxy_uses.intersects(Uses::Storage) {
				continue;
			}
			let Some(source_resource) = self.ensure_image_resource_for_sequence(source_image.0, present_key.sequence_index)
			else {
				continue;
			};
			let Some(destination_resource) =
				self.swapchain_backbuffer_resource(present_key.swapchain, present_key.sequence_index)
			else {
				continue;
			};

			self.transition_tracked_image_into(
				source_image.0,
				&source_resource,
				TextureBarrierState::COPY_SOURCE,
				&mut copy_barriers,
			);
			self.transition_swapchain_texture_into(
				&destination_resource,
				TextureBarrierState::COPY_DESTINATION,
				&mut copy_barriers,
			);
			copies.push((source_image, source_resource, destination_resource));
		}
		if copies.is_empty() {
			return;
		}

		// Transition every proxy first, issue the copies, then return them in one terminal batch.
		copy_barriers.submit(&command_list);
		for (_, source_resource, destination_resource) in &copies {
			unsafe {
				command_list.CopyResource(destination_resource, source_resource);
			}
		}
		let mut present_barriers = EnhancedBarrierBatch::default();
		for (source_image, source_resource, destination_resource) in &copies {
			self.transition_swapchain_texture_into(destination_resource, TextureBarrierState::PRESENT, &mut present_barriers);
			self.transition_tracked_image_into(
				source_image.0,
				source_resource,
				TextureBarrierState::COMMON,
				&mut present_barriers,
			);
		}
		present_barriers.submit(&command_list);
		self.mark_command_buffer_work(command_buffer_handle);
		self.counters.texture_copy_count += copies.len();
	}

	/// Transitions every backbuffer used by one recording into PRESENT through one native barrier batch.
	pub(crate) fn transition_present_resources(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
	) {
		let command_buffer_index = command_buffer_handle.0 as usize;
		let mut resources = std::mem::take(&mut self.command_buffers[command_buffer_index].present_resources);
		let mut barriers = EnhancedBarrierBatch::default();
		let mut transition_count = 0;
		for resource in resources.drain(..) {
			transition_count +=
				usize::from(self.transition_swapchain_texture_into(&resource, TextureBarrierState::PRESENT, &mut barriers));
		}
		self.command_buffers[command_buffer_index].present_resources = resources;
		if transition_count != 0 {
			barriers.submit(command_list);
			self.mark_command_buffer_work(command_buffer_handle);
			self.counters.swapchain_present_transition_count += transition_count;
		}
	}

	/// Signals the fence that tracks `sequence_index` of a logical synchronizer on one queue, and returns the fence with
	/// its new value, or `None` when the sequence, queue, or fence is missing.
	pub(crate) fn signal_synchronizer_for_sequence(
		&mut self,
		queue_handle: QueueHandle,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) -> Option<windows::core::Result<(crate::synchronizer::SynchronizerHandle, u64)>> {
		let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, sequence_index)?;
		let queue = self.queues.get(queue_handle.0 as usize)?.queue.clone();
		let synchronizer = self.synchronizers.get(synchronizer_handle.0 as usize)?;
		let fence = synchronizer.fence.clone();
		let current_value = synchronizer.value;
		let last_signal_queue = synchronizer.last_signal_queue;
		let next_value = synchronizer.value.checked_add(1).expect(
			"DX12 fence value overflowed. The most likely cause is that the synchronizer was signaled more than u64::MAX times.",
		);
		if last_signal_queue.is_some_and(|previous| previous != queue_handle) {
			// Serialize a fence handoff before another queue advances the same progress timeline.
			if let Err(error) = unsafe { queue.Wait(&fence, current_value) } {
				return Some(Err(error));
			}
		}
		if let Err(error) = unsafe { queue.Signal(&fence, next_value) } {
			return Some(Err(error));
		}
		self.synchronizers[synchronizer_handle.0 as usize].value = next_value;
		self.synchronizers[synchronizer_handle.0 as usize].last_signal_queue = Some(queue_handle);
		Some(Ok((synchronizer_handle, next_value)))
	}

	/// Inserts GPU-side waits for every concrete fence in one logical synchronizer chain.
	pub(crate) fn queue_wait_for_synchronizer(&self, queue_handle: QueueHandle, synchronizer_handle: SynchronizerHandle) {
		let queue = self
			.queues
			.get(queue_handle.0 as usize)
			.expect("Invalid DX12 queue handle. The most likely cause is that the handle came from another device.");
		for handle in self.synchronizer_handles(synchronizer_handle) {
			let synchronizer = self
				.synchronizers
				.get(handle.0 as usize)
				.expect("Invalid DX12 synchronizer handle. The most likely cause is that the handle came from another device.");
			unsafe { queue.queue.Wait(&synchronizer.fence, synchronizer.value) }.expect(
				"Failed to queue a DX12 fence wait. The most likely cause is that the queue or fence was invalid or the device was removed.",
			);
		}
	}
}
