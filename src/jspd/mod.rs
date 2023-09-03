//! This module contains all code related to the parsing of the BESL language and the generation of the JSPD.



mod tokenizer;
mod parser;
pub mod lexer;

pub(crate) fn compile_to_jspd(source: &str) -> Result<lexer::Node, CompilationError> {
	let tokens = tokenizer::tokenize(source).map_err(|_e| CompilationError::Undefined)?;
	let (parser_root_node, parser_program) = parser::parse(tokens).map_err(|_e| CompilationError::Undefined)?;
	let jspd = lexer::lex(&parser_root_node, &parser_program).map_err(|_e| CompilationError::Undefined)?;

	return Ok(jspd);
}

#[derive(Debug)]
pub(crate) enum CompilationError {
	Undefined,
}

// pub(crate) fn json_to_jspd(source: &json::JsonValue) -> Result<Node, ()> {
// 	fn process_node(node: &json::JsonValue) -> Result<Node, ()> {
// 		match node {
// 			json::JsonValue::Object(obj) => {
// 				match obj["type"].as_str().unwrap() {
// 					"struct" => {
// 					}
// 				}

// 				for entry in obj {
// 					process_node(entry);
// 				}
// 			}
// 			_ => { Err(()) }
// 		}
// 	}

// 	return process_node(node);
// }