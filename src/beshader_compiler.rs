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

pub fn tokenize(source: &str) -> Vec<String> {
	let interrupt = |c: char| -> bool {
		return c.is_whitespace();
	};

	let can_sequence_continue = |sequence: &str, c: char| -> bool {
		if sequence.len() == 0 { return true; }

		let last = sequence.chars().last().unwrap();

		if last.is_alphabetic() {
			return c.is_alphanumeric() || c == '_';
		} else if last.is_numeric() {
			return c.is_numeric() || c == '.' || c.is_alphabetic();
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

enum ParsingFailReasons {
	/// The parser does not handle this type of syntax.
	NotMine,
	/// The parser started handling a sequence of tokens, but it encountered a syntax error.
	BadSyntax,
}

type ParsingResult<'a> = Result<(json::JsonValue, std::slice::Iter<'a, String>), ParsingFailReasons>;

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

	return Ok((json::object! {}, iter));
}

fn parse_struct(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut iter = iterator;

	let name = iter.next().unwrap();

	if !name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::NotMine); }

	let colon = iter.next().unwrap();

	if colon != ":" { return Err(ParsingFailReasons::NotMine); }

	let struct_token = iter.next().unwrap();

	if struct_token != "struct" { return Err(ParsingFailReasons::NotMine); }

	let open_brace = iter.next().unwrap();

	if open_brace != "{" { return Err(ParsingFailReasons::BadSyntax); }

	let mut members = json::JsonValue::new_object();

	while let Some(v) = iter.next() {
		if v == "}" {
			break;
		} else if v == "," {
			continue;
		}

		let colon = iter.next().unwrap();

		if colon != ":" { return Err(ParsingFailReasons::BadSyntax); }

		let type_name = iter.next().unwrap();

		if !type_name.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

		members[v] = json::object! {
			type: "member",
			data_type: type_name.as_str()
		};
	}

	let mut root_node = json::JsonValue::new_object();

	let mut struct_node = json::object! {
		type: "struct",
	};

	for entry in members.entries() {
		struct_node.insert(entry.0, entry.1.clone());
	}

	root_node[name] = struct_node;

	return Ok((root_node, iter));
}

fn parse_statement(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut iter = iterator;

	let mut statement = json::JsonValue::new_array();

	while let Some(v) = iter.next() {
		if v == ";" {
			break;
		}
		
		statement.push(v.as_str());
	}

	return Ok((statement, iter));
}

fn parse_function(iterator: std::slice::Iter<'_, String>) -> ParsingResult {
	let mut iter = iterator;

	let name = iter.next().unwrap();

	if !name.chars().next().unwrap().is_alphabetic() {
		return Err(ParsingFailReasons::NotMine);
	}

	let colon = iter.next().unwrap();

	if colon != ":" { return Err(ParsingFailReasons::NotMine); }

	let fn_token = iter.next().unwrap();

	if fn_token != "fn" { return Err(ParsingFailReasons::NotMine); }

	let open_brace = iter.next().unwrap();

	if open_brace != "(" { return Err(ParsingFailReasons::BadSyntax); }

	let close_brace = iter.next().unwrap();

	if close_brace != ")" { return Err(ParsingFailReasons::BadSyntax); }

	let arrow = iter.next().unwrap();

	if arrow != "->" { return Err(ParsingFailReasons::BadSyntax); }

	let return_type = iter.next().unwrap();

	if !return_type.chars().next().unwrap().is_alphabetic() { return Err(ParsingFailReasons::BadSyntax); }

	let open_brace = iter.next().unwrap();

	if open_brace != "{" { return Err(ParsingFailReasons::BadSyntax); }

	let mut root_node = json::JsonValue::new_object();

	root_node[name] = json::object! {
		type: "function",
		data_type: return_type.as_str(),
		statements: [],
	};

	loop  {
		let res = if let Ok(r) = parse_statement(iter.clone()) { r } else { return Err(ParsingFailReasons::BadSyntax); };

		root_node[name]["statements"].push(res.0);

		iter = res.1;

		// check if iter is close brace
		if iter.clone().peekable().peek().unwrap().as_str() == "}" {
			iter.next();
			break;
		}
	}

	return Ok((root_node, iter));
}

/// Parse consumes an stream of tokens and return a JSON describing the shader.
pub fn parse(tokens: Vec<String>) -> Result<json::JsonValue, ()> {
	let mut root_node = json::JsonValue::new_object();

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

		for entry in result.0.entries() {
			root_node.insert(entry.0, entry.1.clone());
		}

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
		let source = "struct Light { position: vec3f, color: vec3f, data: Data<int>, array: [u8; 4] };";
		let tokens = tokenize(source);
		assert_eq!(tokens, vec!["struct", "Light", "{", "position", ":", "vec3f", ",", "color", ":", "vec3f", ",", "data", ":", "Data", "<", "int", ">", ",", "array", ":", "[", "u8", ";", "4", "]", "}", ";"]);
	}

	#[test]
	fn test_parse_struct() {
		let source = "
Light: struct {
	position: vec3f,
	color: vec3f
}";

		let tokens = tokenize(source);
		let nodes = parse(tokens);

		assert_eq!(nodes.is_ok(), true);

		let root_node = &nodes.unwrap();

		let struct_node = &root_node["Light"];

		assert_eq!(struct_node["type"], "struct");

		let position_node = &struct_node["position"];

		assert_eq!(position_node["type"], "member");
		assert_eq!(position_node["data_type"], "vec3f");

		let color_node = &struct_node["color"];

		assert_eq!(color_node["type"], "member");
		assert_eq!(color_node["data_type"], "vec3f");
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

		let function_node = &root_node["main"];

		assert_eq!(function_node["type"], "function");
		assert_eq!(function_node["data_type"], "void");

		let statements_node = &function_node["statements"];
		let statement_node = &statements_node[0];

		assert_eq!(statement_node.len(), 12);
	}

	#[test]
	fn test_parse_struct_and_function() {
		let source = "
Light: struct {
	position: vec3f,
	color: vec3f
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

		let struct_node = &root_node["Light"];

		assert_eq!(struct_node["type"], "struct");

		let position_node = &struct_node["position"];

		assert_eq!(&position_node["type"], "member");
		assert_eq!(&position_node["data_type"], "vec3f");

		let color_node = &struct_node["color"];

		assert_eq!(&color_node["type"], "member");
		assert_eq!(&color_node["data_type"], "vec3f");

		let function_node = &root_node["main"];

		assert_eq!(&function_node["type"], "function");
		assert_eq!(function_node["data_type"], "void");
	}
}