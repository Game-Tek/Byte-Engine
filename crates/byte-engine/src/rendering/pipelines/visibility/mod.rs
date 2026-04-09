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
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;
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

fn build_mesh_pass_msl_source(push_constant_fields: &str, view_lookup: &str) -> String {
	format!(
		r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(mesh)
// besl-threadgroup-size:128,1,1

{mesh_outputs}

struct PushConstant {{
{push_constant_fields}
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
	Mesh meshes[{max_instances}];
}};

struct _vertex_positions {{
	packed_float3 positions[{max_vertices}];
}};

struct _vertex_normals {{
	packed_float3 normals[{max_vertices}];
}};

struct _vertex_uvs {{
	packed_float2 uvs[{max_vertices}];
}};

struct _vertex_indices {{
	ushort vertex_indices[{max_primitive_triangles}];
}};

struct _primitive_indices {{
	uchar primitive_indices[{max_primitive_indices}];
}};

struct _meshlets {{
	Meshlet meshlets[{max_meshlets}];
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
	View view = set0.views->views[{view_lookup}];
	uint meshlet_index = threadgroup_position + mesh.base_meshlet_index;
	Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];
	uint primitive_index = thread_index;

	if (thread_index == 0) {{
		out_mesh.set_primitive_count(uint(meshlet.triangle_count));
	}}

	if (primitive_index < uint(meshlet.primitive_count)) {{
		uint vertex_index = mesh.base_vertex_index
			+ uint(set0.vertex_indices->vertex_indices[mesh.base_primitive_index + uint(meshlet.primitive_offset) + primitive_index]);
		float4 position = float4(float3(set0.vertex_positions->positions[vertex_index]), 1.0);
		out_mesh.set_vertex(primitive_index, VertexOutput{{ .position = position * mesh.model * view.view_projection }});
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
		push_constant_fields = push_constant_fields,
		view_lookup = view_lookup,
		max_instances = MAX_INSTANCES,
		max_vertices = MAX_VERTICES,
		max_primitive_triangles = MAX_PRIMITIVE_TRIANGLES,
		max_primitive_indices = MAX_TRIANGLES * 3,
		max_meshlets = MAX_MESHLETS,
	)
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
	build_mesh_pass_msl_source("\tuint instance_index;", "0")
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
	build_mesh_pass_msl_source("\tuint instance_index;\n\tuint view_index;", "push_constant.view_index")
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
	if (pixel_instance_index >= 1024u) { return; }

	uint material_index = meshes.meshes[pixel_instance_index].material_index;
	if (material_index >= 1024u) { return; }

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
	Mesh meshes[1024];
};

struct _views {
	uint views[1];
};

struct _material_count {
	atomic_uint material_count[1024];
};

struct _material_offset {
	uint material_offset[1024];
};

struct _material_offset_scratch_buffer {
	atomic_uint material_offset_scratch[1024];
};

struct _material_evaluation_dispatches {
	uint3 material_evaluation_dispatches[1024];
};

struct _pixel_mapping_buffer {
	ushort2 pixel_mapping[1];
};

struct _set0 {
	constant _views* views [[id(0)]];
	constant _meshes* mesh_data [[id(1)]];
};

struct _set1 {
	device _material_count* material_count_buffer [[id(0)]];
	device _material_offset* material_offset_buffer [[id(1)]];
	device _material_offset_scratch_buffer* material_offset_scratch_buffer [[id(2)]];
	device _material_evaluation_dispatches* material_evaluation_dispatches [[id(3)]];
	device _pixel_mapping_buffer* pixel_mapping_buffer [[id(4)]];
	texture2d<uint, access::read> triangle_index [[id(5)]];
	texture2d<uint, access::read> instance_index_render_target [[id(6)]];
};

kernel void besl_main(uint2 gid [[thread_position_in_grid]], constant _set0& set0 [[buffer(16)]], constant _set1& set1 [[buffer(17)]]) {
	uint width = set1.instance_index_render_target.get_width();
	uint height = set1.instance_index_render_target.get_height();
	if (gid.x >= width || gid.y >= height) { return; }

	uint pixel_instance_index = set1.instance_index_render_target.read(gid).x;
	if (pixel_instance_index == 0xFFFFFFFFu) { return; }
	if (pixel_instance_index >= 1024u) { return; }

	uint material_index = set0.mesh_data->meshes[pixel_instance_index].material_index;
	if (material_index >= 1024u) { return; }
	atomic_fetch_add_explicit(&set1.material_count_buffer->material_count[material_index], 1, memory_order_relaxed);
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

pub fn get_pixel_mapping_msl_source() -> String {
	format!(
		r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(compute)
// Note: Metal threadgroup sizes are set on the pipeline state.

struct Mesh {{
	float4x4 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
}};

struct _views {{
	uint views[1];
}};

struct _mesh_data {{
	Mesh meshes[1024];
}};

struct _material_count {{
	atomic_uint material_count[1024];
}};

struct _material_offset {{
	uint material_offset[1024];
}};

struct _material_offset_scratch_buffer {{
	atomic_uint material_offset_scratch[1024];
}};

struct _material_evaluation_dispatches {{
	uint3 material_evaluation_dispatches[1024];
}};

struct _pixel_mapping_buffer {{
	ushort2 pixel_mapping[{MAX_PIXEL_MAPPING_ENTRIES}];
}};

struct _set0 {{
	constant _views* views [[id(0)]];
	constant _mesh_data* mesh_data [[id(1)]];
}};

struct _set1 {{
	device _material_count* material_count_buffer [[id(0)]];
	device _material_offset* material_offset_buffer [[id(1)]];
	device _material_offset_scratch_buffer* material_offset_scratch_buffer [[id(2)]];
	device _material_evaluation_dispatches* material_evaluation_dispatches [[id(3)]];
	device _pixel_mapping_buffer* pixel_mapping_buffer [[id(4)]];
	texture2d<uint, access::read> triangle_index [[id(5)]];
	texture2d<uint, access::read> instance_index_render_target [[id(6)]];
}};

kernel void besl_main(uint2 coord [[thread_position_in_grid]], constant _set0& set0 [[buffer(16)]], constant _set1& set1 [[buffer(17)]]) {{
	uint width = set1.instance_index_render_target.get_width();
	uint height = set1.instance_index_render_target.get_height();
	if (coord.x >= width || coord.y >= height) {{ return; }}

	uint pixel_instance_index = set1.instance_index_render_target.read(coord).x;
	if (pixel_instance_index == 0xFFFFFFFFu) {{ return; }}
	if (pixel_instance_index >= 1024u) {{ return; }}

	uint material_index = set0.mesh_data->meshes[pixel_instance_index].material_index;
	if (material_index >= 1024u) {{ return; }}

	uint pixel_mapping_index = atomic_fetch_add_explicit(
		&set1.material_offset_scratch_buffer->material_offset_scratch[material_index],
		1,
		memory_order_relaxed
	);
	if (pixel_mapping_index >= {MAX_PIXEL_MAPPING_ENTRIES}u) {{ return; }}

	set1.pixel_mapping_buffer->pixel_mapping[pixel_mapping_index] = ushort2(coord.x, coord.y);
}}
"#
	)
}

pub fn get_pixel_mapping_source() -> String {
	get_pixel_mapping_shader().into_source()
}

pub fn get_pixel_mapping_shader() -> GeneratedPlatformShader {
	generate_pixel_mapping_shader_for_language(PlatformShaderLanguage::current_platform())
}

fn generate_pixel_mapping_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	generate_compute_shader_for_language(language, Extent::square(32), build_pixel_mapping_program)
}

fn generate_compute_shader_for_language(
	language: PlatformShaderLanguage,
	threadgroup_extent: Extent,
	build_program: fn() -> besl::NodeReference,
) -> GeneratedPlatformShader {
	let main_node = build_program();
	let mut shader_generator = PlatformShaderGenerator::new();

	shader_generator
		.generate_for_language(language, &ShaderGenerationSettings::compute(threadgroup_extent), &main_node)
		.unwrap()
}

fn build_pixel_mapping_program() -> besl::NodeReference {
	let source = r#"
	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(instance_index_render_target, coord);
		let pixel_instance_index: u32 = image_load_u32(instance_index_render_target, coord);

		if (pixel_instance_index < 4294967295 && pixel_instance_index < 1024) {
			let material_index: u32 = mesh_data.meshes[pixel_instance_index].material_index;

			if (material_index < 1024) {
				let pixel_mapping_index: u32 = atomic_add(material_offset_scratch_buffer.material_offset_scratch[material_index], 1);

				if (pixel_mapping_index < __MAX_PIXEL_MAPPING_ENTRIES__) {
					pixel_mapping_buffer.pixel_mapping[pixel_mapping_index] = vec2u16(coord.x, coord.y);
				}
			}
		}
	}
	"#
	.replace("__MAX_PIXEL_MAPPING_ENTRIES__", &MAX_PIXEL_MAPPING_ENTRIES.to_string());

	besl::compile_to_besl(&source, Some(build_pixel_mapping_root()))
		.unwrap()
		.get_main()
		.unwrap()
}

fn build_pixel_mapping_root() -> besl::Node {
	let mut root = besl::Node::root();
	let mat4f_t = root.get_child("mat4f").unwrap();
	let u32_t = root.get_child("u32").unwrap();
	let texture_2d = root.get_child("Texture2D").unwrap();
	let vec2u_t = root.get_child("vec2u").unwrap();
	let vec2u16_t = root.get_child("vec2u16").unwrap();
	let mesh = root.add_child(
		besl::Node::r#struct(
			"Mesh",
			vec![
				besl::Node::member("model", mat4f_t).into(),
				besl::Node::member("material_index", u32_t.clone()).into(),
				besl::Node::member("base_vertex_index", u32_t.clone()).into(),
				besl::Node::member("base_primitive_index", u32_t.clone()).into(),
				besl::Node::member("base_triangle_index", u32_t.clone()).into(),
				besl::Node::member("base_meshlet_index", u32_t.clone()).into(),
			],
		)
		.into(),
	);
	let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
	let views_member = besl::Node::array("views", u32_t.clone(), 1);
	let meshes_member = besl::Node::array("meshes", mesh, MAX_INSTANCES);
	let material_count_member = besl::Node::array("material_count", atomic_u32.clone(), MAX_MATERIALS);
	let material_offset_member = besl::Node::array("material_offset", u32_t.clone(), 1);
	let material_offset_scratch_member = besl::Node::array("material_offset_scratch", atomic_u32.clone(), MAX_MATERIALS);
	let material_evaluation_dispatches_member = besl::Node::array("material_evaluation_dispatches", u32_t.clone(), 1);
	let pixel_mapping_member = besl::Node::array("pixel_mapping", vec2u16_t, MAX_PIXEL_MAPPING_ENTRIES);

	root.add_children(vec![
		besl::Node::binding(
			"views",
			besl::BindingTypes::Buffer {
				members: vec![views_member.into()],
			},
			0,
			0,
			true,
			false,
		)
		.into(),
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
			"material_count_buffer",
			besl::BindingTypes::Buffer {
				members: vec![material_count_member.into()],
			},
			1,
			0,
			true,
			true,
		)
		.into(),
		besl::Node::binding(
			"material_offset_buffer",
			besl::BindingTypes::Buffer {
				members: vec![material_offset_member.into()],
			},
			1,
			1,
			true,
			true,
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
			"material_evaluation_dispatches",
			besl::BindingTypes::Buffer {
				members: vec![material_evaluation_dispatches_member.into()],
			},
			1,
			3,
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
			"triangle_index",
			besl::BindingTypes::Image {
				format: "r32ui".to_string(),
			},
			1,
			6,
			true,
			false,
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

pub fn get_gtao_shader() -> GeneratedPlatformShader {
	generate_gtao_shader_for_language(PlatformShaderLanguage::current_platform())
}

pub fn get_gtao_bitfield_blur_x_shader() -> GeneratedPlatformShader {
	generate_gtao_bitfield_blur_x_shader_for_language(PlatformShaderLanguage::current_platform())
}

pub fn get_gtao_bitfield_shader() -> GeneratedPlatformShader {
	generate_gtao_bitfield_shader_for_language(PlatformShaderLanguage::current_platform())
}

pub(crate) fn generate_gtao_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	generate_compute_shader_for_language(language, Extent::square(8), build_gtao_program)
}

pub(crate) fn generate_gtao_bitfield_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	generate_compute_shader_for_language(language, Extent::square(8), build_gtao_bitfield_program)
}

fn build_gtao_program() -> besl::NodeReference {
	let source = r#"
	GTAO_RADIUS: const f32 = 1.0;
	GTAO_BIAS: const f32 = 0.05;
	GTAO_STRENGTH: const f32 = 1.0;
	GTAO_MIN_RADIUS_PIXELS: const f32 = 4.0;
	GTAO_MAX_RADIUS_PIXELS: const f32 = 64.0;
	GTAO_MIN_EFFECTIVE_RADIUS_PIXELS: const f32 = 1.0;
	GTAO_DIRECTIONS: const u32 = 8;
	GTAO_STEPS: const u32 = 6;
	GTAO_PI: const f32 = 3.14159265359;

	interleaved_gradient_noise: fn (pixel: vec2u) -> f32 {
		return fract(52.9829189 * fract(0.06711056 * f32(pixel.x) + 0.00583715 * f32(pixel.y)));
	}

	make_uv: fn (pixel: vec2u, extent: vec2u) -> vec2f {
		let pixel_f: vec2f = vec2f(f32(pixel.x), f32(pixel.y));
		let extent_f: vec2f = vec2f(f32(extent.x), f32(extent.y));
		return (pixel_f + vec2f(0.5, 0.5)) / extent_f;
	}

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

	clamp_sample_coordinate: fn (
		pixel: vec2u,
		extent: vec2u,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32
	) -> vec2u {
		let sample_x: u32 = clamp_add(pixel.x, offset_x, extent.x);
		let sample_y: u32 = clamp_add(pixel.y, offset_y, extent.y);
		if (negative_x == 1) {
			sample_x = clamp_subtract(pixel.x, offset_x);
		}
		if (negative_y == 1) {
			sample_y = clamp_subtract(pixel.y, offset_y);
		}

		return vec2u(sample_x, sample_y);
	}

	reconstruct_view_space_position: fn (uv: vec2f, depth: f32, inverse_projection: mat4f) -> vec3f {
		let ndc: vec2f = vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
		let clip_space: vec4f = vec4f(ndc.x, ndc.y, depth, 1.0);
		let view_space: vec4f = inverse_projection * clip_space;
		let view_position: vec4f = view_space / view_space.w;
		return vec3f(view_position.x, view_position.y, view_position.z);
	}

	sample_view_space_position: fn (
		pixel: vec2u,
		extent: vec2u,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32,
		fallback_position: vec3f,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let sample_coord: vec2u = clamp_sample_coordinate(pixel, extent, offset_x, offset_y, negative_x, negative_y);
		let depth: f32 = fetch(depth_texture, sample_coord).x;
		if (depth == 0.0) {
			return fallback_position;
		}

		let uv: vec2f = make_uv(sample_coord, extent);
		return reconstruct_view_space_position(uv, depth, inverse_projection);
	}

	min_diff: fn (center: vec3f, a: vec3f, b: vec3f) -> vec3f {
		let da: vec3f = a - center;
		let db: vec3f = b - center;
		if (dot(da, da) < dot(db, db)) {
			return da;
		}

		return db;
	}

	reconstruct_normal: fn (
		pixel: vec2u,
		extent: vec2u,
		center_position: vec3f,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let right_position: vec3f = sample_view_space_position(
			pixel,
			extent,
			1,
			0,
			0,
			0,
			center_position,
			depth_texture,
			inverse_projection
		);
		let left_position: vec3f = sample_view_space_position(
			pixel,
			extent,
			1,
			0,
			1,
			0,
			center_position,
			depth_texture,
			inverse_projection
		);
		let top_position: vec3f = sample_view_space_position(
			pixel,
			extent,
			0,
			1,
			0,
			1,
			center_position,
			depth_texture,
			inverse_projection
		);
		let bottom_position: vec3f = sample_view_space_position(
			pixel,
			extent,
			0,
			1,
			0,
			0,
			center_position,
			depth_texture,
			inverse_projection
		);

		let dx: vec3f = min_diff(center_position, right_position, left_position);
		let dy: vec3f = min_diff(center_position, bottom_position, top_position);
		let normal: vec3f = normalize(cross(dx, dy));
		let view_direction: vec3f = vec3f(0.0, 0.0, 0.0) - center_position;

		if (dot(normal, view_direction) < 0.0) {
			return vec3f(0.0, 0.0, 0.0) - normal;
		}

		return normal;
	}

	compute_radii: fn (view_position: vec3f, extent: vec2u, view_fov: vec2f) -> vec3f {
		let tan_half_fov_y: f32 = tan(radians(view_fov.y) * 0.5);
		let pixels_per_unit: f32 = f32(extent.y) / max(2.0 * tan_half_fov_y * abs(view_position.z), 0.001);
		let ideal: f32 = GTAO_RADIUS * pixels_per_unit;
		let radius_fade: f32 = smoothstep(0.0, GTAO_MIN_RADIUS_PIXELS, ideal);
		let screen_radius: f32 = clamp(ideal, GTAO_MIN_EFFECTIVE_RADIUS_PIXELS, GTAO_MAX_RADIUS_PIXELS);
		let world_radius: f32 = screen_radius / pixels_per_unit;
		return vec3f(screen_radius, world_radius, radius_fade);
	}

	sample_direction: fn (
		pixel: vec2u,
		center_position: vec3f,
		normal: vec3f,
		direction: vec2f,
		screen_radius: f32,
		world_radius: f32,
		extent: vec2u,
		noise: f32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> f32 {
		let max_occlusion: f32 = 0.0;
		let radius_sq: f32 = world_radius * world_radius;

		for (let step: u32 = 1; step <= GTAO_STEPS; step = step + 1) {
			let step_noise: f32 = fract(noise + f32(step) * 0.618033988749);
			let step_ratio: f32 = (f32(step) - 0.5 + step_noise * 0.5) / f32(GTAO_STEPS);
			let sample_offset: vec2f = direction * screen_radius * step_ratio;
			let rounded_offset: vec2f = round(sample_offset);
			let offset_x: u32 = u32(abs(rounded_offset.x));
			let offset_y: u32 = u32(abs(rounded_offset.y));
			let negative_x: u32 = 0;
			let negative_y: u32 = 0;
			if (rounded_offset.x < 0.0) {
				negative_x = 1;
			}
			if (rounded_offset.y < 0.0) {
				negative_y = 1;
			}

			let sample_coord: vec2u = clamp_sample_coordinate(pixel, extent, offset_x, offset_y, negative_x, negative_y);
			let sample_depth: f32 = fetch(depth_texture, sample_coord).x;
			if (sample_depth == 0.0) {
				continue;
			}

			let sample_uv: vec2f = make_uv(sample_coord, extent);
			let sample_position: vec3f = reconstruct_view_space_position(sample_uv, sample_depth, inverse_projection);
			let sample_vector: vec3f = sample_position - center_position;
			let distance_sq: f32 = dot(sample_vector, sample_vector);

			if (distance_sq <= 0.00001 || distance_sq > radius_sq) {
				continue;
			}

			let sample_direction_vector: vec3f = sample_vector * inversesqrt(distance_sq);
			let alignment: f32 = max(dot(normal, sample_direction_vector) - GTAO_BIAS, 0.0);
			let falloff: f32 = 1.0 - distance_sq / radius_sq;
			max_occlusion = max(max_occlusion, alignment * falloff * falloff);
		}

		return max_occlusion;
	}

	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(ao_output, coord);
		let extent: vec2u = image_size(ao_output);
		let view: View = views.views[0];
		let inverse_projection: mat4f = view.inverse_projection;
		let view_fov: vec2f = view.fov;

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
			write(ao_output, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			return;
		}

		let uv: vec2f = make_uv(coord, extent);
		let center_position: vec3f = reconstruct_view_space_position(uv, center_depth, inverse_projection);
		let normal: vec3f = reconstruct_normal(coord, extent, center_position, visibility_depth, inverse_projection);
		let radii: vec3f = compute_radii(center_position, extent, view_fov);
		let rotation: f32 = interleaved_gradient_noise(coord) * GTAO_PI;
		let occlusion: f32 = 0.0;

		for (let direction_index: u32 = 0; direction_index < GTAO_DIRECTIONS; direction_index = direction_index + 1) {
			let angle: f32 = rotation + (2.0 * GTAO_PI * f32(direction_index)) / f32(GTAO_DIRECTIONS);
			let direction: vec2f = vec2f(cos(angle), sin(angle));
			let noise_coord: vec2u = coord + vec2u(direction_index * 7, direction_index * 13);
			occlusion = occlusion + sample_direction(
				coord,
				center_position,
				normal,
				direction,
				radii.x,
				radii.y,
				extent,
				interleaved_gradient_noise(noise_coord),
				visibility_depth,
				inverse_projection
			);
		}

		let average_occlusion: f32 = clamp(occlusion / f32(GTAO_DIRECTIONS) * GTAO_STRENGTH, 0.0, 1.0);
		let faded_occlusion: f32 = average_occlusion * radii.z;
		let ao: f32 = 1.0 - faded_occlusion;
		write(ao_output, coord, vec4f(ao, ao, ao, 1.0));
	}
	"#;

	besl::compile_to_besl(source, Some(build_gtao_root()))
		.unwrap()
		.get_main()
		.unwrap()
}

pub(crate) fn generate_gtao_blur_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	generate_compute_shader_for_language(language, Extent::square(8), build_gtao_blur_program)
}

pub(crate) fn generate_gtao_bitfield_blur_x_shader_for_language(language: PlatformShaderLanguage) -> GeneratedPlatformShader {
	generate_compute_shader_for_language(language, Extent::square(8), build_gtao_bitfield_blur_x_program)
}

fn build_gtao_bitfield_program() -> besl::NodeReference {
	let source = r#"
	GTAO_RADIUS: const f32 = 1.0;
	GTAO_BIAS: const f32 = 0.05;
	GTAO_STRENGTH: const f32 = 1.0;
	GTAO_PACKED_WORD_BITS: const u32 = 32;
	GTAO_MIN_RADIUS_PIXELS: const f32 = 4.0;
	GTAO_MAX_RADIUS_PIXELS: const f32 = 64.0;
	GTAO_MIN_EFFECTIVE_RADIUS_PIXELS: const f32 = 1.0;
	GTAO_DIRECTIONS: const u32 = 8;
	GTAO_STEPS: const u32 = 6;
	GTAO_PI: const f32 = 3.14159265359;

	interleaved_gradient_noise: fn (pixel: vec2u) -> f32 {
		return fract(52.9829189 * fract(0.06711056 * f32(pixel.x) + 0.00583715 * f32(pixel.y)));
	}

	make_uv: fn (pixel: vec2u, extent: vec2u) -> vec2f {
		let pixel_f: vec2f = vec2f(f32(pixel.x), f32(pixel.y));
		let extent_f: vec2f = vec2f(f32(extent.x), f32(extent.y));
		return (pixel_f + vec2f(0.5, 0.5)) / extent_f;
	}

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

	clamp_sample_coordinate: fn (
		pixel: vec2u,
		extent: vec2u,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32
	) -> vec2u {
		let sample_x: u32 = clamp_add(pixel.x, offset_x, extent.x);
		let sample_y: u32 = clamp_add(pixel.y, offset_y, extent.y);
		if (negative_x == 1) {
			sample_x = clamp_subtract(pixel.x, offset_x);
		}
		if (negative_y == 1) {
			sample_y = clamp_subtract(pixel.y, offset_y);
		}

		return vec2u(sample_x, sample_y);
	}

	reconstruct_view_space_position: fn (uv: vec2f, depth: f32, inverse_projection: mat4f) -> vec3f {
		let ndc: vec2f = vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
		let clip_space: vec4f = vec4f(ndc.x, ndc.y, depth, 1.0);
		let view_space: vec4f = inverse_projection * clip_space;
		let view_position: vec4f = view_space / view_space.w;
		return vec3f(view_position.x, view_position.y, view_position.z);
	}

	sample_view_space_position: fn (
		pixel: vec2u,
		extent: vec2u,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32,
		fallback_position: vec3f,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let sample_coord: vec2u = clamp_sample_coordinate(pixel, extent, offset_x, offset_y, negative_x, negative_y);
		let depth: f32 = fetch(depth_texture, sample_coord).x;
		if (depth == 0.0) {
			return fallback_position;
		}

		let uv: vec2f = make_uv(sample_coord, extent);
		return reconstruct_view_space_position(uv, depth, inverse_projection);
	}

	min_diff: fn (center: vec3f, a: vec3f, b: vec3f) -> vec3f {
		let da: vec3f = a - center;
		let db: vec3f = b - center;
		if (dot(da, da) < dot(db, db)) {
			return da;
		}

		return db;
	}

	reconstruct_normal: fn (
		pixel: vec2u,
		extent: vec2u,
		center_position: vec3f,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let right_position: vec3f = sample_view_space_position(pixel, extent, 1, 0, 0, 0, center_position, depth_texture, inverse_projection);
		let left_position: vec3f = sample_view_space_position(pixel, extent, 1, 0, 1, 0, center_position, depth_texture, inverse_projection);
		let top_position: vec3f = sample_view_space_position(pixel, extent, 0, 1, 0, 1, center_position, depth_texture, inverse_projection);
		let bottom_position: vec3f = sample_view_space_position(pixel, extent, 0, 1, 0, 0, center_position, depth_texture, inverse_projection);

		let dx: vec3f = min_diff(center_position, right_position, left_position);
		let dy: vec3f = min_diff(center_position, bottom_position, top_position);
		let normal: vec3f = normalize(cross(dx, dy));
		let view_direction: vec3f = vec3f(0.0, 0.0, 0.0) - center_position;

		if (dot(normal, view_direction) < 0.0) {
			return vec3f(0.0, 0.0, 0.0) - normal;
		}

		return normal;
	}

	compute_radii: fn (view_position: vec3f, extent: vec2u, view_fov: vec2f) -> vec3f {
		let tan_half_fov_y: f32 = tan(radians(view_fov.y) * 0.5);
		let pixels_per_unit: f32 = f32(extent.y) / max(2.0 * tan_half_fov_y * abs(view_position.z), 0.001);
		let ideal: f32 = GTAO_RADIUS * pixels_per_unit;
		let radius_fade: f32 = smoothstep(0.0, GTAO_MIN_RADIUS_PIXELS, ideal);
		let screen_radius: f32 = clamp(ideal, GTAO_MIN_EFFECTIVE_RADIUS_PIXELS, GTAO_MAX_RADIUS_PIXELS);
		let world_radius: f32 = screen_radius / pixels_per_unit;
		return vec3f(screen_radius, world_radius, radius_fade);
	}

	sample_direction: fn (
		pixel: vec2u,
		center_position: vec3f,
		normal: vec3f,
		direction: vec2f,
		screen_radius: f32,
		world_radius: f32,
		extent: vec2u,
		noise: f32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> f32 {
		let max_occlusion: f32 = 0.0;
		let radius_sq: f32 = world_radius * world_radius;

		for (let step: u32 = 1; step <= GTAO_STEPS; step = step + 1) {
			let step_noise: f32 = fract(noise + f32(step) * 0.618033988749);
			let step_ratio: f32 = (f32(step) - 0.5 + step_noise * 0.5) / f32(GTAO_STEPS);
			let sample_offset: vec2f = direction * screen_radius * step_ratio;
			let rounded_offset: vec2f = round(sample_offset);
			let offset_x: u32 = u32(abs(rounded_offset.x));
			let offset_y: u32 = u32(abs(rounded_offset.y));
			let negative_x: u32 = 0;
			let negative_y: u32 = 0;
			if (rounded_offset.x < 0.0) {
				negative_x = 1;
			}
			if (rounded_offset.y < 0.0) {
				negative_y = 1;
			}

			let sample_coord: vec2u = clamp_sample_coordinate(pixel, extent, offset_x, offset_y, negative_x, negative_y);
			let sample_depth: f32 = fetch(depth_texture, sample_coord).x;
			if (sample_depth == 0.0) {
				continue;
			}

			let sample_uv: vec2f = make_uv(sample_coord, extent);
			let sample_position: vec3f = reconstruct_view_space_position(sample_uv, sample_depth, inverse_projection);
			let sample_vector: vec3f = sample_position - center_position;
			let distance_sq: f32 = dot(sample_vector, sample_vector);

			if (distance_sq <= 0.00001 || distance_sq > radius_sq) {
				continue;
			}

			let sample_direction_vector: vec3f = sample_vector * inversesqrt(distance_sq);
			let alignment: f32 = max(dot(normal, sample_direction_vector) - GTAO_BIAS, 0.0);
			let falloff: f32 = 1.0 - distance_sq / radius_sq;
			max_occlusion = max(max_occlusion, alignment * falloff * falloff);
		}

		return max_occlusion;
	}

	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(ao_output, coord);
		let extent: vec2u = image_size(ao_output);
		let view: View = views.views[0];
		let inverse_projection: mat4f = view.inverse_projection;
		let view_fov: vec2f = view.fov;

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
			return;
		}

		let uv: vec2f = make_uv(coord, vec2u(extent.x * GTAO_PACKED_WORD_BITS, extent.y));
		let center_position: vec3f = reconstruct_view_space_position(uv, center_depth, inverse_projection);
		let normal: vec3f = reconstruct_normal(coord, vec2u(extent.x * GTAO_PACKED_WORD_BITS, extent.y), center_position, visibility_depth, inverse_projection);
		let radii: vec3f = compute_radii(center_position, vec2u(extent.x * GTAO_PACKED_WORD_BITS, extent.y), view_fov);
		let rotation: f32 = interleaved_gradient_noise(coord) * GTAO_PI;
		let occlusion: f32 = 0.0;

		for (let direction_index: u32 = 0; direction_index < GTAO_DIRECTIONS; direction_index = direction_index + 1) {
			let angle: f32 = rotation + (2.0 * GTAO_PI * f32(direction_index)) / f32(GTAO_DIRECTIONS);
			let direction: vec2f = vec2f(cos(angle), sin(angle));
			let noise_coord: vec2u = coord + vec2u(direction_index * 7, direction_index * 13);
			occlusion = occlusion + sample_direction(
				coord,
				center_position,
				normal,
				direction,
				radii.x,
				radii.y,
				vec2u(extent.x * GTAO_PACKED_WORD_BITS, extent.y),
				interleaved_gradient_noise(noise_coord),
				visibility_depth,
				inverse_projection
			);
		}

		let average_occlusion: f32 = clamp(occlusion / f32(GTAO_DIRECTIONS) * GTAO_STRENGTH, 0.0, 1.0);
		let faded_occlusion: f32 = average_occlusion * radii.z;
		let quantization_noise: f32 = interleaved_gradient_noise(coord + vec2u(19, 47));
		if (faded_occlusion <= quantization_noise) {
			return;
		}

		let packed_pixel: vec2u = vec2u(coord.x / GTAO_PACKED_WORD_BITS, coord.y);
		let bit_index: u32 = coord.x % GTAO_PACKED_WORD_BITS;
		image_atomic_or(ao_output, packed_pixel, 1 << bit_index);
	}
	"#;

	besl::compile_to_besl(source, Some(build_gtao_bitfield_root()))
		.unwrap()
		.get_main()
		.unwrap()
}

