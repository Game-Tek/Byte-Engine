use std::rc::Rc;

use resource_management::resource::material_resource_handler::ProgramGenerator;

use crate::{rendering::{shader_strings::{CALCULATE_FULL_BARY, DISTRIBUTION_GGX, FRESNEL_SCHLICK, GEOMETRY_SMITH}, visibility_model::render_domain::{CAMERA_STRUCT_GLSL, LIGHTING_DATA_STRUCT_GLSL, LIGHT_STRUCT_GLSL, MATERIAL_STRUCT_GLSL, MESHLET_STRUCT_GLSL, MESH_STRUCT_GLSL}}, shader_generator};

pub struct VisibilityShaderGenerator {}

impl VisibilityShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform(&self, mut parent_children: Vec<Rc<jspd::lexer::Node>>) -> (&'static str, jspd::lexer::Node) {
		let value = json::object! {
			"type": "scope",
			"camera": {
				"type": "push_constant",
				"data_type": "Camera*"
			},
			"meshes": {
				"type": "push_constant",
				"data_type": "Mesh*"
			},
			"Camera": {
				"type": "struct",
				"view": {
					"type": "member",
					"data_type": "mat4f",
				},
				"projection": {
					"type": "member",
					"data_type": "mat4f",
				},
				"view_projection": {
					"type": "member",
					"data_type": "mat4f",
				}
			},
			"Mesh": {
				"type": "struct",
				"model": {
					"type": "member",
					"data_type": "mat4f",
				},
			},
			"Vertex": {
				"type": "scope",
				"__only_under": "Vertex",
				"in_position": {
					"type": "in",
					"data_type": "vec3f",
				},
				"in_normal": {
					"type": "in",
					"data_type": "vec3f",
				},
				"out_instance_index": {
					"type": "out",
					"data_type": "u32",
					"interpolation": "flat"
				},
			},
			"Fragment": {
				"type": "scope",
				"__only_under": "Fragment",
				"in_instance_index": {
					"type": "in",
					"data_type": "u32",
					"interpolation": "flat"
				},
				"out_color": {
					"type": "out",
					"data_type": "vec4f",
				}
			}
		};

		let mut node = jspd::json_to_jspd(&value).unwrap();

		if let jspd::lexer::Nodes::Scope { name, children, .. } = &mut node.node {
			children.append(&mut parent_children);
		};

// 		string.push_str(MESH_STRUCT_GLSL);

// 		string.push_str("
// layout(set=0, binding=1, scalar) buffer readonly MeshBuffer {
// 	Mesh meshes[];
// };

// layout(set=0, binding=2, scalar) buffer readonly Positions {
// 	vec3 positions[];
// };

// layout(set=0, binding=3, scalar) buffer readonly Normals {
// 	vec3 normals[];
// };

// layout(set=0, binding=4, scalar) buffer readonly VertexIndices {
// 	uint16_t vertex_indices[];
// };

// layout(set=0, binding=5, scalar) buffer readonly PrimitiveIndices {
// 	uint8_t primitive_indices[];
// };");

// 		string.push_str(MESHLET_STRUCT_GLSL);

// 		string.push_str("
// layout(set=0,binding=6,scalar) buffer readonly MeshletsBuffer {
// 	Meshlet meshlets[];
// };

// layout(set=0,binding=7) uniform sampler2D textures[1];

// layout(set=1,binding=0,scalar) buffer MaterialCount {
// 	uint material_count[];
// };

// layout(set=1,binding=1,scalar) buffer MaterialOffset {
// 	uint material_offset[];
// };

// layout(set=1,binding=4,scalar) buffer PixelMapping {
// 	u16vec2 pixel_mapping[];
// };

// layout(set=1, binding=6, r32ui) uniform readonly uimage2D triangle_index;
// layout(set=2, binding=0, rgba16) uniform image2D out_albedo;
// layout(set=2, binding=2, rgba16) uniform image2D out_diffuse;

// layout(set=2,binding=10) uniform sampler2D ao;
// layout(set=2,binding=11) uniform sampler2D depth_shadow_map;

// layout(push_constant, scalar) uniform PushConstant {
// 	uint material_id;
// } pc;");

// 		string.push_str(CAMERA_STRUCT_GLSL);
// 		string.push_str(LIGHT_STRUCT_GLSL);
// 		string.push_str(LIGHTING_DATA_STRUCT_GLSL);
// 		string.push_str(MATERIAL_STRUCT_GLSL);

// 		string.push_str("layout(set=2,binding=1,scalar) buffer CameraBuffer {
// 			Camera camera;
// 		};
		
// 		layout(set=2,binding=4,scalar) buffer readonly LightingBuffer {
// 			LightingData lighting_data;
// 		};
		
// 		layout(set=2,binding=5,scalar) buffer readonly MaterialsBuffer {
// 			Material materials[];
// 		};");

// 		string.push_str(DISTRIBUTION_GGX);
// 		string.push_str(GEOMETRY_SMITH);
// 		string.push_str(FRESNEL_SCHLICK);
// 		string.push_str(CALCULATE_FULL_BARY);

// 		string.push_str(&format!("layout(local_size_x=32) in;\n"));

// 		for variable in material["variables"].members() {
// 			match variable["data_type"].as_str().unwrap() {
// 				"vec4f" => { // Since GLSL doesn't support vec4f constants, we have to split it into 4 floats.
// 					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 0, "float", format!("{}_r", variable["name"]), "1.0"));
// 					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 1, "float", format!("{}_g", variable["name"]), "0.0"));
// 					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 2, "float", format!("{}_b", variable["name"]), "0.0"));
// 					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 3, "float", format!("{}_a", variable["name"]), "1.0"));
// 					string.push_str(&format!("const {} {} = {};\n", "vec4", variable["name"], format!("vec4({name}_r, {name}_g, {name}_b, {name}_a)", name=variable["name"])));
// 				}
// 				_ => {}
// 			}
// 		}

// string.push_str("
// void main() {
// 	if (gl_GlobalInvocationID.x >= material_count[pc.material_id]) { return; }

// 	uint offset = material_offset[pc.material_id];
// 	ivec2 pixel_coordinates = ivec2(pixel_mapping[offset + gl_GlobalInvocationID.x]);
// 	uint triangle_meshlet_indices = imageLoad(triangle_index, pixel_coordinates).r;
// 	uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
// 	uint meshlet_index = triangle_meshlet_indices >> 8;

// 	Meshlet meshlet = meshlets[meshlet_index];

// 	uint instance_index = meshlet.instance_index;

// 	Mesh mesh = meshes[instance_index];

// 	Material material = materials[pc.material_id];

// 	uint primitive_indices[3] = uint[3](
// 		primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 0],
// 		primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 1],
// 		primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 2]
// 	);

// 	uint vertex_indices[3] = uint[3](
// 		mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + primitive_indices[0]],
// 		mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + primitive_indices[1]],
// 		mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + primitive_indices[2]]
// 	);

// 	vec4 vertex_positions[3] = vec4[3](
// 		vec4(positions[vertex_indices[0]], 1.0),
// 		vec4(positions[vertex_indices[1]], 1.0),
// 		vec4(positions[vertex_indices[2]], 1.0)
// 	);

// 	vec4 vertex_normals[3] = vec4[3](
// 		vec4(normals[vertex_indices[0]], 0.0),
// 		vec4(normals[vertex_indices[1]], 0.0),
// 		vec4(normals[vertex_indices[2]], 0.0)
// 	);

// 	vec2 image_extent = imageSize(triangle_index);

// 	vec2 uv = pixel_coordinates / image_extent;

// 	vec2 nc = uv * 2 - 1;

// 	vec4 clip_space_vertex_positions[3] = vec4[3](camera.view_projection * mesh.model * vertex_positions[0], camera.view_projection * mesh.model * vertex_positions[1], camera.view_projection * mesh.model * vertex_positions[2]);

// 	BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, image_extent);
// 	vec3 barycenter = barycentric_deriv.lambda;

// 	vec3 vertex_position = vec3((mesh.model * vertex_positions[0]).xyz * barycenter.x + (mesh.model * vertex_positions[1]).xyz * barycenter.y + (mesh.model * vertex_positions[2]).xyz * barycenter.z);
// 	vec3 vertex_normal = vec3((vertex_normals[0]).xyz * barycenter.x + (vertex_normals[1]).xyz * barycenter.y + (vertex_normals[2]).xyz * barycenter.z);

// 	vec3 N = normalize(vertex_normal);
// 	vec3 V = normalize(-(camera.view[3].xyz - vertex_position));

// 	vec3 albedo = vec3(1, 0, 0);
// 	vec3 metalness = vec3(0);
// 	float roughness = float(0.5);
// ");

// 		fn visit_node(string: &mut String, shader_node: &jspd::Node, material: &json::JsonValue) {
// 			let variable_names = material["variables"].members().map(|variable| variable["name"].as_str().unwrap()).collect::<Vec<_>>();

					// if variable_names.contains(&name.as_str()) {
					// 	if material["variables"].members().find(|variable| variable["name"].as_str().unwrap() == name.as_str()).unwrap()["data_type"].as_str().unwrap() == "Texture2D" {
					// 		let mut variables = material["variables"].members().filter(|variable| variable["data_type"].as_str().unwrap() == "Texture2D").collect::<Vec<_>>();

					// 		variables.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

					// 		let index = variables.iter().position(|variable| variable["name"].as_str().unwrap() == name.as_str()).unwrap();

					// 		string.push_str(&format!("textures[nonuniformEXT(material.textures[{}])]", index));
					// 	} else {
					// 		string.push_str(name);
					// 	}
					// } else {
					// 	string.push_str(name);
					// }
// 		}

// 		visit_node(&mut string, shader_node, material);

// string.push_str(&format!("
// 	vec3 lo = vec3(0.0);
// 	vec3 diffuse = vec3(0.0);

// 	float ao_factor = texture(ao, uv).r;

// 	for (uint i = 0; i < lighting_data.light_count; ++i) {{
// 		vec3 light_pos = lighting_data.lights[i].position;
// 		vec3 light_color = lighting_data.lights[i].color;
// 		mat4 light_matrix = lighting_data.lights[i].vp_matrix;
// 		uint8_t light_type = lighting_data.lights[i].light_type;

// 		vec3 L = vec3(0.0);

// 		if (light_type == 68) {{ // Infinite
// 			L = normalize(light_pos);
// 		}} else {{
// 			L = normalize(light_pos - vertex_position);
// 		}}

// 		float NdotL = max(dot(N, L), 0.0);

// 		if (NdotL <= 0.0) {{
// 			continue;
// 		}}

// 		float occlusion_factor = 1.0;
// 		float attenuation = 1.0;

// 		if (light_type == 68) {{ // Infinite
// 			vec4 surface_light_clip_position = light_matrix * vec4(vertex_position + N * 0.001, 1.0);
// 			vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
// 			vec2 shadow_uv = surface_light_ndc_position.xy * 0.5 + 0.5;
// 			float z = surface_light_ndc_position.z;
// 			float shadow_sample_depth = texture(depth_shadow_map, shadow_uv).r;
// 			float occlusion_factor = z < shadow_sample_depth ? 0.0 : 1.0;

// 			if (occlusion_factor == 0.0) {{
// 				continue;
// 			}}

// 			attenuation = 1.0;
// 		}} else {{
// 			float distance = length(light_pos - vertex_position);
// 			attenuation = 1.0 / (distance * distance);
// 		}}

// 		vec3 H = normalize(V + L);

// 		vec3 radiance = light_color * attenuation;

// 		vec3 F0 = vec3(0.04);
// 		F0 = mix(F0, albedo, metalness);
// 		vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);

// 		float NDF = distribution_ggx(N, H, roughness);
// 		float G = geometry_smith(N, V, L, roughness);
// 		vec3 specular = (NDF * G * F) / (4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.000001);

// 		vec3 kS = F;
// 		vec3 kD = vec3(1.0) - kS;

// 		kD *= 1.0 - metalness;

// 		vec3 local_diffuse = kD * albedo / PI;

// 		lo += (local_diffuse + specular) * radiance * NdotL * occlusion_factor;
// 		diffuse += local_diffuse;
// 	}};

// 	lo *= ao_factor;
// "));

// 		string.push_str(&format!("imageStore(out_albedo, pixel_coordinates, vec4(lo, 1.0));"));
// 		string.push_str(&format!("imageStore(out_diffuse, pixel_coordinates, vec4(diffuse, 1.0));"));

// 		string.push_str(&format!("\n}}")); // Close main()

		("Visibility", node)
	}
}

