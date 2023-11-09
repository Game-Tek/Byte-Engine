use std::rc::Rc;

use crate::{jspd::{self, lexer}, shader_generator};

use super::shader_generator::ShaderGenerator;

pub struct VisibilityShaderGenerator {}

impl VisibilityShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ShaderGenerator for VisibilityShaderGenerator {
	fn process(&self, mut parent_children: Vec<Rc<lexer::Node>>) -> (&'static str, lexer::Node) {
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

		if let lexer::Nodes::Scope { name, children } = &mut node.node {
			children.append(&mut parent_children);
		};

		("Visibility", node)
	}
}

impl VisibilityShaderGenerator {
	/// Produce a GLSL shader string from a BESL shader node.
	/// This returns an option since for a given input stage the visibility shader generator may not produce any output.
	pub fn transform(&self, material: &json::JsonValue, shader_node: &lexer::Node, stage: &str) -> Option<String> {
		match stage {
			"Vertex" => None,
			"Fragment" => Some(self.fragment_transform(material, shader_node)),
			_ => panic!("Invalid stage"),
		}
	}

	fn fragment_transform(&self, material: &json::JsonValue, shader_node: &lexer::Node) -> String {
		let mut string = shader_generator::generate_glsl_header_block(&json::object! { "glsl": { "version": "450" }, "stage": "Compute" });

		string.push_str("
struct Mesh {
	mat4 model;
	uint material_id;
	uint32_t base_vertex_index;
};

layout(set=0, binding=1, scalar) buffer readonly MeshBuffer {
	Mesh meshes[];
};

layout(set=0, binding=2, scalar) buffer readonly Positions {
	vec3 positions[];
};

layout(set=0, binding=3, scalar) buffer readonly Normals {
	vec3 normals[];
};

layout(set=0, binding=4, scalar) buffer readonly VertexIndices {
	uint16_t vertex_indices[];
};

layout(set=0, binding=5, scalar) buffer readonly PrimitiveIndeces {
	uint8_t primitive_indeces[];
};

struct Meshlet {
	uint32_t instance_index;
	uint16_t vertex_offset;
	uint16_t triangle_offset;
	uint8_t vertex_count;
	uint8_t triangle_count;
};

layout(set=0,binding=6,scalar) buffer readonly MeshletsBuffer {
	Meshlet meshlets[];
};

layout(set=0,binding=7) uniform sampler2D textures[1];

layout(set=1,binding=0,scalar) buffer MaterialCount {
	uint material_count[];
};

layout(set=1,binding=1,scalar) buffer MaterialOffset {
	uint material_offset[];
};

layout(set=1,binding=4,scalar) buffer PixelMapping {
	u16vec2 pixel_mapping[];
};

layout(set=1, binding=6, r32ui) uniform readonly uimage2D triangle_index;
layout(set=1, binding=7, r32ui) uniform readonly uimage2D instance_id;
layout(set=2, binding=0, rgba16) uniform image2D out_albedo;
layout(set=2, binding=7, rgba16) uniform image2D out_position;
layout(set=2, binding=8, rgba16) uniform image2D out_normals;

struct BarycentricDeriv {
	vec3 lambda;
	vec3 ddx;
	vec3 ddy;
};

BarycentricDeriv CalcFullBary(vec4 pt0, vec4 pt1, vec4 pt2, vec2 pixelNdc, vec2 winSize) {
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

	ret.ddy *= -1.0;
	ddySum  *= -1.0;

	float interpW_ddx = 1.0 / (interpInvW + ddxSum);
	float interpW_ddy = 1.0 / (interpInvW + ddySum);

	ret.ddx = interpW_ddx * (ret.lambda * interpInvW + ret.ddx) - ret.lambda;
	ret.ddy = interpW_ddy * (ret.lambda * interpInvW + ret.ddy) - ret.lambda;  

	return ret;
}

const float PI = 3.14159265359;

float DistributionGGX(vec3 N, vec3 H, float roughness) {
    float a      = roughness*roughness;
    float a2     = a*a;
    float NdotH  = max(dot(N, H), 0.0);
    float NdotH2 = NdotH*NdotH;
	
    float num   = a2;
    float denom = (NdotH2 * (a2 - 1.0) + 1.0);
    denom = PI * denom * denom;
	
    return num / denom;
}

float GeometrySchlickGGX(float NdotV, float roughness) {
    float r = (roughness + 1.0);
    float k = (r*r) / 8.0;

    float num   = NdotV;
    float denom = NdotV * (1.0 - k) + k;
	
    return num / denom;
}

float GeometrySmith(vec3 N, vec3 V, vec3 L, float roughness) {
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    float ggx2  = GeometrySchlickGGX(NdotV, roughness);
    float ggx1  = GeometrySchlickGGX(NdotL, roughness);
	
    return ggx1 * ggx2;
}

vec3 CalculateBarycenter(vec3 vertices[3], vec2 p) {
	float D = vertices[0].x * (vertices[1].y - vertices[2].x) + vertices[0].y * (vertices[1].x - vertices[2].y) + vertices[1].x * vertices[2].y - vertices[1].y * vertices[2].x;

	D = D == 0.0 ? 0.00001 : D;

	float lambda1 = ((vertices[1].y - vertices[2].y) * p.x + (vertices[2].x - vertices[1].x) * p.y + (vertices[1].x * vertices[2].y - vertices[1].y * vertices[2].x)) / D;
	float lambda2 = ((vertices[2].y - vertices[0].y) * p.x + (vertices[0].x - vertices[2].x) * p.y + (vertices[2].x * vertices[0].y - vertices[2].y * vertices[0].x)) / D;
	float lambda3 = ((vertices[0].y - vertices[1].y) * p.x + (vertices[1].x - vertices[0].x) * p.y + (vertices[0].x * vertices[1].y - vertices[0].y * vertices[1].x)) / D;

	return vec3(lambda1, lambda2, lambda3);
}

vec4 get_debug_color(uint i) {
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
}\n");

		for variable in material["variables"].members() {
			if variable["type"] == "Static" {
				if variable["data_type"] == "vec4f" { // Since GLSL doesn't support vec4f constants, we have to split it into 4 floats.
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 0, "float", format!("{}_r", variable["name"]), "1.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 1, "float", format!("{}_g", variable["name"]), "0.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 2, "float", format!("{}_b", variable["name"]), "0.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 3, "float", format!("{}_a", variable["name"]), "1.0"));
					string.push_str(&format!("const {} {} = {};\n", "vec4", variable["name"], format!("vec4({name}_r, {name}_g, {name}_b, {name}_a)", name=variable["name"])));
				}
			}
		}

		string.push_str("
struct Camera {
	mat4 view;
	mat4 projection_matrix;
	mat4 view_projection;
};

layout(set=2,binding=5,scalar) buffer CameraBuffer {
	Camera camera;
};

struct Light {
	vec3 position;
	vec3 color;
};

struct LightingData {
	uint light_count;
	Light lights[16];
};

struct Material {
	uint textures[16];
};

layout(set=2,binding=9,scalar) buffer readonly LightingBuffer {
	LightingData lighting_data;
};

layout(set=2,binding=10,scalar) buffer readonly MaterialsBuffer {
	Material materials[];
};

layout(push_constant, scalar) uniform PushConstant {
	uint material_id;
} pc;");

		string.push_str("\nvec3 fresnelSchlick(float cosTheta, vec3 F0) {
			return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
		}");

		string.push_str(&format!("layout(local_size_x=32) in;\n"));

string.push_str("
void main() {
	if (gl_GlobalInvocationID.x >= material_count[pc.material_id]) { return; }

	uint offset = material_offset[pc.material_id];
	u16vec2 be_pixel_xy = pixel_mapping[offset + gl_GlobalInvocationID.x];
	ivec2 be_pixel_coordinate = ivec2(be_pixel_xy.x, be_pixel_xy.y);
	uint BE_TRIANGLE_MESHLET_INDEX = imageLoad(triangle_index, be_pixel_coordinate).r;
	uint BE_MESHLET_TRIANGLE_INDEX = BE_TRIANGLE_MESHLET_INDEX & 0xFF;
	uint BE_MESHLET_INDEX = BE_TRIANGLE_MESHLET_INDEX >> 8;
	uint be_instance_id = imageLoad(instance_id, be_pixel_coordinate).r;

	Mesh mesh = meshes[be_instance_id];
	Material material = materials[pc.material_id];
	Meshlet meshlet = meshlets[BE_MESHLET_INDEX];

	uint primitive_indices[3] = uint[3](
		uint(primitive_indeces[(meshlet.triangle_offset + BE_MESHLET_TRIANGLE_INDEX) * 3 + 0]),
		uint(primitive_indeces[(meshlet.triangle_offset + BE_MESHLET_TRIANGLE_INDEX) * 3 + 1]),
		uint(primitive_indeces[(meshlet.triangle_offset + BE_MESHLET_TRIANGLE_INDEX) * 3 + 2])
	);

	uint vertex_indeces[3] = uint[3](
		mesh.base_vertex_index + uint(vertex_indices[meshlet.vertex_offset + primitive_indices[0]]),
		mesh.base_vertex_index + uint(vertex_indices[meshlet.vertex_offset + primitive_indices[1]]),
		mesh.base_vertex_index + uint(vertex_indices[meshlet.vertex_offset + primitive_indices[2]])
	);

	vec4 vertex_positions[3] = vec4[3](
		vec4(positions[vertex_indeces[0]], 1.0),
		vec4(positions[vertex_indeces[1]], 1.0),
		vec4(positions[vertex_indeces[2]], 1.0)
	);

	vec4 vertex_normals[3] = vec4[3](
		vec4(normals[vertex_indeces[0]], 0.0),
		vec4(normals[vertex_indeces[1]], 0.0),
		vec4(normals[vertex_indeces[2]], 0.0)
	);

	vec2 uv = ((be_pixel_xy) / vec2(1920.0, 1080.0)) * 2 - 1;

	uv.y = -uv.y;

	vec4 a = camera.view_projection * mesh.model * vertex_positions[0];
	vec4 b = camera.view_projection * mesh.model * vertex_positions[1];
	vec4 c = camera.view_projection * mesh.model * vertex_positions[2];

	BarycentricDeriv barycentric_deriv = CalcFullBary(a, b, c, uv, vec2(1920.0, 1080.0));
	vec3 barycenter = barycentric_deriv.lambda;

	vec3 BE_VERTEX_POSITION = vec3((mesh.model * vertex_positions[0]).xyz * barycenter.x + (mesh.model * vertex_positions[1]).xyz * barycenter.y + (mesh.model * vertex_positions[2]).xyz * barycenter.z);
	vec3 BE_VERTEX_NORMAL = vec3((vertex_normals[0]).xyz * barycenter.x + (vertex_normals[1]).xyz * barycenter.y + (vertex_normals[2]).xyz * barycenter.z);

	vec3 N = normalize(BE_VERTEX_NORMAL);
	vec3 V = normalize(camera.view[3].xyz - BE_VERTEX_POSITION);

	vec3 BE_ALBEDO = vec3(1, 0, 0);
	vec3 BE_METALLIC = vec3(0);
	float BE_ROUGHNESS = float(0.5);
");

		fn visit_node(string: &mut String, shader_node: &lexer::Node) {
			match &shader_node.node {
				lexer::Nodes::Scope { name: _, children } => {
					for child in children {
						visit_node(string, child);
					}
				}
				lexer::Nodes::Function { name, params: _, return_type: _, statements, raw: _ } => {
					match name.as_str() {
						_ => {
							for statement in statements {
								visit_node(string, statement);
								string.push_str(";\n");
							}
						}
					}
				}
				lexer::Nodes::Struct { name, template, fields, types } => {
					for field in fields {
						visit_node(string, field);
					}
				}
				lexer::Nodes::Member { name, r#type } => {

				}
				lexer::Nodes::GLSL { code } => {
					string.push_str(code);
				}
				lexer::Nodes::Expression(expression) => {
					match expression {
						lexer::Expressions::Operator { operator, left: _, right } => {
							if operator == &lexer::Operators::Assignment {
								string.push_str(&format!("BE_ALBEDO = vec3("));
								visit_node(string, right);
								string.push_str(")");
							}
						}
						lexer::Expressions::FunctionCall { name, parameters } => {
							match name.as_str() {
								"sample" => {
									string.push_str(&format!("textureGrad("));
									for parameter in parameters {
										visit_node(string, parameter);
									}
									string.push_str(&format!(", uv, vec2(0.5), vec2(0.5f))"));
								}
								_ => {
									string.push_str(&format!("{}(", name));
									for parameter in parameters {
										visit_node(string, parameter);
									}
									string.push_str(&format!(")"));
								}
							}
						}
						lexer::Expressions::Member { name } => {
							match name.as_str() {
								_ => {
									string.push_str(name);
								}
							}
						}
						_ => panic!("Invalid expression")
					}
				}
			}
		}

		visit_node(&mut string, shader_node);

string.push_str(&format!("
	vec3 Lo = vec3(0.0);

	for (uint i = 0; i < lighting_data.light_count; ++i) {{
		vec3 light_pos = lighting_data.lights[i].position;
		vec3 light_color = lighting_data.lights[i].color;

		vec3 L = normalize(light_pos - BE_VERTEX_POSITION);
		vec3 H = normalize(V + L);

		float distance = length(light_pos - BE_VERTEX_POSITION);
		float attenuation = 1.0 / (distance * distance);
		vec3 radiance = light_color * attenuation;

		vec3 F0 = vec3(0.04);
		F0 = mix(F0, BE_ALBEDO, BE_METALLIC);
		vec3 F = fresnelSchlick(max(dot(H, V), 0.0), F0);

		float NDF = DistributionGGX(N, H, BE_ROUGHNESS);
		float G = GeometrySmith(N, V, L, BE_ROUGHNESS);
		vec3 numerator = NDF * G * F;
		float denominator = 4.0 * max(dot(N, V), 0.0) * max(dot(N, L), 0.0) + 0.000001;
		vec3 specular = numerator / denominator;

		vec3 kS = F;
		vec3 kD = vec3(1.0) - kS;

		kD *= 1.0 - BE_METALLIC;

		float NdotL = max(dot(N, L), 0.0);
		Lo += (kD * BE_ALBEDO / PI + specular) * radiance * NdotL;
	}}
"));

		string.push_str(&format!("imageStore(out_albedo, be_pixel_coordinate, vec4(Lo, 1.0));"));
		string.push_str(&format!("imageStore(out_position, be_pixel_coordinate, vec4(BE_VERTEX_POSITION, 1.0));"));
		string.push_str(&format!("imageStore(out_normals, be_pixel_coordinate, vec4(BE_VERTEX_NORMAL, 1.0));"));

		string.push_str(&format!("\n}}")); // Close main()

		string
	}
}