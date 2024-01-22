pub const GET_VIEW_SPACE_POSITION_FROM_DEPTH: &str = {
	"vec3 get_view_space_position_from_depth(sampler2D depth_map, uvec2 coords, mat4 projection_matrix, mat4 view_matrix) {
		float depth_value = texelFetch(depth_map, ivec2(coords), 0).r;
		vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth_map, 0).xy);
		vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
		vec4 view_space = inverse(projection_matrix) * clip_space;
		view_space /= view_space.w;
		vec4 world_space = inverse(view_matrix) * view_space;
		return world_space.xyz;
	}"
};

pub const FRESNEL_SCHLICK : &str = {
	"vec3 fresnel_schlick(float cos_theta, vec3 f0) {
		return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
	}"
};

pub const GEOMETRY_SMITH: &str = {
	"float geometry_schlick_ggx(float n_dot_v, float roughness) {
		float r = (roughness + 1.0);
		float k = (r*r) / 8.0;
		return n_dot_v / (n_dot_v * (1.0 - k) + k);
	}
	
	float geometry_smith(vec3 n, vec3 v, vec3 l, float roughness) {
		return geometry_schlick_ggx(max(dot(n, v), 0.0), roughness) * geometry_schlick_ggx(max(dot(n, l), 0.0), roughness);
	}"
};

/// Needs PI to be defined
pub const DISTRIBUTION_GGX: &str = {
	"float distribution_ggx(vec3 n, vec3 h, float roughness) {
		float a      = roughness*roughness;
		float a2     = a*a;
		float n_dot_h  = max(dot(n, h), 0.0);
		
		float denom = ((n_dot_h*n_dot_h) * (a2 - 1.0) + 1.0);
		denom = PI * denom * denom;
		
		return a2 / denom;
	}"
};

pub const GET_DEBUG_COLOR: &str = {
	"vec4 get_debug_color(uint i) {
		vec4 colors[16] = vec4[16](
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
	
		return colors[i % 16];
	}"
};

pub const CALCULATE_BARYCENTER: &str = {
	"vec3 calculate_barycenter(vec3 vertices[3], vec2 p) {
		float d = vertices[0].x * (vertices[1].y - vertices[2].x) + vertices[0].y * (vertices[1].x - vertices[2].y) + vertices[1].x * vertices[2].y - vertices[1].y * vertices[2].x;
	
		d = d == 0.0 ? 0.00001 : d;
	
		float lambda_1 = ((vertices[1].y - vertices[2].y) * p.x + (vertices[2].x - vertices[1].x) * p.y + (vertices[1].x * vertices[2].y - vertices[1].y * vertices[2].x)) / d;
		float lambda_2 = ((vertices[2].y - vertices[0].y) * p.x + (vertices[0].x - vertices[2].x) * p.y + (vertices[2].x * vertices[0].y - vertices[2].y * vertices[0].x)) / d;
		float lambda_3 = ((vertices[0].y - vertices[1].y) * p.x + (vertices[1].x - vertices[0].x) * p.y + (vertices[0].x * vertices[1].y - vertices[0].y * vertices[1].x)) / d;
	
		return vec3(lambda_1, lambda2_, lambda_3);
	}"
};

pub const CALCULATE_FULL_BARY: &str = {
	"struct BarycentricDeriv {
		vec3 lambda;
		vec3 ddx;
		vec3 ddy;
	};
	
	BarycentricDeriv calculate_full_bary(vec4 pt0, vec4 pt1, vec4 pt2, vec2 pixelNdc, vec2 winSize) {
		BarycentricDeriv ret = BarycentricDeriv(vec3(0), vec3(0), vec3(0));
	
		vec3 invW = 1.0 / vec3(pt0.w, pt1.w, pt2.w);
	
		vec2 ndc0 = pt0.xy * invW.x;
		vec2 ndc1 = pt1.xy * invW.y;
		vec2 ndc2 = pt2.xy * invW.z;
	
		float invDet = 1.0 / determinant(mat2(ndc2 - ndc1, ndc0 - ndc1));
		ret.ddx = vec3(ndc1.y - ndc2.y, ndc2.y - ndc0.y, ndc0.y - ndc1.y) * invDet * invW;
		ret.ddy = vec3(ndc2.x - ndc1.x, ndc0.x - ndc2.x, ndc1.x - ndc0.x) * invDet * invW;
		float ddxSum = dot(ret.ddx, vec3(1));
		float ddySum = dot(ret.ddy, vec3(1));
	
		vec2 deltaVec = pixelNdc - ndc0;
		float interpInvW = invW.x + deltaVec.x * ddxSum + deltaVec.y * ddySum;
		float interpW = 1.0 / interpInvW;
	
		ret.lambda.x = interpW * (invW.x + deltaVec.x * ret.ddx.x + deltaVec.y * ret.ddy.x);
		ret.lambda.y = interpW * (0.0    + deltaVec.x * ret.ddx.y + deltaVec.y * ret.ddy.y);
		ret.lambda.z = interpW * (0.0    + deltaVec.x * ret.ddx.z + deltaVec.y * ret.ddy.z);
	
		ret.ddx *= (2.0 / winSize.x);
		ret.ddy *= (2.0 / winSize.y);
		ddxSum  *= (2.0 / winSize.x);
		ddySum  *= (2.0 / winSize.y);
	
		float interpW_ddx = 1.0 / (interpInvW + ddxSum);
		float interpW_ddy = 1.0 / (interpInvW + ddySum);
	
		ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda;
		ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda;  
	
		return ret;
	}"
};