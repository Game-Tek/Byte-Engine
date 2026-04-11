use std::borrow::Borrow as _;

use crate::rendering::pipelines::visibility::scene_manager::Instance;
use crate::rendering::pipelines::visibility::{
	get_gtao_bitfield_blur_x_shader, get_gtao_bitfield_shader, get_gtao_blur_shader, get_gtao_shader,
	get_material_count_msl_source, get_material_count_source, get_material_offset_msl_source, get_material_offset_source,
	get_pixel_mapping_msl_source, get_pixel_mapping_source, get_shadow_pass_mesh_msl_source, get_shadow_pass_mesh_source,
	get_visibility_pass_mesh_msl_source, get_visibility_pass_mesh_source, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING,
	MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING,
	MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES,
	MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT,
	SHADOW_MAP_RESOLUTION, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING,
	VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING, VISIBILITY_PASS_FRAGMENT_SOURCE,
	VISIBILITY_PASS_FRAGMENT_SOURCE_MSL,
};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Viewport};
use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::device::{Device as _, DeviceCreate as _};
use ghi::frame::Frame as _;
use ghi::implementation::Frame;
use math::Vector2;
use resource_management::glsl;
use resource_management::resources::material;
use utils::{Box, Extent, RGBA};

const GTAO_DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const GTAO_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const GTAO_BLUR_DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const GTAO_BLUR_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const GTAO_BLUR_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

const GTAO_USE_BITFIELD_BINARY_IMPL: bool = false;
const GTAO_PACKED_WORD_BITS: u32 = 32;

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
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		depth_target: ghi::BaseImageHandle,
	) -> Self {
		let visibility_pass_mesh_shader = if ghi::implementation::USES_METAL {
			let visibility_shader = get_visibility_pass_mesh_msl_source();

			device
				.create_shader(
					Some("Visibility Pass Mesh Shader"),
					ghi::shader::Sources::MTL {
						source: visibility_shader.as_str(),
						entry_point: "besl_main",
					},
					ghi::ShaderTypes::Mesh,
					[
						VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					],
				)
				.expect("Failed to create shader")
		} else {
			let visibility_shader = get_visibility_pass_mesh_source();
			let visibility_mesh_shader_artifact = glsl::compile(&visibility_shader, "Visibility Mesh Shader").unwrap();

			device
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
						PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					],
				)
				.expect("Failed to create shader")
		};

		let visibility_pass_fragment_shader = if ghi::implementation::USES_METAL {
			device
				.create_shader(
					Some("Visibility Pass Fragment Shader"),
					ghi::shader::Sources::MTL {
						source: VISIBILITY_PASS_FRAGMENT_SOURCE_MSL,
						entry_point: "visibility_fragment_main",
					},
					ghi::ShaderTypes::Fragment,
					[],
				)
				.expect("Failed to create shader")
		} else {
			let visibility_fragment_shader_artifact =
				glsl::compile(VISIBILITY_PASS_FRAGMENT_SOURCE, "Visibility Fragment Shader").unwrap();

			device
				.create_shader(
					Some("Visibility Pass Fragment Shader"),
					ghi::shader::Sources::SPIRV(visibility_fragment_shader_artifact.borrow().into()),
					ghi::ShaderTypes::Fragment,
					[],
				)
				.expect("Failed to create shader")
		};

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
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
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
		_frame: &mut ghi::implementation::Frame,
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
	shadow_map: ghi::BaseImageHandle,
}

