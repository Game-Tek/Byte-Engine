use super::*;

impl Device {
	/// Collects the per-frame descriptor set handles chained from the root handle.
	pub(crate) fn collect_descriptor_set_handles(&self, handle: DescriptorSetHandle) -> Vec<DescriptorSetHandle> {
		let mut handles = Vec::new();
		let mut current = Some(handle);

		while let Some(handle) = current {
			let Some(set) = self.descriptor_sets.get(handle.0 as usize) else {
				break;
			};
			handles.push(handle);
			current = set.next.map(|handle| DescriptorSetHandle(handle.0));
		}

		handles
	}

	pub(crate) fn query_window_extent(handles: &window::Handles, fallback_extent: Extent) -> Extent {
		let mut rect = RECT::default();
		let ok = unsafe { GetClientRect(handles.hwnd, &mut rect) }.is_ok();

		if !ok {
			return fallback_extent;
		}

		let width = (rect.right - rect.left).max(0) as u32;
		let height = (rect.bottom - rect.top).max(0) as u32;

		if width == 0 || height == 0 {
			fallback_extent
		} else {
			Extent::rectangle(width, height)
		}
	}

	/// Resolves a frame-aware index using the optional frame offset.
	pub(crate) fn frame_index_with_offset(&self, frame_index: usize, frame_offset: Option<i32>, total_frames: usize) -> usize {
		crate::frame_resources::frame_index_with_offset(frame_index, frame_offset.unwrap_or(0), total_frames)
	}

	pub(crate) fn descriptor_set_for_sequence(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
	) -> Option<DescriptorSetHandle> {
		let mut current = Some(descriptor_set);
		for _ in 0..sequence_index {
			let handle = current?;
			let set = self.descriptor_sets.get(handle.0 as usize)?;
			current = set.next.map(|handle| DescriptorSetHandle(handle.0));
		}
		current.or(Some(descriptor_set))
	}

	pub(crate) fn descriptor_set_sequence_index(&self, descriptor_set: DescriptorSetHandle) -> usize {
		for root_index in 0..self.descriptor_sets.len() {
			let mut sequence_index = 0;
			let mut current = Some(DescriptorSetHandle(root_index as u64));
			while let Some(handle) = current {
				if handle == descriptor_set {
					return sequence_index;
				}
				let Some(set) = self.descriptor_sets.get(handle.0 as usize) else {
					break;
				};
				current = set.next.map(|handle| DescriptorSetHandle(handle.0));
				sequence_index += 1;
			}
		}
		0
	}

	#[cfg(test)]
	pub(crate) fn descriptor_sequence_index(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
		slot: ResourceSlot,
	) -> Option<usize> {
		let descriptor_set = self.descriptor_set_for_sequence(descriptor_set, sequence_index)?;
		let descriptors = self.descriptor_sets[descriptor_set.0 as usize].descriptors.get(&slot)?;
		let retained = descriptors.get(&0).or_else(|| descriptors.values().next())?;
		Some(self.frame_index_with_offset(sequence_index as usize, Some(retained.frame_offset), self.frames as usize))
	}

	/// Selects shader-resource states that are legal for the active pipeline's command-list class.
	fn descriptor_read_state(pipeline_kind: PipelineKind) -> D3D12_RESOURCE_STATES {
		if matches!(pipeline_kind, PipelineKind::Raster) {
			D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
		} else {
			// Compute command lists reject PIXEL_SHADER_RESOURCE even when it is combined with NON_PIXEL_SHADER_RESOURCE.
			D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE
		}
	}

	pub(crate) fn descriptor_image_state(
		descriptor: ShaderResourceDescriptor,
		pipeline_kind: PipelineKind,
	) -> D3D12_RESOURCE_STATES {
		if descriptor.kind() == ResourceKind::StorageImage {
			D3D12_RESOURCE_STATE_UNORDERED_ACCESS
		} else {
			Self::descriptor_read_state(pipeline_kind)
		}
	}

