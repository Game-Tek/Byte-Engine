use std::{cell::RefCell, ops::Deref, rc::Rc};

use besl::{Node, NodeReference};
use maths_rs::vec;
use resource_management::asset::material_asset_handler::ProgramGenerator;

use super::common_shader_generator::CommonShaderGenerator;

pub struct VisibilityShaderGenerator {
	out_albedo: besl::parser::Node,
	camera: besl::parser::Node,
	out_diffuse: besl::parser::Node,
	lighting_data: besl::parser::Node,
	materials: besl::parser::Node,
	ao: besl::parser::Node,
	depth_shadow_map: besl::parser::Node,
	push_constant: besl::parser::Node,
	sample_function: besl::parser::Node,
	sample_normal_function: besl::parser::Node,
}

impl VisibilityShaderGenerator {
	pub fn new(scope: Node) -> Self {
		use besl::parser::Node;

		let set2_binding0 = Node::binding("out_albedo", Node::image("rgba16"), 2, 0, false, true);
		let set2_binding1 = Node::binding("camera", Node::buffer("CameraBuffer", vec![Node::member("camera", "Camera")]), 2, 1, true, false);
		let set2_binding2 = Node::binding("out_diffuse", Node::image("rgba16"), 2, 2, false, true);
		let set2_binding4 = Node::binding("lighting_data", Node::buffer("LightingBuffer", vec![Node::member("light_count", "u32"), Node::member("lights", "Light[16]")]), 2, 4, true, false);
		let set2_binding5 = Node::binding("materials", Node::buffer("MaterialBuffer", vec![Node::member("materials", "Material[16]")]), 2, 5, true, false);
		let set2_binding10 = Node::binding("ao", Node::combined_image_sampler(), 2, 10, true, false);
		let set2_binding11 = Node::binding("depth_shadow_map", Node::combined_image_sampler(), 2, 11, true, false);

		let push_constant = Node::push_constant(vec![Node::member("material_id", "u32")]);
		
		let sample_function = Node::intrinsic("sample", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("texture(", Vec::new(), Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv).rgb", Vec::new(), Vec::new())]), "vec3f");
		

		let sample_normal_function = if true {
			Node::intrinsic("sample_normal", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("unit_vector_from_xy(texture(", Vec::new(), Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv).xy)", vec!["unit_vector_from_xy".to_string()], Vec::new())]), "vec3f")
		} else {
			Node::intrinsic("sample_normal", Node::parameter("smplr", "u32"), Node::sentence(vec![Node::glsl("normalize(texture(", Vec::new(), Vec::new()), Node::member_expression("smplr"), Node::glsl(", vertex_uv).xyz * 2.0f - 1.0f)", vec![], Vec::new())]), "vec3f")
		};

		Self {
			out_albedo: set2_binding0,
			camera: set2_binding1,
			out_diffuse: set2_binding2,
			lighting_data: set2_binding4,
			materials: set2_binding5,
			ao: set2_binding10,
			depth_shadow_map: set2_binding11,
			push_constant,
			sample_function,
			sample_normal_function,
		}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, material: &json::JsonValue) -> besl::parser::Node {
		let set2_binding0 = self.out_albedo.clone();
		let set2_binding1 = self.camera.clone();
		let set2_binding2 = self.out_diffuse.clone();
		let set2_binding4 = self.lighting_data.clone();
		let set2_binding5 = self.materials.clone();
		let set2_binding10 = self.ao.clone();
		let set2_binding11 = self.depth_shadow_map.clone();
		let push_constant = self.push_constant.clone();

		let a = "if (gl_GlobalInvocationID.x >= material_count.material_count[push_constant.material_id]) { return; }
		
uint offset = material_offset.material_offset[push_constant.material_id];
ivec2 pixel_coordinates = ivec2(pixel_mapping.pixel_mapping[offset + gl_GlobalInvocationID.x]);
uint triangle_meshlet_indices = imageLoad(triangle_index, pixel_coordinates).r;
uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
uint meshlet_index = triangle_meshlet_indices >> 8;

Meshlet meshlet = meshlets.meshlets[meshlet_index];

uint instance_index = meshlet.instance_index;

Mesh mesh = meshes.meshes[instance_index];

Material material = materials.materials[push_constant.material_id];

uint primitive_indices[3] = uint[3](
	primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.primitive_triangle_offset + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 0],
	primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.primitive_triangle_offset + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 1],
	primitive_indices.primitive_indices[(mesh.base_triangle_index + meshlet.primitive_triangle_offset + meshlet.triangle_offset + meshlet_triangle_index) * 3 + 2]
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

Camera camera = camera.camera;

vec4 world_space_vertex_positions[3] = vec4[3](mesh.model * model_space_vertex_positions[0], mesh.model * model_space_vertex_positions[1], mesh.model * model_space_vertex_positions[2]);
vec4 clip_space_vertex_positions[3] = vec4[3](camera.view_projection * world_space_vertex_positions[0], camera.view_projection * world_space_vertex_positions[1], camera.view_projection * world_space_vertex_positions[2]);

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
vec3 V = normalize(-(camera.view[3].xyz - world_space_vertex_position));

vec3 pos_dx = interpolate_vec3f_with_deriv(ddx, model_space_vertex_positions[0].xyz, model_space_vertex_positions[1].xyz, model_space_vertex_positions[2].xyz);
vec3 pos_dy = interpolate_vec3f_with_deriv(ddy, model_space_vertex_positions[0].xyz, model_space_vertex_positions[1].xyz, model_space_vertex_positions[2].xyz);

vec2 uv_dx = interpolate_vec2f_with_deriv(ddx, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);
vec2 uv_dy = interpolate_vec2f_with_deriv(ddy, vertex_uvs[0], vertex_uvs[1], vertex_uvs[2]);

// // Method 1
float f = 1.0 / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
vec3 T = normalize(f * (uv_dy.y * pos_dx - uv_dx.y * pos_dy));
vec3 B = normalize(f * (-uv_dy.x * pos_dx + uv_dx.x * pos_dy));
mat3 TBN = mat3(T, B, N);

// Method 2
// vec3 T = (uv_dy.y * pos_dx - uv_dx.y * pos_dy) / (uv_dx.x * uv_dy.y - uv_dy.x * uv_dx.y);
// T = normalize(T - N * dot(T, N));
// vec3 B = cross(N, T);
// mat3 TBN = mat3(T, B, N);

vec3 albedo = vec3(1, 0, 0);
vec3 normal = vec3(0, 0, 1);
float metalness = 0;
float roughness = float(0.5);";

		let mut extra = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].members() {
			let name = variable["name"].as_str().unwrap();
			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4" => {
					let x = besl::parser::Node::specialization(name, data_type);
					extra.push(x);
				}
				"Texture2D" => {
					let x = besl::parser::Node::literal(name, besl::parser::Node::glsl(&format!("textures[nonuniformEXT(material.textures[{}])]", texture_count), vec![/* TODO: fix literals "material".to_string(), */"textures".to_string()], Vec::new()));
					extra.push(x);
					texture_count += 1;
				}
				_ => {}
			}
		}

		let b = "
