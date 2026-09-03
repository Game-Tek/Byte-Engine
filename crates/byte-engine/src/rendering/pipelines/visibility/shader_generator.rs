//! Adapts portable material programs into visibility material-evaluation compute shaders.
//!
//! [`VisibilityShaderScope`] declares the buffers, images, structs, and BESL helper functions every visibility
//! shader can reference. [`VisibilityShaderGenerator`] wraps an authored material `main` with the pixel
//! reconstruction prefix and the lighting suffix from [`sources`], and rewrites material shorthand such as
//! `sample_material(texture)` into explicit bindless sampling.

mod ast;
mod sources;
#[cfg(test)]
mod tests;

use besl::parser::Node;
use ghi::AccessPolicies;
use resource_management::asset::JsonObject;
use resource_management::asset::handler::implementations::bema::ProgramGenerator;
use utils::json::{JsonContainerTrait, JsonValueTrait};

use self::ast::*;
use self::sources::*;
use super::layout::{
	MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS, MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
};
use crate::rendering::common_shader_generator::CommonShaderScope;

// BESL array types are spelled out so the scope stays a plain literal; these guards catch limit changes.
const LIGHT_ARRAY: &str = "Light[16]";
const MATERIAL_ARRAY: &str = "Material[1024]";
const MATERIAL_TEXTURE_ARRAY: &str = "u32[16]";
const VERTEX_VEC3_ARRAY: &str = "vec3f[262144]";
const VERTEX_NORMAL_ARRAY: &str = "vec2u16[262144]";
const VERTEX_UV_ARRAY: &str = "vec2f16[262144]";
const SKINNED_VERTEX_ARRAY: &str = "SkinnedVertex[262144]";
const VERTEX_INDEX_ARRAY: &str = "u16[262144]";
const PRIMITIVE_INDEX_ARRAY: &str = "u8[786432]";
const MESHLET_ARRAY: &str = "Meshlet[4096]";
const PIXEL_MAPPING_ARRAY: &str = "vec2u16[8294400]";
const _: () = assert!(
	MAX_LIGHTS == 16
		&& MAX_MATERIALS == 1024
		&& MAX_MATERIAL_TEXTURES == 16
		&& MAX_VERTICES == 262144
		&& MAX_PRIMITIVE_TRIANGLES == 262144
		&& MAX_TRIANGLES * 3 == 786432
		&& MAX_MESHLETS == 4096
		&& MAX_PIXEL_MAPPING_ENTRIES == 8294400,
	"Update the visibility shader scope array types when visibility limits change."
);

/// The `ScopeAccess` struct declares how a generated shader touches the per-sink material dispatch buffers.
///
/// The default grants read and write access everywhere, which suits baking every visibility shader with one
/// generator. Narrow it when a backend needs exact access declarations for one shader family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeAccess {
	pub material_count: AccessPolicies,
	pub material_offset: AccessPolicies,
	pub material_offset_scratch: AccessPolicies,
	pub pixel_mapping: AccessPolicies,
}

impl Default for ScopeAccess {
	fn default() -> Self {
		Self {
			material_count: AccessPolicies::READ_WRITE,
			material_offset: AccessPolicies::READ_WRITE,
			material_offset_scratch: AccessPolicies::READ_WRITE,
			pixel_mapping: AccessPolicies::READ_WRITE,
		}
	}
}

/// The `VisibilityShaderGenerator` struct turns portable material programs into visibility material-evaluation shaders.
///
/// Install it on the material, FBX, and glTF asset handlers so every baked material targets this pipeline.
pub struct VisibilityShaderGenerator {
	scope: Node<'static>,
}

impl Default for VisibilityShaderGenerator {
	fn default() -> Self {
		Self::new()
	}
}

impl VisibilityShaderGenerator {
	pub fn new() -> Self {
		Self::with_access(ScopeAccess::default())
	}

