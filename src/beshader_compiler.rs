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

fn tokenize(source: &str) -> Vec<String> {
	let interrupt = |c: char| -> bool {
		return c.is_whitespace();
	};

	let can_sequence_continue = |sequence: &str, c: char| -> bool {
		if sequence.len() == 0 { return true; }

		let last = sequence.chars().last().unwrap();

		if last.is_alphabetic() {
			return c.is_alphanumeric() || c == '_';
		} else if last.is_numeric() {
			return c.is_numeric() || c == '.';
		} else if last == '.' {
			return c.is_numeric();
		} else if last == '_' {
			return c.is_alphanumeric() || c == '_';
		} else if last == '-' && c == '>' {
			return true;
		} else {
			return false;
		}
	};

	let mut tokens = Vec::new();
	let mut chars = source.chars();
	let mut iterator = chars.next();

	'outer: loop {
		let mut token = String::new();

		'inner: loop {
			match iterator {
				Some(c) => {
					if interrupt(c) {
						iterator = chars.next();
						break 'inner;
					} else if can_sequence_continue(&token, c) {
						token.push(c);
						iterator = chars.next();
					} else {
						break 'inner;
					}
				},
				None => {
					if token.len() > 0 {
						tokens.push(token);
					}

					break 'outer;
				},
			}
		}

		if token.len() > 0 {
			tokens.push(token);
		}
	}

	return tokens;
}

struct Node {
	class: String,
	name: String,
	children: Vec<Node>,
	properties: std::collections::HashMap<String, Vec<Node>>,
}

enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax,
}

type ParsingResult<'a> = Result<(Node, std::slice::Iter<'a, String>), ParsingFailReasons>;

fn parse_macro(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut iter = iterator;

	let hash = iter.next().unwrap();

	if hash != "#" { return Err(ParsingFailReasons::NotMine); }

	let square_bracket = iter.next().unwrap();

	if square_bracket != "[" { return Err(ParsingFailReasons::NotMine); }

	let name = iter.next().unwrap();

	if !name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

	let close_square_bracket = iter.next().unwrap();

	if close_square_bracket != "]" { return Err(ParsingFailReasons::BadSyntax); }

	let root_node = Node { class: "macro".to_owned(), name: name.to_owned(), children: Vec::new(), properties: std::collections::HashMap::new() };

	return Ok((root_node, iter));
}

fn parse_struct(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut root_node = Node { class: "struct".to_owned(), name: String::new(), children: Vec::new(), properties: std::collections::HashMap::new() };

	let mut iter = iterator;

	let name = iter.next().unwrap();

	if !name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::NotMine); }

	let colon = iter.next().unwrap();

	if colon != ":" { return Err(ParsingFailReasons::NotMine); }

	let struct_token = iter.next().unwrap();

	if struct_token != "struct" { return Err(ParsingFailReasons::NotMine); }

	root_node.name = name.to_owned();

	let open_brace = iter.next().unwrap();

	if open_brace != "{" { return Err(ParsingFailReasons::BadSyntax); }

	let mut current_node = &mut root_node;

	'field: while let Some(v) = iter.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iter.next().unwrap();

		if colon != ":" { return Err(ParsingFailReasons::BadSyntax); }

		let type_name = iter.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

		root_node.children.push(Node { class: "field".to_owned(), name: v.to_owned(), children: Vec::new(), properties: std::collections::HashMap::new() });
	}

	return Ok((root_node, iter));
}

fn parse_statement(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut root_node = Node { class: "statement".to_owned(), name: String::new(), children: Vec::new(), properties: std::collections::HashMap::new() };

	let mut iter = iterator;

	while let Some(v) = iter.next() {
		if v == ";" {
			break;
		}

		dbg!(&v);

		root_node.children.push(Node { class: "expression".to_owned(), name: v.to_owned(), children: Vec::new(), properties: std::collections::HashMap::new() });
	}

	return Ok((root_node, iter));
}

fn parse_function(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut root_node = Node { class: "function".to_owned(), name: String::new(), children: Vec::new(), properties: std::collections::HashMap::new() };

	let mut iter = iterator;

	let name = iter.next().unwrap();

	if !name.chars().next().unwrap().is_alphabetic() {
		return Err(ParsingFailReasons::NotMine);
	}

	root_node.name = name.to_owned();

	let colon = iter.next().unwrap();

	if colon != ":" {
		return Err(ParsingFailReasons::NotMine);
	}

	let fn_token = iter.next().unwrap();

	if fn_token != "fn" { return Err(ParsingFailReasons::NotMine); }

	let open_brace = iter.next().unwrap();

	if open_brace != "(" {
		return Err(ParsingFailReasons::BadSyntax);
	}

	let close_brace = iter.next().unwrap();

	if close_brace != ")" {
		return Err(ParsingFailReasons::BadSyntax);
	}

	let arrow = iter.next().unwrap();

	if arrow != "->" {
		return Err(ParsingFailReasons::BadSyntax);
	}

	let return_type = iter.next().unwrap();

	if !return_type.chars().next().unwrap().is_alphabetic() {
		return Err(ParsingFailReasons::BadSyntax);
	}

	root_node.properties.insert("return_type".to_owned(), vec![Node { class: "type".to_owned(), name: return_type.to_owned(), children: Vec::new(), properties: std::collections::HashMap::new() }]);

	let open_brace = iter.next().unwrap();

	if open_brace != "{" {
		return Err(ParsingFailReasons::BadSyntax);
	}

	loop  {
		if let Ok(r) = parse_statement(iter.clone()) {
			iter = r.1;
			root_node.children.push(r.0);
		} else {
			return Err(ParsingFailReasons::BadSyntax);
		}

		// check if iter is close brace
		if iter.clone().peekable().peek().unwrap().as_str() == "}" {
			iter.next();
			break;
		}
	}

	return Ok((root_node, iter));
}

