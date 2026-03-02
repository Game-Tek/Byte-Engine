use std::borrow::Borrow as _;

use crate::rendering::pipelines::visibility::scene_manager::Instance;
use crate::rendering::pipelines::visibility::{
	get_material_count_source, get_material_offset_source, get_pixel_mapping_source, get_visibility_pass_mesh_source,
	INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
	MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, MAX_MESHLETS,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING,
	TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING, VISIBILITY_PASS_FRAGMENT_SOURCE,
};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Viewport};
use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::device::Device as _;
use ghi::frame::Frame;
use ghi::raster_pipeline;
use resource_management::glsl;
use resource_management::resources::material;
use utils::{Box, Extent, RGBA};

#[derive(Clone)]
pub struct VisibilityPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_pipeline: ghi::PipelineHandle,
}

impl VisibilityPass {
	pub fn new(
		device: &mut ghi::Device,
		pipeline_layout: ghi::PipelineLayoutHandle,
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
				ghi::ShaderSource::SPIRV(visibility_mesh_shader_artifact.borrow().into()),
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
				ghi::ShaderSource::SPIRV(visibility_fragment_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Fragment,
				[],
			)
			.expect("Failed to create shader");

		let visibility_pass_shaders = [
			ghi::ShaderParameter::new(&visibility_pass_mesh_shader, ghi::ShaderTypes::Mesh),
			ghi::ShaderParameter::new(&visibility_pass_fragment_shader, ghi::ShaderTypes::Fragment),
		];

		let attachments = [
			ghi::PipelineAttachmentInformation::new(ghi::Formats::U32),
			ghi::PipelineAttachmentInformation::new(ghi::Formats::U32),
			ghi::PipelineAttachmentInformation::new(ghi::Formats::Depth32),
		];

		let vertex_layout = [
			ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let visibility_pass_pipeline = device.create_raster_pipeline(raster_pipeline::Builder::new(
			pipeline_layout,
			&[],
			&visibility_pass_shaders,
			&attachments,
		));

		VisibilityPass {
			pipeline_layout,
			descriptor_set,
			visibility_pass_pipeline,
		}
	}

	pub(super) fn prepare(
		&self,
		frame: &mut ghi::Frame,
		viewport: &Viewport,
		instances: &[Instance],
	) -> impl RenderPassFunction {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let pipeline = self.visibility_pass_pipeline;

		let extent = viewport.extent();
		let instances = instances.iter().copied().collect::<Vec<_>>();

		move |c, t| {
			c.start_region("Visibility Buffer");

			let c = c.start_render_pass(extent, t);

			let c = c.bind_pipeline_layout(pipeline_layout);
			c.bind_descriptor_sets(&[descriptor_set]);
			let c = c.bind_raster_pipeline(pipeline);

			for (i, instance) in instances.iter().enumerate() {
				c.write_push_constant(0, i as u32); // TODO: use actual instance indeces, not loaded meshes indices
				c.dispatch_meshes(instance.meshlet_count, 1, 1);
			}

			c.end_render_pass();

			c.end_region();
		}
	}
}

pub struct MaterialCountPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pipeline: ghi::PipelineHandle,
}

