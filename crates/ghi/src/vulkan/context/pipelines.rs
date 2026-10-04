use super::*;

impl Context {
	/// Adds a built pipeline and returns the handle commands bind it with.
	///
	/// Pipelines built on factory threads bring their own layouts, so an equal layout this context already holds is
	/// shared instead of added again.
	pub(crate) fn add_pipeline(
		&mut self,
		pipeline: vk::Pipeline,
		layout: PipelineLayout,
		shader_handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]>,
	) -> graphics_hardware_interface::PipelineHandle {
		let layouts = &mut self.pipeline_layouts;
		let layout = *self
			.pipeline_layout_indices
			.entry(PipelineLayoutKey::new(&layout))
			.or_insert_with(|| {
				layouts.push(layout);
				graphics_hardware_interface::PipelineLayoutHandle(layouts.len() as u64 - 1)
			});
		self.pipelines.push(Pipeline {
			pipeline,
			layout,
			shader_handles,
		});
		graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64 - 1)
	}
}

/// Builds the descriptor-heap layout shared by the shaders that `parameters` name.
pub(super) fn shader_pipeline_layout(
	shaders: &[Shader],
	parameters: &[crate::pipelines::ShaderParameter],
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
	properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
) -> PipelineLayout {
	let stage_resources = parameters
		.iter()
		.map(|parameter| {
			let shader = &shaders[parameter.handle.0 as usize];
			(shader.stage, shader.shader_resource_descriptors.clone())
		})
		.collect::<Vec<_>>();
	crate::vulkan::build_pipeline_layout(&stage_resources, push_constant_ranges, properties)
}

/// Creates a native raster pipeline and the descriptor-heap layout its shaders need.
///
/// It only reads device state, so detached factories call it on compiler threads and contexts call it for their
/// own pipelines. `shaders` holds the modules that the builder's shader handles index into.
pub(crate) fn build_raster_pipeline(
	device: &ash::Device,
	descriptor_heap_properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
	shaders: &[Shader],
	builder: crate::pipelines::raster::Builder,
) -> (vk::Pipeline, PipelineLayout) {
	use crate::pipelines::raster::{BlendMode, CullMode, FaceWinding, FillMode};

	let layout = shader_pipeline_layout(
		shaders,
		builder.shaders,
		builder.push_constant_ranges,
		descriptor_heap_properties,
	);
	let mut offset_per_binding = [0u32; 8]; // Assume 8 bindings max
	let vertex_input_attribute_descriptions = builder
		.vertex_elements
		.iter()
		.enumerate()
		.map(|(i, vertex_element)| {
			let offset = &mut offset_per_binding[vertex_element.binding as usize];
			let description = vk::VertexInputAttributeDescription::default()
				.binding(vertex_element.binding)
				.location(i as u32)
				.format(vertex_element.format.into())
				.offset(*offset);
			*offset += vertex_element.format.size() as u32;
			description
		})
		.collect::<Vec<_>>();

	let binding_count = builder
		.vertex_elements
		.iter()
		.map(|vertex_element| vertex_element.binding as usize + 1)
		.max()
		.unwrap_or(0);
	let vertex_binding_descriptions = (0..binding_count)
		.map(|binding| {
			vk::VertexInputBindingDescription::default()
				.binding(binding as u32)
				.stride(offset_per_binding[binding])
				.input_rate(vk::VertexInputRate::VERTEX)
		})
		.collect::<Vec<_>>();

	let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
		.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
		.vertex_binding_descriptions(&vertex_binding_descriptions);

	let stage_specializations = builder
		.shaders
		.iter()
		.map(|stage| crate::vulkan::utils::build_specialization_entries(stage.specialization_map))
		.collect::<Vec<_>>();
	let specialization_infos = stage_specializations
		.iter()
		.map(|(data, entries)| vk::SpecializationInfo::default().data(data).map_entries(entries))
		.collect::<Vec<_>>();

	let stage_mappings = builder
		.shaders
		.iter()
		.map(|stage| {
			let shader = &shaders[stage.handle.0 as usize];
			crate::vulkan::build_shader_mappings(&layout, &shader.shader_resource_descriptors)
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
		.zip(&specialization_infos)
		.map(|((stage, mapping_info), specialization_info)| {
			vk::PipelineShaderStageCreateInfo::default()
				.push(mapping_info)
				.stage(stage.stage.into())
				.module(shaders[stage.handle.0 as usize].shader)
				.name(c"main")
				.specialization_info(specialization_info)
		})
		.collect::<Vec<_>>();

	let color_targets = builder.render_targets.iter().filter(|target| !target.format.is_depth());
	let pipeline_color_blend_attachments = color_targets
		.clone()
		.map(|attachment| {
			let (blend_enable, src_color_blend_factor, dst_blend_factor) = match attachment.blend {
				BlendMode::None => (false, vk::BlendFactor::ONE, vk::BlendFactor::ZERO),
				BlendMode::Alpha => (true, vk::BlendFactor::SRC_ALPHA, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
				BlendMode::Premultiplied => (true, vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
			};
			vk::PipelineColorBlendAttachmentState::default()
				.color_write_mask(vk::ColorComponentFlags::RGBA)
				.blend_enable(blend_enable)
				.src_color_blend_factor(src_color_blend_factor)
				.src_alpha_blend_factor(vk::BlendFactor::ONE)
				.dst_color_blend_factor(dst_blend_factor)
				.dst_alpha_blend_factor(dst_blend_factor)
				.color_blend_op(vk::BlendOp::ADD)
				.alpha_blend_op(vk::BlendOp::ADD)
		})
		.collect::<Vec<_>>();
	let color_attachment_formats = color_targets.map(|target| to_format(target.format)).collect::<Vec<_>>();

	let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
		.logic_op(vk::LogicOp::COPY)
		.attachments(&pipeline_color_blend_attachments);

	let depth_attachment = builder.render_targets.iter().find(|target| target.format.is_depth());
	let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
		.color_attachment_formats(&color_attachment_formats)
		.depth_attachment_format(depth_attachment.map_or(vk::Format::UNDEFINED, |target| to_format(target.format)));

	let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
		.depth_test_enable(true)
		.depth_write_enable(builder.depth_write)
		.depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);

	let input_assembly_state =
		vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);

	let viewports = [vk::Viewport::default().y(9.0).width(16.0).height(9.0).max_depth(1.0)];
	let scissors = [vk::Rect2D::default().extent(vk::Extent2D { width: 16, height: 9 })];
	let viewport_state = vk::PipelineViewportStateCreateInfo::default()
		.viewports(&viewports)
		.scissors(&scissors);

	let dynamic_state =
		vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

	let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
		.polygon_mode(match builder.fill_mode {
			FillMode::Solid => vk::PolygonMode::FILL,
			FillMode::Wireframe => vk::PolygonMode::LINE,
		})
		.cull_mode(match builder.cull_mode {
			CullMode::None => vk::CullModeFlags::NONE,
			CullMode::Front => vk::CullModeFlags::FRONT,
			CullMode::Back => vk::CullModeFlags::BACK,
		})
		.front_face(match builder.face_winding {
			FaceWinding::Clockwise => vk::FrontFace::CLOCKWISE,
			FaceWinding::CounterClockwise => vk::FrontFace::COUNTER_CLOCKWISE,
		})
		.line_width(1.0);

	let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
		.rasterization_samples(vk::SampleCountFlags::TYPE_1)
		.min_sample_shading(1.0);

	let mut descriptor_heap_flags =
		vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
	// The render pass and layout stay null because of VK_KHR_dynamic_rendering and descriptor heaps.
	let mut pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
		.vertex_input_state(&vertex_input_state)
		.stages(&stages)
		.color_blend_state(&color_blend_state)
		.input_assembly_state(&input_assembly_state)
		.viewport_state(&viewport_state)
		.dynamic_state(&dynamic_state)
		.rasterization_state(&rasterization_state)
		.multisample_state(&multisample_state)
		.push(&mut descriptor_heap_flags)
		.push(&mut rendering_info);
	if depth_attachment.is_some() {
		pipeline_create_info = pipeline_create_info.depth_stencil_state(&depth_stencil_state);
	}

	let pipeline = unsafe {
		device
			.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_create_info], None)
			.expect("No pipeline")[0]
	};

	(pipeline, layout)
}