impl ShadowPass {
	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		shadow_map: ghi::BaseImageHandle,
	) -> Self {
		let shadow_pass_mesh_shader = if ghi::implementation::USES_METAL {
			let shadow_shader = get_shadow_pass_mesh_msl_source();

			device
				.create_shader(
					Some("Shadow Pass Mesh Shader"),
					ghi::shader::Sources::MTL {
						source: shadow_shader.as_str(),
						entry_point: "besl_main",
					},
					ghi::ShaderTypes::Mesh,
					[
						VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					],
				)
				.expect("Failed to create shader")
		} else {
			let shadow_shader = get_shadow_pass_mesh_source();
			let shadow_mesh_shader_artifact = glsl::compile(&shadow_shader, "Shadow Mesh Shader").unwrap();

			device
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
				.expect("Failed to create shader")
		};

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
			frame.resize_image(shadow_map.into(), extent);
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
		let material_count_shader = if ghi::implementation::USES_METAL {
			device
				.create_shader(
					Some("Material Count Pass Compute Shader"),
					ghi::shader::Sources::MTL {
						source: get_material_count_msl_source(),
						entry_point: "besl_main",
					},
					ghi::ShaderTypes::Compute,
					[
						MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MATERIAL_COUNT_BINDING
							.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
						INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
					],
				)
				.expect("Failed to create shader")
		} else {
			let material_count_shader_artifact =
				glsl::compile(&get_material_count_source(), "Material Count Pass Compute Shader").unwrap();

			device
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
				.expect("Failed to create shader")
		};

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
	material_evaluation_dispatches: ghi::BufferHandle<[[u32; 4]; MAX_MATERIALS]>,
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
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 4]; MAX_MATERIALS]>,
	) -> Self {
		let material_offset_shader = if ghi::implementation::USES_METAL {
			device
				.create_shader(
					Some("Material Offset Pass Compute Shader"),
					ghi::shader::Sources::MTL {
						source: get_material_offset_msl_source(),
						entry_point: "besl_main",
					},
					ghi::ShaderTypes::Compute,
					[
						MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
						MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
						MATERIAL_OFFSET_SCRATCH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
						MATERIAL_EVALUATION_DISPATCHES_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					],
				)
				.expect("Failed to create shader")
		} else {
			let material_offset_shader_artifact =
				glsl::compile(&get_material_offset_source(), "Material Offset Pass Compute Shader").unwrap();

			device
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
				.expect("Failed to create shader")
		};

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
	material_xy: ghi::BufferHandle<[(u16, u16); MAX_PIXEL_MAPPING_ENTRIES]>,
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
		material_xy: ghi::BufferHandle<[(u16, u16); MAX_PIXEL_MAPPING_ENTRIES]>,
	) -> Self {
		let pixel_mapping_shader = if ghi::implementation::USES_METAL {
			let pixel_mapping_shader_source = get_pixel_mapping_msl_source();

			device
				.create_shader(
					Some("Pixel Mapping Pass Compute Shader"),
					ghi::shader::Sources::MTL {
						source: pixel_mapping_shader_source.as_str(),
						entry_point: "besl_main",
					},
					ghi::ShaderTypes::Compute,
					[
						MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
						MATERIAL_OFFSET_SCRATCH_BINDING
							.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
						INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
						MATERIAL_XY_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					],
				)
				.expect("Failed to create shader")
		} else {
			let pixel_mapping_shader_source = get_pixel_mapping_source();
			let pixel_mapping_shader_artifact =
				glsl::compile(&pixel_mapping_shader_source, "Pixel Mapping Pass Compute Shader").unwrap();

			device
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
				.expect("Failed to create shader")
		};

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

/// The `GtaoPass` struct builds a depth-based ambient occlusion term before material evaluation shades the frame.
pub struct GtaoPass {
	base_descriptor_set: ghi::DescriptorSetHandle,
	gtao_descriptor_set: ghi::DescriptorSetHandle,
	blur_descriptor_set_x: ghi::DescriptorSetHandle,
	blur_descriptor_set_y: ghi::DescriptorSetHandle,
	gtao_pipeline: ghi::PipelineHandle,
	blur_pipeline_x: ghi::PipelineHandle,
	blur_pipeline_y: ghi::PipelineHandle,
	ao_map: ghi::BaseImageHandle,
	temp_ao_map: ghi::DynamicImageHandle,
	packed_ao_map: Option<ghi::DynamicImageHandle>,
}

impl GtaoPass {
	fn packed_extent(extent: Extent) -> Extent {
		Extent::rectangle(extent.width().div_ceil(GTAO_PACKED_WORD_BITS), extent.height())
	}

	fn new(
		device: &mut ghi::implementation::Device,
		base_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
	) -> Self {
		let descriptor_set_layout =
			device.create_descriptor_set_template(Some("GTAO Descriptor Set"), &[GTAO_DEPTH_BINDING, GTAO_OUTPUT_BINDING]);
		let gtao_descriptor_set = device.create_descriptor_set(Some("GTAO Descriptor Set"), &descriptor_set_layout);
		let blur_descriptor_set_layout = device.create_descriptor_set_template(
			Some("GTAO Blur Descriptor Set"),
			&[GTAO_BLUR_DEPTH_BINDING, GTAO_BLUR_SOURCE_BINDING, GTAO_BLUR_OUTPUT_BINDING],
		);
		let blur_descriptor_set_x =
			device.create_descriptor_set(Some("GTAO Blur X Descriptor Set"), &blur_descriptor_set_layout);
		let blur_descriptor_set_y =
			device.create_descriptor_set(Some("GTAO Blur Y Descriptor Set"), &blur_descriptor_set_layout);
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let ao_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let temp_ao_map = device.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Blur Intermediate")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let packed_ao_map = GTAO_USE_BITFIELD_BINARY_IMPL.then(|| {
			device.build_dynamic_image(
				ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::Storage | ghi::Uses::Image)
					.name("GTAO Packed AO")
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
		});
		let gtao_output = packed_ao_map.map(|e| e.into()).unwrap_or(ao_map);
		let blur_source_x = packed_ao_map.map(|e| e.into()).unwrap_or(ao_map);

		let _ = device.create_descriptor_binding(
			gtao_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&GTAO_DEPTH_BINDING,
				depth,
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			gtao_descriptor_set,
			ghi::BindingConstructor::image(&GTAO_OUTPUT_BINDING, gtao_output),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_x,
			ghi::BindingConstructor::combined_image_sampler(
				&GTAO_BLUR_DEPTH_BINDING,
				depth,
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_x,
			ghi::BindingConstructor::combined_image_sampler(
				&GTAO_BLUR_SOURCE_BINDING,
				blur_source_x,
				ao_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_x,
			ghi::BindingConstructor::image(&GTAO_BLUR_OUTPUT_BINDING, temp_ao_map),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_y,
			ghi::BindingConstructor::combined_image_sampler(&GTAO_BLUR_DEPTH_BINDING, depth, depth_sampler, ghi::Layouts::Read),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_y,
			ghi::BindingConstructor::combined_image_sampler(
				&GTAO_BLUR_SOURCE_BINDING,
				temp_ao_map,
				ao_sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			blur_descriptor_set_y,
			ghi::BindingConstructor::image(&GTAO_BLUR_OUTPUT_BINDING, ao_map),
		);

		let gtao_shader_bindings = [
			VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			GTAO_DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			GTAO_OUTPUT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
		];
		let gtao_shader = if GTAO_USE_BITFIELD_BINARY_IMPL {
			let gtao_shader = get_gtao_bitfield_shader();
			if gtao_shader.language().is_glsl() {
				let artifact = glsl::compile(gtao_shader.source(), "GTAO Pass Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Pass Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						gtao_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Pass Compute Shader"),
						ghi::shader::Sources::MTL {
							source: gtao_shader.source(),
							entry_point: gtao_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						gtao_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		} else {
			let gtao_shader = get_gtao_shader();
			if gtao_shader.language().is_glsl() {
				let artifact = glsl::compile(gtao_shader.source(), "GTAO Pass Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Pass Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						gtao_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Pass Compute Shader"),
						ghi::shader::Sources::MTL {
							source: gtao_shader.source(),
							entry_point: gtao_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						gtao_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		};

		let gtao_pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&gtao_shader, ghi::ShaderTypes::Compute),
		));

		let blur_shader_bindings = [
			VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			GTAO_BLUR_DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			GTAO_BLUR_SOURCE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			GTAO_BLUR_OUTPUT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
		];
		let blur_x_shader = if GTAO_USE_BITFIELD_BINARY_IMPL {
			let blur_shader = get_gtao_bitfield_blur_x_shader();
			if blur_shader.language().is_glsl() {
				let artifact = glsl::compile(blur_shader.source(), "GTAO Blur X Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Blur X Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Blur X Compute Shader"),
						ghi::shader::Sources::MTL {
							source: blur_shader.source(),
							entry_point: blur_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		} else {
			let blur_shader = get_gtao_blur_shader();
			if blur_shader.language().is_glsl() {
				let artifact = glsl::compile(blur_shader.source(), "GTAO Blur X Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Blur X Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Blur X Compute Shader"),
						ghi::shader::Sources::MTL {
							source: blur_shader.source(),
							entry_point: blur_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		};
		let blur_y_shader = if GTAO_USE_BITFIELD_BINARY_IMPL {
			let blur_shader = get_gtao_blur_shader();
			if blur_shader.language().is_glsl() {
				let artifact = glsl::compile(blur_shader.source(), "GTAO Blur Y Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Blur Y Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Blur Y Compute Shader"),
						ghi::shader::Sources::MTL {
							source: blur_shader.source(),
							entry_point: blur_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		} else {
			let blur_shader = get_gtao_blur_shader();
			if blur_shader.language().is_glsl() {
				let artifact = glsl::compile(blur_shader.source(), "GTAO Blur Y Compute Shader").unwrap();
				device
					.create_shader(
						Some("GTAO Blur Y Compute Shader"),
						ghi::shader::Sources::SPIRV(artifact.borrow().into()),
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			} else {
				device
					.create_shader(
						Some("GTAO Blur Y Compute Shader"),
						ghi::shader::Sources::MTL {
							source: blur_shader.source(),
							entry_point: blur_shader.entry_point(),
						},
						ghi::ShaderTypes::Compute,
						blur_shader_bindings,
					)
					.expect("Failed to create shader")
			}
		};

		let blur_pipeline_x = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, blur_descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&blur_x_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[
				ghi::pipelines::SpecializationMapEntry::new(0, "vec2f".to_string(), Vector2::new(1.0f32, 0.0f32)),
			]),
		));
		let blur_pipeline_y = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[base_descriptor_set_layout, blur_descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&blur_y_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[
				ghi::pipelines::SpecializationMapEntry::new(0, "vec2f".to_string(), Vector2::new(0.0f32, 1.0f32)),
			]),
		));

		Self {
			base_descriptor_set,
			gtao_descriptor_set,
			blur_descriptor_set_x,
			blur_descriptor_set_y,
			gtao_pipeline,
			blur_pipeline_x,
			blur_pipeline_y,
			ao_map,
			temp_ao_map,
			packed_ao_map,
		}
	}

	fn prepare(&self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> impl RenderPassFunction {
		let base_descriptor_set = self.base_descriptor_set;
		let gtao_descriptor_set = self.gtao_descriptor_set;
		let blur_descriptor_set_x = self.blur_descriptor_set_x;
		let blur_descriptor_set_y = self.blur_descriptor_set_y;
		let gtao_pipeline = self.gtao_pipeline;
		let blur_pipeline_x = self.blur_pipeline_x;
		let blur_pipeline_y = self.blur_pipeline_y;
		let ao_map = self.ao_map;
		let temp_ao_map = self.temp_ao_map;
		let packed_ao_map = self.packed_ao_map;
		let extent = viewport.extent();

		frame.resize_image(ao_map.into(), extent);
		frame.resize_image(temp_ao_map.into(), extent);

		if let Some(packed_ao_map) = packed_ao_map {
			frame.resize_image(packed_ao_map.into(), Self::packed_extent(extent));
		}

		move |c, _| {
			c.start_region("GTAO");
			if let Some(packed_ao_map) = packed_ao_map {
				c.clear_images(&[(packed_ao_map.into(), ghi::ClearValue::Color(RGBA::black()))]);
			} else {
				c.clear_images(&[(ao_map.into(), ghi::ClearValue::Color(RGBA::white()))]);
			}

			{
				let c = c.bind_compute_pipeline(gtao_pipeline);
				c.bind_descriptor_sets(&[base_descriptor_set, gtao_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			}

			{
				let c = c.bind_compute_pipeline(blur_pipeline_x);
				c.bind_descriptor_sets(&[base_descriptor_set, blur_descriptor_set_x]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			}

			{
				let c = c.bind_compute_pipeline(blur_pipeline_y);
				c.bind_descriptor_sets(&[base_descriptor_set, blur_descriptor_set_y]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			}

			c.end_region();
		}
	}
}

pub struct MaterialEvaluationPass {
	diffuse: ghi::BaseImageHandle,
	specular: ghi::BaseImageHandle,
	ao_map: ghi::BaseImageHandle,
	ibl_cubemap: ghi::BaseImageHandle,
	/// Base layout descriptor set
	base_descriptor_set: ghi::DescriptorSetHandle,
	/// Visibility passes descriptor set
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// Material evaluation descriptor set
	descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_dispatches: ghi::BufferHandle<[[u32; 4]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	fn new(
		diffuse: ghi::BaseImageHandle,
		specular: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		_shadow_map: ghi::BaseImageHandle,
		ibl_cubemap: ghi::BaseImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 4]; MAX_MATERIALS]>,
	) -> Self {
		MaterialEvaluationPass {
			diffuse,
			specular,
			ao_map,
			ibl_cubemap,
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
		let ibl_cubemap = self.ibl_cubemap;
		let base_descriptor_set = self.base_descriptor_set;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;
		let visibility_descriptor_set = self.visibility_descriptor_set;
		let material_evaluation_descriptor_set = self.descriptor_set;
		let opaque_materials = opaque_materials.to_vec();
		let transparent_materials = transparent_materials.to_vec();

		frame.resize_image(ao_map.into(), viewport.extent());

		move |c, t| {
			c.clear_images(&[
				(diffuse.into(), ghi::ClearValue::Color(RGBA::black())),
				(specular.into(), ghi::ClearValue::Color(RGBA::black())),
				(ao_map.into(), ghi::ClearValue::Color(RGBA::white())),
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
	gtao_pass: GtaoPass,
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
		diffuse: ghi::BaseImageHandle,
		specular: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		shadow_map: ghi::BaseImageHandle,
		ibl_cubemap: ghi::BaseImageHandle,
		depth: ghi::BaseImageHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		material_xy: ghi::BufferHandle<[(u16, u16); MAX_PIXEL_MAPPING_ENTRIES]>,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 4]; MAX_MATERIALS]>,
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
		let gtao_pass = GtaoPass::new(device, base_descriptor_set_layout, base_descriptor_set, depth, ao_map);

		let material_evaluation_dispatches = material_offset_pass.material_evaluation_dispatches.clone();

		let material_evaluation_pass = MaterialEvaluationPass::new(
			diffuse,
			specular,
			ao_map,
			shadow_map,
			ibl_cubemap,
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
			gtao_pass,
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
		let gtao_pass = self.gtao_pass.prepare(frame, viewport);
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
			gtao_pass(c, t);
			material_evaluation_pass(c, t);

			c.end_region();
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn gtao_shader_compiles() {
		if super::GTAO_USE_BITFIELD_BINARY_IMPL {
			let gtao_shader = super::get_gtao_bitfield_shader();
			if gtao_shader.language().is_glsl() {
				resource_management::glsl::compile(gtao_shader.source(), "GTAO Pass Compute Shader").unwrap();
				return;
			}
		}

		let gtao_shader = if super::GTAO_USE_BITFIELD_BINARY_IMPL {
			super::get_gtao_bitfield_shader()
		} else {
			super::get_gtao_shader()
		};
		if gtao_shader.language().is_glsl() {
			resource_management::glsl::compile(gtao_shader.source(), "GTAO Pass Compute Shader").unwrap();
			return;
		}

		use ghi::device::DeviceCreate as _;

		if !ghi::implementation::USES_METAL {
			return;
		}

		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the GTAO shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::COMPUTE), &mut queue)],
			)
			.expect("Expected a Metal device for the GTAO shader test");

		let shader_handle = device.create_shader(
			Some("GTAO Compute Shader"),
			ghi::shader::Sources::MTL {
				source: gtao_shader.source(),
				entry_point: gtao_shader.entry_point(),
			},
			ghi::ShaderTypes::Compute,
			[
				super::VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				super::GTAO_DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				super::GTAO_OUTPUT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			],
		);

		assert!(
			shader_handle.is_ok(),
			"Expected generated GTAO source to compile for the active backend"
		);
	}

	#[test]
	fn gtao_blur_shader_compiles() {
		if super::GTAO_USE_BITFIELD_BINARY_IMPL {
			resource_management::glsl::compile(
				super::get_gtao_bitfield_blur_x_shader().source(),
				"GTAO Blur X Compute Shader",
			)
			.unwrap();
			resource_management::glsl::compile(super::get_gtao_blur_shader().source(), "GTAO Blur Y Compute Shader").unwrap();
			return;
		}

		let blur_shader = super::get_gtao_blur_shader();
		if blur_shader.language().is_glsl() {
			resource_management::glsl::compile(blur_shader.source(), "GTAO Blur Compute Shader").unwrap();
			return;
		}

		use ghi::device::DeviceCreate as _;

		if !ghi::implementation::USES_METAL {
			return;
		}

		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the GTAO blur shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::COMPUTE), &mut queue)],
			)
			.expect("Expected a Metal device for the GTAO blur shader test");

		let shader_handle = device.create_shader(
			Some("GTAO Blur Compute Shader"),
			ghi::shader::Sources::MTL {
				source: blur_shader.source(),
				entry_point: blur_shader.entry_point(),
			},
			ghi::ShaderTypes::Compute,
			[
				super::VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				super::GTAO_BLUR_DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				super::GTAO_BLUR_SOURCE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				super::GTAO_BLUR_OUTPUT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			],
		);

		assert!(
			shader_handle.is_ok(),
			"Expected generated GTAO blur source to compile for the active backend"
		);
	}

	#[test]
	fn gtao_view_space_reconstruction_z_is_positive() {
		use math::{mat::MatInverse as _, Matrix4, Vector3, Vector4};

		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = proj.inverse();

		// Simulate what the GTAO shader does: reconstruct positions for center + neighbors
		// at various depths, compute the normal, and check its direction

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vector3 {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vector4::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			let w = view.w;
			Vector3::new(view.x / w, view.y / w, view.z / w)
		};

		// Project a known view-space point to get its depth
		let project_to_depth = |vx: f32, vy: f32, vz: f32| -> f32 {
			let clip = proj * Vector4::new(vx, vy, vz, 1.0);
			clip.z / clip.w // ndc depth
		};

		// Test at different distances
		for vz in [0.5f32, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0] {
			let depth = project_to_depth(0.0, 0.0, vz);
			let center_px = extent_x / 2;
			let center_py = extent_y / 2;

			let center = reconstruct(center_px, center_py, depth);
			let right = reconstruct(center_px + 1, center_py, depth);
			let left = reconstruct(center_px - 1, center_py, depth);
			let top = reconstruct(center_px, center_py - 1, depth);
			let bottom = reconstruct(center_px, center_py + 1, depth);

			// min_diff for horizontal: pick shorter of (right - center) or (center - left)
			let ap_h = Vector3::new(right.x - center.x, right.y - center.y, right.z - center.z);
			let bp_h = Vector3::new(center.x - left.x, center.y - left.y, center.z - left.z);
			let h_diff = if math::dot(ap_h, ap_h) < math::dot(bp_h, bp_h) {
				ap_h
			} else {
				bp_h
			};

			// min_diff for vertical: pick shorter of (top - center) or (center - bottom)
			let ap_v = Vector3::new(top.x - center.x, top.y - center.y, top.z - center.z);
			let bp_v = Vector3::new(center.x - bottom.x, center.y - bottom.y, center.z - bottom.z);
			let v_diff = if math::dot(ap_v, ap_v) < math::dot(bp_v, bp_v) {
				ap_v
			} else {
				bp_v
			};

			let normal = math::cross(h_diff, v_diff);
			let normal_len = math::length(normal);
			let normal = if normal_len > 1e-8 {
				Vector3::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vector3::new(0.0, 0.0, 1.0)
			};

			// The shader enforces camera-facing: if dot(normal, center_position) > 0, flip.
			// In view space the camera is at origin, so center_position IS the view direction to the point.
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vector3::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"vz={:.1}: center=({:.4},{:.4},{:.4}), normal=({:.4},{:.4},{:.4}), depth={:.6}",
				vz, center.x, center.y, center.z, normal.x, normal.y, normal.z, depth
			);

			// The normal must face toward the camera, i.e. dot(normal, center_position) <= 0.
			// For a flat surface perpendicular to Z: normal.z should be dominant and negative.
			let dot_check = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			assert!(
				dot_check <= 0.0,
				"Normal should face camera (dot(normal, center_position) <= 0) at vz={}, got dot={}",
				vz,
				dot_check
			);
			assert!(
				normal.z.abs() > 0.99,
				"Normal Z should be dominant for flat surface perpendicular to Z at vz={}, got normal.z={}",
				vz,
				normal.z
			);
		}
	}

	/// Simulates the GTAO normal reconstruction on a floor plane (Y=constant)
	/// where depth varies per pixel, and checks for normal sign flips at different distances.
	#[test]
	fn gtao_normal_on_floor_plane() {
		use math::{mat::MatInverse as _, Matrix4, Vector3, Vector4};

		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = proj.inverse();

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vector3 {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vector4::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			Vector3::new(view.x / view.w, view.y / view.w, view.z / view.w)
		};

		let project = |vx: f32, vy: f32, vz: f32| -> (f32, f32, f32) {
			let clip = proj * Vector4::new(vx, vy, vz, 1.0);
			let ndc_x = clip.x / clip.w;
			let ndc_y = clip.y / clip.w;
			let depth = clip.z / clip.w;
			// Inverse of: ndc_x = uv_x * 2 - 1, ndc_y = 1 - uv_y * 2
			let uv_x = (ndc_x + 1.0) / 2.0;
			let uv_y = (1.0 - ndc_y) / 2.0;
			let px = uv_x * extent_x as f32 - 0.5;
			let py = uv_y * extent_y as f32 - 0.5;
			(px, py, depth)
		};

		// Floor plane at Y = -1 (camera looks along +Z, floor is below camera)
		// For a given pixel, we need to find where the ray through that pixel hits Y=-1
		let floor_y = -1.0f32;

		// For a pixel (px, py), reconstruct a ray direction in view space:
		// The ray goes from origin through the point at depth=1 (arbitrary)
		let ray_hit_floor = |px: i32, py: i32| -> Option<(f32, f32)> {
			// Reconstruct view-space direction using depth=0.5 (arbitrary non-zero)
			let p = reconstruct(px, py, 0.5);
			// Ray: origin=(0,0,0), direction=p (normalized doesn't matter, just need ratio)
			// Hit Y=floor_y: t = floor_y / p.y
			if p.y.abs() < 1e-8 {
				return None;
			} // ray parallel to floor
			let t = floor_y / p.y;
			if t <= 0.0 {
				return None;
			} // floor behind camera
			let hit_z = p.z * t;
			if hit_z < near || hit_z > far {
				return None;
			} // outside clip range
	 // Project hit point to get depth
			let hit_x = p.x * t;
			let clip = proj * Vector4::new(hit_x, floor_y, hit_z, 1.0);
			Some((hit_z, clip.z / clip.w))
		};

		let min_diff = |p: Vector3, a: Vector3, b: Vector3| -> Vector3 {
			let ap = Vector3::new(a.x - p.x, a.y - p.y, a.z - p.z);
			let bp = Vector3::new(p.x - b.x, p.y - b.y, p.z - b.z);
			if math::dot(ap, ap) < math::dot(bp, bp) {
				ap
			} else {
				bp
			}
		};

		eprintln!("\n--- Floor plane normal reconstruction ---");
		eprintln!("Testing at various screen Y positions (floor at Y={}):", floor_y);

		let mut found_flip = false;

		// Test across different screen rows (different distances to floor)
		for py in (extent_y / 2 + 50..extent_y - 10).step_by(50) {
			let px = extent_x / 2; // screen center X

			let Some((center_vz, center_depth)) = ray_hit_floor(px, py) else {
				continue;
			};
			let Some((_, left_depth)) = ray_hit_floor(px - 1, py) else {
				continue;
			};
			let Some((_, right_depth)) = ray_hit_floor(px + 1, py) else {
				continue;
			};
			let Some((_, top_depth)) = ray_hit_floor(px, py - 1) else {
				continue;
			};
			let Some((_, bottom_depth)) = ray_hit_floor(px, py + 1) else {
				continue;
			};

			let center = reconstruct(px, py, center_depth);
			let left = reconstruct(px - 1, py, left_depth);
			let right = reconstruct(px + 1, py, right_depth);
			let top = reconstruct(px, py - 1, top_depth);
			let bottom = reconstruct(px, py + 1, bottom_depth);

			let h_diff = min_diff(center, right, left);
			let v_diff = min_diff(center, top, bottom);

			let normal = math::cross(h_diff, v_diff);
			let normal_len = math::length(normal);
			let normal = if normal_len > 1e-8 {
				Vector3::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vector3::new(0.0, 0.0, 1.0)
			};

			// Apply camera-facing check (same as shader)
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vector3::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"py={:4}, vz={:6.2}: h_diff=({:+.6},{:+.6},{:+.6}), v_diff=({:+.6},{:+.6},{:+.6}), normal=({:+.4},{:+.4},{:+.4})",
				py, center_vz, h_diff.x, h_diff.y, h_diff.z, v_diff.x, v_diff.y, v_diff.z, normal.x, normal.y, normal.z,
			);

			// For a floor plane at Y=-1, the normal should point +Y (up, toward camera if cam is above floor)
			if normal.y < 0.0 {
				found_flip = true;
				eprintln!("  ^^^ FLIPPED! Normal Y is negative (pointing into floor)");
			}
		}

		if found_flip {
			eprintln!("\nWARNING: Normal flipped at some distances! This explains the hard boundary.");
		} else {
			eprintln!("\nAll normals consistent (no flip detected in tested range).");
		}
	}
}
