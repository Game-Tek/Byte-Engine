#[doc(hidden)]
pub mod gpu_vertex_data_manager;
pub mod pipeline_manager;
#[doc(hidden)]
pub mod render_pass;
#[doc(hidden)]
pub mod resource_manager;
#[doc(hidden)]
pub mod scene_manager;
#[doc(hidden)]
pub mod shader_generator;
pub(crate) mod skinning;

pub use pipeline_manager::VisibilityPipelineManager;
use resource_management::shader::{
	besl::backends::{
		glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator, platform::PlatformShaderLanguage,
	},
	generator::ShaderGenerationSettings,
};
use utils::Extent;

use crate::rendering::{
	common_shader_generator::CommonShaderScope, pipelines::visibility::shader_generator::VisibilityShaderScope,
	shader_store::ShaderSourceDefinition,
};

/* BASE */
/// Binding to access the views which may be used to render the scene.
pub(crate) const VIEWS_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::TASK
		.union(ghi::Stages::MESH)
		.union(ghi::Stages::FRAGMENT)
		.union(ghi::Stages::RAYGEN)
		.union(ghi::Stages::COMPUTE),
)
.buffer_stride(400)
.buffer_read_only(true);
// ShaderMesh array stride includes tail padding from the CPU matrix alignment; shader Mesh structs carry matching padding.
pub(crate) const MESH_DATA_BUFFER_STRIDE: u32 = if cfg!(target_os = "macos") { 96 } else { 80 };
pub(crate) const MESH_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::TASK
		.union(ghi::Stages::MESH)
		.union(ghi::Stages::FRAGMENT)
		.union(ghi::Stages::COMPUTE),
)
.buffer_stride(MESH_DATA_BUFFER_STRIDE)
.buffer_read_only(true);
pub(crate) const VERTEX_POSITIONS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	2,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_stride(12)
.buffer_read_only(true);
pub(crate) const VERTEX_NORMALS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	3,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_stride(12)
.buffer_read_only(true);
pub(crate) const SKINNED_VERTICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	4,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_stride(32)
.buffer_read_only(true);
pub(crate) const VERTEX_UV_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	5,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_stride(8)
.buffer_read_only(true);
pub(crate) const VERTEX_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	6,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_read_only(true);
pub(crate) const PRIMITIVE_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	7,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::MESH.union(ghi::Stages::COMPUTE),
)
.buffer_read_only(true);
pub(crate) const MESHLET_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	8,
	ghi::descriptors::DescriptorType::StorageBuffer,
	ghi::Stages::TASK.union(ghi::Stages::MESH).union(ghi::Stages::COMPUTE),
)
.buffer_stride(64)
.buffer_read_only(true);
pub(crate) const TEXTURES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(
	9,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
	MAX_BINDLESS_TEXTURES as u32,
);

/* Visibility */
pub(crate) const MATERIAL_COUNT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub(crate) const MATERIAL_OFFSET_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub(crate) const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub(crate) const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE)
		.buffer_stride(16);
pub(crate) const MATERIAL_XY_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE)
		.buffer_stride(8);
