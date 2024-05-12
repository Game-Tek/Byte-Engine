use std::{cell::RefCell, ops::Deref, rc::Rc};

use besl::NodeReference;
use maths_rs::vec;
use resource_management::asset::material_asset_handler::ProgramGenerator;

use crate::{rendering::{shader_strings::{CALCULATE_FULL_BARY, DISTRIBUTION_GGX, FRESNEL_SCHLICK, GEOMETRY_SMITH}, visibility_model::render_domain::{CAMERA_STRUCT_GLSL, LIGHTING_DATA_STRUCT_GLSL, LIGHT_STRUCT_GLSL, MATERIAL_STRUCT_GLSL, MESHLET_STRUCT_GLSL, MESH_STRUCT_GLSL}}, shader_generator};

pub struct VisibilityShaderGenerator {
	mesh_struct: besl::parser::NodeReference,
	camera_struct: besl::parser::NodeReference,
	meshlet_struct: besl::parser::NodeReference,
	light_struct: besl::parser::NodeReference,
	material_struct: besl::parser::NodeReference,
	meshes: besl::parser::NodeReference,
	positions: besl::parser::NodeReference,
	normals: besl::parser::NodeReference,
	tangents: besl::parser::NodeReference,
	uvs: besl::parser::NodeReference,
	vertex_indices: besl::parser::NodeReference,
	primitive_indices: besl::parser::NodeReference,
	meshlets: besl::parser::NodeReference,
	textures: besl::parser::NodeReference,
	material_count: besl::parser::NodeReference,
	material_offset: besl::parser::NodeReference,
	pixel_mapping: besl::parser::NodeReference,
	triangle_index: besl::parser::NodeReference,
	out_albedo: besl::parser::NodeReference,
	camera: besl::parser::NodeReference,
	out_diffuse: besl::parser::NodeReference,
	lighting_data: besl::parser::NodeReference,
	materials: besl::parser::NodeReference,
	ao: besl::parser::NodeReference,
	depth_shadow_map: besl::parser::NodeReference,
	push_constant: besl::parser::NodeReference,
	distribution_ggx: besl::parser::NodeReference,
	geometry_schlick_ggx: besl::parser::NodeReference,
	geometry_smith: besl::parser::NodeReference,
	fresnel_schlick: besl::parser::NodeReference,
	barycentric_deriv: besl::parser::NodeReference,
	calculate_full_bary: besl::parser::NodeReference,
	sample_function: besl::parser::NodeReference,
	sample_normal_function: besl::parser::NodeReference,
	unit_vector_from_xy: besl::parser::NodeReference,
}

