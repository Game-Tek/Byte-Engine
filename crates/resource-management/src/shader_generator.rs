use crate::shader_graph::{build_graph, topological_sort};
use utils::Extent;

/// Generates a graphics API consumable shader from a BESL shader program definition.
pub trait ShaderGenerator {}

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

/// The `ShaderFormatting` struct stores shared string formatting rules for shader generators.
#[derive(Clone, Copy)]
pub(crate) struct ShaderFormatting {
	minified: bool,
}

impl ShaderFormatting {
	pub(crate) fn new(minified: bool) -> Self {
		Self { minified }
	}

	pub(crate) fn break_str(&self) -> &'static str {
		if self.minified {
			""
		} else {
			"\n"
		}
	}

	pub(crate) fn space_str(&self) -> &'static str {
		if self.minified {
			""
		} else {
			" "
		}
	}

	pub(crate) fn comma_str(&self) -> &'static str {
		if self.minified {
			","
		} else {
			", "
		}
	}

	pub(crate) fn push_indentation(&self, string: &mut String, indent: usize) {
		if !self.minified {
			for _ in 0..indent {
				string.push('\t');
			}
		}
	}

	pub(crate) fn push_block_start(&self, string: &mut String) {
		if self.minified {
			string.push_str("){");
		} else {
			string.push_str(") {\n");
		}
	}

	pub(crate) fn push_statement_end(&self, string: &mut String) {
		if self.minified {
			string.push(';');
		} else {
			string.push_str(";\n");
		}
	}
}

/// Returns the reachable non-leaf shader nodes in emission order.
pub(crate) fn ordered_shader_nodes(main_function_node: &besl::NodeReference, backend_name: &str) -> Vec<besl::NodeReference> {
	if !matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }) {
		panic!(
			"{backend_name} shader generation requires a function node as the main function. The provided node was not a function."
		);
	}

	let graph = build_graph(main_function_node.clone());

	topological_sort(&graph)
		.into_iter()
		.filter(|node| {
			let borrowed = node.borrow();
			!borrowed.node().is_leaf()
				&& !matches!(borrowed.node(), besl::Nodes::Conditional { .. } | besl::Nodes::ForLoop { .. })
		})
		.collect()
}

pub(crate) fn emit_comma_separated_nodes<F>(
	string: &mut String,
	formatting: ShaderFormatting,
	nodes: &[besl::NodeReference],
	mut emit_node: F,
) where
	F: FnMut(&mut String, &besl::NodeReference),
{
	for (i, node) in nodes.iter().enumerate() {
		if i > 0 {
			string.push_str(formatting.comma_str());
		}

		emit_node(string, node);
	}
}

pub(crate) fn emit_statement_block<F>(
	string: &mut String,
	formatting: ShaderFormatting,
	statements: &[besl::NodeReference],
	indent: usize,
	mut emit_statement: F,
) where
	F: FnMut(&mut String, &besl::NodeReference),
{
	for statement in statements {
		formatting.push_indentation(string, indent);
		emit_statement(string, statement);
		formatting.push_statement_end(string);
	}
}

pub(crate) fn operator_token(operator: &besl::Operators) -> &'static str {
	match operator {
		besl::Operators::Plus => "+",
		besl::Operators::Minus => "-",
		besl::Operators::Multiply => "*",
		besl::Operators::Divide => "/",
		besl::Operators::Modulo => "%",
		besl::Operators::ShiftLeft => "<<",
		besl::Operators::ShiftRight => ">>",
		besl::Operators::BitwiseAnd => "&",
		besl::Operators::BitwiseOr => "|",
		besl::Operators::Assignment => "=",
		besl::Operators::Equality => "==",
		besl::Operators::LessThan => "<",
		besl::Operators::Inequality => "!=",
		besl::Operators::GreaterThan => ">",
		besl::Operators::LessThanOrEqual => "<=",
		besl::Operators::GreaterThanOrEqual => ">=",
		besl::Operators::LogicalAnd => "&&",
		besl::Operators::LogicalOr => "||",
	}
}

