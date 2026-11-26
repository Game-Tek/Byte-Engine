use super::*;

impl Device {
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

	/// Resolves the frame sequence that a frame offset selects relative to `sequence_index`.
	pub(crate) fn frame_index_with_offset(&self, sequence_index: u8, frame_offset: i32) -> u8 {
		crate::frame_resources::frame_index_with_offset(sequence_index as usize, frame_offset, self.frames as usize) as u8
	}

	/// Returns the frame copy of a descriptor set that frame sequence `sequence_index` binds, or the set itself when
	/// its chain has no copy that far.
	pub(crate) fn descriptor_set_for_sequence(
		&self,
		descriptor_set: DescriptorSetHandle,
		sequence_index: u8,
	) -> DescriptorSetHandle {
		let mut current = descriptor_set;
		for _ in 0..sequence_index {
			let Some(next) = self.descriptor_sets[current.index() as usize].next else {
				return descriptor_set;
			};
			current = DescriptorSetHandle::from_index(next.index());
		}
		current
	}

	/// Selects the shader synchronization scope for the active pipeline and command-list class.
	fn descriptor_sync(pipeline_kind: PipelineKind) -> D3D12_BARRIER_SYNC {
		match pipeline_kind {
			PipelineKind::Raster => D3D12_BARRIER_SYNC_ALL_SHADING,
			PipelineKind::Compute => D3D12_BARRIER_SYNC_COMPUTE_SHADING,
			PipelineKind::RayTracing => windows::Win32::Graphics::Direct3D12::D3D12_BARRIER_SYNC_RAYTRACING,
		}
	}

	pub(crate) fn descriptor_image_state(
		descriptor: ShaderResourceDescriptor,
		pipeline_kind: PipelineKind,
	) -> TextureBarrierState {
		let sync = Self::descriptor_sync(pipeline_kind);
		if descriptor.kind() == ResourceKind::StorageImage {
			TextureBarrierState::unordered_access(sync)
		} else {
			TextureBarrierState::shader_resource(sync)
		}
	}

