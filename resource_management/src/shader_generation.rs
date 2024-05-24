use std::{cell::RefCell, collections::{BTreeMap, HashMap, HashSet}};

use utils::Extent;

pub struct ShaderGenerator {
	minified: bool,
}

impl ShaderGenerator {
	pub fn new() -> Self {
		ShaderGenerator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	pub fn compilation(&self) -> ShaderCompilation {
		ShaderCompilation {
			minified: self.minified,
			present_symbols: HashSet::new(),
		}
	}
}

pub struct GLSLSettings {
	version: String,
}

impl Default for GLSLSettings {
	fn default() -> Self {
		Self {
			version: "450".to_string(),
		}
	}
}

enum Stages {
	Vertex,
	Compute {
		local_size: Extent,
	},
	Task,
	Mesh,
	Fragment,
}

pub struct ShaderGenerationSettings {
	glsl: GLSLSettings,
	stage: Stages,
}

impl ShaderGenerationSettings {
	pub fn compute(extent: Extent) -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: Stages::Compute { local_size: extent } }
	}

	pub fn task() -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: Stages::Task }
	}

	pub fn mesh() -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: Stages::Mesh }
	}

	pub fn fragment() -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: Stages::Fragment }
	}
	
	pub fn vertex() -> ShaderGenerationSettings {
		ShaderGenerationSettings { glsl: GLSLSettings::default(), stage: Stages::Vertex }
	}
}

pub struct ShaderCompilation {
	minified: bool,
	present_symbols: HashSet<besl::NodeReference>,
}

struct Graph {
	set: HashSet<(besl::NodeReference, besl::NodeReference)>,
}

impl Graph {
	pub fn new() -> Self {
		Graph {
			set: HashSet::new(),
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set.insert((from, to));
	}
}

fn topological_sort(graph: &Graph) -> Vec<besl::NodeReference> {
	let mut in_degree = HashMap::new();
	let mut queue = Vec::new();
	let mut result = Vec::new();

	for (from, to) in &graph.set {
		*in_degree.entry(to).or_insert(0) += 1;
	}

	for (from, to) in &graph.set {
		if !in_degree.contains_key(from) {
			queue.push(from.clone());
		}
	}

	while !queue.is_empty() {
		let node = queue.pop().unwrap();
		result.push(node.clone());

		for (from, to) in &graph.set {
			if from == &node {
				*in_degree.get_mut(to).unwrap() -= 1;
				if *in_degree.get(to).unwrap() == 0 {
					queue.push(to.clone());
				}
			}
		}
	}

	result
}

impl ShaderCompilation {
	pub fn generate_shader(&mut self, main_function_node: &besl::NodeReference) -> String {
		let mut string = String::with_capacity(2048);
	
		let graph = self.build_graph(main_function_node.clone());

		let order = topological_sort(&graph);
		
		for node in order {
			self.emit_node_string(&mut string, &node);
		}
	
		string
	}

	pub fn generate_glsl_shader(&mut self, shader_compilation_settings: &ShaderGenerationSettings, main_function_node: &besl::NodeReference) -> String {
		let mut string = String::with_capacity(2048);
		
		if !matches!(main_function_node.borrow().node(), besl::Nodes::Function { .. }) {
			panic!("GLSL shader generation requires a function node as the main function.");
		}

		let graph = self.build_graph(main_function_node.clone());

		let order = topological_sort(&graph);

		self.generate_glsl_header_block(&mut string, shader_compilation_settings);
		
		for node in order {
			self.emit_node_string(&mut string, &node);
		}
	
		string
	}

	fn translate_type(source: &str) -> &str {
		match source {
			"void" => "void",
			"vec2f" => "vec2",
			"vec2u16" => "u16vec2",
			"vec3u" => "uvec3",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			_ => source,
		}
	}

	fn build_graph(&mut self, node: besl::NodeReference) -> Graph {
		let mut graph = Graph::new();

		graph = self.build_graph_inner(node, graph);

		graph
	}

