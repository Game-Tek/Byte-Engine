use super::*;
use crate::parser::declarations::FeatureParser;

/// Runs parsers in order until one accepts the token stream.
pub(crate) fn execute_parsers<'i, 'a: 'i>(
	parsers: &[FeatureParser<'i, 'a>],
	mut iterator: std::slice::Iter<'i, &'a str>,
) -> FeatureParserResult<'i, 'a> {
	let mut error = None;

	for parser in parsers {
		match parser(iterator.clone()) {
			Ok(result) => return Ok(result),
			Err(ParsingFailReasons::NotMine) => {}
			Err(other) => {
				if error.is_none() {
					error = Some(other);
				}
			}
		}
	}

	if let Some(error) = error {
		return Err(error);
	}

	Err(ParsingFailReasons::BadSyntax {
		message: format!(
			"Tried several parsers none could handle the syntax for statement: {}",
			iterator.next().unwrap()
		),
	})
}

/// Runs parsers in order and permits every parser to decline the syntax.
pub(crate) fn try_execute_parsers<'i, 'a: 'i>(
	parsers: &[FeatureParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
) -> Option<FeatureParserResult<'i, 'a>> {
	for parser in parsers {
		if let Ok(result) = parser(iterator.clone()) {
			return Some(Ok(result));
		}
	}

	None
}

/// Runs expression parsers in order until one accepts the token stream.
pub(crate) fn execute_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	mut iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	let mut error = None;

	for parser in parsers {
		match parser(iterator.clone(), expressions.clone()) {
			Ok(result) => return Ok(result),
			Err(ParsingFailReasons::NotMine) => {}
			Err(other) => {
				if error.is_none() {
					error = Some(other);
				}
			}
		}
	}

	if let Some(error) = error {
		return Err(error);
	}

	Err(ParsingFailReasons::BadSyntax {
		message: format!(
			"Tried several parsers none could handle the syntax for statement: {}",
			iterator.next().unwrap()
		),
	})
}

/// Runs expression parsers in order and permits every parser to decline the syntax.
pub(crate) fn try_execute_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
	expressions: Vec<Atoms<'a>>,
) -> Option<ExpressionParserResult<'i, 'a>> {
	for parser in parsers {
		if let Ok(result) = parser(iterator.clone(), expressions.clone()) {
			return Some(Ok(result));
		}
	}

	None
}

pub(crate) fn is_identifier_char(character: char) -> bool {
	// TODO: validate number at end of identifier
	character.is_alphanumeric() || character == '_'
}

pub(crate) fn is_identifier(value: &str) -> bool {
	if value == "struct" || value == "fn" || value == "let" || value == "return" || value == "const" {
		return false;
	}
	value.chars().all(is_identifier_char)
}