vec3 lo = vec3(0.0);
vec3 diffuse = vec3(0.0);

float ao_factor = texture(ao, normalized_xy).r;

normal = normalize(TBN * normal);

for (uint i = 0; i < lighting_data.light_count; ++i) {
	vec3 light_pos = lighting_data.lights[i].position;
	vec3 light_color = lighting_data.lights[i].color;
	mat4 light_matrix = lighting_data.lights[i].view_projection;
	uint8_t light_type = lighting_data.lights[i].light_type;

	vec3 L = vec3(0.0);

	if (light_type == 68) { // Infinite
		L = normalize(-light_pos);
	} else {
		L = normalize(light_pos - world_space_vertex_position);
	}

	float NdotL = max(dot(normal, L), 0.0);

	if (NdotL <= 0.0) { continue; }

	float occlusion_factor = 1.0;
	float attenuation = 1.0;

	if (light_type == 68) { // Infinite
		vec4 surface_light_clip_position = light_matrix * vec4(world_space_vertex_position + normal * 0.001, 1.0);
		vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
		vec2 shadow_uv = surface_light_ndc_position.xy * 0.5 + 0.5;
		float z = surface_light_ndc_position.z;
		float shadow_sample_depth = texture(depth_shadow_map, shadow_uv).r;
		float occlusion_factor = z < shadow_sample_depth ? 0.0 : 1.0;

		if (occlusion_factor == 0.0) { continue; }

		attenuation = 1.0;
	} else {
		float distance = length(light_pos - world_space_vertex_position);
		attenuation = 1.0 / (distance * distance);
	}

	vec3 H = normalize(V + L);

	vec3 radiance = light_color * attenuation;

	vec3 F0 = vec3(0.04);
	F0 = mix(F0, albedo, metalness);
	vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

	float NDF = distribution_ggx(normal, H, roughness);
	float G = geometry_smith(normal, V, L, roughness);
	vec3 specular = (NDF * G * F) / (4.0 * max(dot(normal, V), 0.0) * max(dot(normal, L), 0.0) + 0.000001);

	vec3 kS = F;
	vec3 kD = vec3(1.0) - kS;

	kD *= 1.0 - metalness;

	vec3 local_diffuse = kD * albedo / PI;

	lo += (local_diffuse + specular) * radiance * NdotL * occlusion_factor;
	diffuse += local_diffuse;
}

