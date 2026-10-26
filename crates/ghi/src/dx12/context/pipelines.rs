//! DX12 device operations for pipelines.

use super::*;

impl Device {
	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.shaders, builder.push_constant_ranges);
		let pipeline_state = self.create_graphics_pipeline_state(layout, &builder);
		let shaders = builder.shaders.iter().map(|s| *s.handle).collect();
		let has_mesh_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh));
		self.pipelines.push(Pipeline {
			layout,
			shaders,
			kind: PipelineKind::Raster,
			pipeline_state,
			has_mesh_shader,
		});

		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub(crate) fn create_graphics_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		builder: &pipelines::raster::Builder,
	) -> Option<ID3D12PipelineState> {
		if builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh)) {
			return self.create_mesh_pipeline_state(layout, builder);
		}

		let root_signature = self.root_signature(layout)?;
		let vertex_shader = self.shader_dxil_for_stage(builder.shaders, ShaderTypes::Vertex)?;
		let fragment_shader = self.shader_dxil_for_stage(builder.shaders, ShaderTypes::Fragment)?;
		if vertex_shader.is_empty() || fragment_shader.is_empty() {
			return None;
		}

		let semantic_names = builder
			.vertex_elements
			.iter()
			.map(|element| std::ffi::CString::new(element.name).ok())
			.collect::<Option<Vec<_>>>()?;
		let mut input_elements = Vec::with_capacity(builder.vertex_elements.len());
		let mut byte_offsets_by_slot = HashMap::<u32, u32>::default();
		for (index, element) in builder.vertex_elements.iter().enumerate() {
			let offset = byte_offsets_by_slot.entry(element.binding).or_insert(0);
			input_elements.push(D3D12_INPUT_ELEMENT_DESC {
				SemanticName: PCSTR(semantic_names[index].as_ptr().cast()),
				SemanticIndex: 0,
				Format: Self::vertex_format(element.format)?,
				InputSlot: element.binding,
				AlignedByteOffset: *offset,
				InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
				InstanceDataStepRate: 0,
			});
			*offset += element.format.size() as u32;
		}

		let (blend, render_targets, depth_stencil_format) =
			Self::render_target_state(builder, D3D12_RENDER_TARGET_BLEND_DESC::default())?;

		self.counters.graphics_pipeline_state_create_attempt_count += 1;
		let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			VS: Self::shader_bytecode(&vertex_shader),
			PS: Self::shader_bytecode(&fragment_shader),
			DS: D3D12_SHADER_BYTECODE::default(),
			HS: D3D12_SHADER_BYTECODE::default(),
			GS: D3D12_SHADER_BYTECODE::default(),
			StreamOutput: Default::default(),
			BlendState: blend,
			SampleMask: u32::MAX,
			RasterizerState: Self::rasterizer_desc(builder),
			DepthStencilState: Self::depth_stencil_desc(depth_stencil_format, builder.depth_write),
			InputLayout: D3D12_INPUT_LAYOUT_DESC {
				pInputElementDescs: if input_elements.is_empty() {
					std::ptr::null()
				} else {
					input_elements.as_ptr()
				},
				NumElements: input_elements.len() as u32,
			},
			IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
			PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
			NumRenderTargets: render_targets.NumRenderTargets,
			RTVFormats: render_targets.RTFormats,
			DSVFormat: depth_stencil_format,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		let pipeline_state = unsafe { self.device.CreateGraphicsPipelineState::<ID3D12PipelineState>(&desc) };
		// Pipeline creation synchronously consumes the descriptor. Release the temporary root-signature clone afterward.
		unsafe { std::mem::ManuallyDrop::drop(&mut desc.pRootSignature) };
		self.finish_graphics_pipeline_state("graphics", pipeline_state)
	}

	pub(crate) fn create_mesh_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		builder: &pipelines::raster::Builder,
	) -> Option<ID3D12PipelineState> {
		let root_signature = self.root_signature(layout)?;
		let has_task_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Task));
		let task_shader = if has_task_shader {
			self.shader_dxil_for_stage(builder.shaders, ShaderTypes::Task)?
		} else {
			Vec::new()
		};
		let mesh_shader = self.shader_dxil_for_stage(builder.shaders, ShaderTypes::Mesh)?;
		let has_fragment_shader = builder
			.shaders
			.iter()
			.any(|shader| matches!(shader.stage, ShaderTypes::Fragment));
		let fragment_shader = if has_fragment_shader {
			self.shader_dxil_for_stage(builder.shaders, ShaderTypes::Fragment)?
		} else {
			Vec::new()
		};
		if (has_task_shader && task_shader.is_empty())
			|| mesh_shader.is_empty()
			|| (has_fragment_shader && fragment_shader.is_empty())
		{
			return None;
		}

		let (blend, render_targets, depth_stencil_format) =
			Self::render_target_state(builder, Self::render_target_blend_desc(pipelines::raster::BlendMode::None))?;

		self.counters.graphics_pipeline_state_create_attempt_count += 1;
		let mut stream = MeshPipelineStateStream {
			root_signature: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE,
				value: Some(root_signature),
			},
			amplification_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS,
				value: Self::shader_bytecode(&task_shader),
			},
			mesh_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
				value: Self::shader_bytecode(&mesh_shader),
			},
			pixel_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
				value: Self::shader_bytecode(&fragment_shader),
			},
			blend: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
				value: blend,
			},
			sample_mask: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK,
				value: u32::MAX,
			},
			rasterizer: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER,
				value: Self::rasterizer_desc(builder),
			},
			depth_stencil: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL,
				value: Self::depth_stencil_desc(depth_stencil_format, builder.depth_write),
			},
			depth_stencil_format: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
				value: depth_stencil_format,
			},
			render_targets: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
				value: render_targets,
			},
			sample_desc: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
				value: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			},
			node_mask: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK,
				value: 0,
			},
			flags: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS,
				value: D3D12_PIPELINE_STATE_FLAG_NONE,
			},
		};
		let desc = D3D12_PIPELINE_STATE_STREAM_DESC {
			SizeInBytes: std::mem::size_of::<MeshPipelineStateStream>(),
			pPipelineStateSubobjectStream: (&mut stream as *mut MeshPipelineStateStream).cast(),
		};
		let pipeline_state = unsafe { self.device.CreatePipelineState::<ID3D12PipelineState>(&desc) };
		self.finish_graphics_pipeline_state("mesh", pipeline_state)
	}

	/// Records the outcome of a graphics or mesh pipeline-state creation and logs a failure with the device state.
	fn finish_graphics_pipeline_state(
		&mut self,
		kind: &str,
		pipeline_state: windows::core::Result<ID3D12PipelineState>,
	) -> Option<ID3D12PipelineState> {
		match pipeline_state {
			Ok(pipeline_state) => {
				self.counters.graphics_pipeline_state_last_error = None;
				Some(pipeline_state)
			}
			Err(error) => {
				self.counters.graphics_pipeline_state_last_error = Some(error.code().0);
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 {kind} pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				));
				None
			}
		}
	}

	/// Returns the root signature of a pipeline layout, or `None` for an unknown layout handle.
	fn root_signature(&self, layout: PipelineLayoutHandle) -> Option<ID3D12RootSignature> {
		self.pipeline_layouts
			.get(layout.0 as usize)
			.map(|layout| layout.root_signature.clone())
	}

	/// Describes the builder's color-target blending and formats, plus its depth format, or `None` for a format
	/// without a DXGI mapping.
	///
	/// Unused blend slots keep `unused_blend`, and color targets past the eighth are ignored.
	fn render_target_state(
		builder: &pipelines::raster::Builder,
		unused_blend: D3D12_RENDER_TARGET_BLEND_DESC,
	) -> Option<(D3D12_BLEND_DESC, D3D12_RT_FORMAT_ARRAY, DXGI_FORMAT)> {
		let mut blend = D3D12_BLEND_DESC {
			AlphaToCoverageEnable: BOOL(0),
			IndependentBlendEnable: BOOL(0),
			RenderTarget: [unused_blend; 8],
		};
		let mut formats = D3D12_RT_FORMAT_ARRAY {
			RTFormats: [DXGI_FORMAT_UNKNOWN; 8],
			NumRenderTargets: 0,
		};
		let mut depth_stencil_format = DXGI_FORMAT_UNKNOWN;
		for attachment in builder.render_targets.iter() {
			if attachment.format.is_depth() {
				depth_stencil_format = Self::dxgi_format(attachment.format)?;
				continue;
			}
			let index = formats.NumRenderTargets as usize;
			if index >= formats.RTFormats.len() {
				break;
			}
			blend.RenderTarget[index] = Self::render_target_blend_desc(attachment.blend);
			formats.RTFormats[index] = Self::dxgi_format(attachment.format)?;
			formats.NumRenderTargets += 1;
		}
		blend.IndependentBlendEnable = BOOL((formats.NumRenderTargets > 1) as i32);
		Some((blend, formats, depth_stencil_format))
	}

	/// Describes the fixed-function rasterizer state that a raster pipeline builder selects.
	fn rasterizer_desc(builder: &pipelines::raster::Builder) -> D3D12_RASTERIZER_DESC {
		D3D12_RASTERIZER_DESC {
			FillMode: Self::fill_mode(builder.fill_mode),
			CullMode: Self::cull_mode(builder.cull_mode),
			FrontCounterClockwise: match builder.face_winding {
				pipelines::raster::FaceWinding::Clockwise => BOOL(0),
				pipelines::raster::FaceWinding::CounterClockwise => BOOL(1),
			},
			DepthBias: 0,
			DepthBiasClamp: 0.0,
			SlopeScaledDepthBias: 0.0,
			DepthClipEnable: BOOL(1),
			MultisampleEnable: BOOL(0),
			AntialiasedLineEnable: BOOL(0),
			ForcedSampleCount: 0,
			ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
		}
	}

	/// Describes depth testing against the pipeline's depth attachment, if any, with stencil disabled.
	fn depth_stencil_desc(depth_stencil_format: DXGI_FORMAT, depth_write: bool) -> D3D12_DEPTH_STENCIL_DESC {
		let has_depth_attachment = depth_stencil_format != DXGI_FORMAT_UNKNOWN;
		let disabled_stencil = D3D12_DEPTH_STENCILOP_DESC {
			StencilFailOp: D3D12_STENCIL_OP_KEEP,
			StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
			StencilPassOp: D3D12_STENCIL_OP_KEEP,
			StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
		};
		D3D12_DEPTH_STENCIL_DESC {
			DepthEnable: BOOL(has_depth_attachment as i32),
			DepthWriteMask: if has_depth_attachment && depth_write {
				D3D12_DEPTH_WRITE_MASK_ALL
			} else {
				D3D12_DEPTH_WRITE_MASK_ZERO
			},
			DepthFunc: if has_depth_attachment {
				D3D12_COMPARISON_FUNC_GREATER_EQUAL
			} else {
				D3D12_COMPARISON_FUNC_ALWAYS
			},
			StencilEnable: BOOL(0),
			StencilReadMask: 0xff,
			StencilWriteMask: 0xff,
			FrontFace: disabled_stencil,
			BackFace: disabled_stencil,
		}
	}

	/// Describes DXIL bytes for a pipeline description, passing a null pointer for an absent stage.
	fn shader_bytecode(dxil: &[u8]) -> D3D12_SHADER_BYTECODE {
		D3D12_SHADER_BYTECODE {
			pShaderBytecode: if dxil.is_empty() {
				std::ptr::null()
			} else {
				dxil.as_ptr().cast()
			},
			BytecodeLength: dxil.len(),
		}
	}

	pub(crate) fn shader_dxil_for_stage(
		&mut self,
		shaders: &[pipelines::ShaderParameter],
		stage: ShaderTypes,
	) -> Option<Vec<u8>> {
		let parameter = shaders.iter().find(|parameter| {
			matches!(
				(parameter.stage, stage),
				(ShaderTypes::Vertex, ShaderTypes::Vertex)
					| (ShaderTypes::Fragment, ShaderTypes::Fragment)
					| (ShaderTypes::Task, ShaderTypes::Task)
					| (ShaderTypes::Mesh, ShaderTypes::Mesh)
			)
		})?;
		self.shader_dxil(parameter)
	}

	/// Returns a shader's DXIL, recompiling its HLSL when the parameter carries specialization constants.
	fn shader_dxil(&mut self, parameter: &pipelines::ShaderParameter) -> Option<Vec<u8>> {
		let shader = self.shaders.get(parameter.handle.0 as usize)?;
		if !parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						parameter.stage,
						parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() {
					self.counters.hlsl_specialization_compile_count += 1;
				}
				return dxil;
			}
		}
		shader.dxil.clone()
	}

	pub(crate) fn vertex_format(data_type: DataTypes) -> Option<DXGI_FORMAT> {
		match data_type {
			DataTypes::Float => Some(DXGI_FORMAT_R32_FLOAT),
			DataTypes::Float2 => Some(DXGI_FORMAT_R32G32_FLOAT),
			DataTypes::Float3 => Some(DXGI_FORMAT_R32G32B32_FLOAT),
			DataTypes::Float4 => Some(DXGI_FORMAT_R32G32B32A32_FLOAT),
			DataTypes::Int => Some(DXGI_FORMAT_R32_SINT),
			DataTypes::Int2 => Some(DXGI_FORMAT_R32G32_SINT),
			DataTypes::Int3 => Some(DXGI_FORMAT_R32G32B32_SINT),
			DataTypes::Int4 => Some(DXGI_FORMAT_R32G32B32A32_SINT),
			DataTypes::UInt | DataTypes::U32 => Some(DXGI_FORMAT_R32_UINT),
			DataTypes::UInt2 => Some(DXGI_FORMAT_R32G32_UINT),
			DataTypes::UInt3 => Some(DXGI_FORMAT_R32G32B32_UINT),
			DataTypes::UInt4 => Some(DXGI_FORMAT_R32G32B32A32_UINT),
			DataTypes::U8 | DataTypes::U16 => None,
		}
	}

	pub(crate) fn cull_mode(cull_mode: pipelines::raster::CullMode) -> windows::Win32::Graphics::Direct3D12::D3D12_CULL_MODE {
		match cull_mode {
			pipelines::raster::CullMode::None => D3D12_CULL_MODE_NONE,
			pipelines::raster::CullMode::Front => D3D12_CULL_MODE_FRONT,
			pipelines::raster::CullMode::Back => D3D12_CULL_MODE_BACK,
		}
	}

	/// Maps portable triangle fill behavior to its DX12 rasterizer state.
	pub(crate) fn fill_mode(fill_mode: pipelines::raster::FillMode) -> D3D12_FILL_MODE {
		match fill_mode {
			pipelines::raster::FillMode::Solid => D3D12_FILL_MODE_SOLID,
			pipelines::raster::FillMode::Wireframe => D3D12_FILL_MODE_WIREFRAME,
		}
	}

	pub(crate) fn render_target_blend_desc(blend: pipelines::raster::BlendMode) -> D3D12_RENDER_TARGET_BLEND_DESC {
		let (blend_enable, source, destination) = match blend {
			pipelines::raster::BlendMode::None => (false, D3D12_BLEND_ONE, D3D12_BLEND_ZERO),
			pipelines::raster::BlendMode::Alpha => (true, D3D12_BLEND_SRC_ALPHA, D3D12_BLEND_INV_SRC_ALPHA),
			pipelines::raster::BlendMode::Premultiplied => (true, D3D12_BLEND_ONE, D3D12_BLEND_INV_SRC_ALPHA),
		};
		D3D12_RENDER_TARGET_BLEND_DESC {
			BlendEnable: BOOL(blend_enable as i32),
			LogicOpEnable: BOOL(0),
			SrcBlend: source,
			DestBlend: destination,
			BlendOp: D3D12_BLEND_OP_ADD,
			SrcBlendAlpha: D3D12_BLEND_ONE,
			DestBlendAlpha: destination,
			BlendOpAlpha: D3D12_BLEND_OP_ADD,
			LogicOp: D3D12_LOGIC_OP_NOOP,
			RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
		}
	}

	pub fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(std::slice::from_ref(&builder.shader), builder.push_constant_ranges);
		let shader_parameter = builder.shader;
		let pipeline_state = self.create_compute_pipeline_state(layout, shader_parameter);
		self.pipelines.push(Pipeline {
			layout,
			shaders: vec![*shader_parameter.handle],
			kind: PipelineKind::Compute,
			pipeline_state,
			has_mesh_shader: false,
		});
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub(crate) fn create_compute_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		shader_parameter: pipelines::ShaderParameter,
	) -> Option<ID3D12PipelineState> {
		let root_signature = self.root_signature(layout)?;
		let dxil = self.shader_dxil(&shader_parameter)?;
		if dxil.is_empty() {
			return None;
		}
		self.counters.compute_pipeline_state_create_attempt_count += 1;
		let mut desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			CS: Self::shader_bytecode(&dxil),
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		let pipeline_state = unsafe { self.device.CreateComputePipelineState::<ID3D12PipelineState>(&desc) };
		// Pipeline creation synchronously consumes the descriptor. Release the temporary root-signature clone afterward.
		unsafe { std::mem::ManuallyDrop::drop(&mut desc.pRootSignature) };
		if let Err(error) = &pipeline_state {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to create DX12 compute pipeline state: {error:?}; device removed reason: {removed_reason:?}"
			));
		}
		pipeline_state.ok()
	}
}
