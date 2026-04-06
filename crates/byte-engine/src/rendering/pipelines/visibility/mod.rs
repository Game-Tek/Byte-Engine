pub mod render_pass;
pub mod scene_manager;
pub mod shader_generator;

use resource_management::{
	glsl_shader_generator::GLSLShaderGenerator,
	platform_shader_generator::GeneratedPlatformShader,
	platform_shader_generator::{PlatformShaderGenerator, PlatformShaderLanguage},
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

const MESH_OUTPUT_TYPES_MSL: &str = r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#;

fn build_mesh_program_from_source(source: &'static str, push_constant: besl::parser::Node<'static>) -> besl::NodeReference {
	let mut shader_source = besl::parse(source).unwrap();
	let shader_children = match shader_source.node_mut() {
		besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
		_ => panic!(
			"Mesh shader source must parse into a scope. The most likely cause is invalid BESL syntax in the mesh shader source."
		),
	};

	let mut shader_nodes = vec![push_constant];
	shader_nodes.extend(shader_children);

	let shader = besl::parser::Node::scope("Shader", shader_nodes);
	let mut root = besl::parser::Node::root();

	root.add(vec![
		CommonShaderScope::new(),
		VisibilityShaderScope::new_with_params(false, false, false, true, false, true, false, false),
		shader,
	]);

	besl::lex(root).unwrap().get_main().unwrap()
}

fn generate_mesh_source_for_language(
	source: &'static str,
	push_constant: besl::parser::Node<'static>,
	language: PlatformShaderLanguage,
) -> String {
	let main_node = build_mesh_program_from_source(source, push_constant);
	let mut shader_generator = PlatformShaderGenerator::new();
	let generated = shader_generator
		.generate_for_language(
			language,
			&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)),
			&main_node,
		)
		.unwrap()
		.into_source();

	if language == PlatformShaderLanguage::Msl && !generated.contains("struct VertexOutput") {
		return generated.replacen(
			"using namespace metal;",
			&format!("using namespace metal;\n{}", MESH_OUTPUT_TYPES_MSL),
			1,
		);
	}

	generated
}

pub fn get_visibility_pass_mesh_source() -> String {
	let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);

	generate_mesh_source_for_language(
		r#"
		main: fn () -> void {
			let view: View = views.views[0];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
		push_constant,
		PlatformShaderLanguage::Glsl,
	)
}

pub fn get_visibility_pass_mesh_msl_source() -> String {
	format!(
		r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(mesh)
// besl-threadgroup-size:128,1,1

{mesh_outputs}

struct PushConstant {{
	uint instance_index;
}};

struct View {{
	float4x4 view;
	float4x4 projection;
	float4x4 view_projection;
	float4x4 inverse_view;
	float4x4 inverse_projection;
	float4x4 inverse_view_projection;
	float2 fov;
	float near;
	float far;
}};

struct Mesh {{
	float4x4 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
}};

struct Meshlet {{
	ushort primitive_offset;
	ushort triangle_offset;
	uchar primitive_count;
	uchar triangle_count;
}};

struct _views {{
	View views[8];
}};

struct _meshes {{
	Mesh meshes[64];
}};

struct _vertex_positions {{
	float3 positions[8192];
}};

struct _vertex_normals {{
	float3 normals[8192];
}};

struct _vertex_uvs {{
	float2 uvs[8192];
}};

struct _vertex_indices {{
	ushort vertex_indices[8192];
}};

struct _primitive_indices {{
	uchar primitive_indices[8192];
}};

struct _meshlets {{
	Meshlet meshlets[8192];
}};

struct _set0 {{
	constant _views* views [[id(0)]];
	constant _meshes* meshes [[id(1)]];
	constant _vertex_positions* vertex_positions [[id(2)]];
	constant _vertex_normals* vertex_normals [[id(3)]];
	constant _vertex_uvs* vertex_uvs [[id(4)]];
	constant _vertex_indices* vertex_indices [[id(5)]];
	constant _primitive_indices* primitive_indices [[id(6)]];
	constant _meshlets* meshlets [[id(7)]];
}};

[[mesh]] void besl_main(
	constant PushConstant& push_constant [[buffer(15)]],
	constant _set0& set0 [[buffer(16)]],
	uint threadgroup_position [[threadgroup_position_in_grid]],
	uint thread_index [[thread_index_in_threadgroup]],
	metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh
) {{
	Mesh mesh = set0.meshes->meshes[push_constant.instance_index];
	View view = set0.views->views[0];
	uint meshlet_index = threadgroup_position + mesh.base_meshlet_index;
	Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];
	uint primitive_index = thread_index;

	if (thread_index == 0) {{
		out_mesh.set_primitive_count(uint(meshlet.triangle_count));
	}}

	if (primitive_index < uint(meshlet.primitive_count)) {{
		uint vertex_index = mesh.base_vertex_index
			+ uint(set0.vertex_indices->vertex_indices[mesh.base_primitive_index + uint(meshlet.primitive_offset) + primitive_index]);
		float4 position = float4(set0.vertex_positions->positions[vertex_index], 1.0);
		out_mesh.set_vertex(primitive_index, VertexOutput{{ .position = view.view_projection * mesh.model * position }});
	}}

	if (primitive_index < uint(meshlet.triangle_count)) {{
		uint triangle_base_index = mesh.base_triangle_index + uint(meshlet.triangle_offset) + primitive_index;
		out_mesh.set_index(primitive_index * 3 + 0, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 0]));
		out_mesh.set_index(primitive_index * 3 + 1, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 1]));
		out_mesh.set_index(primitive_index * 3 + 2, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 2]));
		out_mesh.set_primitive(
			primitive_index,
			PrimitiveOutput{{ .instance_index = push_constant.instance_index, .primitive_index = (meshlet_index << 8) | (primitive_index & 255) }}
		);
	}}
}}
"#,
		mesh_outputs = MESH_OUTPUT_TYPES_MSL,
	)
}

