use super::super::*;

/// The `DescriptorAliasState` enum records the enhanced-barrier contract required by one shader-visible alias.
#[derive(Clone, Copy, Debug)]
enum DescriptorAliasState {
	Buffer(BufferBarrierState),
	Texture(TextureBarrierState),
}

impl DescriptorAliasState {
	/// Merges compatible aliases: buffer reads, and texture views whose layouts one layout serves. Rejects buffer writes
	/// alongside other views, and texture views no single layout serves.
	///
	/// Storage views of one texture share the unordered-access layout, so a pass may write several of its mips at once.
	/// A texture that is also sampled, as when a pass reads one mip and writes the next, takes the direct-queue common
	/// layout, which serves both. Every DX12 workload runs on a direct queue, so that layout is always available.
	fn merge(&mut self, other: Self, key: impl std::fmt::Debug) {
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
				let shader_access = |layout| {
					matches!(
						layout,
						D3D12_BARRIER_LAYOUT_SHADER_RESOURCE
							| D3D12_BARRIER_LAYOUT_UNORDERED_ACCESS
							| D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_COMMON
					)
				};
				let compatible =
					current.layout == other.layout || (shader_access(current.layout) && shader_access(other.layout));
				assert!(
					compatible,
					"Incompatible DX12 image descriptor aliases: {key:?} is bound in layouts {:?} and {:?}. The most likely cause is that the bound sets use one image both through shader views and in a fixed-function role. Bind it in one role per pass. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#layout-access-compatibility.",
					current.layout, other.layout,
				);
				if current.layout != other.layout {
					current.layout = D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_COMMON;
				}
				current.sync |= other.sync;
				current.access |= other.access;
			}
			_ => unreachable!("A DX12 descriptor alias key cannot identify both a buffer and an image."),
		}
	}
}

const DESCRIPTOR_LINEAR_SEARCH_LIMIT: usize = 32;

/// The `DescriptorRequirement` struct groups compatible shader uses of one native resource into one barrier.
struct DescriptorRequirement {
	resource: ID3D12Resource,
	/// The image whose state tracking the barrier updates, or `None` for a buffer.
	image: Option<crate::BaseImageHandle>,
	state: DescriptorAliasState,
	/// Whether any descriptor that names the resource may write it.
	writes: bool,
}

/// The `DescriptorRequirementCollection` struct merges the requirements of large descriptor tables without quadratic
/// native-resource scans, and keeps small tables allocation-free.
#[derive(Default)]
struct DescriptorRequirementCollection {
	/// The requirements by native resource key.
	entries: SmallVec<[(usize, DescriptorRequirement); DESCRIPTOR_LINEAR_SEARCH_LIMIT]>,
	/// The position of each key in `entries`, built once the table outgrows a linear search.
	indices: Option<HashMap<usize, usize>>,
}

impl DescriptorRequirementCollection {
	/// Merges one requirement into the one of its native resource, or adds it. Panics when the two uses conflict; see
	/// [`DescriptorAliasState::merge`].
	fn merge_or_insert(&mut self, key: usize, requirement: DescriptorRequirement) {
		let existing_index = match &self.indices {
			Some(indices) => indices.get(&key).copied(),
			None => self.entries.iter().position(|(entry_key, _)| *entry_key == key),
		};
		if let Some(existing_index) = existing_index {
			let existing = &mut self.entries[existing_index].1;
			existing.state.merge(requirement.state, key);
			existing.writes |= requirement.writes;
			return;
		}

		let index = self.entries.len();
		if let Some(indices) = self.indices.as_mut() {
			indices.insert(key, index);
		} else if index == DESCRIPTOR_LINEAR_SEARCH_LIMIT {
			// Build the index once. Typical descriptor tables remain entirely inline.
			let mut indices = self
				.entries
				.iter()
				.enumerate()
				.map(|(entry_index, (entry_key, _))| (*entry_key, entry_index))
				.collect::<HashMap<_, _>>();
			indices.insert(key, index);
			self.indices = Some(indices);
		}
		self.entries.push((key, requirement));
	}
}