fn build_gtao_bitfield_blur_x_program() -> besl::NodeReference {
	let source = r#"
	GTAO_PACKED_WORD_BITS: const u32 = 32;
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

	packed_extent: fn (extent: vec2u) -> vec2u {
		return vec2u(extent.x / GTAO_PACKED_WORD_BITS, extent.y);
	}

	unpack_binary_ao: fn (pixel: vec2u) -> f32 {
		let packed_pixel: vec2u = vec2u(pixel.x / GTAO_PACKED_WORD_BITS, pixel.y);
		let packed_bits: u32 = fetch_u32(ao_source, packed_pixel);
		let bit_index: u32 = pixel.x % GTAO_PACKED_WORD_BITS;
		if (((packed_bits >> bit_index) & 1) == 0) {
			return 1.0;
		}

		return 0.0;
	}

	linear_view_depth: fn (depth: f32, inverse_projection: mat4f) -> f32 {
		let clip_space: vec4f = vec4f(0.0, 0.0, depth, 1.0);
		let view_space: vec4f = inverse_projection * clip_space;
		return max(view_space.z / view_space.w, 0.0001);
	}

	relative_depth_delta: fn (center_linear_depth: f32, sample_depth: f32, inverse_projection: mat4f) -> f32 {
		let sample_linear_depth: f32 = linear_view_depth(sample_depth, inverse_projection);
		return (sample_linear_depth - center_linear_depth) / max(center_linear_depth, 0.0001);
	}

	blur_sample_coordinate: fn (pixel: vec2u, extent: vec2u, offset: u32, negative: u32) -> vec2u {
		if (negative == 1) {
			return vec2u(clamp_subtract(pixel.x, offset), pixel.y);
		}

		return vec2u(clamp_add(pixel.x, offset, extent.x), pixel.y);
	}

	neighbor_depth: fn (pixel: vec2u, extent: vec2u, offset_x: u32, offset_y: u32, negative_x: u32, negative_y: u32, depth_texture: Texture2D) -> f32 {
		let sample_x: u32 = clamp_add(pixel.x, offset_x, extent.x);
		let sample_y: u32 = clamp_add(pixel.y, offset_y, extent.y);
		if (negative_x == 1) {
			sample_x = clamp_subtract(pixel.x, offset_x);
		}
		if (negative_y == 1) {
			sample_y = clamp_subtract(pixel.y, offset_y);
		}
		return fetch(depth_texture, vec2u(sample_x, sample_y)).x;
	}

	neighbor_stats: fn (
		pixel: vec2u,
		extent: vec2u,
		center_linear_depth: f32,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let sample_depth: f32 = neighbor_depth(pixel, extent, offset_x, offset_y, negative_x, negative_y, depth_texture);
		if (sample_depth == 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}

		let delta: f32 = relative_depth_delta(center_linear_depth, sample_depth, inverse_projection);
		return vec3f(delta, delta * delta, 1.0);
	}

	compute_local_depth_variance: fn (
		pixel: vec2u,
		extent: vec2u,
		center_linear_depth: f32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> f32 {
		let sample_aa: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 1, depth_texture, inverse_projection);
		let sample_ab: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 1, depth_texture, inverse_projection);
		let sample_ac: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 1, depth_texture, inverse_projection);
		let sample_ba: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 1, 0, depth_texture, inverse_projection);
		let sample_bb: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 0, 0, 0, depth_texture, inverse_projection);
		let sample_bc: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 0, 0, depth_texture, inverse_projection);
		let sample_ca: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 0, depth_texture, inverse_projection);
		let sample_cb: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 0, depth_texture, inverse_projection);
		let sample_cc: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 0, depth_texture, inverse_projection);

		let mean: f32 = sample_aa.x;
		mean = mean + sample_ab.x;
		mean = mean + sample_ac.x;
		mean = mean + sample_ba.x;
		mean = mean + sample_bb.x;
		mean = mean + sample_bc.x;
		mean = mean + sample_ca.x;
		mean = mean + sample_cb.x;
		mean = mean + sample_cc.x;

		let mean_sq: f32 = sample_aa.y;
		mean_sq = mean_sq + sample_ab.y;
		mean_sq = mean_sq + sample_ac.y;
		mean_sq = mean_sq + sample_ba.y;
		mean_sq = mean_sq + sample_bb.y;
		mean_sq = mean_sq + sample_bc.y;
		mean_sq = mean_sq + sample_ca.y;
		mean_sq = mean_sq + sample_cb.y;
		mean_sq = mean_sq + sample_cc.y;

		let sample_count: f32 = sample_aa.z;
		sample_count = sample_count + sample_ab.z;
		sample_count = sample_count + sample_ac.z;
		sample_count = sample_count + sample_ba.z;
		sample_count = sample_count + sample_bb.z;
		sample_count = sample_count + sample_bc.z;
		sample_count = sample_count + sample_ca.z;
		sample_count = sample_count + sample_cb.z;
		sample_count = sample_count + sample_cc.z;

		if (sample_count <= 1.0) {
			return 0.0;
		}

		let normalized_mean: f32 = mean / sample_count;
		let normalized_mean_sq: f32 = mean_sq / sample_count;
		return max(normalized_mean_sq - normalized_mean * normalized_mean, 0.0);
	}

	compute_bilateral_weight: fn (relative_depth_difference: f32, local_depth_variance: f32) -> f32 {
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let sigma_scale: f32 = 1.0 + local_depth_stddev * GTAO_BLUR_SIGMA_VARIANCE_SCALE;
		let sigma_candidate: f32 = GTAO_BLUR_BASE_RELATIVE_SIGMA / sigma_scale;
		let sigma: f32 = max(GTAO_BLUR_MIN_RELATIVE_SIGMA, sigma_candidate);
		let denominator: f32 = max(2.0 * sigma * sigma, 0.000001);
		let exponent_value: f32 = (relative_depth_difference * relative_depth_difference) / denominator;
		return exp(0.0 - exponent_value);
	}

	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(ao_output, coord);
		let extent: vec2u = image_size(ao_output);
		let full_extent: vec2u = vec2u(extent.x * GTAO_PACKED_WORD_BITS, extent.y);
		let view: View = views.views[0];
		let inverse_projection: mat4f = view.inverse_projection;

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
			write(ao_output, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			return;
		}

		let center_ao: f32 = unpack_binary_ao(coord);
		let center_linear_depth: f32 = linear_view_depth(center_depth, inverse_projection);
		let local_depth_variance: f32 = compute_local_depth_variance(coord, full_extent, center_linear_depth, visibility_depth, inverse_projection);
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let blur_mix: f32 = 1.0 / (1.0 + local_depth_stddev * GTAO_BLUR_VARIANCE_BLEND_SCALE);

		let filtered_ao: f32 = center_ao * GTAO_BLUR_SPATIAL_WEIGHTS[0];
		let total_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[0];

		for (let offset: u32 = 1; offset <= GTAO_BLUR_RADIUS; offset = offset + 1) {
			let spatial_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[offset];

			let positive_coord: vec2u = blur_sample_coordinate(coord, full_extent, offset, 0);
			let positive_depth: f32 = fetch(visibility_depth, positive_coord).x;
			if (positive_depth != 0.0) {
				let positive_difference: f32 = abs(relative_depth_delta(center_linear_depth, positive_depth, inverse_projection));
				let positive_weight: f32 = spatial_weight * compute_bilateral_weight(positive_difference, local_depth_variance);
				filtered_ao = filtered_ao + unpack_binary_ao(positive_coord) * positive_weight;
				total_weight = total_weight + positive_weight;
			}

			let negative_coord: vec2u = blur_sample_coordinate(coord, full_extent, offset, 1);
			let negative_depth: f32 = fetch(visibility_depth, negative_coord).x;
			if (negative_depth != 0.0) {
				let negative_difference: f32 = abs(relative_depth_delta(center_linear_depth, negative_depth, inverse_projection));
				let negative_weight: f32 = spatial_weight * compute_bilateral_weight(negative_difference, local_depth_variance);
				filtered_ao = filtered_ao + unpack_binary_ao(negative_coord) * negative_weight;
				total_weight = total_weight + negative_weight;
			}
		}

		let blurred_ao: f32 = filtered_ao / max(total_weight, 0.00001);
		let final_ao: f32 = mix(center_ao, blurred_ao, blur_mix);
		write(ao_output, coord, vec4f(final_ao, 0.0, 0.0, 1.0));
	}
	"#;

	besl::compile_to_besl(source, Some(build_gtao_bitfield_blur_x_root()))
		.unwrap()
		.get_main()
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

	linear_view_depth: fn (depth: f32, inverse_projection: mat4f) -> f32 {
		let clip_space: vec4f = vec4f(0.0, 0.0, depth, 1.0);
		let view_space: vec4f = inverse_projection * clip_space;
		return max(view_space.z / view_space.w, 0.0001);
	}

	relative_depth_delta: fn (center_linear_depth: f32, sample_depth: f32, inverse_projection: mat4f) -> f32 {
		let sample_linear_depth: f32 = linear_view_depth(sample_depth, inverse_projection);
		return (sample_linear_depth - center_linear_depth) / max(center_linear_depth, 0.0001);
	}

	blur_sample_coordinate: fn (pixel: vec2u, extent: vec2u, offset: u32, negative: u32, direction: vec2f) -> vec2u {
		if (direction.x > 0.0) {
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
		negative_y: u32,
		depth_texture: Texture2D
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
		return fetch(depth_texture, sample_coord).x;
	}

	neighbor_stats: fn (
		pixel: vec2u,
		extent: vec2u,
		center_linear_depth: f32,
		offset_x: u32,
		offset_y: u32,
		negative_x: u32,
		negative_y: u32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> vec3f {
		let sample_depth: f32 = neighbor_depth(pixel, extent, offset_x, offset_y, negative_x, negative_y, depth_texture);
		if (sample_depth == 0.0) {
			return vec3f(0.0, 0.0, 0.0);
		}

		let delta: f32 = relative_depth_delta(center_linear_depth, sample_depth, inverse_projection);
		return vec3f(delta, delta * delta, 1.0);
	}

	compute_local_depth_variance: fn (
		pixel: vec2u,
		extent: vec2u,
		center_linear_depth: f32,
		depth_texture: Texture2D,
		inverse_projection: mat4f
	) -> f32 {
		let sample_aa: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 1, depth_texture, inverse_projection);
		let sample_ab: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 1, depth_texture, inverse_projection);
		let sample_ac: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 1, depth_texture, inverse_projection);
		let sample_ba: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 1, 0, depth_texture, inverse_projection);
		let sample_bb: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 0, 0, 0, depth_texture, inverse_projection);
		let sample_bc: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 0, 0, 0, depth_texture, inverse_projection);
		let sample_ca: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 1, 0, depth_texture, inverse_projection);
		let sample_cb: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 0, 1, 0, 0, depth_texture, inverse_projection);
		let sample_cc: vec3f = neighbor_stats(pixel, extent, center_linear_depth, 1, 1, 0, 0, depth_texture, inverse_projection);

		let mean: f32 = sample_aa.x;
		mean = mean + sample_ab.x;
		mean = mean + sample_ac.x;
		mean = mean + sample_ba.x;
		mean = mean + sample_bb.x;
		mean = mean + sample_bc.x;
		mean = mean + sample_ca.x;
		mean = mean + sample_cb.x;
		mean = mean + sample_cc.x;

		let mean_sq: f32 = sample_aa.y;
		mean_sq = mean_sq + sample_ab.y;
		mean_sq = mean_sq + sample_ac.y;
		mean_sq = mean_sq + sample_ba.y;
		mean_sq = mean_sq + sample_bb.y;
		mean_sq = mean_sq + sample_bc.y;
		mean_sq = mean_sq + sample_ca.y;
		mean_sq = mean_sq + sample_cb.y;
		mean_sq = mean_sq + sample_cc.y;

		let sample_count: f32 = sample_aa.z;
		sample_count = sample_count + sample_ab.z;
		sample_count = sample_count + sample_ac.z;
		sample_count = sample_count + sample_ba.z;
		sample_count = sample_count + sample_bb.z;
		sample_count = sample_count + sample_bc.z;
		sample_count = sample_count + sample_ca.z;
		sample_count = sample_count + sample_cb.z;
		sample_count = sample_count + sample_cc.z;

		if (sample_count <= 1.0) {
			return 0.0;
		}

		let normalized_mean: f32 = mean / sample_count;
		let normalized_mean_sq: f32 = mean_sq / sample_count;
		return max(normalized_mean_sq - normalized_mean * normalized_mean, 0.0);
	}

	compute_bilateral_weight: fn (relative_depth_difference: f32, local_depth_variance: f32) -> f32 {
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let sigma_scale: f32 = 1.0 + local_depth_stddev * GTAO_BLUR_SIGMA_VARIANCE_SCALE;
		let sigma_candidate: f32 = GTAO_BLUR_BASE_RELATIVE_SIGMA / sigma_scale;
		let sigma: f32 = max(GTAO_BLUR_MIN_RELATIVE_SIGMA, sigma_candidate);
		let denominator: f32 = max(2.0 * sigma * sigma, 0.000001);
		let exponent_value: f32 = (relative_depth_difference * relative_depth_difference) / denominator;
		return exp(0.0 - exponent_value);
	}

	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(ao_output, coord);
		let extent: vec2u = image_size(ao_output);
		let inverse_projection: mat4f = views.views[0].inverse_projection;

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
			write(ao_output, coord, vec4f(1.0, 1.0, 1.0, 1.0));
			return;
		}

		let center_ao: f32 = fetch(ao_source, coord).x;
		let center_linear_depth: f32 = linear_view_depth(center_depth, inverse_projection);
		let local_depth_variance: f32 = compute_local_depth_variance(
			coord,
			extent,
			center_linear_depth,
			visibility_depth,
			inverse_projection
		);
		let local_depth_stddev: f32 = sqrt(local_depth_variance);
		let blur_mix: f32 = 1.0 / (1.0 + local_depth_stddev * GTAO_BLUR_VARIANCE_BLEND_SCALE);

		let filtered_ao: f32 = center_ao * GTAO_BLUR_SPATIAL_WEIGHTS[0];
		let total_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[0];

		for (let offset: u32 = 1; offset <= GTAO_BLUR_RADIUS; offset = offset + 1) {
			let spatial_weight: f32 = GTAO_BLUR_SPATIAL_WEIGHTS[offset];

			let positive_coord: vec2u = blur_sample_coordinate(coord, extent, offset, 0, blur_direction);
			let positive_depth: f32 = fetch(visibility_depth, positive_coord).x;
			if (positive_depth != 0.0) {
				let positive_difference: f32 = abs(relative_depth_delta(center_linear_depth, positive_depth, inverse_projection));
				let positive_weight: f32 = spatial_weight * compute_bilateral_weight(positive_difference, local_depth_variance);
				filtered_ao = filtered_ao + fetch(ao_source, positive_coord).x * positive_weight;
				total_weight = total_weight + positive_weight;
			}

			let negative_coord: vec2u = blur_sample_coordinate(coord, extent, offset, 1, blur_direction);
			let negative_depth: f32 = fetch(visibility_depth, negative_coord).x;
			if (negative_depth != 0.0) {
				let negative_difference: f32 = abs(relative_depth_delta(center_linear_depth, negative_depth, inverse_projection));
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

fn build_gtao_view_buffer_root() -> besl::Node {
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
	]);

	root
}

