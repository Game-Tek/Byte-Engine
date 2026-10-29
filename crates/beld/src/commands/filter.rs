//! Parse filter text into terms and test rows against them, for `browse` and `query`.
//!
//! The grammar and every rule are specified in the BELD command-line reference, under Filter, group, and sort.

#[cfg(debug_assertions)]
use resource_management::ResourceTraceLevel;
use resource_management::resource::ResourcePayloadEncoding;
use utils::Glob;

use super::catalog::{Catalog, Field, FieldKind, Status, Value};

/// The `Filter` struct is a parsed filter: a row matches when it matches every term.
#[derive(Debug, Default)]
pub(super) struct Filter {
	terms: Vec<Term>,
}

#[derive(Debug)]
struct Term {
	negated: bool,
	test: Test,
}

#[derive(Debug)]
enum Test {
	/// The ID contains the text, ignoring ASCII case.
	Contains(String),
	/// The whole ID matches a `*` and `?` pattern, ignoring ASCII case.
	Glob(Glob),
	Compare {
		field: Field,
		operator: Operator,
		operand: Operand,
	},
	Flag(Flag),
	#[cfg(debug_assertions)]
	Has(ResourceTraceLevel),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
	Equal,
	NotEqual,
	Contains,
	Greater,
	GreaterOrEqual,
	Less,
	LessOrEqual,
}

/// Operators in the order the parser tries them, so `!=`, `>=`, and `<=` win over `>` and `<`.
const OPERATORS: [(&str, Operator); 7] = [
	("!=", Operator::NotEqual),
	(">=", Operator::GreaterOrEqual),
	("<=", Operator::LessOrEqual),
	("=", Operator::Equal),
	("~", Operator::Contains),
	(">", Operator::Greater),
	("<", Operator::Less),
];

#[derive(Debug)]
struct Operand {
	text: String,
	/// The operand as a number for number, size, and age fields, and for model fields when it parses as one.
	number: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
	Failed,
	Stale,
	Compressed,
	Duplicate,
	Root,
	Unreachable,
	Dangling,
}

impl Filter {
	/// Parses filter text against the catalog's declared properties and configured roots.
	///
	/// Returns an error naming the first term that doesn't parse.
	pub(super) fn parse(text: &str, catalog: &Catalog) -> Result<Filter, String> {
		let terms = text
			.split(' ')
			.filter(|term| !term.is_empty())
			.map(|term| parse_term(term, catalog))
			.collect::<Result<_, _>>()?;
		Ok(Filter { terms })
	}

	/// Returns whether the row matches every term.
	pub(super) fn matches(&self, catalog: &Catalog, row: usize) -> bool {
		self.terms.iter().all(|term| term.test.matches(catalog, row) != term.negated)
	}

	/// Returns the fields the filter compares, so their model values can be prepared first.
	pub(super) fn fields(&self) -> impl Iterator<Item = &Field> {
		self.terms.iter().filter_map(|term| match &term.test {
			Test::Compare { field, .. } => Some(field),
			_ => None,
		})
	}
}

impl Test {
	fn matches(&self, catalog: &Catalog, row: usize) -> bool {
		let entry = &catalog.rows[row];
		match self {
			Test::Contains(text) => contains_ignoring_case(&entry.id, text),
			Test::Glob(pattern) => pattern.matches(&entry.id),
			Test::Compare {
				field,
				operator,
				operand,
			} => compare(catalog.value(row, field), *operator, operand),
			Test::Flag(flag) => match flag {
				Flag::Failed => entry.status == Status::Failed,
				Flag::Stale => entry.status == Status::Stale,
				Flag::Compressed => entry
					.resource
					.as_ref()
					.is_some_and(|resource| resource.encoding() != ResourcePayloadEncoding::Raw),
				Flag::Duplicate => entry.duplicate,
				Flag::Root => entry.root,
				Flag::Unreachable => entry.resource.is_some() && !entry.reachable,
				Flag::Dangling => entry.references.iter().any(|id| catalog.row_of(id).is_none()),
			},
			#[cfg(debug_assertions)]
			Test::Has(level) => entry.trace.iter().any(|item| item.level() == *level),
		}
	}
}

