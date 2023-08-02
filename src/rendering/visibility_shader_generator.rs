use super::shader_generator::ShaderGenerator;

pub struct VisibilityShaderGenerator {}

impl VisibilityShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ShaderGenerator for VisibilityShaderGenerator {
	fn process(&self) -> (&'static str, json::JsonValue) {
		let mut value = json::JsonValue::new_object();

		let mut vertex_input = json::object! {
			"in_position": {
				type: "in",
				location: 0,
				type_name: "vec3f",
				interpolation: "smooth"
			},
			"in_normal": {
				type: "in",
				location: 1,
				type_name: "vec3f",
				interpolation: "smooth"
			},
		};

		let push_constant = json::object! {
			"camera": {
				type_name: "Camera*"
			},
			"model": {
				type_name: "Model*"
			}
		};

		let mut items = json::object! {
			"Camera": {
				type: "struct",
				members: {
					"view": {
						type: "mat4f",
					},
					"projection": {
						type: "mat4f",
					},
					"view_projection": {
						type: "vec3f",
					}
				}
			},
			"Model": {
				type: "struct",
				members: {
					"model": {
						type: "mat4f",
					},
				}
			},
		};

		let mut interface = json::object! {
			"in_instance_index": {
				type: "in",
				location: 0,
				type_name: "i32",
				interpolation: "flat"
			},
			"out_color": {
				type: "in",
				location: 0,
				type_name: "vec4",
				way: "out",
				interpolation: "smooth"
			}
		};

		("visibility", value)
	}
}