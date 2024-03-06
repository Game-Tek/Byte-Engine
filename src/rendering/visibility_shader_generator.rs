use std::{cell::RefCell, rc::Rc};

use jspd::NodeReference;
use maths_rs::vec;
use resource_management::resource::material_resource_handler::ProgramGenerator;

use crate::{rendering::{shader_strings::{CALCULATE_FULL_BARY, DISTRIBUTION_GGX, FRESNEL_SCHLICK, GEOMETRY_SMITH}, visibility_model::render_domain::{CAMERA_STRUCT_GLSL, LIGHTING_DATA_STRUCT_GLSL, LIGHT_STRUCT_GLSL, MATERIAL_STRUCT_GLSL, MESHLET_STRUCT_GLSL, MESH_STRUCT_GLSL}}, shader_generator};

pub struct VisibilityShaderGenerator {
	mesh_struct: jspd::NodeReference,
	camera_struct: jspd::NodeReference,
	meshlet_struct: jspd::NodeReference,
	light_struct: jspd::NodeReference,
	material_struct: jspd::NodeReference,
	lighting_data_struct: jspd::NodeReference,
	meshes: jspd::NodeReference,
	positions: jspd::NodeReference,
	normals: jspd::NodeReference,
	vertex_indices: jspd::NodeReference,
	primitive_indices: jspd::NodeReference,
	meshlets: jspd::NodeReference,
	textures: jspd::NodeReference,
	material_count: jspd::NodeReference,
	material_offset: jspd::NodeReference,
	pixel_mapping: jspd::NodeReference,
	triangle_index: jspd::NodeReference,
	out_albedo: jspd::NodeReference,
	camera: jspd::NodeReference,
	out_diffuse: jspd::NodeReference,
	lighting_data: jspd::NodeReference,
	materials: jspd::NodeReference,
	ao: jspd::NodeReference,
	depth_shadow_map: jspd::NodeReference,
	push_constant: jspd::NodeReference,
	distribution_ggx: jspd::NodeReference,
	geometry_schlick_ggx: jspd::NodeReference,
	geometry_smith: jspd::NodeReference,
	fresnel_schlick: jspd::NodeReference,
	barycentric_deriv: jspd::NodeReference,
	calculate_full_bary: jspd::NodeReference,
}

