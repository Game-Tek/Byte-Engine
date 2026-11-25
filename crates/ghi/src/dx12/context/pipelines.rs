//! DX12 device operations for pipelines.

use super::*;

impl Device {
	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let pipeline = self.compile_raster_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	/// Compiles a raster pipeline from this context's shaders without adding it to the context, so a factory can
	/// compile it on its own thread and a rendering context intern it.
	pub(crate) fn compile_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> CompiledPipeline {
		let layout = self.pipeline_layout(builder.shaders, builder.push_constant_ranges);
		let pipeline_state = self.create_graphics_pipeline_state(&layout.root_signature, &builder);
		self.name_object(&pipeline_state, builder.name);
		let mut vertex_strides = SmallVec::<[u32; 4]>::new();
		for element in builder.vertex_elements {
			let slot = element.binding as usize;
			if vertex_strides.len() <= slot {
				vertex_strides.resize(slot + 1, 0);
			}
			vertex_strides[slot] += element.format.size() as u32;
		}
		CompiledPipeline {
			layout,
			kind: PipelineKind::Raster,
			native: NativePipeline::State(pipeline_state),
			has_mesh_shader: builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh)),
			vertex_strides,
		}
	}

	/// Adds a compiled pipeline to this context and returns the handle recordings bind it with.
	pub(crate) fn intern_pipeline(&mut self, pipeline: CompiledPipeline) -> PipelineHandle {
		let CompiledPipeline {
			layout,
			kind,
			native,
			has_mesh_shader,
			vertex_strides,
		} = pipeline;
		let layout = self.store_pipeline_layout(layout);
		self.pipelines.push(Pipeline {
			layout,
			kind,
			native,
			has_mesh_shader,
			vertex_strides,
		});
		PipelineHandle::from_index((self.pipelines.len() - 1) as u64)
	}

	pub(crate) fn create_graphics_pipeline_state(
		&mut self,
		root_signature: &ID3D12RootSignature,
		builder: &pipelines::raster::Builder,
	) -> ID3D12PipelineState {
		if builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh)) {
			return self.create_mesh_pipeline_state(root_signature, builder);
		}

		let vertex_shader = self.required_stage_dxil(builder.shaders, ShaderTypes::Vertex, "raster");
		let fragment_shader = self.required_stage_dxil(builder.shaders, ShaderTypes::Fragment, "raster");

		// Vertex attributes bind by position, as Vulkan locations and Metal attribute indices do, so element `n` feeds the
		// `TEXCOORDn` semantic that generated HLSL declares for input location `n`.
		let mut input_elements = Vec::with_capacity(builder.vertex_elements.len());
		let mut byte_offsets_by_slot = HashMap::<u32, u32>::default();
		for (index, element) in builder.vertex_elements.iter().enumerate() {
			let offset = byte_offsets_by_slot.entry(element.binding).or_insert(0);
			input_elements.push(D3D12_INPUT_ELEMENT_DESC {
				SemanticName: windows::core::s!("TEXCOORD"),
				SemanticIndex: index as u32,
				Format: Self::vertex_format(element.format).unwrap_or_else(|| {
					panic!("Failed to create a DX12 raster pipeline: vertex element {index} has a type DX12 can't read as a vertex attribute. The most likely cause is an 8- or 16-bit element type. Declare the element as a 32-bit type.")
				}),
				InputSlot: element.binding,
				AlignedByteOffset: *offset,
				InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
				InstanceDataStepRate: 0,
			});
			*offset += element.format.size() as u32;
		}

		let (blend, render_targets, depth_stencil_format) =
			Self::render_target_state(builder, D3D12_RENDER_TARGET_BLEND_DESC::default());

		let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
			VS: Self::shader_bytecode(vertex_shader),
			PS: Self::shader_bytecode(fragment_shader),
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
		root_signature: &ID3D12RootSignature,
		builder: &pipelines::raster::Builder,
	) -> ID3D12PipelineState {
		// Task and fragment stages are optional in a mesh pipeline; an absent stage passes empty bytecode.
		let optional_stage = |stage: ShaderTypes| {
			if builder
				.shaders
				.iter()
				.any(|shader| std::mem::discriminant(&shader.stage) == std::mem::discriminant(&stage))
			{
				self.required_stage_dxil(builder.shaders, stage, "mesh")
			} else {
				&[]
			}
		};
		let task_shader = optional_stage(ShaderTypes::Task);
		let fragment_shader = optional_stage(ShaderTypes::Fragment);
		let mesh_shader = self.required_stage_dxil(builder.shaders, ShaderTypes::Mesh, "mesh");

		let (blend, render_targets, depth_stencil_format) =
			Self::render_target_state(builder, Self::render_target_blend_desc(pipelines::raster::BlendMode::None));

		let mut stream = MeshPipelineStateStream {
			root_signature: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE,
				value: Some(root_signature.clone()),
			},
			amplification_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS,
				value: Self::shader_bytecode(task_shader),
			},
			mesh_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
				value: Self::shader_bytecode(mesh_shader),
			},
			pixel_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
				value: Self::shader_bytecode(fragment_shader),
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

	/// Returns a created graphics or mesh pipeline state, and panics with the device state when creation failed.
	fn finish_graphics_pipeline_state(
		&mut self,
		kind: &str,
		pipeline_state: windows::core::Result<ID3D12PipelineState>,
	) -> ID3D12PipelineState {
		match pipeline_state {
			Ok(pipeline_state) => pipeline_state,
			Err(error) => {
				let report = super::device_removed_report(&self.device);
				self.log_dx12_error(format!("Failed to create DX12 {kind} pipeline state: {error:?}. {report}"));
				panic!(
					"Failed to create a DX12 {kind} pipeline state: {error:?}. The most likely cause is a shader whose interface does not match the pipeline's root signature, vertex layout, or render target formats. Run with validation enabled; the debug layer logs the mismatch."
				);
			}
		}
	}

	/// Describes the builder's color-target blending and formats, plus its depth format, and panics for a format
	/// without a DXGI mapping.
	///
	/// Unused blend slots keep `unused_blend`, and color targets past the eighth are ignored.
	fn render_target_state(
		builder: &pipelines::raster::Builder,
		unused_blend: D3D12_RENDER_TARGET_BLEND_DESC,
	) -> (D3D12_BLEND_DESC, D3D12_RT_FORMAT_ARRAY, DXGI_FORMAT) {
		let attachment_format = |format: crate::Formats| {
			Self::dxgi_format(format).unwrap_or_else(|| {
				panic!(
					"Failed to create a DX12 raster pipeline: attachment format {format:?} has no DXGI format. The most likely cause is a format only another backend supports. Choose an attachment format that has a DXGI equivalent."
				)
			})
		};
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
				depth_stencil_format = attachment_format(attachment.format);
				continue;
			}
			let index = formats.NumRenderTargets as usize;
			if index >= formats.RTFormats.len() {
				break;
			}
			blend.RenderTarget[index] = Self::render_target_blend_desc(attachment.blend);
			formats.RTFormats[index] = attachment_format(attachment.format);
			formats.NumRenderTargets += 1;
		}
		blend.IndependentBlendEnable = BOOL((formats.NumRenderTargets > 1) as i32);
		(blend, formats, depth_stencil_format)
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
	pub(crate) fn shader_bytecode(dxil: &[u8]) -> D3D12_SHADER_BYTECODE {
		D3D12_SHADER_BYTECODE {
			pShaderBytecode: if dxil.is_empty() {
				std::ptr::null()
			} else {
				dxil.as_ptr().cast()
			},
			BytecodeLength: dxil.len(),
		}
	}

	/// Returns the DXIL of the `stage` shader in `shaders`, and panics when the pipeline has no such stage or the
	/// shader has no DXIL, since a pipeline without native state would record no work.
	fn required_stage_dxil(&self, shaders: &[pipelines::ShaderParameter], stage: ShaderTypes, pipeline_kind: &str) -> &[u8] {
		let parameter = shaders.iter().find(|parameter| std::mem::discriminant(&parameter.stage) == std::mem::discriminant(&stage)).unwrap_or_else(|| {
			panic!(
				"Failed to create a DX12 {pipeline_kind} pipeline: it has no {stage:?} shader. The most likely cause is a builder that omits a stage the pipeline requires. Add the {stage:?} shader to the pipeline builder."
			)
		});
		self.required_dxil(parameter, pipeline_kind)
	}

	/// Returns the DXIL `parameter` selects, and panics when its shader has none.
	fn required_dxil(&self, parameter: &pipelines::ShaderParameter, pipeline_kind: &str) -> &[u8] {
		self.shader_dxil(parameter).filter(|dxil| !dxil.is_empty()).unwrap_or_else(|| {
			panic!(
				"Failed to create a DX12 {pipeline_kind} pipeline: its {:?} shader has no DXIL. The most likely cause is a shader created from SPIR-V or MSL, or a stale shader handle. Create the shader from `shader::Sources::DXIL` or HLSL.",
				parameter.stage
			)
		})
	}

	/// Returns a shader's DXIL for `parameter`: the shader's own with an empty specialization map, or else the variant
	/// whose map equals the parameter's, in any order.
	///
	/// DX12 has no specialization constants, so every specialization the pipelines use is compiled ahead of time; see
	/// [`crate::shader::Sources::DXIL`].
	pub(crate) fn shader_dxil(&self, parameter: &pipelines::ShaderParameter) -> Option<&[u8]> {
		let shader = self.shaders.get(parameter.handle.index() as usize)?;
		let map = parameter.specialization_map;
		if map.is_empty() {
			return shader.dxil.as_deref();
		}
		let variant = shader
			.variants
			.iter()
			.find(|(variant, _)| variant.len() == map.len() && map.iter().all(|entry| variant.contains(entry)));
		let Some((_, dxil)) = variant else {
			panic!(
				"No DX12 variant of shader {:?} matches its pipeline's specialization constants {}. The most likely cause is a store baked before these material values existed, or by a bake that did not assemble the shader variants. Bake again with `beld bake --force`. See {}.",
				shader.name.as_deref().unwrap_or("unnamed"),
				specialization_map_text(map),
				::utils::online_docs_url("develop/rendering/shader-variants#specialize-shaders-on-dx12")
			);
		};
		Some(dxil)
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
		let pipeline = self.compile_compute_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	/// Compiles a compute pipeline without adding it to the context. See [`Self::compile_raster_pipeline`].
	pub(crate) fn compile_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> CompiledPipeline {
		let layout = self.pipeline_layout(std::slice::from_ref(&builder.shader), builder.push_constant_ranges);
		let pipeline_state = self.create_compute_pipeline_state(&layout.root_signature, builder.shader);
		self.name_object(&pipeline_state, builder.name);
		CompiledPipeline {
			layout,
			kind: PipelineKind::Compute,
			native: NativePipeline::State(pipeline_state),
			has_mesh_shader: false,
			vertex_strides: SmallVec::new(),
		}
	}

	pub(crate) fn create_compute_pipeline_state(
		&mut self,
		root_signature: &ID3D12RootSignature,
		shader_parameter: pipelines::ShaderParameter,
	) -> ID3D12PipelineState {
		let dxil = self.required_dxil(&shader_parameter, "compute");
		let mut desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
			CS: Self::shader_bytecode(dxil),
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		let pipeline_state = unsafe { self.device.CreateComputePipelineState::<ID3D12PipelineState>(&desc) };
		// Pipeline creation synchronously consumes the descriptor. Release the temporary root-signature clone afterward.
		unsafe { std::mem::ManuallyDrop::drop(&mut desc.pRootSignature) };
		pipeline_state.unwrap_or_else(|error| {
			let report = super::device_removed_report(&self.device);
			self.log_dx12_error(format!("Failed to create DX12 compute pipeline state: {error:?}. {report}"));
			panic!(
				"Failed to create a DX12 compute pipeline state: {error:?}. The most likely cause is a shader whose resources do not match the pipeline's root signature. Run with validation enabled; the debug layer logs the mismatch."
			);
		})
	}
}

/// Writes a specialization map as `id: type = bytes` items, for errors that name one.
fn specialization_map_text(map: &[pipelines::SpecializationMapEntry]) -> String {
	let items = map
		.iter()
		.map(|entry| format!("{}: {} = {:02x?}", entry.constant_id, entry.r#type, entry.value))
		.collect::<Vec<_>>();
	format!("[{}]", items.join(", "))
}