	pub(crate) fn descriptor_buffer_state(
		descriptor: ShaderResourceDescriptor,
		pipeline_kind: PipelineKind,
	) -> D3D12_RESOURCE_STATES {
		match descriptor.kind() {
			ResourceKind::UniformBuffer => D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
			ResourceKind::StorageBuffer if descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
				D3D12_RESOURCE_STATE_UNORDERED_ACCESS
			}
			_ => Self::descriptor_read_state(pipeline_kind),
		}
	}

	pub(crate) fn image_data_mut_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> Option<&mut [u8]> {
		let image = self.images.get_mut(image_handle.0 as usize)?;
		if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index).map(Vec::as_mut_slice)
		} else {
			image.data.as_deref_mut()
		}
	}

	/// Creates the base dynamic image resource when frame zero first records an image descriptor.
	pub(crate) fn materialize_descriptor_base_image_resource(
		&mut self,
		descriptor_set_handle: DescriptorSetHandle,
		descriptor: WriteData,
	) {
		if self.descriptor_set_sequence_index(descriptor_set_handle) != 0 {
			return;
		}
		let image_handle = match descriptor {
			WriteData::Image { handle, .. } => handle,
			WriteData::CombinedImageSampler { image_handle, .. } => image_handle,
			_ => return,
		};
		let Some(image) = self.images.get(image_handle.0 as usize) else {
			return;
		};
		if image.frame_resources.is_none() {
			return;
		}
		// Dynamic buffers keep sequence zero as the base resource; dynamic images need the same descriptor-visible anchor.
		let _ = self.ensure_image_resource_for_sequence(image_handle, 0);
	}

	pub(crate) fn write_native_descriptor_for_heap(
		&mut self,
		resource: PipelineResource,
		retained: RetainedDescriptor,
		array_element: u32,
		sequence_index: u8,
		sampler_heap: bool,
		heap: &DescriptorHeap,
		base_offset: u32,
	) {
		if array_element >= resource.descriptor.count() {
			return;
		}
		let offset = if sampler_heap {
			resource.sampler_offset
		} else {
			resource.cbv_srv_uav_offset
		};
		let Some(offset) = offset else {
			return;
		};
		let slot = base_offset + offset + array_element;
		let resource_sequence =
			self.frame_index_with_offset(sequence_index as usize, Some(retained.frame_offset), self.frames as usize) as u8;

		if sampler_heap {
			let sampler = match retained.descriptor {
				WriteData::CombinedImageSampler { sampler_handle, .. } | WriteData::Sampler(sampler_handle) => {
					Some(sampler_handle)
				}
				_ => None,
			};
			if sampler.is_some() {
				self.write_native_sampler_descriptor(sampler, heap, slot);
			}
			return;
		}

		match retained.descriptor {
			WriteData::Buffer { handle, size } => {
				self.write_native_buffer_descriptor(resource.descriptor, handle, size, resource_sequence, heap, slot)
			}
			WriteData::Image { handle, mip_level, .. } => {
				self.write_native_image_descriptor(resource.descriptor, handle, resource_sequence, None, mip_level, heap, slot)
			}
			WriteData::CombinedImageSampler { image_handle, layer, .. } => self.write_native_image_descriptor(
				resource.descriptor,
				image_handle,
				resource_sequence,
				layer,
				None,
				heap,
				slot,
			),
			WriteData::Swapchain(handle) => {
				let image = self
					.get_swapchain_image_for_sequence(handle, Uses::Storage, resource_sequence)
					.0;
				self.write_native_image_descriptor(
					resource.descriptor,
					image.into(),
					resource_sequence,
					None,
					None,
					heap,
					slot,
				);
			}
			WriteData::AccelerationStructure { handle } => {
				self.write_native_acceleration_structure_descriptor(handle, heap, slot)
			}
			_ => {}
		}
	}

	pub(crate) fn write_native_buffer_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		handle: BaseBufferHandle,
		size: crate::Ranges,
		sequence_index: u8,
		heap: &DescriptorHeap,
		slot: u32,
	) {
		// Descriptor reads should include CPU writes made through the host shadow before the bind.
		self.sync_buffer_for_sequence(handle, sequence_index);
		let Some(resource) = self.buffer_resource_for_sequence(handle, sequence_index) else {
			return;
		};
		let Some(buffer) = self.buffer(handle) else {
			return;
		};
		let buffer_size = match size {
			crate::Ranges::Size(size) => size.min(buffer.size),
			crate::Ranges::Whole => buffer.size,
		};
		let heap_kind = self
			.buffer_heap_kind_for_sequence(handle, sequence_index)
			.unwrap_or(buffer.heap_kind);
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		match descriptor.kind() {
			ResourceKind::UniformBuffer => {
				let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
					BufferLocation: unsafe { resource.GetGPUVirtualAddress() },
					SizeInBytes: Self::align_up(buffer_size.max(1), 256) as u32,
				};
				unsafe { self.device.CreateConstantBufferView(Some(&desc), cpu_handle) };
			}
			ResourceKind::StorageBuffer => {
				let stride = descriptor.buffer_element_stride().max(1);
				if descriptor.access().intersects(crate::AccessPolicies::WRITE) {
					let desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
						Format: DXGI_FORMAT_UNKNOWN,
						ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
						Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
							Buffer: D3D12_BUFFER_UAV {
								FirstElement: 0,
								NumElements: (buffer_size / stride as usize).max(1) as u32,
								StructureByteStride: stride,
								CounterOffsetInBytes: 0,
								Flags: D3D12_BUFFER_UAV_FLAG_NONE,
							},
						},
					};
					unsafe {
						if heap_kind == BufferHeapKind::Default {
							self.device
								.CreateUnorderedAccessView(&resource, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
						} else {
							self.device.CreateUnorderedAccessView(
								None::<&ID3D12Resource>,
								None::<&ID3D12Resource>,
								Some(&desc),
								cpu_handle,
							);
						}
					}
				} else {
					let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
						Format: DXGI_FORMAT_UNKNOWN,
						ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
						Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
						Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
							Buffer: D3D12_BUFFER_SRV {
								FirstElement: 0,
								NumElements: (buffer_size / stride as usize).max(1) as u32,
								StructureByteStride: stride,
								Flags: D3D12_BUFFER_SRV_FLAG_NONE,
							},
						},
					};
					unsafe { self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle) };
				}
			}
			_ => return,
		}
		self.descriptor_write_count += 1;
	}

	pub(crate) fn write_native_acceleration_structure_descriptor(
		&mut self,
		handle: TopLevelAccelerationStructureHandle,
		heap: &DescriptorHeap,
		slot: u32,
	) {
		let Some(acceleration_structure) = self.top_level_acceleration_structures.get(handle.0 as usize) else {
			return;
		};
		let Some(resource) = acceleration_structure.resource.as_ref() else {
			return;
		};
		let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV {
					Location: unsafe { resource.GetGPUVirtualAddress() },
				},
			},
		};
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		unsafe {
			self.device
				.CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), cpu_handle);
		}
		self.descriptor_write_count += 1;
		self.acceleration_structure_descriptor_write_count += 1;
	}

	/// Writes one native image descriptor using the active shader resource representation.
	pub(crate) fn write_native_image_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
		layer: Option<u32>,
		mip_level: Option<u32>,
		heap: &DescriptorHeap,
		slot: u32,
	) {
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, slot);
		let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 as usize) else {
			return;
		};
		let Some(format) = Self::dxgi_shader_resource_format(image.format) else {
			return;
		};
		let uses = image.uses;
		let array_layers = image.array_layers.max(1);
		unsafe {
			if descriptor.kind() == ResourceKind::StorageImage {
				let desc = Self::descriptor_texture_uav_desc(format, descriptor.texture_view(), array_layers, layer, mip_level);
				if uses.intersects(Uses::Storage) {
					self.device
						.CreateUnorderedAccessView(&resource, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
				} else {
					self.device.CreateUnorderedAccessView(
						None::<&ID3D12Resource>,
						None::<&ID3D12Resource>,
						Some(&desc),
						cpu_handle,
					);
				}
				self.image_uav_descriptor_write_count += 1;
			} else {
				let desc = Self::descriptor_texture_srv_desc(
					format,
					descriptor.texture_view(),
					array_layers,
					layer,
					image.mip_levels,
					mip_level,
				);
				self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle);
				self.image_srv_descriptor_write_count += 1;
			}
		}
		self.descriptor_write_count += 1;
	}

	pub(crate) fn write_native_sampler_descriptor(
		&mut self,
		sampler_handle: Option<SamplerHandle>,
		heap: &DescriptorHeap,
		slot: u32,
	) {
		let cpu_handle = self.descriptor_cpu_handle(heap, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, slot);
		let fallback_sampler = Sampler {
			filtering_mode: FilteringModes::Linear,
			reduction_mode: SamplingReductionModes::WeightedAverage,
			mip_map_mode: FilteringModes::Linear,
			addressing_mode: SamplerAddressingModes::Clamp,
			anisotropy: None,
			min_lod: 0.0,
			max_lod: 0.0,
		};
		let sampler = sampler_handle
			.and_then(|handle| self.samplers.get(handle.0 as usize))
			.unwrap_or(&fallback_sampler);
		let filter = Self::sampler_filter(sampler);
		let address_mode = Self::sampler_address_mode(sampler.addressing_mode);
		let max_anisotropy = sampler.anisotropy.unwrap_or(1.0).clamp(1.0, 16.0).round() as u32;
		let desc = D3D12_SAMPLER_DESC {
			Filter: filter,
			AddressU: address_mode,
			AddressV: address_mode,
			AddressW: address_mode,
			MipLODBias: 0.0,
			MaxAnisotropy: max_anisotropy,
			ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
			BorderColor: [0.0, 0.0, 0.0, 0.0],
			MinLOD: sampler.min_lod,
			MaxLOD: sampler.max_lod,
		};
		unsafe {
			self.device.CreateSampler(&desc, cpu_handle);
		}
		#[cfg(test)]
		{
			self.sampler_descriptor_write_records.push(SamplerDescriptorWriteRecord {
				filter,
				address_mode,
				max_anisotropy,
				min_lod: sampler.min_lod,
				max_lod: sampler.max_lod,
			});
		}
		self.descriptor_write_count += 1;
	}

	pub(crate) fn sampler_filter(sampler: &Sampler) -> D3D12_FILTER {
		if sampler.anisotropy.is_some() {
			return match sampler.reduction_mode {
				SamplingReductionModes::WeightedAverage => D3D12_FILTER_ANISOTROPIC,
				SamplingReductionModes::Min => D3D12_FILTER_MINIMUM_ANISOTROPIC,
				SamplingReductionModes::Max => D3D12_FILTER_MAXIMUM_ANISOTROPIC,
			};
		}

		let min = match sampler.filtering_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let mag = min;
		let mip = match sampler.mip_map_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let reduction = match sampler.reduction_mode {
			SamplingReductionModes::WeightedAverage => 0,
			SamplingReductionModes::Min => 2,
			SamplingReductionModes::Max => 3,
		};

		D3D12_FILTER(min | (mag << 2) | (mip << 4) | (reduction << 7))
	}

	pub(crate) fn sampler_address_mode(addressing_mode: SamplerAddressingModes) -> D3D12_TEXTURE_ADDRESS_MODE {
		match addressing_mode {
			SamplerAddressingModes::Repeat => D3D12_TEXTURE_ADDRESS_MODE_WRAP,
			SamplerAddressingModes::Mirror => D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
			SamplerAddressingModes::Clamp => D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			SamplerAddressingModes::Border {} => D3D12_TEXTURE_ADDRESS_MODE_BORDER,
		}
	}
}
