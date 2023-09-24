use std::rc::Rc;

use crate::jspd::{self, lexer};

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