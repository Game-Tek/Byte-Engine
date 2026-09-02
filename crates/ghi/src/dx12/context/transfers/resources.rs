use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R11G11B10_FLOAT};

use super::*;

impl Device {
	/// Returns a non-null pointer with the alignment required for a zero-sized typed buffer.
	pub(crate) fn zero_sized_buffer_pointer(layout: Layout) -> *mut u8 {
		debug_assert_eq!(layout.size(), 0);
		// A zero-sized reference still requires a non-null, aligned pointer. The address is never dereferenced for bytes.
		std::ptr::without_provenance_mut(layout.align())
	}

	pub(crate) fn create_buffer_with_layout(
		&mut self,
		layout: Layout,
		resource_uses: Uses,
		device_accesses: DeviceAccesses,
		storage_kind: BufferStorage,
	) -> u64 {
		Self::validate_buffer_heap_contract(resource_uses, device_accesses);

		// Allocates CPU storage for a buffer with the requested layout.
		let data = if layout.size() == 0 {
			Self::zero_sized_buffer_pointer(layout)
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
			host_generation: 1,
			uploaded_generation: 0,
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

	/// Rejects buffer contracts that cannot be represented by the selected native heap.
	fn validate_buffer_heap_contract(resource_uses: Uses, device_accesses: DeviceAccesses) {
		let uses_readback_heap =
			device_accesses.intersects(DeviceAccesses::CpuRead) && !device_accesses.intersects(DeviceAccesses::CpuWrite);
		if !uses_readback_heap {
			return;
		}

		assert!(
			!device_accesses.intersects(DeviceAccesses::GpuRead),
			"Invalid DX12 readback-buffer access. The most likely cause is that CPU-readable memory was also declared for GPU reads. Readback heaps support only copy or resolve destination access. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/readback-data-using-heaps."
		);
		assert!(
			resource_uses.difference(Uses::TransferDestination).is_empty(),
			"Invalid DX12 readback-buffer usage. The most likely cause is that a CPU-readable buffer was declared for a GPU binding instead of transfer-destination readback. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#readback-heap-resources."
		);
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
		// Every native consumer crosses this seam, so dirty host writes become visible without per-command special cases.
		self.sync_buffer_for_sequence(buffer_handle, sequence_index);
		let buffer = self.buffer(buffer_handle)?;
		let resource = if sequence_index == 0 {
			buffer.resource.clone()
		} else {
			buffer
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.and_then(|resource| resource.resource.clone())
				.or_else(|| buffer.resource.clone())
		};
		if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		resource
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
			Self::mark_buffer_host_write(buffer);
			return Some((buffer.data, buffer.size));
		}
		let size = buffer.size;
		let storage = buffer
			.frame_resources
			.as_mut()
			.and_then(|resources| resources.get_mut(sequence_index as usize))
			.and_then(|resource| resource.as_mut());
		if let Some(storage) = storage {
			Self::mark_buffer_frame_host_write(storage);
			Some((storage.data, size))
		} else {
			Self::mark_buffer_host_write(buffer);
			Some((buffer.data, size))
		}
	}

	pub(crate) fn create_buffer_frame_storage(&self, layout: Layout, access: DeviceAccesses, uses: Uses) -> BufferFrameStorage {
		let data = if layout.size() == 0 {
			Self::zero_sized_buffer_pointer(layout)
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
			host_generation: 1,
			uploaded_generation: 0,
			resource,
			mapped,
			heap_kind,
		}
	}

	/// Rounds native allocations to the full range exposed by DX12 buffer descriptors.
	pub(crate) fn buffer_resource_size(size: usize, uses: Uses) -> usize {
		if uses.intersects(Uses::Uniform) {
			Self::align_up(size.max(1), 256)
		} else if uses.intersects(Uses::Storage) {
			// HLSL packs flat u8 and u16 storage arrays into 32-bit words, so the final partial word needs backing memory.
			Self::align_up(size.max(1), std::mem::size_of::<u32>())
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
		let resource_desc = D3D12_RESOURCE_DESC1 {
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
			SamplerFeedbackMipRegion: Default::default(),
		};

		let mut resource: Option<ID3D12Resource> = None;
		let result = unsafe {
			// Enhanced-barrier buffer creation requires UNDEFINED regardless of the heap type.
			self.device.CreateCommittedResource3(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_BARRIER_LAYOUT_UNDEFINED,
				None,
				None::<&ID3D12ProtectedResourceSession>,
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

	/// Rejects image metadata that cannot describe one native 2D or 3D texture.
	pub(crate) fn validate_image_dimension(extent: Extent, is_3d: bool, array_layers: u32, cube_compatible: bool) {
		assert!(
			array_layers > 0,
			"Invalid DX12 image array size. The most likely cause is that the image builder supplied an empty array."
		);
		if is_3d {
			assert!(
				array_layers == 1 && !cube_compatible,
				"Invalid DX12 3D texture array. The most likely cause is that array or cubemap metadata was combined with a Texture3D extent. DX12 interprets DepthOrArraySize as depth for Texture3D resources. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_resource_desc."
			);
		} else {
			assert!(
				extent.depth() <= 1,
				"Invalid DX12 2D texture depth. The most likely cause is that a 2D image was resized with a 3D extent."
			);
		}
	}

	pub(crate) fn create_image_resource(
		&self,
		extent: Extent,
		is_3d: bool,
		format: Formats,
		uses: Uses,
		array_layers: u32,
		mip_levels: u32,
		optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
	) -> Option<ID3D12Resource> {
		Self::validate_image_dimension(extent, is_3d, array_layers, false);
		let dxgi_format = Self::dxgi_resource_format(format, uses)?;
		if extent.width() == 0 || extent.height() == 0 || (is_3d && extent.depth() == 0) {
			return None;
		}

		let flags = Self::image_resource_flags(format, uses);
		let depth_or_array_size = if is_3d {
			u16::try_from(extent.depth().max(1)).expect(
				"Invalid DX12 image depth. The most likely cause is that the 3D texture depth exceeds the native 16-bit limit.",
			)
		} else {
			u16::try_from(array_layers.max(1)).expect(
				"Invalid DX12 image array size. The most likely cause is that the layer count exceeds the native 16-bit limit.",
			)
		};
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC1 {
			Dimension: if is_3d {
				D3D12_RESOURCE_DIMENSION_TEXTURE3D
			} else {
				D3D12_RESOURCE_DIMENSION_TEXTURE2D
			},
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
			SamplerFeedbackMipRegion: Default::default(),
		};
		let mut resource = None;
		let result = unsafe {
			self.device.CreateCommittedResource3(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_BARRIER_LAYOUT_COMMON,
				optimized_clear_value.as_ref().map(|clear_value| clear_value as *const _),
				None::<&ID3D12ProtectedResourceSession>,
				None,
				&mut resource,
			)
		};
		if let Err(error) = result {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 image resource. Format: {:?}. Extent: {:?}. Dimension: {}D. Uses: {:?}. Array layers: {}. Error: {error:?}. Device removed reason: {removed_reason:?}",
				format,
				extent,
				if is_3d { 3 } else { 2 },
				uses,
				array_layers
			));
			panic!(
				"Failed to create a DX12 image resource. The most likely cause is that the requested format/use combination is invalid, memory is exhausted, or the device was removed. Native error: {error:?}. Device removed reason: {removed_reason:?}."
			);
		} else {
			Some(resource.expect(
				"Failed to create a DX12 image resource. The most likely cause is that the driver reported success without returning the requested native resource.",
			))
		}
	}

	/// Verifies every native view capability promised by an image before its logical handle is published.
	pub(crate) fn validate_image_format_support(&self, format: Formats, uses: Uses, is_3d: bool) {
		assert!(
			!uses.intersects(Uses::DepthStencil) || format.is_depth(),
			"Invalid DX12 depth-stencil image. The most likely cause is that depth-stencil usage was requested for a color format."
		);
		assert!(
			!format.is_depth() || !uses.intersects(Uses::RenderTarget | Uses::Storage),
			"Invalid DX12 depth image usage. The most likely cause is that a depth format was requested as a color render target or unordered-access image. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_resource_flags."
		);
		assert!(
			!is_3d || (!format.is_depth() && !uses.intersects(Uses::RenderTarget | Uses::DepthStencil)),
			"Invalid DX12 3D attachment image. The most likely cause is that the current attachment path requires a 2D render-target or depth-stencil view."
		);
		let resource_format = Self::dxgi_resource_format(format, uses).unwrap_or_else(|| {
			panic!(
				"Unsupported DX12 image format. The most likely cause is that {format:?} has no exact DXGI representation. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
			)
		});
		let attachment_format = Self::dxgi_format(format).unwrap_or_else(|| {
			panic!(
				"Unsupported DX12 image view format. The most likely cause is that {format:?} has no exact typed DXGI view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
			)
		});

		let (dimension_support, dimension_name) = if is_3d {
			(d3d12::D3D12_FORMAT_SUPPORT1_TEXTURE3D, "3D texture")
		} else {
			(d3d12::D3D12_FORMAT_SUPPORT1_TEXTURE2D, "2D texture")
		};
		Self::assert_format_support1(
			self.query_format_support(resource_format),
			dimension_support,
			format,
			dimension_name,
		);
		if uses.intersects(Uses::Image) {
			let shader_format = Self::dxgi_shader_resource_format(format).unwrap_or_else(|| {
				panic!(
					"Unsupported DX12 shader-resource format. The most likely cause is that {format:?} has no exact typed DXGI shader view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
				)
			});
			Self::assert_format_support1(
				self.query_format_support(shader_format),
				d3d12::D3D12_FORMAT_SUPPORT1_SHADER_LOAD | d3d12::D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE,
				format,
				"shader-resource load and sampling",
			);
		}
		if uses.intersects(Uses::InputAttachment) {
			let shader_format = Self::dxgi_shader_resource_format(format).unwrap_or_else(|| {
				panic!(
					"Unsupported DX12 input-attachment format. The most likely cause is that {format:?} has no exact typed DXGI shader view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
				)
			});
			Self::assert_format_support1(
				self.query_format_support(shader_format),
				d3d12::D3D12_FORMAT_SUPPORT1_SHADER_LOAD,
				format,
				"input-attachment shader load",
			);
		}
		if format.is_depth() {
			Self::assert_format_support1(
				self.query_format_support(attachment_format),
				d3d12::D3D12_FORMAT_SUPPORT1_DEPTH_STENCIL,
				format,
				"depth-stencil view",
			);
		} else if uses.intersects(Uses::RenderTarget) {
			Self::assert_format_support1(
				self.query_format_support(attachment_format),
				d3d12::D3D12_FORMAT_SUPPORT1_RENDER_TARGET,
				format,
				"render-target view",
			);
		}
		if uses.intersects(Uses::Storage) {
			let storage_format = Self::dxgi_shader_resource_format(format).unwrap_or_else(|| {
				panic!(
					"Unsupported DX12 storage-image format. The most likely cause is that {format:?} has no exact typed DXGI shader view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
				)
			});
			let storage_support = self.query_format_support(storage_format);
			Self::assert_format_support1(
				storage_support,
				d3d12::D3D12_FORMAT_SUPPORT1_TYPED_UNORDERED_ACCESS_VIEW,
				format,
				"typed unordered-access view",
			);
			Self::assert_format_support2(
				storage_support,
				d3d12::D3D12_FORMAT_SUPPORT2_UAV_TYPED_STORE,
				format,
				"typed unordered-access store",
			);
		}
	}

	/// Verifies optional typed-UAV reads required by one shader resource declaration.
	pub(crate) fn validate_typed_uav_format_support(&self, format: Formats, access: crate::AccessPolicies) {
		if !access.intersects(crate::AccessPolicies::READ) {
			return;
		}
		let native_format = Self::dxgi_shader_resource_format(format).unwrap_or_else(|| {
			panic!(
				"Unsupported DX12 storage-image format. The most likely cause is that {format:?} has no exact typed DXGI view. See https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format."
			)
		});
		Self::assert_format_support2(
			self.query_format_support(native_format),
			d3d12::D3D12_FORMAT_SUPPORT2_UAV_TYPED_LOAD,
			format,
			"typed unordered-access load",
		);
	}

	/// Returns device support for one exact DXGI format and caches the immutable driver result.
	fn query_format_support(&self, format: DXGI_FORMAT) -> d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT {
		if let Some(support) = self.format_support_cache.borrow().get(&format.0).copied() {
			return support;
		}

		let mut support = d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT {
			Format: format,
			..Default::default()
		};
		let result = unsafe {
			self.device.CheckFeatureSupport(
				d3d12::D3D12_FEATURE_FORMAT_SUPPORT,
				(&mut support as *mut d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT).cast(),
				std::mem::size_of::<d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT>() as u32,
			)
		};
		assert!(
			result.is_ok(),
			"Failed to query DX12 format support. The most likely cause is that the device was removed or the driver rejected the capability query. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_feature_data_format_support."
		);
		self.format_support_cache.borrow_mut().insert(format.0, support);
		support
	}

	/// Rejects a format when its driver support does not cover every required resource operation.
	fn assert_format_support1(
		support: d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT,
		required: d3d12::D3D12_FORMAT_SUPPORT1,
		format: Formats,
		operation: &str,
	) {
		assert!(
			support.Support1.contains(required),
			"Unsupported DX12 image operation. The most likely cause is that {format:?} does not support {operation} on this device. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_format_support1."
		);
	}

	/// Rejects a format when its driver support does not cover the required typed-UAV operation.
	fn assert_format_support2(
		support: d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT,
		required: d3d12::D3D12_FORMAT_SUPPORT2,
		format: Formats,
		operation: &str,
	) {
		assert!(
			support.Support2.contains(required),
			"Unsupported DX12 storage-image operation. The most likely cause is that {format:?} does not support {operation} on this device. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_format_support2."
		);
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
		if format.is_depth() && uses.intersects(Uses::Image | Uses::InputAttachment) {
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
			Formats::R8UNORM => Some(DXGI_FORMAT_R8_UNORM),
			Formats::R8SNORM => Some(DXGI_FORMAT_R8_SNORM),
			Formats::R16F => Some(DXGI_FORMAT_R16_FLOAT),
			Formats::R16UNORM => Some(DXGI_FORMAT_R16_UNORM),
			Formats::R16SNORM => Some(DXGI_FORMAT_R16_SNORM),
			Formats::R32F => Some(DXGI_FORMAT_R32_FLOAT),
			Formats::U32 => Some(DXGI_FORMAT_R32_UINT),
			Formats::RG8UNORM => Some(DXGI_FORMAT_R8G8_UNORM),
			Formats::RG8SNORM => Some(DXGI_FORMAT_R8G8_SNORM),
			Formats::RG16F => Some(DXGI_FORMAT_R16G16_FLOAT),
			Formats::RG16UNORM => Some(DXGI_FORMAT_R16G16_UNORM),
			Formats::RG16SNORM => Some(DXGI_FORMAT_R16G16_SNORM),
			Formats::RGBA8UNORM => Some(DXGI_FORMAT_R8G8B8A8_UNORM),
			Formats::RGBA8SNORM => Some(DXGI_FORMAT_R8G8B8A8_SNORM),
			Formats::RGBA8sRGB => Some(DXGI_FORMAT_R8G8B8A8_UNORM_SRGB),
			Formats::RGBA16F => Some(DXGI_FORMAT_R16G16B16A16_FLOAT),
			Formats::RGBA16UNORM => Some(DXGI_FORMAT_R16G16B16A16_UNORM),
			Formats::RGBA16SNORM => Some(DXGI_FORMAT_R16G16B16A16_SNORM),
			Formats::RGBu11u11u10 => Some(DXGI_FORMAT_R11G11B10_FLOAT),
			Formats::BGRAu8 => Some(DXGI_FORMAT_B8G8R8A8_UNORM),
			Formats::BGRAsRGB => Some(DXGI_FORMAT_B8G8R8A8_UNORM_SRGB),
			Formats::Depth16 => Some(DXGI_FORMAT_D16_UNORM),
			Formats::Depth32 => Some(DXGI_FORMAT_D32_FLOAT),
			Formats::BC5 => Some(DXGI_FORMAT_BC5_UNORM),
			Formats::BC5SNORM => Some(DXGI_FORMAT_BC5_SNORM),
			Formats::BC7 => Some(DXGI_FORMAT_BC7_UNORM),
			Formats::BC7SRGB => Some(DXGI_FORMAT_BC7_UNORM_SRGB),
			_ => None,
		}
	}

	/// Marks the base CPU shadow dirty before exposing mutable storage.
	pub(crate) fn mark_buffer_host_write(buffer: &mut Buffer) {
		buffer.host_generation = buffer.host_generation.wrapping_add(1);
		if buffer.host_generation == buffer.uploaded_generation {
			buffer.host_generation = buffer.host_generation.wrapping_add(1);
		}
	}

	/// Marks one frame-local CPU shadow dirty before exposing mutable storage.
	fn mark_buffer_frame_host_write(frame_storage: &mut BufferFrameStorage) {
		frame_storage.host_generation = frame_storage.host_generation.wrapping_add(1);
		if frame_storage.host_generation == frame_storage.uploaded_generation {
			frame_storage.host_generation = frame_storage.host_generation.wrapping_add(1);
		}
	}

	/// Copies a changed base CPU shadow into its mapped native allocation.
	pub(crate) fn sync_buffer_storage(buffer: &mut Buffer) {
		if buffer.mapped.is_null() || buffer.size == 0 || !buffer.access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}
		if buffer.host_generation == buffer.uploaded_generation {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(buffer.data, buffer.mapped, buffer.size);
		}
		buffer.uploaded_generation = buffer.host_generation;
	}

	/// Copies a changed frame-local CPU shadow into its mapped native allocation.
	pub(crate) fn sync_buffer_frame_storage(frame_storage: &mut BufferFrameStorage, size: usize, access: DeviceAccesses) {
		if frame_storage.mapped.is_null() || size == 0 || !access.intersects(DeviceAccesses::CpuWrite) {
			return;
		}
		if frame_storage.host_generation == frame_storage.uploaded_generation {
			return;
		}

		unsafe {
			std::ptr::copy_nonoverlapping(frame_storage.data, frame_storage.mapped, size);
		}
		frame_storage.uploaded_generation = frame_storage.host_generation;
	}

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		self.sync_buffer_for_sequence(buffer_handle, 0);
	}

	pub(crate) fn sync_buffer_for_sequence(&mut self, buffer_handle: impl Into<BaseBufferHandle>, sequence_index: u8) {
		let buffer_handle = buffer_handle.into();
		self.ensure_buffer_frame_storage(buffer_handle, sequence_index);
		if let Some(buffer) = self.buffer_mut(buffer_handle) {
			// Static buffers share one host-mapped resource across all frame sequences.
			// Transfer recordings may run on sequence 1, so do not gate their flushes on sequence 0.
			if sequence_index == 0 || buffer.frame_resources.is_none() {
				Self::sync_buffer_storage(buffer);
				return;
			}
			if let Some(frame_storage) = buffer
				.frame_resources
				.as_mut()
				.and_then(|resources| resources.get_mut(sequence_index as usize))
				.and_then(|resource| resource.as_mut())
			{
				Self::sync_buffer_frame_storage(frame_storage, buffer.size, buffer.access);
			}
		}
	}
}