pub fn get_shadow_pass_mesh_source() -> String {
	let push_constant = besl::parser::Node::push_constant(vec![
		besl::parser::Node::member("instance_index", "u32"),
		besl::parser::Node::member("view_index", "u32"),
	]);

	generate_mesh_source_for_language(
		r#"
		main: fn () -> void {
			let view_index: u32 = push_constant.view_index;
			let view: View = views.views[view_index];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
		push_constant,
		PlatformShaderLanguage::Glsl,
	)
}

pub fn get_shadow_pass_mesh_msl_source() -> String {
	format!(
		r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(mesh)
// besl-threadgroup-size:128,1,1

{mesh_outputs}

struct PushConstant {{
	uint instance_index;
	uint view_index;
}};

struct View {{
	float4x4 view;
	float4x4 projection;
	float4x4 view_projection;
	float4x4 inverse_view;
	float4x4 inverse_projection;
	float4x4 inverse_view_projection;
	float2 fov;
	float near;
	float far;
}};

struct Mesh {{
	float4x4 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
}};

struct Meshlet {{
	ushort primitive_offset;
	ushort triangle_offset;
	uchar primitive_count;
	uchar triangle_count;
}};

struct _views {{
	View views[8];
}};

struct _meshes {{
	Mesh meshes[64];
}};

struct _vertex_positions {{
	float3 positions[8192];
}};

struct _vertex_normals {{
	float3 normals[8192];
}};

struct _vertex_uvs {{
	float2 uvs[8192];
}};

struct _vertex_indices {{
	ushort vertex_indices[8192];
}};

struct _primitive_indices {{
	uchar primitive_indices[8192];
}};

struct _meshlets {{
	Meshlet meshlets[8192];
}};

struct _set0 {{
	constant _views* views [[id(0)]];
	constant _meshes* meshes [[id(1)]];
	constant _vertex_positions* vertex_positions [[id(2)]];
	constant _vertex_normals* vertex_normals [[id(3)]];
	constant _vertex_uvs* vertex_uvs [[id(4)]];
	constant _vertex_indices* vertex_indices [[id(5)]];
	constant _primitive_indices* primitive_indices [[id(6)]];
	constant _meshlets* meshlets [[id(7)]];
}};

[[mesh]] void besl_main(
	constant PushConstant& push_constant [[buffer(15)]],
	constant _set0& set0 [[buffer(16)]],
	uint threadgroup_position [[threadgroup_position_in_grid]],
	uint thread_index [[thread_index_in_threadgroup]],
	metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh
) {{
	Mesh mesh = set0.meshes->meshes[push_constant.instance_index];
	View view = set0.views->views[push_constant.view_index];
	uint meshlet_index = threadgroup_position + mesh.base_meshlet_index;
	Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];
	uint primitive_index = thread_index;

	if (thread_index == 0) {{
		out_mesh.set_primitive_count(uint(meshlet.triangle_count));
	}}

	if (primitive_index < uint(meshlet.primitive_count)) {{
		uint vertex_index = mesh.base_vertex_index
			+ uint(set0.vertex_indices->vertex_indices[mesh.base_primitive_index + uint(meshlet.primitive_offset) + primitive_index]);
		float4 position = float4(set0.vertex_positions->positions[vertex_index], 1.0);
		out_mesh.set_vertex(primitive_index, VertexOutput{{ .position = view.view_projection * mesh.model * position }});
	}}

	if (primitive_index < uint(meshlet.triangle_count)) {{
		uint triangle_base_index = mesh.base_triangle_index + uint(meshlet.triangle_offset) + primitive_index;
		out_mesh.set_index(primitive_index * 3 + 0, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 0]));
		out_mesh.set_index(primitive_index * 3 + 1, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 1]));
		out_mesh.set_index(primitive_index * 3 + 2, uint(set0.primitive_indices->primitive_indices[triangle_base_index * 3 + 2]));
		out_mesh.set_primitive(
			primitive_index,
			PrimitiveOutput{{ .instance_index = push_constant.instance_index, .primitive_index = (meshlet_index << 8) | (primitive_index & 255) }}
		);
	}}
}}
"#,
		mesh_outputs = MESH_OUTPUT_TYPES_MSL,
	)
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