pub(crate) const TRIANGLE_INDEX_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(6, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const INSTANCE_ID_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(7, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

/* Material Evaluation */
pub(crate) const OUT_LIT: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const CAMERA: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const LIGHTING_DATA: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const MATERIALS: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(5, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const AO: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(10, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(crate) const DEPTH_SHADOW_MAP: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(11, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;
const MESHLET_CULLING_TASK_GROUP_SIZE: u32 = 32;

const MAX_MESHLETS: usize = 1024 * 4;
const MAX_INSTANCES: usize = 1024;
const MAX_MATERIALS: usize = 1024;
// Materials keep a small indirection table so generated shaders can use stable per-material slots,
// while the descriptor array itself is a larger scene-wide bindless texture pool.
const MAX_MATERIAL_TEXTURES: usize = 16;
const MAX_BINDLESS_TEXTURES: usize = 1024;
const MAX_LIGHTS: usize = 16;
const MAX_TRIANGLES: usize = 65536 * 4;
const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
const MAX_VERTICES: usize = 65536 * 4;
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;
pub(crate) const SHADOW_CASCADE_COUNT: usize = 4;
pub(crate) const SHADOW_MAP_RESOLUTION: u32 = 2048;

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

fn generate_shader_source_for_language(
	language: PlatformShaderLanguage,
	settings: &ShaderGenerationSettings,
	main_node: &besl::NodeReference,
) -> Result<String, ()> {
	match language {
		PlatformShaderLanguage::Glsl => GLSLShaderGenerator::new().generate(settings, main_node),
		PlatformShaderLanguage::Hlsl => HLSLShaderGenerator::new().generate(settings, main_node),
		PlatformShaderLanguage::Msl => MSLShaderGenerator::new().generate(settings, main_node),
	}
}

fn generate_mesh_source_for_language(
	source: &'static str,
	push_constant: besl::parser::Node<'static>,
	language: PlatformShaderLanguage,
) -> String {
	let main_node = build_mesh_program_from_source(source, push_constant);
	let settings = ShaderGenerationSettings::mesh(64, 126, Extent::line(128));
	let generated = generate_shader_source_for_language(language, &settings, &main_node).unwrap();

	if language == PlatformShaderLanguage::Msl && !generated.contains("struct VertexOutput") {
		return generated.replacen(
			"using namespace metal;",
			&format!("using namespace metal;\n{}", MESH_OUTPUT_TYPES_MSL),
			1,
		);
	}

	generated
}

fn build_mesh_culling_task_msl_source(push_constant_fields: &str, view_lookup: &str) -> String {
	format!(
		r#"#include <metal_stdlib>
using namespace metal;
// #pragma shader_stage(object)
// besl-threadgroup-size:{task_group_size},1,1

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
	float4x3 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
	uint meshlet_count;
	uint skinned_base_vertex_index;
	uint padding0;
}};

struct Meshlet {{
	uint primitive_offset;
	uint triangle_offset;
	uint primitive_count;
	uint triangle_count;
	float4 center_radius;
	float4 cone_apex_cutoff;
	float4 cone_axis;
}};

struct ObjectPayload {{
	uint meshlet_indices[{task_group_size}];
}};

struct _views {{
	View views[8];
}};

struct _meshes {{
	Mesh meshes[{max_instances}];
}};

struct _meshlets {{
	Meshlet meshlets[{max_meshlets}];
}};

struct _vertex_positions {{
	packed_float3 positions[1];
}};

struct _vertex_normals {{
	packed_float3 normals[1];
}};

struct SkinnedVertex {{
	float4 position;
	float4 normal;
}};

struct _skinned_vertices {{
	SkinnedVertex vertices[1];
}};

struct _vertex_uvs {{
	packed_float2 uvs[1];
}};

struct _vertex_indices {{
	ushort vertex_indices[1];
}};

struct _primitive_indices {{
	uchar primitive_indices[1];
}};

struct _set0 {{
	constant _views* views [[id(0)]];
	constant _meshes* meshes [[id(1)]];
	constant _vertex_positions* vertex_positions [[id(2)]];
	constant _vertex_normals* vertex_normals [[id(3)]];
	device const _skinned_vertices* skinned_vertices [[id(4)]];
	constant _vertex_uvs* vertex_uvs [[id(5)]];
	constant _vertex_indices* vertex_indices [[id(6)]];
	constant _primitive_indices* primitive_indices [[id(7)]];
	constant _meshlets* meshlets [[id(8)]];
}};

static void extract_frustum_planes(float4x4 matrix, thread float4* planes) {{
	float4x4 mt = transpose(matrix);
	planes[0] = mt[3] + mt[0];
	planes[1] = mt[3] - mt[0];
	planes[2] = mt[3] - mt[1];
	planes[3] = mt[3] + mt[1];
	planes[4] = mt[2];
	planes[5] = mt[3] - mt[2];

	for (uint i = 0; i < 6; ++i) {{
		planes[i] *= rsqrt(max(dot(planes[i].xyz, planes[i].xyz), 0.000000000001f));
	}}
}}

static bool sphere_intersects_frustum(thread float4* planes, float3 center, float radius) {{
	for (uint i = 0; i < 6; ++i) {{
		if (dot(center, planes[i].xyz) + planes[i].w < -radius) {{
			return false;
		}}
	}}

	return true;
}}

static float3 transform_world_to_object(float4x3 model, float3 world_position, float determinant) {{
	float3 object_x = model[0];
	float3 object_y = model[1];
	float3 object_z = model[2];
	float3 world_delta = world_position - model[3];
	float inverse_determinant = 1.0f / determinant;

	return float3(
		dot(cross(object_y, object_z) * inverse_determinant, world_delta),
		dot(cross(object_z, object_x) * inverse_determinant, world_delta),
		dot(cross(object_x, object_y) * inverse_determinant, world_delta)
	);
}}

static float4x4 model_matrix(Mesh mesh) {{
	return float4x4(
		float4(mesh.model[0], 0.0),
		float4(mesh.model[1], 0.0),
		float4(mesh.model[2], 0.0),
		float4(mesh.model[3], 1.0)
	);
}}

static bool cone_is_backfacing(Mesh mesh, Meshlet meshlet, View view) {{
	float4x3 model = mesh.model;
	float determinant = dot(cross(model[0], model[1]), model[2]);
	if (determinant <= 0.000001f || meshlet.cone_apex_cutoff.w > 1.0f) {{
		return false;
	}}

	float3 camera_position_world = view.inverse_view[3].xyz;
	float3 camera_position_object = transform_world_to_object(model, camera_position_world, determinant);
	float3 cone_view = meshlet.cone_apex_cutoff.xyz - camera_position_object;
	float cone_view_length_squared = dot(cone_view, cone_view);
	float cone_axis_length_squared = dot(meshlet.cone_axis.xyz, meshlet.cone_axis.xyz);

	if (cone_view_length_squared <= 0.000000000001f || cone_axis_length_squared <= 0.000000000001f) {{
		return false;
	}}

	return dot(cone_view * rsqrt(cone_view_length_squared), meshlet.cone_axis.xyz * rsqrt(cone_axis_length_squared)) >= meshlet.cone_apex_cutoff.w;
}}

static bool meshlet_is_visible(Mesh mesh, Meshlet meshlet, View view) {{
	float4 planes[6];
	float4x4 model = model_matrix(mesh);
	extract_frustum_planes(view.view_projection * model, planes);
	bool frustum_visible = sphere_intersects_frustum(planes, meshlet.center_radius.xyz, meshlet.center_radius.w);

	return frustum_visible && !cone_is_backfacing(mesh, meshlet, view);
}}

[[object, max_total_threadgroups_per_mesh_grid({task_group_size})]]
void besl_task_main(
	constant PushConstant& push_constant [[buffer(15)]],
	constant _set0& set0 [[buffer(16)]],
	uint meshlet_thread_index [[thread_position_in_grid]],
	uint thread_index [[thread_index_in_threadgroup]],
	object_data ObjectPayload& payload [[payload]],
	mesh_grid_properties mesh_grid
) {{
	Mesh mesh = set0.meshes->meshes[push_constant.instance_index];
	View view = set0.views->views[{view_lookup}];
	threadgroup atomic_uint visible_count;
	if (thread_index == 0) {{
		atomic_store_explicit(&visible_count, 0u, memory_order_relaxed);
	}}
	threadgroup_barrier(mem_flags::mem_threadgroup);

	if (meshlet_thread_index < mesh.meshlet_count) {{
		uint meshlet_index = mesh.base_meshlet_index + meshlet_thread_index;
		Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];

		// Bind-pose meshlet bounds are not conservative for animation, so posed instances skip task culling.
		if (mesh.skinned_base_vertex_index != 0xffffffffu || meshlet_is_visible(mesh, meshlet, view)) {{
			uint payload_index = atomic_fetch_add_explicit(&visible_count, 1u, memory_order_relaxed);
			payload.meshlet_indices[payload_index] = meshlet_index;
		}}
	}}

	threadgroup_barrier(mem_flags::mem_threadgroup);
	if (thread_index == 0) {{
		mesh_grid.set_threadgroups_per_grid(uint3(atomic_load_explicit(&visible_count, memory_order_relaxed), 1, 1));
	}}
}}
"#,
		push_constant_fields = push_constant_fields,
		view_lookup = view_lookup,
		task_group_size = MESHLET_CULLING_TASK_GROUP_SIZE,
		max_instances = MAX_INSTANCES,
		max_meshlets = MAX_MESHLETS,
	)
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
	float4x3 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
	uint meshlet_count;
	uint skinned_base_vertex_index;
	uint padding0;
}};

struct Meshlet {{
	uint primitive_offset;
	uint triangle_offset;
	uint primitive_count;
	uint triangle_count;
	float4 center_radius;
	float4 cone_apex_cutoff;
	float4 cone_axis;
}};

struct ObjectPayload {{
	uint meshlet_indices[{task_group_size}];
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

struct SkinnedVertex {{
	float4 position;
	float4 normal;
}};

struct _skinned_vertices {{
	SkinnedVertex vertices[{max_vertices}];
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
	device const _skinned_vertices* skinned_vertices [[id(4)]];
	constant _vertex_uvs* vertex_uvs [[id(5)]];
	constant _vertex_indices* vertex_indices [[id(6)]];
	constant _primitive_indices* primitive_indices [[id(7)]];
	constant _meshlets* meshlets [[id(8)]];
}};

[[mesh]] void besl_main(
	constant PushConstant& push_constant [[buffer(15)]],
	constant _set0& set0 [[buffer(16)]],
	uint threadgroup_position [[threadgroup_position_in_grid]],
	uint thread_index [[thread_index_in_threadgroup]],
	const object_data ObjectPayload& payload [[payload]],
	metal::mesh<VertexOutput, PrimitiveOutput, 64, 126, topology::triangle> out_mesh
) {{
	Mesh mesh = set0.meshes->meshes[push_constant.instance_index];
	float4x3 model = mesh.model;
	View view = set0.views->views[{view_lookup}];
	uint meshlet_index = payload.meshlet_indices[threadgroup_position];
	Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];
	uint primitive_index = thread_index;

	if (thread_index == 0) {{
		out_mesh.set_primitive_count(uint(meshlet.triangle_count));
	}}

	if (primitive_index < uint(meshlet.primitive_count)) {{
		uint relative_vertex_index = uint(set0.vertex_indices->vertex_indices[
			mesh.base_primitive_index + meshlet.primitive_offset + primitive_index
		]);
		uint vertex_index = mesh.base_vertex_index + relative_vertex_index;
		float4 position = mesh.skinned_base_vertex_index == 0xffffffffu
			? float4(float3(set0.vertex_positions->positions[vertex_index]), 1.0)
			: set0.skinned_vertices->vertices[mesh.skinned_base_vertex_index + relative_vertex_index].position;
		out_mesh.set_vertex(primitive_index, VertexOutput{{ .position = view.view_projection * float4(model * position, 1.0) }});
	}}

	if (primitive_index < uint(meshlet.triangle_count)) {{
		uint triangle_base_index = mesh.base_triangle_index + meshlet.triangle_offset + primitive_index;
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
		task_group_size = MESHLET_CULLING_TASK_GROUP_SIZE,
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

pub fn get_visibility_pass_mesh_hlsl_source() -> String {
	build_mesh_pass_hlsl_source("uint instance_index;", "0")
}

pub fn get_visibility_pass_task_msl_source() -> String {
	build_mesh_culling_task_msl_source("\tuint instance_index;", "0")
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

pub fn get_shadow_pass_mesh_hlsl_source() -> String {
	build_mesh_pass_hlsl_source("uint instance_index;\nuint view_index;", "push_constant.view_index")
}

pub fn get_shadow_pass_task_msl_source() -> String {
	build_mesh_culling_task_msl_source("\tuint instance_index;\n\tuint view_index;", "push_constant.view_index")
}

fn build_mesh_pass_hlsl_source(push_constant_fields: &str, view_lookup: &str) -> String {
	format!(
		r#"
#pragma pack_matrix(row_major)

struct PushConstant {{
{push_constant_fields}
}};

struct MeshVertex {{
	float4 position : SV_Position;
}};

struct MeshPrimitive {{
	uint instance_index : INSTANCE_INDEX;
	uint primitive_index : PRIMITIVE_INDEX;
}};

struct SkinnedVertex {{
	float4 position;
	float4 normal;
}};

struct Mesh {{
	float4x3 model;
	uint material_index;
	uint base_vertex_index;
	uint base_primitive_index;
	uint base_triangle_index;
	uint base_meshlet_index;
	uint meshlet_count;
	uint skinned_base_vertex_index;
	uint padding0;
}};

struct Meshlet {{
	uint primitive_offset;
	uint triangle_offset;
	uint primitive_count;
	uint triangle_count;
	float4 center_radius;
	float4 cone_apex_cutoff;
	float4 cone_axis;
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

ConstantBuffer<PushConstant> push_constant : register(b0, space1);
StructuredBuffer<View> views : register(t0, space0);
StructuredBuffer<Mesh> meshes : register(t1, space0);
StructuredBuffer<float3> vertex_positions : register(t2, space0);
StructuredBuffer<float3> vertex_normals : register(t3, space0);
StructuredBuffer<SkinnedVertex> skinned_vertices : register(t4, space0);
StructuredBuffer<float2> vertex_uvs : register(t5, space0);
StructuredBuffer<uint> vertex_indices : register(t6, space0);
StructuredBuffer<uint> primitive_indices : register(t7, space0);
StructuredBuffer<Meshlet> meshlets : register(t8, space0);

float4 load_float4(StructuredBuffer<uint> buffer, uint offset) {{
	return float4(
		asfloat(buffer[offset + 0]),
		asfloat(buffer[offset + 1]),
		asfloat(buffer[offset + 2]),
		asfloat(buffer[offset + 3])
	);
}}

float3 load_float3(StructuredBuffer<uint> buffer, uint offset) {{
	return float3(
		asfloat(buffer[offset + 0]),
		asfloat(buffer[offset + 1]),
		asfloat(buffer[offset + 2])
	);
}}

uint load_u16(StructuredBuffer<uint> buffer, uint index) {{
	uint word = buffer[index >> 1];
	uint shift = (index & 1) * 16;
	return (word >> shift) & 0xffffu;
}}

uint load_u8(StructuredBuffer<uint> buffer, uint index) {{
	uint word = buffer[index >> 2];
	uint shift = (index & 3) * 8;
	return (word >> shift) & 0xffu;
}}

float4x4 load_view_projection(uint view_index) {{
	return views[view_index].view_projection;
}}

Mesh load_mesh(uint mesh_index) {{
	return meshes[mesh_index];
}}

Meshlet load_meshlet(uint meshlet_index) {{
	return meshlets[meshlet_index];
}}

float4 transform_position(Mesh mesh, float3 position) {{
	// ShaderMatrix4x3 uploads four float3 affine rows; HLSL multiplies a float4 position by float4x3 to recover world space.
	return float4(mul(float4(position, 1.0f), mesh.model), 1.0f);
}}

[numthreads(128, 1, 1)]
[outputtopology("triangle")]
void main(
	uint3 group_id : SV_GroupID,
	uint thread_index : SV_GroupIndex,
	out vertices MeshVertex vertices[64],
	out indices uint3 triangles[126],
	out primitives MeshPrimitive primitives[126]
) {{
	Mesh mesh = load_mesh(push_constant.instance_index);
	uint meshlet_index = mesh.base_meshlet_index + group_id.x;
	Meshlet meshlet = load_meshlet(meshlet_index);
	float4x4 view_projection = load_view_projection({view_lookup});

	SetMeshOutputCounts(meshlet.primitive_count, meshlet.triangle_count);

	if (thread_index < meshlet.primitive_count) {{
		uint relative_vertex_index = load_u16(vertex_indices, mesh.base_primitive_index + meshlet.primitive_offset + thread_index);
		uint vertex_index = mesh.base_vertex_index + relative_vertex_index;
		float3 position = mesh.skinned_base_vertex_index == 0xffffffffu
			? vertex_positions[vertex_index]
			: skinned_vertices[mesh.skinned_base_vertex_index + relative_vertex_index].position.xyz;
		vertices[thread_index].position = mul(view_projection, transform_position(mesh, position));
	}}

	if (thread_index < meshlet.triangle_count) {{
		uint triangle_base_index = mesh.base_triangle_index + meshlet.triangle_offset + thread_index;
		triangles[thread_index] = uint3(
			load_u8(primitive_indices, triangle_base_index * 3u + 0u),
			load_u8(primitive_indices, triangle_base_index * 3u + 1u),
			load_u8(primitive_indices, triangle_base_index * 3u + 2u)
		);
		primitives[thread_index].instance_index = push_constant.instance_index;
		primitives[thread_index].primitive_index = (meshlet_index << 8) | (thread_index & 255u);
	}}
}}
"#,
		push_constant_fields = push_constant_fields,
		view_lookup = view_lookup,
	)
}

pub(crate) const VISIBILITY_PASS_FRAGMENT_SOURCE_MSL: &str = r#"
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

pub(crate) const VISIBILITY_PASS_FRAGMENT_SOURCE: &str = r#"
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

pub const VISIBILITY_PASS_FRAGMENT_SOURCE_HLSL: &str = r#"
struct FragmentInput {
	uint instance_index : INSTANCE_INDEX;
	uint primitive_index : PRIMITIVE_INDEX;
};

struct FragmentOutput {
	uint primitive_index : SV_Target0;
	uint instance_id : SV_Target1;
};

FragmentOutput main(FragmentInput input) {
	FragmentOutput output;
	output.primitive_index = input.primitive_index;
	output.instance_id = input.instance_index;
	return output;
}
"#;

/// Creates a BESL shader source definition for a visibility compute pass.
fn visibility_compute_shader(
	threadgroup_extent: Extent,
	build_program: fn() -> besl::NodeReference,
) -> ShaderSourceDefinition<'static> {
	ShaderSourceDefinition::Besl {
		settings: ShaderGenerationSettings::compute(threadgroup_extent),
		main_node: build_program(),
	}
}

pub fn get_material_count_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(32), build_material_count_program)
}

fn preserve_visibility_compute_layout(program: &besl::NodeReference) -> besl::NodeReference {
	let main = program
		.get_main()
		.expect("Missing BESL main function. The most likely cause is invalid visibility shader source.");
	let inputs = [
		"views",
		"mesh_data",
		"material_count_buffer",
		"material_offset_buffer",
		"material_offset_scratch_buffer",
		"material_evaluation_dispatches",
		"pixel_mapping_buffer",
		"triangle_index",
		"instance_index_render_target",
	]
	.into_iter()
	.filter_map(|name| program.get_descendant(name))
	.collect();

	// The raw node is intentionally empty. Its inputs keep the complete visibility descriptor layout reachable
	// so Metal's compact argument-buffer IDs match the descriptor set template even when a shader only touches
	// a subset of the bindings.
	main.borrow_mut()
		.add_child(besl::Node::raw(Some(String::new()), None, Some(String::new()), inputs, Vec::new()).into());

	main
}

fn compile_visibility_compute_program(source: &str, pixel_mapping_entries: usize) -> besl::NodeReference {
	let program = besl::compile_to_besl(source, Some(build_visibility_compute_root(pixel_mapping_entries)))
		.expect("Failed to compile visibility BESL shader. The most likely cause is invalid BESL syntax.");
	preserve_visibility_compute_layout(&program)
}

fn build_material_count_program() -> besl::NodeReference {
	let source = r#"
	main: fn () -> void {
		let coord: vec2u = thread_id();
		guard_image_bounds(instance_index_render_target, coord);
		let pixel_instance_index: u32 = image_load_u32(instance_index_render_target, coord);

		if (pixel_instance_index < 4294967295 && pixel_instance_index < 1024) {
			let material_index: u32 = mesh_data.meshes[pixel_instance_index].material_index;

			if (material_index < 1024) {
				atomic_add(material_count_buffer.material_count[material_index], 1);
			}
		}
	}
	"#;

	compile_visibility_compute_program(source, 1)
}

pub fn get_material_offset_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(1), build_material_offset_program)
}

fn build_material_offset_program() -> besl::NodeReference {
	let source = r#"
	main: fn () -> void {
		let coord: vec2u = thread_id();

		if (coord.x == 0 && coord.y == 0) {
			let sum: u32 = 0;

			for (let i: u32 = 0; i < 1024; i = i + 1) {
				let count: u32 = atomic_load(material_count_buffer.material_count[i]);
				material_offset_buffer.material_offset[i] = sum;
				atomic_store(material_offset_scratch_buffer.material_offset_scratch[i], sum);
				material_evaluation_dispatches.material_evaluation_dispatches[i] = vec4u((count + 127) / 128, 1, 1, 0);
				sum = sum + count;
			}
		}
	}
	"#;

	compile_visibility_compute_program(source, 1)
}

pub fn get_pixel_mapping_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(32), build_pixel_mapping_program)
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
					pixel_mapping_buffer.pixel_mapping[pixel_mapping_index] = vec2u16(coord.x + 1, coord.y + 1);
				}
			}
		}
	}
	"#
	.replace("__MAX_PIXEL_MAPPING_ENTRIES__", &MAX_PIXEL_MAPPING_ENTRIES.to_string());

	compile_visibility_compute_program(&source, MAX_PIXEL_MAPPING_ENTRIES)
}

