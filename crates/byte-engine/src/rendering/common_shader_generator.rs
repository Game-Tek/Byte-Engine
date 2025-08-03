use std::{cell::RefCell, rc::Rc};

use resource_management::asset::material_asset_handler::ProgramGenerator;
use utils::json;

use crate::besl::lexer;

///
/// # Functions
/// - `get_view_space_position_from_depth(depth_map: Texture2D, coords: vec2u, inverse_projection_matrix: mat4f) -> vec3f`
pub struct CommonShaderScope {
}

pub struct CommonShaderGenerator {
}

impl CommonShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform(&self, mut root: besl::parser::Node, _: &json::Object) -> besl::parser::Node {
		root
	}
}

impl CommonShaderScope {
	pub fn new() -> besl::parser::Node {
		use besl::parser::Node;

		let uv_derivatives_struct = Node::r#struct("UVDerivatives", vec![Node::member("du", "vec3f"), Node::member("dv", "vec3f")]);

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
		return normalize(tangent * (r * cos(phi)) + bitangent * (r * sin(phi)) + hit_normal.xyz * sqrt(max(0.0, 1.0f - randVal.x)));", &["make_perpendicular_vector"], Vec::new())]);

		let make_normal_from_depth_map = Node::function("make_normal_from_depth_map", vec![Node::parameter("depth_map", "Texture2D"), Node::parameter("coord", "vec2i"), Node::parameter("extent", "vec2u"), Node::parameter("inverse_projection", "mat4f"), Node::parameter("inverse_view", "mat4f")], "vec3f", vec![Node::glsl("
		float c_depth = texelFetch(depth_map, coord, 0).r;

		if (c_depth == 0.0) { return vec3(0.0); }

		float l_depth = texelFetch(depth_map, coord + ivec2(-1, 0), 0).r;
		float r_depth = texelFetch(depth_map, coord + ivec2(1, 0), 0).r;
		float t_depth = texelFetch(depth_map, coord + ivec2(0, -1), 0).r;
		float b_depth = texelFetch(depth_map, coord + ivec2(0, 1), 0).r;

		vec3 c_pos = make_world_space_position_from_depth(c_depth, make_uv(coord, extent), inverse_projection, inverse_view);
		vec3 l_pos = make_world_space_position_from_depth(l_depth, make_uv(coord + ivec2(-1, 0), extent), inverse_projection, inverse_view);
		vec3 r_pos = make_world_space_position_from_depth(r_depth, make_uv(coord + ivec2(1, 0), extent), inverse_projection, inverse_view);
		vec3 t_pos = make_world_space_position_from_depth(t_depth, make_uv(coord + ivec2(0, -1), extent), inverse_projection, inverse_view);
		vec3 b_pos = make_world_space_position_from_depth(b_depth, make_uv(coord + ivec2(0, 1), extent), inverse_projection, inverse_view);

		return make_normal_from_positions(c_pos, l_pos, r_pos, t_pos, b_pos);", &["make_world_space_position_from_depth", "make_uv", "make_normal_from_positions"], Vec::new())]);

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

		return colors[i % 16];".trim();

		let get_debug_color = besl::parser::Node::function("get_debug_color", vec![besl::parser::Node::parameter("i", "u32")], "vec4f", vec![besl::parser::Node::glsl(code, &[], Vec::new())]);

		Node::scope("Common", vec![
			uv_derivatives_struct,

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

			square_vec2,
			square_vec3,
			square_vec4,

			min_diff,
			interleaved_gradient_noise,
			make_perpendicular_vector,
			make_cosine_hemisphere_sample,
			make_uv,
			make_world_space_position_from_depth,
			get_world_space_position_from_depth,
			get_view_space_position_from_depth,
			make_normal_from_positions,
			rotate_directions,

			make_normal_from_depth_map,

			get_debug_color,
		])
	}
}
