//! The `tokenizer` module splits BESL source text into the token stream that the parser consumes.

pub struct Tokens<'a> {
	/// The tokens in the stream.
	pub(crate) tokens: Vec<&'a str>,
}

/// Tokenize consumes a string and returns a stream of tokens.
pub fn tokenize<'a>(source: &'a str) -> Result<Tokens<'a>, ()> {
	let interrupt = |c: char| -> bool { c.is_whitespace() };

	let can_sequence_continue = |token: &str, last: Option<char>, c: char| -> bool {
		let Some(last) = last else {
			return true;
		};

		if last.is_alphabetic() {
			c.is_alphanumeric() || c == '_'
		} else if last.is_numeric() {
			c.is_alphanumeric() || c == '_' || c == '.' && token.chars().all(|character| character.is_ascii_digit())
		} else if last == '.' {
			c.is_numeric()
		} else if last == '_' {
			c.is_alphanumeric() || c == '_'
		} else {
			matches!(
				(last, c),
				('-', '>')
					| ('=', '=') | ('!', '=')
					| ('<', '=') | ('>', '=')
					| ('<', '<') | ('>', '>')
					| ('&', '&') | ('|', '|')
			)
		}
	};

	let mut tokens = Vec::new();
	let mut chars = source.char_indices().peekable();
	let mut token_start: Option<usize> = None;
	let mut token_last: Option<char> = None;

	while let Some((idx, c)) = chars.peek().copied() {
		if c == '/' && chars.clone().nth(1).is_some_and(|(_, next)| next == '/') {
			if let Some(start) = token_start {
				tokens.push(&source[start..idx]);
				token_start = None;
				token_last = None;
			}
			// Line comments are discarded before punctuation tokenization so their contents remain entirely opaque.
			for (_, comment_character) in chars.by_ref() {
				if comment_character == '\n' {
					break;
				}
			}
			continue;
		}

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
				if can_sequence_continue(&source[start..idx], token_last, c) {
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

	#[test]
	fn test_for_loop() {
		let source = "main: fn () -> void { for (let i: u32 = 0; i < 4; i = i + 1) { value = value + i; } }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"main", ":", "fn", "(", ")", "->", "void", "{", "for", "(", "let", "i", ":", "u32", "=", "0", ";", "i", "<",
				"4", ";", "i", "=", "i", "+", "1", ")", "{", "value", "=", "value", "+", "i", ";", "}", "}"
			]
		);
	}

	#[test]
	fn test_comparison_and_logical_operators() {
		let source = "main: fn () -> void { if (a >= b || c != d && e <= f && g > h) { continue; } }";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"main", ":", "fn", "(", ")", "->", "void", "{", "if", "(", "a", ">=", "b", "||", "c", "!=", "d", "&&", "e",
				"<=", "f", "&&", "g", ">", "h", ")", "{", "continue", ";", "}", "}"
			]
		);
	}

	#[test]
	fn line_comments_are_ignored_without_consuming_adjacent_tokens() {
		let source = "main: fn () -> void { value = 1;// punctuation: } / *\nvalue = value + 2; } // eof comment";
		let tokens = tokenize(source).unwrap();
		assert_eq!(
			tokens.tokens,
			vec![
				"main", ":", "fn", "(", ")", "->", "void", "{", "value", "=", "1", ";", "value", "=", "value", "+", "2", ";",
				"}"
			]
		);
	}

	#[test]
	fn numeric_identifier_suffix_does_not_consume_member_accessor() {
		let tokens = tokenize("matrix0.column0 + 1.25").unwrap();
		assert_eq!(tokens.tokens, vec!["matrix0", ".", "column0", "+", "1.25"]);
	}
}
