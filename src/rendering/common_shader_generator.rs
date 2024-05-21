use std::{cell::RefCell, rc::Rc};

use maths_rs::vec;
use resource_management::asset::material_asset_handler::ProgramGenerator;

use crate::besl::lexer;

pub struct CommonShaderGenerator {
	camera_binding: besl::parser::Node,
	mesh_struct: besl::parser::Node,
	camera_struct: besl::parser::Node,
	meshlet_struct: besl::parser::Node,
	light_struct: besl::parser::Node,
	material_struct: besl::parser::Node,
	meshes: besl::parser::Node,
	positions: besl::parser::Node,
	normals: besl::parser::Node,
	uvs: besl::parser::Node,
	vertex_indices: besl::parser::Node,
	primitive_indices: besl::parser::Node,
	meshlets: besl::parser::Node,
	textures: besl::parser::Node,
	material_count: besl::parser::Node,
	material_offset: besl::parser::Node,
	material_offset_scratch: besl::parser::Node,
	material_evaluation_dispatches: besl::parser::Node,
	pixel_mapping: besl::parser::Node,
	triangle_index: besl::parser::Node,
	instance_index: besl::parser::Node,
	compute_vertex_index: besl::parser::Node,
	process_meshlet: besl::parser::Node,
	distribution_ggx: besl::parser::Node,
	geometry_schlick_ggx: besl::parser::Node,
	geometry_smith: besl::parser::Node,
	fresnel_schlick: besl::parser::Node,
	barycentric_deriv: besl::parser::Node,
	calculate_full_bary: besl::parser::Node,
	unit_vector_from_xy: besl::parser::Node,
	interpolate_vec3f_with_deriv: besl::parser::Node,
	interpolate_vec2f_with_deriv: besl::parser::Node,
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, _: &json::JsonValue) -> besl::parser::Node {
		let code = "vec4 colors[16] = vec4[16](
	vec4(0.16863, 0.40392, 0.77647, 1),
	vec4(0.32941, 0.76863, 0.21961, 1),
	vec4(0.81961, 0.16078, 0.67451, 1),
	vec4(0.96863, 0.98824, 0.45490, 1),
	vec4(0.75294, 0.09020, 0.75686, 1),
	vec4(0.30588, 0.95686, 0.54510, 1),
	vec4(0.66667, 0.06667, 0.75686, 1),
	vec4(0.78824, 0.91765, 0.27451, 1),
	vec4(0.40980, 0.12745, 0.48627, 1),
	vec4(0.89804, 0.28235, 0.20784, 1),
	vec4(0.93725, 0.67843, 0.33725, 1),
	vec4(0.95294, 0.96863, 0.00392, 1),
	vec4(1.00000, 0.27843, 0.67843, 1),
	vec4(0.29020, 0.90980, 0.56863, 1),
	vec4(0.30980, 0.70980, 0.27059, 1),
	vec4(0.69804, 0.16078, 0.39216, 1)
);

