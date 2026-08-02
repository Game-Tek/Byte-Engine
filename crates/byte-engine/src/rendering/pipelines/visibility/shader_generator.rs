use std::sync::Arc;
use std::{cell::RefCell, ops::Deref, rc::Rc, sync::OnceLock};

use besl::{parser::Node, NodeReference};
use resource_management::{
	asset::{bema_asset_handler::ProgramGenerator, JsonObject},
	resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT,
};
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use crate::rendering::common_shader_generator::CommonShaderScope;
use crate::rendering::pipelines::visibility::{
	MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
};

fn light_array_type() -> &'static str {
	static LIGHT_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	LIGHT_ARRAY_TYPE
		.get_or_init(|| format!("Light[{MAX_LIGHTS}]").into_boxed_str())
		.as_ref()
}

fn material_array_type() -> &'static str {
	static MATERIAL_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_ARRAY_TYPE
		.get_or_init(|| format!("Material[{MAX_MATERIALS}]").into_boxed_str())
		.as_ref()
}

fn material_texture_array_type() -> &'static str {
	static MATERIAL_TEXTURE_ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();

	MATERIAL_TEXTURE_ARRAY_TYPE
		.get_or_init(|| format!("u32[{MAX_MATERIAL_TEXTURES}]").into_boxed_str())
		.as_ref()
}

fn vertex_vec3_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec3f[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_vec2_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("vec2f[{MAX_VERTICES}]").into_boxed_str())
}

fn skinned_vertex_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("SkinnedVertex[{MAX_VERTICES}]").into_boxed_str())
}

fn vertex_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u16[{MAX_PRIMITIVE_TRIANGLES}]").into_boxed_str())
}

fn primitive_index_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("u8[{}]", MAX_TRIANGLES * 3).into_boxed_str())
}

fn meshlet_array_type() -> &'static str {
	static ARRAY_TYPE: OnceLock<Box<str>> = OnceLock::new();
	ARRAY_TYPE.get_or_init(|| format!("Meshlet[{MAX_MESHLETS}]").into_boxed_str())
}