pub fn get_material_count_msl_source() -> &'static str {
	r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(compute)
// Note: Metal threadgroup sizes are set on the pipeline state.

struct Mesh {
	float4x4 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
};

struct _meshes {
	Mesh meshes[64];
};

struct _material_count {
	atomic_uint material_count[1024];
};

struct _set0 {
	constant _meshes* meshes [[id(1)]];
};

struct _set1 {
	device _material_count* material_count [[id(0)]];
	texture2d<uint, access::read> instance_index_render_target [[id(7)]];
};

kernel void besl_main(uint2 gid [[thread_position_in_grid]], constant _set0& set0 [[buffer(16)]], constant _set1& set1 [[buffer(17)]]) {
	uint width = set1.instance_index_render_target.get_width();
	uint height = set1.instance_index_render_target.get_height();
	if (gid.x >= width || gid.y >= height) { return; }

	uint pixel_instance_index = set1.instance_index_render_target.read(gid).x;
	if (pixel_instance_index == 0xFFFFFFFFu) { return; }

	uint material_index = set0.meshes->meshes[pixel_instance_index].material_index;
	atomic_fetch_add_explicit(&set1.material_count->material_count[material_index], 1, memory_order_relaxed);
}
"#
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

pub fn get_material_offset_msl_source() -> &'static str {
	r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(compute)
// Note: Metal threadgroup sizes are set on the pipeline state.

struct _material_count {
	atomic_uint material_count[1024];
};

struct _material_offset {
	uint material_offset[1024];
};

struct _material_offset_scratch {
	uint material_offset_scratch[1024];
};

struct _material_evaluation_dispatches {
	uint3 material_evaluation_dispatches[1024];
};

struct _set1 {
	device _material_count* material_count [[id(0)]];
	device _material_offset* material_offset [[id(1)]];
	device _material_offset_scratch* material_offset_scratch [[id(2)]];
	device _material_evaluation_dispatches* material_evaluation_dispatches [[id(3)]];
};

kernel void besl_main(uint2 gid [[thread_position_in_grid]], constant _set1& set1 [[buffer(17)]]) {
	if (gid.x != 0 || gid.y != 0) { return; }

	uint sum = 0;
	for (uint i = 0; i < 1024; i++) {
		uint count = atomic_load_explicit(&set1.material_count->material_count[i], memory_order_relaxed);
		set1.material_offset->material_offset[i] = sum;
		set1.material_offset_scratch->material_offset_scratch[i] = sum;
		set1.material_evaluation_dispatches->material_evaluation_dispatches[i] = uint3((count + 127) / 128, 1, 1);
		sum += count;
	}
}
"#
}

