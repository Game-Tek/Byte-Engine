//! The entity search every inspector front end filters entities with, so the console and in-process tools such as an
//! editor find the same entities for the same words.
//!
//! The search's words and forms are specified under [Entity search](/docs/reference/inspector#entity-search).

use neo_frizbee::{Config, Matcher, Pattern};

use super::{InspectedEntity, InspectedValue};

/// Returns the entities of `entities` that pass every word of `search`, an
/// [entity search](/docs/reference/inspector#entity-search): in handle order without a name word, and best match first
/// with one.
///
/// A `type:` word keeps the entities with a type whose name contains its text; any other word fuzzy-matches names.
/// Read the entities with [`DefaultInspector::entities`](super::DefaultInspector::entities) first, then show what this
/// returns.
pub fn find_entities<'a>(entities: &'a [InspectedEntity], search: &str) -> Vec<&'a InspectedEntity> {
	let mut patterns = Vec::new();
	let mut types = Vec::new();
	for word in search.split_whitespace() {
		match word.strip_prefix("type:") {
			Some(text) => types.push(text),
			None => patterns.push(Pattern::parse(word)),
		}
	}
	let mut kept: Vec<&InspectedEntity> = entities
		.iter()
		.filter(|entity| {
			types
				.iter()
				.all(|text| entity.types().iter().any(|name| contains_ignore_case(name, text)))
		})
		.collect();
	kept.sort_unstable_by_key(|entity| entity.handle().id());
	if patterns.is_empty() {
		return kept;
	}
	let names: Vec<&str> = kept.iter().map(|entity| entity.name()).collect();
	// The matcher sorts best match first, then by position in `names`, so equal scores stay in handle order.
	Matcher::from_patterns(&patterns, &Config::default())
		.match_list(&names)
		.into_iter()
		// An entity without a name has nothing to match, so only the negated forms could keep it; none does.
		.filter(|found| !names[found.index as usize].is_empty())
		.map(|found| kept[found.index as usize])
		.collect()
}

impl InspectedEntity {
	/// Returns the entity's name, the `name` property, or an empty string when it has none.
	pub fn name(&self) -> &str {
		match self.property("name") {
			Some(InspectedValue::Text(name)) => name,
			_ => "",
		}
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