pub(crate) fn is_builtin_struct_type(name: &str, supports_atomic_u32: bool) -> bool {
	matches!(
		name,
		"void"
			| "vec2u16"
			| "vec2u" | "vec2i"
			| "vec2f" | "vec3f"
			| "vec4f" | "mat2f"
			| "mat3f" | "mat4f"
			| "f32" | "u8"
			| "u16" | "u32"
			| "i32" | "Texture2D"
			| "ArrayTexture2D"
	) || supports_atomic_u32 && name == "atomicu32"
}

impl ShaderGenerationSettings {
	pub fn compute(extent: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Compute { local_size: extent })
	}

	pub fn task() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Task)
	}

	pub fn mesh(maximum_vertices: u32, maximum_primitives: u32, local_size: Extent) -> ShaderGenerationSettings {
		Self::from_stage(Stages::Mesh {
			maximum_vertices,
			maximum_primitives,
			local_size,
		})
	}

	pub fn fragment() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Fragment)
	}

	pub fn vertex() -> ShaderGenerationSettings {
		Self::from_stage(Stages::Vertex)
	}

	fn from_stage(stage: Stages) -> Self {
		ShaderGenerationSettings {
			glsl: GLSLSettings::default(),
			stage,
			matrix_layout: MatrixLayouts::RowMajor,
			name: "shader".to_string(),
		}
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
			besl::Node::binding(
				"buff",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::member("member", float_type).into()],
				},
				0,
				0,
				true,
				true,
			)
			.into(),
			besl::Node::binding(
				"image",
				besl::BindingTypes::Image {
					format: "r8".to_string(),
				},
				0,
				1,
				false,
				true,
			)
			.into(),
			besl::Node::binding(
				"texture",
				besl::BindingTypes::CombinedImageSampler { format: "".to_string() },
				1,
				0,
				true,
				false,
			)
			.into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn same_named_buffer_member_access() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			pixel_mapping.pixel_mapping[0] = meshes.meshes[1];
		}
		"#;

		let mut root_node = besl::Node::root();
		let u32_type = root_node.get_child("u32").unwrap();

		root_node.add_children(vec![
			besl::Node::binding(
				"meshes",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("meshes", u32_type.clone(), 2)],
				},
				0,
				0,
				true,
				false,
			)
			.into(),
			besl::Node::binding(
				"pixel_mapping",
				besl::BindingTypes::Buffer {
					members: vec![besl::Node::array("pixel_mapping", u32_type, 2)],
				},
				0,
				1,
				false,
				true,
			)
			.into(),
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

		root_node.add_children(vec![besl::Node::specialization("color", vec3f_type).into()]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn input() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![besl::Node::input("color", vec3f_type, 0).into()]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn output() -> besl::NodeReference {
		let script = r#"
		main: fn () -> void {
			color;
		}
		"#;

		let mut root_node = besl::Node::root();

		let vec3f_type = root_node.get_child("vec3f").unwrap();

		root_node.add_children(vec![besl::Node::output("color", vec3f_type, 0).into()]);

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
			sample_user(number);
		}
		"#;

		use besl::parser::Node;

		let number_literal = Node::literal("number", Node::glsl("1.0", &[], &[]));
		let sample_function = Node::intrinsic(
			"sample_user",
			Node::parameter("num", "f32"),
			Node::sentence(vec![
				Node::glsl("0 + ", &[], &[]),
				Node::member_expression("num"),
				Node::glsl(" * 2", &[], &[]),
			]),
			"f32",
		);

		let mut root = besl::parse(&script).unwrap();

		root.add(vec![sample_function.clone(), number_literal.clone()]);

		let root = besl::lex(root).unwrap();

		let main = RefCell::borrow(&root).get_child("main").unwrap();

		main
	}

	pub fn const_variable() -> besl::NodeReference {
		let script = r#"
		PI: const f32 = 3.14;

		main: fn () -> void {
			PI;
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}

	pub fn return_value() -> besl::NodeReference {
		let script = r#"
		main: fn () -> f32 {
			return 1.0;
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();
		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		main
	}
}
