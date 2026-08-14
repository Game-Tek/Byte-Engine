use super::super::*;

impl Context {
	pub fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let (metal_library, metal_entry_point, threadgroup_size) = match shader_source_type {
			crate::shader::Sources::SPIRV(_) => {
				eprintln!(
					"Metal shader creation failed for {:?} shader {:?}. The most likely cause is that SPIR-V was supplied to the Metal backend without translation to MSL or MTLB.",
					stage,
					name.unwrap_or("<unnamed>"),
				);
				return Err(());
			}
			crate::shader::Sources::DXIL(_) | crate::shader::Sources::HLSL { .. } => return Err(()),
			crate::shader::Sources::MTLB {
				binary,
				entry_point,
				threadgroup_size,
			} => {
				let data = DispatchData::from_bytes(binary);
				let library = self.device.newLibraryWithData_error(&data).map_err(|error| {
					eprintln!("Metal shader library load failed: {}", error.localizedDescription());
				})?;

				(Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
			crate::shader::Sources::MTL { source, entry_point } => {
				let threadgroup_size = match stage {
					crate::ShaderTypes::Task | crate::ShaderTypes::Mesh | crate::ShaderTypes::Compute => {
						parse_threadgroup_size_metadata(source)
					}
					_ => None,
				};
				let compile_options = mtl::MTLCompileOptions::new();
				let source = NSString::from_str(source);
				let library = self
					.device
					.newLibraryWithSource_options_error(&source, Some(&compile_options))
					.map_err(|error| {
						eprintln!("Metal shader compilation failed: {}", error.localizedDescription());
					})?;

				(Some(library), Some(entry_point.to_owned()), threadgroup_size)
			}
		};

		let stages = stage.into();

		self.shaders.push(Shader {
			name: crate::debug_name(name),
			stage: stages,
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
			metal_library,
			metal_entry_point,
			threadgroup_size,
		});

		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	pub(super) fn create_metal_function(
		&self,
		shader_parameter: &crate::pipelines::ShaderParameter,
	) -> Option<Retained<ProtocolObject<dyn mtl::MTLFunction>>> {
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let library = shader.metal_library.as_ref()?;
		let entry_point = shader.metal_entry_point.as_ref()?;
		let entry_point = NSString::from_str(entry_point);

		let constant_values = mtl::MTLFunctionConstantValues::new();

		for specialization_map_entry in shader_parameter.specialization_map {
			apply_specialization_map_entry(&constant_values, specialization_map_entry);
		}

		library
			.newFunctionWithName_constantValues_error(&entry_point, &constant_values)
			.map_err(|error| {
				eprintln!("Metal shader specialization failed: {}", error.localizedDescription());
			})
			.ok()
	}

	pub(super) fn create_pipeline_layout(
		&mut self,
		shaders: &[crate::pipelines::ShaderParameter],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let stage_resources = shaders
			.iter()
			.map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				(shader.stage, shader.shader_resource_descriptors.clone())
			})
			.collect::<Vec<_>>();
		let layout = build_pipeline_layout(self.device.as_ref(), &stage_resources, push_constant_ranges);
		self.pipeline_layouts.push(layout);
		graphics_hardware_interface::PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64)
	}

	pub(super) fn intern_pipeline_layout(
		&mut self,
		layout: PipelineLayout,
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		self.pipeline_layouts.push(layout);
		graphics_hardware_interface::PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64)
	}

	pub(super) fn get_or_create_vertex_layout(
		&mut self,
		vertex_elements: &[crate::pipelines::VertexElement],
	) -> VertexLayoutHandle {
		let elements = vertex_elements
			.iter()
			.map(|element| VertexElementDescriptor {
				name: element.name.to_owned(),
				format: element.format,
				binding: element.binding,
			})
			.collect::<Vec<_>>();
		let key = VertexLayoutKey {
			elements: elements.clone(),
		};

		if let Some(handle) = self.vertex_layout_indices.get(&key) {
			return *handle;
		}

		let max_binding = elements
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);
		let mut strides = vec![0; max_binding];
		let vertex_descriptor = mtl::MTLVertexDescriptor::vertexDescriptor();
		let mut binding_offsets = vec![0usize; max_binding];

		for (attribute_index, element) in elements.iter().enumerate() {
			strides[element.binding as usize] += element.format.size() as u32;

			let offset = binding_offsets[element.binding as usize];
			let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
			attribute.setFormat(utils::vertex_format(element.format));
			unsafe {
				attribute.setOffset(offset as _);
				attribute.setBufferIndex(element.binding as _);
			}

			binding_offsets[element.binding as usize] += element.format.size();
		}

		for (binding, stride) in strides.iter().copied().enumerate() {
			let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
			unsafe {
				layout.setStride(stride as _);
				layout.setStepRate(1);
			}
			layout.setStepFunction(mtl::MTLVertexStepFunction::PerVertex);
		}

		self.vertex_layouts.push(VertexLayout {
			elements,
			strides,
			vertex_descriptor,
		});
		let handle = VertexLayoutHandle((self.vertex_layouts.len() - 1) as u64);
		self.vertex_layout_indices.insert(key, handle);
		handle
	}

	pub(super) fn get_or_create_vertex_layout_from_prebuilt(&mut self, vertex_layout: VertexLayout) -> VertexLayoutHandle {
		let key = VertexLayoutKey {
			elements: vertex_layout.elements.clone(),
		};

		if let Some(handle) = self.vertex_layout_indices.get(&key) {
			return *handle;
		}

		self.vertex_layouts.push(vertex_layout);
		let handle = VertexLayoutHandle((self.vertex_layouts.len() - 1) as u64);
		self.vertex_layout_indices.insert(key, handle);
		handle
	}

	pub(super) fn intern_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.pipelines.push(pipeline);
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn intern_raster_pipeline(
		&mut self,
		pipeline: crate::metal::device::Pipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.intern_pipeline_layout(pipeline.layout);
		let vertex_layout = pipeline
			.vertex_layout
			.map(|vertex_layout| self.get_or_create_vertex_layout_from_prebuilt(vertex_layout));

		self.intern_pipeline(Pipeline {
			pipeline: pipeline.pipeline,
			depth_stencil_state: pipeline.depth_stencil_state,
			layout,
			vertex_layout,
			shader_handles: pipeline.shader_handles,
			materializations: RefCell::new(HashMap::default()),
			compute_threadgroup_size: pipeline.compute_threadgroup_size,
			object_threadgroup_size: pipeline.object_threadgroup_size,
			mesh_threadgroup_size: pipeline.mesh_threadgroup_size,
			face_winding: pipeline.face_winding,
			cull_mode: pipeline.cull_mode,
		})
	}

	pub fn intern_compute_pipeline(
		&mut self,
		pipeline: crate::metal::device::ComputePipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.intern_pipeline_layout(pipeline.layout);

		self.intern_pipeline(Pipeline {
			pipeline: pipeline.pipeline,
			depth_stencil_state: pipeline.depth_stencil_state,
			layout,
			vertex_layout: None,
			shader_handles: pipeline.shader_handles,
			materializations: RefCell::new(HashMap::default()),
			compute_threadgroup_size: pipeline.compute_threadgroup_size,
			object_threadgroup_size: pipeline.object_threadgroup_size,
			mesh_threadgroup_size: pipeline.mesh_threadgroup_size,
			face_winding: pipeline.face_winding,
			cull_mode: pipeline.cull_mode,
		})
	}

	pub fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let has_depth_attachment = builder
			.render_targets
			.iter()
			.any(|attachment| attachment.format.channel_layout() == crate::ChannelLayout::Depth);
		let vertex_layout = self.get_or_create_vertex_layout(builder.vertex_elements.as_ref());
		let mut shader_handles = HashMap::default();
		let mut object_function = None;
		let mut vertex_function = None;
		let mut mesh_function = None;
		let mut fragment_function = None;
		let mut object_threadgroup_size = None;
		let mut mesh_threadgroup_size = None;
		for shader_parameter in builder.shaders.iter() {
			let shader = &self.shaders[shader_parameter.handle.0 as usize];
			shader_handles.insert(*shader_parameter.handle, [0; 32]);
			match shader_parameter.stage {
				crate::ShaderTypes::Task => {
					object_function = self.create_metal_function(shader_parameter);
					object_threadgroup_size = shader.threadgroup_size;
				}
				crate::ShaderTypes::Vertex => vertex_function = self.create_metal_function(shader_parameter),
				crate::ShaderTypes::Mesh => {
					mesh_function = self.create_metal_function(shader_parameter);
					mesh_threadgroup_size = shader.threadgroup_size;
				}
				crate::ShaderTypes::Fragment => fragment_function = self.create_metal_function(shader_parameter),
				_ => {}
			}
		}

		let depth_stencil_state = if has_depth_attachment {
			let descriptor = mtl::MTLDepthStencilDescriptor::new();
			descriptor.setDepthCompareFunction(mtl::MTLCompareFunction::GreaterEqual);
			descriptor.setDepthWriteEnabled(builder.depth_write);
			self.device.newDepthStencilStateWithDescriptor(&descriptor)
		} else {
			None
		};

		let raster_pipeline_state = if let Some(mesh_function) = mesh_function.as_ref() {
			let descriptor = mtl::MTLMeshRenderPipelineDescriptor::new();
			#[cfg(debug_assertions)]
			if self.settings.debug_labels {
				if let Some(name) = builder.name {
					descriptor.setLabel(Some(&NSString::from_str(name)));
				}
			}
			unsafe {
				descriptor.setObjectFunction(object_function.as_ref().map(|function| function.as_ref()));
				descriptor.setMeshFunction(Some(mesh_function.as_ref()));
				descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			}

			configure_mesh_render_targets(&descriptor, builder.render_targets.as_ref());

			self.device
				.newRenderPipelineStateWithMeshDescriptor_options_reflection_error(
					&descriptor,
					mtl::MTLPipelineOption::None,
					None,
				)
				.unwrap_or_else(|error| {
					panic!(
						"Metal mesh raster pipeline creation failed: {}. The most likely cause is invalid shader functions or render-target state in the raster pipeline descriptor.",
						error.localizedDescription(),
					)
				})
				.into()
		} else if let Some(vertex_function) = vertex_function.as_ref() {
			let descriptor = mtl::MTLRenderPipelineDescriptor::new();
			#[cfg(debug_assertions)]
			if self.settings.debug_labels {
				if let Some(name) = builder.name {
					descriptor.setLabel(Some(&NSString::from_str(name)));
				}
			}
			descriptor.setVertexFunction(Some(vertex_function.as_ref()));
			descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			descriptor.setVertexDescriptor(Some(&self.vertex_layouts[vertex_layout.0 as usize].vertex_descriptor));

			configure_render_targets(&descriptor, builder.render_targets.as_ref());

			self.device
				.newRenderPipelineStateWithDescriptor_error(&descriptor)
				.unwrap_or_else(|error| {
					panic!(
						"Metal raster pipeline creation failed: {}. The most likely cause is invalid shader functions or render-target state in the raster pipeline descriptor.",
						error.localizedDescription(),
					)
				})
				.into()
		} else {
			let shader_names = builder
				.shaders
				.iter()
				.map(|shader_parameter| {
					let shader = &self.shaders[shader_parameter.handle.0 as usize];
					format!(
						"{:?} {:?}",
						shader_parameter.stage,
						shader.name.as_deref().unwrap_or("<unnamed>")
					)
				})
				.collect::<Vec<_>>()
				.join(", ");
			panic!(
				"Metal raster pipeline creation failed because no vertex or mesh shader function was available. The most likely cause is shader creation failed or SPIR-V was supplied to the Metal backend without translation to MSL or MTLB. Shaders: {shader_names}",
			);
		};

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Raster(raster_pipeline_state),
			depth_stencil_state,
			layout,
			vertex_layout: Some(vertex_layout),
			shader_handles,
			materializations: RefCell::new(HashMap::default()),
			compute_threadgroup_size: None,
			object_threadgroup_size,
			mesh_threadgroup_size,
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		});

		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.create_pipeline_layout(std::slice::from_ref(&builder.shader), builder.push_constant_ranges);
		let shader_handle = *builder.shader.handle;
		let compute_pipeline_state = {
			let shader_parameter = &builder.shader;
			let shader = &self.shaders[shader_handle.0 as usize];
			assert!(
				shader.stage == crate::Stages::COMPUTE,
				"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
			);
			let function = self.create_metal_function(shader_parameter).expect(
				"Metal compute pipeline creation requires a Metal shader function. The most likely cause is that this compute shader was created from SPIR-V, which this backend does not translate to MSL.",
			);

			let descriptor = mtl::MTLComputePipelineDescriptor::new();
			#[cfg(debug_assertions)]
			if self.settings.debug_labels {
				if let Some(name) = builder.name {
					descriptor.setLabel(Some(&NSString::from_str(name)));
				}
			}
			descriptor.setComputeFunction(Some(&function));

			Some(
				self.device
					.newComputePipelineStateWithDescriptor_options_reflection_error(
						&descriptor,
						mtl::MTLPipelineOption::None,
						None,
					)
					.expect("Metal compute pipeline creation failed. The most likely cause is that the shader function was invalid for compute pipeline creation."),
			)
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);
		let compute_threadgroup_size = self.shaders[shader_handle.0 as usize].threadgroup_size;

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			depth_stencil_state: None,
			layout,
			vertex_layout: None,
			shader_handles,
			materializations: RefCell::new(HashMap::default()),
			compute_threadgroup_size,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		self.pipelines.push(Pipeline {
			pipeline: PipelineState::RayTracing,
			depth_stencil_state: None,
			layout,
			vertex_layout: None,
			shader_handles: HashMap::default(),
			materializations: RefCell::new(HashMap::default()),
			compute_threadgroup_size: None,
			object_threadgroup_size: None,
			mesh_threadgroup_size: None,
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		// TODO: Metal ray tracing pipeline mapping.
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}
}