/// Parses one reusable BESL helper function from an isolated source scope.
fn parse_besl_function(source: &'static str, function_name: &str) -> besl::parser::Node<'static> {
	let mut root = besl::parse(source).unwrap_or_else(|_| {
		panic!(
			"Failed to parse `{function_name}`. The most likely cause is invalid BESL syntax in the visibility shader module."
		)
	});

	match root.node_mut() {
		besl::parser::Nodes::Scope { children, .. } if children.len() == 1 => children.remove(0),
		_ => panic!(
			"Invalid `{function_name}` helper scope. The most likely cause is that its BESL source defines more than one top-level element."
		),
	}
}

/// The `VisibilityShaderScope` struct provides material programs with the shared visibility data and lighting contract.
pub struct VisibilityShaderScope {}

/// The `VisibilityShaderGenerator` struct adapts portable material programs for visibility-buffer evaluation.
pub struct VisibilityShaderGenerator {
	scope: besl::parser::Node<'static>,
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
		let view_struct = Node::r#struct(
			"View",
			vec![
				Node::member("view", "mat4f"),
				Node::member("projection", "mat4f"),
				Node::member("view_projection", "mat4f"),
				Node::member("inverse_view", "mat4f"),
				Node::member("inverse_projection", "mat4f"),
				Node::member("inverse_view_projection", "mat4f"),
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
				Node::member("center_radius", "vec4f"),
				Node::member("cone_apex_cutoff", "vec4f"),
				Node::member("cone_axis", "vec4f"),
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
			],
		);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", material_texture_array_type())]);

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
			Node::buffer("Normals", vec![Node::member("normals", vertex_vec3_array_type())]),
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
			Node::buffer("UVs", vec![Node::member("uvs", vertex_vec2_array_type())]),
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
		let material_evaluation_dispatches = Node::device_buffer_binding(
			"material_evaluation_dispatches",
			Node::buffer(
				"MaterialEvaluationDispatches",
				vec![Node::member("material_evaluation_dispatches", "vec4u[1024]")],
			),
			1036,
			material_offset_read,
			material_offset_write,
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

		let compute_vertex_index = {
			let mut root = besl::parse(
				r#"
				compute_vertex_index: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> u32 {
					let relative_index: u16 = vertex_indices.vertex_indices[
						mesh.base_primitive_index + meshlet.primitive_offset + primitive_index
					];
					return mesh.base_vertex_index + u16_to_u32(relative_index);
				}
				"#,
			)
			.expect("Expected compute_vertex_index source to parse");

			match root.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => children.remove(0),
				_ => panic!(
					"Expected compute_vertex_index source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};
		let u16_to_u32 = parse_besl_function("u16_to_u32: fn (value: u16) -> u32 { return u32(value); }", "u16_to_u32");
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
		let cone_shadow_map = Node::binding("cone_shadow_map", Node::combined_array_image_sampler(), 1064, true, false);
		let environment_irradiance = Node::binding("environment_irradiance", Node::combined_image_sampler(), 1054, true, false);
		let environment_specular = Node::binding_array(
			"environment_specular",
			Node::combined_image_sampler(),
			1055,
			true,
			false,
			IBL_PREFILTERED_SPECULAR_MIP_COUNT,
		);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32"), Node::member("blend", "u32")]);

		let sample_function = Node::intrinsic(
			"sample_material",
			Node::parameter("smplr", "u32"),
			Node::sentence(vec![
				Node::raw_code(
					Some("texture(textures[nonuniformEXT(material.textures[".into()),
					Some("textures[material.textures[".into()),
					Some("resources.textures[material.textures[".into()),
					&["textures"],
					&[],
				),
				Node::member_expression("smplr"),
				Node::raw_code(
					Some("])], vertex_uv)".into()),
					Some("]].SampleLevel(textures_sampler, vertex_uv, 0.0)".into()),
					Some("]].sample(resources.textures_sampler[material.textures[smplr]], vertex_uv, level(0.0))".into()),
					&["textures"],
					&[],
				),
			]),
			"vec4f",
		);

		let sample_normal_function = if true {
			Node::intrinsic(
				"sample_normal",
				Node::parameter("smplr", "u32"),
				Node::sentence(vec![
					Node::raw_code(
						Some("unit_vector_from_xy(texture(textures[nonuniformEXT(material.textures[".into()),
						Some("unit_vector_from_xy(textures[material.textures[".into()),
						Some("unit_vector_from_xy(resources.textures[material.textures[".into()),
						&["textures", "unit_vector_from_xy"],
						&[],
					),
					Node::member_expression("smplr"),
					Node::raw_code(
						Some("])], vertex_uv).xy)".into()),
						Some("]].SampleLevel(textures_sampler, vertex_uv, 0.0).xy)".into()),
						Some(
							"]].sample(resources.textures_sampler[material.textures[smplr]], vertex_uv, level(0.0)).xy)".into(),
						),
						&["textures", "unit_vector_from_xy"],
						&[],
					),
				]),
				"vec3f",
			)
		} else {
			Node::intrinsic(
				"sample_normal",
				Node::parameter("smplr", "u32"),
				Node::sentence(vec![
					Node::glsl("normalize(texture(", &[], &[]),
					Node::member_expression("smplr"),
					Node::glsl(", vertex_uv).xyz * 2.0f - 1.0f)", &[], &[]),
				]),
				"vec3f",
			)
		};
		// Depth comparison is "inverted" because the depth buffer is stored in a reversed manner
		let sample_shadow_tap = Node::function(
			"sample_shadow_tap",
			vec![
				Node::parameter("shadow_map", "ArrayTexture2D"),
				Node::parameter("shadow_uv", "vec2f"),
				Node::parameter("surface_depth", "f32"),
				Node::parameter("offset", "vec2f"),
				Node::parameter("shadow_layer", "u32"),
				Node::parameter("shadow_map_extent", "vec2i"),
			],
			"f32",
			vec![Node::raw_code(
				Some(
					"
			vec2 offset_shadow_uv = shadow_uv + offset;
			if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) { return 1.0; }
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			ivec2 shadow_texel = ivec2(clamp(offset_shadow_uv * vec2(shadow_map_extent), vec2(0.0), vec2(shadow_map_extent - 1)));
			float closest_depth = texelFetch(shadow_map, ivec3(shadow_texel, int(shadow_layer)), 0).r;

			return surface_depth < closest_depth ? 0.0 : 1.0"
						.into(),
				),
				Some(
					"
			float2 offset_shadow_uv = shadow_uv + offset;
			if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) { return 1.0; }
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			int2 shadow_texel = int2(clamp(offset_shadow_uv * float2(shadow_map_extent), float2(0.0, 0.0), float2(shadow_map_extent - int2(1, 1))));
			float closest_depth = shadow_map.Load(int4(shadow_texel, int(shadow_layer), 0)).x;

			return surface_depth < closest_depth ? 0.0 : 1.0"
						.into(),
				),
				Some(
					"
			float2 offset_shadow_uv = shadow_uv + offset;
			if (offset_shadow_uv.x < 0.0 || offset_shadow_uv.x > 1.0 || offset_shadow_uv.y < 0.0 || offset_shadow_uv.y > 1.0) { return 1.0; }
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			int2 shadow_texel = int2(clamp(offset_shadow_uv * float2(shadow_map_extent), float2(0.0), float2(shadow_map_extent - 1)));
			float closest_depth = shadow_map.read(uint2(shadow_texel), shadow_layer).x;

			return surface_depth < closest_depth ? 0.0 : 1.0"
						.into(),
				),
				&[],
				&[],
			)],
		);

		let sample_shadow = Node::function(
			"sample_shadow",
			vec![
				Node::parameter("shadow_map", "ArrayTexture2D"),
				Node::parameter("light", "Light"),
				Node::parameter("world_space_position", "vec3f"),
				Node::parameter("view_space_position", "vec3f"),
				Node::parameter("surface_normal", "vec3f"),
				Node::parameter("surface_to_light_direction", "vec3f"),
			],
			"f32",
			vec![Node::raw_code(
				Some("if (light.shadow_views[0] == 0u) { return 1.0; }
			uint shadow_view_index = light.shadow_views[0];
			uint shadow_layer = light.shadow_layer;
			float bias_scale = 1.0f;
			if (light.type == 68) {
				float depth_value = abs(view_space_position.z);
				uint cascade_index = 3;
				for (uint i = 0; i < 4; ++i) {
					if (depth_value < views.views[light.shadow_views[i]].far) { cascade_index = i; break; }
				}
				shadow_view_index = light.shadow_views[cascade_index];
				shadow_layer = cascade_index;
				bias_scale = float(cascade_index + 1u);
			}
			View shadow_view = views.views[shadow_view_index];
			vec4 surface_light_clip_position = shadow_view.view_projection * vec4(world_space_position, 1.0);
			vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
			vec2 shadow_uv = vec2(
				surface_light_ndc_position.x * 0.5f + 0.5f,
				0.5f - surface_light_ndc_position.y * 0.5f
			);
			float normal_alignment = max(dot(normalize(surface_normal), surface_to_light_direction), 0.0);
			float cascade_depth_range = max(shadow_view.far - shadow_view.near, 0.0001f);
			float slope_scaled_bias = 0.0002f * bias_scale * (1.0f - normal_alignment);
			float constant_bias = 0.00002f * bias_scale;
			float cascade_range_bias = cascade_depth_range * 0.0000025f;
			float surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
			float surface_depth = surface_light_ndc_position.z + surface_depth_bias;
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }
			ivec2 shadow_map_extent = textureSize(shadow_map, 0).xy;
			vec2 texel_size = 1.0f / vec2(shadow_map_extent);
			float occlusion = 0.0f;

			const vec2 poisson_disk[8] = vec2[8](
				vec2(-0.613392f,  0.617481f),
				vec2( 0.170019f, -0.040254f),
				vec2(-0.299417f,  0.791925f),
				vec2( 0.645680f,  0.493210f),
				vec2(-0.651784f,  0.717887f),
				vec2( 0.421003f,  0.027070f),
				vec2(-0.817194f, -0.271096f),
				vec2(-0.705374f, -0.668203f)
			);
			float rotation_noise = fract(sin(dot(world_space_position.xz + world_space_position.y, vec2(12.9898f, 78.233f))) * 43758.5453f);
			float rotation_angle = rotation_noise * 6.2831853f;
			mat2 poisson_rotation = mat2(
				cos(rotation_angle), -sin(rotation_angle),
				sin(rotation_angle),  cos(rotation_angle)
			);

			for (int i = 0; i < 8; ++i) {
				vec2 pcf_offset = (poisson_rotation * poisson_disk[i]) * texel_size * 1.5f;
				occlusion += sample_shadow_tap(
					shadow_map,
					shadow_uv,
					surface_depth,
					pcf_offset,
					shadow_layer,
					shadow_map_extent
				);
			}

			return occlusion / 8.0f;".into()),
				Some("if (light.shadow_views[0] == 0u) { return 1.0; }
			uint shadow_view_index = light.shadow_views[0];
			uint shadow_layer = light.shadow_layer;
			float bias_scale = 1.0f;
			if (light.type == 68) {
				float depth_value = abs(view_space_position.z);
				uint cascade_index = 3;
				for (uint i = 0; i < 4; ++i) {
					if (depth_value < views[light.shadow_views[i]].far) { cascade_index = i; break; }
				}
				shadow_view_index = light.shadow_views[cascade_index];
				shadow_layer = cascade_index;
				bias_scale = float(cascade_index + 1u);
			}
			View shadow_view = views[shadow_view_index];
			float4 surface_light_clip_position = mul(shadow_view.view_projection, float4(world_space_position, 1.0));
			float3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
			float2 shadow_uv = float2(
				surface_light_ndc_position.x * 0.5f + 0.5f,
				0.5f - surface_light_ndc_position.y * 0.5f
			);
			float normal_alignment = max(dot(normalize(surface_normal), surface_to_light_direction), 0.0);
			float cascade_depth_range = max(shadow_view.far - shadow_view.near, 0.0001f);
			float slope_scaled_bias = 0.0002f * bias_scale * (1.0f - normal_alignment);
			float constant_bias = 0.00002f * bias_scale;
			float cascade_range_bias = cascade_depth_range * 0.0000025f;
			float surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
			float surface_depth = surface_light_ndc_position.z + surface_depth_bias;
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }
			uint shadow_width; uint shadow_height; uint shadow_layers;
			shadow_map.GetDimensions(shadow_width, shadow_height, shadow_layers);
			int2 shadow_map_extent = int2(shadow_width, shadow_height);
			float2 texel_size = 1.0f / float2(shadow_map_extent);
			float occlusion = 0.0f;

			static const float2 poisson_disk[8] = {
				float2(-0.613392f,  0.617481f),
				float2( 0.170019f, -0.040254f),
				float2(-0.299417f,  0.791925f),
				float2( 0.645680f,  0.493210f),
				float2(-0.651784f,  0.717887f),
				float2( 0.421003f,  0.027070f),
				float2(-0.817194f, -0.271096f),
				float2(-0.705374f, -0.668203f)
			};
			float rotation_noise = frac(sin(dot(world_space_position.xz + world_space_position.y, float2(12.9898f, 78.233f))) * 43758.5453f);
			float rotation_angle = rotation_noise * 6.2831853f;
			float2x2 poisson_rotation = float2x2(
				cos(rotation_angle), -sin(rotation_angle),
				sin(rotation_angle),  cos(rotation_angle)
			);

			for (int i = 0; i < 8; ++i) {
				float2 pcf_offset = mul(poisson_rotation, poisson_disk[i]) * texel_size * 1.5f;
				occlusion += sample_shadow_tap(
					shadow_map,
					shadow_uv,
					surface_depth,
					pcf_offset,
					shadow_layer,
					shadow_map_extent
				);
			}

			return occlusion / 8.0f;".into()),
				Some(
					"if (light.shadow_views[0] == 0u) { return 1.0; }
			uint shadow_view_index = light.shadow_views[0];
			uint shadow_layer = light.shadow_layer;
			float bias_scale = 1.0f;
			if (light.type == 68) {
				float depth_value = abs(view_space_position.z);
				uint cascade_index = 3;
				for (uint i = 0; i < 4; ++i) {
					if (depth_value < resources.views->views[light.shadow_views[i]].far) { cascade_index = i; break; }
				}
				shadow_view_index = light.shadow_views[cascade_index];
				shadow_layer = cascade_index;
				bias_scale = float(cascade_index + 1u);
			}
			View shadow_view = resources.views->views[shadow_view_index];
			float4 surface_light_clip_position = shadow_view.view_projection * float4(world_space_position, 1.0);
			float3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
			float2 shadow_uv = float2(
				surface_light_ndc_position.x * 0.5f + 0.5f,
				0.5f - surface_light_ndc_position.y * 0.5f
			);
			float normal_alignment = max(dot(normalize(surface_normal), surface_to_light_direction), 0.0);
			float cascade_depth_range = max(shadow_view.far - shadow_view.near, 0.0001f);
			float slope_scaled_bias = 0.0002f * bias_scale * (1.0f - normal_alignment);
			float constant_bias = 0.00002f * bias_scale;
			float cascade_range_bias = cascade_depth_range * 0.0000025f;
			float surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
			float surface_depth = surface_light_ndc_position.z + surface_depth_bias;
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }
			int2 shadow_map_extent = int2(shadow_map.get_width(), shadow_map.get_height());
			float2 texel_size = 1.0f / float2(shadow_map_extent);
			float occlusion = 0.0f;

			const float2 poisson_disk[8] = {
				float2(-0.613392f,  0.617481f),
				float2( 0.170019f, -0.040254f),
				float2(-0.299417f,  0.791925f),
				float2( 0.645680f,  0.493210f),
				float2(-0.651784f,  0.717887f),
				float2( 0.421003f,  0.027070f),
				float2(-0.817194f, -0.271096f),
				float2(-0.705374f, -0.668203f)
			};
			float rotation_noise = fract(sin(dot(world_space_position.xz + world_space_position.y, float2(12.9898f, 78.233f))) * 43758.5453f);
			float rotation_angle = rotation_noise * 6.2831853f;
			float2x2 poisson_rotation = float2x2(
				float2(cos(rotation_angle), sin(rotation_angle)),
				float2(-sin(rotation_angle),  cos(rotation_angle))
			);

			for (int i = 0; i < 8; ++i) {
				float2 pcf_offset = (poisson_disk[i] * poisson_rotation) * texel_size * 1.5f;
				occlusion += sample_shadow_tap(
					shadow_map,
					shadow_uv,
					surface_depth,
					pcf_offset,
					shadow_layer,
					shadow_map_extent
				);
			}

			return occlusion / 8.0f;"
						.into(),
				),
				&["sample_shadow_tap", "views"],
				&[],
			)],
		);

		let sample_environment_irradiance = Node::function(
			"sample_environment_irradiance",
			vec![Node::parameter("direction", "vec3f")],
			"vec3f",
			vec![Node::raw_code(
				Some(
					"
			vec3 dir = normalize(direction);
			vec2 environment_uv = vec2(
				atan(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			float environment_half_texel = 0.5 / float(textureSize(environment_irradiance, 0).y);
			environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
			vec4 environment_sample = textureLod(environment_irradiance, environment_uv, 0.0);
			return environment_sample.rgb;"
						.into(),
				),
				Some(
					"
			float3 dir = normalize(direction);
			float2 environment_uv = float2(
				atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			uint environment_width = 0u;
			uint environment_height = 0u;
			environment_irradiance.GetDimensions(environment_width, environment_height);
			float environment_half_texel = 0.5 / float(environment_height);
			environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
			float4 environment_sample = environment_irradiance.SampleLevel(environment_irradiance_sampler, environment_uv, 0.0);
			return environment_sample.rgb;"
						.into(),
				),
				Some(
					"
			float3 dir = normalize(direction);
			float2 environment_uv = float2(
				atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			float environment_half_texel = 0.5 / float(resources.environment_irradiance.get_height());
			environment_uv.y = clamp(environment_uv.y, environment_half_texel, 1.0 - environment_half_texel);
			float4 environment_sample = resources.environment_irradiance.sample(resources.environment_irradiance_sampler, environment_uv, level(0.0));
			return environment_sample.rgb;"
						.into(),
				),
				&["environment_irradiance"],
				&[],
			)],
		);
		let sample_environment_specular = Node::function(
			"sample_environment_specular",
			vec![Node::parameter("direction", "vec3f"), Node::parameter("roughness", "f32")],
			"vec3f",
			vec![Node::raw_code(
				Some(
					"
			vec3 dir = normalize(direction);
			vec2 environment_uv = vec2(
				atan(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			float specular_level = clamp(roughness, 0.0, 1.0) * 7.0;
			uint lower_level = uint(floor(specular_level));
			uint upper_level = min(lower_level + 1u, 7u);
			float lower_half_texel = 0.5 / float(textureSize(environment_specular[nonuniformEXT(lower_level)], 0).y);
			float upper_half_texel = 0.5 / float(textureSize(environment_specular[nonuniformEXT(upper_level)], 0).y);
			vec2 lower_uv = vec2(environment_uv.x, clamp(environment_uv.y, lower_half_texel, 1.0 - lower_half_texel));
			vec2 upper_uv = vec2(environment_uv.x, clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel));
			vec4 lower_sample = textureLod(environment_specular[nonuniformEXT(lower_level)], lower_uv, 0.0);
			vec4 upper_sample = textureLod(environment_specular[nonuniformEXT(upper_level)], upper_uv, 0.0);
			return mix(lower_sample.rgb, upper_sample.rgb, fract(specular_level));"
						.into(),
				),
				Some(
					"
			float3 dir = normalize(direction);
			float2 environment_uv = float2(
				atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			float specular_level = clamp(roughness, 0.0, 1.0) * 7.0;
			uint lower_level = uint(floor(specular_level));
			uint upper_level = min(lower_level + 1u, 7u);
			uint lower_index = NonUniformResourceIndex(lower_level);
			uint upper_index = NonUniformResourceIndex(upper_level);
			uint lower_width = 0u;
			uint lower_height = 0u;
			uint upper_width = 0u;
			uint upper_height = 0u;
			environment_specular[lower_index].GetDimensions(lower_width, lower_height);
			environment_specular[upper_index].GetDimensions(upper_width, upper_height);
			float lower_half_texel = 0.5 / float(lower_height);
			float upper_half_texel = 0.5 / float(upper_height);
			float2 lower_uv = float2(environment_uv.x, clamp(environment_uv.y, lower_half_texel, 1.0 - lower_half_texel));
			float2 upper_uv = float2(environment_uv.x, clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel));
			float4 lower_sample = environment_specular[lower_index].SampleLevel(environment_specular_sampler, lower_uv, 0.0);
			float4 upper_sample = environment_specular[upper_index].SampleLevel(environment_specular_sampler, upper_uv, 0.0);
			return lerp(lower_sample.rgb, upper_sample.rgb, frac(specular_level));"
						.into(),
				),
				Some(
					"
			float3 dir = normalize(direction);
			float2 environment_uv = float2(
				atan2(dir.z, dir.x) * 0.15915494309189535 + 0.5,
				0.5 - asin(clamp(dir.y, -1.0, 1.0)) * 0.3183098861837907
			);
			float specular_level = clamp(roughness, 0.0, 1.0) * 7.0;
			uint lower_level = uint(floor(specular_level));
			uint upper_level = min(lower_level + 1u, 7u);
			float lower_half_texel = 0.5 / float(resources.environment_specular[lower_level].get_height());
			float upper_half_texel = 0.5 / float(resources.environment_specular[upper_level].get_height());
			float2 lower_uv = float2(environment_uv.x, clamp(environment_uv.y, lower_half_texel, 1.0 - lower_half_texel));
			float2 upper_uv = float2(environment_uv.x, clamp(environment_uv.y, upper_half_texel, 1.0 - upper_half_texel));
			float4 lower_sample = resources.environment_specular[lower_level].sample(resources.environment_specular_sampler[lower_level], lower_uv, level(0.0));
			float4 upper_sample = resources.environment_specular[upper_level].sample(resources.environment_specular_sampler[upper_level], upper_uv, level(0.0));
			return mix(lower_sample.rgb, upper_sample.rgb, fract(specular_level));"
						.into(),
				),
				&["environment_specular"],
				&[],
			)],
		);

		Node::scope(
			"Visibility",
			vec![
				view_struct,
				views_binding,
				mesh_struct,
				skinned_vertex_struct,
				meshlet_struct,
				light_struct,
				material_struct,
				sample_shadow_tap,
				sample_shadow,
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
				material_evaluation_dispatches,
				pixel_mapping,
				triangle_index,
				instance_index,
				u16_to_u32,
				cone_attenuation,
				compute_vertex_index,
				set2_binding0,
				set2_binding4,
				set2_binding5,
				set2_binding10,
				set2_binding11,
				cone_shadow_map,
				environment_irradiance,
				environment_specular,
				push_constant,
				sample_function,
				sample_normal_function,
				sample_environment_irradiance,
				sample_environment_specular,
			],
		)
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
		let a = "if (gl_GlobalInvocationID.x >= material_evaluation_dispatches.material_evaluation_dispatches[push_constant.material_id].w) { return; }

		uint offset = material_offset.material_offset[push_constant.material_id];
		uvec2 raw_pixel_coordinates = uvec2(pixel_mapping.pixel_mapping[offset + gl_GlobalInvocationID.x]);
		if (raw_pixel_coordinates.x == 0u || raw_pixel_coordinates.y == 0u) { return; }
		ivec2 pixel_coordinates = ivec2(raw_pixel_coordinates) - ivec2(1);
		ivec2 pixel_mapping_extent = imageSize(triangle_index);
		if (pixel_coordinates.x < 0 || pixel_coordinates.y < 0 || pixel_coordinates.x >= pixel_mapping_extent.x || pixel_coordinates.y >= pixel_mapping_extent.y) { return; }
		uint triangle_meshlet_indices = imageLoad(triangle_index, pixel_coordinates).r;
		uint instance_index = imageLoad(instance_index_render_target, pixel_coordinates).r;
		uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
		uint meshlet_index = triangle_meshlet_indices >> 8;

		Meshlet meshlet = meshlets.meshlets[meshlet_index];

		Mesh mesh = meshes.meshes[instance_index];

		Material material = materials.materials[push_constant.material_id];

		uint primitive_index_base = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3;
		uint primitive_index0 = primitive_indices.primitive_indices[primitive_index_base];
		uint primitive_index1 = primitive_indices.primitive_indices[primitive_index_base + 1];
		uint primitive_index2 = primitive_indices.primitive_indices[primitive_index_base + 2];
		uint vertex_index0 = compute_vertex_index(mesh, meshlet, primitive_index0);
		uint vertex_index1 = compute_vertex_index(mesh, meshlet, primitive_index1);
		uint vertex_index2 = compute_vertex_index(mesh, meshlet, primitive_index2);

		vec4 model_space_vertex_position0 = vec4(vertex_positions.positions[vertex_index0], 1.0);
		vec4 model_space_vertex_position1 = vec4(vertex_positions.positions[vertex_index1], 1.0);
		vec4 model_space_vertex_position2 = vec4(vertex_positions.positions[vertex_index2], 1.0);
		vec4 vertex_normal0 = vec4(vertex_normals.normals[vertex_index0], 0.0);
		vec4 vertex_normal1 = vec4(vertex_normals.normals[vertex_index1], 0.0);
		vec4 vertex_normal2 = vec4(vertex_normals.normals[vertex_index2], 0.0);

		// Use scalars for the three triangle vertices so Metal can keep the hot path out of thread-local array storage.
		if (mesh.skinned_base_vertex_index != 4294967295u) {
			uint skinned_vertex_index0 = mesh.skinned_base_vertex_index + (vertex_index0 - mesh.base_vertex_index);
			uint skinned_vertex_index1 = mesh.skinned_base_vertex_index + (vertex_index1 - mesh.base_vertex_index);
			uint skinned_vertex_index2 = mesh.skinned_base_vertex_index + (vertex_index2 - mesh.base_vertex_index);
			model_space_vertex_position0 = skinned_vertices.vertices[skinned_vertex_index0].position;
			model_space_vertex_position1 = skinned_vertices.vertices[skinned_vertex_index1].position;
			model_space_vertex_position2 = skinned_vertices.vertices[skinned_vertex_index2].position;
			vertex_normal0 = skinned_vertices.vertices[skinned_vertex_index0].normal;
			vertex_normal1 = skinned_vertices.vertices[skinned_vertex_index1].normal;
			vertex_normal2 = skinned_vertices.vertices[skinned_vertex_index2].normal;
		}

		vec2 vertex_uv0 = vertex_uvs.uvs[vertex_index0];
		vec2 vertex_uv1 = vertex_uvs.uvs[vertex_index1];
		vec2 vertex_uv2 = vertex_uvs.uvs[vertex_index2];

		ivec2 image_extent = imageSize(triangle_index);
		vec2 nc = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);

		View view = views.views[0];

		mat4x3 model = mesh.model;
		vec3 world_space_vertex_position0 = model * model_space_vertex_position0;
		vec3 world_space_vertex_position1 = model * model_space_vertex_position1;
		vec3 world_space_vertex_position2 = model * model_space_vertex_position2;
		vec4 clip_space_vertex_position0 = view.view_projection * vec4(world_space_vertex_position0, 1.0);
		vec4 clip_space_vertex_position1 = view.view_projection * vec4(world_space_vertex_position1, 1.0);
		vec4 clip_space_vertex_position2 = view.view_projection * vec4(world_space_vertex_position2, 1.0);
		vec3 world_space_vertex_normal0 = normalize(model * vertex_normal0);
		vec3 world_space_vertex_normal1 = normalize(model * vertex_normal1);
		vec3 world_space_vertex_normal2 = normalize(model * vertex_normal2);

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_position0, clip_space_vertex_position1, clip_space_vertex_position2, nc, vec2(image_extent));
		vec3 barycenter = barycentric_deriv.lambda;
		vec3 ddx = barycentric_deriv.ddx;
		vec3 ddy = barycentric_deriv.ddy;

		vec3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		vec3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_position0.xyz, clip_space_vertex_position1.xyz, clip_space_vertex_position2.xyz);
		vec3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normal0, world_space_vertex_normal1, world_space_vertex_normal2));
		vec2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uv0, vertex_uv1, vertex_uv2);

		vec3 N = world_space_vertex_normal;
		vec3 camera_position = (view.inverse_view * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
		vec3 V = normalize(camera_position - world_space_vertex_position);

		vec3 pos_dx = interpolate_vec3f_with_deriv(ddx, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		vec3 pos_dy = interpolate_vec3f_with_deriv(ddy, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);

		vec2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uv0, vertex_uv1, vertex_uv2);
		vec2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uv0, vertex_uv1, vertex_uv2);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		vec3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		vec3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
		mat3 TBN = mat3(T, B, N);

		vec4 albedo = vec4(1, 0, 0, 1);
		vec3 normal = vec3(0, 0, 1);
		float metalness = 0.0;
		float roughness = float(0.5);
		float occlusion = 1.0;
		vec3 emission = vec3(0.0)"
			.trim();

		let a_msl = "if (gid.x >= resources.material_evaluation_dispatches->material_evaluation_dispatches[push_constant.material_id].w) { return; }

		uint offset = resources.material_offset->material_offset[push_constant.material_id];
		uint2 raw_pixel_coordinates = uint2(resources.pixel_mapping->pixel_mapping[offset + gid.x]);
		if (raw_pixel_coordinates.x == 0u || raw_pixel_coordinates.y == 0u) { return; }
		int2 pixel_coordinates = int2(raw_pixel_coordinates) - int2(1, 1);
		int2 image_extent = int2(resources.triangle_index.get_width(), resources.triangle_index.get_height());
		if (pixel_coordinates.x < 0 || pixel_coordinates.y < 0 || pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) { return; }
		uint triangle_meshlet_indices = resources.triangle_index.read(uint2(pixel_coordinates)).x;
		uint instance_index = resources.instance_index_render_target.read(uint2(pixel_coordinates)).x;
		uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
		uint meshlet_index = triangle_meshlet_indices >> 8;

		Meshlet meshlet = resources.meshlets->meshlets[meshlet_index];

		Mesh mesh = resources.meshes->meshes[instance_index];

		Material material = resources.materials->materials[push_constant.material_id];

		uint primitive_index_base = (mesh.base_triangle_index + uint(meshlet.triangle_offset) + meshlet_triangle_index) * 3;
		uint primitive_index0 = resources.primitive_indices->primitive_indices[primitive_index_base];
		uint primitive_index1 = resources.primitive_indices->primitive_indices[primitive_index_base + 1];
		uint primitive_index2 = resources.primitive_indices->primitive_indices[primitive_index_base + 2];
		uint vertex_index0 = compute_vertex_index(mesh, meshlet, primitive_index0, gid, push_constant, resources);
		uint vertex_index1 = compute_vertex_index(mesh, meshlet, primitive_index1, gid, push_constant, resources);
		uint vertex_index2 = compute_vertex_index(mesh, meshlet, primitive_index2, gid, push_constant, resources);

		float4 model_space_vertex_position0 = float4(resources.vertex_positions->positions[vertex_index0], 1.0);
		float4 model_space_vertex_position1 = float4(resources.vertex_positions->positions[vertex_index1], 1.0);
		float4 model_space_vertex_position2 = float4(resources.vertex_positions->positions[vertex_index2], 1.0);
		float4 vertex_normal0 = float4(resources.vertex_normals->normals[vertex_index0], 0.0);
		float4 vertex_normal1 = float4(resources.vertex_normals->normals[vertex_index1], 0.0);
		float4 vertex_normal2 = float4(resources.vertex_normals->normals[vertex_index2], 0.0);

		// Use scalars for the three triangle vertices so Metal can keep the hot path out of thread-local array storage.
		if (mesh.skinned_base_vertex_index != 4294967295u) {
			uint skinned_vertex_index0 = mesh.skinned_base_vertex_index + (vertex_index0 - mesh.base_vertex_index);
			uint skinned_vertex_index1 = mesh.skinned_base_vertex_index + (vertex_index1 - mesh.base_vertex_index);
			uint skinned_vertex_index2 = mesh.skinned_base_vertex_index + (vertex_index2 - mesh.base_vertex_index);
			model_space_vertex_position0 = resources.skinned_vertices->vertices[skinned_vertex_index0].position;
			model_space_vertex_position1 = resources.skinned_vertices->vertices[skinned_vertex_index1].position;
			model_space_vertex_position2 = resources.skinned_vertices->vertices[skinned_vertex_index2].position;
			vertex_normal0 = resources.skinned_vertices->vertices[skinned_vertex_index0].normal;
			vertex_normal1 = resources.skinned_vertices->vertices[skinned_vertex_index1].normal;
			vertex_normal2 = resources.skinned_vertices->vertices[skinned_vertex_index2].normal;
		}

		float2 vertex_uv0 = resources.vertex_uvs->uvs[vertex_index0];
		float2 vertex_uv1 = resources.vertex_uvs->uvs[vertex_index1];
		float2 vertex_uv2 = resources.vertex_uvs->uvs[vertex_index2];

		float2 nc = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);

		View view = resources.views->views[0];

		float4x3 model = mesh.model;
		float3 world_space_vertex_position0 = model * model_space_vertex_position0;
		float3 world_space_vertex_position1 = model * model_space_vertex_position1;
		float3 world_space_vertex_position2 = model * model_space_vertex_position2;
		float4 clip_space_vertex_position0 = view.view_projection * float4(world_space_vertex_position0, 1.0);
		float4 clip_space_vertex_position1 = view.view_projection * float4(world_space_vertex_position1, 1.0);
		float4 clip_space_vertex_position2 = view.view_projection * float4(world_space_vertex_position2, 1.0);
		float3 world_space_vertex_normal0 = normalize(model * vertex_normal0);
		float3 world_space_vertex_normal1 = normalize(model * vertex_normal1);
		float3 world_space_vertex_normal2 = normalize(model * vertex_normal2);

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_position0, clip_space_vertex_position1, clip_space_vertex_position2, nc, float2(image_extent));
		float3 barycenter = barycentric_deriv.lambda;
		float3 ddx = barycentric_deriv.ddx;
		float3 ddy = barycentric_deriv.ddy;

		float3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		float3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_position0.xyz, clip_space_vertex_position1.xyz, clip_space_vertex_position2.xyz);
		float3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normal0, world_space_vertex_normal1, world_space_vertex_normal2));
		float2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uv0, vertex_uv1, vertex_uv2);

		float3 N = world_space_vertex_normal;
		float3 camera_position = (view.inverse_view * float4(0.0, 0.0, 0.0, 1.0)).xyz;
		float3 V = normalize(camera_position - world_space_vertex_position);

		float3 pos_dx = interpolate_vec3f_with_deriv(ddx, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		float3 pos_dy = interpolate_vec3f_with_deriv(ddy, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);

		float2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uv0, vertex_uv1, vertex_uv2);
		float2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uv0, vertex_uv1, vertex_uv2);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		float3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		float3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
		float3x3 TBN = float3x3(T, B, N);

		float4 albedo = float4(1, 0, 0, 1);
		float3 normal = float3(0, 0, 1);
		float metalness = 0.0;
		float roughness = float(0.5);
		float occlusion = 1.0;
		float3 emission = float3(0.0, 0.0, 0.0)"
			.trim();

		let a_hlsl = "if (dispatch_thread_id.x >= material_evaluation_dispatches[push_constant.material_id].w) { return; }

		uint offset = material_offset[push_constant.material_id];
		uint2 raw_pixel_coordinates = uint2(pixel_mapping[offset + dispatch_thread_id.x]);
		if (raw_pixel_coordinates.x == 0u || raw_pixel_coordinates.y == 0u) { return; }
		int2 pixel_coordinates = int2(raw_pixel_coordinates) - int2(1, 1);
		uint triangle_width; uint triangle_height;
		triangle_index.GetDimensions(triangle_width, triangle_height);
		int2 image_extent = int2(triangle_width, triangle_height);
		if (pixel_coordinates.x < 0 || pixel_coordinates.y < 0 || pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) { return; }
		uint triangle_meshlet_indices = triangle_index[pixel_coordinates];
		uint instance_index = instance_index_render_target[pixel_coordinates];
		uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
		uint meshlet_index = triangle_meshlet_indices >> 8;

		Meshlet meshlet = meshlets[meshlet_index];
		Mesh mesh = meshes[instance_index];
		Material material = materials[push_constant.material_id];

		// DX12 exposes the tightly packed u8 primitive index buffer as 32-bit words.
		uint primitive_indices_base = (mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3u;
		uint primitive_indices_word0 = primitive_indices[primitive_indices_base >> 2u];
		uint primitive_indices_word1 = primitive_indices[(primitive_indices_base + 1u) >> 2u];
		uint primitive_indices_word2 = primitive_indices[(primitive_indices_base + 2u) >> 2u];
		uint primitive_index0 = (primitive_indices_word0 >> ((primitive_indices_base & 3u) * 8u)) & 0xffu;
		uint primitive_index1 = (primitive_indices_word1 >> (((primitive_indices_base + 1u) & 3u) * 8u)) & 0xffu;
		uint primitive_index2 = (primitive_indices_word2 >> (((primitive_indices_base + 2u) & 3u) * 8u)) & 0xffu;
		uint vertex_index0 = compute_vertex_index(mesh, meshlet, primitive_index0);
		uint vertex_index1 = compute_vertex_index(mesh, meshlet, primitive_index1);
		uint vertex_index2 = compute_vertex_index(mesh, meshlet, primitive_index2);

		float4 model_space_vertex_position0 = float4(vertex_positions[vertex_index0], 1.0);
		float4 model_space_vertex_position1 = float4(vertex_positions[vertex_index1], 1.0);
		float4 model_space_vertex_position2 = float4(vertex_positions[vertex_index2], 1.0);
		float4 vertex_normal0 = float4(vertex_normals[vertex_index0], 0.0);
		float4 vertex_normal1 = float4(vertex_normals[vertex_index1], 0.0);
		float4 vertex_normal2 = float4(vertex_normals[vertex_index2], 0.0);

		// Use scalars for the three triangle vertices so Metal can keep the hot path out of thread-local array storage.
		if (mesh.skinned_base_vertex_index != 4294967295u) {
			uint skinned_vertex_index0 = mesh.skinned_base_vertex_index + (vertex_index0 - mesh.base_vertex_index);
			uint skinned_vertex_index1 = mesh.skinned_base_vertex_index + (vertex_index1 - mesh.base_vertex_index);
			uint skinned_vertex_index2 = mesh.skinned_base_vertex_index + (vertex_index2 - mesh.base_vertex_index);
			model_space_vertex_position0 = skinned_vertices[skinned_vertex_index0].position;
			model_space_vertex_position1 = skinned_vertices[skinned_vertex_index1].position;
			model_space_vertex_position2 = skinned_vertices[skinned_vertex_index2].position;
			vertex_normal0 = skinned_vertices[skinned_vertex_index0].normal;
			vertex_normal1 = skinned_vertices[skinned_vertex_index1].normal;
			vertex_normal2 = skinned_vertices[skinned_vertex_index2].normal;
		}

		float2 vertex_uv0 = vertex_uvs[vertex_index0];
		float2 vertex_uv1 = vertex_uvs[vertex_index1];
		float2 vertex_uv2 = vertex_uvs[vertex_index2];

		float2 nc = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);

		View view = views[0];

		float4x3 model = mesh.model;
		float3 world_space_vertex_position0 = mul(model_space_vertex_position0, model);
		float3 world_space_vertex_position1 = mul(model_space_vertex_position1, model);
		float3 world_space_vertex_position2 = mul(model_space_vertex_position2, model);
		float4 clip_space_vertex_position0 = mul(view.view_projection, float4(world_space_vertex_position0, 1.0));
		float4 clip_space_vertex_position1 = mul(view.view_projection, float4(world_space_vertex_position1, 1.0));
		float4 clip_space_vertex_position2 = mul(view.view_projection, float4(world_space_vertex_position2, 1.0));
		float3 world_space_vertex_normal0 = normalize(mul(vertex_normal0, model));
		float3 world_space_vertex_normal1 = normalize(mul(vertex_normal1, model));
		float3 world_space_vertex_normal2 = normalize(mul(vertex_normal2, model));

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_position0, clip_space_vertex_position1, clip_space_vertex_position2, nc, float2(image_extent));
		float3 barycenter = barycentric_deriv.lambda;
		float3 ddx = barycentric_deriv.ddx;
		float3 ddy = barycentric_deriv.ddy;

		float3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		float3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_position0.xyz, clip_space_vertex_position1.xyz, clip_space_vertex_position2.xyz);
		float3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normal0, world_space_vertex_normal1, world_space_vertex_normal2));
		float2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uv0, vertex_uv1, vertex_uv2);

		float3 N = world_space_vertex_normal;
		float3 camera_position = mul(view.inverse_view, float4(0.0, 0.0, 0.0, 1.0)).xyz;
		float3 V = normalize(camera_position - world_space_vertex_position);

		float3 pos_dx = interpolate_vec3f_with_deriv(ddx, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);
		float3 pos_dy = interpolate_vec3f_with_deriv(ddy, world_space_vertex_position0, world_space_vertex_position1, world_space_vertex_position2);

		float2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uv0, vertex_uv1, vertex_uv2);
		float2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uv0, vertex_uv1, vertex_uv2);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		float3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		float3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));

		float4 albedo = float4(1, 0, 0, 1);
		float3 normal = float3(0, 0, 1);
		float metalness = 0.0;
		float roughness = float(0.5);
		float occlusion = 1.0;
		float3 emission = float3(0.0, 0.0, 0.0)"
			.trim();

		let mut extra: Vec<Node<'a>> = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].as_array().unwrap().iter() {
			let name = variable["name"].as_str().unwrap();
			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4f" => {
					let x = besl::parser::Node::specialization(name, data_type);
					extra.push(x);
				}
				"Texture2D" => {
					let slot = format!("{texture_count}u");
					let slot_node = besl::parser::Node::literal_expression(slot);
					let x = besl::parser::Node::constant(name, "u32", slot_node);
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let b_msl = "
		float3 diffuse = float3(0.0, 0.0, 0.0);
		float3 specular = float3(0.0, 0.0, 0.0);

		// GTAO belongs to the opaque depth surface. Reusing it for a transparent
		// surface would composite the opaque surface's occlusion over that surface.
		float ao_factor = push_constant.blend != 0u
			? 1.0
			: resources.ao.read(uint2(pixel_coordinates)).r;

		normal = normalize(TBN * normal);
		float3 F0 = mix(float3(0.04), albedo.xyz, metalness);
		float NdotV = max(dot(normal, V), 0.0);
		float roughness_alpha = roughness * roughness;
		float roughness_alpha_squared = roughness_alpha * roughness_alpha;
		float adjusted_roughness = roughness + 1.0;
		float geometry_k = adjusted_roughness * adjusted_roughness / 8.0;
		float view_fresnel_factor = pow(clamp(1.0 - NdotV, 0.0, 1.0), 5.0);
		float3 one_minus_fresnel_n_dot_v = float3(1.0) - fresnel_schlick_from_factor(view_fresnel_factor, F0);

		for (uint i = 0; i < resources.lighting_data->light_count; ++i) {
			Light light = resources.lighting_data->lights[i];

			float3 L;
			float attenuation = 1.0;

			if (light.type == 68) {
				L = normalize(-light.position.xyz);
			} else {
				float3 surface_to_light = light.position.xyz - world_space_vertex_position;
				float distance_squared = dot(surface_to_light, surface_to_light);
				if (distance_squared <= 0.0) { continue; }
				L = surface_to_light * rsqrt(distance_squared);
				attenuation = 1.0 / distance_squared;
			}

			float NdotL = max(dot(normal, L), 0.0);

			if (NdotL <= 0.0) { continue; }

			float occlusion_factor = 1.0;

			if (light.type == 68) {
				float4 view_space_surface_position = view.view * float4(world_space_vertex_position, 1.0);
				float c_occlusion_factor  = sample_shadow(resources.depth_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L, gid, push_constant, resources);

				occlusion_factor = c_occlusion_factor;

				if (occlusion_factor == 0.0) { continue; }

				attenuation = 1.0;
			} else {
				if (light.type == 1) {
					// Preserve full intensity inside the inner cone and fade to zero at the outer cone.
					float cone_cosine = dot(normalize(light.direction.xyz), -L);
					float cone_factor = cone_attenuation(cone_cosine, light.cone_cosines.x, light.cone_cosines.y);
					if (cone_factor <= 0.0) { continue; }
					attenuation *= cone_factor;
					float4 view_space_surface_position = view.view * float4(world_space_vertex_position, 1.0);
					occlusion_factor = sample_shadow(resources.cone_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L, gid, push_constant, resources);
					if (occlusion_factor == 0.0) { continue; }
				}
			}

			float3 H = normalize(V + L);

			float3 radiance = light.color.xyz * attenuation;

			float half_view_fresnel_factor = pow(clamp(1.0 - max(dot(H, V), 0.0), 0.0, 1.0), 5.0);
			float3 F = fresnel_schlick_from_factor(half_view_fresnel_factor, F0);
			float NDF = distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared);
			float G = geometry_smith_from_terms(NdotV, NdotL, geometry_k);
			float3 local_specular = (NDF * G * F) / (4.0 * NdotV * NdotL + 0.000001);

			float light_fresnel_factor = pow(clamp(1.0 - NdotL, 0.0, 1.0), 5.0);
			float3 kD = (float3(1.0) - fresnel_schlick_from_factor(light_fresnel_factor, F0)) * one_minus_fresnel_n_dot_v;

			kD *= 1.0 - metalness;

			float3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		float3 ambient_irradiance = sample_environment_irradiance(normal, gid, push_constant, resources);
		float3 reflection_direction = reflect(-V, normal);
		float3 reflection_radiance = sample_environment_specular(reflection_direction, roughness, gid, push_constant, resources);

		float3 F_ibl = fresnel_schlick_roughness(NdotV, F0, roughness);
		float3 kD_ibl = (float3(1.0) - F_ibl) * (1.0 - metalness);

		float3 ibl_diffuse = kD_ibl * albedo.xyz * ambient_irradiance;

		float2 env_brdf = float2(1.0, 0.0);
		{
			float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
			float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
			float4 r = roughness * c0 + c1;
			float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
			env_brdf = float2(-1.04, 1.04) * a004 + r.zw;
		}
		float3 ibl_specular = (F0 * env_brdf.x + env_brdf.y) * reflection_radiance;

		float3 ambient = ibl_diffuse + ibl_specular;

		ao_factor *= occlusion;
		float3 lit = (diffuse + specular) * ao_factor + ambient * ao_factor + emission;

		float4 output_color = float4(lit, 1.0);
		if (push_constant.blend != 0u) {
			float source_alpha = clamp(albedo.a, 0.0, 1.0);
			float4 destination_color = resources.lit_map.read(uint2(pixel_coordinates));
			output_color = source_over(float4(lit * source_alpha, source_alpha), destination_color);
		}

		resources.lit_map.write(output_color, uint2(pixel_coordinates))
		"
		.trim();

		let b = "
		vec3 diffuse = vec3(0.0);
		vec3 specular = vec3(0.0);

		// GTAO belongs to the opaque depth surface. Reusing it for a transparent
		// surface would composite the opaque surface's occlusion over that surface.
		float ao_factor = push_constant.blend != 0u
			? 1.0
			: texelFetch(ao, pixel_coordinates, 0).r;

		normal = normalize(TBN * normal);
		vec3 F0 = mix(vec3(0.04), albedo.xyz, metalness);
		float NdotV = max(dot(normal, V), 0.0);
		float roughness_alpha = roughness * roughness;
		float roughness_alpha_squared = roughness_alpha * roughness_alpha;
		float adjusted_roughness = roughness + 1.0;
		float geometry_k = adjusted_roughness * adjusted_roughness / 8.0;
		float view_fresnel_factor = pow(clamp(1.0 - NdotV, 0.0, 1.0), 5.0);
		vec3 one_minus_fresnel_n_dot_v = vec3(1.0) - fresnel_schlick_from_factor(view_fresnel_factor, F0);

		for (uint i = 0; i < lighting_data.light_count; ++i) {
			Light light = lighting_data.lights[i];

			vec3 L;
			float attenuation = 1.0;

			if (light.type == 68) { // Infinite
				L = normalize(-light.position.xyz);
			} else {
				vec3 surface_to_light = light.position.xyz - world_space_vertex_position;
				float distance_squared = dot(surface_to_light, surface_to_light);
				if (distance_squared <= 0.0) { continue; }
				L = surface_to_light * inversesqrt(distance_squared);
				attenuation = 1.0 / distance_squared;
			}

			float NdotL = max(dot(normal, L), 0.0);

			if (NdotL <= 0.0) { continue; }

			float occlusion_factor = 1.0;

			if (light.type == 68) { // Infinite
				vec4 view_space_surface_position = view.view * vec4(world_space_vertex_position, 1.0);
				float c_occlusion_factor  = sample_shadow(depth_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L);

				occlusion_factor = c_occlusion_factor;

				if (occlusion_factor == 0.0) { continue; }

				// attenuation = occlusion_factor;
				attenuation = 1.0;
			} else {
				if (light.type == 1) {
					// Preserve full intensity inside the inner cone and fade to zero at the outer cone.
					float cone_cosine = dot(normalize(light.direction.xyz), -L);
					float cone_factor = cone_attenuation(cone_cosine, light.cone_cosines.x, light.cone_cosines.y);
					if (cone_factor <= 0.0) { continue; }
					attenuation *= cone_factor;
					vec4 view_space_surface_position = view.view * vec4(world_space_vertex_position, 1.0);
					occlusion_factor = sample_shadow(cone_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L);
					if (occlusion_factor == 0.0) { continue; }
				}
			}

			vec3 H = normalize(V + L);

			vec3 radiance = light.color.xyz * attenuation;

			float half_view_fresnel_factor = pow(clamp(1.0 - max(dot(H, V), 0.0), 0.0, 1.0), 5.0);
			vec3 F = fresnel_schlick_from_factor(half_view_fresnel_factor, F0);
			float NDF = distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared);
			float G = geometry_smith_from_terms(NdotV, NdotL, geometry_k);
			vec3 local_specular = (NDF * G * F) / (4.0 * NdotV * NdotL + 0.000001);

			float light_fresnel_factor = pow(clamp(1.0 - NdotL, 0.0, 1.0), 5.0);
			vec3 kD = (vec3(1.0) - fresnel_schlick_from_factor(light_fresnel_factor, F0)) * one_minus_fresnel_n_dot_v;

			kD *= 1.0 - metalness;

			vec3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		vec3 ambient_irradiance = sample_environment_irradiance(normal);
		vec3 reflection_direction = reflect(-V, normal);
		vec3 reflection_radiance = sample_environment_specular(reflection_direction, roughness);

		vec3 F_ibl = fresnel_schlick_roughness(NdotV, F0, roughness);
		vec3 kD_ibl = (vec3(1.0) - F_ibl) * (1.0 - metalness);

		vec3 ibl_diffuse = kD_ibl * albedo.xyz * ambient_irradiance;

		vec2 env_brdf = vec2(1.0, 0.0);
		{
			vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
			vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
			vec4 r = roughness * c0 + c1;
			float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
			env_brdf = vec2(-1.04, 1.04) * a004 + r.zw;
		}
		vec3 ibl_specular = (F0 * env_brdf.x + env_brdf.y) * reflection_radiance;

		vec3 ambient = ibl_diffuse + ibl_specular;

		ao_factor *= occlusion;
		vec3 lit = (diffuse + specular) * ao_factor + ambient * ao_factor + emission;

		vec4 output_color = vec4(lit, 1.0);
		if (push_constant.blend != 0u) {
			float source_alpha = clamp(albedo.a, 0.0, 1.0);
			vec4 destination_color = imageLoad(lit_map, pixel_coordinates);
			output_color = source_over(vec4(lit * source_alpha, source_alpha), destination_color);
		}

		imageStore(lit_map, pixel_coordinates, output_color)
		"
		.trim();

		let b_hlsl = "
		float3 diffuse = float3(0.0, 0.0, 0.0);
		float3 specular = float3(0.0, 0.0, 0.0);

		// GTAO belongs to the opaque depth surface. Reusing it for a transparent
		// surface would composite the opaque surface's occlusion over that surface.
		float ao_factor = push_constant.blend != 0u
			? 1.0
			: ao.Load(int3(pixel_coordinates, 0)).r;

		// Combine the basis explicitly because HLSL matrix constructors treat T, B, and N as rows.
		normal = normalize(normal.x * T + normal.y * B + normal.z * N);
		float3 F0 = lerp(float3(0.04, 0.04, 0.04), albedo.xyz, metalness);
		float NdotV = max(dot(normal, V), 0.0);
		float roughness_alpha = roughness * roughness;
		float roughness_alpha_squared = roughness_alpha * roughness_alpha;
		float adjusted_roughness = roughness + 1.0;
		float geometry_k = adjusted_roughness * adjusted_roughness / 8.0;
		float view_fresnel_factor = pow(clamp(1.0 - NdotV, 0.0, 1.0), 5.0);
		float3 one_minus_fresnel_n_dot_v = float3(1.0, 1.0, 1.0) - fresnel_schlick_from_factor(view_fresnel_factor, F0);

		for (uint i = 0; i < lighting_data[0].light_count; ++i) {
			Light light = lighting_data[0].lights[i];

			float3 L;
			float attenuation = 1.0;

			if (light.type == 68) {
				L = normalize(-light.position.xyz);
			} else {
				float3 surface_to_light = light.position.xyz - world_space_vertex_position;
				float distance_squared = dot(surface_to_light, surface_to_light);
				if (distance_squared <= 0.0) { continue; }
				L = surface_to_light * rsqrt(distance_squared);
				attenuation = 1.0 / distance_squared;
			}

			float NdotL = max(dot(normal, L), 0.0);

			if (NdotL <= 0.0) { continue; }

			float occlusion_factor = 1.0;

			if (light.type == 68) {
				float4 view_space_surface_position = mul(view.view, float4(world_space_vertex_position, 1.0));
				float c_occlusion_factor = sample_shadow(depth_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L);

				occlusion_factor = c_occlusion_factor;

				if (occlusion_factor == 0.0) { continue; }

				attenuation = 1.0;
			} else {
				if (light.type == 1) {
					// Preserve full intensity inside the inner cone and fade to zero at the outer cone.
					float cone_cosine = dot(normalize(light.direction.xyz), -L);
					float cone_factor = cone_attenuation(cone_cosine, light.cone_cosines.x, light.cone_cosines.y);
					if (cone_factor <= 0.0) { continue; }
					attenuation *= cone_factor;
					float4 view_space_surface_position = mul(view.view, float4(world_space_vertex_position, 1.0));
					occlusion_factor = sample_shadow(cone_shadow_map, light, world_space_vertex_position, view_space_surface_position.xyz, world_space_vertex_normal, L);
					if (occlusion_factor == 0.0) { continue; }
				}
			}

			float3 H = normalize(V + L);

			float3 radiance = light.color.xyz * attenuation;

			float half_view_fresnel_factor = pow(clamp(1.0 - max(dot(H, V), 0.0), 0.0, 1.0), 5.0);
			float3 F = fresnel_schlick_from_factor(half_view_fresnel_factor, F0);
			float NDF = distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared);
			float G = geometry_smith_from_terms(NdotV, NdotL, geometry_k);
			float3 local_specular = (NDF * G * F) / (4.0 * NdotV * NdotL + 0.000001);

			float light_fresnel_factor = pow(clamp(1.0 - NdotL, 0.0, 1.0), 5.0);
			float3 kD = (float3(1.0, 1.0, 1.0) - fresnel_schlick_from_factor(light_fresnel_factor, F0)) * one_minus_fresnel_n_dot_v;

			kD *= 1.0 - metalness;

			float3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		float3 ambient_irradiance = sample_environment_irradiance(normal);
		float3 reflection_direction = reflect(-V, normal);
		float3 reflection_radiance = sample_environment_specular(reflection_direction, roughness);

		float3 F_ibl = fresnel_schlick_roughness(NdotV, F0, roughness);
		float3 kD_ibl = (float3(1.0, 1.0, 1.0) - F_ibl) * (1.0 - metalness);

		float3 ibl_diffuse = kD_ibl * albedo.xyz * ambient_irradiance;

		float2 env_brdf = float2(1.0, 0.0);
		{
			float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
			float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
			float4 r = roughness * c0 + c1;
			float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
			env_brdf = float2(-1.04, 1.04) * a004 + r.zw;
		}
		float3 ibl_specular = (F0 * env_brdf.x + env_brdf.y) * reflection_radiance;

		float3 ambient = ibl_diffuse + ibl_specular;

		ao_factor *= occlusion;
		float3 lit = (diffuse + specular) * ao_factor + ambient * ao_factor + emission;

		float4 output_color = float4(lit, 1.0);
		if (push_constant.blend != 0u) {
			float source_alpha = clamp(albedo.a, 0.0, 1.0);
			float4 destination_color = lit_map[pixel_coordinates];
			output_color = source_over(float4(lit * source_alpha, source_alpha), destination_color);
		}

		lit_map[pixel_coordinates] = output_color
		"
		.trim();

		let m = root.get_mut("main").unwrap();

		if let besl::parser::Nodes::Function { statements, .. } = m.node_mut() {
			statements.insert(
				0,
				besl::parser::Node::raw_code(
					Some(a.into()),
					Some(a_hlsl.into()),
					Some(a_msl.into()),
					&[
						"vertex_uvs",
						"ao",
						"depth_shadow_map",
						"push_constant",
						"material_offset",
						"material_offset_scratch",
						"material_evaluation_dispatches",
						"pixel_mapping",
						"meshes",
						"meshlets",
						"materials",
						"primitive_indices",
						"vertex_indices",
						"vertex_positions",
						"vertex_normals",
						"skinned_vertices",
						"triangle_index",
						"instance_index_render_target",
						"views",
						"make_raster_ndc_from_pixel_coordinates",
						"calculate_full_bary",
						"calculate_barycentric_from_position",
						"interpolate_vec3f_with_deriv",
						"interpolate_vec2f_with_deriv",
						"compute_vertex_index",
					],
					&[
						"material",
						"albedo",
						"normal",
						"roughness",
						"metalness",
						"occlusion",
						"emission",
					],
				),
			);
			statements.push(besl::parser::Node::raw_code(
				Some(b.into()),
				Some(b_hlsl.into()),
				Some(b_msl.into()),
				&[
					"lighting_data",
					"lit_map",
					"cone_shadow_map",
					"push_constant",
					"source_over",
					"sample_shadow",
					"sample_environment_irradiance",
					"sample_environment_specular",
					"distribution_ggx_from_terms",
					"geometry_smith_from_terms",
					"fresnel_schlick_from_factor",
					"fresnel_schlick_roughness",
					"cone_attenuation",
				],
				&[],
			));
		}

		root.add(extra);
		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root
	}
}

