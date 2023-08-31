/// Tokenize consumes a string and returns a stream of tokens.
pub fn tokenize(source: &str) -> Result<Vec<String>, ()> {
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

	return Ok(tokens);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_tokenization() {
		let source = "void main() { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens, vec!["void", "main", "(", ")", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", ";", "}"]);
	}

	#[test]
	fn test_tokenization2() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens, vec!["fn", "main", "(", ")", "->", "void", "{", "gl_Position", "=", "vec4", "(", "0.0", ",", "0.0", ",", "0.0", ",", "1.0", ")", ";", "}"]);
	}

	#[test]
	fn test_tokenization3() {
		let source = "struct Light { position: vec3f, color: vec3f, data: Data<int>, array: [u8; 4] };";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens, vec!["struct", "Light", "{", "position", ":", "vec3f", ",", "color", ":", "vec3f", ",", "data", ":", "Data", "<", "int", ">", ",", "array", ":", "[", "u8", ";", "4", "]", "}", ";"]);
	}
}