fn parse(tokens: Vec<String>) -> Result<Node, ()> {
	let mut root_node = Node { class: "root".to_owned(), name: "root".to_owned(), children: Vec::new(), properties: std::collections::HashMap::new() };

	let mut iter = tokens.iter();

	loop {
		let result;

		if let Ok(r) = parse_struct(iter.clone()) {
			result = r;
		} else if let Ok(r) = parse_function(iter.clone()) {
			result = r;
		} else if let Ok(r) = parse_macro(iter.clone()) {
			iter = r.1;
			continue;
			//result = r;
		} else {
			return Err(()); // No parser could handle the expression.
		}

		root_node.children.push(result.0);

		iter = result.1;

		if iter.len() == 0 {
			break;
		}
	}

	return Ok(root_node);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_tokenization() {
		let source = "void main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source);
		assert_eq!(tokens, vec!["void", "main", "(", ")", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", ";", "}"]);
	}

	#[test]
	fn test_tokenization2() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source);
		assert_eq!(tokens, vec!["fn", "main", "(", ")", "->", "void", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", ";", "}"]);
	}

	#[test]
	fn test_tokenization3() {
		let source = "struct Light { position: vec3, color: vec3, data: Data<int>, array: [u8; 4] };";
		let tokens = tokenize(source);
		assert_eq!(tokens, vec!["struct", "Light", "{", "position", ":", "vec3", ",", "color", ":", "vec3", ",", "data", ":", "Data", "<", "int", ">", ",", "array", ":", "[", "u8", ";", "4", "]", "}", ";"]);
	}

	#[test]
	fn test_parse_struct() {
		let source = "
Light: struct {
	position: vec3,
	color: vec3
}";

		let tokens = tokenize(source);
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = nodes.unwrap();

		assert_eq!(&root_node.class, "root");
		assert_eq!(&root_node.name, "root");
		assert_eq!(root_node.children.len(), 1);

		let struct_node = &root_node.children[0];

		assert_eq!(&struct_node.class, "struct");
		assert_eq!(&struct_node.name, "Light");
		assert_eq!(struct_node.children.len(), 2);

		let position_node = &struct_node.children[0];

		assert_eq!(&position_node.class, "field");
		assert_eq!(&position_node.name, "position");
		assert_eq!(position_node.children.len(), 0);

		let color_node = &struct_node.children[1];

		assert_eq!(&color_node.class, "field");
		assert_eq!(&color_node.name, "color");
		assert_eq!(color_node.children.len(), 0);
	}

	#[test]
	fn test_parse_function() {
		let source = "
main: fn () -> void {
	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
}";

		let tokens = tokenize(source);
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = nodes.unwrap();

		assert_eq!(&root_node.class, "root");
		assert_eq!(&root_node.name, "root");
		assert_eq!(root_node.children.len(), 1);

		let function_node = &root_node.children[0];

		assert_eq!(&function_node.class, "function");
		assert_eq!(&function_node.name, "main");
		assert_eq!(function_node.children.len(), 1);

		let statement_node = &function_node.children[0];

		assert_eq!(&statement_node.class, "statement");
		assert_eq!(&statement_node.name, "");
		assert_eq!(statement_node.children.len(), 12);
	}

	#[test]
	fn test_parse_struct_and_function() {
		let source = "
Light: struct {
	position: vec3,
	color: vec3
}

#[vertex]
main: fn () -> void {
	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
	gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
}";

		let tokens = tokenize(source);
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = nodes.unwrap();

		assert_eq!(&root_node.class, "root");
		assert_eq!(&root_node.name, "root");
		assert_eq!(root_node.children.len(), 2);

		let struct_node = &root_node.children[0];

		assert_eq!(&struct_node.class, "struct");
		assert_eq!(&struct_node.name, "Light");
		assert_eq!(struct_node.children.len(), 2);

		let position_node = &struct_node.children[0];

		assert_eq!(&position_node.class, "field");
		assert_eq!(&position_node.name, "position");

		let color_node = &struct_node.children[1];

		assert_eq!(&color_node.class, "field");
		assert_eq!(&color_node.name, "color");

		let function_node = &root_node.children[1];

		assert_eq!(&function_node.class, "function");
		assert_eq!(&function_node.name, "main");
		assert_eq!(function_node.children.len(), 2);
	}
}