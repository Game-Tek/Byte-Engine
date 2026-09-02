use super::super::*;

/// The `DescriptorAliasKey` enum identifies one frame-resolved resource exposed through shader descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DescriptorAliasKey {
	Buffer(BaseBufferHandle, u8),
	Image(crate::BaseImageHandle, u8),
	Swapchain(SwapchainHandle, u8),
}

/// The `DescriptorAliasState` enum records the enhanced-barrier contract required by one shader-visible alias.
#[derive(Clone, Copy, Debug)]
enum DescriptorAliasState {
	Buffer(BufferBarrierState),
	Texture(TextureBarrierState),
}

impl DescriptorAliasState {
	/// Merges compatible read aliases and rejects overlapping whole-resource writes.
	fn merge(&mut self, other: Self) {
		match (self, other) {
			(Self::Buffer(current), Self::Buffer(other)) => {
				assert!(
					current.access != D3D12_BARRIER_ACCESS_UNORDERED_ACCESS
						&& other.access != D3D12_BARRIER_ACCESS_UNORDERED_ACCESS,
					"Incompatible DX12 buffer descriptor aliases. The most likely cause is that one whole buffer was bound simultaneously for overlapping shader reads and writes. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#single-queue-simultaneous-access."
				);
				current.sync |= other.sync;
				current.access |= other.access;
			}
			(Self::Texture(current), Self::Texture(other)) => {
				assert!(
					current.layout == other.layout
						&& current.access != D3D12_BARRIER_ACCESS_UNORDERED_ACCESS
						&& other.access != D3D12_BARRIER_ACCESS_UNORDERED_ACCESS,
					"Incompatible DX12 image descriptor aliases. The most likely cause is that one whole image was bound simultaneously with incompatible shader-resource and unordered-access layouts. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#layout-access-compatibility."
				);
				current.sync |= other.sync;
				current.access |= other.access;
			}
			_ => unreachable!("A DX12 descriptor alias key cannot identify both a buffer and an image."),
		}
	}
}

/// The `DescriptorAliasUse` struct keeps one unique resource and its merged shader access contract.
#[derive(Clone, Copy, Debug)]
struct DescriptorAliasUse {
	key: DescriptorAliasKey,
	state: DescriptorAliasState,
}

const DESCRIPTOR_LINEAR_SEARCH_LIMIT: usize = 32;

/// The `DescriptorAliasCollection` struct keeps small descriptor tables allocation-free and indexes larger tables.
struct DescriptorAliasCollection {
	entries: SmallVec<[DescriptorAliasUse; DESCRIPTOR_LINEAR_SEARCH_LIMIT]>,
	indices: Option<HashMap<DescriptorAliasKey, usize>>,
	capacity_hint: usize,
}

impl DescriptorAliasCollection {
	fn new(capacity_hint: usize) -> Self {
		Self {
			entries: SmallVec::new(),
			indices: None,
			capacity_hint,
		}
	}

	/// Merges one alias in constant time after a large table exceeds the inline search limit.
	fn merge_or_insert(&mut self, alias: DescriptorAliasUse) {
		let existing_index = self
			.indices
			.as_ref()
			.and_then(|indices| indices.get(&alias.key).copied())
			.or_else(|| {
				self.indices
					.is_none()
					.then(|| self.entries.iter().position(|entry| entry.key == alias.key))
					.flatten()
			});
		if let Some(existing_index) = existing_index {
			self.entries[existing_index].state.merge(alias.state);
			return;
		}

		let index = self.entries.len();
		if let Some(indices) = self.indices.as_mut() {
			indices.insert(alias.key, index);
		} else if index == DESCRIPTOR_LINEAR_SEARCH_LIMIT {
			// Build the index once. Typical descriptor tables remain entirely inline.
			let mut indices = HashMap::default();
			indices.reserve(self.capacity_hint.max(index + 1));
			for (existing_index, entry) in self.entries.iter().enumerate() {
				indices.insert(entry.key, existing_index);
			}
			indices.insert(alias.key, index);
			self.indices = Some(indices);
		}
		self.entries.push(alias);
	}
}

