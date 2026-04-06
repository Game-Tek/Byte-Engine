//! The `tokenizer` module splits BESL source text into the token stream that the parser consumes.

pub struct Tokens<'a> {
	/// The tokens in the stream.
	pub(crate) tokens: Vec<&'a str>,
}

/// Tokenize consumes a string and returns a stream of tokens.
pub fn tokenize<'a>(source: &'a str) -> Result<Tokens<'a>, ()> {
	let interrupt = |c: char| -> bool { c.is_whitespace() };

	let can_sequence_continue = |last: Option<char>, c: char| -> bool {
		let Some(last) = last else {
			return true;
		};

		if last.is_alphabetic() {
			c.is_alphanumeric() || c == '_'
		} else if last.is_numeric() {
			c.is_alphanumeric() || c == '.' || c == '_'
		} else if last == '.' {
			c.is_numeric()
		} else if last == '_' {
			c.is_alphanumeric() || c == '_'
		} else if last == '-' && c == '>' {
			true
		} else if last == '=' && c == '=' {
			true
		} else if last == '<' && c == '<' {
			true
		} else if last == '>' && c == '>' {
			true
		} else {
			false
		}
	};

	let mut tokens = Vec::new();
	let mut chars = source.char_indices().peekable();
	let mut token_start: Option<usize> = None;
	let mut token_last: Option<char> = None;

	while let Some((idx, c)) = chars.peek().copied() {
		if interrupt(c) {
			if let Some(start) = token_start {
				tokens.push(&source[start..idx]);
				token_start = None;
				token_last = None;
			}
			chars.next();
			continue;
		}

		match token_start {
			None => {
				token_start = Some(idx);
				token_last = Some(c);
				chars.next();
			}
			Some(start) => {
				if can_sequence_continue(token_last, c) {
					token_last = Some(c);
					chars.next();
				} else {
					tokens.push(&source[start..idx]);
					token_start = None;
					token_last = None;
				}
			}
		}
	}

	if let Some(start) = token_start {
		tokens.push(&source[start..]);
	}

	Ok(Tokens { tokens })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_function() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"fn",
				"main",
				"(",
				")",
				"->",
				"void",
				"{",
				"gl_Position",
				"=",
				"vec4",
				"(",
				"0.0",
				",",
				"0.0",
				",",
				"0.0",
				",",
				"1.0",
				")",
				";",
				"}"
			]
		);
	}

	#[test]
	fn test_operators() {
		let source = "fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0) * 2.0; }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"fn",
				"main",
				"(",
				")",
				"->",
				"void",
				"{",
				"gl_Position",
				"=",
				"vec4",
				"(",
				"0.0",
				",",
				"0.0",
				",",
				"0.0",
				",",
				"1.0",
				")",
				"*",
				"2.0",
				";",
				"}"
			]
		);
	}

	#[test]
	fn test_bitwise_operators() {
		let source = "fn main() -> void { value = 1 << 8 | 2 & 255; }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec!["fn", "main", "(", ")", "->", "void", "{", "value", "=", "1", "<<", "8", "|", "2", "&", "255", ";", "}"]
		);
	}

	#[test]
	fn test_struct() {
		let source = "struct Light { position: vec3f, color: vec3f, data: Data<int>, array: [u8; 4] };";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"struct", "Light", "{", "position", ":", "vec3f", ",", "color", ":", "vec3f", ",", "data", ":", "Data", "<",
				"int", ">", ",", "array", ":", "[", "u8", ";", "4", "]", "}", ";"
			]
		);
	}

	#[test]
	fn test_member() {
		let source = "color: In<vec4f>;";
		let tokens = tokenize(source).unwrap();
		assert_eq!(tokens.tokens, vec!["color", ":", "In", "<", "vec4f", ">", ";"]);
	}
}
