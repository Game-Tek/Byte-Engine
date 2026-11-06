use super::super::*;

/// The most sets one stage can read. Their bases fill at most eight `uint4` root-constant vectors at `b1, space0`.
const MAX_STAGE_SETS: usize = crate::shader::MAX_DESCRIPTOR_SETS;

/// The most 32-bit values a D3D12 root signature holds.
const MAX_ROOT_SIGNATURE_DWORDS: u32 = 64;

impl Device {
	/// Returns the visibility of a stage's root parameters. A compute shader is the pipeline's only stage, so its
	/// parameters are visible to all.
	fn stage_visibility(stage: ShaderTypes) -> D3D12_SHADER_VISIBILITY {
		match stage {
			ShaderTypes::Vertex => D3D12_SHADER_VISIBILITY_VERTEX,
			ShaderTypes::Fragment => D3D12_SHADER_VISIBILITY_PIXEL,
			ShaderTypes::Task => D3D12_SHADER_VISIBILITY_AMPLIFICATION,
			ShaderTypes::Mesh => D3D12_SHADER_VISIBILITY_MESH,
			// Every shader of a DXR pipeline reads the one global root signature.
			ShaderTypes::Compute | ShaderTypes::RayGen | ShaderTypes::ClosestHit => D3D12_SHADER_VISIBILITY_ALL,
			ShaderTypes::Tile => crate::pipelines::tile::unsupported("DX12"),
		}
	}

	/// Builds a pipeline's layout from the sets its stages declare: one root-constant parameter per stage that reads a
	/// set, holding a resource and a sampler base for each of its set positions, after the push constants.
	///
	/// Stages that declare the same layout share its set. A stage's shader reads the bases of the set at its own
	/// position `s` from `_besl_sets[s >> 1][(s & 1) * 2]` and the component after it, so no stage needs remapping.
	pub(crate) fn pipeline_layout(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		push_constant_ranges: &[PushConstantRange],
	) -> NativePipelineLayout {
		let push_constant_dwords = Self::push_constant_dwords(push_constant_ranges);
		let mut root_parameter = u32::from(push_constant_dwords != 0);

		let mut sets = SmallVec::<[PipelineSet; 4]>::new();
		let mut stages = SmallVec::<[StageSets; 3]>::new();
		let mut key = RootSignatureKey {
			push_constant_dwords,
			stages: SmallVec::new(),
		};
		for parameter in shaders {
			let shader = &self.shaders[parameter.handle.index() as usize];
			// The stage's bases end at the last set it reads; the sets after it take no root constants.
			let Some(set_count) = shader
				.descriptor_sets
				.iter()
				.rposition(|set| set.used() != 0)
				.map(|position| position + 1)
			else {
				continue;
			};
			assert!(
				set_count <= MAX_STAGE_SETS,
				"Too many descriptor sets in a DX12 {:?} shader. The most likely cause is that the shader reads a set at position {} where the root constants hold {MAX_STAGE_SETS}. Merge layouts so the shader reads at most {MAX_STAGE_SETS} sets.",
				shader.stage,
				set_count - 1,
			);

			let mut stage_sets = SmallVec::<[Option<u8>; 8]>::with_capacity(set_count);
			for shader_set in &shader.descriptor_sets[..set_count] {
				if shader_set.used() == 0 {
					stage_sets.push(None);
					continue;
				}
				let identity = shader_set.layout().identity();
				let index = match sets.iter().position(|set| set.layout.identity() == identity) {
					Some(index) => {
						sets[index].used |= shader_set.used();
						index
					}
					None => {
						sets.push(PipelineSet {
							layout: shader_set.layout().clone(),
							used: shader_set.used(),
						});
						sets.len() - 1
					}
				};
				stage_sets.push(Some(index as u8));
			}

			let dwords = Self::set_base_dwords(set_count);
			key.stages.push((Self::stage_visibility(shader.stage).0, dwords));
			stages.push(StageSets {
				root_parameter,
				sets: stage_sets,
			});
			root_parameter += 1;
		}

		self.native_pipeline_layout(key, sets, stages, push_constant_ranges)
	}

