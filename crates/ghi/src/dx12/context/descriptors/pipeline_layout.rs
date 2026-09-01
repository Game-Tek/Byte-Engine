use super::super::*;

impl Device {
	pub(crate) fn descriptor_range_type(
		descriptor: ShaderResourceDescriptor,
		sampler_heap: bool,
	) -> Option<D3D12_DESCRIPTOR_RANGE_TYPE> {
		match descriptor.kind() {
			ResourceKind::UniformBuffer if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_CBV),
			ResourceKind::StorageBuffer if !sampler_heap && descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_UAV)
			}
			ResourceKind::StorageBuffer if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_SRV),
			ResourceKind::StorageImage if !sampler_heap => Some(D3D12_DESCRIPTOR_RANGE_TYPE_UAV),
			ResourceKind::SampledImage
			| ResourceKind::InputAttachment
			| ResourceKind::AccelerationStructure
			| ResourceKind::CombinedImageSampler
				if !sampler_heap =>
			{
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SRV)
			}
			ResourceKind::Sampler | ResourceKind::CombinedImageSampler if sampler_heap => {
				Some(D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER)
			}
			_ => None,
		}
	}

	pub(crate) fn resource_range_end(descriptor: ShaderResourceDescriptor) -> u32 {
		descriptor
			.slot()
			.index()
			.checked_add(descriptor.count())
			.expect("DX12 shader resource range overflowed. The most likely cause is an invalid flat slot or resource count.")
	}

	pub(crate) fn resource_representations_match(left: ShaderResourceDescriptor, right: ShaderResourceDescriptor) -> bool {
		left.slot() == right.slot()
			&& left.kind() == right.kind()
			&& left.count() == right.count()
			&& left.texture_view() == right.texture_view()
			&& left.buffer_element_stride() == right.buffer_element_stride()
	}

	pub(crate) fn resource_ranges_overlap(left: ShaderResourceDescriptor, right: ShaderResourceDescriptor) -> bool {
		left.slot().index() < Self::resource_range_end(right) && right.slot().index() < Self::resource_range_end(left)
	}

	/// Merges shader resource declarations and assigns dense native heap offsets.
	pub(crate) fn build_pipeline_resources(&self, shaders: &[pipelines::ShaderParameter]) -> Vec<PipelineResource> {
		let mut descriptors = shaders
			.iter()
			.flat_map(|parameter| self.shaders[parameter.handle.0 as usize].resources.iter().copied())
			.collect::<Vec<_>>();
		descriptors.sort_by_key(|descriptor| descriptor.slot());

		let mut merged = Vec::<ShaderResourceDescriptor>::with_capacity(descriptors.len());
		for descriptor in descriptors {
			if let Some(previous) = merged.last_mut() {
				if previous.slot() == descriptor.slot() {
					assert!(
						Self::resource_representations_match(*previous, descriptor),
						"Conflicting DX12 shader resources. The most likely cause is that shader stages declared the same flat slot with incompatible representations.",
					);
					assert!(
						Self::descriptor_range_type(*previous, false) == Self::descriptor_range_type(descriptor, false),
						"Conflicting DX12 storage access. The most likely cause is that shader stages map the same flat slot to different SRV and UAV register classes.",
					);
					*previous = ShaderResourceDescriptor::new(
						previous.slot(),
						previous.kind(),
						previous.count(),
						previous.access() | descriptor.access(),
					)
					.texture_view_type(previous.texture_view())
					.buffer_stride(previous.buffer_element_stride());
					continue;
				}

				assert!(
					!Self::resource_ranges_overlap(*previous, descriptor),
					"Overlapping DX12 shader resources. The most likely cause is that shader resource arrays reserve intersecting flat slot ranges.",
				);
			}
			merged.push(descriptor);
		}

		let mut cbv_srv_uav_offset = 0u32;
		let mut sampler_offset = 0u32;
		merged
			.into_iter()
			.map(|descriptor| {
				let cbv_offset = Self::descriptor_range_type(descriptor, false).map(|_| {
					let offset = cbv_srv_uav_offset;
					cbv_srv_uav_offset = cbv_srv_uav_offset.checked_add(descriptor.count()).expect(
						"DX12 CBV/SRV/UAV descriptor count overflowed. The most likely cause is an invalid shader resource count.",
					);
					offset
				});
				let native_sampler_offset = Self::descriptor_range_type(descriptor, true).map(|_| {
					let offset = sampler_offset;
					sampler_offset = sampler_offset.checked_add(descriptor.count()).expect(
						"DX12 sampler descriptor count overflowed. The most likely cause is an invalid shader resource count.",
					);
					offset
				});
				PipelineResource {
					descriptor,
					cbv_srv_uav_offset: cbv_offset,
					sampler_offset: native_sampler_offset,
				}
			})
			.collect()
	}

	/// Creates one complete native layout with a root signature and its root-parameter indices.
	pub(crate) fn create_native_pipeline_layout(&self, key: PipelineLayout) -> Option<NativePipelineLayout> {
		let mut resource_ranges = SmallVec::<[D3D12_DESCRIPTOR_RANGE1; 16]>::new();
		let mut sampler_ranges = SmallVec::<[D3D12_DESCRIPTOR_RANGE1; 16]>::new();
		for resource in &key.resources {
			if let (Some(range_type), Some(offset)) = (
				Self::descriptor_range_type(resource.descriptor, false),
				resource.cbv_srv_uav_offset,
			) {
				resource_ranges.push(D3D12_DESCRIPTOR_RANGE1 {
					RangeType: range_type,
					NumDescriptors: resource.descriptor.count(),
					BaseShaderRegister: resource.descriptor.slot().index(),
					RegisterSpace: 0,
					// Command buffers own copied descriptors and may sequence GPU writes through a table.
					// Volatile descriptors and data preserve that workflow in the versioned root signature.
					Flags: D3D12_DESCRIPTOR_RANGE_FLAG_DESCRIPTORS_VOLATILE | D3D12_DESCRIPTOR_RANGE_FLAG_DATA_VOLATILE,
					OffsetInDescriptorsFromTableStart: offset,
				});
			}
			if let (Some(range_type), Some(offset)) = (
				Self::descriptor_range_type(resource.descriptor, true),
				resource.sampler_offset,
			) {
				sampler_ranges.push(D3D12_DESCRIPTOR_RANGE1 {
					RangeType: range_type,
					NumDescriptors: resource.descriptor.count(),
					BaseShaderRegister: resource.descriptor.slot().index(),
					RegisterSpace: 0,
					Flags: D3D12_DESCRIPTOR_RANGE_FLAG_DESCRIPTORS_VOLATILE,
					OffsetInDescriptorsFromTableStart: offset,
				});
			}
		}

		let mut parameters = SmallVec::<[D3D12_ROOT_PARAMETER1; 3]>::new();
		let mut resource_table_root = None;
		let mut sampler_table_root = None;
		if !resource_ranges.is_empty() {
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER1 {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
				Anonymous: D3D12_ROOT_PARAMETER1_0 {
					DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE1 {
						NumDescriptorRanges: resource_ranges.len() as u32,
						pDescriptorRanges: resource_ranges.as_ptr(),
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			resource_table_root = Some(root_parameter_index);
		}
		if !sampler_ranges.is_empty() {
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER1 {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
				Anonymous: D3D12_ROOT_PARAMETER1_0 {
					DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE1 {
						NumDescriptorRanges: sampler_ranges.len() as u32,
						pDescriptorRanges: sampler_ranges.as_ptr(),
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			sampler_table_root = Some(root_parameter_index);
		}

		let push_constant_size = key
			.push_constant_ranges
			.iter()
			.map(|range| range.offset.saturating_add(range.size))
			.max()
			.unwrap_or(0);
		let push_constant_dword_count = push_constant_size.div_ceil(4);
		let descriptor_table_count = u32::from(resource_table_root.is_some()) + u32::from(sampler_table_root.is_some());

		assert!(
			push_constant_dword_count.saturating_add(descriptor_table_count) <= 64,
			"DX12 root signature exceeds 64 DWORDs. The most likely cause is that push constants leave insufficient space for the descriptor tables."
		);
		let mut push_constant_root = None;
		if push_constant_size != 0 {
			assert!(
				key.resources.iter().all(|resource| {
					resource.descriptor.kind() != ResourceKind::UniformBuffer || resource.descriptor.slot().index() != 0
				}),
				"Conflicting DX12 root register. The most likely cause is that push constants and a uniform buffer both use b0, space0.",
			);
			let root_parameter_index = parameters.len() as u32;
			parameters.push(D3D12_ROOT_PARAMETER1 {
				ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
				Anonymous: D3D12_ROOT_PARAMETER1_0 {
					Constants: D3D12_ROOT_CONSTANTS {
						ShaderRegister: 0,
						RegisterSpace: 0,
						Num32BitValues: push_constant_dword_count,
					},
				},
				ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
			});
			push_constant_root = Some(root_parameter_index);
		}

		let desc = D3D12_ROOT_SIGNATURE_DESC2 {
			NumParameters: parameters.len() as u32,
			pParameters: if parameters.is_empty() {
				std::ptr::null()
			} else {
				parameters.as_ptr()
			},
			NumStaticSamplers: 0,
			pStaticSamplers: std::ptr::null(),
			Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
		};
		let versioned_desc = D3D12_VERSIONED_ROOT_SIGNATURE_DESC {
			Version: D3D_ROOT_SIGNATURE_VERSION_1_2,
			Anonymous: D3D12_VERSIONED_ROOT_SIGNATURE_DESC_0 { Desc_1_2: desc },
		};
		let mut blob = None;
		let mut error_blob = None;
		let serialization_result = unsafe {
			self.device_configuration
				.SerializeVersionedRootSignature(&versioned_desc, &mut blob, Some(&mut error_blob))
		};
		if let Err(error) = serialization_result {
			let details = if let Some(error_blob) = error_blob {
				let message = unsafe {
					std::slice::from_raw_parts(error_blob.GetBufferPointer().cast::<u8>(), error_blob.GetBufferSize())
				};
				String::from_utf8_lossy(message).into_owned()
			} else {
				"DX12 returned no serialization diagnostics.".to_string()
			};
			self.log_dx12_error(format!(
				"Failed to serialize DX12 root signature. The most likely cause is an invalid root parameter or an incompatible Agility SDK runtime. Error: {error:?}. {details}"
			));
			return None;
		}
		let Some(blob) = blob else {
			self.log_dx12_error(
				"DX12 root-signature serialization returned no data. The most likely cause is an incompatible Agility SDK runtime.",
			);
			return None;
		};
		let bytes = unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) };
		let root_signature = match unsafe { self.device.CreateRootSignature(0, bytes) } {
			Ok(root_signature) => root_signature,
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 root signature. The most likely cause is an invalid serialized layout or a removed device. Parameters: {}, descriptor tables: {descriptor_table_count}, error: {error:?}, device removed reason: {removed_reason:?}",
					parameters.len(),
				));
				return None;
			}
		};

		Some(NativePipelineLayout {
			key,
			root_signature,
			resource_table_root,
			sampler_table_root,
			push_constant_root,
		})
	}

	pub(crate) fn get_or_create_pipeline_layout(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		push_constant_ranges: &[PushConstantRange],
	) -> PipelineLayoutHandle {
		let resources = self.build_pipeline_resources(shaders);
		let key = PipelineLayout {
			cbv_srv_uav_descriptor_count: resources
				.iter()
				.filter_map(|resource| resource.cbv_srv_uav_offset.map(|offset| offset + resource.descriptor.count()))
				.max()
				.unwrap_or(0),
			sampler_descriptor_count: resources
				.iter()
				.filter_map(|resource| resource.sampler_offset.map(|offset| offset + resource.descriptor.count()))
				.max()
				.unwrap_or(0),
			resources,
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		// Build every native object before publishing the handle so a failed root signature cannot misalign layout state.
		let native_layout = self.create_native_pipeline_layout(key.clone()).expect(
			"Failed to create DX12 pipeline layout. The most likely cause is an invalid root signature or a removed device.",
		);
		let handle = PipelineLayoutHandle(self.pipeline_layouts.len() as u64);
		self.pipeline_layouts.push(native_layout);
		self.pipeline_layout_indices.insert(key, handle);
		handle
	}
}
