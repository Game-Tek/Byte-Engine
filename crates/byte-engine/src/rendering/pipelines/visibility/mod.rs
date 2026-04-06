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

	shader_generator
		.generate_for_language(
			language,
			&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)),
			&main_node,
		)
		.unwrap()
		.into_source()
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
	let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);

	generate_mesh_source_for_language(
		r#"
		main: fn () -> void {
			let view: View = views.views[0];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
		push_constant,
		PlatformShaderLanguage::Msl,
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
			let view: View = views.views[push_constant.view_index];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
		push_constant,
		PlatformShaderLanguage::Glsl,
	)
}

pub fn get_shadow_pass_mesh_msl_source() -> String {
	let push_constant = besl::parser::Node::push_constant(vec![
		besl::parser::Node::member("instance_index", "u32"),
		besl::parser::Node::member("view_index", "u32"),
	]);

	generate_mesh_source_for_language(
		r#"
		main: fn () -> void {
			let view: View = views.views[push_constant.view_index];
			process_meshlet(push_constant.instance_index, view.view_projection);
		}
		"#,
		push_constant,
		PlatformShaderLanguage::Msl,
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

#[cfg(test)]
mod tests {
	use super::{generate_pixel_mapping_shader_for_language, get_shadow_pass_mesh_msl_source, get_shadow_pass_mesh_source};
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
			shader.contains("View view = set0.views->views[push_constant.view_index];"),
			"Expected MSL shadow mesh source to lower BESL accessors through the Metal argument buffer. Shader: {shader}"
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
}
