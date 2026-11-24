//! The entity search every inspector front end filters entities with, so the console and in-process tools such as an
//! editor find the same entities for the same words.
//!
//! The search's words and forms are specified under [Entity search](/docs/reference/inspector#entity-search).

#![cfg_attr(test, expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"))]

use neo_frizbee::{Config, Matcher, Pattern};

use super::InspectedEntity;

/// Returns the entities of `entities` that pass every word of `search`, an
/// [entity search](/docs/reference/inspector#entity-search): in handle order without a name word, and best match first
/// with one.
///
/// A `type:` word keeps the entities with a type whose name contains its text; any other word fuzzy-matches names.
/// Read the entities with [`DefaultInspector::entities`](super::DefaultInspector::entities) first, then show what this
/// returns.
pub fn find_entities<'a>(entities: &'a [InspectedEntity], search: &str) -> Vec<&'a InspectedEntity> {
	let (types, names): (Vec<&str>, Vec<&str>) = search.split_whitespace().partition(|word| word.starts_with("type:"));
	let types: Vec<&str> = types.iter().map(|word| &word["type:".len()..]).collect();
	let mut kept: Vec<&InspectedEntity> = entities
		.iter()
		.filter(|entity| {
			types
				.iter()
				.all(|text| entity.types().iter().any(|name| contains_ignore_case(name, text)))
		})
		.collect();
	kept.sort_unstable_by_key(|entity| entity.handle().id());
	if names.is_empty() {
		return kept;
	}
	let entity_names: Vec<&str> = kept.iter().map(|entity| entity.name()).collect();
	fuzzy_find(&entity_names, &names.join(" "))
		.into_iter()
		.map(|index| kept[index])
		.collect()
}

/// Returns the indices of the `candidates` that match every word of `search`, best match first, as the name words of
/// an [entity search](/docs/reference/inspector#entity-search) match names.
///
/// Use it to fuzzy-find among names that aren't entities, such as resource IDs, the same way the console finds
/// entities. Equal matches keep their order in `candidates`, and an empty candidate never matches. An empty search
/// keeps every non-empty candidate, in order.
pub fn fuzzy_find(candidates: &[&str], search: &str) -> Vec<usize> {
	let patterns: Vec<Pattern> = search.split_whitespace().map(Pattern::parse).collect();
	if patterns.is_empty() {
		return (0..candidates.len()).filter(|&index| !candidates[index].is_empty()).collect();
	}
	// The matcher sorts best match first, then by position in `candidates`.
	Matcher::from_patterns(&patterns, &Config::default())
		.match_list(candidates)
		.into_iter()
		// An empty candidate has nothing to match, so only the negated forms could keep it; none does.
		.filter(|found| !candidates[found.index as usize].is_empty())
		.map(|found| found.index as usize)
		.collect()
}

impl InspectedEntity {
	/// Returns the entity's name, the `name` property, or an empty string when it has none.
	pub fn name(&self) -> &str {
		self.property("name").and_then(|name| name.as_str()).unwrap_or_default()
	}
}

/// Returns whether `text` contains `part`, comparing ASCII letters in any case.
fn contains_ignore_case(text: &str, part: &str) -> bool {
	part.is_empty()
		|| text
			.as_bytes()
			.windows(part.len())
			.any(|window| window.eq_ignore_ascii_case(part.as_bytes()))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::*;
	use crate::configuration::Configuration;
	use crate::core::Creator as _;
	use crate::core::factory::Handle;
	use crate::core::message_bus::{MessageBus, MessageScope};
	use crate::inspector::DefaultInspector;
	use crate::metrics::Metrics;

	/// An inspector over a scope of named entities, and the scope to spawn them in.
	fn world() -> (DefaultInspector, MessageScope) {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let scope = message_bus.new_scope("entity-search-test-world");
		let inspector = DefaultInspector::new(
			crate::core::channel::DefaultChannel::new(),
			Configuration::new(),
			scope.clone(),
			Arc::new(Metrics::new()),
		);
		(inspector, scope)
	}

	fn spawn(scope: &MessageScope, name: &str) -> Handle {
		scope
			.create(String::from("model"))
			.with(crate::gameplay::Name::new(name.to_string()))
			.with(crate::gameplay::Transform::identity())
			.into()
	}

	fn names<'a>(found: &[&'a InspectedEntity]) -> Vec<&'a str> {
		found.iter().map(|entity| entity.name()).collect()
	}

	#[test]
	fn a_name_word_keeps_fuzzy_matches_best_first() {
		let (inspector, scope) = world();
		spawn(&scope, "barrel");
		spawn(&scope, "shipping crate");
		spawn(&scope, "crate");
		let entities = inspector.entities(None, None);

		assert_eq!(names(&find_entities(&entities, "crate")), ["crate", "shipping crate"]);
		assert_eq!(names(&find_entities(&entities, "shpcrt")), ["shipping crate"]);
		assert_eq!(names(&find_entities(&entities, "!crate")), ["barrel"]);
	}

	#[test]
	fn fuzzy_find_ranks_any_names_best_first() {
		let ids = ["meshes/barrel.glb", "props/crate.glb", "crate.glb", ""];
		assert_eq!(fuzzy_find(&ids, "crate"), [2, 1]);
		assert_eq!(fuzzy_find(&ids, "brl"), [0]);
		assert_eq!(fuzzy_find(&ids, ""), [0, 1, 2]);
	}

	#[test]
	fn without_a_name_word_entities_keep_handle_order_and_type_words_filter() {
		let (inspector, scope) = world();
		let first = spawn(&scope, "b");
		let second = spawn(&scope, "a");
		let entities = inspector.entities(None, None);

		let handles = |found: Vec<&InspectedEntity>| found.iter().map(|entity| entity.handle()).collect::<Vec<_>>();
		assert_eq!(handles(find_entities(&entities, "")), [first, second]);
		assert_eq!(handles(find_entities(&entities, "type:STRING")), [first, second]);
		assert!(find_entities(&entities, "type:camera").is_empty());
	}
}