	fn build_graph_inner(&mut self, node: besl::NodeReference, mut graph: Graph) -> Graph {
		if self.present_symbols.contains(&node) { return graph; }

		let node = RefCell::borrow(&node);
		let node = node.node();

		match node {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { children, .. } => {
				for child in children {
					graph = self.build_graph_inner(child.clone(), graph);
				}
			}
			besl::Nodes::Function { statements, .. } => {
				for statement in statements {
					graph = self.build_graph_inner(statement.clone(), graph);
				}
			}
			besl::Nodes::Struct { fields, .. } => {
				graph.add(from, to);

				for field in fields {
					graph = self.build_graph_inner(field.clone(), graph);
				}
			}
			besl::Nodes::PushConstant { members } => {
				for member in members {
					graph = self.build_graph_inner(member.clone(), graph);
				}
			}
			besl::Nodes::Specialization { r#type, .. } => {
				graph = self.build_graph_inner(r#type.clone(), graph);
			}
			besl::Nodes::Member { r#type, .. } => {
				self.build_graph_inner(r#type.clone(), graph);
			}
			besl::Nodes::GLSL { input, output, .. } => {
				for reference in input {
					graph = self.build_graph_inner(reference.clone(), graph);
				}

				for reference in output {
					graph = self.build_graph_inner(reference.clone(), graph);
				}
			}
			besl::Nodes::Parameter { r#type, .. } => {
				self.build_graph_inner(r#type.clone(), graph);
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { operator, left, right } => {
						if operator == &besl::Operators::Assignment {
							graph = self.build_graph_inner(left.clone(), graph);
							graph = self.build_graph_inner(right.clone(), graph);
						}
					}
					besl::Expressions::FunctionCall { parameters, function, .. } => {
						graph = self.build_graph_inner(function.clone(), graph);

						for parameter in parameters {
							graph = self.build_graph_inner(parameter.clone(), graph);
						}
					}
					besl::Expressions::IntrinsicCall { elements: parameters, .. } => {
						for e in parameters {
							graph = self.build_graph_inner(e.clone(), graph);
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							graph = self.build_graph_inner(element.clone(), graph);
						}
					}
					besl::Expressions::Macro { body, .. } => {
						graph = self.build_graph_inner(body.clone(), graph);
					}
					besl::Expressions::Member { source, .. } => {
						match source.borrow().node() {
							besl::Nodes::Expression { .. } => {}
							besl::Nodes::Literal { .. } => {
								graph = self.build_graph_inner(source.clone(), graph);
							}
							besl::Nodes::Member { .. } => {}
							_ => {
								graph = self.build_graph_inner(source.clone(), graph);
							}
						}
					}
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						graph = self.build_graph_inner(r#type.clone(), graph);
					}
					besl::Expressions::Literal { .. } => {
						// graph = self.build_graph_inner(value.clone(), graph);
					}
					besl::Expressions::Return => {}
					besl::Expressions::Accessor { left, right } => {
						graph = self.build_graph_inner(left.clone(), graph);
						graph = self.build_graph_inner(right.clone(), graph);
					}
				}
			}
			besl::Nodes::Binding { r#type, .. } => {
				match r#type {
					besl::BindingTypes::Buffer{ r#type } => {
						graph = self.build_graph_inner(r#type.clone(), graph);
					}
					besl::BindingTypes::Image { .. } => {}
					besl::BindingTypes::CombinedImageSampler => {}
				}
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					graph = self.build_graph_inner(element.clone(), graph);
				}
			}
			besl::Nodes::Literal { value, .. } => {
				graph = self.build_graph_inner(value.clone(), graph);
			}
		}

		graph
	}

	fn emit_node_string(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(&this_node);
	
		match node.node() {
			besl::Nodes::Null => {}
			besl::Nodes::Scope { .. } => {}
			besl::Nodes::Function { name, statements, return_type, params, .. } => {
				string.push_str(Self::translate_type(&return_type.borrow().get_name().unwrap()));

				string.push(' ');

				string.push_str(name);

				string.push('(');

				for (i, param) in params.iter().enumerate() {
					if i > 0 {
						if !self.minified { string.push_str(", "); } else { string.push(','); }
					}
				}

				if self.minified { string.push_str("){"); } else { string.push_str(") {\n"); }
	
				for statement in statements {
					if !self.minified { string.push('\t'); }
					if !self.minified { string.push_str(";\n"); } else { string.push(';'); }
				}
				
				if self.minified { string.push('}') } else { string.push_str("}\n"); }
			}
			besl::Nodes::Struct { name, fields, .. } => {
				if name == "void" || name == "vec2u16" || name == "vec2f" || name == "vec3f" || name == "vec4f" || name == "mat2f" || name == "mat3f" || name == "mat4f" || name == "f32" || name == "u8" || name == "u16" || name == "u32" || name == "i32" { return; }

				string.push_str("struct ");
				string.push_str(name.as_str());

				if self.minified { string.push('{'); } else { string.push_str(" {\n"); }

				for field in fields {
					if !self.minified { string.push('\t'); }
					if self.minified { string.push(';') } else { string.push_str(";\n"); }
				}

				string.push_str("};");

				if !self.minified { string.push('\n'); }
			}
			besl::Nodes::PushConstant { members } => {
				string.push_str("layout(push_constant) uniform PushConstant {");

				if !self.minified { string.push('\n'); }

				for member in members {
					if !self.minified { string.push('\t'); }
					if self.minified { string.push(';') } else { string.push_str(";\n"); }
				}

				string.push_str("} push_constant;");

				if !self.minified { string.push('\n'); }
			}
			besl::Nodes::Specialization { name, r#type } => {
				let mut l_string = String::with_capacity(128);

				let mut members = Vec::new();

				let t = &r#type.borrow().get_name().unwrap();
				let type_name = Self::translate_type(t);

				match r#type.borrow().node() {
					besl::Nodes::Struct { fields, .. } => {
						for (i, field) in fields.iter().enumerate() {
							match field.borrow().node() {
								besl::Nodes::Member { name: member_name, r#type, .. } => {
									let member_name = format!("{}_{}", name, {member_name});
									l_string.push_str(&format!("layout(constant_id={}) const {} {} = {};\n", i, Self::translate_type(&r#type.borrow().get_name().unwrap()), &member_name, "1.0"));
									members.push(member_name);
								}
								_ => {}
							}
						}
					}
					_ => {}
				}

				l_string.push_str(&format!("const {} {} = {};\n", &type_name, name, format!("{}({})", &type_name, members.join(","))));

				string.insert_str(0, &l_string);
			}
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					let type_name = Self::translate_type(type_name.as_str());

					string.push_str(type_name);
					string.push(' ');
				}
				string.push_str(name.as_str());
				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}
			}
			besl::Nodes::GLSL { code, input, .. } => {
				for reference in input {
				}

				string.push_str(code);
			}
			besl::Nodes::Parameter { name, r#type } => {
				string.push_str(&format!("{} {}", Self::translate_type(&r#type.borrow().get_name().unwrap()), name));
			}
			besl::Nodes::Expression(expression) => {
				match expression {
					besl::Expressions::Operator { operator, left, right } => {
						if operator == &besl::Operators::Assignment {
							if self.minified { string.push('=') } else { string.push_str(" = "); }
						}
					}
					besl::Expressions::FunctionCall { parameters, function, .. } => {
						let function = RefCell::borrow(&function);
						let name = function.get_name().unwrap();

						let name = Self::translate_type(&name);

						string.push_str(&format!("{}(", name));
						for (i, parameter) in parameters.iter().enumerate() {
							if i > 0 {
								if self.minified { string.push(',') } else { string.push_str(", "); }
							}
						}
						string.push_str(&format!(")"));
					}
					besl::Expressions::IntrinsicCall { elements: parameters, .. } => {
						for e in parameters {
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
						}
					}
					besl::Expressions::Macro { body, .. } => {
					}
					besl::Expressions::Member { name, source, .. } => {
						match source.borrow().node() {
							besl::Nodes::Expression { .. } => {
								string.push_str(name);
							}
							besl::Nodes::Literal { .. } => {
							}
							besl::Nodes::Member { .. } => { // If member being accessed belongs to a struct don't generate the "member definifition" it already existing inside the member's struct
								string.push_str(name);
							}
							_ => {
								string.push_str(name);
							}
						}						
					}
					besl::Expressions::VariableDeclaration { name, r#type } => {
						string.push_str(&format!("{} {}", Self::translate_type(&r#type.borrow().get_name().unwrap()), name));
					}
					besl::Expressions::Literal { value } => {
						string.push_str(&format!("{}", value));
					}
					besl::Expressions::Return => {
						string.push_str("return");
					}
					besl::Expressions::Accessor { left, right } => {
						string.push('.');
					}
				}
			}
			besl::Nodes::Binding { name, set, binding, read, write, r#type, count, .. } => {
				let binding_type = match r#type {
					besl::BindingTypes::Buffer{ .. } => "buffer",
					besl::BindingTypes::Image{ format, .. } => {
						match format.as_str() {
							"r8ui" | "r16ui" | "r32ui" => "uniform uimage2D",
							_ => "uniform image2D"
						}
					},
					besl::BindingTypes::CombinedImageSampler => "uniform sampler2D",
				};

				string.push_str(&format!("layout(set={},binding={}", set, binding));

				match r#type {
					besl::BindingTypes::Buffer{ .. } => {
						string.push_str(",scalar");
					}
					besl::BindingTypes::Image { format } => {
						string.push(',');
						string.push_str(&format);
					}
					besl::BindingTypes::CombinedImageSampler => {}
				}

				match r#type {
					besl::BindingTypes::Buffer{ .. } | besl::BindingTypes::Image { .. } => {
						string.push_str(&format!(") {}{} ", if *read && !*write { "readonly " } else if *write && !*read { "writeonly " } else { "" }, binding_type));
					}
					besl::BindingTypes::CombinedImageSampler => {
						string.push_str(&format!(") {} ", binding_type));
					}
				}

				match r#type {
					besl::BindingTypes::Buffer{ r#type } => {						
						match RefCell::borrow(&r#type).node() {
							besl::Nodes::Struct { name, fields, .. } => {
								string.push_str(&name);
								string.push('{');

								if !self.minified { string.push('\n'); }

								for field in fields {
									if !self.minified { string.push('\t'); }
									if self.minified { string.push(';') } else { string.push_str(";\n"); }
								}

								string.push('}');
							}
							_ => { panic!("Need struct node type for buffer binding type."); }
						}
					}
					_ => {}
				}

				string.push_str(&name);

				if let Some(count) = count {
					string.push('[');
					string.push_str(count.to_string().as_str());
					string.push(']');
				}

				if !self.minified { string.push_str(";\n"); } else { string.push(';'); }
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
				}
			}
			besl::Nodes::Literal { value, .. } => {
			}
		}
	}

	fn generate_glsl_header_block(&self, glsl_block: &mut String, compilation_settings: &ShaderGenerationSettings) {
		let glsl_version = &compilation_settings.glsl.version;
	
		glsl_block.push_str(&format!("#version {glsl_version} core\n"));
	
		// shader type
	
		match compilation_settings.stage {
			Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			Stages::Task => glsl_block.push_str("#pragma shader_stage(task)\n"),
			Stages::Mesh => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
		}
	
		// extensions
	
		glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
		glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
		glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
		glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");
	
		match compilation_settings.stage {
			Stages::Compute { local_size } => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_arithmetic:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:enable\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_shuffle:enable\n");
				glsl_block.push_str(&format!("layout(local_size_x={},local_size_y={},local_size_z={}) in;\n", local_size.width(), local_size.height(), local_size.depth()));
			}
			Stages::Mesh => {
				glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
				// TODO: make this next lines configurable
				glsl_block.push_str("layout(location=0) perprimitiveEXT out uint out_instance_index[126];\n");
				glsl_block.push_str("layout(location=1) perprimitiveEXT out uint out_primitive_index[126];\n");
				glsl_block.push_str("layout(triangles,max_vertices=64,max_primitives=126) out;\n");
				glsl_block.push_str("layout(local_size_x=128) in;\n");
			}
			_ => {}
		}

		// memory layout declarations
		glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;");

		glsl_block.push_str("const float PI = 3.14159265359;\n");

		if !self.minified { glsl_block.push('\n'); }
	}
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::shader_generation::ShaderGenerator;

