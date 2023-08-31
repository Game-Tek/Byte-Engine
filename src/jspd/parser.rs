//! The beshader_compiler module implements a compiler for the beshader language.
//! The beshader language is a simple, Rust-like language that is used to describe shaders.
//! This module relies on the [`shader_generator`](crate::shader_generator) module to generate the actual shader code.
//! 
//! # Example beShader
//! 
//! ```glsl
//! Light: struct {
//! 	position: vec3,
//! 	color: vec3,
//! }
//! 
//! main: fn () -> void {
//! 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
//! }
//! ```

/// Parse consumes an stream of tokens and return a JSPD describing the shader.
pub(crate) fn parse(tokens: Vec<String>) -> Result<Node, ()> {
	// let mut program_state = ProgramState {
	// 	node: jspd,
	// 	types: HashMap::new(),
	// };

	// program_state.types.insert("vec4f".into(), json::object! { "type": "struct", "x": { "type": "member", "data_type": "f32" }, "y": { "type": "member", "data_type": "f32" }, "z": { "type": "member", "data_type": "f32" }, "w": { "type": "member", "data_type": "f32" } });

	let mut iter = tokens.iter();

	let parsers = vec![
		parse_struct,
		parse_function,
		parse_macro,
	];

	let mut children = vec![];

	loop {
		let result = execute_parsers(&parsers, iter).or(Err(()))?;

		children.push(Rc::new(result.0));

		iter = result.1;

		if iter.len() == 0 {
			break;
		}
	}

	return Ok(Node::Root { children });
}

use std::{collections::HashMap, rc::Rc};

use super::{Node, Lexemes, Lexeme, Precedence};

enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax,
}

type ParsingResult<'a> = Result<(Node, std::slice::Iter<'a, String>), ParsingFailReasons>;
type Parser<'a> = fn(std::slice::Iter<'a, String>) -> ParsingResult<'a>;

type LexerReturn<'a> = Result<(Lexeme, std::slice::Iter<'a, String>), ParsingFailReasons>;
type Lexer<'a> = fn(std::slice::Iter<'a, String>) -> LexerReturn<'a>;

/// Execute a list of lexers on a stream of tokens.
fn execute_lexers<'a>(lexers: &[Lexer<'a>], iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	for lexer in lexers {
		if let Ok(r) = lexer(iterator.clone()) {
			return Ok(r);
		}
	}

	return Err(ParsingFailReasons::BadSyntax); // No lexer could handle this syntax.
}

/// Tries to execute a list of lexers on a stream of tokens. But it's ok if none of them can handle the syntax.
fn try_execute_lexers<'a>(lexers: &[Lexer<'a>], iterator: std::slice::Iter<'a, String>) -> Option<LexerReturn<'a>> {
	for lexer in lexers {
		if let Ok(r) = lexer(iterator.clone()) {
			return Some(Ok(r));
		}
	}

	return None;
}

/// Execute a list of parsers on a stream of tokens.
fn execute_parsers<'a>(parsers: &[Parser<'a>], iterator: std::slice::Iter<'a, String>) -> ParsingResult<'a> {
	for parser in parsers {
		if let Ok(r) = parser(iterator.clone()) {
			return Ok(r);
		}
	}

	return Err(ParsingFailReasons::BadSyntax); // No parser could handle this syntax.
}