fn build_visibility_compute_root(pixel_mapping_entries: usize) -> besl::Node {
	let mut root = besl::Node::root();
	let mat4x3f_t = root.get_child("mat4x3f").unwrap();
	let u32_t = root.get_child("u32").unwrap();
	let texture_2d = root.get_child("Texture2D").unwrap();
	let vec2u_t = root.get_child("vec2u").unwrap();
	let vec2u16_t = root.get_child("vec2u16").unwrap();
	let vec4u_t = root.get_child("vec4u").unwrap();
	let mesh = root.add_child(
		besl::Node::r#struct(
			"Mesh",
			vec![
				besl::Node::member("model", mat4x3f_t).into(),
				besl::Node::member("material_index", u32_t.clone()).into(),
				besl::Node::member("base_vertex_index", u32_t.clone()).into(),
				besl::Node::member("base_primitive_index", u32_t.clone()).into(),
				besl::Node::member("base_triangle_index", u32_t.clone()).into(),
				besl::Node::member("base_meshlet_index", u32_t.clone()).into(),
				besl::Node::member("meshlet_count", u32_t.clone()).into(),
				besl::Node::member("skinned_base_vertex_index", u32_t.clone()).into(),
				besl::Node::member("padding0", u32_t.clone()).into(),
			],
		)
		.into(),
	);
	let atomic_u32 = root.add_child(besl::Node::r#struct("atomicu32", Vec::new()).into());
	let views_member = besl::Node::array("views", u32_t.clone(), 1);
	let meshes_member = besl::Node::array("meshes", mesh, MAX_INSTANCES);
	let material_count_member = besl::Node::array("material_count", atomic_u32.clone(), MAX_MATERIALS);
	let material_offset_member = besl::Node::array("material_offset", u32_t.clone(), MAX_MATERIALS);
	let material_offset_scratch_member = besl::Node::array("material_offset_scratch", atomic_u32.clone(), MAX_MATERIALS);
	let material_evaluation_dispatches_member =
		besl::Node::array("material_evaluation_dispatches", vec4u_t.clone(), MAX_MATERIALS);
	let pixel_mapping_member = besl::Node::array("pixel_mapping", vec2u16_t, pixel_mapping_entries);

	root.add_children(vec![
		besl::Node::binding(
			"views",
			besl::BindingTypes::Buffer {
				members: vec![views_member],
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
				members: vec![material_count_member],
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
				members: vec![material_offset_member],
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
				members: vec![material_evaluation_dispatches_member],
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
			r#type: atomic_u32.clone(),
		})
		.into(),
		besl::Node::new(besl::Nodes::Parameter {
			name: "increment".to_string(),
			r#type: u32_t.clone(),
		})
		.into(),
	]);

	let atomic_load = root.add_child(besl::Node::intrinsic("atomic_load", Vec::new(), u32_t.clone()).into());
	atomic_load
		.borrow_mut()
		.add_children(vec![besl::Node::new(besl::Nodes::Parameter {
			name: "value".to_string(),
			r#type: atomic_u32.clone(),
		})
		.into()]);

	let atomic_store =
		root.add_child(besl::Node::intrinsic("atomic_store", Vec::new(), root.get_child("void").unwrap()).into());
	atomic_store.borrow_mut().add_children(vec![
		besl::Node::new(besl::Nodes::Parameter {
			name: "value".to_string(),
			r#type: atomic_u32,
		})
		.into(),
		besl::Node::new(besl::Nodes::Parameter {
			name: "stored".to_string(),
			r#type: u32_t,
		})
		.into(),
	]);
	root
}

