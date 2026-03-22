use std::borrow::Borrow as _;

use crate::rendering::pipelines::visibility::scene_manager::Instance;
use crate::rendering::pipelines::visibility::{
	get_material_count_source, get_material_offset_source, get_pixel_mapping_source, get_shadow_pass_mesh_source,
	get_visibility_pass_mesh_source, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING,
	MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS,
	MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING,
	PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING,
	VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
	VISIBILITY_PASS_FRAGMENT_SOURCE,
};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Viewport};
use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::device::{Device as _, DeviceCreate as _};
use ghi::frame::Frame as _;
use ghi::graphics_hardware_interface::ImageHandleLike as _;
use ghi::implementation::Frame;
use resource_management::glsl;
use resource_management::resources::material;
use utils::{Box, Extent, RGBA};

#[derive(Clone)]
pub struct VisibilityPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_pipeline: ghi::PipelineHandle,
	attachments: [ghi::AttachmentInformation; 3],
}

impl VisibilityPass {
	pub fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		primitive_index: ghi::ImageHandle,
		instance_id: ghi::ImageHandle,
		depth_target: ghi::ImageHandle,
	) -> Self {
		let visibility_shader = get_visibility_pass_mesh_source();

		let visibility_mesh_shader_artifact = glsl::compile(&visibility_shader, "Visibility Mesh Shader").unwrap();

		let visibility_pass_mesh_shader = device
			.create_shader(
				Some("Visibility Pass Mesh Shader"),
				ghi::shader::Sources::SPIRV(visibility_mesh_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Mesh,
				[
					VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				],
			)
			.expect("Failed to create shader");

		let visibility_fragment_shader_artifact =
			glsl::compile(VISIBILITY_PASS_FRAGMENT_SOURCE, "Visibility Fragment Shader").unwrap();

		let visibility_pass_fragment_shader = device
			.create_shader(
				Some("Visibility Pass Fragment Shader"),
				ghi::shader::Sources::SPIRV(visibility_fragment_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create shader");

		let visibility_pass_shaders = [
			ghi::ShaderParameter::new(&visibility_pass_mesh_shader, ghi::ShaderTypes::Mesh),
			ghi::ShaderParameter::new(&visibility_pass_fragment_shader, ghi::ShaderTypes::Fragment),
		];

		let attachments = [
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::U32),
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::U32),
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32),
		];

		let vertex_layout = [
			ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::pipelines::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let visibility_pass_pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[base_descriptor_set_layout],
			&[ghi::pipelines::PushConstantRange::new(0, 4)],
			&vertex_layout,
			&visibility_pass_shaders,
			&attachments,
		));

		VisibilityPass {
			descriptor_set,
			visibility_pass_pipeline,
			attachments: [
				ghi::AttachmentInformation::new(
					primitive_index,
					ghi::Formats::U32,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Formats::U32,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
					ghi::Formats::Depth32,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				),
			],
		}
	}

	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		viewport: &Viewport,
		instances: &[Instance],
	) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let pipeline = self.visibility_pass_pipeline;
		let attachments = self.attachments;

		let extent = viewport.extent();
		let instances = instances.iter().copied().collect::<Vec<_>>();

		move |c, _| {
			c.start_region("Visibility Buffer");

			let c = c.start_render_pass(extent, &attachments);

			let c = c.bind_raster_pipeline(pipeline);
			c.bind_descriptor_sets(&[descriptor_set]);

			for (i, instance) in instances.iter().enumerate() {
				c.write_push_constant(0, i as u32); // TODO: use actual instance indeces, not loaded meshes indices
				c.dispatch_meshes(instance.meshlet_count, 1, 1);
			}

			c.end_render_pass();

			c.end_region();
		}
	}
}

pub struct ShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	shadow_pass_pipeline: ghi::PipelineHandle,
	shadow_map: ghi::DynamicImageHandle,
}

