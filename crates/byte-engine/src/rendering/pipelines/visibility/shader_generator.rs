use std::{cell::RefCell, ops::Deref, rc::Rc};

use besl::{parser::Node, NodeReference};
use resource_management::asset::bema_asset_handler::ProgramGenerator;
use std::sync::Arc;
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use crate::rendering::common_shader_generator::CommonShaderScope;
use crate::rendering::pipelines::visibility::MAX_PIXEL_MAPPING_ENTRIES;

pub struct VisibilityShaderScope {}

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
				Node::member("model", "mat4f"),
				Node::member("material_index", "u32"),
				Node::member("base_vertex_index", "u32"),
				Node::member("base_primitive_index", "u32"),
				Node::member("base_triangle_index", "u32"),
				Node::member("base_meshlet_index", "u32"),
			],
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
				Node::member("primitive_offset", "u16"),
				Node::member("triangle_offset", "u16"),
				Node::member("primitive_count", "u8"),
				Node::member("triangle_count", "u8"),
			],
		);
		let light_struct = Node::r#struct(
			"Light",
			vec![
				Node::member("position", "vec3f"),
				Node::member("color", "vec3f"),
				Node::member("type", "u8"),
				Node::member("cascades", "u32[8]"),
			],
		);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);

		let views_binding = Node::binding(
			"views",
			Node::buffer("ViewsBuffer", vec![Node::member("views", "View[8]")]),
			0,
			0,
			true,
			false,
		);
		let meshes = Node::binding(
			"meshes",
			Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[1024]")]),
			0,
			1,
			true,
			false,
		);
		let positions = Node::binding(
			"vertex_positions",
			Node::buffer("Positions", vec![Node::member("positions", "vec3f[8192]")]),
			0,
			2,
			true,
			false,
		);
		let normals = Node::binding(
			"vertex_normals",
			Node::buffer("Normals", vec![Node::member("normals", "vec3f[8192]")]),
			0,
			3,
			true,
			false,
		);
		let uvs = Node::binding(
			"vertex_uvs",
			Node::buffer("UVs", vec![Node::member("uvs", "vec2f[8192]")]),
			0,
			5,
			true,
			false,
		);
		let vertex_indices = Node::binding(
			"vertex_indices",
			Node::buffer("VertexIndices", vec![Node::member("vertex_indices", "u16[8192]")]),
			0,
			6,
			true,
			false,
		);
		let primitive_indices = Node::binding(
			"primitive_indices",
			Node::buffer("PrimitiveIndices", vec![Node::member("primitive_indices", "u8[8192]")]),
			0,
			7,
			true,
			false,
		);
		let meshlets = Node::binding(
			"meshlets",
			Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", "Meshlet[8192]")]),
			0,
			8,
			true,
			false,
		);
		let textures = Node::binding_array("textures", Node::combined_image_sampler(), 0, 9, true, false, 16);

		let material_count = Node::binding(
			"material_count",
			Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[1024]")]),
			1,
			0,
			material_count_read,
			material_count_write,
		); // TODO: somehow set read/write properties per shader
		let material_offset = Node::binding(
			"material_offset",
			Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[1024]")]),
			1,
			1,
			material_offset_read,
			material_offset_write,
		);
		let material_offset_scratch = Node::binding(
			"material_offset_scratch",
			Node::buffer(
				"MaterialOffsetScratch",
				vec![Node::member("material_offset_scratch", "u32[1024]")],
			),
			1,
			2,
			material_offset_scratch_read,
			material_offset_scratch_write,
		);
		let material_evaluation_dispatches = Node::binding(
			"material_evaluation_dispatches",
			Node::buffer(
				"MaterialEvaluationDispatches",
				vec![Node::member("material_evaluation_dispatches", "vec4u[1024]")],
			),
			1,
			3,
			material_offset_read,
			material_offset_write,
		);
		let pixel_mapping = Node::binding(
			"pixel_mapping",
			Node::buffer(
				"PixelMapping",
				vec![Node::member(
					"pixel_mapping",
					&format!("vec2u16[{MAX_PIXEL_MAPPING_ENTRIES}]"),
				)],
			),
			1,
			4,
			pixel_mapping_read,
			pixel_mapping_write,
		);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1, 6, true, false);
		let instance_index = Node::binding("instance_index_render_target", Node::image("r32ui"), 1, 7, true, false);

		let compute_vertex_index = Node::function(
			"compute_vertex_index",
			vec![
				Node::parameter("mesh", "Mesh"),
				Node::parameter("meshlet", "Meshlet"),
				Node::parameter("primitive_index", "u32"),
			],
			"u32",
			vec![Node::raw_code(
				Some(
					"return mesh.base_vertex_index + vertex_indices.vertex_indices[mesh.base_primitive_index + meshlet.primitive_offset + primitive_index]; /* Indices in the buffer are relative to each mesh/primitives */"
						.into(),
				),
				None,
				Some(
					"return mesh.base_vertex_index + set0.vertex_indices->vertex_indices[mesh.base_primitive_index + uint(meshlet.primitive_offset) + primitive_index]; /* Indices in the buffer are relative to each mesh/primitives */"
						.into(),
				),
				&["vertex_indices"],
				&[],
			)],
		);
		let compute_vertex_position = {
			let mut root = besl::parse(
				r#"
				compute_vertex_position: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec4f {
					let vertex_index: u32 = compute_vertex_index(mesh, meshlet, primitive_index);
					return vec4f(
						vertex_positions.positions[vertex_index].x,
						vertex_positions.positions[vertex_index].y,
						vertex_positions.positions[vertex_index].z,
						1.0
					);
				}
				"#,
			)
			.expect("Expected compute_vertex_position source to parse");

			match root.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => children.remove(0),
				_ => panic!(
					"Expected compute_vertex_position source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};
		let compute_triangle = {
			let mut root = besl::parse(
				r#"
				compute_triangle: fn (mesh: Mesh, meshlet: Meshlet, primitive_index: u32) -> vec3u {
					let triangle_base_index: u32 = mesh.base_triangle_index + u16_to_u32(meshlet.triangle_offset) + primitive_index;
					return vec3u(
						primitive_indices.primitive_indices[triangle_base_index * 3 + 0],
						primitive_indices.primitive_indices[triangle_base_index * 3 + 1],
						primitive_indices.primitive_indices[triangle_base_index * 3 + 2]
					);
				}
				"#,
			)
			.expect("Expected compute_triangle source to parse");

			match root.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => children.remove(0),
				_ => panic!(
					"Expected compute_triangle source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};

		let mesh_outputs = Node::raw_code(
			Some("".into()),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			Some(
				r#"
struct VertexOutput {
	float4 position [[position]];
};

struct PrimitiveOutput {
	uint instance_index [[flat]] [[user(locn0)]];
	uint primitive_index [[flat]] [[user(locn1)]];
};
"#
				.into(),
			),
			&[],
			&["VertexOutput", "PrimitiveOutput"],
		);
		let out_instance_index = Node::output_array("out_instance_index", "u32", 0, 126);
		let out_primitive_index = Node::output_array("out_primitive_index", "u32", 1, 126);
		let u8_to_u32 = Node::function(
			"u8_to_u32",
			vec![Node::parameter("value", "u8")],
			"u32",
			vec![Node::raw_code(
				Some("return uint(value);".into()),
				Some("return uint(value);".into()),
				Some("return uint(value);".into()),
				&[],
				&[],
			)],
		);
		let u16_to_u32 = Node::function(
			"u16_to_u32",
			vec![Node::parameter("value", "u16")],
			"u32",
			vec![Node::raw_code(
				Some("return uint(value);".into()),
				Some("return uint(value);".into()),
				Some("return uint(value);".into()),
				&[],
				&[],
			)],
		);

		let process_meshlet = {
			let mut process_meshlet = besl::parse(
				r#"
				process_meshlet: fn (instance_index: u32, matrix: mat4f) -> void {
					let mesh: Mesh = meshes.meshes[instance_index];
					let meshlet_index: u32 = threadgroup_position() + mesh.base_meshlet_index;
					let meshlet: Meshlet = meshlets.meshlets[meshlet_index];
					let primitive_index: u32 = thread_idx();

					set_mesh_output_counts(u8_to_u32(meshlet.primitive_count), u8_to_u32(meshlet.triangle_count));

					if (primitive_index < u8_to_u32(meshlet.primitive_count)) {
						set_mesh_vertex_position(
							primitive_index,
							compute_vertex_position(mesh, meshlet, primitive_index) * mesh.model * matrix
						);
					}

					if (primitive_index < u8_to_u32(meshlet.triangle_count)) {
						set_mesh_triangle(primitive_index, compute_triangle(mesh, meshlet, primitive_index));
						out_instance_index[primitive_index] = instance_index;
						out_primitive_index[primitive_index] = meshlet_index << 8 | primitive_index & 255;
					}
				}
				"#,
			)
			.expect("Expected process_meshlet source to parse");

			match process_meshlet.node_mut() {
				besl::parser::Nodes::Scope { children, .. } => {
					let mut function = children.remove(0);

					if let besl::parser::Nodes::Function { statements, .. } = function.node_mut() {
						statements.insert(
							0,
							Node::raw_code(
								Some("".into()),
								Some("".into()),
								Some("".into()),
								&["VertexOutput", "PrimitiveOutput"],
								&[],
							),
						);
					}

					function
				}
				_ => panic!(
					"Expected process_meshlet source to parse into a scope. The most likely cause is invalid BESL syntax in the visibility shader module."
				),
			}
		};

		let set2_binding0 = Node::binding("diffuse_map", Node::image("rgba16"), 2, 0, false, true);
		let set2_binding1 = Node::binding(
			"_unused_set2_binding1",
			Node::buffer("UnusedSet2Binding1", vec![Node::member("padding", "u32")]),
			2,
			1,
			true,
			false,
		);
		let set2_binding2 = Node::binding("specular_map", Node::image("rgba16"), 2, 2, false, true);
		let set2_binding3 = Node::binding("_unused_set2_binding3", Node::image("rgba16"), 2, 3, false, true);
		let set2_binding4 = Node::binding(
			"lighting_data",
			Node::buffer(
				"LightingBuffer",
				vec![Node::member("light_count", "u32"), Node::member("lights", "Light[16]")],
			),
			2,
			4,
			true,
			false,
		);
		let set2_binding5 = Node::binding(
			"materials",
			Node::buffer("MaterialBuffer", vec![Node::member("materials", "Material[16]")]),
			2,
			5,
			true,
			false,
		);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 2, 10, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_array_image_sampler(), 2, 11, true, false);
		let set2_binding12 = Node::binding("visibility_depth", Node::combined_image_sampler(), 2, 12, true, false);
		let set2_binding13 = Node::binding("ibl_cubemap", Node::combined_array_image_sampler(), 2, 13, true, false);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32")]);

		let sample_function = Node::intrinsic(
			"sample",
			Node::parameter("smplr", "u32"),
			Node::sentence(vec![Node::raw_code(
				Some("return texture(textures[nonuniformEXT(material.textures[smplr])], vertex_uv)".into()),
				None,
				Some(
					"return set0.textures[material.textures[smplr]].sample(set0.textures_sampler[material.textures[smplr]], vertex_uv)"
						.into(),
				),
				&["textures"],
				&[],
			)]),
			"vec4f",
		);

		let sample_normal_function = if true {
			Node::intrinsic(
				"sample_normal",
				Node::parameter("smplr", "u32"),
				Node::sentence(vec![Node::raw_code(
					Some(
						"return unit_vector_from_xy(texture(textures[nonuniformEXT(material.textures[smplr])], vertex_uv).xy)"
							.into(),
					),
					None,
					Some(
						"return unit_vector_from_xy(set0.textures[material.textures[smplr]].sample(set0.textures_sampler[material.textures[smplr]], vertex_uv).xy)"
							.into(),
					),
					&["textures", "unit_vector_from_xy"],
					&[],
				)]),
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
				Node::parameter("light", "Light"),
				Node::parameter("world_space_position", "vec3f"),
				Node::parameter("view_space_position", "vec3f"),
				Node::parameter("surface_normal", "vec3f"),
				Node::parameter("offset", "vec2f"),
			],
			"f32",
			vec![Node::raw_code(
				Some(
					"
			float depth_value = abs(view_space_position.z);

			if (light.cascades[0] == 0u) { return 1.0; }

			uint cascade_index = 3;

			for (uint i = 0; i < 4; ++i) {
				if (depth_value < views.views[light.cascades[i]].far) {
					cascade_index = i;
					break;
				}
			}

			View view = views.views[light.cascades[cascade_index]];

			vec4 surface_light_clip_position = view.view_projection * vec4(world_space_position, 1.0);
			vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;

			vec2 shadow_uv = vec2(
				surface_light_ndc_position.x * 0.5f + 0.5f,
				0.5f - surface_light_ndc_position.y * 0.5f
			) + offset;

			float normal_alignment = max(dot(normalize(surface_normal), normalize(-light.position)), 0.0);
			// Slope-scaled depth bias tuning per cascade.
			float cascade_bias_scale = float(cascade_index + 1u);
			float cascade_depth_range = max(view.far - view.near, 0.0001f);
			float slope_scaled_bias = 0.0002f * cascade_bias_scale * (1.0f - normal_alignment);
			float constant_bias = 0.00002f * cascade_bias_scale;
			float cascade_range_bias = cascade_depth_range * 0.0000025f;
			float surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
			float surface_depth = surface_light_ndc_position.z + surface_depth_bias;

			if (shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0) { return 1.0; }
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			ivec2 shadow_map_extent = textureSize(shadow_map, 0).xy;
			ivec2 shadow_texel = ivec2(clamp(shadow_uv * vec2(shadow_map_extent), vec2(0.0), vec2(shadow_map_extent - 1)));
			float closest_depth = texelFetch(shadow_map, ivec3(shadow_texel, int(cascade_index)), 0).r;

			return surface_depth < closest_depth ? 0.0 : 1.0"
						.into(),
				),
				None,
				Some(
					"
			float depth_value = abs(view_space_position.z);

			if (light.cascades[0] == 0u) { return 1.0; }

			uint cascade_index = 3;

			for (uint i = 0; i < 4; ++i) {
				if (depth_value < set0.views->views[light.cascades[i]].far) {
					cascade_index = i;
					break;
				}
			}

			View view = set0.views->views[light.cascades[cascade_index]];

			float4 surface_light_clip_position = float4(world_space_position, 1.0) * view.view_projection;
			float3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;

			float2 shadow_uv = float2(
				surface_light_ndc_position.x * 0.5f + 0.5f,
				0.5f - surface_light_ndc_position.y * 0.5f
			) + offset;

			float normal_alignment = max(dot(normalize(surface_normal), normalize(-light.position)), 0.0);
			float cascade_bias_scale = float(cascade_index + 1u);
			float cascade_depth_range = max(view.far - view.near, 0.0001f);
			float slope_scaled_bias = 0.0002f * cascade_bias_scale * (1.0f - normal_alignment);
			float constant_bias = 0.00002f * cascade_bias_scale;
			float cascade_range_bias = cascade_depth_range * 0.0000025f;
			float surface_depth_bias = max(slope_scaled_bias + cascade_range_bias, constant_bias);
			float surface_depth = surface_light_ndc_position.z + surface_depth_bias;

			if (shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0) { return 1.0; }
			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			int2 shadow_map_extent = int2(shadow_map.get_width(), shadow_map.get_height());
			int2 shadow_texel = int2(clamp(shadow_uv * float2(shadow_map_extent), float2(0.0), float2(shadow_map_extent - 1)));
			float closest_depth = shadow_map.read(uint2(shadow_texel), cascade_index).x;

			return surface_depth < closest_depth ? 0.0 : 1.0"
						.into(),
				),
				&["views"],
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
			],
			"f32",
			vec![Node::raw_code(
				Some("ivec2 shadow_map_extent = textureSize(shadow_map, 0).xy;
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
					light,
					world_space_position,
					view_space_position,
					surface_normal,
					pcf_offset
				);
			}

			return occlusion / 8.0f;".into()),
				None,
				Some(
					"int2 shadow_map_extent = int2(shadow_map.get_width(), shadow_map.get_height());
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
					light,
					world_space_position,
					view_space_position,
					surface_normal,
					pcf_offset,
					gid,
					push_constant,
					set0,
					set1,
					set2
				);
			}

			return occlusion / 8.0f;"
						.into(),
				),
				&["sample_shadow_tap"],
				&[],
			)],
		);

		let sample_ibl_cubemap = Node::function(
			"sample_ibl_cubemap",
			vec![
				Node::parameter("cubemap", "ArrayTexture2D"),
				Node::parameter("direction", "vec3f"),
			],
			"vec3f",
			vec![Node::raw_code(
				Some(
					"
			float direction_length = length(direction);
			if (direction_length <= 0.0) { return vec3(1.0); }

			vec3 dir = direction / direction_length;
			vec3 abs_dir = abs(dir);

			if (max(max(abs_dir.x, abs_dir.y), abs_dir.z) <= 0.0) { return vec3(1.0); }

			vec2 uv = vec2(0.5);
			float face = 0.0;

			if (abs_dir.x >= abs_dir.y && abs_dir.x >= abs_dir.z) {
				float inv_axis = 0.5 / abs_dir.x;
				if (dir.x > 0.0) {
					uv = vec2(-dir.z, -dir.y) * inv_axis + 0.5;
					face = 0.0;
				} else {
					uv = vec2(dir.z, -dir.y) * inv_axis + 0.5;
					face = 1.0;
				}
			} else if (abs_dir.y >= abs_dir.z) {
				float inv_axis = 0.5 / abs_dir.y;
				if (dir.y > 0.0) {
					uv = vec2(dir.x, dir.z) * inv_axis + 0.5;
					face = 2.0;
				} else {
					uv = vec2(dir.x, -dir.z) * inv_axis + 0.5;
					face = 3.0;
				}
			} else {
				float inv_axis = 0.5 / abs_dir.z;
				if (dir.z > 0.0) {
					uv = vec2(dir.x, -dir.y) * inv_axis + 0.5;
					face = 4.0;
				} else {
					uv = vec2(-dir.x, -dir.y) * inv_axis + 0.5;
					face = 5.0;
				}
			}

			uv = clamp(uv, vec2(0.0), vec2(1.0));
			return textureLod(cubemap, vec3(uv, face), 0.0).rgb;"
						.into(),
				),
				None,
				Some(
					"
			float direction_length = length(direction);
			if (direction_length <= 0.0) { return float3(1.0); }

			float3 dir = direction / direction_length;
			float3 abs_dir = abs(dir);

			if (max(max(abs_dir.x, abs_dir.y), abs_dir.z) <= 0.0) { return float3(1.0); }

			float2 uv = float2(0.5);
			float face = 0.0;

			if (abs_dir.x >= abs_dir.y && abs_dir.x >= abs_dir.z) {
				float inv_axis = 0.5 / abs_dir.x;
				if (dir.x > 0.0) {
					uv = float2(-dir.z, -dir.y) * inv_axis + 0.5;
					face = 0.0;
				} else {
					uv = float2(dir.z, -dir.y) * inv_axis + 0.5;
					face = 1.0;
				}
			} else if (abs_dir.y >= abs_dir.z) {
				float inv_axis = 0.5 / abs_dir.y;
				if (dir.y > 0.0) {
					uv = float2(dir.x, dir.z) * inv_axis + 0.5;
					face = 2.0;
				} else {
					uv = float2(dir.x, -dir.z) * inv_axis + 0.5;
					face = 3.0;
				}
			} else {
				float inv_axis = 0.5 / abs_dir.z;
				if (dir.z > 0.0) {
					uv = float2(dir.x, -dir.y) * inv_axis + 0.5;
					face = 4.0;
				} else {
					uv = float2(-dir.x, -dir.y) * inv_axis + 0.5;
					face = 5.0;
				}
			}

			uv = clamp(uv, float2(0.0), float2(1.0));
			constexpr sampler ibl_sampler(coord::normalized, address::clamp_to_edge, filter::linear);
			return cubemap.sample(ibl_sampler, uv, uint(face), level(0.0)).rgb;"
						.into(),
				),
				&[],
				&[],
			)],
		);

		Node::scope(
			"Visibility",
			vec![
				view_struct,
				views_binding,
				mesh_struct,
				meshlet_struct,
				mesh_outputs,
				out_instance_index,
				out_primitive_index,
				light_struct,
				material_struct,
				sample_shadow_tap,
				sample_shadow,
				meshes,
				positions,
				normals,
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
				compute_vertex_index,
				u8_to_u32,
				u16_to_u32,
				compute_vertex_position,
				compute_triangle,
				process_meshlet,
				set2_binding0,
				set2_binding1,
				set2_binding2,
				set2_binding3,
				set2_binding4,
				set2_binding5,
				set2_binding10,
				set2_binding11,
				set2_binding12,
				set2_binding13,
				push_constant,
				sample_function,
				sample_normal_function,
				sample_ibl_cubemap,
			],
		)
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a json::Object) -> besl::parser::Node<'a> {
		let a = "if (gl_GlobalInvocationID.x >= material_count.material_count[push_constant.material_id]) { return; }

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

		uint primitive_indices[3] = uint[3](
			primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 0],
			primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 1],
			primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 2]
		);

		uint vertex_indices[3] = uint[3](
			compute_vertex_index(mesh, meshlet, primitive_indices[0]),
			compute_vertex_index(mesh, meshlet, primitive_indices[1]),
			compute_vertex_index(mesh, meshlet, primitive_indices[2])
		);

		vec4 model_space_vertex_positions[3] = vec4[3](
			vec4(vertex_positions.positions[vertex_indices[0]], 1.0),
			vec4(vertex_positions.positions[vertex_indices[1]], 1.0),
			vec4(vertex_positions.positions[vertex_indices[2]], 1.0)
		);

		vec4 vertex_normals[3] = vec4[3](
			vec4(vertex_normals.normals[vertex_indices[0]], 0.0),
			vec4(vertex_normals.normals[vertex_indices[1]], 0.0),
			vec4(vertex_normals.normals[vertex_indices[2]], 0.0)
		);

		vec2 vertex_uvs[3] = vec2[3](
			vertex_uvs.uvs[vertex_indices[0]],
			vertex_uvs.uvs[vertex_indices[1]],
			vertex_uvs.uvs[vertex_indices[2]]
		);

		ivec2 image_extent = imageSize(triangle_index);
		vec2 normalized_xy = (vec2(pixel_coordinates) + vec2(0.5)) / vec2(image_extent);
		vec2 nc = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);

		View view = views.views[0];
		float surface_depth = texelFetch(visibility_depth, pixel_coordinates, 0).r;
		vec4 surface_clip_position = vec4(nc, surface_depth, 1.0);
		vec4 surface_view_position = view.inverse_projection * surface_clip_position;
		surface_view_position /= surface_view_position.w;
		vec3 world_space_surface_position = (view.inverse_view * surface_view_position).xyz;

		vec4 world_space_vertex_positions[3] = vec4[3](mesh.model * model_space_vertex_positions[0], mesh.model * model_space_vertex_positions[1], mesh.model * model_space_vertex_positions[2]);
		vec4 clip_space_vertex_positions[3] = vec4[3](view.view_projection * world_space_vertex_positions[0], view.view_projection * world_space_vertex_positions[1], view.view_projection * world_space_vertex_positions[2]);

		vec4 world_space_vertex_normals[3] = vec4[3](normalize(mesh.model * vertex_normals[0]), normalize(mesh.model * vertex_normals[1]), normalize(mesh.model * vertex_normals[2]));

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, vec2(image_extent));
		vec3 barycenter = barycentric_deriv.lambda;
		vec3 ddx = barycentric_deriv.ddx;
		vec3 ddy = barycentric_deriv.ddy;

		vec3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);
		vec3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_positions[0].xyz, clip_space_vertex_positions[1].xyz, clip_space_vertex_positions[2].xyz);
		vec3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normals[0].xyz, world_space_vertex_normals[1].xyz, world_space_vertex_normals[2].xyz));
		vec2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		vec3 N = world_space_vertex_normal;
		vec3 camera_position = view.inverse_view[3].xyz;
		vec3 V = normalize(camera_position - world_space_vertex_position);

		vec3 pos_dx = interpolate_vec3f_with_deriv(ddx, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);
		vec3 pos_dy = interpolate_vec3f_with_deriv(ddy, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);

		vec2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);
		vec2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		vec3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		vec3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
		mat3 TBN = mat3(T, B, N);

		vec4 albedo = vec4(1, 0, 0, 1);
		vec3 normal = vec3(0, 0, 1);
		float metalness = 0.0;
		float roughness = float(0.5)"
			.trim();

		let a_msl = "if (gid.x >= set1.material_count->material_count[push_constant.material_id]) { return; }

		uint offset = set1.material_offset->material_offset[push_constant.material_id];
		uint2 raw_pixel_coordinates = uint2(set1.pixel_mapping->pixel_mapping[offset + gid.x]);
		if (raw_pixel_coordinates.x == 0u || raw_pixel_coordinates.y == 0u) { return; }
		int2 pixel_coordinates = int2(raw_pixel_coordinates) - int2(1);
		int2 image_extent = int2(set1.triangle_index.get_width(), set1.triangle_index.get_height());
		if (pixel_coordinates.x < 0 || pixel_coordinates.y < 0 || pixel_coordinates.x >= image_extent.x || pixel_coordinates.y >= image_extent.y) { return; }
		uint triangle_meshlet_indices = set1.triangle_index.read(uint2(pixel_coordinates)).x;
		uint instance_index = set1.instance_index_render_target.read(uint2(pixel_coordinates)).x;
		uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
		uint meshlet_index = triangle_meshlet_indices >> 8;

		Meshlet meshlet = set0.meshlets->meshlets[meshlet_index];

		Mesh mesh = set0.meshes->meshes[instance_index];

		Material material = set2.materials->materials[push_constant.material_id];

		uint primitive_indices[3] = {
			set0.primitive_indices->primitive_indices[(mesh.base_triangle_index + uint(meshlet.triangle_offset) + meshlet_triangle_index) * 3 + 0],
			set0.primitive_indices->primitive_indices[(mesh.base_triangle_index + uint(meshlet.triangle_offset) + meshlet_triangle_index) * 3 + 1],
			set0.primitive_indices->primitive_indices[(mesh.base_triangle_index + uint(meshlet.triangle_offset) + meshlet_triangle_index) * 3 + 2]
		};

		uint vertex_indices[3] = {
			compute_vertex_index(mesh, meshlet, primitive_indices[0], gid, push_constant, set0, set1, set2),
			compute_vertex_index(mesh, meshlet, primitive_indices[1], gid, push_constant, set0, set1, set2),
			compute_vertex_index(mesh, meshlet, primitive_indices[2], gid, push_constant, set0, set1, set2)
		};

		float4 model_space_vertex_positions[3] = {
			float4(set0.vertex_positions->positions[vertex_indices[0]], 1.0),
			float4(set0.vertex_positions->positions[vertex_indices[1]], 1.0),
			float4(set0.vertex_positions->positions[vertex_indices[2]], 1.0)
		};

		float4 vertex_normals[3] = {
			float4(set0.vertex_normals->normals[vertex_indices[0]], 0.0),
			float4(set0.vertex_normals->normals[vertex_indices[1]], 0.0),
			float4(set0.vertex_normals->normals[vertex_indices[2]], 0.0)
		};

		float2 vertex_uvs[3] = {
			set0.vertex_uvs->uvs[vertex_indices[0]],
			set0.vertex_uvs->uvs[vertex_indices[1]],
			set0.vertex_uvs->uvs[vertex_indices[2]]
		};

		float2 normalized_xy = (float2(pixel_coordinates) + float2(0.5)) / float2(image_extent);
		float2 nc = make_raster_ndc_from_pixel_coordinates(pixel_coordinates, image_extent);

		View view = set0.views->views[0];
		float surface_depth = set2.visibility_depth.sample(set2.visibility_depth_sampler, normalized_xy).r;
		float4 surface_clip_position = float4(nc, surface_depth, 1.0);
		float4 surface_view_position = surface_clip_position * view.inverse_projection;
		surface_view_position /= surface_view_position.w;
		float3 world_space_surface_position = (surface_view_position * view.inverse_view).xyz;

		float4 world_space_vertex_positions[3] = {model_space_vertex_positions[0] * mesh.model, model_space_vertex_positions[1] * mesh.model, model_space_vertex_positions[2] * mesh.model};
		float4 clip_space_vertex_positions[3] = {world_space_vertex_positions[0] * view.view_projection, world_space_vertex_positions[1] * view.view_projection, world_space_vertex_positions[2] * view.view_projection};

		float4 world_space_vertex_normals[3] = {normalize(vertex_normals[0] * mesh.model), normalize(vertex_normals[1] * mesh.model), normalize(vertex_normals[2] * mesh.model)};

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, float2(image_extent));
		float3 barycenter = barycentric_deriv.lambda;
		float3 ddx = barycentric_deriv.ddx;
		float3 ddy = barycentric_deriv.ddy;

		float3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);
		float3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_positions[0].xyz, clip_space_vertex_positions[1].xyz, clip_space_vertex_positions[2].xyz);
		float3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normals[0].xyz, world_space_vertex_normals[1].xyz, world_space_vertex_normals[2].xyz));
		float2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		float3 N = world_space_vertex_normal;
		float3 camera_position = view.inverse_view[3].xyz;
		float3 V = normalize(camera_position - world_space_vertex_position);

		float3 pos_dx = interpolate_vec3f_with_deriv(ddx, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);
		float3 pos_dy = interpolate_vec3f_with_deriv(ddy, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);

		float2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);
		float2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		float3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		float3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
		float3x3 TBN = float3x3(T, B, N);

		float4 albedo = float4(1, 0, 0, 1);
		float3 normal = float3(0, 0, 1);
		float metalness = 0.0;
		float roughness = float(0.5)"
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
					let slot = Box::leak(texture_count.to_string().into_boxed_str());
					let mut slot_root = besl::parse(slot).expect("Expected texture slot literal to parse");
					let slot_node = match slot_root.node_mut() {
						besl::parser::Nodes::Scope { children, .. } => children.remove(0),
						_ => panic!(
							"Expected texture slot literal to parse into a scope. The most likely cause is invalid visibility texture slot generation."
						),
					};
					let x = besl::parser::Node::literal(name, slot_node);
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let b_msl = "
		float3 diffuse = float3(0.0);
		float3 specular = float3(0.0);

		float ao_factor = set2.ao.sample(set2.ao_sampler, normalized_xy).r;

		normal = normalize(TBN * normal);
		float3 F0 = mix(float3(0.04), albedo.xyz, metalness);
		float NdotV = max(dot(normal, V), 0.0);

		for (uint i = 0; i < set2.lighting_data->light_count; ++i) {
			Light light = set2.lighting_data->lights[i];

			float3 L = float3(0.0);

			if (light.type == 68) {
				L = normalize(-light.position);
			} else {
				L = normalize(light.position - world_space_vertex_position);
			}

			float NdotL = max(dot(normal, L), 0.0);

			if (NdotL <= 0.0) { continue; }

			float occlusion_factor = 1.0;
			float attenuation = 1.0;

			if (light.type == 68) {
				float4 view_space_surface_position = float4(world_space_surface_position, 1.0) * view.view;
				float c_occlusion_factor  = sample_shadow(set2.depth_shadow_map, light, world_space_surface_position, view_space_surface_position.xyz, world_space_vertex_normal, gid, push_constant, set0, set1, set2);

				occlusion_factor = c_occlusion_factor;

				if (occlusion_factor == 0.0) { continue; }

				attenuation = 1.0;
			} else {
				float distance = length(light.position - world_space_vertex_position);
				attenuation = 1.0 / (distance * distance);
			}

			float3 H = normalize(V + L);

			float3 radiance = light.color * attenuation;

			float3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

			float NDF = distribution_ggx(normal, H, roughness);
			float G = geometry_smith(normal, V, L, roughness);
			float3 local_specular = (NDF * G * F) / (4.0 * max(dot(normal, V), 0.0) * max(dot(normal, L), 0.0) + 0.000001);

			float3 kS = F;
			float3 kD = (float3(1.0) - fresnel_schlick(NdotL, F0)) * (float3(1.0) - fresnel_schlick(NdotV, F0));

			kD *= 1.0 - metalness;

			float3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		float3 irradiance = sample_ibl_cubemap(set2.ibl_cubemap, normal);

		float3 F_ibl = fresnel_schlick_roughness(NdotV, F0, roughness);
		float3 kD_ibl = (float3(1.0) - F_ibl) * (1.0 - metalness);

		float3 ibl_diffuse = kD_ibl * albedo.xyz * irradiance;

		float2 env_brdf = float2(1.0, 0.0);
		{
			float4 c0 = float4(-1.0, -0.0275, -0.572, 0.022);
			float4 c1 = float4(1.0, 0.0425, 1.04, -0.04);
			float4 r = roughness * c0 + c1;
			float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
			env_brdf = float2(-1.04, 1.04) * a004 + r.zw;
		}
		float3 ibl_specular = (F_ibl * env_brdf.x + env_brdf.y) * irradiance;

		float3 ambient = ibl_diffuse + ibl_specular;

		diffuse = diffuse * ao_factor + ambient * ao_factor;
		specular = specular * ao_factor;

		set2.diffuse_map.write(float4(diffuse, albedo.a), uint2(pixel_coordinates));
		set2.specular_map.write(float4(specular, 1.0), uint2(pixel_coordinates))
		"
		.trim();

		let b = "
		vec3 diffuse = vec3(0.0);
		vec3 specular = vec3(0.0);

		float ao_factor = texture(ao, normalized_xy).r;

		normal = normalize(TBN * normal);
		vec3 F0 = mix(vec3(0.04), albedo.xyz, metalness);
		float NdotV = max(dot(normal, V), 0.0);

		for (uint i = 0; i < lighting_data.light_count; ++i) {
			Light light = lighting_data.lights[i];

			vec3 L = vec3(0.0);

			if (light.type == 68) { // Infinite
				L = normalize(-light.position);
			} else {
				L = normalize(light.position - world_space_vertex_position);
			}

			float NdotL = max(dot(normal, L), 0.0);

			if (NdotL <= 0.0) { continue; }

			float occlusion_factor = 1.0;
			float attenuation = 1.0;

			if (light.type == 68) { // Infinite
				vec4 view_space_surface_position = view.view * vec4(world_space_surface_position, 1.0);
				float c_occlusion_factor  = sample_shadow(depth_shadow_map, light, world_space_surface_position, view_space_surface_position.xyz, world_space_vertex_normal);

				occlusion_factor = c_occlusion_factor;

				if (occlusion_factor == 0.0) { continue; }

				// attenuation = occlusion_factor;
				attenuation = 1.0;
			} else {
				float distance = length(light.position - world_space_vertex_position);
				attenuation = 1.0 / (distance * distance);
			}

			vec3 H = normalize(V + L);

			vec3 radiance = light.color * attenuation;

			vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

			float NDF = distribution_ggx(normal, H, roughness);
			float G = geometry_smith(normal, V, L, roughness);
			vec3 local_specular = (NDF * G * F) / (4.0 * max(dot(normal, V), 0.0) * max(dot(normal, L), 0.0) + 0.000001);

			vec3 kS = F;
			vec3 kD = (vec3(1.0) - fresnel_schlick(NdotL, F0)) * (vec3(1.0) - fresnel_schlick(NdotV, F0));

			kD *= 1.0 - metalness;

			vec3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		vec3 irradiance = sample_ibl_cubemap(ibl_cubemap, normal);

		vec3 F_ibl = fresnel_schlick_roughness(NdotV, F0, roughness);
		vec3 kD_ibl = (vec3(1.0) - F_ibl) * (1.0 - metalness);

		vec3 ibl_diffuse = kD_ibl * albedo.xyz * irradiance;

		vec2 env_brdf = vec2(1.0, 0.0);
		{
			vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
			vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
			vec4 r = roughness * c0 + c1;
			float a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
			env_brdf = vec2(-1.04, 1.04) * a004 + r.zw;
		}
		vec3 ibl_specular = (F_ibl * env_brdf.x + env_brdf.y) * irradiance;

		vec3 ambient = ibl_diffuse + ibl_specular;

		diffuse = diffuse * ao_factor + ambient * ao_factor;
		specular = specular * ao_factor;

		imageStore(diffuse_map, pixel_coordinates, vec4(diffuse, albedo.a));
		imageStore(specular_map, pixel_coordinates, vec4(specular, 1.0))
		"
		.trim();

		let m = root.get_mut("main").unwrap();

		match m.node_mut() {
			besl::parser::Nodes::Function { statements, .. } => {
				statements.insert(
					0,
					besl::parser::Node::raw_code(
						Some(a.into()),
						None,
						Some(a_msl.into()),
						&[
							"vertex_uvs",
							"ao",
							"depth_shadow_map",
							"visibility_depth",
							"push_constant",
							"material_offset",
							"material_offset_scratch",
							"material_evaluation_dispatches",
							"pixel_mapping",
							"material_count",
							"meshes",
							"meshlets",
							"materials",
							"primitive_indices",
							"vertex_indices",
							"vertex_positions",
							"vertex_normals",
							"triangle_index",
							"instance_index_render_target",
							"views",
							"make_raster_ndc_from_pixel_coordinates",
							"calculate_full_bary",
							"interpolate_vec3f_with_deriv",
							"interpolate_vec2f_with_deriv",
							"fresnel_schlick",
							"distribution_ggx",
							"geometry_smith",
							"compute_vertex_index",
						],
						&["material", "albedo", "normal", "roughness", "metalness"],
					),
				);
				statements.push(besl::parser::Node::raw_code(
					Some(b.into()),
					None,
					Some(b_msl.into()),
					&[
						"lighting_data",
						"diffuse_map",
						"specular_map",
						"_unused_set2_binding1",
						"_unused_set2_binding3",
						"sample_shadow",
						"ibl_cubemap",
						"sample_ibl_cubemap",
						"fresnel_schlick_roughness",
					],
					&[],
				));
			}
			_ => {}
		}

		root.add(extra);
		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root
	}
}