	#[test]
	fn empty_script() {
		let script = r#"
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&script_node);

		println!("{}", shader);
	}

	#[test]
	fn bindings() {
		let script = r#"
		main: fn () -> void {
			buff;
			image;
			texture;
		}
		"#;

		let buffer_type = besl::Node::r#struct("BufferType", vec![]).into();

		let mut root_node = besl::Node::scope("root".to_string());
		
		root_node.add_children(vec![
			besl::Node::binding("buff", besl::BindingTypes::buffer(buffer_type), 0, 0, true, true).into(),
			besl::Node::binding("image", besl::BindingTypes::Image{ format: "r8".to_string() }, 0, 1, false, true).into(),
			besl::Node::binding("texture", besl::BindingTypes::CombinedImageSampler, 1, 0, true, false).into(),
		]);

		let script_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "layout(set=1,binding=0) uniform sampler2D texture;layout(set=0,binding=1,r8) writeonly uniform image2D image;layout(set=0,binding=0,scalar) buffer BufferType{}buff;void main(){buff;image;texture;}");
	}

	#[test]
	fn fragment_shader() {
		let script = r#"
		main: fn () -> void {
			let albedo: vec3f = vec3f(1.0, 0.0, 0.0);
		}
		"#;

		let script_node = besl::compile_to_besl(&script, None).unwrap();

		let main = RefCell::borrow(&script_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main() {\n\tvec3 albedo = vec3(1.0, 0.0, 0.0);\n}\n");

		let shader_generator = ShaderGenerator::new().minified(true);

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void main(){vec3 albedo=vec3(1.0,0.0,0.0);}");
	}

	#[test]
	fn cull_unused_functions() {
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

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "void used_by_used() {\n}\nvoid used() {\n\tused_by_used();\n}\nvoid main() {\n\tused();\n}\n");
	}

	#[test]
	fn structure() {
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

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main);

		assert_eq!(shader, "struct Vertex {\n\tvec3 position;\n\tvec3 normal;\n};\nVertex use_vertex() {\n}\nvoid main() {\n\tuse_vertex();\n}\n");
	}

	#[test]
	fn push_constant() {
		let script = r#"
		main: fn () -> void {
			push_constant;
		}
		"#;

		let mut root_node = besl::Node::root();

		let u32_t = root_node.get_child("u32").unwrap();
		root_node.add_child(besl::Node::push_constant(vec![besl::Node::member("material_id", u32_t.clone()).into()]).into());

		let program_node = besl::compile_to_besl(&script, Some(root_node)).unwrap();

		let main_node = RefCell::borrow(&program_node).get_child("main").unwrap();

		let shader_generator = ShaderGenerator::new();

		let shader = shader_generator.compilation().generate_shader(&main_node);

		assert_eq!(shader, "layout(push_constant) uniform PushConstant {\n\tuint32_t material_id;\n} push_constant;\nvoid main() {\n\tpush_constant;\n}\n");
	}

	#[test]
	#[ignore = "BROKEN! TODO: FIX"]
	fn test_instrinsic() {
		let script = r#"
		main: fn () -> void {
			sample(number);
		}
		"#;

		use besl::parser::Node;

		let number_literal = Node::literal("number", Node::glsl("1.0", Vec::new(), Vec::new()));
		let sample_function = Node::intrinsic("sample", Node::parameter("num", "f32"), Node::sentence(vec![Node::glsl("0 + ", Vec::new(), Vec::new()), Node::member_expression("num"), Node::glsl(" * 2", Vec::new(), Vec::new())]), "f32");

		let mut program_state = besl::parse(&script).unwrap();

		// let main = program_state.get("main").unwrap();

		// let root = besl::lex(besl::parser::NodeReference::root_with_children(vec![sample_function.clone(), number_literal.clone(), main.clone()]), &program_state).unwrap();

		// let main = root.borrow().get_main().unwrap();

		// let shader_generator = ShaderGenerator::new();

		// let shader = shader_generator.compilation().generate_shader(&main);

		// assert_eq!(shader, "void main() {\n\t0 + 1.0 * 2;\n}\n");
	}
}