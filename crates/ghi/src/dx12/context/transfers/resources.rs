use super::*;

impl Device {
	pub(crate) fn create_buffer_with_layout(
		&mut self,
		layout: Layout,
		resource_uses: Uses,
		device_accesses: DeviceAccesses,
		storage_kind: BufferStorage,
	) -> u64 {
		// Allocates CPU storage for a buffer with the requested layout.
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to allocate buffer storage. The most likely cause is that the system is out of memory.");
		}

		let resource_size = Self::buffer_resource_size(layout.size(), resource_uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, device_accesses);
		let frame_resources = match storage_kind {
			BufferStorage::Static => None,
			BufferStorage::Dynamic => Some((0..self.frames as usize).map(|_| None).collect()),
		};
		let buffer = Buffer {
			data,
			layout,
			size: layout.size(),
			uses: resource_uses,
			access: device_accesses,
			resource,
			mapped,
			heap_kind,
			frame_resources,
		};

		let storage = match storage_kind {
			BufferStorage::Static => &mut self.buffers,
			BufferStorage::Dynamic => &mut self.dynamic_buffers,
		};
		storage.push(buffer);

		let index = (storage.len() - 1) as u64;
		match storage_kind {
			BufferStorage::Static => index,
			BufferStorage::Dynamic => DYNAMIC_BUFFER_HANDLE_FLAG | index,
		}
	}

	pub(crate) fn buffer_index(buffer_handle: BaseBufferHandle) -> (usize, bool) {
		(
			(buffer_handle.0 & !DYNAMIC_BUFFER_HANDLE_FLAG) as usize,
			buffer_handle.0 & DYNAMIC_BUFFER_HANDLE_FLAG != 0,
		)
	}

	pub(crate) fn buffer(&self, buffer_handle: BaseBufferHandle) -> Option<&Buffer> {
		let (index, dynamic) = Self::buffer_index(buffer_handle);
		if dynamic {
			self.dynamic_buffers.get(index)
		} else {
			self.buffers.get(index)
		}
	}

	pub(crate) fn buffer_mut(&mut self, buffer_handle: BaseBufferHandle) -> Option<&mut Buffer> {
		let (index, dynamic) = Self::buffer_index(buffer_handle);
		if dynamic {
			self.dynamic_buffers.get_mut(index)
		} else {
			self.buffers.get_mut(index)
		}
	}

	pub(crate) fn ensure_buffer_frame_storage(&mut self, buffer_handle: BaseBufferHandle, sequence_index: u8) {
		let (_, dynamic) = Self::buffer_index(buffer_handle);
		if !dynamic || sequence_index == 0 {
			return;
		}

		let (layout, access, uses) = match self.buffer(buffer_handle) {
			Some(buffer) if buffer.frame_resources.is_some() => (buffer.layout, buffer.access, buffer.uses),
			_ => return,
		};
		let frame_index = sequence_index as usize;
		let needs_storage = self
			.buffer(buffer_handle)
			.and_then(|buffer| buffer.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(|resource| resource.as_ref())
			.is_none();
		if !needs_storage {
			return;
		}

		let frame_storage = self.create_buffer_frame_storage(layout, access, uses);
		let Some(buffer) = self.buffer_mut(buffer_handle) else {
			return;
		};
		let Some(resources) = buffer.frame_resources.as_mut() else {
			return;
		};
		if resources.len() <= frame_index {
			resources.resize_with(frame_index + 1, || None);
		}
		resources[frame_index] = Some(frame_storage);
	}

	pub(crate) fn buffer_resource_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return buffer.resource.clone();
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.and_then(|resource| resource.resource.clone())
			.or_else(|| buffer.resource.clone())
	}

	pub(crate) fn buffer_heap_kind_for_sequence(
		&self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<BufferHeapKind> {
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return Some(buffer.heap_kind);
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.map(|resource| resource.heap_kind)
			.or(Some(buffer.heap_kind))
	}

	pub(crate) fn buffer_storage_parts_for_sequence(
		&self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<(*const u8, usize)> {
		let buffer = self.buffer(buffer_handle)?;
		if sequence_index == 0 {
			return Some((buffer.data.cast_const(), buffer.size));
		}
		buffer
			.frame_resources
			.as_ref()
			.and_then(|resources| resources.get(sequence_index as usize))
			.and_then(|resource| resource.as_ref())
			.map(|resource| (resource.data.cast_const(), buffer.size))
			.or(Some((buffer.data.cast_const(), buffer.size)))
	}

	pub(crate) fn buffer_storage_parts_mut_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<(*mut u8, usize)> {
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		let buffer = self.buffer_mut(buffer_handle)?;
		if sequence_index == 0 {
			return Some((buffer.data, buffer.size));
		}
		let size = buffer.size;
		buffer
			.frame_resources
			.as_mut()
			.and_then(|resources| resources.get_mut(sequence_index as usize))
			.and_then(|resource| resource.as_mut())
			.map(|resource| (resource.data, size))
			.or(Some((buffer.data, size)))
	}

	pub(crate) fn create_buffer_frame_storage(&self, layout: Layout, access: DeviceAccesses, uses: Uses) -> BufferFrameStorage {
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to allocate buffer storage. The most likely cause is that the system is out of memory.");
		}

		let resource_size = Self::buffer_resource_size(layout.size(), uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, access);
		BufferFrameStorage {
			data,
			layout,
			resource,
			mapped,
			heap_kind,
		}
	}

	/// Rounds uniform allocations to the full range exposed by their aligned CBVs.
	pub(crate) fn buffer_resource_size(size: usize, uses: Uses) -> usize {
		if uses.intersects(Uses::Uniform) {
			Self::align_up(size.max(1), 256)
		} else {
			size
		}
	}

	pub(crate) fn create_buffer_resource(
		&self,
		size: usize,
		device_accesses: DeviceAccesses,
	) -> (Option<ID3D12Resource>, *mut u8, BufferHeapKind) {
		if size == 0 {
			return (None, std::ptr::null_mut(), BufferHeapKind::Default);
		}

		let host_write = device_accesses.intersects(DeviceAccesses::CpuWrite);
		let host_read = device_accesses.intersects(DeviceAccesses::CpuRead);
		let heap_kind = if host_write {
			BufferHeapKind::Upload
		} else if host_read {
			BufferHeapKind::Readback
		} else {
			BufferHeapKind::Default
		};
		let heap_type = match heap_kind {
			BufferHeapKind::Default => D3D12_HEAP_TYPE_DEFAULT,
			BufferHeapKind::Upload => D3D12_HEAP_TYPE_UPLOAD,
			BufferHeapKind::Readback => D3D12_HEAP_TYPE_READBACK,
		};
		let initial_state: D3D12_RESOURCE_STATES = match heap_kind {
			BufferHeapKind::Upload => D3D12_RESOURCE_STATE_GENERIC_READ,
			BufferHeapKind::Readback => D3D12_RESOURCE_STATE_COPY_DEST,
			BufferHeapKind::Default => D3D12_RESOURCE_STATE_COMMON,
		};
		let cpu_visible = host_write || host_read;
		let resource_flags = if heap_kind == BufferHeapKind::Default {
			D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS
		} else {
			D3D12_RESOURCE_FLAG_NONE
		};
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: heap_type,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC {
			Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
			Alignment: 0,
			Width: size.max(1) as u64,
			Height: 1,
			DepthOrArraySize: 1,
			MipLevels: 1,
			Format: DXGI_FORMAT_UNKNOWN,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
			Flags: resource_flags,
		};

		let mut resource: Option<ID3D12Resource> = None;
		let result = unsafe {
			self.device.CreateCommittedResource(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				initial_state,
				None,
				&mut resource,
			)
		};
		if result.is_err() {
			return (None, std::ptr::null_mut(), heap_kind);
		}

		let mapped = if cpu_visible {
			let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
			let read_range = if heap_kind == BufferHeapKind::Readback {
				D3D12_RANGE { Begin: 0, End: size }
			} else {
				D3D12_RANGE { Begin: 0, End: 0 }
			};
			if let Some(resource) = resource.as_ref() {
				let result = unsafe { resource.Map(0, Some(&read_range), Some(&mut mapped)) };
				if result.is_err() {
					std::ptr::null_mut()
				} else {
					mapped.cast::<u8>()
				}
			} else {
				std::ptr::null_mut()
			}
		} else {
			std::ptr::null_mut()
		};

		(resource, mapped, heap_kind)
	}

	pub(crate) fn create_image_resource(
		&self,
		extent: Extent,
		format: Formats,
		uses: Uses,
		array_layers: u32,
		mip_levels: u32,
		optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
	) -> Option<ID3D12Resource> {
		let dxgi_format = Self::dxgi_resource_format(format, uses)?;
		if extent.width() == 0 || extent.height() == 0 {
			return None;
		}

		let flags = Self::image_resource_flags(format, uses);
		let depth_or_array_size = u16::try_from(array_layers.max(1)).expect(
			"Invalid DX12 image array size. The most likely cause is that the layer count exceeds the native 16-bit limit.",
		);
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC {
			Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
			Alignment: 0,
			Width: extent.width().max(1) as u64,
			Height: extent.height().max(1),
			DepthOrArraySize: depth_or_array_size,
			MipLevels: u16::try_from(mip_levels).expect(
				"Invalid DX12 mip count. The most likely cause is that the image metadata exceeds the native 16-bit limit.",
			),
			Format: dxgi_format,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
			Flags: flags,
		};
		let mut resource = None;
		let result = unsafe {
			self.device.CreateCommittedResource(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_RESOURCE_STATE_COMMON,
				optimized_clear_value.as_ref().map(|clear_value| clear_value as *const _),
				&mut resource,
			)
		};
		if let Err(error) = result {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 image resource. Format: {:?}. Extent: {:?}. Uses: {:?}. Array layers: {}. Error: {error:?}. Device removed reason: {removed_reason:?}",
				format,
				extent,
				uses,
				array_layers
			));
			None
		} else {
			resource
		}
	}

	pub(crate) fn image_resource_flags(format: Formats, uses: Uses) -> D3D12_RESOURCE_FLAGS {
		let mut flags = D3D12_RESOURCE_FLAG_NONE;
		if uses.intersects(Uses::RenderTarget) && !format.is_depth() {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
		}
		if uses.intersects(Uses::DepthStencil) || format.is_depth() {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
		}
		if uses.intersects(Uses::Storage) {
			flags |= D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
		}
		flags
	}

	pub(crate) fn optimized_image_clear_value(
		format: Formats,
		flags: D3D12_RESOURCE_FLAGS,
		clear: ClearValue,
	) -> Option<D3D12_CLEAR_VALUE> {
		if flags.contains(D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL) && format.is_depth() {
			let depth = match clear {
				ClearValue::Depth(depth) => depth,
				_ => 0.0,
			};
			return Some(D3D12_CLEAR_VALUE {
				Format: Self::dxgi_format(format).expect("Depth formats require a DX12 DSV format."),
				Anonymous: D3D12_CLEAR_VALUE_0 {
					DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
						Depth: depth,
						Stencil: 0,
					},
				},
			});
		}

		if flags.contains(D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET) {
			return Some(D3D12_CLEAR_VALUE {
				Format: Self::dxgi_format(format)?,
				Anonymous: D3D12_CLEAR_VALUE_0 {
					Color: Self::clear_color_f32(clear),
				},
			});
		}

		None
	}

	/// Creates an RTV description for one selected layer or a shader-selected layer prefix.
	pub(crate) fn render_target_view_desc(
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
		layer_count: u32,
	) -> D3D12_RENDER_TARGET_VIEW_DESC {
		Self::validate_attachment_layers(array_layers, layer, layer_count);
		let format = Self::dxgi_format(format).expect(
			"Unsupported DX12 render-target format. The most likely cause is that the attachment uses a format without a native RTV mapping.",
		);
		D3D12_RENDER_TARGET_VIEW_DESC {
			Format: format,
			ViewDimension: if array_layers > 1 {
				D3D12_RTV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_RTV_DIMENSION_TEXTURE2D
			},
			Anonymous: if array_layers > 1 {
				D3D12_RENDER_TARGET_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_RTV {
						MipSlice: 0,
						FirstArraySlice: layer.unwrap_or(0),
						ArraySize: layer.map_or(layer_count, |_| 1),
						PlaneSlice: 0,
					},
				}
			} else {
				D3D12_RENDER_TARGET_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_RTV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				}
			},
		}
	}

	/// Creates a DSV description for one selected layer or a shader-selected layer prefix.
	pub(crate) fn depth_stencil_view_desc(
		format: Formats,
		array_layers: u32,
		layer: Option<u32>,
		layer_count: u32,
	) -> D3D12_DEPTH_STENCIL_VIEW_DESC {
		Self::validate_attachment_layers(array_layers, layer, layer_count);
		D3D12_DEPTH_STENCIL_VIEW_DESC {
			Format: Self::dxgi_format(format).expect(
				"Unsupported DX12 depth-stencil format. The most likely cause is that the attachment uses a format without a native DSV mapping.",
			),
			ViewDimension: if array_layers > 1 {
				D3D12_DSV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_DSV_DIMENSION_TEXTURE2D
			},
			Flags: D3D12_DSV_FLAG_NONE,
			Anonymous: if array_layers > 1 {
				D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_DSV {
						MipSlice: 0,
						FirstArraySlice: layer.unwrap_or(0),
						ArraySize: layer.map_or(layer_count, |_| 1),
					},
				}
			} else {
				D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_DSV { MipSlice: 0 },
				}
			},
		}
	}

	/// Rejects attachment layers that cannot address the native image array.
	pub(crate) fn validate_attachment_layer(array_layers: u32, layer: Option<u32>) {
		assert!(
			array_layers > 0 && layer.is_none_or(|layer| layer < array_layers),
			"Invalid DX12 attachment layer. The most likely cause is that the render pass requested an array layer outside the image."
		);
	}

	/// Rejects layered attachment declarations that exceed the native image view.
	pub(crate) fn validate_attachment_layers(array_layers: u32, layer: Option<u32>, layer_count: u32) {
		Self::validate_attachment_layer(array_layers, layer);

		assert!(
			layer_count > 0 && layer_count <= array_layers,
			"Invalid DX12 attachment layer count. The most likely cause is that the render pass requested more layers than the image provides."
		);
		assert!(
			layer.is_none() || layer_count == 1,
			"Invalid layered DX12 attachment. The most likely cause is that the attachment selects both one layer and a layered range."
		);
	}

	/// Returns the descriptors required for every layer prefix and every selectable layer.
	pub(crate) fn attachment_descriptor_count(array_layers: u32) -> u32 {
		Self::validate_attachment_layer(array_layers, None);
		if array_layers == 1 {
			1
		} else {
			array_layers.checked_mul(2).expect(
				"Invalid DX12 attachment layer count. The most likely cause is that the image layer count cannot fit in a descriptor heap.",
			)
		}
	}

	/// Maps one attachment selection to its stable slot in the retained CPU descriptor heap.
	pub(crate) fn attachment_descriptor_slot(array_layers: u32, layer: Option<u32>, layer_count: u32) -> u32 {
		Self::validate_attachment_layers(array_layers, layer, layer_count);
		if array_layers == 1 {
			return 0;
		}
		match layer {
			Some(layer) => array_layers + layer,
			None => layer_count - 1,
		}
	}

	/// Maps a retained CPU descriptor slot back to its layer selection.
	pub(crate) fn attachment_descriptor_layers(array_layers: u32, slot: u32) -> (Option<u32>, u32) {
		assert!(
			slot < Self::attachment_descriptor_count(array_layers),
			"Invalid DX12 attachment descriptor slot. The most likely cause is that attachment materialization exceeded its retained descriptor heap."
		);
		if array_layers == 1 {
			(None, 1)
		} else if slot < array_layers {
			(None, slot + 1)
		} else {
			(Some(slot - array_layers), 1)
		}
	}

	pub(crate) fn dxgi_resource_format(format: Formats, uses: Uses) -> Option<DXGI_FORMAT> {
		if format.is_depth() && uses.intersects(Uses::Image) {
			match format {
				Formats::Depth16 => Some(DXGI_FORMAT_R16_TYPELESS),
				Formats::Depth32 => Some(DXGI_FORMAT_R32_TYPELESS),
				_ => unreachable!("Depth format check accepted a non-depth format."),
			}
		} else {
			Self::dxgi_format(format)
		}
	}

	pub(crate) fn dxgi_shader_resource_format(format: Formats) -> Option<DXGI_FORMAT> {
		if format.is_depth() {
			match format {
				Formats::Depth16 => Some(DXGI_FORMAT_R16_UNORM),
				Formats::Depth32 => Some(DXGI_FORMAT_R32_FLOAT),
				_ => unreachable!("Depth format check accepted a non-depth format."),
			}
		} else {
			Self::dxgi_format(format)
		}
	}

	pub(crate) fn dxgi_format(format: Formats) -> Option<DXGI_FORMAT> {
		match format {
			Formats::R8UNORM | Formats::R8F | Formats::R8sRGB => Some(DXGI_FORMAT_R8_UNORM),
			Formats::R8SNORM => Some(DXGI_FORMAT_R8_SNORM),
			Formats::R16F => Some(DXGI_FORMAT_R16_FLOAT),
			Formats::R16UNORM | Formats::R16sRGB => Some(DXGI_FORMAT_R16_UNORM),
			Formats::R16SNORM => Some(DXGI_FORMAT_R16_SNORM),
			Formats::R32F => Some(DXGI_FORMAT_R32_FLOAT),
			Formats::R32UNORM | Formats::R32sRGB | Formats::U32 => Some(DXGI_FORMAT_R32_UINT),
			Formats::RG8UNORM | Formats::RG8F | Formats::RG8sRGB => Some(DXGI_FORMAT_R8G8_UNORM),
			Formats::RG8SNORM => Some(DXGI_FORMAT_R8G8_SNORM),
			Formats::RG16F => Some(DXGI_FORMAT_R16G16_FLOAT),
			Formats::RG16UNORM | Formats::RG16sRGB => Some(DXGI_FORMAT_R16G16_UNORM),
			Formats::RG16SNORM => Some(DXGI_FORMAT_R16G16_SNORM),
			Formats::RGBA8UNORM | Formats::RGBA8F => Some(DXGI_FORMAT_R8G8B8A8_UNORM),
			Formats::RGBA8SNORM => Some(DXGI_FORMAT_R8G8B8A8_SNORM),
			Formats::RGBA8sRGB => Some(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB),
			Formats::RGBA16F => Some(DXGI_FORMAT_R16G16B16A16_FLOAT),
			Formats::RGBA16UNORM | Formats::RGBA16sRGB => Some(DXGI_FORMAT_R16G16B16A16_UNORM),
			Formats::RGBA16SNORM => Some(DXGI_FORMAT_R16G16B16A16_SNORM),
			Formats::BGRAu8 => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
			// DX12 swapchains expose BGRA backbuffers as UNORM, so the pipeline format must match that native RTV.
			Formats::BGRAsRGB => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
			Formats::Depth16 => Some(DXGI_FORMAT_D16_UNORM),
			Formats::Depth32 => Some(DXGI_FORMAT_D32_FLOAT),
			Formats::BC5 => Some(DXGI_FORMAT_BC5_UNORM),
			Formats::BC5SNORM => Some(DXGI_FORMAT_BC5_SNORM),
			Formats::BC7 => Some(DXGI_FORMAT_BC7_UNORM),
			Formats::BC7SRGB => Some(DXGI_FORMAT_BC7_UNORM_SRGB),
			_ => None,
		}
	}

	pub(crate) fn sync_buffer_storage(buffer: &Buffer) {
		if buffer.mapped.is_null() || buffer.size == 0 || !buffer.access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(buffer.data, buffer.mapped, buffer.size);
		}
	}

	pub(crate) fn sync_buffer_frame_storage(frame_storage: &BufferFrameStorage, size: usize, access: DeviceAccesses) {
		if frame_storage.mapped.is_null() || size == 0 || !access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(frame_storage.data, frame_storage.mapped, size);
		}
	}

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		self.sync_buffer_for_sequence(buffer_handle, 0);
	}

	pub(crate) fn sync_buffer_for_sequence(&mut self, buffer_handle: impl Into<BaseBufferHandle>, sequence_index: u8) {
		let buffer_handle = buffer_handle.into();
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		if let Some(buffer) = self.buffer(buffer_handle) {
			// Static buffers share one host-mapped resource across all frame sequences.
			// Transfer recordings may run on sequence 1, so do not gate their flushes on sequence 0.
			if sequence_index == 0 || buffer.frame_resources.is_none() {
				Self::sync_buffer_storage(buffer);
				return;
			}
			if let Some(frame_storage) = buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
			{
				Self::sync_buffer_frame_storage(frame_storage, buffer.size, buffer.access);
			}
		}
	}
}
