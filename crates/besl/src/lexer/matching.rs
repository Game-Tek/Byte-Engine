//! Checks `match` statements against Rust's rules and normalizes them into switch-ready arms.
//!
//! These functions only see pattern values and statement lists. [`super::lowering`] lexes the scrutinee and the
//! arm bodies, then calls [`MatchDomain::of`], [`MatchDomain::pattern_value`], and [`normalize_arms`] to build a
//! [`Nodes::Match`](super::Nodes::Match).

use std::collections::HashSet;

use super::{LexError, MatchArm, NodeReference};
use crate::parser;

const MATCH_DOCUMENTATION: &str = "https://byte-engine.0x44491229.dev/docs/reference/besl/language#match";

/// The `MatchDomain` struct describes the values a match scrutinee can hold,
/// so the lexer can type-check patterns and prove a match exhaustive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MatchDomain {
	/// The scrutinee's type name, used in error messages.
	type_name: &'static str,
	minimum: i64,
	maximum: i64,
}

impl MatchDomain {
	/// Returns the domain of a scrutinee type, or an error when BESL can't match that type yet.
	pub(super) fn of(r#type: Option<&NodeReference>) -> Result<Self, LexError> {
		let Some(r#type) = r#type else {
			return Err(LexError::invalid(format!(
				"Can't infer the type of the match scrutinee. The most likely cause is a scrutinee expression without a known scalar type. Store it in a typed `let` first. See {MATCH_DOCUMENTATION}."
			)));
		};

		let (type_name, minimum, maximum) = match r#type.borrow().get_name() {
			Some("bool") => ("bool", 0, 1),
			Some("u8") => ("u8", 0, u8::MAX.into()),
			Some("u16") => ("u16", 0, u16::MAX.into()),
			Some("u32") => ("u32", 0, u32::MAX.into()),
			Some("i32") => ("i32", i32::MIN.into(), i32::MAX.into()),
			other => {
				return Err(LexError::invalid(format!(
					"Can't match on a value of type `{}`. The most likely cause is a scrutinee that isn't a `bool`, `u8`, `u16`, `u32`, or `i32` value. See {MATCH_DOCUMENTATION}.",
					other.unwrap_or("unnamed")
				)));
			}
		};

		Ok(Self {
			type_name,
			minimum,
			maximum,
		})
	}

	/// Returns the value a literal pattern matches, or `None` for `_`.
	/// Rejects literals of another type and literals out of the scrutinee type's range, as Rust does.
	pub(super) fn pattern_value(&self, pattern: &parser::MatchPattern) -> Result<Option<i64>, LexError> {
		let parser::MatchPattern::Literal { value, negative } = *pattern else {
			return Ok(None);
		};

		let magnitude = match value {
			// Only signed types accept a leading `-`.
			_ if negative && self.minimum == 0 => None,
			"false" | "true" if self.type_name == "bool" => Some(i64::from(value == "true")),
			_ if self.type_name == "bool" => None,
			// Integer literals are plain decimal digits. A value too large for `i64` is out of every range.
			_ if value.bytes().all(|byte| byte.is_ascii_digit()) => Some(value.parse().unwrap_or(i64::MAX)),
			_ => None,
		};
		let Some(magnitude) = magnitude else {
			return Err(LexError::invalid(format!(
				"Match pattern `{}{value}` doesn't have type `{}`. The most likely cause is a pattern of a different type than the value being matched. See {MATCH_DOCUMENTATION}.",
				if negative { "-" } else { "" },
				self.type_name
			)));
		};

		let value = if negative { -magnitude } else { magnitude };
		if !(self.minimum..=self.maximum).contains(&value) {
			return Err(LexError::invalid(format!(
				"Match pattern `{value}` is out of range for `{}`. The most likely cause is a literal larger than the scrutinee type can hold. See {MATCH_DOCUMENTATION}.",
				self.type_name
			)));
		}

		Ok(Some(value))
	}
}

/// Applies Rust's first-match-wins rule to lexed arms and checks that they cover `domain`.
///
/// Each input arm pairs its pattern values, with `None` for `_`, with its statements. The result has distinct
/// labels, drops values and arms no input could reach, and returns the statements for every other value as the
/// default. A match that covers every value without `_` turns its last arm into the default, so every target sees
/// a `switch` that always takes a branch.
pub(super) fn normalize_arms(
	domain: MatchDomain,
	arms: Vec<(Vec<Option<i64>>, Vec<NodeReference>)>,
) -> Result<(Vec<MatchArm>, Vec<NodeReference>), LexError> {
	let mut seen = HashSet::new();
	let mut normalized = Vec::with_capacity(arms.len());

	for (values, statements) in arms {
		// A `_` alternative catches every value earlier arms left, so later arms can never run.
		if values.contains(&None) {
			return Ok((normalized, statements));
		}

		let values: Vec<i64> = values.into_iter().flatten().filter(|value| seen.insert(*value)).collect();
		if !values.is_empty() {
			normalized.push(MatchArm { values, statements });
		}
	}

	if seen.len() as u64 != domain.maximum.abs_diff(domain.minimum) + 1 {
		return Err(LexError::invalid(format!(
			"Non-exhaustive match on `{}`. The most likely cause is a missing `_` arm for the values no other arm lists. See {MATCH_DOCUMENTATION}.",
			domain.type_name
		)));
	}

	let last = normalized
		.pop()
		.expect("A type with at least two values is only covered by at least one arm");
	Ok((normalized, last.statements))
}
