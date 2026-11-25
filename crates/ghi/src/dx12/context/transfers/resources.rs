use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R11G11B10_FLOAT};

use super::*;

impl Device {
	/// Creates every copy of a buffer up front and returns the buffer's index, so a buffer built mid-frame never shares
	/// a copy with another frame in flight.
	pub(crate) fn create_buffer(
		&mut self,
		layout: Layout,
		uses: Uses,
		host_access: crate::HostAccess,
		copies: crate::Copies,
		name: Option<&str>,
	) -> u64 {
		let name = self.debug_name(name);
		let mut storages = (0..copies.count(self.frames))
			.map(|_| self.create_buffer_memory(layout.size(), host_access, uses, name.as_deref()))
			.collect::<Vec<_>>();
		let memory = storages.remove(0);
		self.buffers.push(Buffer {
			memory,
			frame_copies: storages,
			size: layout.size(),
			align: layout.align(),
			uses,
			host_access,
			copies,
			name,
		});
		(self.buffers.len() - 1) as u64
	}

	pub(crate) fn buffer(&self, buffer_handle: BaseBufferHandle) -> &Buffer {
		&self.buffers[buffer_handle.index() as usize]
	}

	/// Returns the native resource of the copy `sequence_index` uses, and keeps it alive with the recording command
	/// buffer, which every native consumer goes through.
	pub(crate) fn buffer_resource_for_sequence(
		&mut self,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let resource = self.buffer(buffer_handle).memory(sequence_index).resource.clone();
		if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		resource
	}

	/// Creates the native resource of one copy of a `size`-byte buffer, and maps it for its lifetime when the CPU
	/// accesses it.
	///
	/// # Panics
	///
	/// Panics when the device can't allocate the memory.
	pub(crate) fn create_buffer_memory(
		&self,
		size: usize,
		host_access: crate::HostAccess,
		uses: Uses,
		name: Option<&str>,
	) -> BufferMemory {
		let resource_size = Self::buffer_resource_size(size, uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, host_access);
		if let Some(resource) = resource.as_ref() {
			self.name_object(resource, name);
		}
		assert!(
			size == 0 || resource.is_some(),
			"Failed to allocate a {size}-byte DX12 buffer. The most likely cause is that the device is out of memory. Free resources or lower the memory settings that size this buffer."
		);
		BufferMemory {
			resource,
			mapped,
			heap_kind,
		}
	}

	/// Rounds native allocations to the full range exposed by DX12 buffer descriptors.
	pub(crate) fn buffer_resource_size(size: usize, uses: Uses) -> usize {
		if uses.intersects(Uses::Uniform) {
			size.max(1).next_multiple_of(256)
		} else if uses.intersects(Uses::Storage) {
			// HLSL packs flat u8 and u16 storage arrays into 32-bit words, so the final partial word needs backing memory.
			size.max(1).next_multiple_of(std::mem::size_of::<u32>())
		} else {
			size
		}
	}

	/// Returns the heap a buffer the CPU accesses as `host_access` lives in.
	pub(crate) fn buffer_heap_kind(host_access: crate::HostAccess) -> BufferHeapKind {
		match host_access {
			crate::HostAccess::None => BufferHeapKind::Default,
			crate::HostAccess::Write => BufferHeapKind::Upload,
			crate::HostAccess::Read => BufferHeapKind::Readback,
		}
	}

