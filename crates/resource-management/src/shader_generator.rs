use utils::Extent;

/// Generates a graphics API consumable shader from a BESL shader program definition.
pub trait ShaderGenerator {

}

pub enum Stages {
	Vertex,
	Compute {
		local_size: Extent,
	},
	Task,
	Mesh {
		maximum_vertices: u32,
		maximum_primitives: u32,
		local_size: Extent,
	},
	Fragment,
}

pub enum MatrixLayouts {
	RowMajor,
	ColumnMajor,
}

pub struct GLSLSettings {
	pub(crate) version: String,
}

impl Default for GLSLSettings {
	fn default() -> Self {
		Self {
			version: "450".to_string(),
		}
	}
}

pub struct ShaderGenerationSettings {
	pub(crate) glsl: GLSLSettings,
	pub(crate) stage: Stages,
	pub(crate) matrix_layout: MatrixLayouts,
	pub(crate) name: String,
}

impl ShaderGenerationSettings {
	pub fn compute(extent: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Compute { local_size: extent })
	}

	pub fn task() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Task)
	}

	pub fn mesh(maximum_vertices: u32, maximum_primitives: u32, local_size: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Mesh{ maximum_vertices, maximum_primitives, local_size })
	}

	pub fn fragment() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Fragment)
	}

	pub fn vertex() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Vertex)
	}

	fn from_stage(stage: Stages) -> Self {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage, matrix_layout: MatrixLayouts::RowMajor, name: "shader".to_string() }
	}

	pub fn name(mut self, name: String) -> Self {
		self.name = name;
		self
	}
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

	pub fn bindings() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let mut root_node = besl::Node::root();

		let float_type = root_node.get_child("f32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding("buff", besl::BindingTypes::Buffer{ members: vec![besl::Node::member("member", float_type).into()] }, 0, 0, true, true).into(),
			besl::Node::binding("image", besl::BindingTypes::Image{ format: "r8".to_string() }, 0, 1, false, true).into(),
			besl::Node::binding("texture", besl::BindingTypes::CombinedImageSampler{ format: "".to_string() }, 1, 0, true, false).into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn specializations() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![
			besl::Node::specialization("color", vec3f_type).into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn fragment_shader() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn cull_unused_functions() -> besl::NodeReference {
		let script = r#"
		used_by_used: fn () -> void {}
		used: fn() -> void {
			used_by_used();
		}
		not_used: fn() -> void {}

		main: fn () -> void {
			used();
		}
		"#;

		let main_function_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		main
	}

	pub fn structure() -> besl::NodeReference {
		let script = r#"
		Vertex: struct {
			position: vec3f,
			normal: vec3f,
		}

		use_vertex: fn () -> Vertex {}

		main: fn () -> void {
			use_vertex();
		}
		"#;

		let main_function_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&main_function_node).get_child("main").unwrap();

		main
	}

	pub fn push_constant() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			push_constant;
		}
		"#;

		let mut root_node = besl::Node::root();

		let u32_t = root_node.get_child("u32").unwrap();
		root_node.add_child(besl::Node::push_constant(vec![besl::Node::member("material_id", u32_t.clone()).into()]).into());

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&program_node).get_child("main").unwrap();

		main
	}

	pub fn intrinsic() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			sample(number);
		}
		"#;

		use besl::parser::Node;

		let number_literal = Node::literal("number", Node::glsl("1.0", &[], Vec::new()));
		let sample_function = Node::intrinsic("sample", Node::parameter("num", "f32"), Node::sentence(vec![Node::glsl("0 + ", &[], Vec::new()), Node::member_expression("num"), Node::glsl(" * 2", &[], Vec::new())]), "f32");

		let mut root = besl::parse(&script).unwrap();

		root.add(vec![sample_function.clone(), number_literal.clone(),]);

		let root = besl::lex(root).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		main
	}
}