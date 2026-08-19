use besl::parser::Node;
use resource_management::asset::{handler::implementations::bema::ProgramGenerator, JsonObject};
use utils::json::{self, JsonContainerTrait, JsonValueTrait};

use super::ast::*;
use super::scope::VisibilityShaderGenerator;
use crate::rendering::common_shader_generator::CommonShaderScope;

impl ProgramGenerator for VisibilityShaderGenerator {
	fn transform<'a>(&self, mut root: besl::parser::Node<'a>, material: &'a JsonObject) -> besl::parser::Node<'a> {
		let mut extra: Vec<Node<'a>> = Vec::new();

		let mut texture_slots = Vec::new();

		let mut texture_count = 0;

		for variable in material["variables"].as_array().unwrap().iter() {
			let name = variable["name"].as_str().unwrap();

			let data_type = variable["data_type"].as_str().unwrap();

			match data_type {
				"u32" | "f32" | "vec2f" | "vec3f" | "vec4f" => {
					let x = besl::parser::Node::specialization(name, data_type);

					extra.push(x);
				}
				"Texture2D" => {
					texture_slots.push((name, texture_count));

					let slot = format!("{texture_count}u");

					let slot_node = besl::parser::Node::literal_expression(slot);

					let x = besl::parser::Node::constant(name, "u32", slot_node);

					extra.push(x);

					texture_count += 1;
				}
				_ => {}
			}
		}

		let m = root.get_mut("main").unwrap();

		let reconstruction_features = material_reconstruction_features(m);

		add_material_sample_context(m, &texture_slots);

		narrow_material_property_assignments(m);

		if let besl::parser::Nodes::Function { statements, .. } = m.node_mut() {
			statements.splice(0..0, material_evaluation_prefix_statements(reconstruction_features));

			statements.extend(material_evaluation_suffix_statements(reconstruction_features));
		}

		root.add(extra);

		root.add(vec![CommonShaderScope::new(), self.scope.clone()]);

		root
	}
}
