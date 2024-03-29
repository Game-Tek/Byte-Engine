use std::{cell::RefCell, rc::Rc};

use resource_management::asset::material_asset_handler::ProgramGenerator;

use crate::jspd::lexer;

pub(crate) struct CommonShaderGenerator {
}

impl ProgramGenerator for CommonShaderGenerator {
	fn transform(&self, program_state: &mut jspd::parser::ProgramState, _: &json::JsonValue) -> Vec<jspd::parser::NodeReference> {
		let code = "vec4 get_debug_color(uint i) {
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
}";

		// RefCell::borrow_mut(&scope).add_child(jspd::Node::glsl(code.to_string(), Vec::new()));

		vec![jspd::parser::NodeReference::glsl(code, Vec::new(), Vec::new()).into()]
	}
}

impl CommonShaderGenerator {
	pub const SCOPE: &'static str = "Common";

	pub fn new() -> Self {
		Self {
		}
	}
}