#[cfg(test)]
mod tests {
	use resource_management::asset::bema_asset_handler::ProgramGenerator;
	use resource_management::{
		msl_shader_generator::MSLShaderGenerator,
		shader_generator::{ShaderGenerationSettings, ShaderGenerator as _},
		spirv_shader_generator::SPIRVShaderGenerator,
	};
	use utils::json;
	use utils::Extent;

	use crate::besl;
	use crate::rendering::map_shader_binding_to_shader_binding_descriptor;

	#[test]
	fn write_to_albedo() {
		let material = json::object! {
			"variables": []
		};

		let shader_source = "main: fn () -> void { albedo = vec4f(1, 2, 3, 4); }";

		let shader_node = besl::parse(shader_source).unwrap();

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		let _node = besl::lex(shader).unwrap();
	}

	#[test]
	fn vec4f_variable() {
		let material = json::object! {
			"variables": [
				{
					"name": "albedo",
					"data_type": "vec4f",
					"value": "Purple"
				}
			]
		};

		let shader_source = "main: fn () -> void { out_color = albedo; }";

		let shader_node = besl::parse(shader_source).unwrap();

		let shader_generator = super::VisibilityShaderGenerator::new(true, true, true, true, true, true, true, true);

		let shader = shader_generator.transform(shader_node, &material);

		println!("{:#?}", shader);

		// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	}

	#[test]
	fn material_evaluation_msl_source_compiles_for_metal() {
		use ghi::device::DeviceCreate as _;

		if !ghi::implementation::USES_METAL {
			return;
		}

		let material = json::object! {
			"variables": []
		};

		let shader_source = "main: fn () -> void { albedo = vec4f(1.0, 1.0, 1.0, 1.0); }";
		let shader_node = besl::parse(shader_source).unwrap();
		let shader_generator = super::VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);
		let shader = shader_generator.transform(shader_node, &material);
		let root = besl::lex(shader).unwrap();
		let main_node = root.get_main().unwrap();
		let settings = ShaderGenerationSettings::compute(Extent::line(128));
		let mut source_generator = MSLShaderGenerator::new();
		let source = source_generator.generate(&settings, &main_node).unwrap();
		let reflected_shader = SPIRVShaderGenerator::new().generate(&settings, &main_node).unwrap();
		let bindings = reflected_shader
			.bindings()
			.iter()
			.map(map_shader_binding_to_shader_binding_descriptor)
			.collect::<Vec<_>>();

		let mut instance = ghi::implementation::Instance::new(ghi::device::Features::new())
			.expect("Expected a Metal instance for the material evaluation shader test");
		let mut queue = None;
		let mut device = instance
			.create_device(
				ghi::device::Features::new(),
				&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::COMPUTE), &mut queue)],
			)
			.expect("Expected a Metal device for the material evaluation shader test");

		let shader_handle = device.create_shader(
			Some("Material Evaluation Shader"),
			ghi::shader::Sources::MTL {
				source: source.as_str(),
				entry_point: "besl_main",
			},
			ghi::ShaderTypes::Compute,
			bindings,
		);

		assert!(
			shader_handle.is_ok(),
			"Expected the material evaluation MSL source to compile for Metal"
		);
		assert!(
			source.contains("constant _material_offset_scratch* material_offset_scratch [[id(2)]];")
				&& source.contains("constant _material_evaluation_dispatches* material_evaluation_dispatches [[id(3)]];")
				&& source.contains("constant _pixel_mapping* pixel_mapping [[id(4)]];"),
			"Expected the material evaluation MSL source to preserve the full visibility set1 binding layout. Shader: {source}"
		);
		assert!(
			source.contains("texture2d<float, access::write> diffuse_map [[id(0)]];")
				&& source.contains("constant __unused_set2_binding1* _unused_set2_binding1 [[id(1)]];")
				&& source.contains("texture2d<float, access::write> specular_map [[id(2)]];")
				&& source.contains("texture2d<float, access::write> _unused_set2_binding3 [[id(3)]];")
				&& source.contains("texture2d<float> ao [[id(6)]];")
				&& source.contains("sampler ao_sampler [[id(7)]];")
				&& source.contains("texture2d<float> visibility_depth [[id(10)]];")
				&& source.contains("sampler visibility_depth_sampler [[id(11)]];"),
			"Expected the material evaluation MSL source to preserve the full visibility set2 binding layout. Shader: {source}"
		);
	}

	// #[test]
	// fn multiple_textures() {
	// 	let material = json::object! {
	// 		"variables": [
	// 			{
	// 				"name": "albedo",
	// 				"data_type": "Texture2D",
	// 			},
	// 			{
	// 				"name": "normal",
	// 				"data_type": "Texture2D",
	// 			}
	// 		]
	// 	};

	// 	let shader_source = "main: fn () -> void { out_color = sample(albedo); }";

	// 	let shader_node = besl::compile_to_besl(shader_source, None).unwrap();

	// 	let shader_generator = super::VisibilityShaderGenerator::new();

	// 	let shader = shader_generator.transform(&material, &shader_node, "Fragment").expect("Failed to generate shader");

	// 	// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	// }
}