/// Creates a native compute pipeline whose flat bindings map directly into descriptor heaps.
///
/// Like [`build_raster_pipeline`], it only reads device state, so detached factories and contexts share it.
pub(crate) fn build_compute_pipeline(
	device: &ash::Device,
	descriptor_heap_properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
	shaders: &[Shader],
	builder: crate::pipelines::compute::Builder,
) -> crate::vulkan::ComputePipeline {
	let shader_parameter = builder.shader;
	let shader = &shaders[shader_parameter.handle.0 as usize];
	let layout = shader_pipeline_layout(
		shaders,
		std::slice::from_ref(&shader_parameter),
		builder.push_constant_ranges,
		descriptor_heap_properties,
	);
	let mappings = crate::vulkan::build_shader_mappings(&layout, &shader.shader_resource_descriptors);
	let mut mapping_info = vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings);
	let (specialization_entries_buffer, specialization_map_entries) =
		crate::vulkan::utils::build_specialization_entries(shader_parameter.specialization_map);
	let specialization_info = vk::SpecializationInfo::default()
		.data(&specialization_entries_buffer)
		.map_entries(&specialization_map_entries);
	let stage = vk::PipelineShaderStageCreateInfo::default()
		.push(&mut mapping_info)
		.stage(vk::ShaderStageFlags::COMPUTE)
		.module(shader.shader)
		.name(c"main")
		.specialization_info(&specialization_info);
	let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
	let create_infos = [vk::ComputePipelineCreateInfo::default()
		.push(&mut flags)
		.stage(stage)
		.layout(vk::PipelineLayout::null())];
	let pipeline = unsafe {
		device
			.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
			.expect("Vulkan descriptor-heap compute pipeline creation failed. The most likely cause is an invalid shader resource mapping or specialization constant.")[0]
	};

	crate::vulkan::ComputePipeline {
		pipeline,
		layout,
		shader_handles: HashMap::from_iter([(*shader_parameter.handle, [0; 32])]),
	}
}