pub fn get_pixel_mapping_source() -> String {
	get_pixel_mapping_shader().into_source()
}

pub fn get_pixel_mapping_shader() -> GeneratedPlatformShader {
	generate_pixel_mapping_shader_for_language(PlatformShaderLanguage::current_platform())
}

fn generate_pixel_mapping_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	let main_node = build_pixel_mapping_program();
	let mut shader_generator = PlatformShaderGenerator::new();

	shader_generator
		.generate_for_language(language, &ShaderGenerationSettings::compute(Extent::square(32)), &main_node)
		.unwrap()
}

fn build_pixel_mapping_program() -> besl::NodeReference {
	let source = r#"
	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(instance_index_render_target, coord);
		let pixel_instance_index: u32 = image_load_u32(instance_index_render_target, coord);

		if (pixel_instance_index < 4294967295) {
			let material_index: u32 = mesh_data.meshes[pixel_instance_index].material_index;
			pixel_mapping_buffer.pixel_mapping[atomic_add(material_offset_scratch_buffer.material_offset_scratch[material_index], 1)] = vec2u16(coord.x, coord.y);
		}
	}
	"#;

	besl::compile_to_besl(source, Some(build_pixel_mapping_root()))
		.unwrap()
		.get_main()
		.unwrap()
}

fn build_pixel_mapping_root() -> besl::Node {
	let mut root = besl::Node::root();
	let u32_t = root.get_child("u32").unwrap();
	let texture_2d = root.get_child("Texture2D").unwrap();
	let vec2u_t = root.get_child("vec2u").unwrap();
	let vec2u16_t = root.get_child("vec2u16").unwrap();
	let mesh_material_index = besl::Node::member("material_index", u32_t.clone()).into();
	let mesh = root.add_child(besl::Node::r#struct("Mesh", vec![mesh_material_index]).into());
	let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
	let meshes_member = besl::Node::array("meshes", mesh, 64);
	let material_offset_scratch_member = besl::Node::array("material_offset_scratch", atomic_u32.clone(), 2073600);
	let pixel_mapping_member = besl::Node::array("pixel_mapping", vec2u16_t, 2073600);

	root.add_children(vec![
		besl::Node::binding(
			"mesh_data",
			besl::BindingTypes::Buffer {
				members: vec![meshes_member.clone()],
			},
			0,
			1,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"material_offset_scratch_buffer",
			besl::BindingTypes::Buffer {
				members: vec![material_offset_scratch_member.clone()],
			},
			1,
			2,
			true,
			true,
		)
		.into(),
		besl::Node::binding(
			"pixel_mapping_buffer",
			besl::BindingTypes::Buffer {
				members: vec![pixel_mapping_member.clone()],
			},
			1,
			4,
			false,
			true,
		)
		.into(),
		besl::Node::binding(
			"instance_index_render_target",
			besl::BindingTypes::Image {
				format: "r32ui".to_string(),
			},
			1,
			7,
			true,
			false,
		)
		.into(),
	]);

	let image_load_u32 = root.add_child(besl::Node::intrinsic("image_load_u32", Vec::new(), u32_t.clone()).into());
	image_load_u32.borrow_mut().add_children(vec![
		besl::Node::new(besl::Nodes::Parameter {
			name: "image".to_string(),
			r#type: texture_2d.clone(),
		})
		.into(),
		besl::Node::new(besl::Nodes::Parameter {
			name: "coord".to_string(),
			r#type: vec2u_t.clone(),
		})
		.into(),
	]);

	let atomic_add = root.add_child(besl::Node::intrinsic("atomic_add", Vec::new(), u32_t.clone()).into());
	atomic_add.borrow_mut().add_children(vec![
		besl::Node::new(besl::Nodes::Parameter {
			name: "value".to_string(),
			r#type: atomic_u32,
		})
		.into(),
		besl::Node::new(besl::Nodes::Parameter {
			name: "increment".to_string(),
			r#type: u32_t.clone(),
		})
		.into(),
	]);
	root
}

pub fn get_gtao_blur_shader() -> GeneratedPlatformShader {
	generate_gtao_blur_shader_for_language(PlatformShaderLanguage::current_platform())
}

pub(crate) fn generate_gtao_blur_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	let main_node = build_gtao_blur_program();
	let mut shader_generator = PlatformShaderGenerator::new();

	shader_generator
		.generate_for_language(language, &ShaderGenerationSettings::compute(Extent::square(8)), &main_node)
		.unwrap()
}