return colors[i % 16];";

		let mesh_struct = self.mesh_struct.clone();
		let camera_struct = self.camera_struct.clone();
		let meshlet_struct = self.meshlet_struct.clone();
		let light_struct = self.light_struct.clone();
		let material_struct = self.material_struct.clone();
		let barycentric_deriv = self.barycentric_deriv.clone();

		let camera_binding = self.camera_binding.clone();
		let material_offset = self.material_offset.clone();
		let material_offset_scratch = self.material_offset_scratch.clone();
		let material_evaluation_dispatches = self.material_evaluation_dispatches.clone();
		let meshes = self.meshes.clone();
		let material_count = self.material_count.clone();
		let uvs = self.uvs.clone();
		let textures = self.textures.clone();
		let pixel_mapping = self.pixel_mapping.clone();
		let triangle_index = self.triangle_index.clone();
		let instance_index = self.instance_index.clone();
		let meshlets = self.meshlets.clone();
		let primitive_indices = self.primitive_indices.clone();
		let vertex_indices = self.vertex_indices.clone();
		let positions = self.positions.clone();
		let normals = self.normals.clone();

		let compute_vertex_index = self.compute_vertex_index.clone();
		let process_meshlet = self.process_meshlet.clone();
		let distribution_ggx = self.distribution_ggx.clone();
		let geometry_schlick_ggx = self.geometry_schlick_ggx.clone();
		let geometry_smith = self.geometry_smith.clone();
		let fresnel_schlick = self.fresnel_schlick.clone();
		let calculate_full_bary = self.calculate_full_bary.clone();
		let interpolate_vec2f_with_deriv = self.interpolate_vec2f_with_deriv.clone();
		let interpolate_vec3f_with_deriv = self.interpolate_vec3f_with_deriv.clone();
		let unit_vector_from_xy = self.unit_vector_from_xy.clone();

		let get_debug_color = besl::parser::Node::function("get_debug_color", vec![besl::parser::Node::parameter("i", "u32")], "vec4f", vec![besl::parser::Node::glsl(code, Vec::new(), Vec::new())]);

		root.add(vec![mesh_struct, camera_struct, meshlet_struct, light_struct, barycentric_deriv, material_struct]);
		root.add(vec![camera_binding, material_offset, material_offset_scratch, material_evaluation_dispatches, meshes, material_count, uvs, textures, pixel_mapping, triangle_index, meshlets, primitive_indices, vertex_indices, positions, normals, instance_index]);
		root.add(vec![compute_vertex_index, process_meshlet, distribution_ggx, geometry_schlick_ggx, geometry_smith, fresnel_schlick, calculate_full_bary, interpolate_vec2f_with_deriv, interpolate_vec3f_with_deriv, unit_vector_from_xy]);
		root.add(vec![get_debug_color]);

		root
	}
}

impl CommonShaderGenerator {
	pub const SCOPE: &'static str = "Common";

	pub fn new() -> Self {
		Self::new_with_params(true, true, true, true, true, true, true, true)
	}

