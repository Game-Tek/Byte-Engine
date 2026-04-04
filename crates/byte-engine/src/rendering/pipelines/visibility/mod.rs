pub mod render_pass;
pub mod scene_manager;
pub mod shader_generator;

use resource_management::{
	glsl_shader_generator::GLSLShaderGenerator, msl_shader_generator::MSLShaderGenerator,
	shader_generator::ShaderGenerationSettings,
};
pub use scene_manager::VisibilityWorldRenderDomain;
use utils::Extent;

use crate::rendering::{
	common_shader_generator::CommonShaderScope, pipelines::visibility::shader_generator::VisibilityShaderScope,
};

/* BASE */
/// Binding to access the views which may be used to render the scene.
pub const VIEWS_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH
		.union(ghi::Stages::FRAGMENT)
		.union(ghi::Stages::RAYGEN)
		.union(ghi::Stages::COMPUTE),
);
pub const MESH_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::FRAGMENT).union(ghi::Stages::COMPUTE),
);
pub const VERTEX_POSITIONS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	2,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const VERTEX_NORMALS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	3,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const VERTEX_UV_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	5,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const VERTEX_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	6,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const PRIMITIVE_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	7,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const MESHLET_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	8,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
);
pub const TEXTURES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(
	9,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
	16,
);

/* Visibility */
pub const MATERIAL_COUNT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_OFFSET_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_XY_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const TRIANGLE_INDEX_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(6, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const INSTANCE_ID_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(7, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

/* Material Evaluation */
pub const OUT_DIFFUSE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const CAMERA: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const OUT_SPECULAR: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const LIGHTING_DATA: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const MATERIALS: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(5, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const AO: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(10, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const DEPTH_SHADOW_MAP: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(11, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;

const MAX_MESHLETS: usize = 1024 * 4;
const MAX_INSTANCES: usize = 1024;
const MAX_MATERIALS: usize = 1024;
const MAX_LIGHTS: usize = 16;
const MAX_TRIANGLES: usize = 65536 * 4;
const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
const MAX_VERTICES: usize = 65536 * 4;
pub const SHADOW_CASCADE_COUNT: usize = 4;
pub const SHADOW_MAP_RESOLUTION: u32 = 2048;

fn build_mesh_program(main: besl::parser::Node<'static>, push_constant: besl::parser::Node<'static>) -> besl::NodeReference {
	let shader = besl::parser::Node::scope("Shader", vec![push_constant, main]);
	let mut root = besl::parser::Node::root();

	root.add(vec![
		CommonShaderScope::new(),
		VisibilityShaderScope::new_with_params(false, false, false, true, false, true, false, false),
		shader,
	]);

	besl::lex(root).unwrap().get_main().unwrap()
}

pub fn get_visibility_pass_mesh_source() -> String {
	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::glsl(
			r#"
		View view = views.views[0];
		process_meshlet(push_constant.instance_index, view.view_projection);
		"#,
			&["View", "views", "push_constant", "process_meshlet"],
			&[],
		)],
	);
	let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
	let main_node = build_mesh_program(main, push_constant);

	GLSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node)
		.unwrap()
}

pub fn get_visibility_pass_mesh_msl_source() -> String {
	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::raw_code(
			Some(
				r#"
		View view = views.views[0];
		process_meshlet(push_constant.instance_index, view.view_projection);
		"#
				.into(),
			),
			Some(
				r#"
		process_meshlet(
			push_constant.instance_index,
			set0.views->views[0].view_projection,
			set0,
			threadgroup_position,
			thread_index,
			out_mesh
		);
		"#
				.into(),
			),
			&["push_constant", "process_meshlet", "views"],
			&[],
		)],
	);
	let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);
	let main_node = build_mesh_program(main, push_constant);

	MSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node)
		.unwrap()
}

pub fn get_shadow_pass_mesh_source() -> String {
	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::glsl(
			r#"
		View view = views.views[push_constant.view_index];
		process_meshlet(push_constant.instance_index, view.view_projection);
		"#,
			&["View", "views", "push_constant", "process_meshlet"],
			&[],
		)],
	);
	let push_constant = besl::parser::Node::push_constant(vec![
		besl::parser::Node::member("instance_index", "u32"),
		besl::parser::Node::member("view_index", "u32"),
	]);
	let main_node = build_mesh_program(main, push_constant);

	GLSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node)
		.unwrap()
}

pub fn get_shadow_pass_mesh_msl_source() -> String {
	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::raw_code(
			Some(
				r#"
		View view = views.views[push_constant.view_index];
		process_meshlet(push_constant.instance_index, view.view_projection);
		"#
				.into(),
			),
			Some(
				r#"
		process_meshlet(
			push_constant.instance_index,
			set0.views->views[push_constant.view_index].view_projection,
			set0,
			threadgroup_position,
			thread_index,
			out_mesh
		);
		"#
				.into(),
			),
			&["push_constant", "process_meshlet", "views"],
			&[],
		)],
	);
	let push_constant = besl::parser::Node::push_constant(vec![
		besl::parser::Node::member("instance_index", "u32"),
		besl::parser::Node::member("view_index", "u32"),
	]);
	let main_node = build_mesh_program(main, push_constant);

	MSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node)
		.unwrap()
}