fn build_gtao_blur_program() -> besl::NodeReference {
	let source = r#"
	GTAO_BLUR_RADIUS: const u32 = 4;
	GTAO_BLUR_SPATIAL_WEIGHTS: const f32[5] = f32[5](
		0.2270270270,
		0.1945945946,
		0.1216216216,
		0.0540540541,
		0.0162162162
	);
	GTAO_BLUR_BASE_RELATIVE_SIGMA: const f32 = 0.03;
	GTAO_BLUR_MIN_RELATIVE_SIGMA: const f32 = 0.004;
	GTAO_BLUR_SIGMA_VARIANCE_SCALE: const f32 = 32.0;
	GTAO_BLUR_VARIANCE_BLEND_SCALE: const f32 = 96.0;

	clamp_subtract: fn (value: u32, offset: u32) -> u32 {
		if (value < offset) {
			return 0;
		}

		return value - offset;
	}

	clamp_add: fn (value: u32, offset: u32, extent: u32) -> u32 {
		let maximum: u32 = extent - 1;
		if (value + offset > maximum) {
			return maximum;
		}

		return value + offset;
	}

	linear_view_depth: fn (depth: f32) -> f32 {
		let clip_space: vec4f = vec4f(0.0, 0.0, depth, 1.0);
		let view_space: vec4f = views.views[0].inverse_projection * clip_space;
		return max(view_space.z / view_space.w, 0.0001);
	}

	relative_depth_delta: fn (center_linear_depth: f32, sample_depth: f32) -> f32 {
		let sample_linear_depth: f32 = linear_view_depth(sample_depth);
		return (sample_linear_depth - center_linear_depth) / max(center_linear_depth, 0.0001);
	}

	blur_sample_coordinate: fn (pixel: vec2u, extent: vec2u, offset: u32, negative: u32) -> vec2u {
		if (blur_direction.x > 0.0) {
			if (negative == 1) {
				return vec2u(clamp_subtract(pixel.x, offset), pixel.y);
			}

			return vec2u(clamp_add(pixel.x, offset, extent.x), pixel.y);
		}

		if (negative == 1) {
			return vec2u(pixel.x, clamp_subtract(pixel.y, offset));
		}

		return vec2u(pixel.x, clamp_add(pixel.y, offset, extent.y));
	}

	neighbor_depth: fn (
		pixel: vec2u,
		extent: vec2u,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32
	) -> f32 {
		let sample_x: u32 = clamp_add(pixel.x, offset_x, extent.x);
		let sample_y: u32 = clamp_add(pixel.y, offset_y, extent.y);
		if (negative_x == 1) {
			sample_x = clamp_subtract(pixel.x, offset_x);
		}
		if (negative_y == 1) {
			sample_y = clamp_subtract(pixel.y, offset_y);
		}
		let sample_coord: vec2u = vec2u(sample_x, sample_y);
		return fetch(visibility_depth, sample_coord).x;
	}

	neighbor_stats: fn (
		pixel: vec2u,
		extent: vec2u,
		center_linear_depth: f32,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32
	) -> vec3f {
		let sample_depth: f32 = neighbor_depth(pixel, extent, offset_x, offset_y, negative_x, negative_y);
		if (sample_depth == 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}

		let delta: f32 = relative_depth_delta(center_linear_depth, sample_depth);
		return vec3f(delta, delta * delta, 1.0);
	}

	compute_local_depth_variance: fn (pixel: vec2u, extent: vec2u, center_linear_depth: f32) -> f32 {
		let sample_00: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 1);
		let sample_01: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 1);
		let sample_02: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 1);
		let sample_10: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 1, 0);
		let sample_11: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 0, 0, 0);
		let sample_12: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 0, 0);
		let sample_20: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 0);
		let sample_21: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 0);
		let sample_22: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 0);

		let mean: f32 = sample_00.x;
		mean = mean + sample_01.x;
		mean = mean + sample_02.x;
		mean = mean + sample_10.x;
		mean = mean + sample_11.x;
		mean = mean + sample_12.x;
		mean = mean + sample_20.x;
		mean = mean + sample_21.x;
		mean = mean + sample_22.x;

		let mean_sq: f32 = sample_00.y;
		mean_sq = mean_sq + sample_01.y;
		mean_sq = mean_sq + sample_02.y;
		mean_sq = mean_sq + sample_10.y;
		mean_sq = mean_sq + sample_11.y;
		mean_sq = mean_sq + sample_12.y;
		mean_sq = mean_sq + sample_20.y;
		mean_sq = mean_sq + sample_21.y;
		mean_sq = mean_sq + sample_22.y;

		let sample_count: f32 = sample_00.z;
		sample_count = sample_count + sample_01.z;
		sample_count = sample_count + sample_02.z;
		sample_count = sample_count + sample_10.z;
		sample_count = sample_count + sample_11.z;
		sample_count = sample_count + sample_12.z;
		sample_count = sample_count + sample_20.z;
		sample_count = sample_count + sample_21.z;
		sample_count = sample_count + sample_22.z;

		if (sample_count <= 1.0) {
			return 0.0;
		}

		let normalized_mean: f32 = mean / sample_count;
		let normalized_mean_sq: f32 = mean_sq / sample_count;
		return max(normalized_mean_sq - normalized_mean * normalized_mean, 0.0);
	}

	compute_bilateral_weight: fn (relative_depth_difference: f32, local_depth_variance: f32) -> f32 {
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let sigma: f32 = max(
			GTAO_BLUR_MIN_RELATIVE_SIGMA,
			GTAO_BLUR_BASE_RELATIVE_SIGMA / (1.0 + local_depth_stddev * GTAO_BLUR_SIGMA_VARIANCE_SCALE)
		);
		return exp(-(relative_depth_difference * relative_depth_difference) / max(2.0 * sigma * sigma, 0.000001));
	}

	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(ao_output, coord);
		let extent: vec2u = image_size(ao_output);

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
			write(ao_output, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			return;
		}

		let center_ao: f32 = fetch(ao_source, coord).x;
		let center_linear_depth: f32 = linear_view_depth(center_depth);
		let local_depth_variance: f32 = compute_local_depth_variance(coord, extent, center_linear_depth);
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let blur_mix: f32 = 1.0 / (1.0 + local_depth_stddev * GTAO_BLUR_VARIANCE_BLEND_SCALE);

		let filtered_ao: f32 = center_ao * GTAO_BLUR_SPATIAL_WEIGHTS[0];
		let total_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[0];

		for (let offset: u32 = 1; offset <= GTAO_BLUR_RADIUS; offset = offset + 1) {
			let spatial_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[offset];

			let positive_coord: vec2u = blur_sample_coordinate(coord, extent, offset, 0);
			let positive_depth: f32 = fetch(visibility_depth, positive_coord).x;
			if (positive_depth != 0.0) {
				let positive_difference: f32 = abs(relative_depth_delta(center_linear_depth, positive_depth));
				let positive_weight: f32 = spatial_weight * compute_bilateral_weight(positive_difference, local_depth_variance);
				filtered_ao = filtered_ao + fetch(ao_source, positive_coord).x * positive_weight;
				total_weight = total_weight + positive_weight;
			}

			let negative_coord: vec2u = blur_sample_coordinate(coord, extent, offset, 1);
			let negative_depth: f32 = fetch(visibility_depth, negative_coord).x;
			if (negative_depth != 0.0) {
				let negative_difference: f32 = abs(relative_depth_delta(center_linear_depth, negative_depth));
				let negative_weight: f32 = spatial_weight * compute_bilateral_weight(negative_difference, local_depth_variance);
				filtered_ao = filtered_ao + fetch(ao_source, negative_coord).x * negative_weight;
				total_weight = total_weight + negative_weight;
			}
		}

		let blurred_ao: f32 = filtered_ao / max(total_weight, 0.00001);
		let final_ao: f32 = mix(center_ao, blurred_ao, blur_mix);
		write(ao_output, coord, vec4f(final_ao, 0.0, 0.0, 1.0));
	}
	"#;

	besl::compile_to_besl(source, Some(build_gtao_blur_root()))
		.unwrap()
		.get_main()
		.unwrap()
}