fn build_gtao_blur_root() -> besl::Node {
	let mut root = build_gtao_view_buffer_root();
	let vec2f_type = root.get_child("vec2f").unwrap();

	root.add_children(vec![
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

fn build_gtao_bitfield_blur_x_root() -> besl::Node {
	let mut root = build_gtao_view_buffer_root();

	root.add_children(vec![
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
			besl::BindingTypes::CombinedImageSampler {
				format: "r32ui".to_string(),
			},
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
	]);

	root
}

fn build_gtao_root() -> besl::Node {
	let mut root = build_gtao_view_buffer_root();

	root.add_children(vec![
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
			"ao_output",
			besl::BindingTypes::Image {
				format: "r8".to_string(),
			},
			1,
			1,
			false,
			true,
		)
		.into(),
	]);

	root
}

fn build_gtao_bitfield_root() -> besl::Node {
	let mut root = build_gtao_view_buffer_root();

	root.add_children(vec![
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
			"ao_output",
			besl::BindingTypes::Image {
				format: "r32ui".to_string(),
			},
			1,
			1,
			true,
			true,
		)
		.into(),
	]);

	root
}

#[cfg(test)]
mod tests {
	use super::{
		MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESH_DATA_BINDING, MESHLET_DATA_BINDING,
		PRIMITIVE_INDICES_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING,
		VIEWS_DATA_BINDING, generate_gtao_bitfield_blur_x_shader_for_language, generate_gtao_bitfield_shader_for_language,
		generate_gtao_blur_shader_for_language, generate_gtao_shader_for_language, generate_pixel_mapping_shader_for_language,
		get_material_count_msl_source, get_material_offset_msl_source, get_pixel_mapping_msl_source,
		get_shadow_pass_mesh_msl_source, get_shadow_pass_mesh_source, get_visibility_pass_mesh_msl_source,
	};
	use resource_management::platform_shader_generator::PlatformShaderLanguage;

	#[test]
	fn shadow_mesh_glsl_source_uses_besl_accessors() {
		let shader = get_shadow_pass_mesh_source();

		assert!(
			shader.contains("View view = views.views[push_constant.view_index];")
				|| shader.contains("uint32_t view_index = push_constant.view_index;")
					&& shader.contains("View view = views.views[view_index];"),
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
	fn shadow_mesh_msl_source_matches_visibility_buffer_layout() {
		let shader = get_shadow_pass_mesh_msl_source();

		assert!(
			shader.contains(&format!("packed_float3 positions[{MAX_VERTICES}];"))
				&& shader.contains(&format!("packed_float2 uvs[{MAX_VERTICES}];"))
				&& shader.contains(&format!("ushort vertex_indices[{MAX_PRIMITIVE_TRIANGLES}];"))
				&& shader.contains(&format!("uchar primitive_indices[{}];", MAX_TRIANGLES * 3))
				&& shader.contains(&format!("Meshlet meshlets[{MAX_MESHLETS}];")),
			"Expected the shadow mesh MSL source to preserve the packed visibility buffer layout. Shader: {shader}"
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
	fn visibility_mesh_msl_source_matches_visibility_buffer_layout() {
		let shader = get_visibility_pass_mesh_msl_source();

		assert!(
			shader.contains(&format!("packed_float3 positions[{MAX_VERTICES}];"))
				&& shader.contains(&format!("packed_float2 uvs[{MAX_VERTICES}];"))
				&& shader.contains(&format!("ushort vertex_indices[{MAX_PRIMITIVE_TRIANGLES}];"))
				&& shader.contains(&format!("uchar primitive_indices[{}];", MAX_TRIANGLES * 3))
				&& shader.contains(&format!("Meshlet meshlets[{MAX_MESHLETS}];")),
			"Expected the visibility mesh MSL source to preserve the packed visibility buffer layout. Shader: {shader}"
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
				&& shader.contains("set0.mesh_data->meshes[pixel_instance_index].material_index")
				&& shader.contains("constant _views* views [[id(0)]];")
				&& shader.contains("constant _meshes* mesh_data [[id(1)]];")
				&& shader.contains("device _material_count* material_count_buffer [[id(0)]];")
				&& shader.contains("texture2d<uint, access::read> instance_index_render_target [[id(6)]];")
				&& shader.contains(
					"atomic_fetch_add_explicit(&set1.material_count_buffer->material_count[material_index], 1, memory_order_relaxed)"
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
		let shader = get_pixel_mapping_msl_source();

		assert!(
			shader.contains("float4x4 model;")
				&& shader.contains("uint material_index;")
				&& shader.contains("uint base_meshlet_index;")
				&& shader.contains("constant _views* views [[id(0)]];")
				&& shader.contains("constant _mesh_data* mesh_data [[id(1)]];")
				&& shader.contains("device _material_offset_scratch_buffer* material_offset_scratch_buffer [[id(2)]];")
				&& shader.contains("device _pixel_mapping_buffer* pixel_mapping_buffer [[id(4)]];")
				&& shader.contains("texture2d<uint, access::read> instance_index_render_target [[id(6)]];"),
			"Expected MSL pixel mapping source to preserve the full mesh buffer layout. Shader: {shader}"
		);
		assert!(
			shader.contains("set1.instance_index_render_target.read(coord).x"),
			"Expected MSL pixel mapping source to lower the integer image load through the Metal texture API. Shader: {shader}"
		);
		assert!(
			shader.contains("atomic_fetch_add_explicit(")
				&& shader.contains("&set1.material_offset_scratch_buffer->material_offset_scratch[material_index]")
				&& shader.contains("memory_order_relaxed"),
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
			shader.contains("texelFetch(ao_source, ivec2(positive_coord),0)")
				|| shader.contains("texelFetch(ao_source,ivec2(positive_coord),0)"),
			"Expected generated GLSL blur shader to lower BESL fetch calls to texelFetch. Shader: {shader}"
		);

		resource_management::glsl::compile(&shader, "GTAO Blur Compute Shader")
			.expect("Expected generated GLSL blur shader to compile");
	}

	#[test]
	fn gtao_glsl_source_compiles_and_uses_fetch_lowering() {
		let shader = generate_gtao_shader_for_language(PlatformShaderLanguage::Glsl).into_source();

		assert!(
			shader.contains("texelFetch(visibility_depth, ivec2(coord),0)")
				|| shader.contains("texelFetch(visibility_depth,ivec2(coord),0)"),
			"Expected generated GLSL GTAO shader to lower BESL fetch calls to texelFetch. Shader: {shader}"
		);
		assert!(
			shader.contains("cross(dx,dy)") || shader.contains("cross(dx, dy)"),
			"Expected generated GLSL GTAO shader to preserve the cross-product normal reconstruction. Shader: {shader}"
		);

		resource_management::glsl::compile(&shader, "GTAO Compute Shader")
			.expect("Expected generated GLSL GTAO shader to compile");
	}

	#[test]
	fn gtao_msl_source_uses_argument_buffer_accessors() {
		let shader = generate_gtao_shader_for_language(PlatformShaderLanguage::Msl).into_source();

		assert!(
			shader.contains("View view = set0.views->views[0];"),
			"Expected generated MSL GTAO shader to lower the view lookup through the Metal argument buffer. Shader: {shader}"
		);
		assert!(
			shader.contains("view.inverse_projection") && shader.contains("view.fov"),
			"Expected generated MSL GTAO shader to read inverse projection and FOV from the loaded view. Shader: {shader}"
		);
	}

	#[test]
	fn gtao_bitfield_glsl_source_compiles_and_uses_image_atomic_or() {
		let shader = generate_gtao_bitfield_shader_for_language(PlatformShaderLanguage::Glsl).into_source();

		assert!(
			shader.contains("imageAtomicOr(ao_output, ivec2(packed_pixel), 1 << bit_index)")
				|| shader.contains("imageAtomicOr(ao_output,ivec2(packed_pixel),1 << bit_index)"),
			"Expected generated GLSL bitfield GTAO shader to lower packed writes through imageAtomicOr. Shader: {shader}"
		);

		resource_management::glsl::compile(&shader, "Bitfield GTAO Compute Shader")
			.expect("Expected generated GLSL bitfield GTAO shader to compile");
	}

	#[test]
	fn gtao_bitfield_blur_x_glsl_source_compiles_and_uses_integer_fetch() {
		let shader = generate_gtao_bitfield_blur_x_shader_for_language(PlatformShaderLanguage::Glsl).into_source();

		assert!(
			shader.contains("uniform usampler2D ao_source"),
			"Expected generated GLSL bitfield blur shader to use an unsigned sampler for packed AO reads. Shader: {shader}"
		);
		assert!(
			shader.contains("texelFetch(ao_source, ivec2(packed_pixel),0).x")
				|| shader.contains("texelFetch(ao_source,ivec2(packed_pixel),0).x"),
			"Expected generated GLSL bitfield blur shader to lower integer AO fetches through texelFetch. Shader: {shader}"
		);

		resource_management::glsl::compile(&shader, "Bitfield GTAO Blur X Compute Shader")
			.expect("Expected generated GLSL bitfield blur shader to compile");
	}
}