/// The `DescriptorRequirementKey` enum identifies one native resource and its barrier category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DescriptorRequirementKey {
	Buffer(usize),
	Image(usize),
}

/// The `DescriptorRequirementHandle` enum preserves the logical handle needed to update resource state tracking.
enum DescriptorRequirementHandle {
	Buffer(BaseBufferHandle),
	Image(crate::BaseImageHandle),
}

/// The `DescriptorRequirement` struct groups compatible shader reads of one native resource into one barrier.
struct DescriptorRequirement {
	key: DescriptorRequirementKey,
	handle: DescriptorRequirementHandle,
	resource: ID3D12Resource,
	state: DescriptorAliasState,
}

/// The `DescriptorRequirementCollection` struct avoids quadratic native-resource scans for large descriptor tables.
struct DescriptorRequirementCollection {
	entries: SmallVec<[DescriptorRequirement; DESCRIPTOR_LINEAR_SEARCH_LIMIT]>,
	indices: Option<HashMap<DescriptorRequirementKey, usize>>,
	capacity_hint: usize,
}

impl DescriptorRequirementCollection {
	fn new(capacity_hint: usize) -> Self {
		Self {
			entries: SmallVec::new(),
			indices: None,
			capacity_hint,
		}
	}

	/// Merges one native requirement while preserving the first logical handle chosen for state tracking.
	fn merge_or_insert(&mut self, requirement: DescriptorRequirement) {
		let existing_index = self
			.indices
			.as_ref()
			.and_then(|indices| indices.get(&requirement.key).copied())
			.or_else(|| {
				self.indices
					.is_none()
					.then(|| self.entries.iter().position(|entry| entry.key == requirement.key))
					.flatten()
			});
		if let Some(existing_index) = existing_index {
			self.entries[existing_index].state.merge(requirement.state);
			return;
		}

		let index = self.entries.len();
		if let Some(indices) = self.indices.as_mut() {
			indices.insert(requirement.key, index);
		} else if index == DESCRIPTOR_LINEAR_SEARCH_LIMIT {
			// Build the index once. Typical descriptor tables remain entirely inline.
			let mut indices = HashMap::default();
			indices.reserve(self.capacity_hint.max(index + 1));
			for (existing_index, entry) in self.entries.iter().enumerate() {
				indices.insert(entry.key, existing_index);
			}
			indices.insert(requirement.key, index);
			self.indices = Some(indices);
		}
		self.entries.push(requirement);
	}
}

impl Device {
	pub(crate) fn bind_descriptor_heaps(&mut self, command_buffer_handle: CommandBufferHandle, sets: &[DescriptorSetHandle]) {
		self.bind_descriptor_heaps_and_tables(command_buffer_handle, None, sets, 0);
	}