	/// Creates a committed buffer resource in the heap `host_access` selects, and maps it when the CPU accesses it.
	pub(crate) fn create_buffer_resource(
		&self,
		size: usize,
		host_access: crate::HostAccess,
	) -> (Option<ID3D12Resource>, *mut u8, BufferHeapKind) {
		let heap_kind = Self::buffer_heap_kind(host_access);
		if size == 0 {
			return (None, std::ptr::null_mut(), heap_kind);
		}

		let cpu_visible = heap_kind != BufferHeapKind::Default;
		// Every heap here takes shader writes, so any buffer can be cleared and written as an unordered-access view.
		let resource_flags = D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS;
		let heap_properties = match heap_kind {
			// CPU-cached system memory, the only memory both the CPU reads and shaders may write on a discrete GPU.
			BufferHeapKind::Readback => D3D12_HEAP_PROPERTIES {
				Type: d3d12::D3D12_HEAP_TYPE_CUSTOM,
				CPUPageProperty: d3d12::D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
				MemoryPoolPreference: d3d12::D3D12_MEMORY_POOL_L0,
				CreationNodeMask: 1,
				VisibleNodeMask: 1,
			},
			// Write-combined system memory, as an upload heap is on a discrete GPU, which shaders may also write.
			BufferHeapKind::Upload => D3D12_HEAP_PROPERTIES {
				Type: d3d12::D3D12_HEAP_TYPE_CUSTOM,
				CPUPageProperty: d3d12::D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE,
				MemoryPoolPreference: d3d12::D3D12_MEMORY_POOL_L0,
				CreationNodeMask: 1,
				VisibleNodeMask: 1,
			},
			BufferHeapKind::Default => D3D12_HEAP_PROPERTIES {
				Type: D3D12_HEAP_TYPE_DEFAULT,
				CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
				MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
				CreationNodeMask: 1,
				VisibleNodeMask: 1,
			},
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
		// A handle without memory would make later GPU work silently read and write nothing, so failure stops here.
		if let Err(error) = result {
			let report = super::device_removed_report(&self.device);
			panic!(
				"Failed to create a {size}-byte DX12 buffer in the {heap_kind:?} heap: {error:?}. The most likely cause is that the GPU ran out of memory or the device was removed. Free unused buffers or lower resource sizes.
{report}"
			);
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
				if let Err(error) = result {
					let report = super::device_removed_report(&self.device);
					panic!(
						"Failed to map a {size}-byte DX12 buffer in the {heap_kind:?} heap: {error:?}. The most likely cause is that the device was removed. Check the report below for the command that faulted.
{report}"
					);
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
		name: Option<&str>,
	) -> Option<ID3D12Resource> {
		let resource_desc = Self::image_resource_desc(extent, is_3d, format, uses, array_layers, mip_levels)?;
		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
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
		let resource = self.expect_image_resource(result, resource, format, extent, is_3d, uses, array_layers);
		self.name_object(&resource, name);
		Some(resource)
	}

	/// Creates an image-group member at `offset` inside `heap`.
	///
	/// The member starts in the undefined layout, so its first barrier must discard its contents.
	pub(crate) fn create_placed_image_resource(
		&self,
		heap: &ID3D12Heap,
		offset: u64,
		resource_desc: &D3D12_RESOURCE_DESC1,
		(format, extent, is_3d, uses, array_layers): (Formats, Extent, bool, Uses, u32),
		optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
		name: Option<&str>,
	) -> ID3D12Resource {
		let mut resource = None;
		let result = unsafe {
			self.device.CreatePlacedResource2(
				heap,
				offset,
				resource_desc,
				D3D12_BARRIER_LAYOUT_UNDEFINED,
				optimized_clear_value.as_ref().map(|clear_value| clear_value as *const _),
				None,
				&mut resource,
			)
		};
		let resource = self.expect_image_resource(result, resource, format, extent, is_3d, uses, array_layers);
		self.name_object(&resource, name);
		resource
	}

	/// Returns the name a resource built with `name` keeps for its native copies, which is none unless debug labels are
	/// on.
	pub(crate) fn debug_name(&self, name: Option<&str>) -> Option<Box<str>> {
		name.filter(|_| self.settings.debug_labels).map(Box::from)
	}

	/// Gives `object` the debug name `name`, which the debug layer and GPU captures show, when debug labels are on.
	pub(crate) fn name_object<'a>(&self, object: impl Into<&'a d3d12::ID3D12Object>, name: Option<&str>) {
		if let Some(name) = name.filter(|_| self.settings.debug_labels) {
			// SAFETY: The name is a live wide string for the call, which copies it. A name is only a debugging aid, so
			// failing to set one is ignored.
			let _ = unsafe { object.into().SetName(&windows::core::HSTRING::from(name)) };
		}
	}

	/// Unwraps a native image creation result, logging the device state before a failure panics.
	fn expect_image_resource(
		&self,
		result: windows::core::Result<()>,
		resource: Option<ID3D12Resource>,
		format: Formats,
		extent: Extent,
		is_3d: bool,
		uses: Uses,
		array_layers: u32,
	) -> ID3D12Resource {
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
		}
		resource.expect(
			"Failed to create a DX12 image resource. The most likely cause is that the driver reported success without returning the requested native resource.",
		)
	}

	/// Describes a native texture for an image, or returns `None` when the image has no extent or native format.
	pub(crate) fn image_resource_desc(
		extent: Extent,
		is_3d: bool,
		format: Formats,
		uses: Uses,
		array_layers: u32,
		mip_levels: u32,
	) -> Option<D3D12_RESOURCE_DESC1> {
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
		Some(resource_desc)
	}

	/// Verifies every native view capability promised by an image before its logical handle is published.
	pub(crate) fn validate_image_format_support(&self, format: Formats, uses: Uses, is_3d: bool) {
		assert!(
			!uses.intersects(Uses::DepthStencil) || format.is_depth(),
			"Invalid DX12 depth-stencil image. The most likely cause is that depth-stencil usage was requested for a color format."
		);
		// A depth format with render-target use attaches as a depth target, so only storage use is invalid.
		assert!(
			!format.is_depth() || !uses.intersects(Uses::Storage),
			"Invalid DX12 depth image usage. The most likely cause is that a depth format was requested as a storage image, which D3D12 depth resources can't be. Read depth through a sampled view, or copy it into a color image. See https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_resource_flags."
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
			// Integer formats never filter, so shaders read them with loads and D3D12 reports no sampling support.
			if format == Formats::U32 {
				Self::assert_format_support1(
					self.query_format_support(shader_format),
					d3d12::D3D12_FORMAT_SUPPORT1_SHADER_LOAD,
					format,
					"shader-resource load",
				);
			} else {
				Self::assert_format_support1(
					self.query_format_support(shader_format),
					d3d12::D3D12_FORMAT_SUPPORT1_SHADER_LOAD | d3d12::D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE,
					format,
					"shader-resource load and sampling",
				);
			}
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

	/// Queries device support for every DXGI format an image of any [`Formats`] variant creates or views, for the table
	/// [`Self::query_format_support`] reads. The result is sorted by format and holds each format once.
	pub(crate) fn query_format_supports(device: &ID3D12Device10) -> Box<[d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT]> {
		// Depth formats map to a typeless resource, a typed shader view and a depth view; the rest map to one format.
		let mut formats = Formats::ALL
			.iter()
			.flat_map(|&format| {
				[
					Self::dxgi_format(format),
					Self::dxgi_shader_resource_format(format),
					Self::dxgi_resource_format(format, Uses::Image),
				]
			})
			.flatten()
			.collect::<Vec<_>>();
		formats.sort_unstable_by_key(|format| format.0);
		formats.dedup();
		formats
			.into_iter()
			.map(|format| {
				let unsupported = d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT {
					Format: format,
					..Default::default()
				};
				// A format the driver rejects the query for supports nothing, so creating an image with it fails
				// validation with the operation it lacks instead of failing device creation for every application.
				Self::feature_support(device, d3d12::D3D12_FEATURE_FORMAT_SUPPORT, unsupported).unwrap_or(unsupported)
			})
			.collect()
	}

	/// Returns device support for one exact DXGI format from the table the device queried at creation.
	fn query_format_support(&self, format: DXGI_FORMAT) -> d3d12::D3D12_FEATURE_DATA_FORMAT_SUPPORT {
		let index = self
			.format_support
			.binary_search_by_key(&format.0, |support| support.Format.0)
			.unwrap_or_else(|_| {
				panic!(
					"DX12 format support is unknown for {format:?}. The most likely cause is that a Formats variant maps to this DXGI format but is missing from Formats::ALL, so device creation never queried it. Add the variant to Formats::ALL."
				)
			});
		self.format_support[index]
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
		// Integer render targets clear through a UAV too: a float clear color can't hold every 32-bit pattern exactly.
		if uses.intersects(Uses::Storage) || (uses.intersects(Uses::RenderTarget) && format == Formats::U32) {
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

	/// Creates an RTV description for `layer_count` layers from `first_layer`: one selected layer, or a shader-selected
	/// layer range.
	pub(crate) fn render_target_view_desc(
		format: Formats,
		array_layers: u32,
		first_layer: u32,
		layer_count: u32,
	) -> D3D12_RENDER_TARGET_VIEW_DESC {
		Self::validate_attachment_layers(array_layers, first_layer, layer_count);
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
						FirstArraySlice: first_layer,
						ArraySize: layer_count,
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

	/// Creates a DSV description for `layer_count` layers from `first_layer`: one selected layer, or a shader-selected
	/// layer range.
	pub(crate) fn depth_stencil_view_desc(
		format: Formats,
		array_layers: u32,
		first_layer: u32,
		layer_count: u32,
	) -> D3D12_DEPTH_STENCIL_VIEW_DESC {
		Self::validate_attachment_layers(array_layers, first_layer, layer_count);
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
						FirstArraySlice: first_layer,
						ArraySize: layer_count,
					},
				}
			} else {
				D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_DSV { MipSlice: 0 },
				}
			},
		}
	}

	/// Rejects attachment layer ranges that the native image array does not hold.
	pub(crate) fn validate_attachment_layers(array_layers: u32, first_layer: u32, layer_count: u32) {
		assert!(
			layer_count > 0 && first_layer.checked_add(layer_count).is_some_and(|end| end <= array_layers),
			"Invalid DX12 attachment layer range. The most likely cause is that the render pass requested layers outside the image."
		);
	}

	/// Returns the descriptors required for every layer range of an image: one per first layer and layer count.
	pub(crate) fn attachment_descriptor_count(array_layers: u32) -> u32 {
		assert!(
			array_layers > 0,
			"Invalid DX12 attachment layer count. The most likely cause is an image without array layers."
		);
		array_layers.checked_mul(array_layers + 1).map(|count| count / 2).expect(
			"Invalid DX12 attachment layer count. The most likely cause is that the image layer count cannot fit in a descriptor heap.",
		)
	}

	/// Returns the first slot of the ranges that start at `first_layer`: the ranges of every earlier first layer come
	/// before them, `array_layers - f` for first layer `f`.
	fn attachment_descriptor_range_base(array_layers: u32, first_layer: u32) -> u32 {
		first_layer * (2 * array_layers - first_layer + 1) / 2
	}

	/// Maps one attachment's layer range to its stable slot in the retained CPU descriptor heap.
	pub(crate) fn attachment_descriptor_slot(array_layers: u32, first_layer: u32, layer_count: u32) -> u32 {
		Self::validate_attachment_layers(array_layers, first_layer, layer_count);
		Self::attachment_descriptor_range_base(array_layers, first_layer) + layer_count - 1
	}

	/// Maps a retained CPU descriptor slot back to its first layer and layer count.
	pub(crate) fn attachment_descriptor_layers(array_layers: u32, slot: u32) -> (u32, u32) {
		assert!(
			slot < Self::attachment_descriptor_count(array_layers),
			"Invalid DX12 attachment descriptor slot. The most likely cause is that attachment materialization exceeded its retained descriptor heap."
		);
		let first_layer = (0..array_layers)
			.rfind(|first_layer| Self::attachment_descriptor_range_base(array_layers, *first_layer) <= slot)
			.unwrap_or(0);
		(
			first_layer,
			slot - Self::attachment_descriptor_range_base(array_layers, first_layer) + 1,
		)
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
}
