use super::super::*;

impl Device {
	/// Initializes a pipeline-defined descriptor table so sparse arrays have valid native entries.
	pub(crate) fn initialize_descriptor_heap_defaults(
		&self,
		layout: &PipelineLayout,
		sampler_heap: bool,
		heap: &DescriptorHeap,
		base_offset: u32,
	) {
		let heap_type = if sampler_heap {
			D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
		} else {
			D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
		};
		for resource in &layout.resources {
			let offset = if sampler_heap {
				resource.sampler_offset
			} else {
				resource.cbv_srv_uav_offset
			};
			let Some(offset) = offset else {
				continue;
			};
			for array_element in 0..resource.descriptor.count() {
				let cpu_handle = self.descriptor_cpu_handle(heap, heap_type, base_offset + offset + array_element);
				if sampler_heap {
					self.write_default_sampler_descriptor(cpu_handle);
				} else {
					self.write_null_cbv_srv_uav_descriptor(resource.descriptor, cpu_handle);
				}
			}
		}
	}

	/// Writes a null CBV, SRV, or UAV that matches one pipeline resource representation.
	pub(crate) fn write_null_cbv_srv_uav_descriptor(
		&self,
		descriptor: ShaderResourceDescriptor,
		cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
	) {
		match descriptor.kind() {
			ResourceKind::UniformBuffer => unsafe {
				self.device.CreateConstantBufferView(None, cpu_handle);
			},
			ResourceKind::StorageBuffer => unsafe {
				if descriptor.access().intersects(crate::AccessPolicies::WRITE) {
					self.device.CreateUnorderedAccessView(
						None::<&ID3D12Resource>,
						None::<&ID3D12Resource>,
						Some(&Self::null_buffer_uav_desc(descriptor.buffer_element_stride())),
						cpu_handle,
					);
				} else {
					self.device.CreateShaderResourceView(
						None::<&ID3D12Resource>,
						Some(&Self::null_buffer_srv_desc(descriptor.buffer_element_stride())),
						cpu_handle,
					);
				}
			},
			ResourceKind::StorageImage => unsafe {
				self.device.CreateUnorderedAccessView(
					None::<&ID3D12Resource>,
					None::<&ID3D12Resource>,
					Some(&Self::null_texture_uav_desc(descriptor.texture_view())),
					cpu_handle,
				);
			},
			ResourceKind::AccelerationStructure => unsafe {
				self.device.CreateShaderResourceView(
					None::<&ID3D12Resource>,
					Some(&Self::null_acceleration_structure_srv_desc()),
					cpu_handle,
				);
			},
			ResourceKind::SampledImage | ResourceKind::CombinedImageSampler | ResourceKind::InputAttachment => unsafe {
				self.device.CreateShaderResourceView(
					None::<&ID3D12Resource>,
					Some(&Self::null_texture_srv_desc(descriptor.texture_view())),
					cpu_handle,
				);
			},
			ResourceKind::Sampler => {}
		}
	}

	/// Writes the default sampler used by unbound sampler slots.
	pub(crate) fn write_default_sampler_descriptor(&self, cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE) {
		let desc = D3D12_SAMPLER_DESC {
			Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
			AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
			MipLODBias: 0.0,
			MaxAnisotropy: 1,
			ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
			BorderColor: [0.0, 0.0, 0.0, 0.0],
			MinLOD: 0.0,
			MaxLOD: 0.0,
		};
		unsafe {
			self.device.CreateSampler(&desc, cpu_handle);
		}
	}