fn build_gtao_blur_root() -> besl::Node {
	let mut root = besl::Node::root();
	let mat4f_type = root.get_child("mat4f").unwrap();
	let vec2f_type = root.get_child("vec2f").unwrap();
	let f32_type = root.get_child("f32").unwrap();

	let view_type = root.add_child(
		besl::Node::r#struct(
			"View",
			vec![
				besl::Node::member("view", mat4f_type.clone()).into(),
				besl::Node::member("projection", mat4f_type.clone()).into(),
				besl::Node::member("view_projection", mat4f_type.clone()).into(),
				besl::Node::member("inverse_view", mat4f_type.clone()).into(),
				besl::Node::member("inverse_projection", mat4f_type.clone()).into(),
				besl::Node::member("inverse_view_projection", mat4f_type.clone()).into(),
				besl::Node::member("fov", vec2f_type.clone()).into(),
				besl::Node::member("near", f32_type.clone()).into(),
				besl::Node::member("far", f32_type.clone()).into(),
			],
		)
		.into(),
	);

	root.add_children(vec![
		besl::Node::binding(
			"views",
			besl::BindingTypes::Buffer {
				members: vec![besl::Node::array("views", view_type, 8)],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"visibility_depth",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			1,
			0,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"ao_source",
			besl::BindingTypes::CombinedImageSampler { format: String::new() },
			1,
			1,
			true,
			false,
		)
		.into(),
		besl::Node::binding(
			"ao_output",
			besl::BindingTypes::Image {
				format: "r8".to_string(),
			},
			1,
			2,
			false,
			true,
		)
		.into(),
		besl::Node::specialization("blur_direction", vec2f_type).into(),
	]);

	root
}

