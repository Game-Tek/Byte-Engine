use super::super::*;

impl Device {
	pub(crate) fn bind_descriptor_heaps(&mut self, command_buffer_handle: CommandBufferHandle, sets: &[DescriptorSetHandle]) {
		self.bind_descriptor_heaps_and_tables(command_buffer_handle, None, sets, 0);
	}

	/// Transitions the concrete resources referenced by the active pipeline's retained set union.
	pub(crate) fn flush_pending_descriptor_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let pipeline_kind = pipeline.kind;
		let Some(layout) = self.pipeline_layouts.get(pipeline.layout.0 as usize).cloned() else {
			return;
		};

		let mut retained = SmallVec::<[(ShaderResourceDescriptor, RetainedDescriptor); 32]>::new();
		for resource in &layout.resources {
			for &set_handle in sets {
				let Some(set_handle) = self.descriptor_set_for_sequence(set_handle, sequence_index) else {
					continue;
				};
				let Some(descriptors) = self.descriptor_sets[set_handle.0 as usize]
					.descriptors
					.get(&resource.descriptor.slot())
				else {
					continue;
				};
				retained.extend(
					descriptors
						.values()
						.copied()
						.map(|descriptor| (resource.descriptor, descriptor)),
				);
			}
		}

		// Complete deferred uploads before collecting barriers. Holding a batch across a copy command
		// would move an earlier transition past the command that depends on it.
		for (_, retained_descriptor) in &retained {
			let resource_sequence = self.frame_index_with_offset(
				sequence_index as usize,
				Some(retained_descriptor.frame_offset),
				self.frames as usize,
			) as u8;
			match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => self.sync_buffer_for_sequence(handle, resource_sequence),
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => self.flush_pending_texture_syncs(command_buffer_handle, Some(handle), Some(resource_sequence)),
				_ => {}
			}
		}

		let mut barriers = EnhancedBarrierBatch::default();
		for (resource_descriptor, retained_descriptor) in retained {
			let resource_sequence = self.frame_index_with_offset(
				sequence_index as usize,
				Some(retained_descriptor.frame_offset),
				self.frames as usize,
			) as u8;
			match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => {
					// Buffer contents can change without changing the retained descriptor or its native heap.
					let Some(resource) = self.buffer_resource_for_sequence(handle, resource_sequence) else {
						continue;
					};
					if self.buffer_heap_kind_for_sequence(handle, resource_sequence) != Some(BufferHeapKind::Default) {
						continue;
					}
					self.transition_tracked_buffer_into(
						handle,
						&resource,
						Self::descriptor_buffer_state(resource_descriptor, pipeline_kind),
						&mut barriers,
					);
					self.mark_command_buffer_work(command_buffer_handle);
				}
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => {
					let Some(resource) = self.ensure_image_resource_for_sequence(handle, resource_sequence) else {
						continue;
					};
					self.transition_tracked_image_into(
						handle,
						&resource,
						Self::descriptor_image_state(resource_descriptor, pipeline_kind),
						&mut barriers,
					);
					self.mark_command_buffer_work(command_buffer_handle);
				}
				WriteData::Swapchain(handle) => {
					let image = self
						.get_swapchain_image_for_sequence(handle, Uses::Storage, resource_sequence)
						.0;
					let Some(resource) = self.ensure_image_resource_for_sequence(image.into(), resource_sequence) else {
						continue;
					};
					self.transition_tracked_image_into(
						image.into(),
						&resource,
						Self::descriptor_image_state(resource_descriptor, pipeline_kind),
						&mut barriers,
					);
					self.mark_command_buffer_work(command_buffer_handle);
				}
				_ => {}
			}
		}
		Self::submit_resource_barriers(&command_list, &barriers);
	}

	pub(crate) fn descriptor_matches_kind(descriptor: WriteData, kind: ResourceKind) -> bool {
		match descriptor {
			WriteData::Buffer { .. } => matches!(kind, ResourceKind::UniformBuffer | ResourceKind::StorageBuffer),
			WriteData::Image { .. } | WriteData::Swapchain(_) => {
				matches!(
					kind,
					ResourceKind::SampledImage | ResourceKind::StorageImage | ResourceKind::InputAttachment
				)
			}
			WriteData::CombinedImageSampler { .. } => kind == ResourceKind::CombinedImageSampler,
			WriteData::Sampler(_) => kind == ResourceKind::Sampler,
			WriteData::AccelerationStructure { .. } => kind == ResourceKind::AccelerationStructure,
			WriteData::StaticSamplers | WriteData::CombinedImageSamplerArray => false,
		}
	}

	/// Validates native allocation requirements that are stricter than retained descriptor kinds.
	pub(crate) fn validate_descriptor_resource(
		&self,
		shader_resource: ShaderResourceDescriptor,
		retained: RetainedDescriptor,
		sequence_index: u8,
	) {
		match retained.descriptor {
			WriteData::Buffer { handle, .. } if shader_resource.kind() == ResourceKind::StorageBuffer => {
				let buffer = self.buffer(handle).expect(
					"Invalid DX12 buffer descriptor. The most likely cause is that the retained buffer handle is stale.",
				);

				assert!(
					buffer.uses.intersects(Uses::Storage),
					"Invalid DX12 storage-buffer descriptor. The most likely cause is that the buffer was not created with storage usage."
				);
				if shader_resource.access().intersects(crate::AccessPolicies::WRITE) {
					assert!(
						self.buffer_heap_kind_for_sequence(handle, sequence_index) == Some(BufferHeapKind::Default),
						"Invalid writable DX12 storage-buffer descriptor. The most likely cause is that the buffer uses a host-visible heap that cannot provide a UAV."
					);
				}
			}
			WriteData::Image { handle, mip_level, .. } => {
				self.validate_image_descriptor_resource(shader_resource, handle, None, mip_level)
			}
			WriteData::CombinedImageSampler { image_handle, layer, .. } => {
				self.validate_image_descriptor_resource(shader_resource, image_handle, layer, None)
			}
			_ => {}
		}
	}

	/// Validates image usage, dimension metadata, and optional selected subresources.
	pub(crate) fn validate_image_descriptor_resource(
		&self,
		shader_resource: ShaderResourceDescriptor,
		image_handle: crate::BaseImageHandle,
		layer: Option<u32>,
		mip_level: Option<u32>,
	) {
		let image = self
			.images
			.get(image_handle.0 as usize)
			.expect("Invalid DX12 image descriptor. The most likely cause is that the retained image handle is stale.");

		assert!(
			shader_resource.texture_view() != TextureViewTypes::Texture3D,
			"Unsupported DX12 Texture3D descriptor view. The most likely cause is that the image was allocated by the current 2D-only image path."
		);
		if shader_resource.texture_view() == TextureViewTypes::TextureCubeArray {
			assert!(
				layer.is_none() && image.array_layers > 0 && image.array_layers.is_multiple_of(6),
				"Invalid DX12 cube-array descriptor view. The most likely cause is that the image layer count is not divisible by six."
			);
		}
		if shader_resource.kind() == ResourceKind::StorageImage {
			assert!(
				image.uses.intersects(Uses::Storage),
				"Invalid DX12 storage-image descriptor. The most likely cause is that the image was not created with storage usage."
			);
		}
		if let Some(mip_level) = mip_level {
			assert!(
				mip_level < image.mip_levels,
				"Invalid DX12 image descriptor mip level. The most likely cause is that the selected mip exceeds the image mip count. mip_level={mip_level}, mip_levels={}",
				image.mip_levels,
			);
		}
		if let Some(layer) = layer {
			assert!(
				shader_resource.texture_view() == TextureViewTypes::Texture2DArray,
				"Invalid DX12 selected-layer descriptor. The most likely cause is that the shader resource declares Texture2D instead of Texture2DArray."
			);
			assert!(
				layer < image.array_layers.max(1),
				"Invalid DX12 image descriptor layer. The most likely cause is that the selected layer exceeds the image array size."
			);
		} else if shader_resource.texture_view() == TextureViewTypes::Texture2D {
			assert!(
				image.array_layers <= 1,
				"Invalid DX12 Texture2D descriptor view. The most likely cause is that an array image requires Texture2DArray metadata."
			);
		}
	}

	/// Validates that bound retained sets form one complete, non-overlapping flat resource union.
	pub(crate) fn validate_descriptor_sets(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.0 as usize];
		let sequence_sets = sets
			.iter()
			.map(|&set| self.descriptor_set_for_sequence(set, sequence_index).unwrap_or(set))
			.collect::<SmallVec<[DescriptorSetHandle; 8]>>();

		let mut occupied_slots = HashSet::default();
		for &set_handle in &sequence_sets {
			let set = &self.descriptor_sets[set_handle.0 as usize];
			for &slot in set.descriptors.keys() {
				if layout.resources.iter().any(|resource| resource.descriptor.slot() == slot) {
					assert!(
						occupied_slots.insert(slot),
						"Overlapping retained descriptor sets. The most likely cause is that two bound sets write the same flat resource slot.",
					);
					continue;
				}
				let is_array_interior = layout.resources.iter().any(|resource| {
					let start = resource.descriptor.slot().index();
					let slot = slot.index();
					start < slot && slot < Self::resource_range_end(resource.descriptor)
				});

				assert!(
					!is_array_interior,
					"Invalid retained descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array's base slot.",
				);
				// Retained sets can be shared by several passes, so descriptors outside this pipeline interface remain dormant.
			}
		}

		for resource in &layout.resources {
			let owners = sequence_sets
				.iter()
				.filter_map(|set_handle| {
					self.descriptor_sets[set_handle.0 as usize]
						.descriptors
						.get(&resource.descriptor.slot())
				})
				.collect::<SmallVec<[&HashMap<u32, RetainedDescriptor>; 4]>>();

			assert!(
				owners.len() <= 1,
				"Overlapping retained descriptor sets. The most likely cause is that two bound sets own the same active shader resource.",
			);
			if resource.descriptor.count() == 1 {
				assert!(
					owners.first().is_some_and(|descriptors| descriptors.contains_key(&0)),
					"Missing retained descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					resource.descriptor.slot().index(),
				);
			}
			if let Some(descriptors) = owners.first() {
				for (&array_element, retained) in descriptors.iter() {
					assert!(
						array_element < resource.descriptor.count(),
						"Descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(retained.descriptor, resource.descriptor.kind()),
						"Descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
					self.validate_descriptor_resource(resource.descriptor, *retained, sequence_index);
				}
			}
		}
	}

	pub(crate) fn bind_descriptor_heaps_and_tables(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let layout_handle = pipeline.layout;
		let pipeline_kind = pipeline.kind;

		let Some(materialization) = self.materialize_descriptor_heaps(layout_handle, sets, sequence_index) else {
			return;
		};
		self.retain_descriptor_materialization(command_buffer_handle, &materialization);
		let mut heaps = [None, None];
		let mut heap_count = 0usize;
		if let Some(heap) = materialization.cbv_srv_uav_heap.as_ref() {
			heaps[heap_count] = Some(heap.native.clone());
			heap_count += 1;
		}
		if let Some(heap) = materialization.sampler_heap.as_ref() {
			heaps[heap_count] = Some(heap.native.clone());
			heap_count += 1;
		}
		if heap_count == 0 {
			return;
		}

		unsafe {
			command_list.SetDescriptorHeaps(&heaps[..heap_count]);
		}
		self.descriptor_heap_bind_count += 1;
		let Some(Some(_root_signature)) = self.pipeline_root_signatures.get(layout_handle.0 as usize) else {
			panic!(
				"Failed to bind DX12 descriptor tables because the pipeline layout has no native root signature. The most likely cause is that root signature creation failed while the pipeline kept descriptor table metadata."
			);
		};
		let Some(root_tables) = self.pipeline_root_tables.get(layout_handle.0 as usize).cloned() else {
			return;
		};
		let mut table_binds = 0;
		for table in root_tables {
			let heap = if table.sampler_heap {
				materialization.sampler_heap.as_ref()
			} else {
				materialization.cbv_srv_uav_heap.as_ref()
			};
			let Some(heap) = heap else {
				continue;
			};
			let heap_type = if table.sampler_heap {
				D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
			} else {
				D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
			};
			let handle = self.descriptor_gpu_handle(heap, heap_type, 0);
			unsafe {
				match pipeline_kind {
					PipelineKind::Compute | PipelineKind::RayTracing => {
						command_list.SetComputeRootDescriptorTable(table.root_parameter_index, handle)
					}
					PipelineKind::Raster => command_list.SetGraphicsRootDescriptorTable(table.root_parameter_index, handle),
				}
			}
			table_binds += 1;
			#[cfg(test)]
			{
				self.descriptor_table_bind_records.push(DescriptorTableBindRecord {
					root_parameter_index: table.root_parameter_index,
					set_index: 0,
					binding_index: 0,
					sampler_heap: table.sampler_heap,
					heap_slot: 0,
				});
			}
		}
		self.descriptor_table_bind_count += table_binds;
	}

	pub(crate) fn write_push_constants_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: Option<PipelineHandle>,
		offset: u32,
		bytes: &[u8],
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(pipeline_handle) = pipeline_handle else {
			return;
		};
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let Some(constants) = self.pipeline_root_constants.get(pipeline.layout.0 as usize) else {
			return;
		};

		assert!(
			offset.is_multiple_of(4) && bytes.len().is_multiple_of(4),
			"Invalid DX12 push-constant write alignment. The most likely cause is that the offset or data size is not a multiple of four bytes."
		);
		if bytes.is_empty() {
			return;
		}
		let byte_count = u32::try_from(bytes.len()).expect(
			"Invalid DX12 push-constant write size. The most likely cause is that the data exceeds the addressable root-constant range.",
		);
		let end = offset.checked_add(byte_count).expect(
			"Invalid DX12 push-constant write range. The most likely cause is that the offset and data size overflow the root-constant range.",
		);
		let range = constants
			.iter()
			.find(|range| offset >= range.offset && end <= range.offset.saturating_add(range.size))
			.copied()
			.expect(
				"Invalid DX12 push-constant write range. The most likely cause is that no active pipeline range contains the requested bytes.",
			);

		let destination_offset = offset / 4;
		let word_count = byte_count / 4;
		let compute_root = matches!(pipeline.kind, PipelineKind::Compute | PipelineKind::RayTracing);
		unsafe {
			if compute_root {
				command_list.SetComputeRoot32BitConstants(
					range.root_parameter_index,
					word_count,
					bytes.as_ptr().cast(),
					destination_offset,
				);
			} else {
				command_list.SetGraphicsRoot32BitConstants(
					range.root_parameter_index,
					word_count,
					bytes.as_ptr().cast(),
					destination_offset,
				);
			}
		}
		self.push_constant_write_count += 1;
		#[cfg(test)]
		{
			self.push_constant_write_records.push(PushConstantWriteRecord {
				root_parameter_index: range.root_parameter_index,
				offset,
				size: bytes.len() as u32,
				compute_root,
			});
		}
	}
}