#[cfg(test)]
mod tests {
	use resource_management::asset::{bema_asset_handler::ProgramGenerator, JsonObject};
	use resource_management::pbr::{
		generate_textured_brdf_program, BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfNode, BrdfTexture,
		BrdfValue,
	};
	use resource_management::shader::besl::backends::{
		glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator,
	};
	use resource_management::shader::besl::evaluation::ProgramEvaluation;
	use resource_management::shader::generator::ShaderGenerationSettings;
	use utils::json::{self, JsonContainerTrait, JsonValueTrait};

	use crate::besl;

	macro_rules! material_metadata {
		($($json:tt)*) => {
			serde_json::json!({ $($json)* })
				.as_object()
				.expect("test material metadata should be an object")
				.clone()
		};
	}

	#[test]
	fn write_to_albedo() {
		let material = material_metadata! {
			"variables": []
		};

		let shader_source = "main: fn () -> void { albedo = vec4f(1, 2, 3, 4); }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		let _node = besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn vec4f_variable() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "albedo",
					"data_type": "vec4f",
					"value": "Purple"
				}
			]
		};

		let shader_source = "main: fn () -> void { out_color = albedo; }";

		let shader_node = besl::parse(shader_source).expect("expected test value");

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		println!("{:#?}", shader);
	}

	/// Verifies material texture variables produce valid BESL.
	#[test]
	fn texture_variable_transform_produces_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		besl::lex(shader).expect("expected test value");
	}

	#[test]
	fn material_evaluation_texture_variables_produce_valid_besl() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "base_color",
					"data_type": "Texture2D"
				},
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_source = "main: fn () -> void { albedo = sample_material(base_color); normal = sample_normal(normal_map); }";
		let shader_node = besl::parse(shader_source).expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies HLSL transforms tangent-space normals with the same basis convention as GLSL and MSL.
	#[test]
	fn material_evaluation_hlsl_combines_tangent_basis_vectors() {
		let material = material_metadata! {
			"variables": [
				{
					"name": "normal_map",
					"data_type": "Texture2D"
				}
			]
		};
		let shader_node =
			besl::parse("main: fn () -> void { normal = sample_normal(normal_map); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let source = HLSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect(
				"Failed to emit the HLSL material pass. The most likely cause is an invalid tangent-basis shader contract.",
			);

		assert!(
			source.contains("normal = normalize(normal.x * T + normal.y * B + normal.z * N);"),
			"HLSL did not combine the tangent basis explicitly. The most likely cause is that the material pass reintroduced a row-versus-column matrix assumption."
		);
		assert!(
			!source.contains("mul(TBN, normal)"),
			"HLSL multiplied a row-constructed tangent basis as a column basis. The most likely cause is that the material pass reintroduced the faceted-normal transform."
		);
	}

	/// Verifies material evaluation keeps per-pixel and per-light terms out of the repeated PCF tap path.
	#[test]
	fn material_evaluation_hoists_shared_terms_and_uses_direct_ao_reads() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("test material should parse");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material))
			.expect("material evaluation should produce valid BESL");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the GLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the HLSL material pass. The most likely cause is an invalid visibility shader contract.");
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect("Failed to emit the MSL material pass. The most likely cause is an invalid visibility shader contract.");

		assert!(glsl.contains("texelFetch(ao, pixel_coordinates, 0).r"));
		assert!(hlsl.contains("ao.Load(int3(pixel_coordinates, 0)).r"));
		assert!(msl.contains("resources.ao.read(uint2(pixel_coordinates)).r"));
		assert!(msl.contains("float3 world_space_vertex_position0"));
		assert!(!msl.contains("world_space_vertex_positions[3]"));
		assert!(!msl.contains("primitive_indices[3]"));
		assert!(msl.contains("geometry_smith_from_terms(NdotV, NdotL, geometry_k)"));
		assert!(msl.contains("distribution_ggx_from_terms(max(dot(normal, H), 0.0), roughness_alpha_squared)"));
		assert!(msl.contains("View shadow_view = resources.views->views[shadow_view_index];"));
		assert!(msl.contains(
			"float sample_shadow_tap(texture2d_array<float> shadow_map, float2 shadow_uv, float surface_depth, float2 offset, uint shadow_layer, int2 shadow_map_extent)"
		));
		assert!(msl.contains("float2 offset_shadow_uv = shadow_uv + offset;"));
	}

	/// Verifies material evaluation with skinned geometry produces valid BESL.
	#[test]
	fn material_evaluation_with_skinning_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		besl::lex(shader).expect("expected test value");
	}

	/// Verifies material evaluation samples the bound environment without a procedural fallback.
	#[test]
	fn material_evaluation_with_environment_ibl_produces_valid_besl() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect("expected material evaluation main");
		let source = MSLShaderGenerator::new()
			.generate(&ShaderGenerationSettings::compute(utils::Extent::square(8)), &main)
			.expect("expected valid Metal material evaluation source");
		assert!(!source.contains("sample_analytical_reflection"));
		assert!(!source.contains("environment_sample.a"));
		assert!(!source.contains("lower_sample.a"));
	}

	#[test]
	fn material_evaluation_emits_cone_attenuation_for_every_backend() {
		let material = material_metadata! {
			"variables": []
		};
		let shader_node =
			besl::parse("main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }").expect("expected test value");
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = besl::lex(shader_generator.transform(shader_node, &material)).expect("expected test value");
		let main = shader.get_main().expect(
			"Missing material evaluation main. The most likely cause is that visibility material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));

		let glsl = GLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the GLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let hlsl = HLSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the HLSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);
		let msl = MSLShaderGenerator::new().generate(&settings, &main).expect(
			"Failed to emit the MSL cone-light material pass. The most likely cause is an invalid visibility shader contract.",
		);

		for source in [&glsl, &hlsl, &msl] {
			assert!(source.contains("cone_cosines"));
			assert!(source.contains("cone_attenuation"));
			assert!(source.contains("light.type == 1"));
			assert!(source.contains("_light_count_padding"));
			assert!(source.contains("shadow_layer"));
			assert!(source.contains("cone_shadow_map"));
		}
		assert!(glsl.contains("vec4 position"));
		assert!(glsl.contains("uint32_t type"));
		assert!(hlsl.contains("float4 position"));
		assert!(hlsl.contains("uint32_t type"));
		assert!(msl.contains("float4 position"));
		assert!(msl.contains("uint type"));

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-cone-light-material",
		)
		.expect("Failed to compile the MSL cone-light material pass. The most likely cause is invalid generated Metal source.");
	}

	/// Compiles a production-generated trivial material evaluation pass and guards its required semantic resource access.
	#[test]
	fn trivial_generated_material_evaluation_pass_links_and_reflects_required_bindings() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.constant(BrdfValue::Vector4([0.8, 0.6, 0.4, 1.0]));
		let metallic = builder.constant(BrdfValue::Scalar(0.25));
		let roughness = builder.constant(BrdfValue::Scalar(0.5));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let material_program = generate_textured_brdf_program(&material).expect(
			"Failed to generate the trivial material program. The most likely cause is an invalid BRDF material graph.",
		);
		let material_metadata = material_metadata! {
			"variables": []
		};

		// Material evaluation reads the exact dispatch count, offset, and mapping state while retaining the lit target for transparent blending.
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(material_program, &material_metadata);
		let program = besl::lex(shader).expect(
			"Failed to link the trivial material evaluation pass. The most likely cause is a drifted visibility shader contract.",
		);
		let main = program.get_main().expect(
			"Missing trivial material evaluation main. The most likely cause is that material generation stopped producing an entry point.",
		);
		let evaluation = ProgramEvaluation::from_main(&main).expect(
			"Failed to reflect the trivial material evaluation pass. The most likely cause is an invalid visibility resource graph.",
		);

		for slot in [1034, 1036, 1037] {
			let binding = evaluation.bindings().iter().find(|binding| binding.slot == slot).unwrap_or_else(|| {
				panic!(
					"Missing required material evaluation binding at slot {slot}. The most likely cause is that generated material reachability drifted."
				)
			});
			assert!(
				binding.read,
				"Material evaluation binding at slot {slot} is not readable. The most likely cause is incorrect visibility scope access metadata."
			);
		}

		// These strides are the public CPU/GPU storage contract retained by baked shader artifacts.
		for (slot, expected_stride) in [
			(0, 400),
			(1, crate::rendering::pipelines::visibility::MESH_DATA_BUFFER_STRIDE),
			(2, 12),
			(3, 12),
			(4, 32),
			(5, 8),
			(6, crate::rendering::pipelines::visibility::VERTEX_INDEX_BUFFER_STRIDE),
			(7, crate::rendering::pipelines::visibility::PRIMITIVE_INDEX_BUFFER_STRIDE),
			(8, 64),
			(1034, 4),
			(1035, 4),
			(1036, 16),
			(1037, 4),
			(1045, 1552),
			(1046, 64),
		] {
			let binding = evaluation
				.bindings()
				.iter()
				.find(|binding| binding.slot == slot)
				.unwrap_or_else(|| {
					panic!(
					"Missing material evaluation binding at slot {slot}. The most likely cause is that visibility resource retention drifted."
				)
				});
			assert_eq!(
				binding.buffer_stride,
				Some(expected_stride),
				"Unexpected storage-buffer stride at slot {slot}. The most likely cause is that the BESL storage layout diverged from its CPU record."
			);
		}

		let lit_binding = evaluation.bindings().iter().find(|binding| binding.slot == 1041).expect(
			"Missing material evaluation lit binding. The most likely cause is that generated shading stopped retaining its output target.",
		);
		assert!(
			lit_binding.read && lit_binding.write,
			"Material evaluation lit binding is not read-write. The most likely cause is that transparent source-over access drifted."
		);
	}

	/// Verifies native material evaluation emits one bindless sample for a texture shared by several BRDF roles.
	#[test]
	fn generated_material_evaluation_reuses_shared_texture_sample() {
		let mut builder = BrdfMaterialBuilder::new();
		let texture = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Blue);
		let roughness = builder.extract_channel(texture, resource_management::pbr::BrdfChannel::Green);
		let normal = builder.add(BrdfNode::NormalMap {
			source: texture,
			scale: 0.5,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: texture,
			strength: 0.75,
		});
		let emission = builder.add(BrdfNode::Emission { color: texture });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color: texture,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let material_program = generate_textured_brdf_program(&material).expect(
			"Failed to generate the shared-texture material program. The most likely cause is an invalid BRDF material graph.",
		);
		let material_metadata = material_metadata! {
			"variables": [{
				"name": "gltf_texture_3",
				"data_type": "Texture2D"
			}]
		};

		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(material_program, &material_metadata);
		let program = besl::lex(shader).expect(
			"Failed to link the shared-texture material evaluation pass. The most likely cause is a drifted visibility shader contract.",
		);
		let main = program.get_main().expect(
			"Missing shared-texture material entry point. The most likely cause is that material generation stopped producing an entry point.",
		);
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		let glsl = GLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture GLSL material pass. The most likely cause is an invalid visibility shader contract.",
			);
		let hlsl = HLSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture HLSL material pass. The most likely cause is an invalid visibility shader contract.",
			);
		let msl = MSLShaderGenerator::new()
			.generate(&settings, &main)
			.expect(
				"Failed to emit the shared-texture MSL material pass. The most likely cause is an invalid visibility shader contract.",
			);

		assert_eq!(
			glsl.match_indices("texture(textures[nonuniformEXT(material.textures[").count(),
			1,
			"The generated GLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			hlsl.match_indices("textures[material.textures[").count(),
			1,
			"The generated HLSL material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert_eq!(
			msl.match_indices("resources.textures[material.textures[").count(),
			1,
			"The generated material sampled the shared texture more than once. The most likely cause is that BRDF texture-sample reuse was bypassed."
		);
		assert!(
			msl.contains("float4 material_texture_sample_0"),
			"The generated material did not retain its reusable texel local. The most likely cause is that texture-sample lowering stopped emitting the cache binding."
		);
		assert_eq!(
			msl.match_indices("decode_material_normal(material_texture_sample_0)").count(),
			1,
			"The scaled normal map decoded the shared texel more than once. The most likely cause is that normal scaling bypassed the reusable helper."
		);

		#[cfg(target_os = "macos")]
		resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(
			&msl,
			"visibility-shared-material-texture-sample",
		)
		.expect(
			"Failed to compile the shared-texture MSL material pass. The most likely cause is invalid generated Metal source.",
		);
	}

	/// Ensures every reflected resource has a retained write in the material-evaluation pass.
	#[test]
	fn material_evaluation_flat_interface_matches_retained_resource_slots() {
		let retained_ranges = [
			(0, 1),
			(1, 1),
			(2, 1),
			(3, 1),
			(4, 1),
			(5, 1),
			(6, 1),
			(7, 1),
			(8, 1),
			(9, 1024),
			(1033, 1),
			(1034, 1),
			(1035, 1),
			(1036, 1),
			(1037, 1),
			(1039, 1),
			(1040, 1),
			(1041, 1),
			(1045, 1),
			(1046, 1),
			(1051, 1),
			(1052, 1),
			(1053, 1),
			(1054, 1),
			(1055, 8),
			(1064, 1),
		];
		let cases = [
			(
				material_metadata! {
					"variables": []
				},
				"main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }",
			),
			(
				material_metadata! {
					"variables": [{
						"name": "base_color",
						"data_type": "Texture2D"
					}]
				},
				"main: fn () -> void { albedo = sample_material(base_color); }",
			),
		];

		for (material, shader_source) in cases {
			let shader_node = besl::parse(shader_source).expect("expected test value");
			let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
			let shader = shader_generator.transform(shader_node, &material);
			let root = besl::lex(shader).expect("expected test value");
			let main_node = root.get_main().expect("expected test value");
			let evaluation =
				ProgramEvaluation::from_main(&main_node).expect("Expected material evaluation reflection to succeed");
			let lit_binding = evaluation.bindings().iter().find(|binding| binding.slot == 1041).expect(
				"Missing material lit binding. The most likely cause is that material output stopped retaining slot 1041.",
			);
			assert!(
				lit_binding.read && lit_binding.write,
				"Material lit binding is not read-write. The most likely cause is that transparent source-over access was removed from the visibility scope."
			);
			assert!(
				evaluation.bindings().iter().all(|binding| binding.slot != 1053),
				"Material evaluation still depends on opaque visibility depth. The most likely cause is that surface reconstruction stopped using the winning triangle's barycentrics."
			);
			let unexpected_ranges = evaluation
				.bindings()
				.iter()
				.map(|binding| (binding.slot, binding.count))
				.filter(|binding| !retained_ranges.contains(binding))
				.collect::<Vec<_>>();

			assert!(
				unexpected_ranges.is_empty(),
				"Material evaluation reflected resources that none of its retained descriptor sets writes: {unexpected_ranges:?}"
			);
		}
	}
}