impl ShadowPass {
	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		shadow_map: ghi::DynamicImageHandle,
	) -> Self {
		let shadow_shader = get_shadow_pass_mesh_source();
		let shadow_mesh_shader_artifact = glsl::compile(&shadow_shader, "Shadow Mesh Shader").unwrap();

		let shadow_pass_mesh_shader = device
			.create_shader(
				Some("Shadow Pass Mesh Shader"),
				ghi::shader::Sources::SPIRV(shadow_mesh_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Mesh,
				[
					VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				],
			)
			.expect("Failed to create shader");

		let attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32)];
		let vertex_layout = [
			ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::pipelines::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let shadow_pass_pipeline = device.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[base_descriptor_set_layout],
			&[ghi::pipelines::PushConstantRange::new(0, 8)],
			&vertex_layout,
			&[ghi::ShaderParameter::new(&shadow_pass_mesh_shader, ghi::ShaderTypes::Mesh)],
			&attachments,
		));

		Self {
			descriptor_set,
			shadow_pass_pipeline,
			shadow_map,
		}
	}

	fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		instances: &[Instance],
		shadow_enabled: bool,
	) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let pipeline = self.shadow_pass_pipeline;
		let shadow_map = self.shadow_map;
		let extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let instances = instances.iter().copied().collect::<Vec<_>>();

		if shadow_enabled {
			frame.resize_image(shadow_map, extent);
		}

		move |c, _| {
			if !shadow_enabled {
				return;
			}

			c.start_region("Shadow Map");

			for cascade in 0..SHADOW_CASCADE_COUNT {
				c.start_region(&format!("Cascade {}", cascade));

				let attachments = [ghi::AttachmentInformation::new(
					shadow_map,
					ghi::Formats::Depth32,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layer(cascade as u32)];

				let c = c.start_render_pass(extent, &attachments);
				let c = c.bind_raster_pipeline(pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);

				c.write_push_constant(4, (cascade + 1) as u32);

				for (i, instance) in instances.iter().enumerate() {
					c.write_push_constant(0, i as u32);
					c.dispatch_meshes(instance.meshlet_count, 1, 1);
				}

				c.end_render_pass();
				c.end_region();
			}

			c.end_region();
		}
	}
}

pub struct MaterialCountPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pipeline: ghi::PipelineHandle,
}

impl MaterialCountPass {
	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	) -> Self {
		let material_count_shader_artifact =
			glsl::compile(&get_material_count_source(), "Material Count Pass Compute Shader").unwrap();

		let material_count_shader = device
			.create_shader(
				Some("Material Count Pass Compute Shader"),
				ghi::shader::Sources::SPIRV(material_count_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					MATERIAL_COUNT_BINDING
						.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
					INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				],
			)
			.expect("Failed to create shader");

		let material_count_pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, visibility_descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&material_count_shader, ghi::ShaderTypes::Compute),
		));

		MaterialCountPass {
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	fn prepare(&self, frame: &ghi::implementation::Frame, viewport: &Viewport) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.pipeline;
		let material_count_buffer = self.material_count_buffer;

		let extent = viewport.extent();

		move |c, _| {
			c.start_region("Material Count");

			c.clear_buffers(&[material_count_buffer.into()]);

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_pass_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

			c.end_region();
		}
	}

	fn get_material_count_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_count_buffer.into()
	}
}

pub struct MaterialOffsetPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	material_offset_pipeline: ghi::PipelineHandle,
}

impl MaterialOffsetPass {
	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	) -> Self {
		let material_offset_shader_artifact =
			glsl::compile(&get_material_offset_source(), "Material Offset Pass Compute Shader").unwrap();

		let material_offset_shader = device
			.create_shader(
				Some("Material Offset Pass Compute Shader"),
				ghi::shader::Sources::SPIRV(material_offset_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
					MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					MATERIAL_OFFSET_SCRATCH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					MATERIAL_EVALUATION_DISPATCHES_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create shader");

		let material_offset_pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, visibility_descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&material_offset_shader, ghi::ShaderTypes::Compute),
		));

		MaterialOffsetPass {
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			descriptor_set,
			visibility_pass_descriptor_set,
			material_offset_pipeline,
		}
	}

	fn prepare(&self) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let visibility_passes_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.material_offset_pipeline;
		let material_offset_buffer = self.material_offset_buffer;
		let material_offset_scratch_buffer = self.material_offset_scratch_buffer;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;

		move |c, _| {
			c.start_region("Material Offset");

			c.clear_buffers(&[
				material_offset_buffer.into(),
				material_offset_scratch_buffer.into(),
				material_evaluation_dispatches.into(),
			]);

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::line(1), Extent::line(1)));
			c.end_region();
		}
	}

	fn get_material_offset_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_buffer.into()
	}

	fn get_material_offset_scratch_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_scratch_buffer.into()
	}
}

