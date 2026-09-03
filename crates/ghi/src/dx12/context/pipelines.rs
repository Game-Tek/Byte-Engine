//! DX12 device operations for pipelines.

use super::*;

impl Device {
	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let pipeline_state = self.create_graphics_pipeline_state(layout, &builder);
		let shaders = builder.shaders.iter().map(|s| *s.handle).collect();
		let has_mesh_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Mesh));
		self.pipelines.push(Pipeline {
			layout,
			shaders,
			kind: PipelineKind::Raster,
			pipeline_state,
			ray_tracing_state_object: None,
			ray_tracing_shader_identifiers: HashMap::default(),
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

		let root_signature = self
			.pipeline_layouts
			.get(layout.0 as usize)
			.map(|layout| layout.root_signature.clone())?;
		let vertex_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Vertex)?;
		let fragment_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Fragment)?;
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

		let mut render_targets = [D3D12_RENDER_TARGET_BLEND_DESC::default(); 8];
		let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
		let mut render_target_count = 0usize;
		let mut depth_stencil_format = DXGI_FORMAT_UNKNOWN;
		for attachment in builder.render_targets.iter() {
			if attachment.format.is_depth() {
				depth_stencil_format = Self::dxgi_format(attachment.format)?;
				continue;
			}
			if render_target_count >= rtv_formats.len() {
				break;
			}
			render_targets[render_target_count] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[render_target_count] = Self::dxgi_format(attachment.format)?;
			render_target_count += 1;
		}
		let has_depth_attachment = depth_stencil_format != DXGI_FORMAT_UNKNOWN;

		self.graphics_pipeline_state_create_attempt_count += 1;
		let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			VS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: vertex_shader.as_ptr().cast(),
				BytecodeLength: vertex_shader.len(),
			},
			PS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: fragment_shader.as_ptr().cast(),
				BytecodeLength: fragment_shader.len(),
			},
			DS: D3D12_SHADER_BYTECODE::default(),
			HS: D3D12_SHADER_BYTECODE::default(),
			GS: D3D12_SHADER_BYTECODE::default(),
			StreamOutput: Default::default(),
			BlendState: D3D12_BLEND_DESC {
				AlphaToCoverageEnable: BOOL(0),
				IndependentBlendEnable: BOOL((render_target_count > 1) as i32),
				RenderTarget: render_targets,
			},
			SampleMask: u32::MAX,
			RasterizerState: D3D12_RASTERIZER_DESC {
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
			},
			DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
				DepthEnable: BOOL(has_depth_attachment as i32),
				DepthWriteMask: if has_depth_attachment && builder.depth_write {
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
				FrontFace: Self::disabled_stencil_op_desc(),
				BackFace: Self::disabled_stencil_op_desc(),
			},
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
			NumRenderTargets: render_target_count as u32,
			RTVFormats: rtv_formats,
			DSVFormat: depth_stencil_format,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			NodeMask: 0,
			CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
			Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
		};

		let pipeline_state = unsafe { self.device.CreateGraphicsPipelineState::<ID3D12PipelineState>(&desc) };
		// Pipeline creation synchronously consumes the descriptor. Release the temporary root-signature clone afterward.
		unsafe { std::mem::ManuallyDrop::drop(&mut desc.pRootSignature) };
		match pipeline_state {
			Ok(pipeline_state) => {
				self.graphics_pipeline_state_last_error = None;
				Some(pipeline_state)
			}
			Err(error) => {
				self.graphics_pipeline_state_last_error = Some(error.code().0);
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 graphics pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				));
				None
			}
		}
	}

	pub(crate) fn create_mesh_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		builder: &pipelines::raster::Builder,
	) -> Option<ID3D12PipelineState> {
		let root_signature = self
			.pipeline_layouts
			.get(layout.0 as usize)
			.map(|layout| layout.root_signature.clone())?;
		let has_task_shader = builder.shaders.iter().any(|shader| matches!(shader.stage, ShaderTypes::Task));
		let task_shader = if has_task_shader {
			self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Task)?
		} else {
			Vec::new()
		};
		let mesh_shader = self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Mesh)?;
		let has_fragment_shader = builder
			.shaders
			.iter()
			.any(|shader| matches!(shader.stage, ShaderTypes::Fragment));
		let fragment_shader = if has_fragment_shader {
			self.shader_dxil_for_stage(builder.shaders.as_ref(), ShaderTypes::Fragment)?
		} else {
			Vec::new()
		};
		if (has_task_shader && task_shader.is_empty())
			|| mesh_shader.is_empty()
			|| (has_fragment_shader && fragment_shader.is_empty())
		{
			return None;
		}

		let mut render_targets = [Self::render_target_blend_desc(pipelines::raster::BlendMode::None); 8];
		let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
		let mut render_target_count = 0usize;
		let mut depth_stencil_format = DXGI_FORMAT_UNKNOWN;
		for attachment in builder.render_targets.iter() {
			if attachment.format.is_depth() {
				depth_stencil_format = Self::dxgi_format(attachment.format)?;
				continue;
			}
			if render_target_count >= rtv_formats.len() {
				break;
			}
			render_targets[render_target_count] = Self::render_target_blend_desc(attachment.blend);
			rtv_formats[render_target_count] = Self::dxgi_format(attachment.format)?;
			render_target_count += 1;
		}
		let has_depth_attachment = depth_stencil_format != DXGI_FORMAT_UNKNOWN;

		self.graphics_pipeline_state_create_attempt_count += 1;
		let mut stream = MeshPipelineStateStream {
			root_signature: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE,
				value: Some(root_signature),
			},
			amplification_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: if task_shader.is_empty() {
						std::ptr::null()
					} else {
						task_shader.as_ptr().cast()
					},
					BytecodeLength: task_shader.len(),
				},
			},
			mesh_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: mesh_shader.as_ptr().cast(),
					BytecodeLength: mesh_shader.len(),
				},
			},
			pixel_shader: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
				value: D3D12_SHADER_BYTECODE {
					pShaderBytecode: if fragment_shader.is_empty() {
						std::ptr::null()
					} else {
						fragment_shader.as_ptr().cast()
					},
					BytecodeLength: fragment_shader.len(),
				},
			},
			blend: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
				value: D3D12_BLEND_DESC {
					AlphaToCoverageEnable: BOOL(0),
					IndependentBlendEnable: BOOL((render_target_count > 1) as i32),
					RenderTarget: render_targets,
				},
			},
			sample_mask: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK,
				value: u32::MAX,
			},
			rasterizer: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER,
				value: D3D12_RASTERIZER_DESC {
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
				},
			},
			depth_stencil: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL,
				value: D3D12_DEPTH_STENCIL_DESC {
					DepthEnable: BOOL(has_depth_attachment as i32),
					DepthWriteMask: if has_depth_attachment && builder.depth_write {
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
					FrontFace: Self::disabled_stencil_op_desc(),
					BackFace: Self::disabled_stencil_op_desc(),
				},
			},
			depth_stencil_format: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
				value: depth_stencil_format,
			},
			render_targets: PipelineStateStreamSubobject {
				subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
				value: D3D12_RT_FORMAT_ARRAY {
					RTFormats: rtv_formats,
					NumRenderTargets: render_target_count as u32,
				},
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
		match unsafe { self.device.CreatePipelineState::<ID3D12PipelineState>(&desc) } {
			Ok(pipeline_state) => {
				self.graphics_pipeline_state_last_error = None;
				Some(pipeline_state)
			}
			Err(error) => {
				self.graphics_pipeline_state_last_error = Some(error.code().0);
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 mesh pipeline state: {error:?}; device removed reason: {removed_reason:?}"
				));
				None
			}
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
		let shader = self.shaders.get(parameter.handle.0 as usize)?;
		if !parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						stage,
						parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() {
					self.hlsl_specialization_compile_count += 1;
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
		let blend_enable = matches!(blend, pipelines::raster::BlendMode::Alpha);
		D3D12_RENDER_TARGET_BLEND_DESC {
			BlendEnable: BOOL(blend_enable as i32),
			LogicOpEnable: BOOL(0),
			SrcBlend: if blend_enable {
				D3D12_BLEND_SRC_ALPHA
			} else {
				D3D12_BLEND_ONE
			},
			DestBlend: if blend_enable {
				D3D12_BLEND_INV_SRC_ALPHA
			} else {
				D3D12_BLEND_ZERO
			},
			BlendOp: D3D12_BLEND_OP_ADD,
			SrcBlendAlpha: D3D12_BLEND_ONE,
			DestBlendAlpha: if blend_enable {
				D3D12_BLEND_INV_SRC_ALPHA
			} else {
				D3D12_BLEND_ZERO
			},
			BlendOpAlpha: D3D12_BLEND_OP_ADD,
			LogicOp: D3D12_LOGIC_OP_NOOP,
			RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
		}
	}

	pub(crate) fn disabled_stencil_op_desc() -> D3D12_DEPTH_STENCILOP_DESC {
		D3D12_DEPTH_STENCILOP_DESC {
			StencilFailOp: D3D12_STENCIL_OP_KEEP,
			StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
			StencilPassOp: D3D12_STENCIL_OP_KEEP,
			StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
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
			ray_tracing_state_object: None,
			ray_tracing_shader_identifiers: HashMap::default(),
			has_mesh_shader: false,
		});
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub(crate) fn create_compute_pipeline_state(
		&mut self,
		layout: PipelineLayoutHandle,
		shader_parameter: pipelines::ShaderParameter,
	) -> Option<ID3D12PipelineState> {
		let root_signature = self
			.pipeline_layouts
			.get(layout.0 as usize)
			.map(|layout| layout.root_signature.clone())?;
		let shader = self.shaders.get(shader_parameter.handle.0 as usize)?;
		let dxil = if !shader_parameter.specialization_map.is_empty() {
			if let Some(hlsl) = shader.hlsl.as_ref() {
				let dxil = self
					.compile_hlsl(
						hlsl.name.as_deref(),
						&hlsl.source,
						&hlsl.entry_point,
						shader_parameter.stage,
						shader_parameter.specialization_map,
					)
					.ok();
				if dxil.is_some() {
					self.hlsl_specialization_compile_count += 1;
				}
				dxil
			} else {
				shader.dxil.clone()
			}
		} else {
			shader.dxil.clone()
		}?;
		if dxil.is_empty() {
			return None;
		}
		self.compute_pipeline_state_create_attempt_count += 1;
		let mut desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
			pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
			CS: D3D12_SHADER_BYTECODE {
				pShaderBytecode: dxil.as_ptr().cast(),
				BytecodeLength: dxil.len(),
			},
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

	pub fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let shaders = builder.shaders;
		let (ray_tracing_state_object, ray_tracing_shader_identifiers) = self.create_ray_tracing_state_object(layout, &shaders);
		self.pipelines.push(Pipeline {
			layout,
			shaders: shaders.iter().map(|s| *s.handle).collect(),
			kind: PipelineKind::RayTracing,
			pipeline_state: None,
			ray_tracing_state_object,
			ray_tracing_shader_identifiers,
			has_mesh_shader: false,
		});

		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub(crate) fn create_ray_tracing_state_object(
		&mut self,
		layout: PipelineLayoutHandle,
		shaders: &[pipelines::ShaderParameter],
	) -> (
		Option<ID3D12StateObject>,
		HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]>,
	) {
		if !shaders.iter().any(|shader| {
			self.shaders
				.get(shader.handle.0 as usize)
				.and_then(|shader| shader.dxil.as_ref())
				.is_some_and(|dxil| !dxil.is_empty())
		}) {
			return (None, HashMap::default());
		}
		let Some(root_signature) = self
			.pipeline_layouts
			.get(layout.0 as usize)
			.map(|layout| layout.root_signature.clone())
		else {
			return (None, HashMap::default());
		};
		self.ray_tracing_state_object_create_attempt_count += 1;

		let mut export_names = Vec::with_capacity(shaders.len());
		let mut source_export_names = Vec::with_capacity(shaders.len());
		let mut exports = Vec::with_capacity(shaders.len());
		let mut libraries = Vec::with_capacity(shaders.len());
		let mut hit_group_names = Vec::with_capacity(shaders.len());
		let mut hit_groups = Vec::with_capacity(shaders.len());
		let mut identifier_exports = Vec::with_capacity(shaders.len());
		let mut subobjects = Vec::with_capacity(shaders.len() * 2 + 4);

		for shader_parameter in shaders {
			let Some(shader) = self.shaders.get(shader_parameter.handle.0 as usize) else {
				continue;
			};
			let Some(dxil) = shader.dxil.as_ref() else {
				continue;
			};
			if dxil.is_empty() {
				continue;
			}
			let export_name = wide_null(&format!("ghi_shader_{}", shader_parameter.handle.0));
			export_names.push(export_name);
			let export_name = PCWSTR(export_names.last().expect("Export name was just pushed.").as_ptr());
			let source_export_name = wide_null(
				shader
					.hlsl
					.as_ref()
					.map(|source| source.entry_point.as_str())
					.unwrap_or("main"),
			);
			source_export_names.push(source_export_name);
			let source_export_name = PCWSTR(
				source_export_names
					.last()
					.expect("Source export name was just pushed.")
					.as_ptr(),
			);
			let mut identifier_export = export_name;
			exports.push(D3D12_EXPORT_DESC {
				Name: export_name,
				ExportToRename: source_export_name,
				Flags: D3D12_EXPORT_FLAG_NONE,
			});
			let export = exports.last().expect("Export descriptor was just pushed.");
			libraries.push(D3D12_DXIL_LIBRARY_DESC {
				DXILLibrary: D3D12_SHADER_BYTECODE {
					pShaderBytecode: dxil.as_ptr().cast(),
					BytecodeLength: dxil.len(),
				},
				NumExports: 1,
				pExports: export,
			});
			let library = libraries.last().expect("DXIL library descriptor was just pushed.");
			subobjects.push(D3D12_STATE_SUBOBJECT {
				Type: D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
				pDesc: (library as *const D3D12_DXIL_LIBRARY_DESC).cast(),
			});

			match shader_parameter.stage {
				ShaderTypes::ClosestHit | ShaderTypes::AnyHit | ShaderTypes::Intersection => {
					let is_any_hit = matches!(shader_parameter.stage, ShaderTypes::AnyHit);
					let is_closest_hit = matches!(shader_parameter.stage, ShaderTypes::ClosestHit);
					let is_intersection = matches!(shader_parameter.stage, ShaderTypes::Intersection);
					let hit_group_name = wide_null(&format!("ghi_hit_group_{}", shader_parameter.handle.0));
					hit_group_names.push(hit_group_name);
					let hit_group_name = PCWSTR(hit_group_names.last().expect("Hit group name was just pushed.").as_ptr());
					identifier_export = hit_group_name;
					hit_groups.push(D3D12_HIT_GROUP_DESC {
						HitGroupExport: hit_group_name,
						Type: if is_intersection {
							D3D12_HIT_GROUP_TYPE_PROCEDURAL_PRIMITIVE
						} else {
							D3D12_HIT_GROUP_TYPE_TRIANGLES
						},
						AnyHitShaderImport: if is_any_hit { export_name } else { PCWSTR::null() },
						ClosestHitShaderImport: if is_closest_hit { export_name } else { PCWSTR::null() },
						IntersectionShaderImport: if is_intersection { export_name } else { PCWSTR::null() },
					});
					let hit_group = hit_groups.last().expect("Hit group descriptor was just pushed.");
					subobjects.push(D3D12_STATE_SUBOBJECT {
						Type: D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP,
						pDesc: (hit_group as *const D3D12_HIT_GROUP_DESC).cast(),
					});
				}
				_ => {}
			}
			identifier_exports.push((*shader_parameter.handle, identifier_export));
		}

		if subobjects.is_empty() {
			return (None, HashMap::default());
		}
		let mut global_root_signature = D3D12_GLOBAL_ROOT_SIGNATURE {
			pGlobalRootSignature: std::mem::ManuallyDrop::new(Some(root_signature)),
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
			pDesc: (&global_root_signature as *const D3D12_GLOBAL_ROOT_SIGNATURE).cast(),
		});
		let shader_config = D3D12_RAYTRACING_SHADER_CONFIG {
			MaxPayloadSizeInBytes: 32,
			MaxAttributeSizeInBytes: 32,
		};
		let shader_config_subobject_index = subobjects.len();
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
			pDesc: (&shader_config as *const D3D12_RAYTRACING_SHADER_CONFIG).cast(),
		});
		let shader_config_exports = identifier_exports.iter().map(|(_, export)| *export).collect::<Vec<_>>();
		let shader_config_association = D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION {
			pSubobjectToAssociate: (&subobjects[shader_config_subobject_index] as *const D3D12_STATE_SUBOBJECT).cast(),
			NumExports: shader_config_exports.len() as u32,
			pExports: shader_config_exports.as_ptr(),
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_SUBOBJECT_TO_EXPORTS_ASSOCIATION,
			pDesc: (&shader_config_association as *const D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION).cast(),
		});
		let pipeline_config = D3D12_RAYTRACING_PIPELINE_CONFIG {
			MaxTraceRecursionDepth: 1,
		};
		subobjects.push(D3D12_STATE_SUBOBJECT {
			Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
			pDesc: (&pipeline_config as *const D3D12_RAYTRACING_PIPELINE_CONFIG).cast(),
		});
		let desc = D3D12_STATE_OBJECT_DESC {
			Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
			NumSubobjects: subobjects.len() as u32,
			pSubobjects: subobjects.as_ptr(),
		};
		let state_object = unsafe { self.device.CreateStateObject::<ID3D12StateObject>(&desc) };
		// State-object creation synchronously consumes every subobject. Release the temporary root-signature clone afterward.
		unsafe { std::mem::ManuallyDrop::drop(&mut global_root_signature.pGlobalRootSignature) };
		let state_object = match state_object {
			Ok(state_object) => state_object,
			Err(error) => {
				let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
				self.log_dx12_error(format!(
					"Failed to create DX12 ray tracing state object: {error:?}. The most likely cause is an invalid DXIL library, hit group export, or ray tracing root signature. Device removed reason: {removed_reason:?}"
				));
				return (None, HashMap::default());
			}
		};
		let identifiers = Self::ray_tracing_shader_identifiers(&state_object, &identifier_exports);
		(Some(state_object), identifiers)
	}

	pub(crate) fn ray_tracing_shader_identifiers(
		state_object: &ID3D12StateObject,
		exports: &[(ShaderHandle, PCWSTR)],
	) -> HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]> {
		let Ok(properties) = state_object.cast::<ID3D12StateObjectProperties>() else {
			return HashMap::default();
		};
		let mut identifiers = HashMap::default();
		for &(shader_handle, export_name) in exports {
			let identifier = unsafe { properties.GetShaderIdentifier(export_name) };
			if identifier.is_null() {
				continue;
			}
			let mut bytes = [0u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize];
			unsafe {
				std::ptr::copy_nonoverlapping(identifier.cast::<u8>(), bytes.as_mut_ptr(), bytes.len());
			}
			identifiers.insert(shader_handle, bytes);
		}
		identifiers
	}
}
