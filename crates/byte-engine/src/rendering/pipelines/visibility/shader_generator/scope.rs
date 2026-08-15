use besl::parser::Node;

use super::ast::*;
use super::sources::*;
use crate::rendering::common_shader_generator::CommonShaderScope;
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_PIXEL_MAPPING_ENTRIES};

/// The `VisibilityShaderScope` struct provides material programs with the shared visibility data and lighting contract.
pub struct VisibilityShaderScope {}

/// The `VisibilityShaderGenerator` struct adapts portable material programs for visibility-buffer evaluation.
pub struct VisibilityShaderGenerator {
	pub(super) scope: besl::parser::Node<'static>,
}

impl VisibilityShaderGenerator {
	pub fn new(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> Self {
		Self {
			scope: VisibilityShaderScope::new_with_params(
				material_count_read,
				material_count_write,
				material_offset_read,
				material_offset_write,
				material_offset_scratch_read,
				material_offset_scratch_write,
				pixel_mapping_read,
				pixel_mapping_write,
			),
		}
	}
}

impl VisibilityShaderScope {
	pub fn new<'a>() -> besl::parser::Node<'a> {
		Self::new_with_params(true, true, true, true, true, true, true, true)
	}

	pub fn new_with_params<'a>(
		material_count_read: bool,
		material_count_write: bool,
		material_offset_read: bool,
		material_offset_write: bool,
		material_offset_scratch_read: bool,
		material_offset_scratch_write: bool,
		pixel_mapping_read: bool,
		pixel_mapping_write: bool,
	) -> besl::parser::Node<'a> {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct(
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
		);
		let skinned_vertex_struct = Node::r#struct(
			"SkinnedVertex",
			vec![Node::member("position", "vec4f"), Node::member("normal", "vec4f")],
		);
		let triangle_interpolation_struct = Node::r#struct(
			"TriangleInterpolation",
			vec![
				Node::member("origin", "vec2f"),
				Node::member("inverse_w", "vec3f"),
				Node::member("raw_ddx", "vec3f"),
				Node::member("raw_ddy", "vec3f"),
			],
		);
		let view_struct = Node::r#struct(
			"View",
			vec![
				Node::member("view", "mat4x3f"),
				Node::member("view_projection", "mat4f"),
				Node::member("inverse_view", "mat4x3f"),
				Node::member("fov", "vec2f"),
				Node::member("near", "f32"),
				Node::member("far", "f32"),
			],
		);
		let meshlet_struct = Node::r#struct(
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
		);
		let light_struct = Node::r#struct(
			"Light",
			vec![
				// Use explicit 16-byte vector fields so every storage-buffer backend shares the CPU layout.
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
		);
		let material_struct = Node::r#struct(
			"Material",
			vec![
				Node::member("textures", material_texture_array_type()),
				Node::member("coverage_factor", "f32"),
				Node::member("coverage_texture_slot", "u32"),
				Node::member("alpha_cutoff", "f32"),
				Node::member("padding", "u32"),
			],
		);

		let views_binding = Node::constant_buffer_binding(
			"views",
			Node::buffer("ViewsBuffer", vec![Node::member("views", "View[9]")]),
			0,
			true,
			false,
		);
		let meshes = Node::device_buffer_binding(
			"meshes",
			Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[1024]")]),
			1,
			true,
			false,
		);
		let positions = Node::device_buffer_binding(
			"vertex_positions",
			Node::buffer("Positions", vec![Node::member("positions", vertex_vec3_array_type())]),
			2,
			true,
			false,
		);
		let normals = Node::device_buffer_binding(
			"vertex_normals",
			Node::buffer("Normals", vec![Node::member("normals", vertex_normal_array_type())]),
			3,
			true,
			false,
		);
		let skinned_vertices = Node::device_buffer_binding(
			"skinned_vertices",
			Node::buffer("SkinnedVertices", vec![Node::member("vertices", skinned_vertex_array_type())]),
			4,
			true,
			false,
		);
		let uvs = Node::device_buffer_binding(
			"vertex_uvs",
			Node::buffer("UVs", vec![Node::member("uvs", vertex_uv_array_type())]),
			5,
			true,
			false,
		);
		let vertex_indices = Node::device_buffer_binding(
			"vertex_indices",
			Node::buffer(
				"VertexIndices",
				vec![Node::member("vertex_indices", vertex_index_array_type())],
			),
			6,
			true,
			false,
		);
		let primitive_indices = Node::device_buffer_binding(
			"primitive_indices",
			Node::buffer(
				"PrimitiveIndices",
				vec![Node::member("primitive_indices", primitive_index_array_type())],
			),
			7,
			true,
			false,
		);
		let meshlets = Node::device_buffer_binding(
			"meshlets",
			Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", meshlet_array_type())]),
			8,
			true,
			false,
		);
		let textures = Node::binding_array(
			"textures",
			Node::combined_image_sampler(),
			9,
			true,
			false,
			MAX_BINDLESS_TEXTURES as u32,
		);

		let material_count = Node::device_buffer_binding(
			"material_count",
			Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[1024]")]),
			1033,
			material_count_read,
			material_count_write,
		); // TODO: somehow set read/write properties per shader
		let material_offset = Node::device_buffer_binding(
			"material_offset",
			Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[1024]")]),
			1034,
			material_offset_read,
			material_offset_write,
		);
		let material_offset_scratch = Node::device_buffer_binding(
			"material_offset_scratch",
			Node::buffer(
				"MaterialOffsetScratch",
				vec![Node::member("material_offset_scratch", "u32[1024]")],
			),
			1035,
			material_offset_scratch_read,
			material_offset_scratch_write,
		);
		let pixel_mapping = Node::device_buffer_binding(
			"pixel_mapping",
			Node::buffer(
				"PixelMapping",
				vec![Node::member(
					"pixel_mapping",
					&format!("vec2u16[{MAX_PIXEL_MAPPING_ENTRIES}]"),
				)],
			),
			1037,
			pixel_mapping_read,
			pixel_mapping_write,
		);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1039, true, false);
		let instance_index = Node::binding("instance_index_render_target", Node::image("r32ui"), 1040, true, false);

		// Resolve all three triangle vertices together so mesh and meshlet offsets are computed once.
		let compute_vertex_indices = parse_besl_function(
			r#"
			compute_vertex_indices: fn (mesh: Mesh, meshlet: Meshlet, primitive_index_base: u32) -> u32[3] {
				let vertex_index_base: u32 = mesh.base_vertex_index;
				let relative_index_base: u32 = mesh.base_primitive_index + meshlet.primitive_offset;
				let primitive_index0: u32 = u32(primitive_indices.primitive_indices[primitive_index_base]);
				let primitive_index1: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 1]);
				let primitive_index2: u32 = u32(primitive_indices.primitive_indices[primitive_index_base + 2]);
				return u32[3](
					vertex_index_base + u16_to_u32(vertex_indices.vertex_indices[relative_index_base + primitive_index0]),
					vertex_index_base + u16_to_u32(vertex_indices.vertex_indices[relative_index_base + primitive_index1]),
					vertex_index_base + u16_to_u32(vertex_indices.vertex_indices[relative_index_base + primitive_index2])
				);
			}
			"#,
			"compute_vertex_indices",
		);
		// Share the clip-space basis between geometry and optional UV interpolation.
		let compute_triangle_interpolation = parse_besl_function(
			r#"
			compute_triangle_interpolation: fn (
				clip_position0: vec4f,
				clip_position1: vec4f,
				clip_position2: vec4f
			) -> TriangleInterpolation {
				let inverse_w: vec3f = vec3f(
					1.0 / clip_position0.w,
					1.0 / clip_position1.w,
					1.0 / clip_position2.w
				);
				let origin: vec2f = vec2f(
					clip_position0.x * inverse_w.x,
					clip_position0.y * inverse_w.x
				);
				let ndc1: vec2f = vec2f(
					clip_position1.x * inverse_w.y,
					clip_position1.y * inverse_w.y
				);
				let ndc2: vec2f = vec2f(
					clip_position2.x * inverse_w.z,
					clip_position2.y * inverse_w.z
				);
				let determinant: f32 =
					(ndc2.x - ndc1.x) * (origin.y - ndc1.y) -
					(origin.x - ndc1.x) * (ndc2.y - ndc1.y);
				let inverse_determinant: f32 = 1.0 / determinant;
				let raw_ddx: vec3f = vec3f(
					ndc1.y - ndc2.y,
					ndc2.y - origin.y,
					origin.y - ndc1.y
				) * inverse_determinant * inverse_w;
				let raw_ddy: vec3f = vec3f(
					ndc2.x - ndc1.x,
					origin.x - ndc2.x,
					ndc1.x - origin.x
				) * inverse_determinant * inverse_w;
				return TriangleInterpolation(origin, inverse_w, raw_ddx, raw_ddy);
			}
			"#,
			"compute_triangle_interpolation",
		);
		let u16_to_u32 = parse_besl_function("u16_to_u32: fn (value: u16) -> u32 { return u32(value); }", "u16_to_u32");
		let decode_f16_vec2 = parse_besl_function(DECODE_F16_VEC2_SOURCE, "decode_f16_vec2");
		let decode_octahedral_normal = parse_besl_function(DECODE_OCTAHEDRAL_NORMAL_SOURCE, "decode_octahedral_normal");
		let cone_attenuation = parse_besl_function(
			"cone_attenuation: fn (cosine: f32, inner_cosine: f32, outer_cosine: f32) -> f32 { return clamp((cosine - outer_cosine) / (inner_cosine - outer_cosine), 0.0, 1.0); }",
			"cone_attenuation",
		);
		let set2_binding0 = Node::binding("lit_map", Node::image("rgba16"), 1041, true, true);
		let set2_binding4 = Node::constant_buffer_binding(
			"lighting_data",
			Node::buffer(
				"LightingBuffer",
				vec![
					Node::member("light_count", "u32"),
					// Keep the light array at the CPU record's 16-byte boundary on scalar-layout backends.
					Node::member("_light_count_padding", "u32[3]"),
					Node::member("lights", light_array_type()),
				],
			),
			1045,
			true,
			false,
		);
		let set2_binding5 = Node::device_buffer_binding(
			"materials",
			Node::buffer("MaterialBuffer", vec![Node::member("materials", material_array_type())]),
			1046,
			true,
			false,
		);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 1051, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_array_image_sampler(), 1052, true, false);
		let directional_shadow_depth_pyramid = Node::binding(
			"directional_shadow_depth_pyramid",
			Node::combined_image_sampler(),
			1053,
			true,
			false,
		);
		let cone_shadow_map = Node::binding("cone_shadow_map", Node::combined_array_image_sampler(), 1064, true, false);
		let point_shadow_map = Node::binding(
			"point_shadow_map",
			Node::combined_cube_array_image_sampler(),
			1065,
			true,
			false,
		);
		let environment_irradiance = Node::binding(
			"environment_irradiance",
			Node::combined_cube_image_sampler(),
			1054,
			true,
			false,
		);
		let environment_specular =
			Node::binding("environment_specular", Node::combined_cube_image_sampler(), 1055, true, false);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32"), Node::member("blend", "u32")]);

		let sample_function = Node::intrinsic_with_parameters(
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

		// Keep normal-map decoding in the visibility material module. BESL only
		// lowers the general texture-array gradient sample used by these helpers.
		let ies_profile_uv = parse_besl_function(IES_PROFILE_UV_SOURCE, "ies_profile_uv");
		let sample_ies_profile = parse_besl_function(IES_PROFILE_SAMPLE_SOURCE, "sample_ies_profile");
		let sample_normal_function = parse_besl_function(
			r#"
			sample_visibility_normal: fn (
				texture_index: u32,
				uv: vec2f,
				uv_derivative_x: vec2f,
				uv_derivative_y: vec2f
			) -> vec3f {
				let encoded: vec4f = sample_texture_2d_array_grad(
					textures, texture_index, uv, uv_derivative_x, uv_derivative_y
				);
				return unit_vector_from_xy(vec2f(encoded.x, encoded.y));
			}
			"#,
			"sample_visibility_normal",
		);
		// Lighting helpers are authored once. Texture operations that differ by API remain typed intrinsics below.
		let shadow_receiver_plane_depth_gradient =
			parse_besl_function(SHADOW_RECEIVER_PLANE_SOURCE, "shadow_receiver_plane_depth_gradient");
		let sample_shadow_tap = parse_besl_function(SHADOW_TAP_SOURCE, "sample_shadow_tap");
		let rotate_shadow_poisson_offset = parse_besl_function(SHADOW_POISSON_ROTATION_SOURCE, "rotate_shadow_poisson_offset");
		let sample_rotated_shadow_tap = parse_besl_function(ROTATED_SHADOW_TAP_SOURCE, "sample_rotated_shadow_tap");
		let sample_directional_shadow_tap = parse_besl_function(DIRECTIONAL_SHADOW_TAP_SOURCE, "sample_directional_shadow_tap");
		let directional_shadow_area_is_fully_lit =
			parse_besl_function(DIRECTIONAL_SHADOW_DEPTH_PROBE_SOURCE, "directional_shadow_area_is_fully_lit");
		let compute_shadow_rotation = parse_besl_function(SHADOW_ROTATION_SOURCE, "compute_shadow_rotation");
		let sample_cone_shadow = parse_besl_function(CONE_SHADOW_SOURCE, "sample_cone_shadow");
		let point_shadow_receiver_depth =
			parse_besl_function(POINT_SHADOW_RECEIVER_DEPTH_SOURCE, "point_shadow_receiver_depth");
		let point_shadow_occlusion = parse_besl_function(POINT_SHADOW_OCCLUSION_SOURCE, "point_shadow_occlusion");
		let point_shadow_receiver_vector =
			parse_besl_function(POINT_SHADOW_RECEIVER_VECTOR_SOURCE, "point_shadow_receiver_vector");
		let point_shadow_receiver_plane_normal = parse_besl_function(
			POINT_SHADOW_RECEIVER_PLANE_NORMAL_SOURCE,
			"point_shadow_receiver_plane_normal",
		);
		let point_shadow_texel_direction =
			parse_besl_function(POINT_SHADOW_TEXEL_DIRECTION_SOURCE, "point_shadow_texel_direction");
		let sample_point_shadow_tap = parse_besl_function(POINT_SHADOW_TAP_SOURCE, "sample_point_shadow_tap");
		let sample_point_shadow = parse_besl_function(POINT_SHADOW_SOURCE, "sample_point_shadow");
		let sample_directional_shadow = parse_besl_function(DIRECTIONAL_SHADOW_SOURCE, "sample_directional_shadow");
		let sample_environment_irradiance = parse_besl_function(ENVIRONMENT_IRRADIANCE_SOURCE, "sample_environment_irradiance");
		let sample_environment_specular = parse_besl_function(ENVIRONMENT_SPECULAR_SOURCE, "sample_environment_specular");

		Node::scope(
			"Visibility",
			vec![
				view_struct,
				views_binding,
				mesh_struct,
				skinned_vertex_struct,
				triangle_interpolation_struct,
				meshlet_struct,
				light_struct,
				material_struct,
				directional_shadow_depth_pyramid,
				shadow_receiver_plane_depth_gradient,
				sample_shadow_tap,
				rotate_shadow_poisson_offset,
				sample_rotated_shadow_tap,
				sample_directional_shadow_tap,
				directional_shadow_area_is_fully_lit,
				compute_shadow_rotation,
				sample_cone_shadow,
				sample_directional_shadow,
				meshes,
				positions,
				normals,
				skinned_vertices,
				uvs,
				vertex_indices,
				primitive_indices,
				meshlets,
				textures,
				material_count,
				material_offset,
				material_offset_scratch,
				pixel_mapping,
				triangle_index,
				instance_index,
				u16_to_u32,
				decode_f16_vec2,
				decode_octahedral_normal,
				cone_attenuation,
				compute_vertex_indices,
				compute_triangle_interpolation,
				set2_binding0,
				set2_binding4,
				set2_binding5,
				set2_binding10,
				set2_binding11,
				cone_shadow_map,
				point_shadow_map,
				point_shadow_receiver_depth,
				point_shadow_occlusion,
				point_shadow_receiver_vector,
				point_shadow_receiver_plane_normal,
				point_shadow_texel_direction,
				sample_point_shadow_tap,
				sample_point_shadow,
				environment_irradiance,
				environment_specular,
				push_constant,
				sample_function,
				ies_profile_uv,
				sample_ies_profile,
				sample_normal_function,
				sample_environment_irradiance,
				sample_environment_specular,
			],
		)
	}
}