fn parse_macro(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut iter = iterator;

	iter.next().and_then(|v| if v == "#" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().and_then(|v| if v == "[" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iter.next().ok_or(ParsingFailReasons::BadSyntax)?;
	iter.next().and_then(|v| if v == "]" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	return Ok((Node::Root { children: vec![]}, iter));
}

fn parse_struct(mut iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?.to_string();
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "struct" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	let mut fields = vec![];

	while let Some(v) = iterator.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iterator.next().unwrap();

		if colon != ":" { return Err(ParsingFailReasons::BadSyntax); }

		let type_name = iterator.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

		fields.push(Rc::new(Node::Member { name: v.to_string(), ty: None }));
	}

	return Ok((Node::Struct { name, fields }, iterator));
}

fn make_lexeme(cont: Option<Lexeme>, a: Lexemes, children: Option<Vec<Rc<Lexeme>>>) -> Lexeme {
	if let Some(cont) = cont {
		if cont.lexeme.precedence() > a.precedence() {
			let mut cont = cont.clone();
			cont.children.insert(0, Rc::new(make_lexeme(None, a, children)));
			cont
		} else {
			let lexeme = Lexeme {
				lexeme: a,
				children: vec![Rc::new(cont)],
			};
	
			lexeme
		}
	} else {
		Lexeme {
			lexeme: a,
			children: children.unwrap_or(vec![]),
		}
	}
}

fn parse_var_decl<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().ok_or(ParsingFailReasons::BadSyntax)?;

	let possible_following_expressions: Vec<Lexer> = vec![
		parse_assignment,
	];

	let (cont, new_iterator) = execute_lexers(&possible_following_expressions, iterator.clone())?;

	let lexeme = make_lexeme(Some(cont), Lexemes::VariableDeclaration, None);

	return Ok((lexeme, new_iterator));
}

fn parse_assignment<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	iterator.next().and_then(|v| if v == "=" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let possible_following_expressions: Vec<Lexer> = vec![
		parse_rvalue,
	];

	let (cont, new_iterator) = execute_lexers(&possible_following_expressions, iterator.clone())?;

	let lexeme = make_lexeme(Some(cont), Lexemes::Assignment, None);

	return Ok((lexeme, new_iterator));
}

fn parse_variable<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;

	let lexers: Vec<Lexer> = vec![
		parse_assignment,
	];

	if let Some(Ok((cont, new_iterator))) = try_execute_lexers(&lexers, iterator.clone()) {
		return Ok((make_lexeme(Some(cont), Lexemes::Member, None), new_iterator));
	} else {
		return Ok((make_lexeme(None, Lexemes::Member, None), iterator));
	}
}

fn parse_accessor<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?;

	let lexers: Vec<Lexer> = vec![
		parse_variable,
	];

	let (cont, new_iterator) = execute_lexers(&lexers, iterator.clone())?;

	return Ok((make_lexeme(Some(cont), Lexemes::Member, None), new_iterator));
}

fn parse_literal(mut iterator: std::slice::Iter<'_, String>) -> LexerReturn {
	let name = iterator.next().and_then(|v| if v == "1.0" || v == "0.0" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	return Ok((make_lexeme(None, Lexemes::Literal, None), iterator));
}

fn parse_rvalue<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	let parsers = vec![
		parse_function_call,
		parse_literal,
		parse_variable,
	];

	return execute_lexers(&parsers, iterator.clone());
}

fn parse_function_call(mut iterator: std::slice::Iter<'_, String>) -> LexerReturn {
	iterator.next().ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let mut children = vec![];

	loop {
		let (lexeme, new_iterator) = if let Ok(r) = parse_rvalue(iterator.clone()) { r } else { return Err(ParsingFailReasons::BadSyntax); };

		children.push(Rc::new(lexeme));

		iterator = new_iterator;

		// Check if iter is comma
		if iterator.clone().peekable().peek().unwrap().as_str() == "," {
			iterator.next();
		}

		// check if iter is close brace
		if iterator.clone().peekable().peek().unwrap().as_str() == ")" {
			iterator.next();
			break;
		}
	}

	return Ok((make_lexeme(None, Lexemes::FunctionCall, Some(children)), iterator));
}

fn parse_statement<'a>(mut iterator: std::slice::Iter<'a, String>) -> LexerReturn<'a> {
	let parsers = vec![
		parse_var_decl,
		parse_variable,
		parse_function_call,
	];

	let (lexeme, mut new_iterator) = if let Ok(r) = execute_lexers(&parsers, iterator.clone()) { r } else { return Err(ParsingFailReasons::BadSyntax); };

	new_iterator.next().and_then(|f| if f == ";" { Some(f) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	return Ok((lexeme, new_iterator));
}

fn parse_function<'a>(mut iterator: std::slice::Iter<'a, String>) -> ParsingResult<'a> {
	let name = iterator.next().ok_or(ParsingFailReasons::NotMine)?.to_string();
	iterator.next().and_then(|v| if v == ":" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "fn" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "(" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == ")" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;
	iterator.next().and_then(|v| if v == "->" { Some(v) } else { None }).ok_or(ParsingFailReasons::NotMine)?;

	let return_type = iterator.next().ok_or(ParsingFailReasons::BadSyntax)?;

	iterator.next().and_then(|v| if v == "{" { Some(v) } else { None }).ok_or(ParsingFailReasons::BadSyntax)?;

	let mut statements = vec![];

	loop {
		let (lexeme, new_iterator) = if let Ok(r) = parse_statement(iterator.clone()) { r } else { return Err(ParsingFailReasons::BadSyntax); };

		iterator = new_iterator;

		statements.push(Rc::new(lexeme));

		// check if iter is close brace
		if iterator.clone().peekable().peek().unwrap().as_str() == "}" {
			iterator.next();
			break;
		}
	}

	return Ok((Node::Function { name, params: vec![], return_type: Rc::new(Node::Root { children: vec![] }), statements }, iterator));
}