	/// Builds a ray tracing pipeline's layout: one root-constant parameter, which every shader of the pipeline reads,
	/// with the bases of every set the ray generation shader declares, after the push constants.
	///
	/// A closest hit reads the global root signature too, and may read a set the ray generation shader doesn't, so the
	/// parameter holds every declared set, used or not. A closest hit's sets are the first of the ray generation
	/// shader's, in the same order, so it reads each set's bases at the position the ray generation shader does.
	pub(crate) fn ray_tracing_pipeline_layout(
		&mut self,
		ray_generation: &pipelines::ShaderParameter,
		hit_groups: &[pipelines::ShaderParameter],
		push_constant_ranges: &[PushConstantRange],
	) -> NativePipelineLayout {
		let push_constant_dwords = Self::push_constant_dwords(push_constant_ranges);
		let declared = &self.shaders[ray_generation.handle.index() as usize].descriptor_sets;
		assert!(
			declared.len() <= MAX_STAGE_SETS,
			"Too many descriptor sets in a DX12 ray generation shader. The most likely cause is that the shader declares {} sets where the root constants hold {MAX_STAGE_SETS}. Merge layouts so the shader declares at most {MAX_STAGE_SETS} sets.",
			declared.len(),
		);
		let mut sets = declared
			.iter()
			.map(|set| PipelineSet {
				layout: set.layout().clone(),
				used: set.used(),
			})
			.collect::<SmallVec<[PipelineSet; 4]>>();
		for hit_group in hit_groups {
			let hit_sets = &self.shaders[hit_group.handle.index() as usize].descriptor_sets;
			assert!(
				hit_sets.len() <= sets.len()
					&& hit_sets
						.iter()
						.zip(&sets)
						.all(|(hit_set, set)| hit_set.layout().identity() == set.layout.identity()),
				"DX12 ray tracing pipeline creation failed because a closest-hit shader's descriptor sets aren't the first of the ray generation shader's. The most likely cause is a closest hit that declares a set the ray generation shader doesn't, or declares the sets in another order. Declare the ray generation shader's sets first, in the same order, in every closest hit."
			);
			for (set, hit_set) in sets.iter_mut().zip(hit_sets) {
				set.used |= hit_set.used();
			}
		}

		let mut key = RootSignatureKey {
			push_constant_dwords,
			stages: SmallVec::new(),
		};
		let mut stages = SmallVec::<[StageSets; 3]>::new();
		if !sets.is_empty() {
			key.stages
				.push((D3D12_SHADER_VISIBILITY_ALL.0, Self::set_base_dwords(sets.len())));
			stages.push(StageSets {
				root_parameter: u32::from(push_constant_dwords != 0),
				sets: (0..sets.len() as u8).map(Some).collect(),
			});
		}
		self.native_pipeline_layout(key, sets, stages, push_constant_ranges)
	}

	/// Returns how many root constants hold push constants of `push_constant_ranges`.
	fn push_constant_dwords(push_constant_ranges: &[PushConstantRange]) -> u32 {
		push_constant_ranges
			.iter()
			.map(|range| range.offset.saturating_add(range.size))
			.max()
			.unwrap_or(0)
			.div_ceil(4)
	}

	/// Returns a layout of `sets`, read by `stages`, with the root signature of shape `key`, which layouts of the same
	/// shape share.
	fn native_pipeline_layout(
		&mut self,
		key: RootSignatureKey,
		sets: SmallVec<[PipelineSet; 4]>,
		stages: SmallVec<[StageSets; 3]>,
		push_constant_ranges: &[PushConstantRange],
	) -> NativePipelineLayout {
		let push_constant_dwords = key.push_constant_dwords;
		let root_signature = match self.root_signatures.get(&key) {
			Some(root_signature) => root_signature.clone(),
			None => {
				let root_signature = self.create_root_signature(&key).expect(
					"Failed to create a DX12 root signature. The most likely cause is an incompatible Agility SDK runtime or a removed device. Check the logged serialization diagnostics.",
				);
				self.root_signatures.insert(key, root_signature.clone());
				root_signature
			}
		};

		NativePipelineLayout {
			root_signature,
			sets,
			stages,
			push_constant_root: (push_constant_dwords != 0).then_some(0),
			push_constant_ranges: push_constant_ranges.into(),
		}
	}

	/// Adds `layout` to the context, so pipelines and descriptor bindings can name it by handle.
	pub(crate) fn store_pipeline_layout(&mut self, layout: NativePipelineLayout) -> PipelineLayoutHandle {
		self.pipeline_layouts.push(layout);
		PipelineLayoutHandle::from_index((self.pipeline_layouts.len() - 1) as u64)
	}

	/// Returns how many root constants hold the bases of `set_count` sets: two per set, in whole `uint4` vectors, as
	/// the shader declares them.
	pub(crate) fn set_base_dwords(set_count: usize) -> u32 {
		(set_count.div_ceil(2) * 4) as u32
	}

