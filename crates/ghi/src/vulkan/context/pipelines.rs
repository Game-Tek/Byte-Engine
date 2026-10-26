use super::*;
use crate::vulkan::pipeline_binaries::{PipelineBinaries, create_pipeline};

impl Context {
	/// Adds a built pipeline and returns the handle commands bind it with.
	///
	/// Pipelines built on factory threads bring their own layouts, so an equal layout this context already holds is
	/// shared instead of added again.
	pub(crate) fn add_pipeline(
		&mut self,
		pipeline: vk::Pipeline,
		layout: PipelineLayout,
	) -> graphics_hardware_interface::PipelineHandle {
		let layouts = &mut self.pipeline_layouts;
		let layout = *self
			.pipeline_layout_indices
			.entry(PipelineLayoutKey::new(&layout))
			.or_insert_with(|| {
				layouts.push(layout);
				graphics_hardware_interface::PipelineLayoutHandle(layouts.len() as u64 - 1)
			});
		self.pipelines.push(Pipeline { pipeline, layout });
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
			(shader.stage, shader.shader_resource_descriptors.as_slice())
		})
		.collect::<StageList<_>>();
	crate::vulkan::build_pipeline_layout(&stage_resources, push_constant_ranges, properties)
}

/// The `StageList` type alias holds one value per shader stage without touching the heap for raster and compute
/// pipelines, which have at most a handful of stages.
pub(super) type StageList<T> = SmallVec<[T; 4]>;

/// Creates a pipeline from its shaders, reading their code only when the pipeline has to be compiled.
///
/// `create` gets the pipeline's stages and creates the pipeline from them with [`create_pipeline`]. Its first
/// argument is the stages that carry each shader's SPIR-V inline, so no shader module exists, and its second is the
/// same stages with each shader named by its identifier instead; pass them on as `compiled` and `identified`.
///
/// When every shader has an identifier, `create` first runs with the identified stages alone, which finds a
/// pipeline the cache holds without touching any code. Only when that reports `VK_PIPELINE_COMPILE_REQUIRED` does
/// `create` run again with the code, which each shader either keeps or was lent through
/// [`crate::pipelines::ShaderParameter::with_code`]. Every stage is chained to its descriptor-heap mapping and its
/// specialization constants, and everything the stages point at lives until `create` returns.
///
/// # Panics
///
/// Panics when the pipeline has to be compiled and a shader that keeps no code was lent none.
pub(super) fn with_shader_stages(
	shaders: &[Shader],
	parameters: &[crate::pipelines::ShaderParameter],
	layout: &PipelineLayout,
	create: impl Fn(
		Option<&[vk::PipelineShaderStageCreateInfo<'_>]>,
		Option<&[vk::PipelineShaderStageCreateInfo<'_>]>,
	) -> Result<vk::Pipeline, vk::Result>,
) -> Result<vk::Pipeline, vk::Result> {
	let shaders = parameters.iter().map(|parameter| &shaders[parameter.handle.0 as usize]);

	let mappings = shaders
		.clone()
		.map(|shader| crate::vulkan::build_shader_mappings(layout, &shader.shader_resource_descriptors))
		.collect::<StageList<_>>();
	let specializations = parameters
		.iter()
		.map(|parameter| crate::vulkan::utils::build_specialization_entries(parameter.specialization_map))
		.collect::<StageList<_>>();
	let specialization_infos = specializations
		.iter()
		.map(|(data, entries)| vk::SpecializationInfo::default().data(data).map_entries(entries))
		.collect::<StageList<_>>();
	// Each list of stages links its own mapping structures, because a structure can sit in only one chain.
	let mapping_infos = || {
		let mapping_info = |mappings| vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings);
		mappings
			.iter()
			.map(|mappings| mapping_info(mappings))
			.collect::<StageList<_>>()
	};

	let mut identifiers = shaders
		.clone()
		.map(|shader| {
			let identifier = vk::PipelineShaderStageModuleIdentifierCreateInfoEXT::default();
			Some(identifier.identifier(shader.identifier.as_ref()?.bytes()))
		})
		.collect::<Option<StageList<_>>>();
	let mut identified_mapping_infos = mapping_infos();
	let identified_stages = identifiers
		.as_mut()
		.map(|identifiers| shader_stages(parameters, &mut identified_mapping_infos, identifiers, &specialization_infos));
	if let Some(identified_stages) = &identified_stages {
		match create(None, Some(identified_stages)) {
			Err(vk::Result::PIPELINE_COMPILE_REQUIRED) => {}
			result => return result,
		}
	}

	// Viewing lent code as words reads it only when it lies off a word boundary and has to be copied.
	let code = shaders
		.zip(parameters)
		.map(|(shader, parameter)| match &shader.code {
			Some(code) => Some(std::borrow::Cow::Borrowed(code.words())),
			None => parameter.code.and_then(crate::vulkan::spirv_words),
		})
		.collect::<Option<StageList<_>>>()
		.expect("Shader code is unavailable. The most likely cause is that a shader created from a keyed source was used in a pipeline without lending its code through `ShaderParameter::with_code`.");
	let mut code = code
		.iter()
		.map(|code| vk::ShaderModuleCreateInfo::default().code(code))
		.collect::<StageList<_>>();
	let mut code_mapping_infos = mapping_infos();
	let stages = shader_stages(parameters, &mut code_mapping_infos, &mut code, &specialization_infos);
	create(Some(&stages), identified_stages.as_deref())
}

/// Builds the stage create infos of a pipeline, each chained to its heap mapping and to its entry of `sources`,
/// which tells the driver what the shader is: its SPIR-V or its identifier.
fn shader_stages<'a, 'b: 'a, 'c: 'a, S>(
	parameters: &[crate::pipelines::ShaderParameter],
	mapping_infos: &'a mut [vk::ShaderDescriptorSetAndBindingMappingInfoEXT<'b>],
	sources: &'a mut [S],
	specialization_infos: &'a [vk::SpecializationInfo<'b>],
) -> StageList<vk::PipelineShaderStageCreateInfo<'a>>
where
	S: vk::Extends<vk::PipelineShaderStageCreateInfo<'a>> + vk::TaggedStructure<'c>,
{
	parameters
		.iter()
		.zip(mapping_infos)
		.zip(sources)
		.zip(specialization_infos)
		.map(|(((parameter, mapping_info), source), specialization_info)| {
			vk::PipelineShaderStageCreateInfo::default()
				.push(mapping_info)
				.push(source)
				.stage(parameter.stage.into())
				.name(c"main")
				.specialization_info(specialization_info)
		})
		.collect()
}

