//! A hash map that allows for tracking the staleness of values.

use gxhash::{HashMap, HashMapExt};

/// Possible states of an entry in a `StaleHashMap`.
pub enum Entry<V> {
	/// A value for a key exists and the value is up-to-date.
	Fresh(V),
	/// A value for a key exists but the value is stale.
	Stale(V),
	/// No value for a key exists.
	Empty,
}

/// A hash map that allows for tracking the staleness of values.
pub struct StaleHashMap<K, H, V>(HashMap<K, (H, V)>);

impl<K: Eq + std::hash::Hash, H: PartialEq, V> Default for StaleHashMap<K, H, V> {
	fn default() -> Self {
		Self::new()
	}
}

impl<K: Eq + std::hash::Hash, H: PartialEq, V> StaleHashMap<K, H, V> {
	/// Creates a new `StaleHashMap`.
	pub fn new() -> Self {
		Self(HashMap::with_capacity(1024))
	}

	/// Creates a new `StaleHashMap` with the specified capacity.
	pub fn with_capacity(capacity: usize) -> Self {
		Self(HashMap::with_capacity(capacity))
	}

	/// Returns a reference to the value only if it is not stale.
	pub fn get(&self, key: &K, hash: H) -> Option<&V> {
		match self.0.get(key) {
			Some((old_hash, value)) => {
				if *old_hash == hash {
					Some(value)
				} else {
					None
				}
			}
			None => None,
		}
	}

	pub fn entry(&self, key: &K, hash: H) -> Entry<&V> {
		match self.0.get(key) {
			Some((old_hash, value)) => {
				if *old_hash == hash {
					Entry::Fresh(value)
				} else {
					Entry::Stale(value)
				}
			}
			None => Entry::Empty,
		}
	}

	pub fn insert(&mut self, key: K, hash: H, value: V) {
		self.0.insert(key, (hash, value));
	}
}

#[cfg(test)]
mod tests {
	use super::{Entry, StaleHashMap};

	#[test]
	fn matching_revision_is_fresh_and_mismatched_revision_retains_stale_value() {
		let mut map = StaleHashMap::new();
		map.insert("mesh", 7, "compiled");

		assert_eq!(map.get(&"mesh", 7), Some(&"compiled"));
		assert_eq!(map.get(&"mesh", 8), None);
		assert!(matches!(map.entry(&"mesh", 7), Entry::Fresh(&"compiled")));
		assert!(matches!(map.entry(&"mesh", 8), Entry::Stale(&"compiled")));
		assert!(matches!(map.entry(&"missing", 7), Entry::Empty));
	}

	#[test]
	fn reinsertion_atomically_replaces_revision_and_value() {
		let mut map = StaleHashMap::with_capacity(1);
		map.insert("shader", 1, String::from("old"));
		map.insert("shader", 2, String::from("new"));

		assert_eq!(map.get(&"shader", 1), None);
		assert_eq!(map.get(&"shader", 2).map(String::as_str), Some("new"));
	}

	#[test]
	fn revisions_are_scoped_to_their_keys() {
		let mut map = StaleHashMap::default();
		map.insert("a", 1, 10);
		map.insert("b", 2, 20);

		assert_eq!(map.get(&"a", 1), Some(&10));
		assert_eq!(map.get(&"b", 2), Some(&20));
		assert_eq!(map.get(&"a", 2), None);
		assert_eq!(map.get(&"b", 1), None);
	}
}