#[cfg(test)]
mod tests {
	use super::{
		generate_gtao_blur_shader_for_language, generate_pixel_mapping_shader_for_language, get_material_count_msl_source,
		get_material_offset_msl_source, get_shadow_pass_mesh_msl_source, get_shadow_pass_mesh_source,
		get_visibility_pass_mesh_msl_source, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING,
		VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
	};
	use resource_management::platform_shader_generator::PlatformShaderLanguage;

	#[test]
	fn shadow_mesh_glsl_source_uses_besl_accessors() {
		let shader = get_shadow_pass_mesh_source();

		assert!(
			shader.contains("View view = views.views[push_constant.view_index];"),
			"Expected GLSL shadow mesh source to read the selected view through BESL accessors. Shader: {shader}"
		);
	}

	#[test]
	fn shadow_mesh_msl_source_uses_argument_buffer_accessors() {
		let shader = get_shadow_pass_mesh_msl_source();

		assert!(
			shader.contains("View view = set0.views->views[push_constant.view_index];")
				&& shader.contains("Mesh mesh = set0.meshes->meshes[push_constant.instance_index];"),
			"Expected MSL shadow mesh source to lower BESL accessors through the Metal argument buffer. Shader: {shader}"
		);
	}

	#[test]
	fn shadow_mesh_msl_source_compiles_for_metal() {
		use ghi::device::DeviceCreate as _;

		if !ghi::implementation::USES_METAL {
			return;
		}

		let shader = get_shadow_pass_mesh_msl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the shadow mesh shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a Metal device for the shadow mesh shader test");

		let shader_handle = device.create_shader(
			Some("Shadow Pass Mesh Shader"),
			ghi::shader::Sources::MTL {
				source: shader.as_str(),
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
		);

		assert!(
			shader_handle.is_ok(),
			"Expected the shadow mesh MSL source to compile for Metal"
		);
	}

	#[test]
	fn visibility_mesh_msl_source_uses_argument_buffer_accessors() {
		let shader = get_visibility_pass_mesh_msl_source();

		assert!(
			shader.contains("View view = set0.views->views[0];")
				&& shader.contains("Mesh mesh = set0.meshes->meshes[push_constant.instance_index];"),
			"Expected MSL visibility mesh source to lower BESL accessors through the Metal argument buffer. Shader: {shader}"
		);
	}

	#[test]
	fn visibility_mesh_msl_source_compiles_for_metal() {
		use ghi::device::DeviceCreate as _;

		if !ghi::implementation::USES_METAL {
			return;
		}

		let shader = get_visibility_pass_mesh_msl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the visibility mesh shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a Metal device for the visibility mesh shader test");

		let shader_handle = device.create_shader(
			Some("Visibility Pass Mesh Shader"),
			ghi::shader::Sources::MTL {
				source: shader.as_str(),
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
		);

		assert!(
			shader_handle.is_ok(),
			"Expected the visibility mesh MSL source to compile for Metal"
		);
	}

	#[test]
	fn material_count_msl_source_uses_argument_buffer_accessors() {
		let shader = get_material_count_msl_source();

		assert!(
			shader.contains("set1.instance_index_render_target.read(gid).x")
				&& shader.contains("set0.meshes->meshes[pixel_instance_index].material_index")
				&& shader.contains(
					"atomic_fetch_add_explicit(&set1.material_count->material_count[material_index], 1, memory_order_relaxed)"
				),
			"Expected MSL material count source to lower through Metal argument buffers. Shader: {shader}"
		);
	}

	#[test]
	fn material_offset_msl_source_uses_argument_buffer_accessors() {
		let shader = get_material_offset_msl_source();

		assert!(
			shader.contains("atomic_load_explicit(&set1.material_count->material_count[i], memory_order_relaxed)")
				&& shader.contains("set1.material_offset->material_offset[i] = sum;")
				&& shader.contains(
					"set1.material_evaluation_dispatches->material_evaluation_dispatches[i] = uint3((count + 127) / 128, 1, 1);"
				),
			"Expected MSL material offset source to lower through Metal argument buffers. Shader: {shader}"
		);
	}

	#[test]
	fn pixel_mapping_glsl_source_uses_besl_intrinsics() {
		let shader = generate_pixel_mapping_shader_for_language(PlatformShaderLanguage::Glsl).into_source();

		assert!(
			shader.contains("imageLoad(instance_index_render_target, ivec2(coord)).x"),
			"Expected GLSL pixel mapping source to lower the integer image load through a BESL intrinsic. Shader: {shader}"
		);
		assert!(
			shader.contains("atomicAdd(material_offset_scratch_buffer.material_offset_scratch[material_index], 1)"),
			"Expected GLSL pixel mapping source to lower the scratch offset increment through a BESL intrinsic. Shader: {shader}"
		);
	}

	#[test]
	fn pixel_mapping_msl_source_uses_platform_argument_buffer_lowering() {
		let shader = generate_pixel_mapping_shader_for_language(PlatformShaderLanguage::Msl).into_source();

		assert!(
			shader.contains("set1.instance_index_render_target.read(coord).x"),
			"Expected MSL pixel mapping source to lower the integer image load through the Metal texture API. Shader: {shader}"
		);
		assert!(
			shader.contains(
				"atomic_fetch_add_explicit(&set1.material_offset_scratch_buffer->material_offset_scratch[material_index], 1, memory_order_relaxed)"
			),
			"Expected MSL pixel mapping source to lower the scratch offset increment through the Metal atomic API. Shader: {shader}"
		);
	}

	#[test]
	fn gtao_blur_glsl_source_compiles_and_uses_const_array_weights() {
		let shader = generate_gtao_blur_shader_for_language(PlatformShaderLanguage::Glsl).into_source();

		assert!(
			shader.contains("const float[5] GTAO_BLUR_SPATIAL_WEIGHTS"),
			"Expected generated GLSL blur shader to preserve the const array weights. Shader: {shader}"
		);
		assert!(
			shader.contains("texelFetch(ao_source,ivec2(positive_coord),0).x"),
			"Expected generated GLSL blur shader to lower BESL fetch calls to texelFetch. Shader: {shader}"
		);

		resource_management::glsl::compile(&shader, "GTAO Blur Compute Shader")
			.expect("Expected generated GLSL blur shader to compile");
	}
}
