use super::*;

impl Context {
	pub(crate) fn create_vulkan_graphics_pipeline_create_info<'a, R>(
		&'a mut self,
		builder: crate::pipelines::raster::Builder,
		after_build: impl FnOnce(
			&'a mut Self,
			crate::pipelines::raster::Builder,
			graphics_hardware_interface::PipelineLayoutHandle,
			vk::GraphicsPipelineCreateInfo,
		) -> R,
	) -> R {
		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
		;

		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let pipeline_create_info = pipeline_create_info.layout(vk::PipelineLayout::null());

		let mut vertex_input_attribute_descriptions = vec![];

		let mut offset_per_binding = [0, 0, 0, 0, 0, 0, 0, 0]; // Assume 8 bindings max

		for (i, vertex_element) in builder.vertex_elements.iter().enumerate() {
			let ve = vk::VertexInputAttributeDescription::default()
				.binding(vertex_element.binding)
				.location(i as u32)
				.format(vertex_element.format.into())
				.offset(offset_per_binding[vertex_element.binding as usize]);

			vertex_input_attribute_descriptions.push(ve);

			offset_per_binding[vertex_element.binding as usize] += vertex_element.format.size() as u32;
		}

		let vertex_binding_descriptions = if let Some(max_binding) = builder.vertex_elements.iter().map(|ve| ve.binding).max() {
			let max_binding = max_binding as usize + 1;

			let mut vertex_binding_descriptions = Vec::with_capacity(max_binding);

			for i in 0..max_binding {
				vertex_binding_descriptions.push(
					vk::VertexInputBindingDescription::default()
						.binding(i as u32)
						.stride(offset_per_binding[i as usize])
						.input_rate(vk::VertexInputRate::VERTEX),
				)
			}

			vertex_binding_descriptions
		} else {
			Vec::new()
		};

		let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
			.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
			.vertex_binding_descriptions(&vertex_binding_descriptions);

		let pipeline_create_info = pipeline_create_info.vertex_input_state(&vertex_input_state);

		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);
		let mut entries = [vk::SpecializationMapEntry::default(); 32];
		let mut entry_count = 0;
		let specilization_info_count = 0;

		let stage_mappings = builder
			.shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];
				crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors)
			})
			.collect::<Vec<_>>();
		let mut mapping_infos = stage_mappings
			.iter()
			.map(|mappings| vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings))
			.collect::<Vec<_>>();
		let stages = builder
			.shaders
			.iter()
			.zip(mapping_infos.iter_mut())
			.map(|(stage, mapping_info)| {
				for entry in stage.specialization_map.iter() {
					specialization_entries_buffer.extend_from_slice(entry.get_data());

					entries[entry_count] = vk::SpecializationMapEntry::default()
						.constant_id(entry.get_constant_id())
						.size(entry.get_size())
						.offset(specialization_entries_buffer.len() as u32);

					entry_count += 1;
				}

				let shader = &self.shaders[stage.handle.0 as usize];

				assert!(specilization_info_count == 0);

				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		let pipeline_create_info = pipeline_create_info.stages(&stages);

		let pipeline_color_blend_attachments = builder
			.render_targets
			.iter()
			.filter(|a| !a.format.is_depth())
			.map(|attachment| {
				let blend_state =
					vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);

				match attachment.blend {
					crate::pipelines::raster::BlendMode::None => blend_state
						.blend_enable(false)
						.src_color_blend_factor(vk::BlendFactor::ONE)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ZERO)
						.dst_alpha_blend_factor(vk::BlendFactor::ZERO)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
					crate::pipelines::raster::BlendMode::Alpha => blend_state
						.blend_enable(true)
						.src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
				}
			})
			.collect::<Vec<_>>();

		let color_attachement_formats: Vec<vk::Format> = builder
			.render_targets
			.iter()
			.filter(|a| !a.format.is_depth())
			.map(|a| to_format(a.format))
			.collect::<Vec<_>>();

		let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
			.logic_op_enable(false)
			.logic_op(vk::LogicOp::COPY)
			.attachments(&pipeline_color_blend_attachments)
			.blend_constants([0.0, 0.0, 0.0, 0.0]);

		let has_depth = builder.render_targets.iter().find(|attachment| attachment.format.is_depth());
		let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
			.color_attachment_formats(&color_attachement_formats)
			.depth_attachment_format(if let Some(depth_attachment) = has_depth {
				to_format(depth_attachment.format)
			} else {
				vk::Format::UNDEFINED
			});

		let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

		let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
			.depth_test_enable(true)
			.depth_write_enable(builder.depth_write)
			.depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
			.depth_bounds_test_enable(false)
			.stencil_test_enable(false)
			.front(vk::StencilOpState::default())
			.back(vk::StencilOpState::default());

		let pipeline_create_info = if has_depth.is_some() {
			pipeline_create_info.depth_stencil_state(&depth_stencil_state)
		} else {
			pipeline_create_info
		};

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

		let viewports = [vk::Viewport::default()
			.x(0.0)
			.y(9.0)
			.width(16.0)
			.height(9.0)
			.min_depth(0.0)
			.max_depth(1.0)];

		let scissors = [vk::Rect2D::default()
			.offset(vk::Offset2D { x: 0, y: 0 })
			.extent(vk::Extent2D { width: 16, height: 9 })];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors);

		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
			.dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

		let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
			.depth_clamp_enable(false)
			.rasterizer_discard_enable(false)
			.polygon_mode(match builder.fill_mode {
				crate::pipelines::raster::FillMode::Solid => vk::PolygonMode::FILL,
				crate::pipelines::raster::FillMode::Wireframe => vk::PolygonMode::LINE,
			})
			.cull_mode(match builder.cull_mode {
				crate::pipelines::raster::CullMode::None => vk::CullModeFlags::NONE,
				crate::pipelines::raster::CullMode::Front => vk::CullModeFlags::FRONT,
				crate::pipelines::raster::CullMode::Back => vk::CullModeFlags::BACK,
			})
			.front_face(match builder.face_winding {
				crate::pipelines::raster::FaceWinding::Clockwise => vk::FrontFace::CLOCKWISE,
				crate::pipelines::raster::FaceWinding::CounterClockwise => vk::FrontFace::COUNTER_CLOCKWISE,
			})
			.depth_bias_enable(false)
			.depth_bias_constant_factor(0.0)
			.depth_bias_clamp(0.0)
			.depth_bias_slope_factor(0.0)
			.line_width(1.0);

		let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
			.sample_shading_enable(false)
			.rasterization_samples(vk::SampleCountFlags::TYPE_1)
			.min_sample_shading(1.0)
			.alpha_to_coverage_enable(false)
			.alpha_to_one_enable(false);

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info
			.viewport_state(&viewport_state)
			.dynamic_state(&dynamic_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state);
		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let pipeline_create_info = pipeline_create_info
			.push(&mut descriptor_heap_flags)
			.push(&mut rendering_info);

		after_build(self, builder, pipeline_layout_handle, pipeline_create_info)
	}

	/// Returns a cached descriptor-heap layout derived from shader resource metadata.
	pub(crate) fn get_or_create_pipeline_layout(
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
		let layout = crate::vulkan::build_pipeline_layout(
			&stage_resources,
			push_constant_ranges,
			&self.device.descriptor_heap_properties,
		);
		let key = PipelineLayoutKey::new(&layout);
		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		let handle = graphics_hardware_interface::PipelineLayoutHandle(self.pipeline_layouts.len() as u64);
		self.pipeline_layouts.push(layout);
		self.pipeline_layout_indices.insert(key, handle);
		handle
	}

	pub(crate) fn create_vulkan_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_graphics_pipeline_create_info(
			builder,
			|this, _builder, pipeline_layout_handle, pipeline_create_info| {
				let pipeline_create_infos = [pipeline_create_info];

				let pipelines = unsafe {
					this.device
						.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None)
						.expect("No pipeline")
				};

				let pipeline = pipelines[0];

				let handle = graphics_hardware_interface::PipelineHandle(this.pipelines.len() as u64);

				this.pipelines.push(Pipeline {
					pipeline,
					layout: pipeline_layout_handle,
					shader_handles: HashMap::new(),
				});

				handle
			},
		)
	}
}
