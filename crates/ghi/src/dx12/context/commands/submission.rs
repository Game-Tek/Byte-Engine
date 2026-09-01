use super::super::*;

impl Device {
	pub(crate) fn submit_command_buffer(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		synchronizer_handle: SynchronizerHandle,
	) -> bool {
		let command_buffer_index = command_buffer_handle.0 as usize;
		let Some(command_buffer) = self.command_buffers.get(command_buffer_index) else {
			return false;
		};
		let Some(command_list) = command_buffer.command_list.as_ref() else {
			return false;
		};
		let command_list = (*command_list).clone();
		let is_open = command_buffer.is_open;
		let queue_handle = command_buffer.queue_handle;
		let sequence_index = command_buffer.sequence_index;

		let recorded_work = self
			.command_buffers
			.get(command_buffer_index)
			.map(|command_buffer| command_buffer.recorded_work)
			.unwrap_or(false);
		if is_open {
			let result = unsafe { command_list.Close() };
			if result.is_err() {
				self.drain_debug_messages();
				panic!(
					"Failed to close a DX12 command list. The most likely cause is that command list recording failed or the command list was already closed."
				);
			}
			if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_index) {
				command_buffer.is_open = false;
			}
		}

		if !recorded_work {
			self.empty_command_list_skip_count += 1;
			self.complete_synchronizer_for_sequence_from_cpu(synchronizer_handle, sequence_index);
			return false;
		}

		let readback_handles = self
			.texture_readbacks
			.entries()
			.filter_map(|(handle, readback)| (readback.command_buffer_handle == Some(command_buffer_handle)).then_some(handle))
			.collect::<SmallVec<[_; 4]>>();
		let Some(queue) = self.queues.get(queue_handle.0 as usize) else {
			return false;
		};
		let command_list = command_list.cast::<ID3D12CommandList>().expect(
			"Failed to cast a DX12 graphics command list for execution. The most likely cause is an incompatible command list object.",
		);
		let command_lists = [Some(command_list)];
		unsafe {
			queue.queue.ExecuteCommandLists(&command_lists);
		}
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_index) {
			command_buffer.recorded_texture_syncs.clear();
		}
		self.commit_command_buffer_resource_states(command_buffer_handle);
		// Native submission now owns the staging lifetime, even if later fence setup fails.
		for &handle in &readback_handles {
			self.texture_readbacks.mark_submitted(handle);
		}
		self.native_command_list_execute_count += 1;
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_index) {
			command_buffer.last_submission = Some((synchronizer_handle, sequence_index));
		}
		self.signal_synchronizer_for_sequence(queue_handle, synchronizer_handle, sequence_index);
		let completion = self
			.synchronizer_for_sequence(synchronizer_handle, sequence_index)
			.and_then(|handle| {
				self.synchronizers
					.get(handle.0 as usize)
					.map(|synchronizer| (handle, synchronizer.value))
			});
		for handle in readback_handles {
			if let Some(readback) = self.texture_readbacks.get_mut(handle) {
				readback.completion = completion;
			}
		}
		true
	}

	/// Records terminal swapchain transitions before later command buffers derive their opening layouts.
	pub(crate) fn finish_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) {
		let command_list = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone());
		if let Some(command_list) = command_list {
			self.transition_present_resources(command_buffer_handle, &command_list);
		}
		self.finish_command_buffer_state_transaction(command_buffer_handle);
	}

	pub(crate) fn record_present_preparation(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		present_keys: &[PresentKey],
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};

		for present_key in present_keys {
			let Some((source_image, proxy_uses)) =
				self.swapchains.get(present_key.swapchain.0 as usize).and_then(|swapchain| {
					let image_index = (present_key.sequence_index as usize).min(swapchain.images.len().saturating_sub(1));
					swapchain.images[image_index]
						.or(swapchain.images[0])
						.map(|image| (image, swapchain.proxy_uses[image_index]))
				})
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

			// Copy the engine swapchain proxy image into the actual DXGI backbuffer before Present.
			let mut copy_barriers = EnhancedBarrierBatch::default();
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
			Self::submit_resource_barriers(&command_list, &copy_barriers);
			unsafe {
				command_list.CopyResource(&destination_resource, &source_resource);
			}
			let mut present_barriers = EnhancedBarrierBatch::default();
			self.transition_swapchain_texture_into(&destination_resource, TextureBarrierState::PRESENT, &mut present_barriers);
			self.transition_tracked_image_into(
				source_image.0,
				&source_resource,
				TextureBarrierState::COMMON,
				&mut present_barriers,
			);
			Self::submit_resource_barriers(&command_list, &present_barriers);
			self.mark_command_buffer_work(command_buffer_handle);
			self.texture_copy_count += 1;
		}
	}

	/// Transitions every backbuffer used by one recording into PRESENT through one native barrier batch.
	pub(crate) fn transition_present_resources(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
	) {
		let Some(resources) = self.present_transitions.remove(&command_buffer_handle) else {
			return;
		};
		let mut barriers = EnhancedBarrierBatch::default();
		let mut transition_count = 0;
		for resource in resources {
			transition_count +=
				usize::from(self.transition_swapchain_texture_into(&resource, TextureBarrierState::PRESENT, &mut barriers));
		}
		if transition_count != 0 {
			Self::submit_resource_barriers(command_list, &barriers);
			self.mark_command_buffer_work(command_buffer_handle);
			self.swapchain_present_transition_count += transition_count;
		}
	}

	pub(crate) fn signal_private_synchronizer(
		&mut self,
		queue_handle: QueueHandle,
		synchronizer_handle: crate::synchronizer::SynchronizerHandle,
	) {
		let Some(queue) = self.queues.get(queue_handle.0 as usize) else {
			return;
		};
		let Some(synchronizer) = self.synchronizers.get_mut(synchronizer_handle.0 as usize) else {
			return;
		};
		synchronizer.value = synchronizer.value.saturating_add(1);
		let result = unsafe { queue.queue.Signal(&synchronizer.fence, synchronizer.value) };
		if result.is_err() {
			panic!(
				"Failed to signal a DX12 fence. The most likely cause is that the queue or fence was invalid or the device was removed."
			);
		}
	}

	pub(crate) fn signal_synchronizer_for_sequence(
		&mut self,
		queue_handle: QueueHandle,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.signal_private_synchronizer(queue_handle, handle);
	}

	/// Completes an empty submission without sending a no-op command list to the GPU queue.
	pub(crate) fn complete_private_synchronizer_from_cpu(
		&mut self,
		synchronizer_handle: crate::synchronizer::SynchronizerHandle,
	) {
		let Some(synchronizer) = self.synchronizers.get_mut(synchronizer_handle.0 as usize) else {
			return;
		};
		synchronizer.value = synchronizer.value.saturating_add(1);
		let result = unsafe { synchronizer.fence.Signal(synchronizer.value) };
		if result.is_err() {
			panic!(
				"Failed to complete a DX12 fence from the CPU. The most likely cause is that the fence was invalid or the device was removed."
			);
		}
	}

	/// Completes an empty frame sequence without submitting work to a DX12 queue.
	pub(crate) fn complete_synchronizer_for_sequence_from_cpu(
		&mut self,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.complete_private_synchronizer_from_cpu(handle);
	}
}
