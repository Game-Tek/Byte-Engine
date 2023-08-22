use super::shader_generator::ShaderGenerator;

pub struct VisibilityShaderGenerator {}

impl VisibilityShaderGenerator {
	pub fn new() -> Self {
		Self {}
	}
}

impl ShaderGenerator for VisibilityShaderGenerator {
	fn process(&self) -> (&'static str, json::JsonValue) {
		let value = json::object! {
			"type": "scope",
			"in_position": {
				type: "in",
				in_position: {
					type: "member",
					data_type: "vec3f",
				}
			},
			"in_normal": {
				type: "in",
				in_normal: {
					type: "member",
					data_type: "vec3f",
				}
			},
			"pc": {
				type: "push_constant",
				"camera": {
					type: "member",
					data_type: "Camera*"
				},
				"meshes": {
					type: "member",
					data_type: "Mesh*"
				},
			},
			"Camera": {
				type: "struct",
				"view": {
					type: "member",
					data_type: "mat4f",
				},
				"projection": {
					type: "member",
					data_type: "mat4f",
				},
				"view_projection": {
					type: "member",
					data_type: "mat4f",
				}
			},
			"Mesh": {
				type: "struct",
				"model": {
					type: "member",
					data_type: "mat4f",
				},
			},
			"in_instance_index": {
				type: "in",
				in_instance_index: {
					type: "member",
					data_type: "u32",
				},
				interpolation: "flat"
			},
			"out_color": {
				type: "out",
				out_Color: {
					type: "member",
					data_type: "vec4f",
				}
			}
		};

		("Visibility", value)
	}
}