impl VisibilityShaderGenerator {
	pub fn new(scope: NodeReference) -> Self {
		use besl::parser::NodeReference;

		let mesh_struct = NodeReference::r#struct("Mesh", vec![NodeReference::member("model", "mat4f"), NodeReference::member("material_index", "u32"), NodeReference::member("base_vertex_index", "u32")]);
		let camera_struct = NodeReference::r#struct("Camera", vec![NodeReference::member("view", "mat4f"), NodeReference::member("projection_matrix", "mat4f"), NodeReference::member("view_projection", "mat4f"), NodeReference::member("inverse_view_matrix", "mat4f"), NodeReference::member("inverse_projection_matrix", "mat4f"), NodeReference::member("inverse_view_projection_matrix", "mat4f")]);
		let meshlet_struct = NodeReference::r#struct("Meshlet", vec![NodeReference::member("instance_index", "u32"), NodeReference::member("vertex_offset", "u16"), NodeReference::member("triangle_offset", "u16"), NodeReference::member("vertex_count", "u8"), NodeReference::member("triangle_count", "u8")]);
		let light_struct = NodeReference::r#struct("Light", vec![NodeReference::member("view_matrix", "mat4f"), NodeReference::member("projection_matrix", "mat4f"), NodeReference::member("vp_matrix", "mat4f"), NodeReference::member("position", "vec3f"), NodeReference::member("color", "vec3f"), NodeReference::member("light_type", "u8")]);
		let material_struct = NodeReference::r#struct("Material", vec![NodeReference::member("textures", "u32[16]")]);

		let set0_binding1 = NodeReference::binding("meshes", NodeReference::buffer("MeshBuffer", vec![NodeReference::member("meshes", "Mesh[64]")]), 0, 1, true, false);
		let set0_binding2 = NodeReference::binding("positions", NodeReference::buffer("Positions", vec![NodeReference::member("positions", "vec3f[8192]")]), 0, 2, true, false);
		let set0_binding3 = NodeReference::binding("normals", NodeReference::buffer("Normals", vec![NodeReference::member("normals", "vec3f[8192]")]), 0, 3, true, false);
		let vertex_tangents_binding = NodeReference::binding("tangents", NodeReference::buffer("Tangents", vec![NodeReference::member("tangents", "vec3f[8192]")]), 0, 4, true, false);
		let vertex_uvs_binding = NodeReference::binding("uvs", NodeReference::buffer("UVs", vec![NodeReference::member("uvs", "vec2f[8192]")]), 0, 5, true, false);
		let set0_binding4 = NodeReference::binding("vertex_indices", NodeReference::buffer("VertexIndices", vec![NodeReference::member("vertex_indices", "u16[8192]")]), 0, 6, true, false);
		let set0_binding5 = NodeReference::binding("primitive_indices", NodeReference::buffer("PrimitiveIndices", vec![NodeReference::member("primitive_indices", "u8[8192]")]), 0, 7, true, false);
		let set0_binding6 = NodeReference::binding("meshlets", NodeReference::buffer("MeshletsBuffer", vec![NodeReference::member("meshlets", "Meshlet[8192]")]), 0, 8, true, false);
		let set0_binding7 = NodeReference::binding_array("textures", NodeReference::combined_image_sampler(), 0, 9, true, false, 16);

		let set1_binding0 = NodeReference::binding("material_count", NodeReference::buffer("MaterialCount", vec![NodeReference::member("material_count", "u32[2073600]")]), 1, 0, true, false);
		let set1_binding1 = NodeReference::binding("material_offset", NodeReference::buffer("MaterialOffset", vec![NodeReference::member("material_offset", "u32[2073600")]), 1, 1, true, false);
		let set1_binding4 = NodeReference::binding("pixel_mapping", NodeReference::buffer("PixelMapping", vec![NodeReference::member("pixel_mapping", "vec2u16[2073600]")]), 1, 4, true, false);
		let set1_binding6 = NodeReference::binding("triangle_index", NodeReference::image("r32ui"), 1, 6, true, false);

		let set2_binding0 = NodeReference::binding("out_albedo", NodeReference::image("rgba16"), 2, 0, false, true);
		let set2_binding1 = NodeReference::binding("camera", NodeReference::buffer("CameraBuffer", vec![NodeReference::member("camera", "Camera")]), 2, 1, true, false);
		let set2_binding2 = NodeReference::binding("out_diffuse", NodeReference::image("rgba16"), 2, 2, false, true);
		let set2_binding4 = NodeReference::binding("lighting_data", NodeReference::buffer("LightingBuffer", vec![NodeReference::member("light_count", "u32"), NodeReference::member("lights", "Light[16]")]), 2, 4, true, false);
		let set2_binding5 = NodeReference::binding("materials", NodeReference::buffer("MaterialBuffer", vec![NodeReference::member("materials", "Material[16]")]), 2, 5, true, false);
		let set2_binding10 = NodeReference::binding("ao", NodeReference::combined_image_sampler(), 2, 10, true, false);
		let set2_binding11 = NodeReference::binding("depth_shadow_map", NodeReference::combined_image_sampler(), 2, 11, true, false);

		let push_constant = NodeReference::push_constant(vec![NodeReference::member("material_id", "u32")]);

		let distribution_ggx = NodeReference::function("distribution_ggx", vec![NodeReference::member("n", "vec3f"), NodeReference::member("h", "vec3f"), NodeReference::member("roughness", "f32")], "f32", vec![NodeReference::glsl("float a = roughness*roughness; float a2 = a*a; float n_dot_h = max(dot(n, h), 0.0); float denom = ((n_dot_h*n_dot_h) * (a2 - 1.0) + 1.0); denom = PI * denom * denom; return a2 / denom;", Vec::new(), Vec::new())]);
		let geometry_schlick_ggx = NodeReference::function("geometry_schlick_ggx", vec![NodeReference::member("n_dot_v", "f32"), NodeReference::member("roughness", "f32")], "f32", vec![NodeReference::glsl("float r = (roughness + 1.0); float k = (r*r) / 8.0; return n_dot_v / (n_dot_v * (1.0 - k) + k);", Vec::new(), Vec::new())]);
		let geometry_smith = NodeReference::function("geometry_smith", vec![NodeReference::member("n", "vec3f"), NodeReference::member("v", "vec3f"), NodeReference::member("l", "vec3f"), NodeReference::member("roughness", "f32")], "f32", vec![NodeReference::glsl("return geometry_schlick_ggx(max(dot(n, v), 0.0), roughness) * geometry_schlick_ggx(max(dot(n, l), 0.0), roughness);", Vec::new(), Vec::new())]);
		let fresnel_schlick = NodeReference::function("fresnel_schlick", vec![NodeReference::member("cos_theta", "f32"), NodeReference::member("f0", "vec3f")], "vec3f", vec![NodeReference::glsl("return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);", Vec::new(), Vec::new())]);
		
		let barycentric_deriv = NodeReference::r#struct("BarycentricDeriv", vec![NodeReference::member("lambda", "vec3f"), NodeReference::member("ddx", "vec3f"), NodeReference::member("ddy", "vec3f")]);

		let calculate_full_bary = NodeReference::function("calculate_full_bary", vec![NodeReference::member("pt0", "vec4f"), NodeReference::member("pt1", "vec4f"), NodeReference::member("pt2", "vec4f"), NodeReference::member("pixelNdc", "vec2f"), NodeReference::member("winSize", "vec2f")], "BarycentricDeriv", vec![NodeReference::glsl("BarycentricDeriv ret = BarycentricDeriv(vec3(0), vec3(0), vec3(0)); vec3 invW = 1.0 / vec3(pt0.w, pt1.w, pt2.w); vec2 ndc0 = pt0.xy * invW.x; vec2 ndc1 = pt1.xy * invW.y; vec2 ndc2 = pt2.xy * invW.z; float invDet = 1.0 / determinant(mat2(ndc2 - ndc1, ndc0 - ndc1)); ret.ddx = vec3(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet * invW; ret.ddy = vec3(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet * invW; float ddxSum = dot(ret.ddx, vec3(1)); float ddySum = dot(ret.ddy, vec3(1)); vec2 deltaVec = pixelNdc - ndc0; float interpInvW = invW.x + deltaVec.x * ddxSum + deltaVec.y * ddySum; float interpW = 1.0 / interpInvW; ret.lambda.x = interpW * (invW.x + deltaVec.x * ret.ddx.x + deltaVec.y * ret.ddy.x); ret.lambda.y = interpW * (0.0    + deltaVec.x * ret.ddx.y + deltaVec.y * ret.ddy.y); ret.lambda.z = interpW * (0.0    + deltaVec.x * ret.ddx.z + deltaVec.y * ret.ddy.z); ret.ddx *= (2.0 / winSize.x); ret.ddy *= (2.0 / winSize.y); ddxSum  *= (2.0 / winSize.x); ddySum  *= (2.0 / winSize.y);  float interpW_ddx = 1.0 / (interpInvW + ddxSum); float interpW_ddy = 1.0 / (interpInvW + ddySum);  ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda; ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda; return ret;", Vec::new(), Vec::new())]);
		
		let sample_function = NodeReference::intrinsic("sample", NodeReference::parameter("smplr", "u32"), NodeReference::sentence(vec![NodeReference::glsl("texture(", Vec::new(), Vec::new()), NodeReference::member_expression("smplr"), NodeReference::glsl(", vertex_uv).rgb", Vec::new(), Vec::new())]), "vec3f");
		let unit_vector_from_xy = NodeReference::function("unit_vector_from_xy", vec![NodeReference::member("v", "vec2f")], "vec3f", vec![NodeReference::glsl("v = v * 2.0f - 1.0f; return normalize(vec3(v, sqrt(max(0.0f, 1.0f - v.x * v.x - v.y * v.y))));", Vec::new(), Vec::new())]);

		let sample_normal_function = if true {
			NodeReference::intrinsic("sample_normal", NodeReference::parameter("smplr", "u32"), NodeReference::sentence(vec![NodeReference::glsl("unit_vector_from_xy(texture(", Vec::new(), Vec::new()), NodeReference::member_expression("smplr"), NodeReference::glsl(", vertex_uv).xy)", vec!["unit_vector_from_xy".to_string()], Vec::new())]), "vec3f")
		} else {
			NodeReference::intrinsic("sample_normal", NodeReference::parameter("smplr", "u32"), NodeReference::sentence(vec![NodeReference::glsl("normalize(texture(", Vec::new(), Vec::new()), NodeReference::member_expression("smplr"), NodeReference::glsl(", vertex_uv).xyz * 2.0f - 1.0f)", vec![], Vec::new())]), "vec3f")
		};

		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,
			meshes: set0_binding1,
			positions: set0_binding2,
			normals: set0_binding3,
			tangents: vertex_tangents_binding,
			uvs: vertex_uvs_binding,
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
			sample_function,
			unit_vector_from_xy,
			sample_normal_function,
		}
	}
}

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform(&self, program_state: &mut besl::parser::ProgramState, material: &json::JsonValue) -> Vec<besl::parser::NodeReference> {
		let mesh_struct = self.mesh_struct.clone();
		let camera_struct = self.camera_struct.clone();
		let meshlet_struct = self.meshlet_struct.clone();
		let light_struct = self.light_struct.clone();
		let material_struct = self.material_struct.clone();
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
	vec4(positions.positions[vertex_indices[0]], 1.0),
	vec4(positions.positions[vertex_indices[1]], 1.0),
	vec4(positions.positions[vertex_indices[2]], 1.0)
);