	pub fn with_access(access: ScopeAccess) -> Self {
		Self {
			scope: VisibilityShaderScope::new(access),
		}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: Node<'a>, material: &'a JsonObject) -> Node<'a> {
		let mut declarations = Vec::new();
		let mut texture_slots = Vec::new();
		for variable in material["variables"].as_array().expect("material variables").iter() {
			let name = variable["name"].as_str().expect("material variable name");
			let data_type = variable["data_type"].as_str().expect("material variable type");
			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4f" => declarations.push(Node::specialization(name, data_type)),
				"Texture2D" => {
					let slot = texture_slots.len() as u32;
					texture_slots.push((name, slot));
					declarations.push(Node::constant(name, "u32", Node::literal_expression(format!("{slot}u"))));
				}
				_ => {}
			}
		}

		let main = root.get_mut("main").expect("material program main");
		let features = material_reconstruction_features(main);
		add_material_sample_context(main, &texture_slots);
		narrow_material_property_assignments(main);
		if let besl::parser::Nodes::Function { statements, .. } = main.node_mut() {
			statements.splice(0..0, material_evaluation_prefix_statements(features));
			statements.extend(material_evaluation_suffix_statements(features));
		}

		root.add(declarations);
		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);
		root
	}
}

/// The `VisibilityShaderScope` struct provides material programs with the shared visibility data and lighting contract.
pub struct VisibilityShaderScope;

