use std::{cell::RefCell, rc::Rc};

use maths_rs::vec;
use resource_management::asset::material_asset_handler::ProgramGenerator;
use utils::json;

use crate::besl::lexer;

///
/// # Functions
/// - `get_view_space_position_from_depth(depth_map: Texture2D, coords: vec2u, inverse_projection_matrix: mat4f) -> vec3f`
pub struct CommonShaderGenerator {
	camera_binding: besl::parser::Node,
	mesh_struct: besl::parser::Node,
	camera_struct: besl::parser::Node,
	meshlet_struct: besl::parser::Node,
	light_struct: besl::parser::Node,
	material_struct: besl::parser::Node,
	uv_derivatives_struct: besl::parser::Node,
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
	sin_from_tan: besl::parser::Node,
	make_uv: besl::parser::Node,
	snap_uv: besl::parser::Node,
	tangent: besl::parser::Node,
	min_diff: besl::parser::Node,
	interleaved_gradient_noise: besl::parser::Node,
	make_perpendicular_vector: besl::parser::Node,
	make_cosine_hemisphere_sample: besl::parser::Node,
	square_vec2: besl::parser::Node,
	square_vec3: besl::parser::Node,
	square_vec4: besl::parser::Node,
	make_world_space_position_from_depth: besl::parser::Node,
	get_world_space_position_from_depth: besl::parser::Node,
	get_view_space_position_from_depth: besl::parser::Node,
	rotate_directions: besl::parser::Node,
	make_normal_from_positions: besl::parser::Node,
	make_normal_from_depth_map: besl::parser::Node,
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, _: &json::Object) -> besl::parser::Node {
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
		let uv_derivatives_struct = self.uv_derivatives_struct.clone();

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
		let sin_from_tan = self.sin_from_tan.clone();
		let snap_uv = self.snap_uv.clone();
		let tangent = self.tangent.clone();
		let min_diff = self.min_diff.clone();
		let interleaved_gradient_noise = self.interleaved_gradient_noise.clone();
		let make_perpendicular_vector = self.make_perpendicular_vector.clone();
		let make_cosine_hemisphere_sample = self.make_cosine_hemisphere_sample.clone();
		let make_uv = self.make_uv.clone();
		let square_vec2 = self.square_vec2.clone();
		let square_vec3 = self.square_vec3.clone();
		let square_vec4 = self.square_vec4.clone();

		let make_world_space_position_from_depth = self.make_world_space_position_from_depth.clone();
		let get_world_space_position_from_depth = self.get_world_space_position_from_depth.clone();
		let get_view_space_position_from_depth = self.get_view_space_position_from_depth.clone();
		let rotate_directions = self.rotate_directions.clone();
		let make_normal_from_positions = self.make_normal_from_positions.clone();
		let make_normal_from_depth_map = self.make_normal_from_depth_map.clone();

		let get_debug_color = besl::parser::Node::function("get_debug_color", vec![besl::parser::Node::parameter("i", "u32")], "vec4f", vec![besl::parser::Node::glsl(code, &[], Vec::new())]);

		root.add(vec![mesh_struct, camera_struct, meshlet_struct, light_struct, barycentric_deriv, material_struct, uv_derivatives_struct]);
		root.add(vec![camera_binding, material_offset, material_offset_scratch, material_evaluation_dispatches, meshes, material_count, uvs, textures, pixel_mapping, triangle_index, meshlets, primitive_indices, vertex_indices, positions, normals, instance_index]);
		root.add(vec![compute_vertex_index, process_meshlet, distribution_ggx, geometry_schlick_ggx, geometry_smith, fresnel_schlick, calculate_full_bary, interpolate_vec2f_with_deriv, interpolate_vec3f_with_deriv, unit_vector_from_xy, sin_from_tan, snap_uv, tangent, square_vec2, square_vec3, square_vec4, min_diff]);
		root.add(vec![make_uv, interleaved_gradient_noise, make_perpendicular_vector, make_cosine_hemisphere_sample, make_world_space_position_from_depth, get_world_space_position_from_depth, get_view_space_position_from_depth, rotate_directions, make_normal_from_positions, make_normal_from_depth_map]);
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

		let mesh_struct = Node::r#struct("Mesh", vec![Node::member("model", "mat4f"), Node::member("material_index", "u32"), Node::member("base_vertex_index", "u32"), Node::member("base_primitive_index", "u32"), Node::member("base_triangle_index", "u32"), Node::member("base_meshlet_index", "u32")]);
		let camera_struct = Node::r#struct("Camera", vec![Node::member("view", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("inverse_view_matrix", "mat4f"), Node::member("inverse_projection_matrix", "mat4f"), Node::member("inverse_view_projection_matrix", "mat4f"), Node::member("fov", "vec2f")]);
		let meshlet_struct = Node::r#struct("Meshlet", vec![Node::member("primitive_offset", "u16"), Node::member("triangle_offset", "u16"), Node::member("primitive_count", "u8"), Node::member("triangle_count", "u8")]);
		let light_struct = Node::r#struct("Light", vec![Node::member("view_matrix", "mat4f"), Node::member("projection_matrix", "mat4f"), Node::member("view_projection", "mat4f"), Node::member("position", "vec3f"), Node::member("color", "vec3f"), Node::member("light_type", "u8")]);
		let material_struct = Node::r#struct("Material", vec![Node::member("textures", "u32[16]")]);
		let uv_derivatives_struct = Node::r#struct("UVDerivatives", vec![Node::member("du", "vec3f"), Node::member("dv", "vec3f")]);

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

		let square_vec2 = Node::function("vec2f_squared_length", vec![Node::parameter("v", "vec2f")], "f32", vec![Node::glsl("return dot(v, v)", &[], Vec::new())]);
		let square_vec3 = Node::function("vec3f_squared_length", vec![Node::parameter("v", "vec3f")], "f32", vec![Node::glsl("return dot(v, v)", &[], Vec::new())]);
		let square_vec4 = Node::function("vec4f_squared_length", vec![Node::parameter("v", "vec4f")], "f32", vec![Node::glsl("return dot(v, v)", &[], Vec::new())]);

		let min_diff = Node::function("min_diff", vec![Node::parameter("p", "vec3f"), Node::parameter("a", "vec3f"), Node::parameter("b", "vec3f")], "vec3f", vec![Node::glsl("vec3 ap = a - p; vec3 bp = p - b; return (vec3f_squared_length(ap) < vec3f_squared_length(bp)) ? ap : bp;", &["vec3f_squared_length"], Vec::new())]);

		let interleaved_gradient_noise = Node::function("interleaved_gradient_noise", vec![Node::parameter("pixel_x", "u32"), Node::parameter("pixel_y", "u32"), Node::parameter("frame", "u32")], "f32", vec![Node::glsl("frame = frame % 64; /* need to periodically reset frame to avoid numerical issues */ float x = float(pixel_x) + 5.588238f * float(frame); float y = float(pixel_y) + 5.588238f * float(frame); return mod(52.9829189f * mod(0.06711056f * x + 0.00583715f * y, 1.0f), 1.0f);", &[], Vec::new())]);

		let make_world_space_position_from_depth = Node::function("make_world_space_position_from_depth", vec![Node::parameter("depth", "f32"), Node::parameter("uv", "vec2f"), Node::parameter("inverse_projection_matrix", "mat4f"), Node::parameter("inverse_view_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		vec4 clip_space = vec4(uv * 2.0 - 1.0, depth, 1.0);
		vec4 view_space = inverse_projection_matrix * clip_space;
		view_space /= view_space.w;
		vec4 world_space = inverse_view_matrix * view_space;
		return world_space.xyz;", &[], Vec::new())]);

		let get_world_space_position_from_depth = Node::function("get_world_space_position_from_depth", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("coords", "vec2u"), Node::parameter("inverse_projection_matrix", "mat4f"), Node::parameter("inverse_view_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		float depth_value = texelFetch(depth_map, ivec2(coords), 0).r;
		vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth_map, 0).xy);
		return make_world_space_position_from_depth(depth_value, uv, inverse_projection_matrix, inverse_view_matrix);", &["make_world_space_position_from_depth"], Vec::new())]);

		let get_view_space_position_from_depth = Node::function("get_view_space_position_from_depth", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("uv", "vec2f"), Node::parameter("inverse_projection_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		float depth_value = texture(depth_map, uv).r;
		vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
		vec4 view_space = inverse_projection_matrix * clip_space;
		view_space /= view_space.w;
		return view_space.xyz;", &[], Vec::new())]);

		let sin_from_tan = Node::function("sin_from_tan", vec![Node::parameter("x", "f32")], "f32", vec![Node::glsl("return x * inversesqrt(x*x + 1.0)", &[], Vec::new())]);
		let tangent = Node::function("tangent", vec![Node::parameter("p", "vec3f"), Node::parameter("s", "vec3f")], "f32", vec![Node::glsl("return (p.z - s.z) * inversesqrt(dot(s.xy - p.xy, s.xy - p.xy))", &[], Vec::new())]);

		// Calculates an approximate normal from 5 positions.
		let make_normal_from_positions = Node::function("make_normal_from_positions", vec![Node::parameter("p", "vec3f"), Node::parameter("pr", "vec3f"), Node::parameter("pl", "vec3f"), Node::parameter("pt", "vec3f"), Node::parameter("pb", "vec3f")], "vec3f", vec![Node::glsl("return normalize(cross(min_diff(p, pr, pl), min_diff(p, pt, pb)))", &["min_diff"], Vec::new())]);

		let make_perpendicular_vector = Node::function("make_perpendicular_vector", vec![Node::parameter("v", "vec3f")], "vec3f", vec![Node::glsl("return normalize(abs(v.x) > abs(v.z) ? vec3(-v.y, v.x, 0.0) : vec3(0.0, -v.z, v.y));", &[], Vec::new())]);

		// Should we add .5 to the coordinates before dividing by the extent?
		let snap_uv = Node::function("snap_uv", vec![Node::parameter("uv", "vec2f"), Node::parameter("extent", "vec2u")], "vec2f", vec![Node::glsl("return round(uv * vec2(extent)) * (1.0f / vec2(extent))", &[], Vec::new())]);

		// Get a cosine-weighted random vector centered around a specified normal direction.
		let make_cosine_hemisphere_sample = Node::function("make_cosine_hemisphere_sample", vec![Node::parameter("rand_1", "f32"), Node::parameter("rand_2", "f32"), Node::parameter("hit_normal", "vec3f")], "vec3f", vec![Node::glsl("
		vec2 randVal = vec2(rand_1, rand_2);
		vec3 bitangent = make_perpendicular_vector(hit_normal);
		vec3 tangent = cross(bitangent, hit_normal);
		float r = sqrt(randVal.x);
		float phi = 2.0f * PI * randVal.y;
		return normalize(tangent * (r * cos(phi).x) + bitangent * (r * sin(phi)) + hit_normal.xyz * sqrt(max(0.0, 1.0f - randVal.x)));", &["make_perpendicular_vector"], Vec::new())]);

		let make_normal_from_depth_map = Node::function("make_normal_from_depth_map", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("coord", "vec2i"), Node::parameter("extent", "vec2u"), Node::parameter("inverse_projection_matrix", "mat4f"), Node::parameter("inverse_view_matrix", "mat4f")], "vec3f", vec![Node::glsl("
		float c_depth = texelFetch(depth_map, coord, 0).r;
		float l_depth = texelFetch(depth_map, coord + ivec2(-1, 0), 0).r;
		float r_depth = texelFetch(depth_map, coord + ivec2(1, 0), 0).r;
		float t_depth = texelFetch(depth_map, coord + ivec2(0, -1), 0).r;
		float b_depth = texelFetch(depth_map, coord + ivec2(0, 1), 0).r;

		vec3 c_pos = make_world_space_position_from_depth(c_depth, make_uv(coord, extent), inverse_projection_matrix, inverse_view_matrix);
		vec3 l_pos = make_world_space_position_from_depth(l_depth, make_uv(coord + ivec2(-1, 0), extent), inverse_projection_matrix, inverse_view_matrix);
		vec3 r_pos = make_world_space_position_from_depth(r_depth, make_uv(coord + ivec2(1, 0), extent), inverse_projection_matrix, inverse_view_matrix);
		vec3 t_pos = make_world_space_position_from_depth(t_depth, make_uv(coord + ivec2(0, -1), extent), inverse_projection_matrix, inverse_view_matrix);
		vec3 b_pos = make_world_space_position_from_depth(b_depth, make_uv(coord + ivec2(0, 1), extent), inverse_projection_matrix, inverse_view_matrix);

		return make_normal_from_positions(c_pos, r_pos, l_pos, t_pos, b_pos);", &["make_world_space_position_from_depth", "make_uv", "make_normal_from_positions"], Vec::new())]);

		let make_uv = Node::function("make_uv", vec![Node::parameter("coordinates", "vec2i"), Node::parameter("extent", "vec2u")], "vec2f", vec![Node::glsl("return (vec2(coordinates) + 0.5f) / vec2(extent);", &[], Vec::new())]);
		let rotate_directions = Node::function("rotate_directions", vec![Node::parameter("dir", "vec2f"), Node::parameter("cos_sin", "vec2f")], "vec2f", vec![Node::glsl("return vec2(dir.x*cos_sin.x - dir.y*cos_sin.y,dir.x*cos_sin.y + dir.y*cos_sin.x)", &[], Vec::new())]);

		let distribution_ggx = Node::function("distribution_ggx", vec![Node::member("n", "vec3f"), Node::member("h", "vec3f"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("float a = roughness*roughness; float a2 = a*a; float n_dot_h = max(dot(n, h), 0.0); float denom = ((n_dot_h*n_dot_h) * (a2 - 1.0) + 1.0); denom = PI * denom * denom; return a2 / denom;", &[], Vec::new())]);
		let geometry_schlick_ggx = Node::function("geometry_schlick_ggx", vec![Node::member("n_dot_v", "f32"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("float r = (roughness + 1.0); float k = (r*r) / 8.0; return n_dot_v / (n_dot_v * (1.0 - k) + k);", &[], Vec::new())]);
		let geometry_smith = Node::function("geometry_smith", vec![Node::member("n", "vec3f"), Node::member("v", "vec3f"), Node::member("l", "vec3f"), Node::member("roughness", "f32")], "f32", vec![Node::glsl("return geometry_schlick_ggx(max(dot(n, v), 0.0), roughness) * geometry_schlick_ggx(max(dot(n, l), 0.0), roughness);", &["geometry_schlick_ggx"], Vec::new())]);
		let fresnel_schlick = Node::function("fresnel_schlick", vec![Node::member("cos_theta", "f32"), Node::member("f0", "vec3f")], "vec3f", vec![Node::glsl("return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);", &[], Vec::new())]);
		
		let barycentric_deriv = Node::r#struct("BarycentricDeriv", vec![Node::member("lambda", "vec3f"), Node::member("ddx", "vec3f"), Node::member("ddy", "vec3f")]);

		let calculate_full_bary = Node::function("calculate_full_bary", vec![Node::member("pt0", "vec4f"), Node::member("pt1", "vec4f"), Node::member("pt2", "vec4f"), Node::member("pixelNdc", "vec2f"), Node::member("winSize", "vec2f")], "BarycentricDeriv", vec![Node::glsl("BarycentricDeriv ret = BarycentricDeriv(vec3(0), vec3(0), vec3(0)); vec3 invW = 1.0 / vec3(pt0.w, pt1.w, pt2.w); vec2 ndc0 = pt0.xy * invW.x; vec2 ndc1 = pt1.xy * invW.y; vec2 ndc2 = pt2.xy * invW.z; float invDet = 1.0 / determinant(mat2(ndc2 - ndc1, ndc0 - ndc1)); ret.ddx = vec3(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet * invW; ret.ddy = vec3(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet * invW; float ddxSum = dot(ret.ddx, vec3(1)); float ddySum = dot(ret.ddy, vec3(1)); vec2 deltaVec = pixelNdc - ndc0; float interpInvW = invW.x + deltaVec.x * ddxSum + deltaVec.y * ddySum; float interpW = 1.0 / interpInvW; ret.lambda.x = interpW * (invW.x + deltaVec.x * ret.ddx.x + deltaVec.y * ret.ddy.x); ret.lambda.y = interpW * (0.0    + deltaVec.x * ret.ddx.y + deltaVec.y * ret.ddy.y); ret.lambda.z = interpW * (0.0    + deltaVec.x * ret.ddx.z + deltaVec.y * ret.ddy.z); ret.ddx *= (2.0 / winSize.x); ret.ddy *= (2.0 / winSize.y); ddxSum  *= (2.0 / winSize.x); ddySum  *= (2.0 / winSize.y);  float interpW_ddx = 1.0 / (interpInvW + ddxSum); float interpW_ddy = 1.0 / (interpInvW + ddySum);  ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda; ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda; return ret;", &[], Vec::new())]);
		let interpolate_vec3f_with_deriv = Node::function("interpolate_vec3f_with_deriv", vec![Node::member("interp", "vec3f"), Node::member("v0", "vec3f"), Node::member("v1", "vec3f"), Node::member("v2", "vec3f")], "vec3f", vec![Node::glsl("return vec3(dot(vec3(v0.x, v1.x, v2.x), interp), dot(vec3(v0.y, v1.y, v2.y), interp), dot(vec3(v0.z, v1.z, v2.z), interp));", &[], Vec::new())]);
		let interpolate_vec2f_with_deriv = Node::function("interpolate_vec2f_with_deriv", vec![Node::member("interp", "vec3f"), Node::member("v0", "vec2f"), Node::member("v1", "vec2f"), Node::member("v2", "vec2f")], "vec2f", vec![Node::glsl("return vec2(dot(vec3(v0.x, v1.x, v2.x), interp), dot(vec3(v0.y, v1.y, v2.y), interp));", &[], Vec::new())]);

		let unit_vector_from_xy = Node::function("unit_vector_from_xy", vec![Node::member("v", "vec2f")], "vec3f", vec![Node::glsl("v = v * 2.0f - 1.0f; return normalize(vec3(v, sqrt(max(0.0f, 1.0f - v.x * v.x - v.y * v.y))));", &[], Vec::new())]);

		Self {
			mesh_struct,
			camera_struct,
			meshlet_struct,
			light_struct,
			material_struct,
			uv_derivatives_struct,

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
			sin_from_tan,
			snap_uv,
			tangent,
			min_diff,
			interleaved_gradient_noise,
			make_perpendicular_vector,
			make_cosine_hemisphere_sample,
			make_uv,
			square_vec2,
			square_vec3,
			square_vec4,
			make_world_space_position_from_depth,
			get_world_space_position_from_depth,
			get_view_space_position_from_depth,
			make_normal_from_positions,
			rotate_directions,

			make_normal_from_depth_map,
		}
	}
}