lo *= ao_factor;

imageStore(out_albedo, pixel_coordinates, vec4(lo, 1.0));
imageStore(out_diffuse, pixel_coordinates, vec4(diffuse, 1.0));";

		let push_constant = self.push_constant.clone();

		let lighting_data = self.lighting_data.clone();
		let out_albedo = self.out_albedo.clone();
		let out_diffuse = self.out_diffuse.clone();

		let common_shader_generator = CommonShaderGenerator::new();

		root = common_shader_generator.transform(root, material);

		let m = root.get_mut("main").unwrap();

		match m.node_mut() {
			besl::parser::Nodes::Function { statements, .. } => {
				statements.insert(0, besl::parser::Node::glsl(a, vec!["vertex_uvs".to_string(), "ao".to_string(), "depth_shadow_map".to_string(), "push_constant".to_string(), "material_offset".to_string(), "pixel_mapping".to_string(), "material_count".to_string(), "meshes".to_string(), "meshlets".to_string(), "materials".to_string(), "primitive_indices".to_string(), "vertex_indices".to_string(), "vertex_positions".to_string(), "vertex_normals".to_string(), "triangle_index".to_string(), "camera".to_string(), "calculate_full_bary".to_string(), "interpolate_vec3f_with_deriv".to_string(), "interpolate_vec2f_with_deriv".to_string(), "fresnel_schlick".to_string(), "distribution_ggx".to_string(), "geometry_smith".to_string(), "compute_vertex_index".to_string()], vec!["material".to_string(), "albedo".to_string(), "normal".to_string(), "roughness".to_string(), "metalness".to_string()]));
				statements.push(besl::parser::Node::glsl(b, vec!["lighting_data".to_string(), "out_albedo".to_string(), "out_diffuse".to_string()], Vec::new()));
			}
			_ => {}
		}

		root.add(vec![self.lighting_data.clone(), push_constant, set2_binding11, set2_binding1, set2_binding5, set2_binding10, lighting_data, out_albedo, out_diffuse, self.sample_function.clone(), self.sample_normal_function.clone()]);
		root.add(extra);
		root.sort(); // TODO: we have to sort the nodes because the order of the nodes is important for the generated shader, ideally the lexer should be able to handle this

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