	/// Serializes and creates a root signature of the shape `key` describes: push constants at `b0`, then each stage's
	/// set bases at `b1`, and shader access to both heaps by index instead of through tables.
	fn create_root_signature(&self, key: &RootSignatureKey) -> Option<ID3D12RootSignature> {
		let total_dwords = key.push_constant_dwords + key.stages.iter().map(|(_, dwords)| dwords).sum::<u32>();
		assert!(
			total_dwords <= MAX_ROOT_SIGNATURE_DWORDS,
			"DX12 root signature exceeds {MAX_ROOT_SIGNATURE_DWORDS} DWORDs ({total_dwords}). The most likely cause is that the push constants and the set bases of every stage don't fit together. Shrink the push constants or have the stages read fewer sets."
		);

		let root_constants = |register: u32, values: u32, visibility: D3D12_SHADER_VISIBILITY| D3D12_ROOT_PARAMETER1 {
			ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
			Anonymous: D3D12_ROOT_PARAMETER1_0 {
				Constants: D3D12_ROOT_CONSTANTS {
					ShaderRegister: register,
					RegisterSpace: 0,
					Num32BitValues: values,
				},
			},
			ShaderVisibility: visibility,
		};
		let mut parameters = SmallVec::<[D3D12_ROOT_PARAMETER1; 4]>::new();
		if key.push_constant_dwords != 0 {
			parameters.push(root_constants(0, key.push_constant_dwords, D3D12_SHADER_VISIBILITY_ALL));
		}
		for &(visibility, dwords) in &key.stages {
			parameters.push(root_constants(1, dwords, D3D12_SHADER_VISIBILITY(visibility)));
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
			Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
				| D3D12_ROOT_SIGNATURE_FLAG_CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED
				| D3D12_ROOT_SIGNATURE_FLAG_SAMPLER_HEAP_DIRECTLY_INDEXED,
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
				match Self::d3d_blob_bytes(&error_blob) {
					Ok(message) => String::from_utf8_lossy(message).into_owned(),
					Err(reason) => reason.to_string(),
				}
			} else {
				"DX12 returned no serialization diagnostics.".to_string()
			};
			self.log_dx12_error(format!(
				"Failed to serialize DX12 root signature. The most likely cause is an invalid root parameter or an incompatible Agility SDK runtime. Update the Agility SDK to one with root signature 1.2. Error: {error:?}. {details}"
			));
			return None;
		}
		let Some(blob) = blob else {
			self.log_dx12_error(
				"DX12 root-signature serialization returned no data. The most likely cause is an incompatible Agility SDK runtime. Update the Agility SDK to one with root signature 1.2.",
			);
			return None;
		};
		let bytes = match Self::d3d_blob_bytes(&blob) {
			Ok([]) => {
				self.log_dx12_error(
					"DX12 root-signature serialization returned empty data. The most likely cause is an incompatible Agility SDK runtime. Update the Agility SDK to one with root signature 1.2.",
				);
				return None;
			}
			Ok(bytes) => bytes,
			Err(reason) => {
				self.log_dx12_error(reason);
				return None;
			}
		};
		match unsafe { self.device.CreateRootSignature(0, bytes) } {
			Ok(root_signature) => Some(root_signature),
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 root signature. The most likely cause is an invalid serialized layout or a removed device. Check the device removed reason. Parameters: {}, error: {error:?}, device removed reason: {removed_reason:?}",
					parameters.len(),
				));
				None
			}
		}
	}

	/// Borrows the bytes reported by a live D3D blob without constructing a slice from a null pointer.
	fn d3d_blob_bytes(blob: &windows::Win32::Graphics::Direct3D::ID3DBlob) -> Result<&[u8], &'static str> {
		let byte_len = unsafe { blob.GetBufferSize() };
		if byte_len == 0 {
			return Ok(&[]);
		}
		let bytes = unsafe { blob.GetBufferPointer() }.cast::<u8>();
		if bytes.is_null() {
			return Err(
				"DX12 returned a null pointer for nonempty serialized root-signature data. The most likely cause is an invalid runtime response. Restart the application, and update the graphics driver if it repeats.",
			);
		}
		// ID3DBlob owns byte_len initialized bytes at this pointer and remains borrowed for the returned slice lifetime.
		Ok(unsafe { std::slice::from_raw_parts(bytes, byte_len) })
	}
}