pub fn get_gtao_blur_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(8), build_gtao_blur_program)
}

pub fn get_gtao_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(8), build_gtao_program)
}

pub fn get_gtao_bitfield_blur_x_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(8), build_gtao_bitfield_blur_x_program)
}

pub fn get_gtao_bitfield_shader() -> ShaderSourceDefinition<'static> {
	visibility_compute_shader(Extent::square(8), build_gtao_bitfield_program)
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
		let inverse_projection: mat4f = views.views[0].inverse_projection;
		let view_fov: vec2f = views.views[0].fov;

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

fn build_gtao_bitfield_program() -> besl::NodeReference {
	// The pass dispatches full-resolution pixels even though each destination texel packs 32 horizontal samples.
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
		let extent: vec2u = texture_size(visibility_depth);
		if (coord.x >= extent.x) {
			return;
		}
		if (coord.y >= extent.y) {
			return;
		}
		let inverse_projection: mat4f = views.views[0].inverse_projection;
		let view_fov: vec2f = views.views[0].fov;

		let center_depth: f32 = fetch(visibility_depth, coord).x;
		if (center_depth == 0.0) {
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
		let inverse_projection: mat4f = views.views[0].inverse_projection;

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

	root.add_children(vec![besl::Node::binding(
		"views",
		besl::BindingTypes::Buffer {
			members: vec![besl::Node::array("views", view_type, 8)],
		},
		0,
		0,
		true,
		false,
	)
	.into()]);

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

/// The `ShaderMeshletData` struct stores meshlet offsets and object-space culling bounds for GPU visibility passes.
#[derive(Copy, Clone)]
#[repr(C, align(16))]
pub(super) struct ShaderMeshletData {
	/// Base index into the vertex indices buffer
	/// ```glsl
	/// vertex_index = mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + gl_LocalInvocationID.x];
	/// ```
	primitive_offset: u32,
	/// Base index into the primitive/triangle indices buffer
	/// This is stored as index / 3, as the meshlet contains 3 indices per triangle
	/// ```glsl
	/// triangle_index = primitive_indices.primitive_indices[(meshlet.triangle_offset + gl_LocalInvocationID.x) * 3 + 0..2]
	/// ```
	triangle_offset: u32,
	/// The number of primitives in the meshlet
	/// Primitives are meshlet local indices
	primitive_count: u32,
	// The number of triangles in the meshlet
	triangle_count: u32,
	/// Object-space bounding sphere encoded as xyz center and w radius.
	center_radius: [f32; 4],
	/// Object-space normal-cone apex encoded as xyz apex and w cutoff.
	cone_apex_cutoff: [f32; 4],
	/// Object-space normal-cone axis encoded as xyz axis.
	cone_axis: [f32; 4],
}

#[cfg(test)]
mod tests {
	use besl::vm::{
		output_slot, DescriptorBindings, DescriptorSlot, ExecutableProgram, ExecutionConfig, MeshOutputs, SpecializationValues,
		Texture, Value,
	};

	use super::{
		build_gtao_bitfield_blur_x_program, build_gtao_bitfield_program, build_gtao_blur_program, build_gtao_program,
		build_material_count_program, build_material_offset_program, build_mesh_program_from_source,
		build_pixel_mapping_program, get_shadow_pass_mesh_msl_source, get_visibility_pass_mesh_hlsl_source,
		get_visibility_pass_mesh_msl_source, get_visibility_pass_task_msl_source, MESHLET_DATA_BINDING, MESH_DATA_BINDING,
		PRIMITIVE_INDICES_BINDING, SKINNED_VERTICES_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING,
		VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
	};
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const VIEWS_SLOT: DescriptorSlot = DescriptorSlot::new(0, 0);
	const MESH_DATA_SLOT: DescriptorSlot = DescriptorSlot::new(0, 1);
	const MATERIAL_COUNT_SLOT: DescriptorSlot = DescriptorSlot::new(1, 0);
	const MATERIAL_OFFSET_SLOT: DescriptorSlot = DescriptorSlot::new(1, 1);
	const MATERIAL_OFFSET_SCRATCH_SLOT: DescriptorSlot = DescriptorSlot::new(1, 2);
	const MATERIAL_DISPATCH_SLOT: DescriptorSlot = DescriptorSlot::new(1, 3);
	const PIXEL_MAPPING_SLOT: DescriptorSlot = DescriptorSlot::new(1, 4);
	const INSTANCE_INDEX_SLOT: DescriptorSlot = DescriptorSlot::new(1, 7);
	const VERTEX_POSITIONS_SLOT: DescriptorSlot = DescriptorSlot::new(0, 2);
	const SKINNED_VERTICES_SLOT: DescriptorSlot = DescriptorSlot::new(0, 4);
	const VERTEX_INDICES_SLOT: DescriptorSlot = DescriptorSlot::new(0, 6);
	const PRIMITIVE_INDICES_SLOT: DescriptorSlot = DescriptorSlot::new(0, 7);
	const MESHLETS_SLOT: DescriptorSlot = DescriptorSlot::new(0, 8);
	const FIXTURE_INSTANCE_INDEX: usize = 3;
	const FIXTURE_MESHLET_INDEX: usize = 5;
	const MESH_TEST_INSTRUCTION_LIMIT: usize = 4_000_000;

	/// Returns a column-major identity matrix in the BESL VM representation.
	fn identity_matrix() -> [f32; 16] {
		[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
	}

	/// Builds the exact production visibility mesh main for VM execution.
	fn visibility_mesh_program() -> besl::NodeReference {
		build_mesh_program_from_source(
			r#"
		main: fn () -> void {
			let view: View = views.views[0];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
			besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]),
		)
	}

	/// Builds the exact production shadow mesh main for VM execution.
	fn shadow_mesh_program() -> besl::NodeReference {
		build_mesh_program_from_source(
			r#"
		main: fn () -> void {
			let view_index: u32 = push_constant.view_index;
			let view: View = views.views[view_index];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
			besl::parser::Node::push_constant(vec![
				besl::parser::Node::member("instance_index", "u32"),
				besl::parser::Node::member("view_index", "u32"),
			]),
		)
	}

	/// Creates one identity-transformed triangle meshlet in the production visibility buffer layouts.
	fn mesh_triangle_buffers(
		program: &ExecutableProgram,
	) -> (
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
		besl::vm::Buffer,
	) {
		let mut views = buffer(program, VIEWS_SLOT);
		views
			.write_indexed_field("views", 0, "view_projection", Value::Mat4F(identity_matrix()))
			.expect("Failed to initialize the mesh view. The most likely cause is a drifted View layout.");

		let mut meshes = buffer(program, MESH_DATA_SLOT);
		meshes
			.write_indexed_field(
				"meshes",
				FIXTURE_INSTANCE_INDEX,
				"model",
				Value::Mat4x3F([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
			)
			.expect("Failed to initialize the mesh model matrix. The most likely cause is a drifted Mesh layout.");
		for (field, value) in [
			("base_vertex_index", 0),
			("base_primitive_index", 0),
			("base_triangle_index", 0),
			("base_meshlet_index", FIXTURE_MESHLET_INDEX as u32),
			("meshlet_count", 1),
			("skinned_base_vertex_index", u32::MAX),
		] {
			meshes
				.write_indexed_field("meshes", FIXTURE_INSTANCE_INDEX, field, Value::U32(value))
				.expect("Failed to initialize a mesh offset. The most likely cause is a drifted Mesh layout.");
		}

		let mut positions = buffer(program, VERTEX_POSITIONS_SLOT);
		for (index, position) in [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]].into_iter().enumerate() {
			positions
				.write_indexed("positions", index, Value::Vec3F(position))
				.expect("Failed to initialize a mesh vertex. The most likely cause is a drifted position layout.");
		}
		let skinned_vertices = buffer(program, SKINNED_VERTICES_SLOT);

		let mut vertex_indices = buffer(program, VERTEX_INDICES_SLOT);
		let mut primitive_indices = buffer(program, PRIMITIVE_INDICES_SLOT);
		for (index, value) in [0, 1, 2].into_iter().enumerate() {
			vertex_indices
				.write_indexed("vertex_indices", index, Value::U16(value))
				.expect("Failed to initialize a vertex index. The most likely cause is a drifted index layout.");
			primitive_indices
				.write_indexed("primitive_indices", index, Value::U8(value as u8))
				.expect("Failed to initialize a triangle index. The most likely cause is a drifted primitive layout.");
		}

		let mut meshlets = buffer(program, MESHLETS_SLOT);
		for (field, value) in [
			("primitive_offset", 0),
			("triangle_offset", 0),
			("primitive_count", 3),
			("triangle_count", 1),
		] {
			meshlets
				.write_indexed_field("meshlets", FIXTURE_MESHLET_INDEX, field, Value::U32(value))
				.expect("Failed to initialize a meshlet field. The most likely cause is a drifted Meshlet layout.");
		}

		(
			views,
			meshes,
			positions,
			skinned_vertices,
			vertex_indices,
			primitive_indices,
			meshlets,
		)
	}

	/// Executes one production mesh main and verifies its complete one-triangle output contract.
	fn assert_triangle_mesh_program(
		program: besl::NodeReference,
		has_view_index: bool,
		skinned_positions: Option<[[f32; 4]; 3]>,
	) {
		let program = crate::rendering::shader_vm_test::compile(program);
		let (
			mut views,
			mut meshes,
			mut positions,
			mut skinned_vertices,
			mut vertex_indices,
			mut primitive_indices,
			mut meshlets,
		) = mesh_triangle_buffers(&program);
		if let Some(skinned_positions) = skinned_positions {
			const SKINNED_BASE_VERTEX: usize = 7;
			meshes
				.write_indexed_field(
					"meshes",
					FIXTURE_INSTANCE_INDEX,
					"skinned_base_vertex_index",
					Value::U32(SKINNED_BASE_VERTEX as u32),
				)
				.expect("Failed to select skinned mesh vertices. The most likely cause is a drifted Mesh layout.");
			for (index, position) in skinned_positions.into_iter().enumerate() {
				skinned_vertices
					.write_indexed_field("vertices", SKINNED_BASE_VERTEX + index, "position", Value::Vec4F(position))
					.expect(
						"Failed to initialize a skinned mesh vertex. The most likely cause is a drifted SkinnedVertex layout.",
					);
			}
		}
		let push_constant_layout = program
			.push_constant_layout()
			.expect(
				"Missing mesh push constant layout. The most likely cause is that the production mesh main no longer uses it.",
			)
			.clone();
		let mut push_constant = besl::vm::Buffer::new(push_constant_layout);
		push_constant
			.write("instance_index", Value::U32(FIXTURE_INSTANCE_INDEX as u32))
			.expect("Failed to initialize the mesh instance index. The most likely cause is a drifted push constant layout.");
		if has_view_index {
			push_constant
				.write("view_index", Value::U32(0))
				.expect("Failed to initialize the shadow view index. The most likely cause is a drifted push constant layout.");
		}

		let mut out_instance_indices = buffer(&program, output_slot(0));
		let mut out_primitive_indices = buffer(&program, output_slot(1));
		let mut mesh_outputs = MeshOutputs::new();
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut meshes);
			descriptors.bind_buffer(VERTEX_POSITIONS_SLOT, &mut positions);
			descriptors.bind_buffer(SKINNED_VERTICES_SLOT, &mut skinned_vertices);
			descriptors.bind_buffer(VERTEX_INDICES_SLOT, &mut vertex_indices);
			descriptors.bind_buffer(PRIMITIVE_INDICES_SLOT, &mut primitive_indices);
			descriptors.bind_buffer(MESHLETS_SLOT, &mut meshlets);
			descriptors.bind_buffer(output_slot(0), &mut out_instance_indices);
			descriptors.bind_buffer(output_slot(1), &mut out_primitive_indices);
			descriptors.bind_push_constant(&mut push_constant);
			descriptors.bind_mesh_outputs(&mut mesh_outputs);

			// Mesh invocations share their capture just as lanes in one production mesh workgroup share output arrays.
			for thread_idx in 0..3 {
				let config = ExecutionConfig::new(MESH_TEST_INSTRUCTION_LIMIT)
					.with_call_depth_limit(128)
					.with_thread_idx(thread_idx)
					.with_threadgroup_position(0);
				program.run_main_with_config(&mut descriptors, &config).expect(
					"Failed to execute a production mesh shader with the BESL VM. The most likely cause is missing mesh intrinsic support or an invalid fixture binding.",
				);
			}
		}

		assert_eq!(mesh_outputs.vertex_count(), 3);
		assert_eq!(mesh_outputs.primitive_count(), 1);
		let expected_positions =
			skinned_positions.unwrap_or([[-1.0, -1.0, 0.0, 1.0], [1.0, -1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
		for (index, expected) in expected_positions.into_iter().enumerate() {
			let actual = mesh_outputs
				.vertex_position(index)
				.expect("Missing mesh vertex output. The most likely cause is that a mesh invocation did not write its lane.");
			assert_rgba_close(actual, expected, 0.00001);
		}
		assert_eq!(mesh_outputs.triangle(0), Some([0, 1, 2]));
		assert_eq!(
			read_u32(&out_instance_indices, "out_instance_index", 0),
			FIXTURE_INSTANCE_INDEX as u32
		);
		assert_eq!(
			read_u32(&out_primitive_indices, "out_primitive_index", 0),
			(FIXTURE_MESHLET_INDEX as u32) << 8
		);
	}

	/// Verifies visibility mesh output geometry and metadata through the BESL VM.
	#[test]
	fn visibility_mesh_main_emits_identity_triangle_and_metadata() {
		assert_triangle_mesh_program(visibility_mesh_program(), false, None);
	}

	/// Verifies that posed instances source raster positions from their frame-local deformation range.
	#[test]
	fn visibility_mesh_main_reads_skinned_positions() {
		assert_triangle_mesh_program(
			visibility_mesh_program(),
			false,
			Some([[2.0, 3.0, 4.0, 1.0], [5.0, 6.0, 7.0, 1.0], [8.0, 9.0, 10.0, 1.0]]),
		);
	}

	/// Verifies shadow mesh output geometry and metadata through the BESL VM.
	#[test]
	fn shadow_mesh_main_emits_identity_triangle_and_metadata() {
		assert_triangle_mesh_program(shadow_mesh_program(), true, None);
	}

	/// Creates the minimum camera data shared by the GTAO shader fixtures.
	fn gtao_views(program: &ExecutableProgram) -> besl::vm::Buffer {
		let mut views = buffer(program, VIEWS_SLOT);
		views
			.write_indexed_field("views", 0, "inverse_projection", Value::Mat4F(identity_matrix()))
			.expect("Failed to initialize the GTAO inverse projection. The most likely cause is a drifted View layout.");
		views
			.write_indexed_field("views", 0, "fov", Value::Vec2F([60.0, 60.0]))
			.expect("Failed to initialize the GTAO field of view. The most likely cause is a drifted View layout.");
		views
	}

	/// Reads one unsigned scalar from an indexed visibility buffer member.
	fn read_u32(buffer: &besl::vm::Buffer, member: &str, index: usize) -> u32 {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM u32 array element. The most likely cause is a drifted visibility buffer layout.")
		{
			Value::U32(value) => value,
			value => panic!(
				"Unexpected visibility buffer value: {value:?}. The most likely cause is a drifted material buffer type."
			),
		}
	}

	/// Reads one dispatch tuple from an indexed visibility buffer member.
	fn read_vec4u(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u32; 4] {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM vec4u array element. The most likely cause is a drifted visibility buffer layout.")
		{
			Value::Vec4U(value) => value,
			value => panic!(
				"Unexpected visibility dispatch value: {value:?}. The most likely cause is a drifted dispatch buffer type."
			),
		}
	}

	/// Reads one packed pixel coordinate from the visibility mapping buffer.
	fn read_vec2u16(buffer: &besl::vm::Buffer, member: &str, index: usize) -> [u16; 2] {
		match buffer
			.read_indexed(member, index)
			.expect("Failed to read a VM vec2u16 array element. The most likely cause is a drifted pixel mapping layout.")
		{
			Value::Vec2U16(value) => value,
			value => panic!(
				"Unexpected visibility pixel mapping value: {value:?}. The most likely cause is a drifted mapping buffer type."
			),
		}
	}

	/// Exercises the production material prepasses as one stateful VM pipeline.
	#[test]
	fn visibility_material_compute_pipeline_counts_offsets_and_maps_valid_pixels() {
		let material_count_program = crate::rendering::shader_vm_test::compile(build_material_count_program());
		let material_offset_program = crate::rendering::shader_vm_test::compile(build_material_offset_program());
		let pixel_mapping_program = crate::rendering::shader_vm_test::compile(build_pixel_mapping_program());

		// Three visible instances span two materials; the fourth texel is the renderer's empty-pixel sentinel.
		let mut mesh_data = buffer(&material_count_program, MESH_DATA_SLOT);
		for (mesh_index, material_index) in [(0, 2), (1, 5), (2, 2)] {
			mesh_data
				.write_indexed_field("meshes", mesh_index, "material_index", Value::U32(material_index))
				.expect("Failed to initialize a VM mesh. The most likely cause is a drifted Mesh buffer layout.");
		}

		let mut instance_indices = Texture::new(2, 2)
			.expect("Failed to create the visibility index fixture. The most likely cause is an invalid test extent.");
		for (coordinate, instance_index) in [([0, 0], 0), ([1, 0], 1), ([0, 1], u32::MAX), ([1, 1], 2)] {
			instance_indices
				.write_u32(coordinate, instance_index)
				.expect("Failed to initialize a visibility index texel. The most likely cause is an invalid coordinate.");
		}

		let mut material_counts = buffer(&material_count_program, MATERIAL_COUNT_SLOT);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			for coordinate in [[0, 0], [1, 0], [0, 1], [1, 1]] {
				run_at(&material_count_program, &mut descriptors, coordinate);
			}
		}

		assert_eq!(read_u32(&material_counts, "material_count", 2), 2);
		assert_eq!(read_u32(&material_counts, "material_count", 5), 1);
		assert_eq!(read_u32(&material_counts, "material_count", 0), 0);

		// The offset pass converts sparse counts into exclusive offsets and one indirect dispatch tuple per material.
		let mut material_offsets = buffer(&material_offset_program, MATERIAL_OFFSET_SLOT);
		let mut material_offset_scratch = buffer(&material_offset_program, MATERIAL_OFFSET_SCRATCH_SLOT);
		let mut material_dispatches = buffer(&material_offset_program, MATERIAL_DISPATCH_SLOT);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MATERIAL_COUNT_SLOT, &mut material_counts);
			descriptors.bind_buffer(MATERIAL_OFFSET_SLOT, &mut material_offsets);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(MATERIAL_DISPATCH_SLOT, &mut material_dispatches);
			run_at(&material_offset_program, &mut descriptors, [0, 0]);
		}

		assert_eq!(read_u32(&material_offsets, "material_offset", 2), 0);
		assert_eq!(read_u32(&material_offsets, "material_offset", 5), 2);
		assert_eq!(read_u32(&material_offsets, "material_offset", 6), 3);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 0),
			[0, 1, 1, 0]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 2),
			[1, 1, 1, 0]
		);
		assert_eq!(
			read_vec4u(&material_dispatches, "material_evaluation_dispatches", 5),
			[1, 1, 1, 0]
		);

		// Mapping reuses the scratch offsets as atomic cursors and stores one-based coordinates for later zero-sentinel checks.
		let mut pixel_mapping = buffer(&pixel_mapping_program, PIXEL_MAPPING_SLOT);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(MESH_DATA_SLOT, &mut mesh_data);
			descriptors.bind_buffer(MATERIAL_OFFSET_SCRATCH_SLOT, &mut material_offset_scratch);
			descriptors.bind_buffer(PIXEL_MAPPING_SLOT, &mut pixel_mapping);
			descriptors.bind_image(INSTANCE_INDEX_SLOT, &mut instance_indices);
			for coordinate in [[0, 0], [1, 0], [0, 1], [1, 1]] {
				run_at(&pixel_mapping_program, &mut descriptors, coordinate);
			}
		}

		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 0), [1, 1]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 1), [2, 2]);
		assert_eq!(read_vec2u16(&pixel_mapping, "pixel_mapping", 2), [2, 1]);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 2), 2);
		assert_eq!(read_u32(&material_offset_scratch, "material_offset_scratch", 5), 3);
	}

	/// Executes the standard GTAO shader with one deterministic depth fixture.
	fn run_gtao_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut views = gtao_views(program);
		let mut depth = texture_2d(width, height, depth_texels);
		let mut output = empty_image(width, height);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_texture(DescriptorSlot::new(1, 0), &mut depth);
			descriptors.bind_image(DescriptorSlot::new(1, 1), &mut output);
			run_at(program, &mut descriptors, coordinate);
		}
		rgba(&output, coordinate)
	}

	/// Verifies the standard GTAO shader's background contract and bounded foreground output.
	#[test]
	fn gtao_writes_white_for_background_and_bounded_finite_foreground() {
		let program = crate::rendering::shader_vm_test::compile(build_gtao_program());
		let background = run_gtao_fixture(&program, 1, 1, &[[0.0, 0.0, 0.0, 1.0]], [0, 0]);
		assert_rgba_close(background, [1.0, 1.0, 1.0, 1.0], 0.00001);

		// A recessed center surrounded by nearer depth exercises reconstruction, normal estimation, and the bounded AO integral.
		let mut foreground = [[0.35, 0.0, 0.0, 1.0]; 25];
		foreground[12] = [0.75, 0.0, 0.0, 1.0];
		let foreground = run_gtao_fixture(&program, 5, 5, &foreground, [2, 2]);
		for channel in foreground[..3].iter().copied() {
			assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
		}
		assert_eq!(foreground[3], 1.0);
	}

	/// Reads one unsigned integer texel from a VM image fixture.
	fn fetch_u32(texture: &Texture, coordinate: [u32; 2]) -> u32 {
		match texture
			.fetch_u32(coordinate)
			.expect("Failed to read an integer VM image. The most likely cause is an invalid assertion coordinate.")
		{
			Value::U32(value) => value,
			value => panic!(
				"Unexpected integer image value: {value:?}. The most likely cause is a drifted integer image representation."
			),
		}
	}

	/// Verifies background rejection and the packed foreground-bit convention.
	#[test]
	fn gtao_bitfield_skips_background_and_sets_the_foreground_pixel_bit() {
		let program = crate::rendering::shader_vm_test::compile(build_gtao_bitfield_program());

		let mut background_views = gtao_views(&program);
		let mut background_depth = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut background_bits = Texture::new(1, 1)
			.expect("Failed to create the GTAO bitfield fixture. The most likely cause is an invalid test extent.");
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut background_views);
			descriptors.bind_texture(DescriptorSlot::new(1, 0), &mut background_depth);
			descriptors.bind_image(DescriptorSlot::new(1, 1), &mut background_bits);
			run_at(&program, &mut descriptors, [0, 0]);
		}
		assert_eq!(fetch_u32(&background_bits, [0, 0]), 0);

		// Coordinate (13, 28) has low deterministic quantization noise and occupies bit 13 of the first packed word.
		let mut foreground_texels = [[0.35, 0.0, 0.0, 1.0]; 96 * 96];
		// A flat local patch keeps the reconstructed normal stable while the nearer surrounding depths occlude farther steps.
		for y in 27..=29 {
			for x in 12..=14 {
				foreground_texels[y * 96 + x] = [0.85, 0.0, 0.0, 1.0];
			}
		}
		let mut foreground_views = gtao_views(&program);
		let mut foreground_depth = texture_2d(96, 96, &foreground_texels);
		let mut foreground_bits = Texture::new(3, 96)
			.expect("Failed to create the GTAO bitfield output. The most likely cause is an invalid test extent.");
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut foreground_views);
			descriptors.bind_texture(DescriptorSlot::new(1, 0), &mut foreground_depth);
			descriptors.bind_image(DescriptorSlot::new(1, 1), &mut foreground_bits);
			run_at(&program, &mut descriptors, [13, 28]);
		}
		assert_eq!(fetch_u32(&foreground_bits, [0, 28]) & (1 << 13), 1 << 13);
	}

	/// Runs the bitfield decoder with a uniform packed AO word.
	fn run_bitfield_blur_fixture(program: &ExecutableProgram, packed_bits: u32) -> [f32; 4] {
		let mut views = gtao_views(program);
		let mut depth = texture_2d(32, 1, &[[0.5, 0.0, 0.0, 1.0]; 32]);
		let mut source = Texture::new(1, 1)
			.expect("Failed to create a packed GTAO fixture. The most likely cause is an invalid test extent.");
		source
			.write_u32([0, 0], packed_bits)
			.expect("Failed to initialize packed GTAO bits. The most likely cause is an invalid fixture coordinate.");
		let mut output = empty_image(1, 1);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_texture(DescriptorSlot::new(1, 0), &mut depth);
			descriptors.bind_texture(DescriptorSlot::new(1, 1), &mut source);
			descriptors.bind_image(DescriptorSlot::new(1, 2), &mut output);
			run_at(program, &mut descriptors, [0, 0]);
		}
		rgba(&output, [0, 0])
	}

	/// Verifies that packed binary AO values decode to the expected continuous endpoints.
	#[test]
	fn gtao_bitfield_blur_decodes_clear_and_set_words_to_opposite_endpoints() {
		let program = crate::rendering::shader_vm_test::compile(build_gtao_bitfield_blur_x_program());
		assert_rgba_close(run_bitfield_blur_fixture(&program, 0), [1.0, 0.0, 0.0, 1.0], 0.00001);
		assert_rgba_close(run_bitfield_blur_fixture(&program, u32::MAX), [0.0, 0.0, 0.0, 1.0], 0.00001);
	}

	/// Compiles the generic GTAO blur with a host-selected axis specialization.
	fn compile_gtao_blur(direction: [f32; 2]) -> ExecutableProgram {
		let mut specializations = SpecializationValues::new();
		specializations.set("blur_direction", Value::Vec2F(direction));
		ExecutableProgram::compile_with_specializations(build_gtao_blur_program(), &specializations).expect(
			"Failed to compile the GTAO blur shader with the BESL VM. The most likely cause is missing specialization or VM support.",
		)
	}

	/// Runs one generic GTAO blur invocation for a selected specialization direction.
	fn run_gtao_blur_fixture(
		program: &ExecutableProgram,
		width: u32,
		height: u32,
		depth_texels: &[[f32; 4]],
		ao_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let mut views = gtao_views(program);
		let mut depth = texture_2d(width, height, depth_texels);
		let mut ao = texture_2d(width, height, ao_texels);
		let mut output = empty_image(width, height);
		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(VIEWS_SLOT, &mut views);
			descriptors.bind_texture(DescriptorSlot::new(1, 0), &mut depth);
			descriptors.bind_texture(DescriptorSlot::new(1, 1), &mut ao);
			descriptors.bind_image(DescriptorSlot::new(1, 2), &mut output);
			run_at(program, &mut descriptors, coordinate);
		}
		rgba(&output, coordinate)
	}

	/// Verifies specialization-controlled blur direction without disturbing uniform input.
	#[test]
	fn gtao_blur_preserves_uniform_ao_and_obeys_x_y_specializations() {
		let blur_x = compile_gtao_blur([1.0, 0.0]);
		let blur_y = compile_gtao_blur([0.0, 1.0]);
		let depth = [[0.5, 0.0, 0.0, 1.0]; 25];
		let uniform_ao = [[0.37, 0.0, 0.0, 1.0]; 25];
		assert_rgba_close(
			run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &uniform_ao, [2, 2]),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);
		assert_rgba_close(
			run_gtao_blur_fixture(&blur_y, 5, 5, &depth, &uniform_ao, [2, 2]),
			[0.37, 0.0, 0.0, 1.0],
			0.00001,
		);

		// Horizontal variation is smoothed by the X specialization, while every Y sample still observes the center column.
		let directional_ao: [[f32; 4]; 25] = std::array::from_fn(|index| {
			if index % 5 == 2 {
				[1.0, 0.0, 0.0, 1.0]
			} else {
				[0.0, 0.0, 0.0, 1.0]
			}
		});
		let horizontal = run_gtao_blur_fixture(&blur_x, 5, 5, &depth, &directional_ao, [2, 2]);
		let vertical = run_gtao_blur_fixture(&blur_y, 5, 5, &depth, &directional_ao, [2, 2]);
		assert!(
			horizontal[0] < 0.8,
			"Expected X blur to mix neighboring columns, found {horizontal:?}"
		);
		assert!(
			(vertical[0] - 1.0).abs() < 0.00001,
			"Expected Y blur to preserve the center column, found {vertical:?}"
		);
	}

	#[test]
	fn shader_meshlet_data_matches_metal_buffer_layout() {
		assert_eq!(std::mem::align_of::<super::ShaderMeshletData>(), 16);
		assert_eq!(std::mem::size_of::<super::ShaderMeshletData>(), 64);
	}

	#[test]
	fn visibility_mesh_hlsl_uses_shader_matrix4x3_layout() {
		let source = get_visibility_pass_mesh_hlsl_source();

		assert!(
			source.contains("#pragma pack_matrix(row_major)"),
			"Expected DX12 visibility mesh HLSL to use row-major matrix storage. The most likely cause is that ShaderMatrix4x3 bytes are being interpreted as column-major float4x3 columns."
		);
		assert!(
			source.contains("float4x3 model;"),
			"Expected DX12 visibility mesh HLSL to read ShaderMatrix4x3 as four packed float3 affine rows. The most likely cause is that the shader-side mesh layout drifted from the CPU upload layout."
		);
		assert!(
			source.contains("return float4(mul(float4(position, 1.0f), mesh.model), 1.0f);"),
			"Expected DX12 visibility mesh HLSL to multiply model-space positions by the packed float4x3 model. The most likely cause is that the model transform was decoded as three float4 rows."
		);
		assert!(
			!source.contains("float4 row0;") && !source.contains("float4 row1;") && !source.contains("float4 row2;"),
			"Expected DX12 visibility mesh HLSL to avoid the obsolete three-float4 model layout. The most likely cause is that the shader is decoding ShaderMatrix4x3 with the wrong row stride."
		);
	}

	#[test]
	fn visibility_mesh_hlsl_source_compiles_for_dx12() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		if !ghi::implementation::USES_DX12 {
			return;
		}

		let shader = get_visibility_pass_mesh_hlsl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a DX12 instance for the visibility mesh shader test");
		let mut queue = None;
		let mut context = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a DX12 device for the visibility mesh shader test")
			.create_context()
			.expect("Expected a DX12 context");

		let shader_handle = context.create_shader(
			Some("Visibility Pass Mesh Shader Layout Test"),
			ghi::shader::Sources::HLSL {
				source: shader.as_str(),
				entry_point: "main",
			},
			ghi::ShaderTypes::Mesh,
			[
				VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				SKINNED_VERTICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		);

		assert!(
			shader_handle.is_ok(),
			"Expected the visibility mesh HLSL source to compile for DX12"
		);
	}

	#[test]
	fn shadow_mesh_msl_source_compiles_for_metal() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		if !ghi::implementation::USES_METAL {
			return;
		}

		let shader = get_shadow_pass_mesh_msl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the shadow mesh shader test");
		let mut queue = None;
		let mut context = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a Metal device for the shadow mesh shader test")
			.create_context()
			.expect("Expected a Metal context");

		let shader_handle = context.create_shader(
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
				SKINNED_VERTICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
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
	fn visibility_task_msl_source_compiles_for_metal() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		if !ghi::implementation::USES_METAL {
			return;
		}

		let shader = get_visibility_pass_task_msl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the visibility task shader test");
		let mut queue = None;
		let mut context = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a Metal device for the visibility task shader test")
			.create_context()
			.expect("Expected a Metal context");

		let shader_handle = context.create_shader(
			Some("Visibility Pass Task Shader"),
			ghi::shader::Sources::MTL {
				source: shader.as_str(),
				entry_point: "besl_task_main",
			},
			ghi::ShaderTypes::Task,
			[
				VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		);

		assert!(
			shader_handle.is_ok(),
			"Expected the visibility task MSL source to compile for Metal"
		);
	}

	#[test]
	fn visibility_mesh_msl_source_compiles_for_metal() {
		use ghi::{
			context::{Context as _, ContextCreate as _},
			device::Device as _,
		};

		if !ghi::implementation::USES_METAL {
			return;
		}

		let shader = get_visibility_pass_mesh_msl_source();
		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the visibility mesh shader test");
		let mut queue = None;
		let mut context = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue)],
			)
			.expect("Expected a Metal device for the visibility mesh shader test")
			.create_context()
			.expect("Expected a Metal context");

		let shader_handle = context.create_shader(
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
				SKINNED_VERTICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
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
}