	pub(crate) fn null_buffer_uav_desc(stride: u32) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_UAV {
					FirstElement: 0,
					NumElements: 1,
					StructureByteStride: stride.max(1),
					CounterOffsetInBytes: 0,
					Flags: D3D12_BUFFER_UAV_FLAG_NONE,
				},
			},
		}
	}

	pub(crate) fn raw_buffer_clear_uav_desc(size: usize) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: DXGI_FORMAT_R32_TYPELESS,
			ViewDimension: D3D12_UAV_DIMENSION_BUFFER,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_UAV {
					FirstElement: 0,
					NumElements: (size / std::mem::size_of::<u32>()).max(1) as u32,
					StructureByteStride: 0,
					CounterOffsetInBytes: 0,
					Flags: D3D12_BUFFER_UAV_FLAG_RAW,
				},
			},
		}
	}

	pub(crate) fn null_buffer_srv_desc(stride: u32) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_BUFFER,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				Buffer: D3D12_BUFFER_SRV {
					FirstElement: 0,
					NumElements: 1,
					StructureByteStride: stride.max(1),
					Flags: D3D12_BUFFER_SRV_FLAG_NONE,
				},
			},
		}
	}

	pub(crate) fn null_texture_uav_desc(texture_view_type: TextureViewTypes) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		match texture_view_type {
			TextureViewTypes::TextureCube | TextureViewTypes::TextureCubeArray => {
				panic!("Unsupported DX12 cubemap UAV. The most likely cause is that a read-only cubemap was declared writable.")
			}
			TextureViewTypes::Texture2DArray => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2DARRAY,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
						MipSlice: 0,
						FirstArraySlice: 0,
						ArraySize: 1,
						PlaneSlice: 0,
					},
				},
			},
			TextureViewTypes::Texture3D => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_UAV {
						MipSlice: 0,
						FirstWSlice: 0,
						WSize: 1,
					},
				},
			},
			TextureViewTypes::Texture2D => D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: DXGI_FORMAT_R32_UINT,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				},
			},
		}
	}

	pub(crate) fn texture_uav_desc(
		format: DXGI_FORMAT,
		extent: Extent,
		is_3d: bool,
		array_layers: u32,
	) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		let array_layers = array_layers.max(1);
		if is_3d {
			assert!(
				array_layers == 1,
				"Invalid DX12 Texture3D UAV. The most likely cause is that array metadata was attached to a 3D texture."
			);
			return D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_UAV {
						MipSlice: 0,
						FirstWSlice: 0,
						WSize: extent.depth().max(1),
					},
				},
			};
		}
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: format,
			ViewDimension: if array_layers > 1 {
				D3D12_UAV_DIMENSION_TEXTURE2DARRAY
			} else {
				D3D12_UAV_DIMENSION_TEXTURE2D
			},
			Anonymous: if array_layers > 1 {
				D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
						MipSlice: 0,
						FirstArraySlice: 0,
						ArraySize: array_layers,
						PlaneSlice: 0,
					},
				}
			} else {
				D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: 0,
						PlaneSlice: 0,
					},
				}
			},
		}
	}

	/// Resolves the native array slice range for a shader image view.
	pub(crate) fn descriptor_array_range(array_layers: u32, layer: Option<u32>) -> (u32, u32) {
		let array_layers = array_layers.max(1);
		if let Some(layer) = layer {
			assert!(
				layer < array_layers,
				"Invalid DX12 image descriptor layer. The most likely cause is that the selected layer exceeds the image array size."
			);
			(layer, 1)
		} else {
			(0, array_layers)
		}
	}

	/// Creates a UAV whose native dimension matches the shader resource declaration.
	pub(crate) fn descriptor_texture_uav_desc(
		format: DXGI_FORMAT,
		texture_view_type: TextureViewTypes,
		extent: Extent,
		is_3d: bool,
		array_layers: u32,
		layer: Option<u32>,
		mip_level: Option<u32>,
	) -> D3D12_UNORDERED_ACCESS_VIEW_DESC {
		let mip_level = mip_level.unwrap_or(0);

		assert!(
			layer.is_none() || texture_view_type == TextureViewTypes::Texture2DArray,
			"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
		);
		if texture_view_type == TextureViewTypes::Texture3D {
			assert!(
				is_3d && array_layers == 1 && layer.is_none(),
				"Invalid DX12 Texture3D UAV. The most likely cause is that the image is 2D or carries array-layer metadata."
			);
			return D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE3D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_UAV {
						MipSlice: mip_level,
						FirstWSlice: 0,
						WSize: crate::image::mip_extent(extent, mip_level).depth().max(1),
					},
				},
			};
		}
		assert!(
			!is_3d,
			"Invalid DX12 2D UAV. The most likely cause is that a Texture3D image was bound to a 2D shader resource."
		);
		if matches!(
			texture_view_type,
			TextureViewTypes::TextureCube | TextureViewTypes::TextureCubeArray
		) {
			panic!("Unsupported DX12 cubemap UAV. The most likely cause is that a read-only cubemap was declared writable.");
		}
		if texture_view_type == TextureViewTypes::Texture2D && layer.is_none() {
			assert!(
				array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata or a selected layer."
			);
			return D3D12_UNORDERED_ACCESS_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
				Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_UAV {
						MipSlice: mip_level,
						PlaneSlice: 0,
					},
				},
			};
		}

		// DX12 represents a selected array layer as a one-slice Texture2DArray view.
		let (first_array_slice, array_size) = Self::descriptor_array_range(array_layers, layer);
		D3D12_UNORDERED_ACCESS_VIEW_DESC {
			Format: format,
			ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2DARRAY,
			Anonymous: D3D12_UNORDERED_ACCESS_VIEW_DESC_0 {
				Texture2DArray: D3D12_TEX2D_ARRAY_UAV {
					MipSlice: mip_level,
					FirstArraySlice: first_array_slice,
					ArraySize: array_size,
					PlaneSlice: 0,
				},
			},
		}
	}

	pub(crate) fn null_texture_srv_desc(texture_view_type: TextureViewTypes) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		match texture_view_type {
			TextureViewTypes::TextureCube => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURECUBE,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					TextureCube: D3D12_TEXCUBE_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::TextureCubeArray => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURECUBEARRAY,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					TextureCubeArray: D3D12_TEXCUBE_ARRAY_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						First2DArrayFace: 0,
						NumCubes: 1,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::Texture2DArray => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						FirstArraySlice: 0,
						ArraySize: 1,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::Texture3D => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE3D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
			TextureViewTypes::Texture2D => D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: DXGI_FORMAT_R8G8B8A8_UNORM,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_SRV {
						MostDetailedMip: 0,
						MipLevels: 1,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			},
		}
	}

	/// Creates an SRV whose native dimension matches the shader resource declaration.
	pub(crate) fn descriptor_texture_srv_desc(
		format: DXGI_FORMAT,
		texture_view_type: TextureViewTypes,
		is_3d: bool,
		array_layers: u32,
		layer: Option<u32>,
		mip_levels: u32,
		mip_level: Option<u32>,
	) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		let most_detailed_mip = mip_level.unwrap_or(0);
		let mip_count = mip_level.map_or(mip_levels, |_| 1);

		assert!(
			layer.is_none() || texture_view_type == TextureViewTypes::Texture2DArray,
			"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
		);
		if texture_view_type == TextureViewTypes::Texture3D {
			assert!(
				is_3d && array_layers == 1 && layer.is_none(),
				"Invalid DX12 Texture3D SRV. The most likely cause is that the image is 2D or carries array-layer metadata."
			);
			return D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE3D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture3D: D3D12_TEX3D_SRV {
						MostDetailedMip: most_detailed_mip,
						MipLevels: mip_count,
						ResourceMinLODClamp: 0.0,
					},
				},
			};
		}
		assert!(
			!is_3d,
			"Invalid DX12 2D SRV. The most likely cause is that a Texture3D image was bound to a 2D shader resource."
		);
		if texture_view_type == TextureViewTypes::TextureCube {
			assert!(
				layer.is_none() && array_layers == 6,
				"Invalid DX12 cubemap descriptor view. The most likely cause is that the image is not a six-layer cubemap."
			);
			return D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURECUBE,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					TextureCube: D3D12_TEXCUBE_SRV {
						MostDetailedMip: most_detailed_mip,
						MipLevels: mip_count,
						ResourceMinLODClamp: 0.0,
					},
				},
			};
		}
		if texture_view_type == TextureViewTypes::TextureCubeArray {
			assert!(
				layer.is_none() && array_layers > 0 && array_layers.is_multiple_of(6),
				"Invalid DX12 cube-array descriptor view. The most likely cause is that the image layer count is not divisible by six."
			);
			return D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURECUBEARRAY,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					TextureCubeArray: D3D12_TEXCUBE_ARRAY_SRV {
						MostDetailedMip: most_detailed_mip,
						MipLevels: mip_count,
						First2DArrayFace: 0,
						NumCubes: array_layers / 6,
						ResourceMinLODClamp: 0.0,
					},
				},
			};
		}
		if texture_view_type == TextureViewTypes::Texture2D && layer.is_none() {
			assert!(
				array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata or a selected layer."
			);
			return D3D12_SHADER_RESOURCE_VIEW_DESC {
				Format: format,
				ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
				Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
				Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
					Texture2D: D3D12_TEX2D_SRV {
						MostDetailedMip: most_detailed_mip,
						MipLevels: mip_count,
						PlaneSlice: 0,
						ResourceMinLODClamp: 0.0,
					},
				},
			};
		}

		// DX12 represents a selected array layer as a one-slice Texture2DArray view.
		let (first_array_slice, array_size) = Self::descriptor_array_range(array_layers, layer);
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: format,
			ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				Texture2DArray: D3D12_TEX2D_ARRAY_SRV {
					MostDetailedMip: most_detailed_mip,
					MipLevels: mip_count,
					FirstArraySlice: first_array_slice,
					ArraySize: array_size,
					PlaneSlice: 0,
					ResourceMinLODClamp: 0.0,
				},
			},
		}
	}

	pub(crate) fn null_acceleration_structure_srv_desc() -> D3D12_SHADER_RESOURCE_VIEW_DESC {
		D3D12_SHADER_RESOURCE_VIEW_DESC {
			Format: DXGI_FORMAT_UNKNOWN,
			ViewDimension: D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE,
			Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV { Location: 0 },
			},
		}
	}
}
