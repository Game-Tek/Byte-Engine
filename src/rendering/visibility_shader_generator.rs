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

		string.push_str("layout(set=0,binding=0,scalar) buffer MaterialCount {
	uint material_count[];
};
layout(set=0,binding=1,scalar) buffer MaterialOffset {
	uint material_offset[];
};
layout(set=0,binding=4,scalar) buffer PixelMapping {
	u16vec2 pixel_mapping[];
};
layout(set=0, binding=6, r8ui) uniform readonly uimage2D vertex_id;
layout(set=1, binding=0, rgba16) uniform image2D out_albedo;
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
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 0, "float", format!("be_variable_{}_r", variable["name"]), "1.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 1, "float", format!("be_variable_{}_g", variable["name"]), "0.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 2, "float", format!("be_variable_{}_b", variable["name"]), "0.0"));
					string.push_str(&format!("layout(constant_id={}) const {} {} = {};", 3, "float", format!("be_variable_{}_a", variable["name"]), "1.0"));
					string.push_str(&format!("const {} {} = {};\n", "vec4", format!("be_variable_{}", variable["name"]), format!("vec4({name}_r, {name}_g, {name}_b, {name}_a)", name=format!("be_variable_{}", variable["name"]))));
				}
			}
		}

		string.push_str("layout(push_constant, scalar) uniform PushConstant {
	layout(offset=16) uint material_id;
} pc;");

		string.push_str(&format!("layout(local_size_x=32) in;\n"));
		string.push_str(&format!("void main() {{\n"));
		string.push_str(&format!("if (gl_GlobalInvocationID.x >= material_count[pc.material_id]) {{ return; }}")); // This bounds check is necessary since we're using a local_size_x of 32.
		string.push_str(&format!("uint offset = material_offset[pc.material_id];"));
		string.push_str(&format!("u16vec2 be_pixel_xy = pixel_mapping[offset + gl_GlobalInvocationID.x];"));
		string.push_str(&format!("ivec2 be_pixel_coordinate = ivec2(be_pixel_xy.x, be_pixel_xy.y);"));
		string.push_str(&format!("uint be_vertex_id = imageLoad(vertex_id, be_pixel_coordinate).r;"));

		fn visit_node(string: &mut String, shader_node: &lexer::Node) {
			match &shader_node.node {
				lexer::Nodes::Scope { name, children } => {
					for child in children {
						visit_node(string, child);
					}
				}
				lexer::Nodes::Function { name, params, return_type, statements, raw } => {
					for statement in statements {
						visit_node(string, statement);
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
					string.push_str(&code);
				}
				lexer::Nodes::Expression(expression) => {
					match expression {
						lexer::Expressions::Operator { operator, left, right } => {
							string.push_str(&format!("imageStore(out_albedo, be_pixel_coordinate, be_variable_color);"));
							// format!("imageStore(out_albedo, ivec2(gl_FragCoord.xy), vec4(1.0, 0.0, 0.0, 1.0));")
						}
						_ => panic!("Invalid expression")
					}
				}
			}
		}

		visit_node(&mut string, shader_node);

		string.push_str(&format!("}}"));

		string
	}
}