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

impl <K: Eq + std::hash::Hash, H: PartialEq, V> StaleHashMap<K, H, V> {
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
			},
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
			},
			None => Entry::Empty,
		}
	}

	pub fn insert(&mut self, key: K, hash: H, value: V) {
		self.0.insert(key, (hash, value));
	}
}