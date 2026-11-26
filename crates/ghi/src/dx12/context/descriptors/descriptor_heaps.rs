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

	/// Creates one CPU-only descriptor heap.
	pub(crate) fn create_cpu_descriptor_heap(
		&self,
		heap_type: windows::Win32::Graphics::Direct3D12::D3D12_DESCRIPTOR_HEAP_TYPE,
		descriptor_count: u32,
	) -> DescriptorHeap {
		let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
			Type: heap_type,
			NumDescriptors: descriptor_count,
			Flags: Default::default(),
			NodeMask: 0,
		};
		let native = unsafe { self.device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) }.unwrap_or_else(|error| {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			let message = format!(
				"Failed to create a CPU-only DX12 descriptor heap: {error:?}. The most likely cause is descriptor heap exhaustion or device removal. Heap type: {heap_type:?}. Descriptor count: {descriptor_count}. Device removed reason: {removed_reason:?}"
			);
			self.log_dx12_error(&message);
			panic!("{message}");
		});
		DescriptorHeap {
			cpu_start: unsafe { native.GetCPUDescriptorHandleForHeapStart() },
			gpu_start: None,
			native,
		}
	}

	/// Allocates one retained CPU descriptor slot from stable, reusable heap pages.
	fn allocate_retained_cpu_descriptor(&mut self) -> RetainedCpuDescriptor {
		if let Some((page_index, slot)) = self.free_clear_uav_descriptor_slots.pop() {
			let heap = self.clear_uav_descriptor_pages[page_index].heap.clone();
			return RetainedCpuDescriptor { heap, page_index, slot };
		}

		if self
			.clear_uav_descriptor_pages
			.last()
			.is_none_or(|page| page.used >= page.capacity)
		{
			let capacity = 256;
			let heap = self.create_cpu_descriptor_heap(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, capacity);
			self.clear_uav_descriptor_pages
				.push(DescriptorHeapArena { heap, capacity, used: 0 });
		}

		let page_index = self.clear_uav_descriptor_pages.len() - 1;
		let page = &mut self.clear_uav_descriptor_pages[page_index];
		let slot = page.used;
		page.used += 1;
		RetainedCpuDescriptor {
			heap: page.heap.clone(),
			page_index,
			slot,
		}
	}

	/// Returns the GPU and CPU handles of a UAV over `resource` for one clear.
	///
	/// The CPU descriptor is retained per resource in a CPU-only heap, as clears require; its copy takes the next
	/// descriptor of the command buffer's clear range in the shader-visible heap.
	pub(crate) fn clear_uav_descriptors(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		resource: &ID3D12Resource,
		description: &D3D12_UNORDERED_ACCESS_VIEW_DESC,
	) -> (D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_CPU_DESCRIPTOR_HANDLE) {
		let resource_key = Self::native_resource_key(resource);
		let descriptor = match self.retained_clear_uav_descriptors.get(&resource_key) {
			Some(descriptor) => descriptor.clone(),
			None => {
				let descriptor = self.allocate_retained_cpu_descriptor();
				let cpu_handle =
					self.descriptor_cpu_handle(&descriptor.heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor.slot);
				unsafe {
					self.device
						.CreateUnorderedAccessView(resource, None::<&ID3D12Resource>, Some(description), cpu_handle);
				}
				self.retained_clear_uav_descriptors.insert(resource_key, descriptor.clone());
				descriptor
			}
		};
		let cpu = self.descriptor_cpu_handle(&descriptor.heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, descriptor.slot);
		let slot = self.reserve_clear_descriptor(command_buffer_handle);
		let destination = self.shader_visible_cpu_handle(false, slot);
		unsafe {
			self.device
				.CopyDescriptorsSimple(1, destination, cpu, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
		};
		let gpu = self.descriptor_gpu_handle(
			&self.shader_visible_heaps.resources,
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
			slot,
		);
		(gpu, cpu)
	}

	/// Forgets everything cached by the native resource keys of released or replaced resources: their attachment
	/// views, clear descriptors, and barrier states. A later resource at the same address would otherwise match them.
	pub(crate) fn forget_native_resources(&mut self, resources: &[usize]) {
		if resources.is_empty() {
			return;
		}
		self.attachment_views.retain(|key, _| !resources.contains(&key.resource));
		for resource in resources {
			if let Some(descriptor) = self.retained_clear_uav_descriptors.remove(resource) {
				self.free_clear_uav_descriptor_slots
					.push((descriptor.page_index, descriptor.slot));
			}
			self.image_states.remove(resource);
			self.buffer_states.remove(resource);
		}
	}

	/// Creates the shader-visible resource and sampler heaps of a context, and writes the fixed samplers at the start
	/// of the sampler heap, where shaders index them by their state.
	pub(crate) fn create_shader_visible_heaps(device: &ID3D12Device10, resource_capacity: u32) -> ShaderVisibleHeaps {
		let create = |heap_type, descriptor_count| {
			let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
				Type: heap_type,
				NumDescriptors: descriptor_count,
				Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
				NodeMask: 0,
			};
			let native = unsafe { device.CreateDescriptorHeap::<ID3D12DescriptorHeap>(&heap_desc) }.unwrap_or_else(|error| {
				panic!(
					"Failed to create a shader-visible DX12 descriptor heap of {descriptor_count} descriptors. The most likely cause is that the device is out of memory or was removed. Close other GPU applications and restart. Heap type: {heap_type:?}. Error: {error:?}"
				)
			});
			DescriptorHeap {
				cpu_start: unsafe { native.GetCPUDescriptorHandleForHeapStart() },
				gpu_start: Some(unsafe { native.GetGPUDescriptorHandleForHeapStart() }),
				native,
			}
		};
		let resources = create(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, resource_capacity);
		let samplers = create(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, SAMPLER_HEAP_CAPACITY);

		let sampler_stride = unsafe { device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER) } as usize;
		for index in 0..FIXED_SAMPLER_COUNT {
			let desc = Self::fixed_sampler_desc(index);
			let mut handle = samplers.cpu_start;
			handle.ptr += index as usize * sampler_stride;
			unsafe { device.CreateSampler(&desc, handle) };
		}

		let mut free_samplers = ::utils::RangeAllocator::new(SAMPLER_HEAP_CAPACITY as usize, 16);
		// The fixed samplers stay for the context's lifetime.
		free_samplers.take(FIXED_SAMPLER_COUNT as usize, 1);
		ShaderVisibleHeaps {
			resources,
			samplers,
			free_resources: ::utils::RangeAllocator::new(resource_capacity as usize, 64),
			free_samplers,
			resource_capacity,
		}
	}

	/// Describes the fixed sampler at sampler heap `index`, as [`crate::sampler::StaticSampler::fixed`] lays them out.
	///
	/// A fixed sampler reads every level of the image, so it doesn't clamp the level of detail.
	fn fixed_sampler_desc(index: u32) -> D3D12_SAMPLER_DESC {
		let sampler = crate::sampler::StaticSampler::fixed(index);
		let address = Self::sampler_address_mode(sampler.addressing_mode);
		D3D12_SAMPLER_DESC {
			Filter: Self::sampler_filter(
				sampler.filtering_mode,
				sampler.mip_map_mode,
				sampler.reduction_mode,
				sampler.max_anisotropy > 1,
			),
			AddressU: address,
			AddressV: address,
			AddressW: address,
			MipLODBias: 0.0,
			MaxAnisotropy: u32::from(sampler.max_anisotropy),
			ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
			BorderColor: [0.0, 0.0, 0.0, 0.0],
			MinLOD: 0.0,
			MaxLOD: D3D12_FLOAT32_MAX,
		}
	}

	/// Leases `count` contiguous descriptors of the shader-visible resource heap, or of the sampler heap when
	/// `sampler_heap` is set, and returns the index of the first.
	///
	/// # Panics
	///
	/// Panics when the heap has no free range that large.
	pub(crate) fn lease_shader_visible_range(&mut self, sampler_heap: bool, count: u32) -> u32 {
		let heaps = &mut self.shader_visible_heaps;
		let (free, capacity) = if sampler_heap {
			(&mut heaps.free_samplers, SAMPLER_HEAP_CAPACITY)
		} else {
			(&mut heaps.free_resources, heaps.resource_capacity)
		};
		let range = free.take(count as usize, 1).unwrap_or_else(|| {
			panic!(
				"The DX12 shader-visible {} heap has no room for {count} more descriptors. The most likely cause is that the application keeps more descriptor sets, frame copies, or array elements alive than the heap's {capacity} descriptors hold. Create fewer or smaller descriptor sets, or run fewer frames in flight.",
				if sampler_heap { "sampler" } else { "resource" },
			)
		});
		range.start as u32
	}

	/// Returns the CPU handle of one descriptor of a shader-visible heap.
	pub(crate) fn shader_visible_cpu_handle(&self, sampler_heap: bool, index: u32) -> D3D12_CPU_DESCRIPTOR_HANDLE {
		let (heap, heap_type) = if sampler_heap {
			(&self.shader_visible_heaps.samplers, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)
		} else {
			(&self.shader_visible_heaps.resources, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
		};
		self.descriptor_cpu_handle(heap, heap_type, index)
	}

	/// Reserves one descriptor of the command buffer's clear range in the shader-visible resource heap, growing the
	/// range when the recording has used all of it, and returns its heap index.
	pub(crate) fn reserve_clear_descriptor(&mut self, command_buffer_handle: CommandBufferHandle) -> u32 {
		let index = command_buffer_handle.index() as usize;
		let (capacity, used) = self.command_buffers[index]
			.clear_descriptors
			.as_ref()
			.map_or((0, 0), |lease| (lease.range.len() as u32, lease.used));
		if used == capacity {
			// This recording may still read the old range, so it goes back once the recording's submission completes.
			let capacity = (capacity * 2).max(64);
			let start = self.lease_shader_visible_range(false, capacity) as usize;
			let command_buffer = &mut self.command_buffers[index];
			if let Some(previous) = command_buffer.clear_descriptors.replace(DescriptorLease {
				range: start..start + capacity as usize,
				used: 0,
			}) {
				command_buffer.retired_clear_descriptors.push(previous.range);
			}
		}
		let lease = self.command_buffers[index]
			.clear_descriptors
			.as_mut()
			.expect("A clear descriptor range was leased above");
		let slot = lease.range.start as u32 + lease.used;
		lease.used += 1;
		slot
	}

	/// Binds the context's shader-visible heaps to a command list that just opened.
	///
	/// Shaders index both heaps directly, and a root signature that allows it requires the heaps to be set before it,
	/// so every recording sets them first and never changes them.
	pub(crate) fn bind_shader_visible_heaps(&mut self, command_list: &ID3D12GraphicsCommandList7) {
		let heaps = [
			Some(self.shader_visible_heaps.resources.native.clone()),
			Some(self.shader_visible_heaps.samplers.native.clone()),
		];
		unsafe { command_list.SetDescriptorHeaps(&heaps) };
	}

	/// Retains a descriptor heap until the command buffer's previous submission has completed.
	pub(crate) fn retain_descriptor_heap(&mut self, command_buffer_handle: CommandBufferHandle, heap: &DescriptorHeap) {
		let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
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
		resource: &ID3D12Resource,
	) {
		let command_buffer = &mut self.command_buffers[command_buffer_handle.index() as usize];
		let identity = resource.as_raw() as usize;
		if !command_buffer.retained_resource_keys.insert(identity) {
			return;
		}
		command_buffer.retained_resources.push(resource.clone());
	}
}