vec4 vertex_normals[3] = vec4[3](
	vec4(normals.normals[vertex_indices[0]], 0.0),
	vec4(normals.normals[vertex_indices[1]], 0.0),
	vec4(normals.normals[vertex_indices[2]], 0.0)
);

vec2 vertex_uvs[3] = vec2[3](
	uvs.uvs[vertex_indices[0]],
	uvs.uvs[vertex_indices[1]],
	uvs.uvs[vertex_indices[2]]
);

vec2 image_extent = imageSize(triangle_index);

vec2 normalized_xy = pixel_coordinates / image_extent;

vec2 nc = normalized_xy * 2 - 1;

Camera camera = camera.camera;

vec4 clip_space_vertex_positions[3] = vec4[3](camera.view_projection * mesh.model * vertex_positions[0], camera.view_projection * mesh.model * vertex_positions[1], camera.view_projection * mesh.model * vertex_positions[2]);

BarycentricDeriv barycentric_deriv = calculate_full_bary(clip_space_vertex_positions[0], clip_space_vertex_positions[1], clip_space_vertex_positions[2], nc, image_extent);
vec3 barycenter = barycentric_deriv.lambda;
vec3 ddx = barycentric_deriv.ddx;
vec3 ddy = barycentric_deriv.ddy;

vec3 vertex_position = vec3((mesh.model * vertex_positions[0]).xyz * barycenter.x + (mesh.model * vertex_positions[1]).xyz * barycenter.y + (mesh.model * vertex_positions[2]).xyz * barycenter.z);
vec3 vertex_normal = vec3((mesh.model * vertex_normals[0]).xyz * barycenter.x + (mesh.model * vertex_normals[1]).xyz * barycenter.y + (mesh.model * vertex_normals[2]).xyz * barycenter.z);
vec2 vertex_uv = vec2((vertex_uvs[0]).xy * barycenter.x + (vertex_uvs[1]).xy * barycenter.y + (vertex_uvs[2]).xy * barycenter.z);