pub struct PixelMappingPass {
	material_xy: ghi::BufferHandle<[(u16, u16); 2073600]>,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	pixel_mapping_pipeline: ghi::PipelineHandle,
}

impl PixelMappingPass {
	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
		material_xy: ghi::BufferHandle<[(u16, u16); 2073600]>,
	) -> Self {
		let pixel_mapping_shader_artifact =
			glsl::compile(&get_pixel_mapping_source(), "Pixel Mapping Pass Compute Shader").unwrap();

		let pixel_mapping_shader = device
			.create_shader(
				Some("Pixel Mapping Pass Compute Shader"),
				ghi::shader::Sources::SPIRV(pixel_mapping_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					MATERIAL_OFFSET_SCRATCH_BINDING
						.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
					INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
					MATERIAL_XY_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create shader");

		let pixel_mapping_pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, visibility_descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&pixel_mapping_shader, ghi::ShaderTypes::Compute),
		));

		PixelMappingPass {
			material_xy,
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	pub(super) fn prepare(&self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let pipeline = self.pixel_mapping_pipeline;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;
		let material_xy = self.material_xy;

		let extent = viewport.extent();

		move |c, _| {
			c.start_region("Pixel Mapping");

			c.clear_buffers(&[material_xy.into()]);

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

			c.end_region();
		}
	}
}

pub struct MaterialEvaluationPass {
	diffuse: ghi::ImageHandle,
	specular: ghi::ImageHandle,
	ao_map: ghi::DynamicImageHandle,
	/// Base layout descriptor set
	base_descriptor_set: ghi::DescriptorSetHandle,
	/// Visibility passes descriptor set
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// Material evaluation descriptor set
	descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	fn new(
		diffuse: ghi::ImageHandle,
		specular: ghi::ImageHandle,
		ao_map: ghi::DynamicImageHandle,
		_shadow_map: ghi::DynamicImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	) -> Self {
		MaterialEvaluationPass {
			diffuse,
			specular,
			ao_map,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			material_evaluation_dispatches,
		}
	}

	fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		viewport: &Viewport,
		opaque_materials: &[(String, u32, ghi::PipelineHandle)],
		transparent_materials: &[(String, u32, ghi::PipelineHandle)],
	) -> impl RenderPassFunction {
		let diffuse = self.diffuse;
		let specular = self.specular;
		let ao_map = self.ao_map;
		let base_descriptor_set = self.base_descriptor_set;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;
		let visibility_descriptor_set = self.visibility_descriptor_set;
		let material_evaluation_descriptor_set = self.descriptor_set;
		let opaque_materials = opaque_materials.to_vec();
		let transparent_materials = transparent_materials.to_vec();
		frame.resize_image(ao_map, viewport.extent());

		move |c, t| {
			c.clear_images(&[
				(diffuse, ghi::ClearValue::Color(RGBA::black())),
				(specular, ghi::ClearValue::Color(RGBA::black())),
				(ao_map.into_image_handle(), ghi::ClearValue::Color(RGBA::white())),
			]);

			c.start_region("Material Evaluation");

			c.start_region("Opaque");

			for (name, index, pipeline) in &opaque_materials {
				c.start_region(&format!("Material: {}", name));
				let c = c.bind_compute_pipeline(*pipeline);
				c.bind_descriptor_sets(&[
					base_descriptor_set,
					visibility_descriptor_set,
					material_evaluation_descriptor_set,
				]);
				c.write_push_constant(0, *index);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();

			c.start_region("Transparent");

			for (name, index, pipeline) in &transparent_materials {
				// TODO: sort by distance to camera
				c.start_region(&format!("Material: {}", name));
				let c = c.bind_compute_pipeline(*pipeline);
				c.bind_descriptor_sets(&[
					base_descriptor_set,
					visibility_descriptor_set,
					material_evaluation_descriptor_set,
				]);
				c.write_push_constant(0, *index);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();

			c.end_region();
		}
	}
}

pub struct VisibilityPipelineRenderPass {
	shadow_pass: ShadowPass,
	visibility_pass: VisibilityPass,
	material_count_pass: MaterialCountPass,
	material_offset_pass: MaterialOffsetPass,
	pixel_mapping_pass: PixelMappingPass,
	material_evaluation_pass: MaterialEvaluationPass,
}

impl VisibilityPipelineRenderPass {
	pub fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		diffuse: ghi::ImageHandle,
		specular: ghi::ImageHandle,
		ao_map: ghi::DynamicImageHandle,
		shadow_map: ghi::DynamicImageHandle,
		depth: ghi::ImageHandle,
		primitive_index: ghi::ImageHandle,
		instance_id: ghi::ImageHandle,
		material_xy: ghi::BufferHandle<[(u16, u16); 2073600]>,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	) -> Self {
		let shadow_pass = ShadowPass::new(device, base_descriptor_set_layout, base_descriptor_set, shadow_map);
		let visibility_pass = VisibilityPass::new(
			device,
			base_descriptor_set_layout,
			base_descriptor_set,
			primitive_index,
			instance_id,
			depth,
		);
		let material_count_pass = MaterialCountPass::new(
			device,
			base_descriptor_set_layout,
			visibility_descriptor_set_layout,
			base_descriptor_set,
			visibility_descriptor_set,
			material_count_buffer,
		);
		let material_offset_pass = MaterialOffsetPass::new(
			device,
			base_descriptor_set_layout,
			visibility_descriptor_set_layout,
			base_descriptor_set,
			visibility_descriptor_set,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);
		let pixel_mapping_pass = PixelMappingPass::new(
			device,
			base_descriptor_set_layout,
			visibility_descriptor_set_layout,
			base_descriptor_set,
			visibility_descriptor_set,
			material_xy,
		);

		let material_evaluation_dispatches = material_offset_pass.material_evaluation_dispatches.clone();

		let material_evaluation_pass = MaterialEvaluationPass::new(
			diffuse,
			specular,
			ao_map,
			shadow_map,
			base_descriptor_set,
			visibility_descriptor_set,
			material_evaluation_descriptor_set,
			material_evaluation_dispatches,
		);

		Self {
			shadow_pass,
			visibility_pass,
			material_count_pass,
			material_offset_pass,
			pixel_mapping_pass,
			material_evaluation_pass,
		}
	}

	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		viewport: &Viewport,
		instances: &[Instance],
		opaque_materials: &[(String, u32, ghi::PipelineHandle)],
		transparent_materials: &[(String, u32, ghi::PipelineHandle)],
		shadow_enabled: bool,
	) -> impl RenderPassFunction {
		let shadow_pass = self.shadow_pass.prepare(frame, instances, shadow_enabled);
		let visibility_pass = self.visibility_pass.prepare(frame, viewport, instances);
		let material_count_pass = self.material_count_pass.prepare(frame, viewport);
		let material_offset_pass = self.material_offset_pass.prepare();
		let pixel_mapping_pass = self.pixel_mapping_pass.prepare(frame, viewport);
		let material_evaluation_pass =
			self.material_evaluation_pass
				.prepare(frame, viewport, opaque_materials, transparent_materials);

		move |c, t| {
			c.start_region("Visibility Render Model");

			shadow_pass(c, t);
			visibility_pass(c, t);
			material_count_pass(c, t);
			material_offset_pass(c, t);
			pixel_mapping_pass(c, t);
			material_evaluation_pass(c, t);

			c.end_region();
		}
	}
}
