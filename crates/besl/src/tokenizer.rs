//! Splits BESL source into tokens for [`crate::parser`].

/// Splits a source string into a token stream.
pub fn tokenize(source: &str) -> Vec<&str> {
	/// Reports whether `c` extends `token`, the non-empty token read so far.
	fn can_sequence_continue(token: &str, c: char) -> bool {
		let Some(last) = token.chars().next_back() else {
			return true;
		};

		if last.is_alphabetic() || last == '_' {
			c.is_alphanumeric() || c == '_'
		} else if last.is_numeric() {
			c.is_alphanumeric() || c == '_' || c == '.' && token.chars().all(|character| character.is_ascii_digit())
		} else if last == '.' {
			c.is_numeric()
		} else {
			matches!(
				(last, c),
				('-', '>')
					| ('=', '>') | ('=', '=')
					| ('!', '=') | ('<', '=')
					| ('>', '=') | ('<', '<')
					| ('>', '>') | ('&', '&')
					| ('|', '|')
			)
		}
	}

	let mut tokens = Vec::new();
	let mut chars = source.char_indices().peekable();
	let mut token_start: Option<usize> = None;

	while let Some((idx, c)) = chars.peek().copied() {
		let comment = c == '/' && chars.clone().nth(1).is_some_and(|(_, next)| next == '/');
		// Whitespace, a comment, or a character that cannot extend the current token ends that token.
		if let Some(start) = token_start
			&& (comment || c.is_whitespace() || !can_sequence_continue(&source[start..idx], c))
		{
			tokens.push(&source[start..idx]);
			token_start = None;
		}
		if comment {
			// Line comments are discarded before punctuation tokenization so their contents remain entirely opaque.
			chars.by_ref().find(|&(_, character)| character == '\n');
			continue;
		}
		if !c.is_whitespace() {
			token_start.get_or_insert(idx);
		}
		chars.next();
	}

	if let Some(start) = token_start {
		tokens.push(&source[start..]);
	}

	tokens
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Asserts that `source` splits into the space-separated tokens of `expected`.
	fn assert_tokens(source: &str, expected: &str) {
		assert_eq!(tokenize(source), expected.split(' ').collect::<Vec<_>>());
	}

	#[test]
	fn test_operators() {
		assert_tokens(
			"fn main() -> void { gl_Position = vec4(0.0, 0.0, 0.0, 1.0) * 2.0; }",
			"fn main ( ) -> void { gl_Position = vec4 ( 0.0 , 0.0 , 0.0 , 1.0 ) * 2.0 ; }",
		);
	}

	#[test]
	fn test_bitwise_operators() {
		assert_tokens(
			"fn main() -> void { value = 1 << 8 | 2 ^ 3 & 255; }",
			"fn main ( ) -> void { value = 1 << 8 | 2 ^ 3 & 255 ; }",
		);
	}

	#[test]
	fn test_comparison_and_logical_operators() {
		assert_tokens(
			"main: fn () -> void { if (a >= b || c != d && e <= f && g > h) { continue; } }",
			"main : fn ( ) -> void { if ( a >= b || c != d && e <= f && g > h ) { continue ; } }",
		);
	}

	#[test]
	fn prefix_and_ternary_operators_split_from_adjacent_tokens() {
		// `!=` stays one token, while `!` before an operand and a doubled `-` split, so `--a` is two negations.
		assert_tokens("x = a != !b ? ~c : --d;", "x = a != ! b ? ~ c : - - d ;");
	}

	#[test]
	fn line_comments_are_ignored_without_consuming_adjacent_tokens() {
		assert_tokens(
			"main: fn () -> void { value = 1;// punctuation: } / *\nvalue = value + 2; } // eof comment",
			"main : fn ( ) -> void { value = 1 ; value = value + 2 ; }",
		);
	}

	#[test]
	fn numeric_identifier_suffix_does_not_consume_member_accessor() {
		assert_tokens("matrix0.column0 + 1.25", "matrix0 . column0 + 1.25");
	}
}