vec3 N = normalize(vec3(mesh.model * vec4(vertex_normal, 0)));
vec3 V = normalize(-(camera.view[3].xyz - vertex_position));

vec3 edge1 = ddx.x * vec3(vertex_positions[0]) + ddx.y * vec3(vertex_positions[1]) + ddx.z * vec3(vertex_positions[2]);
vec3 edge2 = ddy.x * vec3(vertex_positions[0]) + ddy.y * vec3(vertex_positions[1]) + ddy.z * vec3(vertex_positions[2]);
vec2 duv1 = barycentric_deriv.ddx.x * vec2(vertex_uvs[0]) + barycentric_deriv.ddx.y * vec2(vertex_uvs[1]) + barycentric_deriv.ddx.z * vec2(vertex_uvs[2]);
vec2 duv2 = barycentric_deriv.ddy.x * vec2(vertex_uvs[0]) + barycentric_deriv.ddy.y * vec2(vertex_uvs[1]) + barycentric_deriv.ddy.z * vec2(vertex_uvs[2]);
float f = 1.0 / (duv1.x * duv2.y - duv2.x * duv1.y);
vec3 T = normalize(f * (duv2.y * edge1 - duv1.y * edge2));
vec3 B = normalize(f * (-duv2.x * edge1 + duv1.x * edge2));
mat3 TBN = mat3(T, B, N);