	pub fn new_with_params(material_count_read: bool, material_count_write: bool, material_offset_read: bool, material_offset_write: bool, material_offset_scratch_read: bool, material_offset_scratch_write: bool, pixel_mapping_read: bool, pixel_mapping_write: bool) -> Self {
		use besl::parser::Node;

		let mesh_struct = Node::r#struct("Mesh", vec![Node::member("model", "mat4f"), Node::member("material_index", "u32"), Node::member("base_vertex_index", "u32"), Node::member("base_triangle_index", "u32")]);
		let camera_struct = Node::r#struct("Camera", vec![Node::member("view", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("inverse_view_matrix", "mat4f"), Node::member("inverse_projection_matrix", "mat4f"), Node::member("inverse_view_projection_matrix", "mat4f")]);
		let meshlet_struct = Node::r#struct("Meshlet", vec![Node::member("instance_index", "u32"), Node::member("vertex_offset", "u16"), Node::member("triangle_offset", "u16"), Node::member("vertex_count", "u8"), Node::member("triangle_count", "u8"), Node::member("primitive_vertex_offset", "u32"), Node::member("primitive_triangle_offset", "u32")]);
		let light_struct = Node::r#struct("Light", vec![Node::member("view_matrix", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("position", "vec3f"), Node::member("color", "vec3f"), Node::member("light_type", "u8")]);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);

		let camera_binding = Node::binding("camera", Node::buffer("CameraBuffer", vec![Node::member("camera", "Camera")]), 0, 0, true, false);
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
		let instance_index = Node::binding("instance_index", Node::image("r32ui"), 1, 7, true, false);

		let compute_vertex_index = Node::function("compute_vertex_index", vec![Node::parameter("mesh", "Mesh"), Node::parameter("meshlet", "Meshlet"), Node::parameter("primitive_index", "u32")], "u32", vec![Node::glsl("return mesh.base_vertex_index + meshlet.primitive_vertex_offset + vertex_indices.vertex_indices[mesh.base_vertex_index + meshlet.vertex_offset + primitive_index]; // Indices are relative to primitives", vec!["vertex_indices".to_string()], Vec::new())]);

		let process_meshlet = Node::function("process_meshlet", vec![Node::parameter("matrix", "mat4f")], "void", vec![Node::glsl("uint meshlet_index = gl_WorkGroupID.x;
		Meshlet meshlet = meshlets.meshlets[meshlet_index];
		Mesh mesh = meshes.meshes[meshlet.instance_index];
	
		uint instance_index = meshlet.instance_index;
	
		SetMeshOutputsEXT(meshlet.vertex_count, meshlet.triangle_count);

		uint primitive_index = gl_LocalInvocationID.x;
	
		if (primitive_index < uint(meshlet.vertex_count)) {
			uint vertex_index = compute_vertex_index(mesh, meshlet, primitive_index);
			gl_MeshVerticesEXT[primitive_index].gl_Position = matrix * mesh.model * vec4(vertex_positions.positions[vertex_index], 1.0);
		}
		
		if (primitive_index < uint(meshlet.triangle_count)) {
			uint triangle_index = mesh.base_triangle_index + meshlet.primitive_triangle_offset + meshlet.triangle_offset + primitive_index;
			uint triangle_indices[3] = uint[](primitive_indices.primitive_indices[triangle_index * 3 + 0], primitive_indices.primitive_indices[triangle_index * 3 + 1], primitive_indices.primitive_indices[triangle_index * 3 + 2]);
			gl_PrimitiveTriangleIndicesEXT[primitive_index] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
			out_instance_index[primitive_index] = instance_index;
			out_primitive_index[primitive_index] = (meshlet_index << 8) | (primitive_index & 0xFF);
		}", vec!["meshes".to_string(), "vertex_positions".to_string(), "vertex_indices".to_string(), "primitive_indices".to_string(), "meshlets".to_string(), "compute_vertex_index".to_string()], Vec::new())]);

		let square_vec2 = Node::function("vec2f_squared_length", vec![Node::parameter("v", "vec2")], "vec2", vec![Node::glsl("return dot(v, v);", Vec::new(), Vec::new())]);
		let square_vec3 = Node::function("vec3f_squared_length", vec![Node::parameter("v", "vec3")], "vec3", vec![Node::glsl("return dot(v, v);", Vec::new(), Vec::new())]);
		let square_vec4 = Node::function("vec4f_squared_length", vec![Node::parameter("v", "vec4")], "vec4", vec![Node::glsl("return dot(v, v);", Vec::new(), Vec::new())]);

		let min_diff = Node::function("min_diff", vec![Node::parameter("p", "vec3f"), Node::parameter("a", "vec3f"), Node::parameter("b", "vec3f")], "vec3f", vec![Node::glsl("vec3 ap = a - p; vec3 bp = p - b; return (length_squared(ap) < length_squared(bp)) ? ap : bp;", vec!["vec3f_length_squared".to_string()], Vec::new())]);

		let interleaved_gradient_noise = Node::function("interleaved_gradient_noise", vec![Node::parameter("pixel_x", "u32"), Node::parameter("pixel_y", "u32"), Node::parameter("frame", "u32")], "f32", vec![Node::glsl("frame = frame % 64; // need to periodically reset frame to avoid numerical issues float x = float(pixel_x) + 5.588238f * float(frame); float y = float(pixel_y) + 5.588238f * float(frame); return mod(52.9829189f * mod(0.06711056f * x + 0.00583715f * y, 1.0f), 1.0f);", Vec::new(), Vec::new())]);

		let get_world_space_position_from_depth = Node::function("get_world_space_position_from_depth", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("coords", "vec2u"), Node::parameter("inverse_projection_matrix", "mat4f"), Node::parameter("inverse_view_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		float depth_value = texelFetch(depth_map, ivec2(coords), 0).r;
		vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth_map, 0).xy);
		vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
		vec4 view_space = inverse_projection_matrix * clip_space;
		view_space /= view_space.w;
		vec4 world_space = inverse_view_matrix * view_space;
		return world_space.xyz;", Vec::new(), Vec::new())]);

		let get_view_space_position_from_depth = Node::function("get_view_space_position_from_depth", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("coords", "vec2u"), Node::parameter("inverse_projection_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		float depth_value = texelFetch(depth_map, ivec2(coords), 0).r;
		vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth_map, 0).xy);
		vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
		vec4 view_space = inverse_projection_matrix * clip_space;
		view_space /= view_space.w;
		return view_space.xyz;", Vec::new(), Vec::new())]);

		let make_normal_from_neighbouring_depth_samples = Node::function("make_normal_from_neighbouring_depth_samples", vec![Node::parameter("p", "vec3"), Node::parameter("pr", "vec3"), Node::parameter("pl", "vec3"), Node::parameter("pt", "vec3"), Node::parameter("pb", "vec3")], "vec3f", vec![Node::glsl("return normalize(cross(min_diff(p, pr, pl), min_diff(p, pt, pb)))", vec!["min_diff".to_string()], Vec::new())]);

		let get_perpendicular_vector = Node::function("get_perpendicular_vector", vec![Node::parameter("v", "vec3f")], "vec3f", vec![Node::glsl("return normalize(abs(v.x) > abs(v.z) ? vec3(-v.y, v.x, 0.0) : vec3(0.0, -v.z, v.y));", Vec::new(), Vec::new())]);

		// Get a cosine-weighted random vector centered around a specified normal direction.
		let get_cosine_hemisphere_sample = Node::function("get_cosine_hemisphere_sample", vec![Node::parameter("rand1", "float"), Node::parameter("rand2", "float"), Node::parameter("hit_norm", "vec3")], "vec3f", vec![Node::glsl("// Get 2 random numbers to select our sample with
		vec2 randVal = vec2(rand1, rand2);
	
		// Cosine weighted hemisphere sample from RNG
		vec3 bitangent = get_perpendicular_vector(hit_norm);
		vec3 tangent = cross(bitangent, hit_norm);
		float r = sqrt(randVal.x);
		float phi = 2.0f * PI * randVal.y;
	
		// Get our cosine-weighted hemisphere lobe sample direction
		return normalize(tangent * (r * cos(phi).x) + bitangent * (r * sin(phi)) + hit_norm.xyz * sqrt(max(0.0, 1.0f - randVal.x)));", vec!["get_perpendicular_vector".to_string()], Vec::new())]);

		let make_uv = Node::function("make_uv", vec![Node::parameter("coordinates", "vec2u"), Node::parameter("extent", "vec2u")], "vec2", vec![Node::glsl("return (vec2(coordinates) + 0.5f) / vec2(extent);", Vec::new(), Vec::new())]);

		let distribution_ggx = Node::function("distribution_ggx", vec![Node::member("n", "vec3f"), Node::member("h", "vec3f"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("float a = roughness*roughness; float a2 = a*a; float n_dot_h = max(dot(n, h), 0.0); float denom = ((n_dot_h*n_dot_h) * (a2 - 1.0) + 1.0); denom = PI * denom * denom; return a2 / denom;", Vec::new(), Vec::new())]);
		let geometry_schlick_ggx = Node::function("geometry_schlick_ggx", vec![Node::member("n_dot_v", "f32"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("float r = (roughness + 1.0); float k = (r*r) / 8.0; return n_dot_v / (n_dot_v * (1.0 - k) + k);", Vec::new(), Vec::new())]);
		let geometry_smith = Node::function("geometry_smith", vec![Node::member("n", "vec3f"), Node::member("v", "vec3f"), Node::member("l", "vec3f"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("return geometry_schlick_ggx(max(dot(n, v), 0.0), roughness) * geometry_schlick_ggx(max(dot(n, l), 0.0), roughness);", Vec::new(), Vec::new())]);
		let fresnel_schlick = Node::function("fresnel_schlick", vec![Node::member("cos_theta", "f32"), Node::member("f0", "vec3f")], "vec3f", vec![Node::glsl("return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);", Vec::new(), Vec::new())]);
		
		let barycentric_deriv = Node::r#struct("BarycentricDeriv", vec![Node::member("lambda", "vec3f"), Node::member("ddx", "vec3f"), Node::member("ddy", "vec3f")]);

		let calculate_full_bary = Node::function("calculate_full_bary", vec![Node::member("pt0", "vec4f"), Node::member("pt1", "vec4f"), Node::member("pt2", "vec4f"), Node::member("pixelNdc", "vec2f"), Node::member("winSize", "vec2f")], "BarycentricDeriv", vec![Node::glsl("BarycentricDeriv ret = BarycentricDeriv(vec3(0), vec3(0), vec3(0)); vec3 invW = 1.0 / vec3(pt0.w, pt1.w, pt2.w); vec2 ndc0 = pt0.xy * invW.x; vec2 ndc1 = pt1.xy * invW.y; vec2 ndc2 = pt2.xy * invW.z; float invDet = 1.0 / determinant(mat2(ndc2 - ndc1, ndc0 - ndc1)); ret.ddx = vec3(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet * invW; ret.ddy = vec3(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet * invW; float ddxSum = dot(ret.ddx, vec3(1)); float ddySum = dot(ret.ddy, vec3(1)); vec2 deltaVec = pixelNdc - ndc0; float interpInvW = invW.x + deltaVec.x * ddxSum + deltaVec.y * ddySum; float interpW = 1.0 / interpInvW; ret.lambda.x = interpW * (invW.x + deltaVec.x * ret.ddx.x + deltaVec.y * ret.ddy.x); ret.lambda.y = interpW * (0.0    + deltaVec.x * ret.ddx.y + deltaVec.y * ret.ddy.y); ret.lambda.z = interpW * (0.0    + deltaVec.x * ret.ddx.z + deltaVec.y * ret.ddy.z); ret.ddx *= (2.0 / winSize.x); ret.ddy *= (2.0 / winSize.y); ddxSum  *= (2.0 / winSize.x); ddySum  *= (2.0 / winSize.y);  float interpW_ddx = 1.0 / (interpInvW + ddxSum); float interpW_ddy = 1.0 / (interpInvW + ddySum);  ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda; ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda; return ret;", Vec::new(), Vec::new())]);
		let interpolate_vec3f_with_deriv = Node::function("interpolate_vec3f_with_deriv", vec![Node::member("interp", "vec3f"), Node::member("v0", "vec3f"), Node::member("v1", "vec3f"), Node::member("v2", "vec3f")], "vec3f", vec![Node::glsl("return vec3(dot(vec3(v0.x, v1.x, v2.x), interp), dot(vec3(v0.y, v1.y, v2.y), interp), dot(vec3(v0.z, v1.z, v2.z), interp));", Vec::new(), Vec::new())]);
		let interpolate_vec2f_with_deriv = Node::function("interpolate_vec2f_with_deriv", vec![Node::member("interp", "vec3f"), Node::member("v0", "vec2f"), Node::member("v1", "vec2f"), Node::member("v2", "vec2f")], "vec2f", vec![Node::glsl("return vec2(dot(vec3(v0.x, v1.x, v2.x), interp), dot(vec3(v0.y, v1.y, v2.y), interp));", Vec::new(), Vec::new())]);

		let unit_vector_from_xy = Node::function("unit_vector_from_xy", vec![Node::member("v", "vec2f")], "vec3f", vec![Node::glsl("v = v * 2.0f - 1.0f; return normalize(vec3(v, sqrt(max(0.0f, 1.0f - v.x * v.x - v.y * v.y))));", Vec::new(), Vec::new())]);

		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,

			camera_binding,
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
			distribution_ggx,
			geometry_schlick_ggx,
			geometry_smith,
			fresnel_schlick,
			barycentric_deriv,
			calculate_full_bary,
			interpolate_vec3f_with_deriv,
			interpolate_vec2f_with_deriv,
			unit_vector_from_xy,
		}
	}
}