impl VisibilityShaderGenerator {
	pub fn new(scope: NodeReference) -> Self {
		let void = RefCell::borrow(&scope).get_child("void").unwrap();
		let vec3f = RefCell::borrow(&scope).get_child("vec3f").unwrap();
		let mat4f = RefCell::borrow(&scope).get_child("mat4f32").unwrap();
		let float32_t = RefCell::borrow(&scope).get_child("f32").unwrap();
		let uint8_t = RefCell::borrow(&scope).get_child("u8").unwrap();
		let uint16_t = RefCell::borrow(&scope).get_child("u16").unwrap();
		let uint32_t = RefCell::borrow(&scope).get_child("u32").unwrap();
		let vec2u16 = RefCell::borrow(&scope).get_child("vec2u16").unwrap();

		let mesh_struct = jspd::Node::r#struct("Mesh", vec![jspd::Node::member("model", mat4f.clone()), jspd::Node::member("material_index", uint32_t.clone()), jspd::Node::member("base_vertex_index", uint32_t.clone())]);
		let camera_struct = jspd::Node::r#struct("Camera", vec![jspd::Node::member("view", mat4f.clone()), jspd::Node::member("projection_matrix", mat4f.clone()), jspd::Node::member("view_projection", mat4f.clone()), jspd::Node::member("inverse_view_matrix", mat4f.clone()), jspd::Node::member("inverse_projection_matrix", mat4f.clone()), jspd::Node::member("inverse_view_projection_matrix", mat4f.clone())]);
		let meshlet_struct = jspd::Node::r#struct("Meshlet", vec![jspd::Node::member("instance_index", uint32_t.clone()), jspd::Node::member("vertex_offset", uint16_t.clone()), jspd::Node::member("triangle_offset", uint16_t.clone()), jspd::Node::member("vertex_count", uint8_t.clone()), jspd::Node::member("triangle_count", uint8_t.clone())]);
		let light_struct = jspd::Node::r#struct("Light", vec![jspd::Node::member("view_matrix", mat4f.clone()), jspd::Node::member("projection_matrix", mat4f.clone()), jspd::Node::member("vp_matrix", mat4f.clone()), jspd::Node::member("position", vec3f.clone()), jspd::Node::member("color", vec3f.clone()), jspd::Node::member("light_type", uint8_t.clone())]);
		let material_struct = jspd::Node::r#struct("Material", vec![jspd::Node::member("textures", mat4f.clone())]);
		let lighting_data_struct = jspd::Node::r#struct("LightingData", vec![jspd::Node::member("light_count", uint32_t.clone()), jspd::Node::array("lights", light_struct.clone(), 16)]);

		let set0_binding1 = jspd::Node::binding("mesh_buffer", jspd::BindingTypes::buffer(jspd::Node::r#struct("MeshBuffer", vec![jspd::Node::array("meshes", mesh_struct.clone(), 64)])), 0, 1, true, false);
		let set0_binding2 = jspd::Node::binding("positions", jspd::BindingTypes::buffer(jspd::Node::r#struct("Positions", vec![jspd::Node::array("positions", vec3f.clone(), 8192)])), 0, 2, true, false);
		let set0_binding3 = jspd::Node::binding("normals", jspd::BindingTypes::buffer(jspd::Node::r#struct("Normals", vec![jspd::Node::array("normals", vec3f.clone(), 8192)])), 0, 3, true, false);
		let set0_binding4 = jspd::Node::binding("vertex_indices", jspd::BindingTypes::buffer(jspd::Node::r#struct("VertexIndices", vec![jspd::Node::array("vertex_indices", uint16_t.clone(), 8192)])), 0, 4, true, false);
		let set0_binding5 = jspd::Node::binding("primitive_indices", jspd::BindingTypes::buffer(jspd::Node::r#struct("PrimitiveIndices", vec![jspd::Node::array("primitive_indices", uint8_t.clone(), 8192)])), 0, 5, true, false);
		let set0_binding6 = jspd::Node::binding("meshlets", jspd::BindingTypes::buffer(jspd::Node::r#struct("MeshletsBuffer", vec![jspd::Node::array("meshlets", meshlet_struct.clone(), 8192)])), 0, 6, true, false);
		let set0_binding7 = jspd::Node::binding_array("textures", jspd::BindingTypes::CombinedImageSampler, 0, 7, true, false, 16);

		let set1_binding0 = jspd::Node::binding("material_count", jspd::BindingTypes::buffer(jspd::Node::r#struct("MaterialCount", vec![jspd::Node::array("material_count", uint32_t.clone(), 1920 * 1080)])), 1, 0, true, false);
		let set1_binding1 = jspd::Node::binding("material_offset", jspd::BindingTypes::buffer(jspd::Node::r#struct("MaterialOffset", vec![jspd::Node::array("material_offset", uint32_t.clone(), 1920 * 1080)])), 1, 1, true, false);
		let set1_binding4 = jspd::Node::binding("pixel_mapping", jspd::BindingTypes::buffer(jspd::Node::r#struct("PixelMapping", vec![jspd::Node::array("pixel_mapping", vec2u16.clone(), 1920 * 1080)])), 1, 4, true, false);
		let set1_binding6 = jspd::Node::binding("triangle_index", jspd::BindingTypes::Image{ format: "r32ui".to_string() }, 1, 6, true, false);

		let set2_binding0 = jspd::Node::binding("out_albedo", jspd::BindingTypes::Image{ format: "rgba16".to_string() }, 2, 0, false, true);
		let set2_binding1 = jspd::Node::binding("camera", jspd::BindingTypes::buffer(jspd::Node::r#struct("CameraBuffer", vec![jspd::Node::member("camera", camera_struct.clone())])), 2, 1, true, false);
		let set2_binding2 = jspd::Node::binding("out_diffuse", jspd::BindingTypes::Image{ format: "rgba16".to_string() }, 2, 2, false, true);
		let set2_binding4 = jspd::Node::binding("lighting_data", jspd::BindingTypes::buffer(jspd::Node::r#struct("LightingBuffer", vec![jspd::Node::member("lighting_data", lighting_data_struct.clone())])), 2, 4, true, false);
		let set2_binding5 = jspd::Node::binding("material_buffer", jspd::BindingTypes::buffer(jspd::Node::r#struct("MaterialBuffer", vec![jspd::Node::member("materials", material_struct.clone())])), 2, 5, true, false);
		let set2_binding10 = jspd::Node::binding("ao", jspd::BindingTypes::CombinedImageSampler, 2, 10, true, false);
		let set2_binding11 = jspd::Node::binding("depth_shadow_map", jspd::BindingTypes::CombinedImageSampler, 2, 11, true, false);

		let push_constant = jspd::Node::push_constant(vec![jspd::Node::member("material_id", uint32_t.clone())]);

		let distribution_ggx = jspd::Node::function("distribution_ggx", vec![jspd::Node::member("n", vec3f.clone()), jspd::Node::member("h", vec3f.clone()), jspd::Node::member("roughness", float32_t.clone())], float32_t.clone(), vec![], Some("float a = roughness*roughness; float a2 = a*a; float n_dot_h = max(dot(n, h), 0.0); float denom = ((n_dot_h*n_dot_h) * (a2 - 1.0) + 1.0); denom = PI * denom * denom; return a2 / denom;".to_string()));
		let geometry_schlick_ggx = jspd::Node::function("geometry_schlick_ggx", vec![jspd::Node::member("n_dot_v", float32_t.clone()), jspd::Node::member("roughness", float32_t.clone())], float32_t.clone(), vec![], Some("float r = (roughness + 1.0); float k = (r*r) / 8.0; return n_dot_v / (n_dot_v * (1.0 - k) + k);".to_string()));
		let geometry_smith = jspd::Node::function("geometry_smith", vec![jspd::Node::member("n", vec3f.clone()), jspd::Node::member("v", vec3f.clone()), jspd::Node::member("l", vec3f.clone()), jspd::Node::member("roughness", float32_t.clone())], float32_t.clone(), vec![], Some("return geometry_schlick_ggx(max(dot(n, v), 0.0), roughness) * geometry_schlick_ggx(max(dot(n, l), 0.0), roughness);".to_string()));
		let fresnel_schlick = jspd::Node::function("fresnel_schlick", vec![jspd::Node::member("cos_theta", float32_t.clone()), jspd::Node::member("f0", vec3f.clone())], vec3f.clone(), vec![], Some("return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);".to_string()));
		
		let barycentric_deriv = jspd::Node::r#struct("BarycentricDeriv", vec![jspd::Node::member("lambda", vec3f.clone()), jspd::Node::member("ddx", vec3f.clone()), jspd::Node::member("ddy", vec3f.clone())]);

		let calculate_full_bary = jspd::Node::function("calculate_full_bary", vec![], barycentric_deriv.clone(), vec![], Some("BarycentricDeriv ret = BarycentricDeriv(vec3(0), vec3(0), vec3(0)); vec3 invW = 1.0 / vec3(pt0.w, pt1.w, pt2.w); vec2 ndc0 = pt0.xy * invW.x; vec2 ndc1 = pt1.xy * invW.y; vec2 ndc2 = pt2.xy * invW.z; float invDet = 1.0 / determinant(mat2(ndc2 - ndc1, ndc0 - ndc1)); ret.ddx = vec3(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet * invW; ret.ddy = vec3(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet * invW; float ddxSum = dot(ret.ddx, vec3(1)); float ddySum = dot(ret.ddy, vec3(1)); vec2 deltaVec = pixelNdc - ndc0; float interpInvW = invW.x + deltaVec.x * ddxSum + deltaVec.y * ddySum; float interpW = 1.0 / interpInvW; ret.lambda.x = interpW * (invW.x + deltaVec.x * ret.ddx.x + deltaVec.y * ret.ddy.x); ret.lambda.y = interpW * (0.0    + deltaVec.x * ret.ddx.y + deltaVec.y * ret.ddy.y); ret.lambda.z = interpW * (0.0    + deltaVec.x * ret.ddx.z + deltaVec.y * ret.ddy.z); ret.ddx *= (2.0 / winSize.x); ret.ddy *= (2.0 / winSize.y); ddxSum  *= (2.0 / winSize.x); ddySum  *= (2.0 / winSize.y);  float interpW_ddx = 1.0 / (interpInvW + ddxSum); float interpW_ddy = 1.0 / (interpInvW + ddySum);  ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda; ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda; return ret;".to_string()));
		
		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,
			lighting_data_struct,
			meshes: set0_binding1,
			positions: set0_binding2,
			normals: set0_binding3,
			vertex_indices: set0_binding4,
			primitive_indices: set0_binding5,
			meshlets: set0_binding6,
			textures: set0_binding7,
			material_count: set1_binding0,
			material_offset: set1_binding1,
			pixel_mapping: set1_binding4,
			triangle_index: set1_binding6,
			out_albedo: set2_binding0,
			camera: set2_binding1,
			out_diffuse: set2_binding2,
			lighting_data: set2_binding4,
			materials: set2_binding5,
			ao: set2_binding10,
			depth_shadow_map: set2_binding11,
			push_constant,
			distribution_ggx,
			geometry_schlick_ggx,
			geometry_smith,
			fresnel_schlick,
			barycentric_deriv,
			calculate_full_bary,	
		}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn pre_transform(&self, scope: jspd::NodeReference) -> jspd::NodeReference {
		let mesh_struct = self.mesh_struct.clone();
		let camera_struct = self.camera_struct.clone();
		let meshlet_struct = self.meshlet_struct.clone();
		let light_struct = self.light_struct.clone();
		let material_struct = self.material_struct.clone();
		let lighting_data_struct = self.lighting_data_struct.clone();
		let set0_binding1 = self.meshes.clone();
		let set0_binding2 = self.positions.clone();
		let set0_binding3 = self.normals.clone();
		let set0_binding4 = self.vertex_indices.clone();
		let set0_binding5 = self.primitive_indices.clone();
		let set0_binding6 = self.meshlets.clone();
		let set0_binding7 = self.textures.clone();
		let set1_binding0 = self.material_count.clone();
		let set1_binding1 = self.material_offset.clone();
		let set1_binding4 = self.pixel_mapping.clone();
		let set1_binding6 = self.triangle_index.clone();
		let set2_binding0 = self.out_albedo.clone();
		let set2_binding1 = self.camera.clone();
		let set2_binding2 = self.out_diffuse.clone();
		let set2_binding4 = self.lighting_data.clone();
		let set2_binding5 = self.materials.clone();
		let set2_binding10 = self.ao.clone();
		let set2_binding11 = self.depth_shadow_map.clone();
		let push_constant = self.push_constant.clone();
		let distribution_ggx = self.distribution_ggx.clone();
		let geometry_schlick_ggx = self.geometry_schlick_ggx.clone();
		let geometry_smith = self.geometry_smith.clone();
		let frshnel_schlick = self.fresnel_schlick.clone();
		let calculate_full_bary = self.calculate_full_bary.clone();

		RefCell::borrow_mut(&scope).add_children(vec![mesh_struct, camera_struct, meshlet_struct, light_struct, material_struct, lighting_data_struct, set0_binding1, set0_binding2, set0_binding3, set0_binding4, set0_binding5, set0_binding6, set0_binding7, set1_binding0, set1_binding1, set1_binding4, set1_binding6, set2_binding0, set2_binding1, set2_binding2, set2_binding4, set2_binding5, set2_binding10, set2_binding11, push_constant, distribution_ggx, geometry_schlick_ggx, geometry_smith, frshnel_schlick, calculate_full_bary]);

		scope
	}

	fn post_transform(&self, main_function_node: jspd::NodeReference) -> jspd::NodeReference {
		let a = "if (gl_GlobalInvocationID.x >= material_count[push_constant.material_id]) { return; }
		
			uint offset = material_offset[push_constant.material_id];
			ivec2 pixel_coordinates = ivec2(pixel_mapping[offset + gl_GlobalInvocationID.x]);
			uint triangle_meshlet_indices = imageLoad(triangle_index, pixel_coordinates).r;
			uint meshlet_triangle_index = triangle_meshlet_indices & 0xFF;
			uint meshlet_index = triangle_meshlet_indices >> 8;
		
			Meshlet meshlet = meshlets[meshlet_index];
		
			uint instance_index = meshlet.instance_index;
		
			Mesh mesh = meshes[instance_index];
		
			Material material = materials[push_constant.material_id];
		
			uint primitive_indices[3] = uint[3](
				primitive_indices.primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 0],
				primitive_indices.primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 1],
				primitive_indices.primitive_indices[(meshlet.triangle_offset + meshlet_triangle_index) * 3 + 2]
			);
		
			uint vertex_indices[3] = uint[3](
				mesh.base_vertex_index + vertex_indices.vertex_indices[meshlet.vertex_offset + primitive_indices[0]],
				mesh.base_vertex_index + vertex_indices.vertex_indices[meshlet.vertex_offset + primitive_indices[1]],
				mesh.base_vertex_index + vertex_indices.vertex_indices[meshlet.vertex_offset + primitive_indices[2]]
			);
		
			vec4 vertex_positions[3] = vec4[3](
				vec4(positions[vertex_indices[0]], 1.0),
				vec4(positions[vertex_indices[1]], 1.0),
				vec4(positions[vertex_indices[2]], 1.0)
			);
		
			vec4 vertex_normals[3] = vec4[3](
				vec4(normals[vertex_indices[0]], 0.0),
				vec4(normals[vertex_indices[1]], 0.0),
				vec4(normals[vertex_indices[2]], 0.0)
			);
		
			vec2 image_extent = imageSize(triangle_index);
		
			vec2 uv = pixel_coordinates / image_extent;
		
			vec2 nc = uv * 2 - 1;
		
			vec4 clip_space_vertex_positions[3] = vec4[3](camera.view_projection * mesh.model * vertex_positions[0], camera.view_projection * mesh.model * vertex_positions[1], camera.view_projection * mesh.model * vertex_positions[2]);
		
			BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, image_extent);
			vec3 barycenter = barycentric_deriv.lambda;
		
			vec3 vertex_position = vec3((mesh.model * vertex_positions[0]).xyz * barycenter.x + (mesh.model * vertex_positions[1]).xyz * barycenter.y + (mesh.model * vertex_positions[2]).xyz * barycenter.z);
			vec3 vertex_normal = vec3((vertex_normals[0]).xyz * barycenter.x + (vertex_normals[1]).xyz * barycenter.y + (vertex_normals[2]).xyz * barycenter.z);
		
			vec3 N = normalize(vertex_normal);
			vec3 V = normalize(-(camera.view[3].xyz - vertex_position));
		
			vec3 albedo = vec3(1, 0, 0);
			vec3 metalness = vec3(0);
			float roughness = float(0.5);
		";

		// "textures[nonuniformEXT(material.textures[{}])]"
		// for variable in material["variables"].members() {
		// 	match variable["data_type"].as_str().unwrap() {
		// 		"vec4f" => { // Since GLSL doesn't support vec4f constants, we have to split it into 4 floats.
		// 			string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 0, "float", format!("{}_r", variable["name"]), "1.0"));
		// 			string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 1, "float", format!("{}_g", variable["name"]), "0.0"));
		// 			string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 2, "float", format!("{}_b", variable["name"]), "0.0"));
		// 			string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 3, "float", format!("{}_a", variable["name"]), "1.0"));
		// 			string.push_str(&format!("const {} {} = {};\n", "vec4", variable["name"], format!("vec4({name}_r, {name}_g, {name}_b, {name}_a)", name=variable["name"])));
		// 		}
		// 		_ => {}
		// 	}
		// }

		// string.push_str(&format!("layout(local_size_x=32) in;\n"));

		let b = "
			vec3 lo = vec3(0.0);
			vec3 diffuse = vec3(0.0);
		
			float ao_factor = texture(ao, uv).r;
		
			for (uint i = 0; i < lighting_data.light_count; ++i) {{
				vec3 light_pos = lighting_data.lights[i].position;
				vec3 light_color = lighting_data.lights[i].color;
				mat4 light_matrix = lighting_data.lights[i].vp_matrix;
				uint8_t light_type = lighting_data.lights[i].light_type;
		
				vec3 L = vec3(0.0);
		
				if (light_type == 68) {{ // Infinite
					L = normalize(light_pos);
				}} else {{
					L = normalize(light_pos - vertex_position);
				}}
		
				float NdotL = max(dot(N, L), 0.0);
		
				if (NdotL <= 0.0) {{
					continue;
				}}
		
				float occlusion_factor = 1.0;
				float attenuation = 1.0;
		
				if (light_type == 68) {{ // Infinite
					vec4 surface_light_clip_position = light_matrix * vec4(vertex_position + N * 0.001, 1.0);
					vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
					vec2 shadow_uv = surface_light_ndc_position.xy * 0.5 + 0.5;
					float z = surface_light_ndc_position.z;
					float shadow_sample_depth = texture(depth_shadow_map, shadow_uv).r;
					float occlusion_factor = z < shadow_sample_depth ? 0.0 : 1.0;
		
					if (occlusion_factor == 0.0) {{
						continue;
					}}
		
					attenuation = 1.0;
				}} else {{
					float distance = length(light_pos - vertex_position);
					attenuation = 1.0 / (distance * distance);
				}}
		
				vec3 H = normalize(V + L);
		
				vec3 radiance = light_color * attenuation;
		
				vec3 F0 = vec3(0.04);
				F0 = mix(F0, albedo, metalness);
				vec3 F = fresnel_schlick(max(dot(H, V), 0.0), F0);
		
				float NDF = distribution_ggx(N, H, roughness);
				float G = geometry_smith(N, V, L, roughness);
				vec3 specular = (NDF * G * F) / (4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.000001);
		
				vec3 kS = F;
				vec3 kD = vec3(1.0) - kS;
		
				kD *= 1.0 - metalness;
		
				vec3 local_diffuse = kD * albedo / PI;
		
				lo += (local_diffuse + specular) * radiance * NdotL * occlusion_factor;
				diffuse += local_diffuse;
			}};
		
			lo *= ao_factor;

			imageStore(out_albedo, pixel_coordinates, vec4(lo, 1.0));
			imageStore(out_diffuse, pixel_coordinates, vec4(diffuse, 1.0));
		";

		let push_constant = self.push_constant.clone();
		let material_offset = self.material_offset.clone();
		let meshes = self.meshes.clone();
		let meshlets = self.meshlets.clone();
		let materials = self.material_struct.clone();
		let primitive_indices = self.primitive_indices.clone();
		let vertex_indices = self.vertex_indices.clone();
		let positions = self.positions.clone();
		let normals = self.normals.clone();

		let lighting_data = self.lighting_data.clone();
		let out_albedo = self.out_albedo.clone();
		let out_diffuse = self.out_diffuse.clone();

		match RefCell::borrow_mut(&main_function_node).node_mut() {
			jspd::Nodes::Function { statements, .. } => {
				statements.insert(0, jspd::Node::glsl(a.to_string(), vec![push_constant, material_offset, meshes, meshlets, materials, primitive_indices, vertex_indices, positions, normals]));
				statements.push(jspd::Node::glsl(b.to_string(), vec![lighting_data, out_albedo, out_diffuse]));
			}
			_ => {}
		}

		main_function_node
	}
}

#[cfg(test)]
mod tests {
    use crate::jspd;

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

	// 	let shader_node = jspd::compile_to_jspd(shader_source, None).unwrap();

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

	// 	let shader_node = jspd::compile_to_jspd(shader_source, None).unwrap();

	// 	let shader_generator = super::VisibilityShaderGenerator::new();

	// 	let shader = shader_generator.transform(&material, &shader_node, "Fragment").expect("Failed to generate shader");

	// 	// shaderc::Compiler::new().unwrap().compile_into_spirv(shader.as_str(), shaderc::ShaderKind::Compute, "shader.glsl", "main", None).unwrap();
	// }
}