/// Parses one space-free term.
fn parse_term(term: &str, catalog: &Catalog) -> Result<Term, String> {
	let (negated, body) = match term.strip_prefix('-') {
		Some(body) if !body.is_empty() => (true, body),
		_ => (false, term),
	};

	let test = if let Some(flag) = body.strip_prefix("is:") {
		Test::Flag(parse_flag(flag, catalog.has_roots)?)
	} else if let Some(level) = body.strip_prefix("has:") {
		parse_level(level)?
	} else if let Some((position, symbol, operator)) = find_operator(body) {
		let (name, value) = (&body[..position], &body[position + symbol.len()..]);
		if name.is_empty() || value.is_empty() {
			return Err(format!(
				"Invalid filter term '{term}'. The most likely cause is a missing field name or value. Write it as `field{symbol}value`, such as `class=Image`."
			));
		}
		let field = Field::parse(name, &catalog.properties)?;
		let operand = parse_operand(term, &field, operator, value)?;
		Test::Compare {
			field,
			operator,
			operand,
		}
	} else if body.contains(['*', '?']) {
		Test::Glob(Glob::new(body))
	} else {
		Test::Contains(body.to_string())
	};

	Ok(Term { negated, test })
}

/// Finds the first operator in a term, preferring the two-character operators at the same position.
fn find_operator(body: &str) -> Option<(usize, &'static str, Operator)> {
	body.char_indices().find_map(|(position, _)| {
		OPERATORS
			.iter()
			.find(|(symbol, _)| body[position..].starts_with(symbol))
			.map(|&(symbol, operator)| (position, symbol, operator))
	})
}

/// Reads the comparison value in the unit the field's kind takes.
fn parse_operand(term: &str, field: &Field, operator: Operator, value: &str) -> Result<Operand, String> {
	let ordering = !matches!(operator, Operator::Equal | Operator::NotEqual | Operator::Contains);
	let invalid = |expected: &str| {
		format!(
			"Invalid filter term '{term}'. The most likely cause is that '{value}' isn't {expected}, which `{}` takes. Change the value, for example to {}.",
			field.name(),
			match field.kind() {
				FieldKind::Size => "`1MiB`",
				FieldKind::Age => "`2h`",
				_ => "`10`",
			}
		)
	};

	let number = match (field.kind(), operator) {
		(_, Operator::Contains) => None,
		(FieldKind::Text, _) if ordering => {
			return Err(format!(
				"Invalid filter term '{term}'. The most likely cause is that `{}` is text, which only `=`, `!=`, and `~` compare. Use one of those operators.",
				field.name()
			));
		}
		(FieldKind::Text, _) => None,
		(FieldKind::Number | FieldKind::Ratio, _) => Some(value.parse::<f64>().map_err(|_| invalid("a number"))?),
		(FieldKind::Size, _) => Some(parse_size(value).ok_or_else(|| invalid("a size"))?),
		(FieldKind::Age, _) => Some(parse_age(value).ok_or_else(|| invalid("an age"))?),
		(FieldKind::Model, _) if ordering => Some(value.parse::<f64>().map_err(|_| invalid("a number"))?),
		(FieldKind::Model, _) => value.parse::<f64>().ok(),
	};

	Ok(Operand {
		text: value.to_string(),
		number,
	})
}

/// Parses a decimal number followed by `B`, `KiB`, `MiB`, or `GiB` in any case, or by nothing, into bytes.
fn parse_size(value: &str) -> Option<f64> {
	let split = value
		.find(|character: char| character.is_ascii_alphabetic())
		.unwrap_or(value.len());
	let (number, unit) = value.split_at(split);
	let scale = match unit.to_ascii_lowercase().as_str() {
		"" | "b" => 1.0,
		"kib" => 1024.0,
		"mib" => 1024.0 * 1024.0,
		"gib" => 1024.0 * 1024.0 * 1024.0,
		_ => return None,
	};
	Some(number.parse::<f64>().ok()? * scale)
}