pub const VISIBILITY_PASS_FRAGMENT_SOURCE_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};

struct FragmentIn {
	VertexOutput vertex;
	PrimitiveOutput primitive;
};

struct FragmentOutput {
	uint primitive_index [[color(0)]];
	uint instance_id [[color(1)]];
};

fragment FragmentOutput visibility_fragment_main(FragmentIn in [[stage_in]]) {
	FragmentOutput out;
	out.primitive_index = in.primitive.primitive_index;
	out.instance_id = in.primitive.instance_index;
	return out;
}
"#;

pub const VISIBILITY_PASS_FRAGMENT_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(fragment)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_mesh_shader: require

layout(location=0) perprimitiveEXT flat in uint in_instance_index;
layout(location=1) perprimitiveEXT flat in uint in_primitive_index;

layout(location=0) out uint out_primitive_index;
layout(location=1) out uint out_instance_id;

void main() {
	out_primitive_index = in_primitive_index;
	out_instance_id = in_instance_index;
}
"#;

pub fn get_material_count_source() -> String {
	let main_code = r#"
	// If thread is out of bound respect to the material_id texture, return
	ivec2 extent = imageSize(instance_index_render_target);
	if (gl_GlobalInvocationID.x >= extent.x || gl_GlobalInvocationID.y >= extent.y) { return; }

	uint pixel_instance_index = imageLoad(instance_index_render_target, ivec2(gl_GlobalInvocationID.xy)).r;

	if (pixel_instance_index == 0xFFFFFFFF) { return; }

	uint material_index = meshes.meshes[pixel_instance_index].material_index;

	atomicAdd(material_count.material_count[material_index], 1);
	"#;

	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::glsl(
			main_code,
			&["meshes", "material_count", "instance_index_render_target"],
			&[],
		)],
	);

	let shader = besl::parser::Node::scope("Shader", vec![main]);

	let mut root = besl::parser::Node::root();

	root.add(vec![
		CommonShaderScope::new(),
		VisibilityShaderScope::new_with_params(false, false, false, true, false, true, false, false),
		shader,
	]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.get_main().unwrap();

	let glsl = GLSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node)
		.unwrap();

	glsl
}

pub fn get_material_offset_source() -> String {
	let main_code = r#"
	uint sum = 0;

	for (uint i = 0; i < 1024; i++) { /* 1024 is the maximum number of materials */
		material_offset.material_offset[i] = sum;
		material_offset_scratch.material_offset_scratch[i] = sum;
		material_evaluation_dispatches.material_evaluation_dispatches[i] = uvec3((material_count.material_count[i] + 127) / 128, 1, 1);
		sum += material_count.material_count[i];
	}
	"#;

	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::glsl(
			main_code,
			&[
				"material_offset",
				"material_offset_scratch",
				"material_count",
				"material_evaluation_dispatches",
			],
			&[],
		)],
	);

	let shader = besl::parser::Node::scope("Shader", vec![main]);

	let mut root = besl::parser::Node::root();

	root.add(vec![
		CommonShaderScope::new(),
		VisibilityShaderScope::new_with_params(false, false, false, true, false, true, false, false),
		shader,
	]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.get_main().unwrap();

	let glsl = GLSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::compute(Extent::square(1)), &main_node)
		.unwrap();

	glsl
}

pub fn get_pixel_mapping_source() -> String {
	let main_code = r#"
	ivec2 extent = imageSize(instance_index_render_target);
	// If thread is out of bound respect to the material_id texture, return
	if (gl_GlobalInvocationID.x >= extent.x || gl_GlobalInvocationID.y >= extent.y) { return; }

	uint pixel_instance_index = imageLoad(instance_index_render_target, ivec2(gl_GlobalInvocationID.xy)).r;

	if (pixel_instance_index == 0xFFFFFFFF) { return; }

	uint material_index = meshes.meshes[pixel_instance_index].material_index;

	uint offset = atomicAdd(material_offset_scratch.material_offset_scratch[material_index], 1);

	pixel_mapping.pixel_mapping[offset] = u16vec2(gl_GlobalInvocationID.xy);
	"#;

	let main = besl::parser::Node::function(
		"main",
		Vec::new(),
		"void",
		vec![besl::parser::Node::glsl(
			main_code,
			&[
				"meshes",
				"material_offset_scratch",
				"pixel_mapping",
				"instance_index_render_target",
			],
			&[],
		)],
	);

	let mut root = besl::parser::Node::root();

	let shader = besl::parser::Node::scope("Shader", vec![main]);

	root.add(vec![
		CommonShaderScope::new(),
		VisibilityShaderScope::new_with_params(false, false, false, true, false, true, false, false),
		shader,
	]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.get_main().unwrap();

	let glsl = GLSLShaderGenerator::new()
		.generate(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node)
		.unwrap();

	glsl
}
