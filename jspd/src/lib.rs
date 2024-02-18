//! This module contains all code related to the parsing of the BESL language and the generation of the JSPD.

#![feature(new_uninit)]

use std::{collections::HashMap, rc::Rc};

mod tokenizer;
mod parser;
pub mod lexer;

pub use lexer::Expressions;
pub use lexer::Operators;
pub use lexer::Node;
pub use lexer::Nodes;

pub use crate::lexer::NodeReference;

pub fn compile_to_jspd(source: &str) -> Result<NodeReference, CompilationError> {
	if source.split_whitespace().next() == None {
		return Ok(lexer::Node::scope("".to_string(), Vec::new()));
	}

	let tokens = tokenizer::tokenize(source).map_err(|_e| CompilationError::Undefined)?;
	let (parser_root_node, parser_program) = parser::parse(tokens).map_err(|_e| CompilationError::Undefined)?;
	let jspd = lexer::lex(&parser_root_node, &parser_program).map_err(|_e| CompilationError::Undefined)?;

	Ok(jspd)
}

#[derive(Debug)]
pub enum CompilationError {
	Undefined,
}

// Expects a JSON object, describing the program in a parsed form.
pub fn json_to_jspd(source: &json::JsonValue) -> Result<NodeReference, ()> {
	fn process_parser_nodes(name: &str, node: &json::JsonValue, parser_program: &mut parser::ProgramState) -> Result<Rc<parser::Node>, ()> {
		let parser_node = match node {
			json::JsonValue::Object(obj) => {
				match obj["type"].as_str().unwrap() {
					"struct" => {
						let node = parser::Node {
							node: parser::Nodes::Struct { 
								name: name.to_string(),
								fields: obj.iter().filter(|(key, _value)| key != &"name" && key != &"type").map(|(key, value)| {
									process_parser_nodes(key, value, parser_program).unwrap()
								}).collect::<Vec<Rc<parser::Node>>>(),
							},
						};

						parser_program.types.insert(name.to_string(), Rc::new(node.clone()));

						node
					}
					"scope" => {
						parser::Node {
							node: parser::Nodes::Scope {
								name: name.to_string(),
								children: obj.iter().filter(|(key, _value)| key != &"name" && key != &"type" && key != &"__only_under").map(|(key, value)| {
									process_parser_nodes(key, value, parser_program).unwrap()
								}).collect::<Vec<Rc<parser::Node>>>(),
							},
						}
					}
					"function" => {
						parser::Node {
							node: parser::Nodes::Function {
								name: name.to_string(),
								params: Vec::new(),
								return_type: obj["return_type"].as_str().unwrap().to_string(),
								raw: None,
								statements: Vec::new(),
							},
						}
					}
					"push_constant" => {
						parser::Node {
							node: parser::Nodes::Member {
								name: name.to_string(),
								r#type: format!("PushConstant<{}>", obj["data_type"].as_str().unwrap()),
							},
						}
					}
					"in" => {
						parser::Node {
							node: parser::Nodes::Member {
								name: name.to_string(),
								r#type: format!("In<{}>", obj["data_type"].as_str().unwrap()),
							},
						}
					}
					"out" => {
						parser::Node {
							node: parser::Nodes::Member {
								name: name.to_string(),
								r#type: format!("Out<{}>", obj["data_type"].as_str().unwrap()),
							},
						}
					}
					"member" => {
						parser::Node {
							node: parser::Nodes::Member {
								name: name.to_string(),
								r#type: obj["data_type"].as_str().unwrap().to_string(),
							},
						}
					}
					_ => { panic!("Unsupported node type;") }
				}
			}
			_ => { panic!("Unsupported node type;") }
		};

		Ok(Rc::new(parser_node))
	}

	let mut parser_program = parser::ProgramState {
		types: HashMap::new(),
	};

	let root_parser_node = process_parser_nodes("root", source, &mut parser_program).map_err(|_e| ())?;

	parser::declare_intrinsic_types(&mut parser_program);

	lexer::lex(&root_parser_node, &parser_program).map_err(|_e| ())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_json() {
		let source = r#"{
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
					"data_type": "mat4f"
				},
				"projection": {
					"type": "member",
					"data_type": "mat4f"
				},
				"view_projection": {
					"type": "member",
					"data_type": "mat4f"
				}
			},
			"Mesh": {
				"type": "struct",
				"model": {
					"type": "member",
					"data_type": "mat4f"
				}
			},
			"Vertex": {
				"type": "scope",
				"__only_under": "Vertex",
				"in_position": {
					"type": "in",
					"data_type": "vec3f"
				},
				"in_normal": {
					"type": "in",
					"data_type": "vec3f"
				},
				"out_instance_index": {
					"type": "out",
					"data_type": "u32",
					"interpolation": "flat"
				}
			},
			"Fragment": {
				"type": "scope",
				"__only_under": "Fragment",
				"in_instance_index": {
					"type": "in",
					"data_type": "u32",
					"interpolation": "flat"
				},
				"out_material_index": {
					"type": "out",
					"data_type": "u32"
				}
			}
		}"#;

		let json = json::parse(&source).unwrap();

		let jspd = json_to_jspd(&json).unwrap();

		let jspd = jspd.borrow();

		if let lexer::Nodes::Scope { name, children } = jspd.node() {
			assert_eq!(name, "root");
			assert!(children.len() > 1);
		} else {
			panic!("Root node is not a scope.");
		}
	}
}