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
		self.pending_texture_syncs.retain(|(pending, ..)| *pending != image_handle.0);
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

	/// Recreates every member of an image group as placed resources in heaps that members with disjoint lifetimes share.
	///
	/// Does nothing when the group is already placed from the same requests. Every sequence can reference a member, so
	/// the replacement waits for idle queues like a static image resize. Members keep their handles.
	pub(crate) fn place_image_group(&mut self, group: crate::ImageGroupHandle, requests: &[crate::ImageGroupMember]) {
		let Some(requests) = self.image_groups.requests_in_member_order(group, requests) else {
			return;
		};
		self.wait_for_all_queues_idle().expect(
			"Failed to wait for DX12 queues before placing an image group. The most likely cause is that the device was removed.",
		);
		self.prepare_for_topology_change().expect(
			"Failed to reset DX12 command lists before placing an image group. The most likely cause is that a completed command list became invalid.",
		);
		self.process_all_tasks_after_idle();

		let separate_render_targets = self.resource_heap_tier() == D3D12_RESOURCE_HEAP_TIER_1;
		let members = requests
			.iter()
			.map(|request| {
				let index = request.image.0 as usize;
				let image = &self.images[index];
				let desc = Self::image_resource_desc(
					request.extent,
					image.is_3d,
					image.format,
					image.uses,
					image.array_layers,
					image.mip_levels,
				)
				.expect(
					"Image-group member has no native texture description. The most likely cause is a zero extent or a format the member's uses cannot be created with.",
				);
				(index, desc)
			})
			.collect::<Vec<_>>();
		let requirements = members
			.iter()
			.zip(&requests)
			.map(|((_, desc), request)| {
				let info = unsafe { self.device.GetResourceAllocationInfo2(0, 1, desc, None) };
				let render_target_flags = D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET.0 | D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL.0;
				// Resource heap tier 1 keeps render-target and depth textures in heaps of their own.
				let category = u32::from(separate_render_targets && desc.Flags.0 & render_target_flags != 0);
				(
					crate::image_group::MemoryRequirements {
						size: info.SizeInBytes,
						alignment: info.Alignment,
						category,
					},
					request.lifetime.clone(),
				)
			})
			.collect::<Vec<_>>();
		let placement = crate::image_group::pack(&requirements);

		const HEAP_ALIGNMENT: u64 = D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT as u64;
		let heaps = placement
			.heaps
			.iter()
			.map(|layout| {
				let flags = match (separate_render_targets, layout.category) {
					(false, _) => D3D12_HEAP_FLAG_DENY_BUFFERS,
					(true, 1) => D3D12_HEAP_FLAG_ALLOW_ONLY_RT_DS_TEXTURES,
					(true, _) => D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
				};
				let heap_desc = D3D12_HEAP_DESC {
					SizeInBytes: layout.size.next_multiple_of(HEAP_ALIGNMENT),
					Properties: D3D12_HEAP_PROPERTIES {
						Type: D3D12_HEAP_TYPE_DEFAULT,
						CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
						MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
						CreationNodeMask: 1,
						VisibleNodeMask: 1,
					},
					// Zero selects the 64 KiB default. Only multisampled textures need the larger 4 MiB alignment.
					Alignment: if layout.alignment > HEAP_ALIGNMENT {
						D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT as u64
					} else {
						0
					},
					Flags: flags,
				};
				let mut heap: Option<ID3D12Heap> = None;
				unsafe { self.device.CreateHeap(&heap_desc, &mut heap) }.expect(
					"Failed to create a DX12 heap for an image group. The most likely cause is that device memory is exhausted.",
				);
				heap.expect(
					"Failed to create a DX12 heap for an image group. The most likely cause is that the driver reported success without returning the heap.",
				)
			})
			.collect::<SmallVec<[_; 2]>>();

		for (((index, desc), slot), request) in members.into_iter().zip(&placement.slots).zip(&requests) {
			let image = &self.images[index];
			let (format, uses, array_layers, is_3d, optimized_clear_value) = (
				image.format,
				image.uses,
				image.array_layers,
				image.is_3d,
				image.optimized_clear_value,
			);
			let resource = self.create_placed_image_resource(
				&heaps[slot.heap],
				slot.offset,
				&desc,
				(format, request.extent, is_3d, uses, array_layers),
				optimized_clear_value,
			);
			if let Some(key) = self.images[index].resource.take().as_ref().map(Self::native_resource_key) {
				self.invalidate_attachment_views_for_resources(&[key]);
				self.invalidate_clear_uav_descriptors_for_resources(&[key]);
				self.image_states.remove(&key);
			}
			self.materialize_image_attachment_views(&resource, format, uses, array_layers);
			// The member starts in the undefined layout, so the barrier that first initializes it discards its contents.
			self.image_states.insert(
				Self::native_resource_key(&resource),
				TextureBarrierState::new(
					D3D12_BARRIER_SYNC_NONE,
					D3D12_BARRIER_ACCESS_NO_ACCESS,
					D3D12_BARRIER_LAYOUT_UNDEFINED,
				),
			);

			let image = &mut self.images[index];
			image.extent = request.extent;
			image.data = utils::texture_copy_size(format, request.extent).map(|size| vec![0u8; size]);
			image.resource = Some(resource);
			self.pending_texture_syncs.retain(|(pending, ..)| pending.0 as usize != index);
		}

		// The queues are idle and the previous members were released above, so their heaps can go now.
		self.image_group_heaps[group.0 as usize] = heaps;
		self.invalidate_descriptor_materializations();
		self.image_groups.commit(group, requests, placement);
	}

	/// Returns the resource heap tier, which decides whether render-target textures can share a heap with others.
	fn resource_heap_tier(&self) -> D3D12_RESOURCE_HEAP_TIER {
		let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS::default();
		let result = unsafe {
			self.device.CheckFeatureSupport(
				D3D12_FEATURE_D3D12_OPTIONS,
				(&mut options as *mut D3D12_FEATURE_DATA_D3D12_OPTIONS).cast(),
				std::mem::size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS>() as u32,
			)
		};
		// Assume the most restrictive tier when the query fails.
		result.map_or(D3D12_RESOURCE_HEAP_TIER_1, |()| options.ResourceHeapTier)
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