/// Creates a native raster pipeline and the descriptor-heap layout its shaders need.
///
/// It only reads device state, so detached factories call it on compiler threads and contexts call it for their
/// own pipelines. `shaders` holds the modules that the builder's shader handles index into.
pub(crate) fn build_raster_pipeline(
	device: &ash::Device,
	descriptor_heap_properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
	pipeline_binaries: Option<&PipelineBinaries>,
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
		.collect::<SmallVec<[_; 16]>>();

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
		.collect::<SmallVec<[_; 8]>>();

	let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
		.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
		.vertex_binding_descriptions(&vertex_binding_descriptions);

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
		.collect::<SmallVec<[_; 8]>>();
	let color_attachment_formats = color_targets
		.map(|target| to_format(target.format))
		.collect::<SmallVec<[_; 8]>>();

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

	// The render pass and layout stay null because of VK_KHR_dynamic_rendering and descriptor heaps.
	let mut pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
		.vertex_input_state(&vertex_input_state)
		.color_blend_state(&color_blend_state)
		.input_assembly_state(&input_assembly_state)
		.viewport_state(&viewport_state)
		.dynamic_state(&dynamic_state)
		.rasterization_state(&rasterization_state)
		.multisample_state(&multisample_state)
		.push(&mut rendering_info);
	if depth_attachment.is_some() {
		pipeline_create_info = pipeline_create_info.depth_stencil_state(&depth_stencil_state);
	}

	let pipeline = with_shader_stages(shaders, builder.shaders, &layout, |stages, identified_stages| {
		let compiled = stages.map(|stages| pipeline_create_info.stages(stages));
		let identified = identified_stages.map(|stages| pipeline_create_info.stages(stages));
		create_pipeline(pipeline_binaries, compiled.as_ref(), identified.as_ref(), |create_info| {
			unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), std::slice::from_ref(create_info), None) }
				.map(|pipelines| pipelines[0])
				.map_err(|(_, result)| result)
		})
	})
	.expect("Vulkan raster pipeline creation failed. The most likely cause is an invalid shader resource mapping or a shader interface that does not match the render targets.");

	(pipeline, layout)
}

/// Creates a native compute pipeline whose flat bindings map directly into descriptor heaps.
///
/// Like [`build_raster_pipeline`], it only reads device state, so detached factories and contexts share it.
pub(crate) fn build_compute_pipeline(
	device: &ash::Device,
	descriptor_heap_properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
	pipeline_binaries: Option<&PipelineBinaries>,
	shaders: &[Shader],
	builder: crate::pipelines::compute::Builder,
) -> crate::vulkan::ComputePipeline {
	let shader_parameter = builder.shader;
	let parameters = std::slice::from_ref(&shader_parameter);
	let layout = shader_pipeline_layout(shaders, parameters, builder.push_constant_ranges, descriptor_heap_properties);

	let pipeline = with_shader_stages(shaders, parameters, &layout, |stages, identified_stages| {
		let create_info = vk::ComputePipelineCreateInfo::default().layout(vk::PipelineLayout::null());
		let compiled = stages.map(|stages| create_info.stage(stages[0]));
		let identified = identified_stages.map(|stages| create_info.stage(stages[0]));
		create_pipeline(pipeline_binaries, compiled.as_ref(), identified.as_ref(), |create_info| {
			unsafe { device.create_compute_pipelines(vk::PipelineCache::null(), std::slice::from_ref(create_info), None) }
				.map(|pipelines| pipelines[0])
				.map_err(|(_, result)| result)
		})
	})
	.expect("Vulkan descriptor-heap compute pipeline creation failed. The most likely cause is an invalid shader resource mapping or specialization constant.");

	crate::vulkan::ComputePipeline { pipeline, layout }
}