impl Device {
	/// Returns the native list only while its reusable command-buffer handle owns an open recording.
	fn descriptor_command_list_for_recording(&self, command_buffer_handle: CommandBufferHandle) -> ID3D12GraphicsCommandList7 {
		let command_buffer = self
			.command_buffers
			.get(command_buffer_handle.index() as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		assert!(
			command_buffer.lifecycle == CommandBufferLifecycle::Recording && command_buffer.is_open,
			"DX12 descriptors require an open command-buffer recording. The most likely cause is that descriptor barriers or native tables were bound before recording began or after it ended."
		);
		command_buffer.command_list.clone()
	}

	/// Returns, for each set the pipeline's stages read, the frame copy for `sequence_index` of the bound set created
	/// from the same layout, as an index into the device's descriptor sets.
	///
	/// # Panics
	///
	/// Panics when no bound set, or more than one, was created from a layout the pipeline reads.
	pub(crate) fn resolve_bound_sets(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> SmallVec<[usize; 4]> {
		let pipeline = &self.pipelines[pipeline_handle.index() as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.index() as usize];
		layout
			.sets
			.iter()
			.map(|pipeline_set| {
				let identity = pipeline_set.layout.identity();
				let mut matches = sets
					.iter()
					.filter(|set| self.descriptor_sets[set.index() as usize].layout.identity() == identity);
				let set = *matches.next().unwrap_or_else(|| {
					panic!(
						"Missing descriptor set for layout `{}`. The most likely cause is that no set created from this layout was bound for the pipeline, or that a shader was built from another version of the layout. Bind a set created from the layout the shader declares.",
						pipeline_set.layout.name(),
					)
				});
				assert!(
					matches.next().is_none(),
					"Ambiguous descriptor sets for layout `{}`. The most likely cause is that two sets created from the same layout were bound for one pipeline. Bind one set per layout.",
					pipeline_set.layout.name(),
				);
				self.descriptor_set_for_sequence(set, sequence_index).unwrap_or(set).index() as usize
			})
			.collect()
	}

	/// Calls `visit` with each retained descriptor that the pipeline's stages use in the resolved sets, and the layout
	/// resource it fills.
	fn visit_used_descriptors(
		&self,
		layout: &NativePipelineLayout,
		resolved: &[usize],
		mut visit: impl FnMut(ShaderResourceDescriptor, RetainedDescriptor),
	) {
		for (pipeline_set, &set_index) in layout.sets.iter().zip(resolved) {
			let set = &self.descriptor_sets[set_index];
			for (resource_index, &resource) in set.layout.resources().iter().enumerate() {
				if pipeline_set.used & (1 << resource_index) == 0 {
					continue;
				}
				let base = set.layout.element_base(resource_index) as usize;
				for retained in set.descriptors[base..base + resource.count() as usize].iter().flatten() {
					visit(resource, *retained);
				}
			}
		}
	}

	/// Transitions the resources the bound sets give the pipeline into the states its stages use them in.
	pub(crate) fn flush_pending_descriptor_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let command_list = self.descriptor_command_list_for_recording(command_buffer_handle);
		let resolved = self.resolve_bound_sets(pipeline_handle, sets, sequence_index);
		let pipeline = &self.pipelines[pipeline_handle.index() as usize];
		let pipeline_kind = pipeline.kind;
		let layout = &self.pipeline_layouts[pipeline.layout.index() as usize];

		// The scratch list keeps its capacity in the command buffer, so binding allocates nothing once it has grown.
		let mut retained =
			std::mem::take(&mut self.command_buffers[command_buffer_handle.index() as usize].descriptor_sync_scratch);
		self.visit_used_descriptors(layout, &resolved, |resource, descriptor| {
			retained.push((resource, descriptor))
		});

		let mut requirements = DescriptorRequirementCollection::default();
		for (resource_descriptor, retained_descriptor) in retained.drain(..) {
			let resource_sequence = self.frame_index_with_offset(sequence_index, retained_descriptor.frame_offset);
			let writes = resource_descriptor.access().intersects(crate::AccessPolicies::WRITE);
			let (resource, image, state) = match retained_descriptor.descriptor {
				WriteData::Buffer { handle, .. } => {
					// Buffer contents can change without changing the retained descriptor or its native heap.
					let resource = self.buffer_resource_for_sequence(handle, resource_sequence).expect(
						"Missing DX12 descriptor buffer resource. The most likely cause is that native buffer allocation failed before the descriptor was bound.",
					);
					let state = DescriptorAliasState::Buffer(Self::descriptor_buffer_state(resource_descriptor, pipeline_kind));
					(resource, None, state)
				}
				WriteData::Image { handle, .. }
				| WriteData::CombinedImageSampler {
					image_handle: handle, ..
				} => {
					// An image without memory yet is bound as a null view, which needs no transition.
					let Some(resource) = self.ensure_image_resource_for_sequence(handle, resource_sequence) else {
						continue;
					};
					let state = DescriptorAliasState::Texture(Self::descriptor_image_state(resource_descriptor, pipeline_kind));
					(resource, Some(handle), state)
				}
				WriteData::Swapchain(handle) => {
					let uses = Self::descriptor_image_use(resource_descriptor);
					let image = self.get_swapchain_image(handle, uses).0;
					let resource = self.ensure_image_resource_for_sequence(image.into(), resource_sequence).expect(
						"Missing DX12 swapchain proxy resource. The most likely cause is that proxy image allocation failed before presentation.",
					);
					let state = DescriptorAliasState::Texture(Self::descriptor_image_state(resource_descriptor, pipeline_kind));
					(resource, Some(image.into()), state)
				}
				// Builds end with a global barrier that makes structures readable, so a structure needs no transition, only to
				// live until the recording completes.
				WriteData::AccelerationStructure { handle } => {
					if let Some(resource) = self.top_level_copy_resource(handle, resource_sequence) {
						self.retain_command_buffer_resource(command_buffer_handle, &resource);
					}
					continue;
				}
				WriteData::Sampler(_) => continue,
			};
			requirements.merge_or_insert(
				Self::native_resource_key(&resource),
				DescriptorRequirement {
					resource,
					image,
					state,
					writes,
				},
			);
		}
		self.command_buffers[command_buffer_handle.index() as usize].descriptor_sync_scratch = retained;

		let mut barriers = EnhancedBarrierBatch::default();
		for (_, requirement) in &requirements.entries {
			match (requirement.image, requirement.state) {
				(None, DescriptorAliasState::Buffer(state)) => {
					self.transition_tracked_buffer_into(&requirement.resource, state, requirement.writes, &mut barriers);
				}
				(Some(image), DescriptorAliasState::Texture(state)) => {
					self.transition_tracked_image_into(image, &requirement.resource, state, requirement.writes, &mut barriers);
				}
				_ => unreachable!("A DX12 descriptor requirement names an image exactly when it holds a texture state."),
			}
		}
		if !requirements.entries.is_empty() {
			self.mark_command_buffer_work(command_buffer_handle);
		}
		barriers.submit(&command_list);
	}

	/// Normalizes a per-frame image to the frame copy that `sequence_index` selects; an image with one copy has one.
	fn descriptor_image_alias_key(&self, handle: crate::BaseImageHandle, sequence_index: u8) -> (crate::BaseImageHandle, u8) {
		let dynamic = self.images[handle.index() as usize].frame_resources.is_some();
		(handle, if dynamic { sequence_index } else { 0 })
	}

	/// Validates the bound sets against a pipeline, then rejects shader descriptors that name an image still attached
	/// in the active render pass.
	pub(crate) fn validate_descriptor_binding_contract(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
		attachments: &[crate::BaseImageHandle],
	) {
		let resolved = self.validate_descriptor_sets(pipeline_handle, sets, sequence_index);
		if attachments.is_empty() {
			return;
		}
		let layout = &self.pipeline_layouts[self.pipelines[pipeline_handle.index() as usize].layout.index() as usize];
		self.visit_used_descriptors(layout, &resolved, |_, retained| {
			let (WriteData::Image { handle, .. } | WriteData::CombinedImageSampler { image_handle: handle, .. }) =
				retained.descriptor
			else {
				return;
			};
			let resource_sequence = self.frame_index_with_offset(sequence_index, retained.frame_offset);
			let key = self.descriptor_image_alias_key(handle, resource_sequence);
			assert!(
				attachments
					.iter()
					.all(|&attachment| self.descriptor_image_alias_key(attachment, sequence_index) != key),
				"Incompatible DX12 attachment and descriptor aliases. The most likely cause is that one whole image was bound simultaneously as an attachment and shader resource. See https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#layout-access-compatibility."
			);
		});
	}

	/// Validates native allocation requirements that are stricter than retained descriptor kinds.
	pub(crate) fn validate_descriptor_resource(&self, shader_resource: ShaderResourceDescriptor, retained: RetainedDescriptor) {
		match retained.descriptor {
			WriteData::Buffer { handle } if shader_resource.kind() == ResourceKind::UniformBuffer => {
				let buffer = self.buffer(handle);
				assert!(
					buffer.uses.intersects(Uses::Uniform),
					"Invalid DX12 uniform-buffer descriptor. The most likely cause is that the buffer was not created with uniform usage."
				);
				assert!(
					buffer.size != 0 && buffer.size <= 64 * 1024,
					"Invalid DX12 constant-buffer view size. The most likely cause is that the requested range is empty or exceeds the 64-KiB shader-visible constant-buffer limit. See https://learn.microsoft.com/en-us/windows/win32/direct3d12/constants."
				);
			}
			WriteData::Buffer { handle, .. } if shader_resource.kind() == ResourceKind::StorageBuffer => {
				let buffer = self.buffer(handle);
				assert!(
					buffer.uses.intersects(Uses::Storage),
					"Invalid DX12 storage-buffer descriptor. The most likely cause is that the buffer was not created with storage usage."
				);
			}
			WriteData::Image {
				handle,
				mip_level,
				array_layer,
				..
			} => self.validate_image_descriptor_resource(shader_resource, handle, array_layer, mip_level),
			WriteData::CombinedImageSampler { image_handle, .. } => {
				self.validate_image_descriptor_resource(shader_resource, image_handle, None, None)
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
			.get(image_handle.index() as usize)
			.expect("Invalid DX12 image descriptor. The most likely cause is that the retained image handle is stale.");
		// A zero-sized image has no memory yet, so its descriptor is a null view that reads zeros. Passes bind such images
		// for sources they skip this frame, behind a flag the shader reads.

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

	/// Validates the bound sets against a pipeline: one set for each layout it reads, with every single resource its
	/// stages use written and fit for its native view. Returns the resolved sets; see [`Self::resolve_bound_sets`].
	///
	/// A write already checks its kind and array element against its set's layout, so only completeness and the
	/// native requirements of each resource remain. Conflicting uses of one resource panic when the binding transitions
	/// it; see [`Self::flush_pending_descriptor_texture_syncs`].
	pub(crate) fn validate_descriptor_sets(
		&self,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) -> SmallVec<[usize; 4]> {
		let resolved = self.resolve_bound_sets(pipeline_handle, sets, sequence_index);
		let pipeline = &self.pipelines[pipeline_handle.index() as usize];
		let layout = &self.pipeline_layouts[pipeline.layout.index() as usize];

		for (pipeline_set, &set_index) in layout.sets.iter().zip(&resolved) {
			let set = &self.descriptor_sets[set_index];
			for (resource_index, &resource) in set.layout.resources().iter().enumerate() {
				// An array may be populated in part; a single resource a stage uses must be written.
				if pipeline_set.used & (1 << resource_index) == 0 || resource.count() != 1 {
					continue;
				}
				assert!(
					set.descriptors[set.layout.element_base(resource_index) as usize].is_some(),
					"Missing descriptor at resource slot {} of layout `{}`. The most likely cause is that a resource the pipeline uses was not written before rendering. Write the resource into the set before binding it.",
					resource.slot().index(),
					set.layout.name(),
				);
			}
		}
		self.visit_used_descriptors(layout, &resolved, |resource, retained| {
			self.validate_descriptor_resource(resource, retained)
		});
		resolved
	}

	/// Gives the pipeline's stages the bound sets: writes the descriptors of each resolved frame copy that changed
	/// since it was last bound, then each stage's set bases into its root constants.
	pub(crate) fn bind_descriptor_sets_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
		sets: &[DescriptorSetHandle],
		sequence_index: u8,
	) {
		let command_list = self.descriptor_command_list_for_recording(command_buffer_handle);
		let resolved = self.resolve_bound_sets(pipeline_handle, sets, sequence_index);
		let command_buffer = &self.command_buffers[command_buffer_handle.index() as usize];
		let reader = SetReader {
			command_buffer: command_buffer_handle,
			frame: command_buffer.active_sequence_index,
			generation: command_buffer.generation,
		};
		for &set_index in &resolved {
			self.prepare_set_for_bind(set_index, reader);
		}

		let pipeline = &self.pipelines[pipeline_handle.index() as usize];
		let compute_root = matches!(pipeline.kind, PipelineKind::Compute | PipelineKind::RayTracing);
		let layout = &self.pipeline_layouts[pipeline.layout.index() as usize];
		for stage in &layout.stages {
			// Set `s` keeps its resource base at value `2s` and its sampler base after it; sets the stage doesn't read
			// and the padding of the last `uint4` stay zero.
			let mut bases = [0u32; 32];
			for (position, set) in stage.sets.iter().enumerate() {
				if let Some(set) = set {
					let set = &self.descriptor_sets[resolved[*set as usize]];
					bases[position * 2] = set.resource_base;
					bases[position * 2 + 1] = set.sampler_base;
				}
			}
			let count = Self::set_base_dwords(stage.sets.len());
			unsafe {
				if compute_root {
					command_list.SetComputeRoot32BitConstants(stage.root_parameter, count, bases.as_ptr().cast(), 0);
				} else {
					command_list.SetGraphicsRoot32BitConstants(stage.root_parameter, count, bases.as_ptr().cast(), 0);
				}
			}
		}
	}

	pub(crate) fn write_push_constants_native(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		pipeline_handle: PipelineHandle,
		offset: u32,
		bytes: &[u8],
	) {
		let command_list = self.command_list(command_buffer_handle);
		let pipeline = &self.pipelines[pipeline_handle.index() as usize];
		let pipeline_kind = pipeline.kind;
		let layout = &self.pipeline_layouts[pipeline.layout.index() as usize];
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
	}
}