#[derive(Clone)]
struct ProgramState {
	node: json::JsonValue,
	types: HashMap<String, json::JsonValue>,
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::jspd::tokenizer::tokenize;

	#[test]
	fn test_parse_struct() {
		let source = "
Light: struct {
	position: vec3f,
	color: vec3f
}";

		let tokens = tokenize(source).unwrap();
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = &nodes.unwrap();

		if let (Node::Struct { name, fields }, light) = (&root_node["Light"], &root_node["Light"]) {
			assert_eq!(name, "Light");

			if let (Node::Member { name, ty }, position) = (&light["position"], &light["position"]) {
				assert_eq!(name, "position");
			} else {
				panic!("Not a member");
			}

			if let (Node::Member { name, ty }, color) = (&light["color"], &light["color"]) {
				assert_eq!(name, "color");
			} else {
				panic!("Not a member");
			}
		} else {
			panic!("Not a struct");
		}
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	position: vec4f = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = position;
}";

		let tokens = tokenize(source).unwrap();
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = nodes.unwrap();

		if let Node::Function { name, params, return_type, statements } = &root_node["main"] {
			assert_eq!(name, "main");
			assert_eq!(params.len(), 0);
			
			assert_eq!(statements.len(), 2);

			let statement = &statements[0];

			if let (Lexemes::Assignment, lexeme) = (statement.lexeme, statement) {
				assert_eq!(lexeme.children.len(), 2);

				let left = &lexeme.children[0];

				if let (Lexemes::VariableDeclaration, lexeme) = (left.lexeme, left) {
					assert_eq!(lexeme.children.len(), 0);
				} else {
					panic!("Not a member");
				}

				let right = &lexeme.children[1];

				if let (Lexemes::FunctionCall, lexeme) = (right.lexeme, right) {
					assert_eq!(lexeme.children.len(), 4);

					let param = &lexeme.children[0];

					if let (Lexemes::Literal, lexeme) = (param.lexeme, param) {
						assert_eq!(lexeme.children.len(), 0);
					} else {
						panic!("Not a literal");
					}

					let param = &lexeme.children[1];

					if let (Lexemes::Literal, lexeme) = (param.lexeme, param) {
						assert_eq!(lexeme.children.len(), 0);
					} else {
						panic!("Not a literal");
					}

					let param = &lexeme.children[2];

					if let (Lexemes::Literal, lexeme) = (param.lexeme, param) {
						assert_eq!(lexeme.children.len(), 0);
					} else {
						panic!("Not a literal");
					}

					let param = &lexeme.children[3];

					if let (Lexemes::Literal, lexeme) = (param.lexeme, param) {
						assert_eq!(lexeme.children.len(), 0);
					} else {
						panic!("Not a literal");
					}
				} else {
					panic!("Not a function call");
				}
			} else {
				panic!("Not an assignment");
			}
		} else {
			panic!("Not a function");
		}
	}

// 	#[test]
// 	fn test_parse_struct_and_function() {
// 		let source = "
// Light: struct {
// 	position: vec3f,
// 	color: vec3f
// }

// #[vertex]
// main: fn () -> void {
// 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
// 	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
// }";

// 		let tokens = tokenize(source).unwrap();
// 		let nodes = parse(tokens);

// 		assert_eq!(nodes.is_ok(), true);

// 		let root_node = nodes.unwrap();

// 		let struct_node = &root_node["Light"];

// 		assert_eq!(struct_node["type"], "struct");

// 		let position_node = &struct_node["position"];

// 		assert_eq!(&position_node["type"], "member");
// 		assert_eq!(&position_node["data_type"], "vec3f");

// 		let color_node = &struct_node["color"];

// 		assert_eq!(&color_node["type"], "member");
// 		assert_eq!(&color_node["data_type"], "vec3f");

// 		let function_node = &root_node["main"];

// 		assert_eq!(&function_node["type"], "function");
// 		assert_eq!(function_node["data_type"], "void");
// 	}
}