/// Parses a decimal number followed by `s`, `m`, `h`, or `d` into seconds.
fn parse_age(value: &str) -> Option<f64> {
	let (number, unit) = value.split_at(value.len().checked_sub(1)?);
	let scale = match unit {
		"s" => 1.0,
		"m" => 60.0,
		"h" => 3600.0,
		"d" => 86_400.0,
		_ => return None,
	};
	Some(number.parse::<f64>().ok()? * scale)
}

/// Parses the flag of an `is:` term; `has_roots` says whether the configuration names any root pattern.
fn parse_flag(flag: &str, has_roots: bool) -> Result<Flag, String> {
	let parsed = match flag {
		"failed" => Flag::Failed,
		"stale" => Flag::Stale,
		"compressed" => Flag::Compressed,
		"duplicate" => Flag::Duplicate,
		"root" => Flag::Root,
		"unreachable" => Flag::Unreachable,
		"dangling" => Flag::Dangling,
		_ => {
			return Err(format!(
				"Unknown flag 'is:{flag}'. The most likely cause is a misspelled flag. Use `is:failed`, `is:stale`, `is:compressed`, `is:duplicate`, `is:root`, `is:unreachable`, or `is:dangling`."
			));
		}
	};
	if matches!(parsed, Flag::Failed | Flag::Unreachable | Flag::Dangling) && !cfg!(debug_assertions) {
		return Err(format!(
			"The flag 'is:{flag}' needs a debug build of BELD. The most likely cause is that this BELD is a release build, which reads no traces or references. Run a debug build of BELD to use it."
		));
	}
	if parsed == Flag::Unreachable && !has_roots {
		return Err(
			"The flag 'is:unreachable' needs roots. The most likely cause is that the configuration has no `resources.roots` patterns, so no resource is reachable. Add the IDs your application loads to `resources.roots` in `config.json`, or pass another file with `--config`."
				.to_string(),
		);
	}
	Ok(parsed)
}

/// Parses the level of a `has:` term into a test of the bake trace.
fn parse_level(level: &str) -> Result<Test, String> {
	if !matches!(level, "warnings" | "errors") {
		return Err(format!(
			"Unknown term 'has:{level}'. The most likely cause is a misspelled trace level. Use `has:warnings` or `has:errors`."
		));
	}
	#[cfg(not(debug_assertions))]
	return Err(format!(
		"The term 'has:{level}' needs a debug build of BELD. The most likely cause is that this BELD is a release build, which reads no traces. Run a debug build of BELD to use it."
	));
	#[cfg(debug_assertions)]
	Ok(Test::Has(if level == "warnings" {
		ResourceTraceLevel::Warn
	} else {
		ResourceTraceLevel::Error
	}))
}

/// Compares a row's value with the operand. A missing value matches only `!=`.
fn compare(value: Option<Value<'_>>, operator: Operator, operand: &Operand) -> bool {
	let Some(value) = value else {
		return operator == Operator::NotEqual;
	};
	let equal = || match &value {
		Value::Text(text) => text.as_ref() == operand.text,
		Value::Number(number) => operand.number == Some(*number),
	};
	let order = || {
		value
			.as_number()
			.zip(operand.number)
			.and_then(|(left, right)| left.partial_cmp(&right))
	};

	match operator {
		Operator::Equal => equal(),
		Operator::NotEqual => !equal(),
		Operator::Contains => contains_ignoring_case(&value.to_text(), &operand.text),
		Operator::Greater => order().is_some_and(std::cmp::Ordering::is_gt),
		Operator::GreaterOrEqual => order().is_some_and(std::cmp::Ordering::is_ge),
		Operator::Less => order().is_some_and(std::cmp::Ordering::is_lt),
		Operator::LessOrEqual => order().is_some_and(std::cmp::Ordering::is_le),
	}
}

/// Returns whether `text` contains `needle`, ignoring ASCII case, without allocating.
fn contains_ignoring_case(text: &str, needle: &str) -> bool {
	needle.is_empty()
		|| text
			.as_bytes()
			.windows(needle.len())
			.any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
