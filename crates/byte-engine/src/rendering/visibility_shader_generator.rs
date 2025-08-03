use std::{cell::RefCell, ops::Deref, rc::Rc};

use besl::{parser::Node, NodeReference};
use resource_management::asset::material_asset_handler::ProgramGenerator;
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use crate::rendering::common_shader_generator::CommonShaderScope;

use super::common_shader_generator::CommonShaderGenerator;

pub struct VisibilityShaderScope {
}

pub struct VisibilityShaderGenerator {
	scope: Node,
}

impl VisibilityShaderGenerator {
	pub fn new(material_count_read: bool, material_count_write: bool, material_offset_read: bool, material_offset_write: bool, material_offset_scratch_read: bool, material_offset_scratch_write: bool, pixel_mapping_read: bool, pixel_mapping_write: bool) -> Self {
		Self {
			scope: VisibilityShaderScope::new_with_params(material_count_read, material_count_write, material_offset_read, material_offset_write, material_offset_scratch_read, material_offset_scratch_write, pixel_mapping_read, pixel_mapping_write),
		}
	}
}

impl VisibilityShaderScope {
	pub fn new() -> besl::parser::Node {
		Self::new_with_params(true, true, true, true, true, true, true, true)
	}

	pub fn new_with_params(material_count_read: bool, material_count_write: bool, material_offset_read: bool, material_offset_write: bool, material_offset_scratch_read: bool, material_offset_scratch_write: bool, pixel_mapping_read: bool, pixel_mapping_write: bool) -> besl::parser::Node {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct("Mesh", vec![Node::member("model", "mat4f"), Node::member("material_index", "u32"), Node::member("base_vertex_index", "u32"), Node::member("base_primitive_index", "u32"), Node::member("base_triangle_index", "u32"), Node::member("base_meshlet_index", "u32")]);
		let view_struct = Node::r#struct("View", vec![Node::member("view", "mat4f"), Node::member("projection", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("inverse_view", "mat4f"), Node::member("inverse_projection", "mat4f"), Node::member("inverse_view_projection", "mat4f"), Node::member("fov", "vec2f"), Node::member("near", "f32"), Node::member("far", "f32"),]);
		let meshlet_struct = Node::r#struct("Meshlet", vec![Node::member("primitive_offset", "u16"), Node::member("triangle_offset", "u16"), Node::member("primitive_count", "u8"), Node::member("triangle_count", "u8")]);
		let light_struct = Node::r#struct("Light", vec![Node::member("position", "vec3f"), Node::member("color", "vec3f"), Node::member("type", "u8"), Node::member("cascades", "u32[8]")]);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);

		let views_binding = Node::binding("views", Node::buffer("ViewsBuffer", vec![Node::member("views", "View[8]")]), 0, 0, true, false);
		let meshes = Node::binding("meshes", Node::buffer("MeshBuffer", vec![Node::member("meshes", "Mesh[64]")]), 0, 1, true, false);
		let positions = Node::binding("vertex_positions", Node::buffer("Positions", vec![Node::member("positions", "vec3f[8192]")]), 0, 2, true, false);
		let normals = Node::binding("vertex_normals", Node::buffer("Normals", vec![Node::member("normals", "vec3f[8192]")]), 0, 3, true, false);
		let uvs = Node::binding("vertex_uvs", Node::buffer("UVs", vec![Node::member("uvs", "vec2f[8192]")]), 0, 5, true, false);
		let vertex_indices = Node::binding("vertex_indices", Node::buffer("VertexIndices", vec![Node::member("vertex_indices", "u16[8192]")]), 0, 6, true, false);
		let primitive_indices = Node::binding("primitive_indices", Node::buffer("PrimitiveIndices", vec![Node::member("primitive_indices", "u8[8192]")]), 0, 7, true, false);
		let meshlets = Node::binding("meshlets", Node::buffer("MeshletsBuffer", vec![Node::member("meshlets", "Meshlet[8192]")]), 0, 8, true, false);
		let textures = Node::binding_array("textures", Node::combined_image_sampler(), 0, 9, true, false, 16);

		let material_count = Node::binding("material_count", Node::buffer("MaterialCount", vec![Node::member("material_count", "u32[2073600]")]), 1, 0, material_count_read, material_count_write); // TODO: somehow set read/write properties per shader
		let material_offset = Node::binding("material_offset", Node::buffer("MaterialOffset", vec![Node::member("material_offset", "u32[2073600")]), 1, 1, material_offset_read, material_offset_write);
		let material_offset_scratch = Node::binding("material_offset_scratch", Node::buffer("MaterialOffsetScratch", vec![Node::member("material_offset_scratch", "u32[2073600]")]), 1, 2, material_offset_scratch_read, material_offset_scratch_write);
		let material_evaluation_dispatches = Node::binding("material_evaluation_dispatches", Node::buffer("MaterialEvaluationDispatches", vec![Node::member("material_evaluation_dispatches", "vec3u[2073600]")]), 1, 3, material_offset_read, material_offset_write);
		let pixel_mapping = Node::binding("pixel_mapping", Node::buffer("PixelMapping", vec![Node::member("pixel_mapping", "vec2u16[2073600]")]), 1, 4, pixel_mapping_read, pixel_mapping_write);
		let triangle_index = Node::binding("triangle_index", Node::image("r32ui"), 1, 6, true, false);
		let instance_index = Node::binding("instance_index_render_target", Node::image("r32ui"), 1, 7, true, false);

		let compute_vertex_index = Node::function("compute_vertex_index", vec![Node::parameter("mesh", "Mesh"), Node::parameter("meshlet", "Meshlet"), Node::parameter("primitive_index", "u32")], "u32", vec![Node::glsl("return mesh.base_vertex_index + vertex_indices.vertex_indices[mesh.base_primitive_index + meshlet.primitive_offset + primitive_index]; /* Indices in the buffer are relative to each mesh/primitives */", &["vertex_indices"], Vec::new())]);

		let process_meshlet = Node::function("process_meshlet", vec![Node::parameter("instance_index", "u32"), Node::parameter("matrix", "mat4f")], "void", vec![Node::glsl("
		Mesh mesh = meshes.meshes[instance_index];

		uint meshlet_index = gl_WorkGroupID.x + mesh.base_meshlet_index;
		Meshlet meshlet = meshlets.meshlets[meshlet_index];

		SetMeshOutputsEXT(meshlet.primitive_count, meshlet.triangle_count);

		uint primitive_index = gl_LocalInvocationID.x;

		if (primitive_index < uint(meshlet.primitive_count)) {
			uint vertex_index = compute_vertex_index(mesh, meshlet, primitive_index);
			gl_MeshVerticesEXT[primitive_index].gl_Position = matrix * mesh.model * vec4(vertex_positions.positions[vertex_index], 1.0);
		}

		if (primitive_index < uint(meshlet.triangle_count)) {
			uint triangle_index = (mesh.base_triangle_index + meshlet.triangle_offset + primitive_index) * 3;
			uint triangle_indices[3] = uint[](primitive_indices.primitive_indices[triangle_index + 0], primitive_indices.primitive_indices[triangle_index + 1], primitive_indices.primitive_indices[triangle_index + 2]);
			gl_PrimitiveTriangleIndicesEXT[primitive_index] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
			out_instance_index[primitive_index] = instance_index;
			out_primitive_index[primitive_index] = (meshlet_index << 8) | (primitive_index & 0xFF);
		}", &["meshes", "vertex_positions", "vertex_indices", "primitive_indices", "meshlets", "compute_vertex_index"], Vec::new())]);

		let set2_binding0 = Node::binding("diffuse_map", Node::image("rgba16"), 2, 0, false, true);
		let set2_binding2 = Node::binding("specular_map", Node::image("rgba16"), 2, 2, false, true);
		let set2_binding4 = Node::binding("lighting_data", Node::buffer("LightingBuffer", vec![Node::member("light_count", "u32"), Node::member("lights", "Light[16]")]), 2, 4, true, false);
		let set2_binding5 = Node::binding("materials", Node::buffer("MaterialBuffer", vec![Node::member("materials", "Material[16]")]), 2, 5, true, false);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 2, 10, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_array_image_sampler(), 2, 11, true, false);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32")]);

		let sample_function = Node::intrinsic("sample", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("texture(", &[], Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv)", &[], Vec::new())]), "vec4f");

		let sample_normal_function = if true {
			Node::intrinsic("sample_normal", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("unit_vector_from_xy(texture(", &[], Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv).xy)", &["unit_vector_from_xy"], Vec::new())]), "vec3f")
		} else {
			Node::intrinsic("sample_normal", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("normalize(texture(", &[], Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv).xyz * 2.0f - 1.0f)", &[], Vec::new())]), "vec3f")
		};

		// Depth comparison is "inverted" because the depth buffer is stored in a reversed manner
		let sample_shadow = Node::function("sample_shadow", vec![Node::parameter("shadow_map", "ArrayTexture2D"), Node::parameter("light", "Light"), Node::parameter("world_space_position", "vec3f"), Node::parameter("view_space_position", "vec3f"), Node::parameter("surface_normal", "vec3f"), Node::parameter("offset", "vec2f")], "f32", vec![Node::glsl("
			float depth_value = abs(view_space_position.z);

			uint cascade_index = -1;

			for (uint i = 0; i < 4; ++i) {
				if (depth_value < views.views[light.cascades[i]].far) {
					cascade_index = light.cascades[i];
					break;
				}
			}

			View view = views.views[light.cascades[cascade_index]];

			vec4 surface_light_clip_position = view.view_projection * vec4(world_space_position + surface_normal * 0.001, 1.0);
			vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;

			vec2 shadow_uv = surface_light_ndc_position.xy * 0.5f + 0.5f;

			float surface_depth = surface_light_ndc_position.z;

			if (surface_depth < 0 || surface_depth > 1.0f) { return 1.0; }

			float closest_depth = texture(shadow_map, vec3(shadow_uv, float(cascade_index))).r;

			return surface_depth < closest_depth ? 0.0 : 1.0", &["views"], Vec::new())]);

		Node::scope("Visibility", vec![
			view_struct,
			views_binding,
			mesh_struct,
			meshlet_struct,
			light_struct,
			material_struct,

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
			process_meshlet,

			set2_binding0,
			set2_binding2,
			set2_binding4,
			set2_binding5,
			set2_binding10,
			set2_binding11,
			push_constant,
			sample_function,
			sample_normal_function,
			sample_shadow,
		])
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, material: &json::Object) -> besl::parser::Node {
		let a = "if (gl_GlobalInvocationID.x >= material_count.material_count[push_constant.material_id]) { return; }

		uint offset = material_offset.material_offset[push_constant.material_id];
		ivec2 pixel_coordinates = ivec2(pixel_mapping.pixel_mapping[offset + gl_GlobalInvocationID.x]);
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

		vec2 image_extent = imageSize(triangle_index);

		vec2 normalized_xy = pixel_coordinates / image_extent;

		vec2 nc = normalized_xy * 2 - 1;

		View view = views.views[0];

		vec4 world_space_vertex_positions[3] = vec4[3](mesh.model * model_space_vertex_positions[0], mesh.model * model_space_vertex_positions[1], mesh.model * model_space_vertex_positions[2]);
		vec4 clip_space_vertex_positions[3] = vec4[3](view.view_projection * world_space_vertex_positions[0], view.view_projection * world_space_vertex_positions[1], view.view_projection * world_space_vertex_positions[2]);

		vec4 world_space_vertex_normals[3] = vec4[3](normalize(mesh.model * vertex_normals[0]), normalize(mesh.model * vertex_normals[1]), normalize(mesh.model * vertex_normals[2]));

		BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, image_extent);
		vec3 barycenter = barycentric_deriv.lambda;
		vec3 ddx = barycentric_deriv.ddx;
		vec3 ddy = barycentric_deriv.ddy;

		vec3 world_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, world_space_vertex_positions[0].xyz, world_space_vertex_positions[1].xyz, world_space_vertex_positions[2].xyz);
		vec3 clip_space_vertex_position = interpolate_vec3f_with_deriv(barycenter, clip_space_vertex_positions[0].xyz, clip_space_vertex_positions[1].xyz, clip_space_vertex_positions[2].xyz);
		vec3 world_space_vertex_normal = normalize(interpolate_vec3f_with_deriv(barycenter, world_space_vertex_normals[0].xyz, world_space_vertex_normals[1].xyz, world_space_vertex_normals[2].xyz));
		vec2 vertex_uv = interpolate_vec2f_with_deriv(barycenter, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		vec3 N = world_space_vertex_normal;
		// vec3 V = normalize(view.view[3].xyz - world_space_vertex_position); /* Grey spots sometimes appear in renders, might be due to this line */
		vec3 V = normalize(-(view.view[3].xyz - world_space_vertex_position));

		vec3 pos_dx = interpolate_vec3f_with_deriv(ddx, model_space_vertex_positions[0].xyz, model_space_vertex_positions[1].xyz, model_space_vertex_positions[2].xyz);
		vec3 pos_dy = interpolate_vec3f_with_deriv(ddy, model_space_vertex_positions[0].xyz, model_space_vertex_positions[1].xyz, model_space_vertex_positions[2].xyz);

		vec2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);
		vec2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

		float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
		vec3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
		vec3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
		mat3 TBN = mat3(T, B, N);

		vec4 albedo = vec4(1, 0, 0, 1);
		vec3 normal = vec3(0, 0, 1);
		float metalness = 0;
		float roughness = float(0.5);".trim();

		let mut extra = Vec::new();

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
					let x = besl::parser::Node::literal(name, besl::parser::Node::glsl(&format!("textures[nonuniformEXT(material.textures[{}])]", texture_count), &[/* TODO: fix literals "material".to_string(), */"textures"], Vec::new()));
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let b = "
		vec3 diffuse = vec3(0.0);
		vec3 specular = vec3(0.0);

		float ao_factor = texture(ao, normalized_xy).r;

		normal = normalize(TBN * normal);

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
				vec4 view_space_vertex_position = view.view * vec4(world_space_vertex_position, 1.0);
				float c_occlusion_factor  = sample_shadow(depth_shadow_map, light, world_space_vertex_position, view_space_vertex_position.xyz, normal, vec2( 0.00,  0.00));

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

			vec3 F0 = vec3(0.04);
			F0 = mix(F0, albedo.xyz, metalness);
			vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

			float NDF = distribution_ggx(normal, H, roughness);
			float G = geometry_smith(normal, V, L, roughness);
			vec3 local_specular = (NDF * G * F) / (4.0 * max(dot(normal, V), 0.0) * max(dot(normal, L), 0.0) + 0.000001);

			vec3 kS = F;
			vec3 kD = vec3(1.0) - kS;

			kD *= 1.0 - metalness;

			vec3 local_diffuse = kD * albedo.xyz / PI;

			diffuse += local_diffuse * radiance * NdotL * occlusion_factor;
			specular += local_specular * radiance * NdotL * occlusion_factor;
		}

		diffuse *= ao_factor;

		imageStore(diffuse_map, pixel_coordinates, vec4(diffuse, albedo.a));
		imageStore(specular_map, pixel_coordinates, vec4(specular, 1.0));
		".trim();

		let m = root.get_mut("main").unwrap();

		match m.node_mut() {
			besl::parser::Nodes::Function { statements, .. } => {
				statements.insert(0, besl::parser::Node::glsl(a, &["vertex_uvs", "ao", "depth_shadow_map", "push_constant", "material_offset", "pixel_mapping", "material_count", "meshes", "meshlets", "materials", "primitive_indices", "vertex_indices", "vertex_positions", "vertex_normals", "triangle_index", "instance_index_render_target", "views", "calculate_full_bary", "interpolate_vec3f_with_deriv", "interpolate_vec2f_with_deriv", "fresnel_schlick", "distribution_ggx", "geometry_smith", "compute_vertex_index"], vec!["material".to_string(), "albedo".to_string(), "normal".to_string(), "roughness".to_string(), "metalness".to_string()]));
				statements.push(besl::parser::Node::glsl(b, &["lighting_data", "diffuse_map", "specular_map", "sample_shadow"], Vec::new()));
			}
			_ => {}
		}

		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root.add(extra);

		root
	}
}

#[cfg(test)]
mod tests {
    use crate::besl;

	// #[test]
	// fn vec4f_variable() {
	// 	let material = json::object! {
	// 		"variables": [
	// 			{
	// 				"name": "albedo",
	// 				"data_type": "vec4f",
	// 				"value": "Purple"
	// 			}
	// 		]
	// 	};

	// 	let shader_source = "main: fn () -> void { out_color = albedo; }";

	// 	let shader_node = besl::compile_to_besl(shader_source, None).unwrap();

	// 	let shader_generator = super::VisibilityShaderGenerator::new();

	// 	let shader = shader_generator.transform(&material, &shader_node, "Fragment").expect("Failed to generate shader");

	// 	// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	// }

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