impl VisibilityShaderGenerator {
	/// Produce a GLSL shader string from a BESL shader node.
	/// This returns an option since for a given input stage the visibility shader generator may not produce any output.

	fn fragment_transform(&self, material: &json::JsonValue, shader_node: &jspd::lexer::Node) -> String {
		let mut string = shader_generator::generate_glsl_header_block(&shader_generator::ShaderGenerationSettings::new("Compute"));



		string
	}
}

#[cfg(test)]
mod tests {
    use crate::jspd;

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

		let shader_node = jspd::compile_to_jspd(shader_source, None).unwrap();

		let shader_generator = super::VisibilityShaderGenerator::new();

		let shader = shader_generator.transform(&material, &shader_node, "Fragment").expect("Failed to generate shader");

		// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	}

	#[test]
	fn multiple_textures() {
		let material = json::object! {
			"variables": [
				{
					"name": "albedo",
					"data_type": "Texture2D",
				},
				{
					"name": "normal",
					"data_type": "Texture2D",
				}
			]
		};

		let shader_source = "main: fn () -> void { out_color = sample(albedo); }";

		let shader_node = jspd::compile_to_jspd(shader_source, None).unwrap();

		let shader_generator = super::VisibilityShaderGenerator::new();

		let shader = shader_generator.transform(&material, &shader_node, "Fragment").expect("Failed to generate shader");

		// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	}
}