vec3 albedo = vec3(1, 0, 0);
vec3 normal = normalize(vertex_normal);
float metalness = 0;
float roughness = float(0.5);";

		let mut extra = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].members() {
			let name = variable["name"].as_str().unwrap();
			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4" => {
					let x = besl::parser::NodeReference::specialization(name, data_type);
					program_state.insert(name.to_string(), x.clone());
					extra.push(x);
				}
				"Texture2D" => {
					let x = besl::parser::NodeReference::literal(name, besl::parser::NodeReference::glsl(&format!("textures[nonuniformEXT(material.textures[{}])]", texture_count), vec!["textures".to_string()], Vec::new()));
					program_state.insert(name.to_string(), x.clone());
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

roughness = roughness * roughness;
normal = normalize(TBN * normal);

for (uint i = 0; i < lighting_data.light_count; ++i) {
	vec3 light_pos = lighting_data.lights[i].position;
	vec3 light_color = lighting_data.lights[i].color;
	mat4 light_matrix = lighting_data.lights[i].vp_matrix;
	uint8_t light_type = lighting_data.lights[i].light_type;

	vec3 L = vec3(0.0);

	if (light_type == 68) { // Infinite
		L = normalize(-light_pos);
	} else {
		L = normalize(light_pos - vertex_position);
	}

	float NdotL = max(dot(normal, L), 0.0);

	// if (NdotL <= 0.0) { continue; }

	float occlusion_factor = 1.0;
	float attenuation = 1.0;

	if (light_type == 68) { // Infinite
		vec4 surface_light_clip_position = light_matrix * vec4(vertex_position + normal * 0.001, 1.0);
		vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;
		vec2 shadow_uv = surface_light_ndc_position.xy * 0.5 + 0.5;
		float z = surface_light_ndc_position.z;
		float shadow_sample_depth = texture(depth_shadow_map, shadow_uv).r;
		float occlusion_factor = z < shadow_sample_depth ? 0.0 : 1.0;

		if (occlusion_factor == 0.0) { continue; }

		attenuation = 1.0;
	} else {
		float distance = length(light_pos - vertex_position);
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
		let material_offset = self.material_offset.clone();
		let meshes = self.meshes.clone();
		let meshlets = self.meshlets.clone();
		let material = self.material_struct.clone();
		let primitive_indices = self.primitive_indices.clone();
		let vertex_indices = self.vertex_indices.clone();
		let positions = self.positions.clone();
		let normals = self.normals.clone();

		let lighting_data = self.lighting_data.clone();
		let out_albedo = self.out_albedo.clone();
		let out_diffuse = self.out_diffuse.clone();

		let mut m = program_state.get_mut("main").unwrap().clone();

		match m.node_mut() {
			besl::parser::Nodes::Function { statements, .. } => {
				statements.insert(0, besl::parser::NodeReference::glsl(a, vec!["uvs".to_string(), "ao".to_string(), "depth_shadow_map".to_string(), "push_constant".to_string(), "material_offset".to_string(), "pixel_mapping".to_string(), "material_count".to_string(), "meshes".to_string(), "meshlets".to_string(), "materials".to_string(), "primitive_indices".to_string(), "vertex_indices".to_string(), "positions".to_string(), "normals".to_string(), "triangle_index".to_string(), "camera".to_string(), "calculate_full_bary".to_string(), "fresnel_schlick".to_string(), "distribution_ggx".to_string(), "geometry_smith".to_string(), "geometry_schlick_ggx".to_string(), "BarycentricDeriv".to_string()], vec!["albedo".to_string(), "normal".to_string(), "roughness".to_string(), "metalness".to_string()]));
				statements.push(besl::parser::NodeReference::glsl(b, vec!["lighting_data".to_string(), "out_albedo".to_string(), "out_diffuse".to_string()], Vec::new()));
			}
			_ => {}
		}

		program_state.insert("Mesh".to_string(), mesh_struct.clone());
		program_state.insert("Camera".to_string(), camera_struct.clone());
		program_state.insert("Meshlet".to_string(), meshlet_struct.clone());
		program_state.insert("Light".to_string(), light_struct.clone());
		program_state.insert("Material".to_string(), material_struct.clone());
		program_state.insert("BarycentricDeriv".to_string(), self.barycentric_deriv.clone());

		program_state.insert("calculate_full_bary".to_string(), self.calculate_full_bary.clone());
		program_state.insert("distribution_ggx".to_string(), self.distribution_ggx.clone());
		program_state.insert("geometry_schlick_ggx".to_string(), self.geometry_schlick_ggx.clone());
		program_state.insert("geometry_smith".to_string(), self.geometry_smith.clone());
		program_state.insert("fresnel_schlick".to_string(), self.fresnel_schlick.clone());
		program_state.insert("unit_vector_from_xy".to_string(), self.unit_vector_from_xy.clone());

		program_state.insert("meshes".to_string(), set0_binding1.clone());
		program_state.insert("positions".to_string(), set0_binding2.clone());
		program_state.insert("normals".to_string(), set0_binding3.clone());
		program_state.insert("tangents".to_string(), self.tangents.clone());
		program_state.insert("uvs".to_string(), self.uvs.clone());
		program_state.insert("vertex_indices".to_string(), set0_binding4.clone());
		program_state.insert("primitive_indices".to_string(), set0_binding5.clone());
		program_state.insert("meshlets".to_string(), set0_binding6.clone());
		program_state.insert("textures".to_string(), set0_binding7.clone());
		program_state.insert("material_count".to_string(), set1_binding0.clone());
		program_state.insert("material_offset".to_string(), set1_binding1.clone());
		program_state.insert("pixel_mapping".to_string(), set1_binding4.clone());
		program_state.insert("triangle_index".to_string(), set1_binding6.clone());
		program_state.insert("out_albedo".to_string(), set2_binding0.clone());
		program_state.insert("camera".to_string(), set2_binding1.clone());
		program_state.insert("out_diffuse".to_string(), set2_binding2.clone());
		program_state.insert("lighting_data".to_string(), set2_binding4.clone());
		program_state.insert("materials".to_string(), set2_binding5.clone());
		program_state.insert("ao".to_string(), set2_binding10.clone());
		program_state.insert("depth_shadow_map".to_string(), set2_binding11.clone());
		program_state.insert("main".to_string(), m.clone());
		program_state.insert("sample".to_string(), self.sample_function.clone());
		program_state.insert("sample_normal".to_string(), self.sample_normal_function.clone());

		let mut ret = Vec::with_capacity(32);
		ret.append(&mut vec![push_constant, self.barycentric_deriv.clone(), set2_binding11, material_offset, meshes, set0_binding1, set1_binding0, self.uvs.clone(), set0_binding7, set1_binding4, set1_binding6, set2_binding1, meshlets, material, set2_binding5, set2_binding10, primitive_indices, vertex_indices, positions, normals, lighting_data, out_albedo, out_diffuse, self.calculate_full_bary.clone(), self.distribution_ggx.clone(), self.geometry_schlick_ggx.clone(), self.geometry_smith.clone(), self.fresnel_schlick.clone(), self.sample_function.clone(), self.unit_vector_from_xy.clone(), self.sample_normal_function.clone()]);
		ret.append(&mut extra);
		ret.push(m);
		ret
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