	pub(crate) fn descriptor_buffer_state(
		descriptor: ShaderResourceDescriptor,
		pipeline_kind: PipelineKind,
	) -> BufferBarrierState {
		let sync = Self::descriptor_sync(pipeline_kind);
		match descriptor.kind() {
			ResourceKind::UniformBuffer => BufferBarrierState::constant_buffer(sync),
			ResourceKind::StorageBuffer if descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
				BufferBarrierState::unordered_access(sync)
			}
			_ => BufferBarrierState::shader_resource(sync),
		}
	}

	/// Maps one image descriptor kind to the logical image capability its native view requires.
	pub(crate) fn descriptor_image_use(descriptor: ShaderResourceDescriptor) -> Uses {
		match descriptor.kind() {
			ResourceKind::StorageImage => Uses::Storage,
			ResourceKind::InputAttachment => Uses::InputAttachment,
			ResourceKind::SampledImage | ResourceKind::CombinedImageSampler => Uses::Image,
			_ => Uses::empty(),
		}
	}

	/// Creates the first copy of a per-frame image when frame zero first records an image descriptor.
	pub(crate) fn materialize_descriptor_base_image_resource(
		&mut self,
		descriptor_set_handle: DescriptorSetHandle,
		descriptor: WriteData,
	) {
		if self.descriptor_sets[descriptor_set_handle.index() as usize].sequence_index != 0 {
			return;
		}
		let image_handle = match descriptor {
			WriteData::Image { handle, .. } => handle,
			WriteData::CombinedImageSampler { image_handle, .. } => image_handle,
			_ => return,
		};
		if self.images[image_handle.index() as usize].frame_resources.is_none() {
			return;
		}
		// Per-frame buffers keep sequence zero as the base resource; per-frame images need the same descriptor-visible anchor.
		let _ = self.ensure_image_resource_for_sequence(image_handle, 0);
	}

	pub(crate) fn write_native_buffer_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		handle: BaseBufferHandle,
		sequence_index: u8,
		slot: u32,
	) {
		let Some(resource) = self.buffer_resource_for_sequence(handle, sequence_index) else {
			return;
		};
		let buffer = self.buffer(handle);
		let buffer_size = buffer.size;
		let cpu_handle = self.shader_visible_cpu_handle(false, slot);
		match descriptor.kind() {
			ResourceKind::UniformBuffer => {
				let size_in_bytes = Self::constant_buffer_view_size(buffer, &resource, buffer_size);
				let desc = D3D12_CONSTANT_BUFFER_VIEW_DESC {
					BufferLocation: unsafe { resource.GetGPUVirtualAddress() },
					SizeInBytes: size_in_bytes,
				};
				unsafe { self.device.CreateConstantBufferView(Some(&desc), cpu_handle) };
			}
			ResourceKind::StorageBuffer => {
				let stride = descriptor.buffer_element_stride().max(1);
				let Ok(native_size) = usize::try_from(unsafe { resource.GetDesc() }.Width) else {
					return;
				};
				let stride_usize = stride as usize;
				let element_count = buffer_size.div_ceil(stride_usize).min(native_size / stride_usize);
				let Ok(element_count) = u32::try_from(element_count) else {
					return;
				};
				if element_count == 0 {
					return;
				}
				unsafe {
					if descriptor.access().intersects(crate::AccessPolicies::WRITE) {
						let desc = Self::structured_buffer_uav_desc(stride, element_count);
						self.device
							.CreateUnorderedAccessView(&resource, None::<&ID3D12Resource>, Some(&desc), cpu_handle);
					} else {
						let desc = Self::structured_buffer_srv_desc(stride, element_count);
						self.device.CreateShaderResourceView(&resource, Some(&desc), cpu_handle);
					}
				}
			}
			_ => return,
		}
	}

	/// Validates and rounds one logical uniform-buffer range for a native CBV.
	fn constant_buffer_view_size(buffer: &Buffer, resource: &ID3D12Resource, requested_size: usize) -> u32 {
		assert!(
			buffer.uses.intersects(Uses::Uniform),
			"Invalid DX12 uniform-buffer descriptor. The most likely cause is that the buffer was not created with uniform usage."
		);
		assert!(
			requested_size != 0,
			"Invalid DX12 constant-buffer view size. The most likely cause is that an empty logical buffer was bound as a uniform buffer. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/creating-descriptors#constant-buffer-view."
		);
		let aligned_size = requested_size
			.checked_add(255)
			.map(|size| size & !255)
			.expect(
				"Invalid DX12 constant-buffer view size. The most likely cause is that 256-byte alignment overflowed the addressable range. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/creating-descriptors#constant-buffer-view.",
			);
		assert!(
			aligned_size <= 64 * 1024,
			"Invalid DX12 constant-buffer view size. The most likely cause is that the requested range exceeds the 64-KiB shader-visible constant-buffer limit. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/constants."
		);
		let native_width = usize::try_from(unsafe { resource.GetDesc() }.Width).expect(
			"Invalid DX12 constant-buffer allocation width. The most likely cause is that the native resource exceeds the process addressable range.",
		);
		assert!(
			aligned_size <= native_width,
			"Invalid DX12 constant-buffer view range. The most likely cause is that the 256-byte-aligned descriptor exceeds the native buffer allocation. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/creating-descriptors#constant-buffer-view."
		);
		u32::try_from(aligned_size).expect(
			"Invalid DX12 constant-buffer view size. The most likely cause is that the aligned range exceeds the native 32-bit descriptor field.",
		)
	}

	/// Writes one native image descriptor using the active shader resource representation.
	pub(crate) fn write_native_image_descriptor(
		&mut self,
		descriptor: ShaderResourceDescriptor,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
		layer: Option<u32>,
		mip_level: Option<u32>,
		slot: u32,
	) {
		let cpu_handle = self.shader_visible_cpu_handle(false, slot);
		// An image without memory yet, such as a zero-sized image or an unplaced group member, gets a null view of its
		// format. Resizing or placing it marks its descriptors stale, so the view is written again when it has memory.
		let resource = self.ensure_image_resource_for_sequence(image_handle, sequence_index);
		let image = &self.images[image_handle.index() as usize];
		let format = Self::dxgi_shader_resource_format(image.format).unwrap_or_else(|| {
			panic!(
				"Unsupported DX12 descriptor image format. The most likely cause is that {:?} has no exact typed DXGI shader view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format.",
				image.format,
			)
		});
		let uses = image.uses;
		let extent = image.extent;
		let is_3d = image.is_3d;
		let array_layers = image.array_layers.max(1);
		unsafe {
			if descriptor.kind() == ResourceKind::StorageImage {
				assert!(
					uses.intersects(Uses::Storage),
					"Invalid DX12 storage-image descriptor. The most likely cause is that the image was not created with storage usage."
				);
				let desc = Self::descriptor_texture_uav_desc(
					format,
					descriptor.texture_view(),
					extent,
					is_3d,
					array_layers,
					layer,
					mip_level,
				);
				self.device
					.CreateUnorderedAccessView(resource.as_ref(), None::<&ID3D12Resource>, Some(&desc), cpu_handle);
			} else {
				let desc = Self::descriptor_texture_srv_desc(
					format,
					descriptor.texture_view(),
					is_3d,
					array_layers,
					layer,
					image.mip_levels,
					mip_level,
				);
				self.device
					.CreateShaderResourceView(resource.as_ref(), Some(&desc), cpu_handle);
			}
		}
	}

	/// Writes the native sampler of `sampler_handle` at `slot` of the shader-visible sampler heap.
	pub(crate) fn write_native_sampler_descriptor(&mut self, sampler_handle: SamplerHandle, slot: u32) {
		let cpu_handle = self.shader_visible_cpu_handle(true, slot);
		let sampler = self.samplers.get(sampler_handle.index() as usize).expect(
			"Invalid DX12 sampler descriptor. The most likely cause is that the sampler handle came from another device. Write samplers built by this device.",
		);
		let filter = Self::sampler_filter(
			sampler.filtering_mode,
			sampler.mip_map_mode,
			sampler.reduction_mode,
			sampler.anisotropy.is_some(),
		);
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
	}

	/// Encodes the D3D12 filter of a sampler, for the sampler heap and for the static samplers of a root signature.
	pub(crate) fn sampler_filter(
		filtering_mode: FilteringModes,
		mip_map_mode: FilteringModes,
		reduction_mode: SamplingReductionModes,
		anisotropic: bool,
	) -> D3D12_FILTER {
		if anisotropic {
			return match reduction_mode {
				SamplingReductionModes::WeightedAverage => D3D12_FILTER_ANISOTROPIC,
				SamplingReductionModes::Min => D3D12_FILTER_MINIMUM_ANISOTROPIC,
				SamplingReductionModes::Max => D3D12_FILTER_MAXIMUM_ANISOTROPIC,
			};
		}

		let min = match filtering_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let mag = min;
		let mip = match mip_map_mode {
			FilteringModes::Closest => 0,
			FilteringModes::Linear => 1,
		};
		let reduction = match reduction_mode {
			SamplingReductionModes::WeightedAverage => 0,
			SamplingReductionModes::Min => 2,
			SamplingReductionModes::Max => 3,
		};

		D3D12_FILTER(
			((min << D3D12_MIN_FILTER_SHIFT)
				| (mag << D3D12_MAG_FILTER_SHIFT)
				| (mip << D3D12_MIP_FILTER_SHIFT)
				| (reduction << D3D12_FILTER_REDUCTION_TYPE_SHIFT)) as i32,
		)
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