impl VisibilityShaderScope {
	/// Builds the declarative scope; every binding slot here mirrors [`super::layout`].
	pub fn new<'a>(access: ScopeAccess) -> Node<'a> {
		let structs = vec![
			Node::r#struct(
				"View",
				vec![
					Node::member("view", "mat4x3f"),
					Node::member("view_projection", "mat4f"),
					Node::member("inverse_view", "mat4x3f"),
					Node::member("fov", "vec2f"),
					Node::member("near", "f32"),
					Node::member("far", "f32"),
				],
			),
			Node::constant_buffer_binding(
				"views",
				Node::buffer("ViewsBuffer", vec![Node::member("views", "View[9]")]),
				0,
				true,
				false,
			),
			Node::r#struct(
				"Mesh",
				vec![
					Node::member("model", "mat4x3f"),
					Node::member("material_index", "u32"),
					Node::member("base_vertex_index", "u32"),
					Node::member("base_primitive_index", "u32"),
					Node::member("base_triangle_index", "u32"),
					Node::member("base_meshlet_index", "u32"),
					Node::member("meshlet_count", "u32"),
					Node::member("skinned_base_vertex_index", "u32"),
					Node::member("padding0", "u32"),
				],
			),
			Node::r#struct(
				"SkinnedVertex",
				vec![Node::member("position", "vec4f"), Node::member("normal", "vec4f")],
			),
			Node::r#struct(
				"TriangleInterpolation",
				vec![
					Node::member("origin", "vec2f"),
					Node::member("inverse_w", "vec3f"),
					Node::member("raw_ddx", "vec3f"),
					Node::member("raw_ddy", "vec3f"),
				],
			),
			Node::r#struct(
				"Meshlet",
				vec![
					Node::member("primitive_offset", "u32"),
					Node::member("triangle_offset", "u32"),
					Node::member("primitive_count", "u32"),
					Node::member("triangle_count", "u32"),
					Node::member("center_radius", "packed_vec4f"),
					Node::member("cone_apex_cutoff", "packed_vec4f"),
					Node::member("cone_axis", "vec2u16"),
				],
			),
			Node::r#struct(
				"Light",
				vec![
					// Explicit 16-byte vector fields keep every storage-buffer backend on the CPU layout.
					Node::member("position", "vec4f"),
					Node::member("color", "vec4f"),
					Node::member("direction", "vec4f"),
					Node::member("cone_cosines", "vec2f"),
					Node::member("type", "u32"),
					Node::member("shadow_views", "u32[8]"),
					Node::member("shadow_layer", "u32"),
					Node::member("ies_profile_texture", "u32"),
					Node::member("ies_c0_tangent", "vec2u16"),
					Node::member("_ies_padding", "u32[2]"),
				],
			),
			Node::r#struct(
				"Material",
				vec![
					Node::member("textures", MATERIAL_TEXTURE_ARRAY),
					Node::member("coverage_factor", "f32"),
					Node::member("coverage_texture_slot", "u32"),
					Node::member("alpha_cutoff", "f32"),
					Node::member("padding", "u32"),
				],
			),
		];

		let read_buffer = |name, buffer_name, member, r#type, slot| {
			Node::device_buffer_binding(
				name,
				Node::buffer(buffer_name, vec![Node::member(member, r#type)]),
				slot,
				true,
				false,
			)
		};
		let access_buffer = |name, buffer_name, member, r#type, slot, access: AccessPolicies| {
			Node::device_buffer_binding(
				name,
				Node::buffer(buffer_name, vec![Node::member(member, r#type)]),
				slot,
				access.contains(AccessPolicies::READ),
				access.contains(AccessPolicies::WRITE),
			)
		};
		let sampled = |name, image, slot| Node::binding(name, image, slot, true, false);
		let base_bindings = vec![
			read_buffer("meshes", "MeshBuffer", "meshes", "Mesh[1024]", 1),
			read_buffer("vertex_positions", "Positions", "positions", VERTEX_VEC3_ARRAY, 2),
			read_buffer("vertex_normals", "Normals", "normals", VERTEX_NORMAL_ARRAY, 3),
			read_buffer("skinned_vertices", "SkinnedVertices", "vertices", SKINNED_VERTEX_ARRAY, 4),
			read_buffer("vertex_uvs", "UVs", "uvs", VERTEX_UV_ARRAY, 5),
			read_buffer("vertex_indices", "VertexIndices", "vertex_indices", VERTEX_INDEX_ARRAY, 6),
			read_buffer(
				"primitive_indices",
				"PrimitiveIndices",
				"primitive_indices",
				PRIMITIVE_INDEX_ARRAY,
				7,
			),
			read_buffer("meshlets", "MeshletsBuffer", "meshlets", MESHLET_ARRAY, 8),
			Node::binding_array(
				"textures",
				Node::combined_image_sampler(),
				9,
				true,
				false,
				MAX_BINDLESS_TEXTURES as u32,
			),
			access_buffer(
				"material_count",
				"MaterialCount",
				"material_count",
				"u32[1024]",
				1033,
				access.material_count,
			),
			access_buffer(
				"material_offset",
				"MaterialOffset",
				"material_offset",
				"u32[1024]",
				1034,
				access.material_offset,
			),
			access_buffer(
				"material_offset_scratch",
				"MaterialOffsetScratch",
				"material_offset_scratch",
				"u32[1024]",
				1035,
				access.material_offset_scratch,
			),
			access_buffer(
				"pixel_mapping",
				"PixelMapping",
				"pixel_mapping",
				PIXEL_MAPPING_ARRAY,
				1037,
				access.pixel_mapping,
			),
			Node::binding("triangle_index", Node::image("r32ui"), 1039, true, false),
			Node::binding("instance_index_render_target", Node::image("r32ui"), 1040, true, false),
		];
		let material_evaluation_bindings = vec![
			Node::binding("lit_map", Node::image("rgba16f"), 1041, true, true),
			Node::constant_buffer_binding(
				"lighting_data",
				Node::buffer(
					"LightingBuffer",
					vec![
						Node::member("light_count", "u32"),
						// Keep the light array at the CPU record's 16-byte boundary on scalar-layout backends.
						Node::member("_light_count_padding", "u32[3]"),
						Node::member("lights", LIGHT_ARRAY),
					],
				),
				1045,
				true,
				false,
			),
			read_buffer("materials", "MaterialBuffer", "materials", MATERIAL_ARRAY, 1046),
			sampled("ao", Node::combined_image_sampler(), 1051),
			sampled("depth_shadow_map", Node::combined_array_image_sampler(), 1052),
			sampled("directional_shadow_depth_pyramid", Node::combined_image_sampler(), 1053),
			sampled("environment_irradiance", Node::combined_cube_image_sampler(), 1054),
			sampled("environment_specular", Node::combined_cube_image_sampler(), 1055),
			sampled("cone_shadow_map", Node::combined_array_image_sampler(), 1064),
			sampled("point_shadow_map", Node::combined_cube_array_image_sampler(), 1065),
			Node::push_constant(vec![Node::member("material_id", "u32"), Node::member("blend", "u32")]),
		];

		// Texture operations that differ by API stay typed intrinsics; every other helper is authored once in BESL.
		let sample_texture = Node::intrinsic_with_parameters(
			"sample_texture_2d_array_grad",
			vec![
				Node::parameter("texture_array", "Texture2D"),
				Node::parameter("texture_index", "u32"),
				Node::parameter("uv", "vec2f"),
				Node::parameter("uv_derivative_x", "vec2f"),
				Node::parameter("uv_derivative_y", "vec2f"),
			],
			Node::sentence(vec![Node::member_expression("textures")]),
			"vec4f",
		);
		// Helpers are listed in dependency order: each one only calls helpers declared above it.
		let helpers = [
			(U16_TO_U32_SOURCE, "u16_to_u32"),
			(DECODE_F16_VEC2_SOURCE, "decode_f16_vec2"),
			(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal"),
			(CONE_ATTENUATION_SOURCE, "cone_attenuation"),
			(COMPUTE_VERTEX_INDICES_SOURCE, "compute_vertex_indices"),
			(COMPUTE_TRIANGLE_INTERPOLATION_SOURCE, "compute_triangle_interpolation"),
			(SAMPLE_VISIBILITY_NORMAL_SOURCE, "sample_visibility_normal"),
			(IES_PROFILE_UV_SOURCE, "ies_profile_uv"),
			(IES_PROFILE_SAMPLE_SOURCE, "sample_ies_profile"),
			(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient"),
			(SHADOW_TAP_SOURCE, "sample_shadow_tap"),
			(SHADOW_POISSON_ROTATION_SOURCE, "rotate_shadow_poisson_offset"),
			(ROTATED_SHADOW_TAP_SOURCE, "sample_rotated_shadow_tap"),
			(DIRECTIONAL_SHADOW_TAP_SOURCE, "sample_directional_shadow_tap"),
			(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit"),
			(SHADOW_ROTATION_SOURCE, "compute_shadow_rotation"),
			(CONE_SHADOW_SOURCE, "sample_cone_shadow"),
			(DIRECTIONAL_SHADOW_SOURCE, "sample_directional_shadow"),
			(POINT_SHADOW_RECEIVER_DEPTH_SOURCE, "point_shadow_receiver_depth"),
			(POINT_SHADOW_OCCLUSION_SOURCE, "point_shadow_occlusion"),
			(POINT_SHADOW_RECEIVER_VECTOR_SOURCE, "point_shadow_receiver_vector"),
			(
				POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE,
				"point_shadow_receiver_plane_normal",
			),
			(POINT_SHADOW_TEXEL_DIRECTION_SOURCE, "point_shadow_texel_direction"),
			(POINT_SHADOW_TAP_SOURCE, "sample_point_shadow_tap"),
			(POINT_SHADOW_SOURCE, "sample_point_shadow"),
			(ENVIRONMENT_IRRADIANCE_SOURCE, "sample_environment_irradiance"),
			(ENVIRONMENT_SPECULAR_SOURCE, "sample_environment_specular"),
		];

		let mut children = structs;
		children.extend(base_bindings);
		children.extend(material_evaluation_bindings);
		children.push(sample_texture);
		children.extend(helpers.into_iter().map(|(source, name)| parse_besl_function(source, name)));
		Node::scope("Visibility", children)
	}
}