impl MaterialCountPass {
	fn new(
		device: &mut ghi::Device,
		pipeline_layout: ghi::PipelineLayoutHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let material_count_shader_artifact =
			glsl::compile(&get_material_count_source(), "Material Count Pass Compute Shader").unwrap();

		let material_count_shader = device
			.create_shader(
				Some("Material Count Pass Compute Shader"),
				ghi::ShaderSource::SPIRV(material_count_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					MATERIAL_COUNT_BINDING
						.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
					INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				],
			)
			.expect("Failed to create shader");

		let material_count_pipeline = device.create_compute_pipeline(
			pipeline_layout,
			ghi::ShaderParameter::new(&material_count_shader, ghi::ShaderTypes::Compute),
		);

		let material_count_buffer = device.create_buffer(
			Some("Material Count"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
			ghi::DeviceAccesses::HostOnly,
		);

		MaterialCountPass {
			pipeline_layout,
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	fn prepare(&self, frame: &ghi::Frame, viewport: &Viewport) -> impl RenderPassFunction {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.pipeline;
		let material_count_buffer = self.material_count_buffer;

		let extent = viewport.extent();

		move |c, _| {
			c.start_region("Material Count");

			c.clear_buffers(&[material_count_buffer.into()]);

			c.bind_descriptor_sets(&[descriptor_set, visibility_pass_descriptor_set]);
			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

			c.end_region();
		}
	}

	fn get_material_count_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_count_buffer.into()
	}
}

pub struct MaterialOffsetPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	material_offset_pipeline: ghi::PipelineHandle,
}

impl MaterialOffsetPass {
	fn new(
		device: &mut ghi::Device,
		pipeline_layout: ghi::PipelineLayoutHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let material_offset_shader_artifact =
			glsl::compile(&get_material_offset_source(), "Material Offset Pass Compute Shader").unwrap();

		let material_offset_shader = device
			.create_shader(
				Some("Material Offset Pass Compute Shader"),
				ghi::ShaderSource::SPIRV(material_offset_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
					MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					MATERIAL_OFFSET_SCRATCH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
					MATERIAL_EVALUATION_DISPATCHES_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create shader");

		let material_offset_pipeline = device.create_compute_pipeline(
			pipeline_layout,
			ghi::ShaderParameter::new(&material_offset_shader, ghi::ShaderTypes::Compute),
		);

		let material_evaluation_dispatches = device.create_buffer(
			Some("Material Evaluation Dipatches"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect,
			ghi::DeviceAccesses::DeviceOnly,
		);
		let material_offset_buffer = device.create_buffer(
			Some("Material Offset"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
			ghi::DeviceAccesses::DeviceOnly,
		);
		let material_offset_scratch_buffer = device.create_buffer(
			Some("Material Offset Scratch"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
			ghi::DeviceAccesses::DeviceOnly,
		);

		MaterialOffsetPass {
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			pipeline_layout,
			descriptor_set,
			visibility_pass_descriptor_set,
			material_offset_pipeline,
		}
	}

	fn prepare(&self) -> impl RenderPassFunction {
		let pipeline_layout = self.pipeline_layout;
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

			c.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
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
	material_xy: ghi::ImageHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	pixel_mapping_pipeline: ghi::PipelineHandle,
}

impl PixelMappingPass {
	fn new(
		device: &mut ghi::Device,
		pipeline_layout: ghi::PipelineLayoutHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let pixel_mapping_shader_artifact =
			glsl::compile(&get_pixel_mapping_source(), "Pixel Mapping Pass Compute Shader").unwrap();

		let pixel_mapping_shader = device
			.create_shader(
				Some("Pixel Mapping Pass Compute Shader"),
				ghi::ShaderSource::SPIRV(pixel_mapping_shader_artifact.borrow().into()),
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

		let pixel_mapping_pipeline = device.create_compute_pipeline(
			pipeline_layout,
			ghi::ShaderParameter::new(&pixel_mapping_shader, ghi::ShaderTypes::Compute),
		);

		let material_xy = device.build_image(ghi::image::Builder::new(
			ghi::Formats::RG16UNORM,
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
		));

		PixelMappingPass {
			material_xy,
			pipeline_layout,
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	pub(super) fn prepare(&self, frame: &mut ghi::Frame, viewport: &Viewport) -> impl RenderPassFunction {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let pipeline = self.pixel_mapping_pipeline;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;
		let material_xy = self.material_xy;

		let extent = viewport.extent();

		frame.resize_image(material_xy, extent);

		move |c, _| {
			c.start_region("Pixel Mapping");

			c.clear_images(&[(material_xy.into(), ghi::ClearValue::Integer(0, 0, 0, 0))]);

			c.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

			c.end_region();
		}
	}
}

pub struct MaterialEvaluationPass {
	visibility_pipeline_layout: ghi::PipelineLayoutHandle,
	/// Material evaluation pipeline layout
	pipeline_layout: ghi::PipelineLayoutHandle,
	diffuse: ghi::ImageHandle,
	specular: ghi::ImageHandle,
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
		visibility_pipeline_layout: ghi::PipelineLayoutHandle,
		pipeline_layout: ghi::PipelineLayoutHandle,
		diffuse: ghi::ImageHandle,
		specular: ghi::ImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_dispatches: ghi::BufferHandle<[(u32, u32, u32); MAX_MATERIALS]>,
	) -> Self {
		MaterialEvaluationPass {
			visibility_pipeline_layout,
			pipeline_layout,
			diffuse,
			specular,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			material_evaluation_dispatches,
		}
	}

	fn prepare(
		&self,
		frame: &mut ghi::Frame,
		viewport: &Viewport,
		opaque_materials: &[(&str, usize, ghi::PipelineHandle)],
		transparent_materials: &[(&str, usize, ghi::PipelineHandle)],
	) -> impl RenderPassFunction {
		let diffuse = self.diffuse;
		let specular = self.specular;
		let base_descriptor_set = self.base_descriptor_set;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;
		let visibility_pipeline_layout = self.visibility_pipeline_layout;
		let material_evaluation_pipeline_layout = self.pipeline_layout;
		let visibility_descriptor_set = self.visibility_descriptor_set;
		let material_evaluation_descriptor_set = self.descriptor_set;
		let opaque_materials = opaque_materials
			.iter()
			.map(|e| (e.0.to_string(), e.1, e.2))
			.collect::<Vec<_>>();
		let transparent_materials = transparent_materials
			.iter()
			.map(|e| (e.0.to_string(), e.1, e.2))
			.collect::<Vec<_>>();

		move |c, t| {
			c.clear_images(&[
				(diffuse, ghi::ClearValue::Color(RGBA::black())),
				(specular, ghi::ClearValue::Color(RGBA::black())),
			]);

			let c = c.bind_pipeline_layout(visibility_pipeline_layout);

			c.start_region("Material Evaluation");

			c.start_region("Opaque");

			let c = c.bind_pipeline_layout(material_evaluation_pipeline_layout);

			c.write_push_constant(0, 0); // Set view index to 0 (camera)

			for (name, index, pipeline) in &opaque_materials {
				c.start_region(&format!("Material: {}", name));
				c.bind_descriptor_sets(&[
					base_descriptor_set,
					visibility_descriptor_set,
					material_evaluation_descriptor_set,
				]);
				c.write_push_constant(4, *index); // Set material index
				let c = c.bind_compute_pipeline(*pipeline);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();

			c.start_region("Transparent");

			for (name, index, pipeline) in &transparent_materials {
				// TODO: sort by distance to camera
				c.start_region(&format!("Material: {}", name));
				c.bind_descriptor_sets(&[
					base_descriptor_set,
					visibility_descriptor_set,
					material_evaluation_descriptor_set,
				]);
				c.write_push_constant(4, *index); // Set material index
				let c = c.bind_compute_pipeline(*pipeline);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();

			c.end_region();
		}
	}
}

pub struct VisibilityPipelineRenderPass {
	visibility_pass: VisibilityPass,
	material_count_pass: MaterialCountPass,
	material_offset_pass: MaterialOffsetPass,
	pixel_mapping_pass: PixelMappingPass,
	material_evaluation_pass: MaterialEvaluationPass,
}

impl VisibilityPipelineRenderPass {
	pub fn new(
		device: &mut ghi::Device,
		base_pipeline_layout: ghi::PipelineLayoutHandle,
		visibility_pipeline_layout: ghi::PipelineLayoutHandle,
		material_evaluation_pipeline_layout: ghi::PipelineLayoutHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		diffuse: ghi::ImageHandle,
		specular: ghi::ImageHandle,
		depth: ghi::ImageHandle,
		primitive_index: ghi::ImageHandle,
		instance_id: ghi::ImageHandle,
	) -> Self {
		let visibility_pass = VisibilityPass::new(
			device,
			base_pipeline_layout,
			base_descriptor_set,
			primitive_index,
			instance_id,
			depth,
		);
		let material_count_pass = MaterialCountPass::new(
			device,
			visibility_pipeline_layout,
			base_descriptor_set,
			visibility_descriptor_set,
		);
		let material_offset_pass = MaterialOffsetPass::new(
			device,
			visibility_pipeline_layout,
			base_descriptor_set,
			visibility_descriptor_set,
		);
		let pixel_mapping_pass = PixelMappingPass::new(
			device,
			visibility_pipeline_layout,
			base_descriptor_set,
			visibility_descriptor_set,
		);

		let material_evaluation_dispatches = material_offset_pass.material_evaluation_dispatches.clone();

		let material_evaluation_pass = MaterialEvaluationPass::new(
			visibility_pipeline_layout,
			material_evaluation_pipeline_layout,
			diffuse,
			specular,
			base_descriptor_set,
			visibility_descriptor_set,
			material_evaluation_descriptor_set,
			material_evaluation_dispatches,
		);

		Self {
			visibility_pass,
			material_count_pass,
			material_offset_pass,
			pixel_mapping_pass,
			material_evaluation_pass,
		}
	}

	pub(super) fn prepare(
		&self,
		frame: &mut ghi::Frame,
		viewport: &Viewport,
		instances: &[Instance],
	) -> impl RenderPassFunction {
		let visibility_pass = self.visibility_pass.prepare(frame, viewport, instances);
		let material_count_pass = self.material_count_pass.prepare(frame, viewport);
		let material_offset_pass = self.material_offset_pass.prepare();
		let pixel_mapping_pass = self.pixel_mapping_pass.prepare(frame, viewport);
		let material_evaluation_pass = self.material_evaluation_pass.prepare(frame, viewport, &[], &[]);

		move |c, t| {
			c.start_region("Visibility Render Model");

			visibility_pass(c, t);
			material_count_pass(c, t);
			material_offset_pass(c, t);
			pixel_mapping_pass(c, t);
			material_evaluation_pass(c, t);

			c.end_region();
		}
	}
}

// let diffuse_target: ghi::ImageHandle = diffuse_target.into();
// 		let specular_target: ghi::ImageHandle = specular_target.into();
// 		let depth_target: ghi::ImageHandle = depth_target.into();
// 		let primitive_index: ghi::ImageHandle = primitive_index.into();
// 		let instance_id: ghi::ImageHandle = instance_id.into();
// let material_count_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_COUNT_BINDING, material_count_pass.get_material_count_buffer()));
// 		let material_offset_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_BINDING, material_offset_pass.get_material_offset_buffer()));
// 		let material_offset_scratch_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_pass.get_material_offset_scratch_buffer()));
// 		let material_evaluation_dispatches_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_EVALUATION_DISPATCHES_BINDING, material_offset_pass.material_evaluation_dispatches.into()));
// 		let material_xy_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_XY_BINDING, pixel_mapping_pass.material_xy.into()));
// 		let vertex_id_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::image(&TRIANGLE_INDEX_BINDING, primitive_index, ghi::Layouts::General));
// 		let instance_id_binding = device.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::image(&INSTANCE_ID_BINDING, instance_id, ghi::Layouts::General));
// let occlusion_map = device.build_image(ghi::image::Builder::new(ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination).name("Occlusion Map").extent(extent).device_accesses(ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead).use_case(ghi::UseCases::DYNAMIC));
// 		let shadow_map = device.build_image(ghi::image::Builder::new(ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination).name("Shadow Map").extent(extent).device_accesses(ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead).use_case(ghi::UseCases::DYNAMIC).array_layers(NonZeroU32::new(1)));
// let diffuse_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::image(&bindings[0], diffuse_target, ghi::Layouts::General));
// 		let camera_data_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[1], views_data_buffer_handle.into()));
// 		let specular_target_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::image(&bindings[2], specular_target, ghi::Layouts::General));
// 		let light_data_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[4], light_data_buffer.into()));
// 		let materials_data_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[5], materials_data_buffer_handle.into()));
// 		let occlussion_texture_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&bindings[6], occlusion_map, sampler, ghi::Layouts::Read));
// 		let shadow_map_binding = device.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&bindings[7], shadow_map, depth_sampler, ghi::Layouts::Read));
