use super::super::*;

impl Device {
	/// Returns the device-constant stride for a native descriptor heap type.
	pub(crate) fn descriptor_handle_increment_size(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
	) -> u32 {
		self.descriptor_handle_increment_sizes[heap_type.0 as usize]
	}

	pub(crate) fn descriptor_cpu_handle(
		&self,
		heap: &DescriptorHeap,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		slot: u32,
	) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		let mut handle = heap.cpu_start;
		let stride = self.descriptor_handle_increment_size(heap_type) as usize;
		handle.ptr = handle.ptr.saturating_add(slot as usize * stride);
		handle
	}

	pub(crate) fn descriptor_gpu_handle(
		&self,
		heap: &DescriptorHeap,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		slot: u32,
	) -> D3D12_GPU_DESCRIPTOR_HANDLE {
		let mut handle = heap.gpu_start.expect(
			"Missing GPU descriptor heap start. The most likely cause is that a CPU-only heap was used for a GPU descriptor table.",
		);
		let stride = self.descriptor_handle_increment_size(heap_type) as u64;
		handle.ptr = handle.ptr.saturating_add(slot as u64 * stride);
		handle
	}

	/// Creates a shader-visible heap for retained tables or transient GPU descriptor operations.
	pub(crate) fn create_shader_visible_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
	) -> Option<DescriptorHeap> {
		let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
			NodeMask: 0,
		};
		match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) } {
			Ok(native) => Some(DescriptorHeap {
				cpu_start: unsafe { native.GetCPUDescriptorHandleForHeapStart() },
				gpu_start: Some(unsafe { native.GetGPUDescriptorHandleForHeapStart() }),
				native,
			}),
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				let message = format!(
					"Failed to create a shader-visible DX12 descriptor heap. The most likely cause is descriptor heap exhaustion or device removal. Heap type: {:?}. Descriptor count: {descriptor_count}. Error: {error:?}. Device removed reason: {removed_reason:?}",
					heap_type
				);
				self.log_dx12_error(&message);
				panic!("{message}");
			}
		}
	}

	/// Creates one CPU-readable descriptor heap for reusable command-buffer staging.
	pub(crate) fn create_cpu_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
	) -> Option<DescriptorHeap> {
		let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: Default::default(),
			NodeMask: 0,
		};
		let heap = match unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) } {
			Ok(native) => DescriptorHeap {
				cpu_start: unsafe { native.GetCPUDescriptorHandleForHeapStart() },
				gpu_start: None,
				native,
			},
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				let message = format!(
					"Failed to create a CPU-only DX12 descriptor heap: {error:?}. The most likely cause is descriptor heap exhaustion or device removal. Heap type: {:?}. Descriptor count: {descriptor_count}. Device removed reason: {removed_reason:?}",
					heap_type
				);
				self.log_dx12_error(&message);
				panic!("{message}");
			}
		};
		Some(heap)
	}

	/// Allocates one retained CPU descriptor slot from stable, reusable heap pages.
	pub(crate) fn allocate_retained_cpu_descriptor(&mut self) -> Option<RetainedCpuDescriptor> {
		if let Some((page_index, slot)) = self.free_clear_uav_descriptor_slots.pop() {
			let heap = self.clear_uav_descriptor_pages.get(page_index)?.heap.clone();
			return Some(RetainedCpuDescriptor { heap, page_index, slot });
		}

		let needs_page = self
			.clear_uav_descriptor_pages
			.last()
			.map(|page| page.used >= page.capacity)
			.unwrap_or(true);
		if needs_page {
			let capacity = 256;
			let heap = self.create_cpu_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, capacity)?;
			self.clear_uav_descriptor_pages
				.push(DescriptorHeapArena { heap, capacity, used: 0 });
		}

		let page_index = self.clear_uav_descriptor_pages.len().checked_sub(1)?;
		let page = self.clear_uav_descriptor_pages.get_mut(page_index)?;
		let slot = page.used;
		page.used = page.used.saturating_add(1);
		Some(RetainedCpuDescriptor {
			heap: page.heap.clone(),
			page_index,
			slot,
		})
	}

	/// Returns one retained CPU UAV descriptor for a native resource, creating it on first use.
	pub(crate) fn retained_clear_uav_descriptor(
		&mut self,
		resource: &ID3D12Resource,
		description: &D3D12_UNORDERED_ACCESS_VIEW_DESC,
	) -> Option<RetainedCpuDescriptor> {
		let resource_key = Self::native_resource_key(resource);
		if let Some(descriptor) = self.retained_clear_uav_descriptors.get(&resource_key) {
			return Some(descriptor.clone());
		}

		let descriptor = self.allocate_retained_cpu_descriptor()?;
		let cpu_handle = self.descriptor_cpu_handle(&descriptor.heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor.slot);
		unsafe {
			self.device
				.CreateUnorderedAccessView(resource, None::<&ID3D12Resource>, Some(description), cpu_handle);
		}
		self.retained_clear_uav_descriptors.insert(resource_key, descriptor.clone());
		Some(descriptor)
	}

	/// Releases retained clear descriptors after their native backing resources are replaced.
	pub(crate) fn invalidate_clear_uav_descriptors_for_resources(&mut self, resources: &[usize]) {
		// Preserve queued writes before their retained source slots can be recycled for replacement resources.
		for command_buffer_index in 0..self.command_buffers.len() {
			self.flush_pending_clear_descriptor_copies(CommandBufferHandle(command_buffer_index as u64));
		}
		for resource in resources {
			let Some(descriptor) = self.retained_clear_uav_descriptors.remove(resource) else {
				continue;
			};
			self.free_clear_uav_descriptor_slots
				.push((descriptor.page_index, descriptor.slot));
		}
	}

	pub(crate) fn reserve_staged_descriptor_range(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sampler_heap: bool,
		descriptor_count: u32,
	) -> Option<(DescriptorHeap, u32)> {
		if descriptor_count == 0 {
			return None;
		}

		let heap_type = if sampler_heap {
			D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
		} else {
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
		};
		let command_buffer_index = command_buffer_handle.0 as usize;
		let (current_capacity, current_used) = {
			let command_buffer = self.command_buffers.get(command_buffer_index)?;
			let arena = if sampler_heap {
				command_buffer.sampler_staging_heap.as_ref()
			} else {
				command_buffer.cbv_srv_uav_staging_heap.as_ref()
			};
			arena.map(|arena| (arena.capacity, arena.used)).unwrap_or((0, 0))
		};
		let required = current_used.saturating_add(descriptor_count);

		if required > current_capacity {
			let capacity = required.max(current_capacity.saturating_mul(2)).max(256);
			let heap = self.create_shader_visible_descriptor_heap(heap_type, capacity)?;
			let command_buffer = self.command_buffers.get_mut(command_buffer_index)?;
			let target_arena = if sampler_heap {
				&mut command_buffer.sampler_staging_heap
			} else {
				&mut command_buffer.cbv_srv_uav_staging_heap
			};
			if let Some(previous) = target_arena.replace(DescriptorHeapArena { heap, capacity, used: 0 }) {
				if previous.used > 0 {
					command_buffer.retained_descriptor_heaps.push(previous.heap.native);
				}
			}
		}

		let command_buffer = self.command_buffers.get_mut(command_buffer_index)?;
		let arena = if sampler_heap {
			command_buffer.sampler_staging_heap.as_mut()
		} else {
			command_buffer.cbv_srv_uav_staging_heap.as_mut()
		}?;
		let offset = arena.used;
		arena.used = arena.used.saturating_add(descriptor_count);
		Some((arena.heap.clone(), offset))
	}

	/// Binds the command buffer's active staged descriptor heaps after transient descriptor writes.
	pub(crate) fn bind_active_staged_descriptor_heaps(&mut self, command_buffer_handle: CommandBufferHandle) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(command_buffer) = self.command_buffers.get(command_buffer_handle.0 as usize) else {
			return;
		};

		let mut heaps = [None, None];
		let mut heap_count = 0usize;
		if let Some(arena) = command_buffer
			.cbv_srv_uav_staging_heap
			.as_ref()
			.filter(|arena| arena.used > 0)
		{
			heaps[heap_count] = Some(arena.heap.native.clone());
			heap_count += 1;
		}
		if let Some(arena) = command_buffer.sampler_staging_heap.as_ref().filter(|arena| arena.used > 0) {
			heaps[heap_count] = Some(arena.heap.native.clone());
			heap_count += 1;
		}
		if heap_count == 0 {
			return;
		}

		unsafe {
			command_list.SetDescriptorHeaps(&heaps[..heap_count]);
		}
		self.descriptor_heap_bind_count += 1;
	}
	/// Retains each bound heap until this command buffer's submitted work has completed.
	pub(crate) fn retain_descriptor_materialization(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		materialization: &DescriptorMaterialization,
	) {
		for heap in [
			materialization.cbv_srv_uav_heap.as_ref(),
			materialization.sampler_heap.as_ref(),
		]
		.into_iter()
		.flatten()
		{
			self.retain_descriptor_heap(command_buffer_handle, heap);
		}
	}

	/// Retains a descriptor heap until the command buffer's previous submission has completed.
	pub(crate) fn retain_descriptor_heap(&mut self, command_buffer_handle: CommandBufferHandle, heap: &DescriptorHeap) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		let identity = heap.native.as_raw();
		if command_buffer
			.retained_descriptor_heaps
			.iter()
			.any(|retained| retained.as_raw() == identity)
		{
			return;
		}
		command_buffer.retained_descriptor_heaps.push(heap.native.clone());
	}

	/// Retains a temporary GPU resource until the command buffer's previous submission has completed.
	pub(crate) fn retain_command_buffer_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: ID3D12Resource,
	) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		command_buffer.retained_resources.push(resource);
	}

	/// Retains an upload resource and tracks its live command-buffer-scoped allocation.
	pub(crate) fn retain_command_buffer_upload_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: ID3D12Resource,
	) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		command_buffer.retained_resources.push(resource);
		command_buffer.retained_upload_resource_count += 1;
	}
}
