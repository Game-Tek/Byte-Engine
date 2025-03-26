use std::marker::PhantomData;

pub struct Tokens<'a> {
	/// The tokens in the stream.
	pub(crate) tokens: Vec<String>,
	_lifetime: PhantomData<&'a str>,
}

/// Tokenize consumes a string and returns a stream of tokens.
pub fn tokenize<'a>(source: &'a str) -> Result<Tokens<'a>, ()> {
	let interrupt = |c: char| -> bool {
		c.is_whitespace()
	};

	let can_sequence_continue = |sequence: &str, c: char| -> bool {
		if sequence.is_empty() { return true; }

		let last = sequence.chars().last().unwrap();

		if last.is_alphabetic() {
			c.is_alphanumeric() || c == '_'
		} else if last.is_numeric() {
			c.is_numeric() || c == '.' || c.is_alphabetic()
		} else if last == '.' {
			c.is_numeric()
		} else if last == '_' {
			c.is_alphanumeric() || c == '_'
		} else if last == '-' && c == '>' {
			true
		} else {
			false
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
					if !token.is_empty() {
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

	Ok(Tokens { tokens, _lifetime: PhantomData })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_function() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens.tokens, vec!["fn", "main", "(", ")", "->", "void", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", ";", "}"]);
	}

	#[test]
	fn test_operators() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0) * 2.0; }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens.tokens, vec!["fn", "main", "(", ")", "->", "void", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", "*", "2.0", ";", "}"]);
	}

	#[test]
	fn test_struct() {
		let source = "struct Light { position: vec3f, color: vec3f, data: Data<int>, array: [u8; 4] };";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens.tokens, vec!["struct", "Light", "{", "position", ":", "vec3f", ",", "color", ":", "vec3f", ",", "data", ":", "Data", "<", "int", ">", ",", "array", ":", "[", "u8", ";", "4", "]", "}", ";"]);
	}

	#[test]
	fn test_member() {
		let source = "color: In<vec4f>;";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens.tokens, vec!["color", ":", "In", "<", "vec4f", ">", ";"]);
	}
}