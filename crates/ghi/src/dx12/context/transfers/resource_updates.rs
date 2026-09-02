use super::*;

impl Device {
	pub(crate) fn dynamic_buffer_slice_mut<T: crate::Pod>(
		&mut self,
		buffer_handle: DynamicBufferHandle<T>,
		sequence_index: u8,
	) -> &mut T {
		let handle = buffer_handle.into();
		let Some((data, _)) = self.buffer_storage_parts_mut_for_sequence(handle, sequence_index) else {
			panic!("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		};
		unsafe { &mut *(data as *mut T) }
	}

	/// Resizes the active sequence immediately and schedules every other dynamic sequence after its fence completes.
	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent, sequence_index: u8) {
		assert!(
			sequence_index < self.frames,
			"Invalid DX12 image sequence. The most likely cause is that the frame predates a frames-in-flight change."
		);
		let Some((current_extent, format, is_3d, array_layers, dynamic)) =
			self.images.get(image_handle.0.0 as usize).map(|image| {
				(
					image.extent,
					image.format,
					image.is_3d,
					image.array_layers,
					image.frame_resources.is_some(),
				)
			})
		else {
			return;
		};
		if current_extent == extent {
			return;
		}
		Self::validate_image_dimension(extent, is_3d, array_layers, false);
		if !dynamic {
			// A static texture can be referenced by every sequence, so replace it only at a global idle boundary.
			self.wait_for_all_queues_idle().expect(
				"Failed to wait for DX12 queues before resizing a shared image. The most likely cause is that the device was removed.",
			);
			self.prepare_for_topology_change().expect(
				"Failed to reset DX12 command lists before resizing a shared image. The most likely cause is that a completed command list became invalid.",
			);
			self.process_all_tasks_after_idle();
		}

		let data_size = utils::texture_copy_size(format, extent);
		let image = &mut self.images[image_handle.0.0 as usize];
		image.extent = extent;
		if let Some(size) = data_size {
			let data = image.data.get_or_insert_default();
			data.resize(size, 0);
			data.fill(0);
			if let Some(frame_data) = image.frame_data.as_mut() {
				frame_data.resize_with(self.frames as usize, Vec::new);
				for data in frame_data {
					data.resize(size, 0);
					data.fill(0);
				}
			}
		} else {
			image.data = None;
			if let Some(frame_data) = image.frame_data.as_mut() {
				frame_data.clear();
				frame_data.resize_with(self.frames as usize, Vec::new);
			}
		}
		self.pending_texture_syncs.retain(|(pending, _)| *pending != image_handle.0);
		self.resize_image_resource_for_sequence(image_handle, extent, sequence_index);
		if dynamic {
			for offset in 1..self.frames {
				let target_sequence = (sequence_index + offset) % self.frames;
				self.defer_task(
					target_sequence,
					DeferredTask::ResizeImage {
						handle: image_handle,
						extent,
					},
				);
			}
		}
		self.invalidate_descriptor_materializations();
	}

	/// Replaces one sequence's native image after that sequence's fence has made its old resource safe to retire.
	fn resize_image_resource_for_sequence(&mut self, image_handle: ImageHandle, extent: Extent, sequence_index: u8) {
		let image_index = image_handle.0.0 as usize;
		let Some(image) = self.images.get(image_index) else {
			return;
		};
		let (is_3d, format, uses, array_layers, mip_levels, optimized_clear_value, dynamic) = (
			image.is_3d,
			image.format,
			image.uses,
			image.array_layers,
			image.mip_levels,
			image.optimized_clear_value,
			image.frame_resources.is_some(),
		);
		let old_resource = if dynamic {
			self.images[image_index]
				.frame_resources
				.as_mut()
				.and_then(|resources| resources.get_mut(sequence_index as usize))
				.and_then(Option::take)
		} else {
			self.images[image_index].resource.take()
		};
		if let Some(key) = old_resource.as_ref().map(Self::native_resource_key) {
			self.invalidate_attachment_views_for_resources(&[key]);
			self.invalidate_clear_uav_descriptors_for_resources(&[key]);
			self.image_states.remove(&key);
		}

		let resource = self.create_image_resource(extent, is_3d, format, uses, array_layers, mip_levels, optimized_clear_value);
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, format, uses, array_layers);
		}
		if dynamic {
			let resources = self.images[image_index].frame_resources.as_mut().unwrap();
			if resources.len() <= sequence_index as usize {
				resources.resize(self.frames as usize, None);
			}
			resources[sequence_index as usize] = resource;
			self.queue_texture_sync_for_sequence(image_handle.0, sequence_index);
		} else {
			self.images[image_index].resource = resource;
		}
		// The current sequence fence completed before this function runs, so its replaced resource can now be released.
		drop(old_resource);
	}

	/// Adds work directly to the sequence that owns its lifetime.
	pub(crate) fn defer_task(&mut self, sequence_index: u8, task: DeferredTask) {
		let tasks = self.deferred_tasks.get_mut(sequence_index as usize).expect(
			"Invalid DX12 deferred-task sequence. The most likely cause is that a task outlived a frames-in-flight change.",
		);
		if let DeferredTask::ResizeImage { handle, extent } = &task {
			if let Some(pending_extent) = tasks.iter_mut().rev().find_map(|pending| match pending {
				DeferredTask::ResizeImage {
					handle: pending_handle,
					extent,
				} if pending_handle == handle => Some(extent),
				_ => None,
			}) {
				// Only the newest extent matters before this sequence can use the image again.
				*pending_extent = *extent;
				return;
			}
		}
		tasks.push(task);
	}

	/// Executes only tasks owned by a sequence whose frame fence has just completed.
	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let index = sequence_index as usize;
		let mut ready = std::mem::take(self.deferred_tasks.get_mut(index).expect(
			"Invalid DX12 deferred-task sequence. The most likely cause is that a frame predates a frames-in-flight change.",
		));
		for task in ready.drain(..) {
			self.execute_task(task, sequence_index);
		}

		// Reuse the allocation and preserve tasks scheduled while the detached snapshot was executing.
		ready.append(&mut self.deferred_tasks[index]);
		self.deferred_tasks[index] = ready;
	}

	/// Drains every deferred operation after a global queue-idle boundary proves all sequence resources are unused.
	pub(crate) fn process_all_tasks_after_idle(&mut self) {
		while self.deferred_tasks.iter().any(|tasks| !tasks.is_empty()) {
			for sequence_index in 0..crate::MAX_FRAMES_IN_FLIGHT as u8 {
				self.process_tasks(sequence_index);
			}
		}
	}

	/// Applies one deferred operation without recursively consuming tasks created by that operation.
	fn execute_task(&mut self, task: DeferredTask, sequence_index: u8) {
		match task {
			DeferredTask::RetireResource(resource) => drop(resource),
			DeferredTask::RetireBufferFrameStorage(storage) => drop(storage),
			DeferredTask::ResizeImage { handle, extent } => {
				self.resize_image_resource_for_sequence(handle, extent, sequence_index);
			}
		}
	}
}