	/// Returns the native list only while its reusable command-buffer handle owns an open recording.
	fn descriptor_command_list_for_recording(&self, command_buffer_handle: CommandBufferHandle) -> ID3D12GraphicsCommandList7 {
		let command_buffer = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		assert!(
			command_buffer.lifecycle == CommandBufferLifecycle::Recording && command_buffer.is_open,
			"DX12 descriptors require an open command-buffer recording. The most likely cause is that descriptor barriers or native tables were bound before recording began or after it ended."
		);
		command_buffer.command_list.clone().expect(
			"Missing DX12 command list. The most likely cause is that native command-buffer creation failed before descriptor binding.",
		)
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
		let command_list = self.descriptor_command_list_for_recording(command_buffer_handle);
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		let pipeline_kind = pipeline.kind;
		let Some(layout) = self
			.pipeline_layouts
			.get(pipeline.layout.0 as usize)
			.map(|layout| &layout.key)
		else {
			return;
		};

		let mut retained = std::mem::take(&mut self.command_buffers[command_buffer_handle.0 as usize].descriptor_sync_scratch);
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
				// Native buffer resolution centrally flushes dirty CPU shadows.
				WriteData::Buffer { .. } => {}
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => self.flush_pending_texture_syncs(command_buffer_handle, Some(handle), Some(resource_sequence)),
				_ => {}
			}
		}

		let mut requirements = DescriptorRequirementCollection::new(retained.len());
		for (resource_descriptor, retained_descriptor) in retained.drain(..) {
			let resource_sequence = self.frame_index_with_offset(
				sequence_index as usize,
				Some(retained_descriptor.frame_offset),
				self.frames as usize,
			) as u8;
			match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => {
					// Buffer contents can change without changing the retained descriptor or its native heap.
					let resource = self.buffer_resource_for_sequence(handle, resource_sequence).expect(
						"Missing DX12 descriptor buffer resource. The most likely cause is that native buffer allocation failed before the descriptor was bound.",
					);
					let state = Self::descriptor_buffer_state(resource_descriptor, pipeline_kind);
					requirements.merge_or_insert(DescriptorRequirement {
						key: DescriptorRequirementKey::Buffer(Self::native_resource_key(&resource)),
						handle: DescriptorRequirementHandle::Buffer(handle),
						resource,
						state: DescriptorAliasState::Buffer(state),
					});
				}
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => {
					let resource = self.ensure_image_resource_for_sequence(handle, resource_sequence).expect(
						"Missing DX12 descriptor image resource. The most likely cause is that a deferred image was bound before native allocation completed.",
					);
					let state = Self::descriptor_image_state(resource_descriptor, pipeline_kind);
					requirements.merge_or_insert(DescriptorRequirement {
						key: DescriptorRequirementKey::Image(Self::native_resource_key(&resource)),
						handle: DescriptorRequirementHandle::Image(handle),
						resource,
						state: DescriptorAliasState::Texture(state),
					});
				}
				WriteData::Swapchain(handle) => {
					let uses = Self::descriptor_image_use(resource_descriptor);
					let image = self.get_swapchain_image_for_sequence(handle, uses, resource_sequence).0;
					let resource = self.ensure_image_resource_for_sequence(image.into(), resource_sequence).expect(
						"Missing DX12 swapchain proxy resource. The most likely cause is that proxy image allocation failed before presentation.",
					);
					let state = Self::descriptor_image_state(resource_descriptor, pipeline_kind);
					requirements.merge_or_insert(DescriptorRequirement {
						key: DescriptorRequirementKey::Image(Self::native_resource_key(&resource)),
						handle: DescriptorRequirementHandle::Image(image.into()),
						resource,
						state: DescriptorAliasState::Texture(state),
					});
				}
				WriteData::AccelerationStructure { handle } => {
					let resource = self
						.top_level_acceleration_structures
						.get(handle.0 as usize)
						.and_then(|acceleration_structure| acceleration_structure.resource.clone())
						.expect(
							"Missing DX12 acceleration-structure descriptor resource. The most likely cause is that native acceleration-structure allocation failed before binding.",
						);
					self.retain_command_buffer_resource(command_buffer_handle, &resource);
				}
				_ => {}
			}
		}
		self.command_buffers[command_buffer_handle.0 as usize].descriptor_sync_scratch = retained;

		let mut barriers = EnhancedBarrierBatch::default();
		for requirement in requirements.entries {
			match (requirement.handle, requirement.state) {
				(DescriptorRequirementHandle::Buffer(handle), DescriptorAliasState::Buffer(state)) => {
					let heap_kind = self
						.buffer_heap_kind_for_resource(handle, &requirement.resource)
						.unwrap_or(BufferHeapKind::Default);
					self.transition_tracked_buffer_into(handle, &requirement.resource, state, &mut barriers);
					if heap_kind == BufferHeapKind::Default {
						self.mark_command_buffer_work(command_buffer_handle);
					}
				}
				(DescriptorRequirementHandle::Image(handle), DescriptorAliasState::Texture(state)) => {
					self.transition_tracked_image_into(handle, &requirement.resource, state, &mut barriers);
					self.mark_command_buffer_work(command_buffer_handle);
				}
				_ => unreachable!("A DX12 native descriptor requirement cannot change resource category."),
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

	/// Normalizes a dynamic buffer alias to the frame resource selected by its descriptor offset.
	fn descriptor_buffer_alias_key(&self, handle: BaseBufferHandle, sequence_index: u8) -> DescriptorAliasKey {
		let (_, dynamic) = Self::buffer_index(handle);
		DescriptorAliasKey::Buffer(handle, if dynamic { sequence_index } else { 0 })
	}

	/// Normalizes a dynamic image alias to the frame resource selected by its descriptor offset.
	fn descriptor_image_alias_key(&self, handle: crate::BaseImageHandle, sequence_index: u8) -> DescriptorAliasKey {
		let dynamic = self
			.images
			.get(handle.0 as usize)
			.is_some_and(|image| image.frame_resources.is_some());
		DescriptorAliasKey::Image(handle, if dynamic { sequence_index } else { 0 })
	}

	/// Collects active descriptors while proving that every repeated whole-resource binding is compatible.
	fn descriptor_alias_uses(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> SmallVec<[DescriptorAliasUse; 32]> {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.0 as usize].key;
		let pipeline_kind = pipeline.kind;
		let capacity_hint = layout
			.resources
			.iter()
			.map(|resource| resource.descriptor.count() as usize)
			.sum();
		let mut aliases = DescriptorAliasCollection::new(capacity_hint);

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

				for retained in descriptors.values() {
					let resource_sequence = self.frame_index_with_offset(
						sequence_index as usize,
						Some(retained.frame_offset),
						self.frames as usize,
					) as u8;
					let alias = match retained.descriptor {
						WriteData::Buffer { handle, .. } => DescriptorAliasUse {
							key: self.descriptor_buffer_alias_key(handle, resource_sequence),
							state: DescriptorAliasState::Buffer(Self::descriptor_buffer_state(
								resource.descriptor,
								pipeline_kind,
							)),
						},
						WriteData::Image { handle, .. }
						| WriteData::CombinedImageSampler {
							image_handle: handle, ..
						} => DescriptorAliasUse {
							key: self.descriptor_image_alias_key(handle, resource_sequence),
							state: DescriptorAliasState::Texture(Self::descriptor_image_state(
								resource.descriptor,
								pipeline_kind,
							)),
						},
						WriteData::Swapchain(handle) => DescriptorAliasUse {
							key: DescriptorAliasKey::Swapchain(handle, resource_sequence),
							state: DescriptorAliasState::Texture(Self::descriptor_image_state(
								resource.descriptor,
								pipeline_kind,
							)),
						},
						_ => continue,
					};

					aliases.merge_or_insert(alias);
				}
			}
		}

		aliases.entries
	}

	/// Rejects shader descriptors that alias an attachment still active in the render pass.
	fn validate_descriptor_attachment_aliases(
		&self,
		aliases: &[DescriptorAliasUse],
		sequence_index: u8,
		attachments: &[crate::BaseImageHandle],
	) {
		if attachments.is_empty() {
			return;
		}

		for &attachment in attachments {
			let attachment_key = self.descriptor_image_alias_key(attachment, sequence_index);
			assert!(
				aliases.iter().all(|alias| alias.key != attachment_key),
				"Incompatible DX12 attachment and descriptor aliases. The most likely cause is that one whole image was bound simultaneously as an attachment and shader resource. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#layout-access-compatibility."
			);
		}
	}

	/// Validates one binding and reuses its resolved aliases for active-attachment checks.
	pub(crate) fn validate_descriptor_binding_contract(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
		attachments: &[crate::BaseImageHandle],
	) {
		let aliases = self.validate_descriptor_sets_and_collect_aliases(pipeline_handle, sets, sequence_index);
		self.validate_descriptor_attachment_aliases(&aliases, sequence_index, attachments);
	}

	/// Validates native allocation requirements that are stricter than retained descriptor kinds.
	pub(crate) fn validate_descriptor_resource(
		&self,
		shader_resource: ShaderResourceDescriptor,
		retained: RetainedDescriptor,
		sequence_index: u8,
	) {
		match retained.descriptor {
			WriteData::Buffer { handle, size } if shader_resource.kind() == ResourceKind::UniformBuffer => {
				let buffer = self.buffer(handle).expect(
					"Invalid DX12 buffer descriptor. The most likely cause is that the retained buffer handle is stale.",
				);
				assert!(
					self.buffer_heap_kind_for_sequence(handle, sequence_index) != Some(BufferHeapKind::Readback),
					"Invalid DX12 uniform-buffer descriptor. The most likely cause is that a readback-heap buffer was bound for GPU reads. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#readback-heap-resources."
				);
				assert!(
					buffer.uses.intersects(Uses::Uniform),
					"Invalid DX12 uniform-buffer descriptor. The most likely cause is that the buffer was not created with uniform usage."
				);
				let requested_size = match size {
					crate::Ranges::Size(size) => size,
					crate::Ranges::Whole => buffer.size,
				};
				assert!(
					requested_size <= buffer.size,
					"Invalid DX12 buffer descriptor range. The most likely cause is that the requested descriptor size exceeds the logical buffer allocation."
				);
				assert!(
					requested_size != 0 && requested_size <= 64 * 1024,
					"Invalid DX12 constant-buffer view size. The most likely cause is that the requested range is empty or exceeds the 64-KiB shader-visible constant-buffer limit. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/constants."
				);
			}
			WriteData::Buffer { handle, .. } if shader_resource.kind() == ResourceKind::StorageBuffer => {
				let buffer = self.buffer(handle).expect(
					"Invalid DX12 buffer descriptor. The most likely cause is that the retained buffer handle is stale.",
				);
				assert!(
					self.buffer_heap_kind_for_sequence(handle, sequence_index) != Some(BufferHeapKind::Readback),
					"Invalid DX12 storage-buffer descriptor. The most likely cause is that a readback-heap buffer was bound for shader access. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#readback-heap-resources."
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
			image.extent.width() != 0 && image.extent.height() != 0 && (!image.is_3d || image.extent.depth() != 0),
			"Invalid DX12 image descriptor extent. The most likely cause is that a deferred zero-sized image was bound before it was resized."
		);

		if shader_resource.texture_view() == TextureViewTypes::Texture3D {
			assert!(
				image.is_3d && image.array_layers == 1 && layer.is_none(),
				"Invalid DX12 Texture3D descriptor view. The most likely cause is that the image is 2D, has array metadata, or selects a 2D array layer."
			);
		} else {
			assert!(
				!image.is_3d,
				"Invalid DX12 2D descriptor view. The most likely cause is that a Texture3D image was bound to a 2D, array, or cubemap shader resource."
			);
		}
		if shader_resource.texture_view() == TextureViewTypes::TextureCubeArray {
			assert!(
				layer.is_none() && image.array_layers > 0 && image.array_layers.is_multiple_of(6),
				"Invalid DX12 cube-array descriptor view. The most likely cause is that the image layer count is not divisible by six."
			);
		}
		let required_use = Self::descriptor_image_use(shader_resource);
		assert!(
			required_use.is_empty() || image.uses.intersects(required_use),
			"Invalid DX12 image descriptor usage. The most likely cause is that the image was not created for the shader resource kind declared by the active pipeline."
		);
		if shader_resource.kind() == ResourceKind::StorageImage {
			self.validate_typed_uav_format_support(image.format, shader_resource.access());
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
		let _ = self.validate_descriptor_sets_and_collect_aliases(pipeline_handle, sets, sequence_index);
	}

	/// Validates retained sets and returns their already-merged aliases for the next binding check.
	fn validate_descriptor_sets_and_collect_aliases(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> SmallVec<[DescriptorAliasUse; DESCRIPTOR_LINEAR_SEARCH_LIMIT]> {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.0 as usize].key;
		let sequence_sets = sets
			.iter()
			.map(|&set| self.descriptor_set_for_sequence(set, sequence_index).unwrap_or(set))
			.collect::<SmallVec<[DescriptorSetHandle; 8]>>();

		for &set_handle in &sequence_sets {
			let set = &self.descriptor_sets[set_handle.0 as usize];
			for &slot in set.descriptors.keys() {
				// Pipeline resources are sorted and non-overlapping, so only the insertion point and its predecessor can match.
				let slot_index = slot.index();
				let insertion_index = layout
					.resources
					.partition_point(|resource| resource.descriptor.slot().index() < slot_index);
				if layout
					.resources
					.get(insertion_index)
					.is_some_and(|resource| resource.descriptor.slot() == slot)
				{
					continue;
				}
				let is_array_interior = insertion_index.checked_sub(1).is_some_and(|previous_index| {
					let descriptor = layout.resources[previous_index].descriptor;
					descriptor.slot().index() < slot_index && slot_index < Self::resource_range_end(descriptor)
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

		// Descriptor tables expose every retained view to one draw or dispatch. Validate repeated
		// resources as one simultaneous access contract before any barrier or native heap write.
		self.descriptor_alias_uses(pipeline_handle, sets, sequence_index)
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
		let command_list = self.descriptor_command_list_for_recording(command_buffer_handle);
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
		let cbv_srv_uav_identity = materialization
			.cbv_srv_uav_heap
			.as_ref()
			.map(|heap| heap.native.as_raw() as usize);
		let sampler_identity = materialization
			.sampler_heap
			.as_ref()
			.map(|heap| heap.native.as_raw() as usize);
		let heaps_changed = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.is_none_or(|command_buffer| {
				command_buffer.bound_cbv_srv_uav_heap != cbv_srv_uav_identity
					|| command_buffer.bound_sampler_heap != sampler_identity
			});

		if heaps_changed {
			unsafe {
				command_list.SetDescriptorHeaps(&heaps[..heap_count]);
			}
			if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
				command_buffer.bound_cbv_srv_uav_heap = cbv_srv_uav_identity;
				command_buffer.bound_sampler_heap = sampler_identity;
			}
			self.descriptor_heap_bind_count += 1;
		}
		let Some((resource_table_root, sampler_table_root)) = self
			.pipeline_layouts
			.get(layout_handle.0 as usize)
			.map(|layout| (layout.resource_table_root, layout.sampler_table_root))
		else {
			return;
		};
		let mut table_binds = 0;
		for (root_parameter_index, sampler_heap) in [(resource_table_root, false), (sampler_table_root, true)] {
			let Some(root_parameter_index) = root_parameter_index else {
				continue;
			};
			let heap = if sampler_heap {
				materialization.sampler_heap.as_ref()
			} else {
				materialization.cbv_srv_uav_heap.as_ref()
			};
			let Some(heap) = heap else {
				continue;
			};
			let heap_type = if sampler_heap {
				D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER
			} else {
				D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV
			};
			let handle = self.descriptor_gpu_handle(heap, heap_type, 0);
			unsafe {
				match pipeline_kind {
					PipelineKind::Compute | PipelineKind::RayTracing => {
						command_list.SetComputeRootDescriptorTable(root_parameter_index, handle)
					}
					PipelineKind::Raster => command_list.SetGraphicsRootDescriptorTable(root_parameter_index, handle),
				}
			}
			table_binds += 1;
			#[cfg(test)]
			{
				self.descriptor_table_bind_records.push(DescriptorTableBindRecord {
					root_parameter_index,
					set_index: 0,
					binding_index: 0,
					sampler_heap,
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
		let Some((pipeline_kind, layout_handle)) = self
			.pipelines
			.get(pipeline_handle.0 as usize)
			.map(|pipeline| (pipeline.kind, pipeline.layout))
		else {
			return;
		};
		let Some(layout) = self.pipeline_layouts.get(layout_handle.0 as usize) else {
			return;
		};
		let Some(root_parameter_index) = layout.push_constant_root else {
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
		assert!(
			layout
				.key
				.push_constant_ranges
				.iter()
				.any(|range| offset >= range.offset && end <= range.offset.saturating_add(range.size)),
			"Invalid DX12 push-constant write range. The most likely cause is that no active pipeline range contains the requested bytes.",
		);

		let destination_offset = offset / 4;
		let word_count = byte_count / 4;
		let compute_root = matches!(pipeline_kind, PipelineKind::Compute | PipelineKind::RayTracing);
		unsafe {
			if compute_root {
				command_list.SetComputeRoot32BitConstants(
					root_parameter_index,
					word_count,
					bytes.as_ptr().cast(),
					destination_offset,
				);
			} else {
				command_list.SetGraphicsRoot32BitConstants(
					root_parameter_index,
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
				root_parameter_index,
				offset,
				size: bytes.len() as u32,
				compute_root,
			});
		}
	}
}
