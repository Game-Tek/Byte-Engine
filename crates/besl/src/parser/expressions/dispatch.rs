use super::*;
use crate::parser::declarations::FeatureParser;

/// Runs `attempt` with each parser in order and returns the first success.
///
/// A parser that declines with [`ParsingFailReasons::NotMine`] lets the next one try. When none succeeds, the first
/// real error wins, so the report comes from the parser that recognized the syntax.
fn first_match<P, T>(
	parsers: &[P],
	iterator: &std::slice::Iter<'_, &str>,
	mut attempt: impl FnMut(&P) -> Result<T, ParsingFailReasons>,
) -> Result<T, ParsingFailReasons> {
	let mut error = None;
	for parser in parsers {
		match attempt(parser) {
			Ok(result) => return Ok(result),
			Err(ParsingFailReasons::NotMine) => {}
			Err(other) => {
				error.get_or_insert(other);
			}
		}
	}

	Err(error.unwrap_or_else(|| ParsingFailReasons::BadSyntax {
		message: format!(
			"Tried several parsers none could handle the syntax for statement: {}",
			iterator.clone().next().unwrap()
		),
	}))
}

/// Runs declaration parsers in order until one accepts the token stream. See [`first_match`] for error selection.
pub(crate) fn execute_parsers<'i, 'a: 'i>(
	parsers: &[FeatureParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
) -> FeatureParserResult<'i, 'a> {
	first_match(parsers, &iterator, |parser| parser(iterator.clone()))
}

/// Runs expression parsers in order until one appends its atoms to `atoms`. A parser that fails leaves `atoms` as it
/// found it. See [`first_match`] for error selection.
pub(crate) fn execute_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
	atoms: &mut Vec<Atoms<'a>>,
) -> ExpressionParserResult<'i, 'a> {
	first_match(parsers, &iterator, |parser| {
		let length = atoms.len();
		parser(iterator.clone(), atoms).inspect_err(|_| atoms.truncate(length))
	})
}

/// Runs expression parsers in order for optional syntax, returning where the first successful one stopped. When
/// every parser fails, whatever its reason, `atoms` is left unchanged and the result is `None`.
pub(crate) fn try_expression_parsers<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	iterator: &std::slice::Iter<'i, &'a str>,
	atoms: &mut Vec<Atoms<'a>>,
) -> Option<std::slice::Iter<'i, &'a str>> {
	parsers.iter().find_map(|parser| {
		let length = atoms.len();
		let result = parser(iterator.clone(), atoms).ok();
		if result.is_none() {
			atoms.truncate(length);
		}
		result
	})
}

/// Parses one complete expression with the first accepting parser and builds its syntax node.
pub(crate) fn parse_expression_node<'i, 'a: 'i>(
	parsers: &[ExpressionParser<'i, 'a>],
	iterator: std::slice::Iter<'i, &'a str>,
) -> FeatureParserResult<'i, 'a> {
	let mut atoms = Vec::new();
	let iterator = execute_expression_parsers(parsers, iterator, &mut atoms)?;
	Ok((expression_atoms_to_node(&atoms), iterator))
}

pub(crate) fn is_identifier_char(character: char) -> bool {
	// TODO: validate number at end of identifier
	character.is_alphanumeric() || character == '_'
}

pub(crate) fn is_identifier(value: &str) -> bool {
	if value == "struct" || value == "fn" || value == "let" || value == "return" || value == "const" || value == "match" {
		return false;
	}
	value.chars().all(is_identifier_char)
}
