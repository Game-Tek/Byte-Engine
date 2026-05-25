use std::ops::{Index, IndexMut};

/// The `StableVec` struct exists to provide vector-like storage for values whose indices must survive removals.
#[derive(Debug, Clone)]
pub struct StableVec<T> {
	entries: Vec<Option<T>>,
	vacant_indices: Vec<usize>,
	len: usize,
}

impl<T> StableVec<T> {
	pub fn new() -> Self {
		Self {
			entries: Vec::new(),
			vacant_indices: Vec::new(),
			len: 0,
		}
	}

	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			entries: Vec::with_capacity(capacity),
			vacant_indices: Vec::new(),
			len: 0,
		}
	}

	pub fn len(&self) -> usize {
		self.len
	}

	pub fn is_empty(&self) -> bool {
		self.len == 0
	}

	pub fn capacity(&self) -> usize {
		self.entries.capacity()
	}

	pub fn slots_len(&self) -> usize {
		self.entries.len()
	}

	/// Inserts a value into the first available slot and returns its stable index.
	pub fn push(&mut self, value: T) -> usize {
		if let Some(index) = self.vacant_indices.pop() {
			self.entries[index] = Some(value);
			self.len += 1;
			return index;
		}

		let index = self.entries.len();
		self.entries.push(Some(value));
		self.len += 1;
		index
	}

	/// Inserts a value at an exact slot without shifting any existing indices.
	pub fn insert(&mut self, index: usize, value: T) -> Option<T> {
		assert!(
			index <= self.entries.len(),
			"StableVec insert index is out of bounds. The most likely cause is that a stale or foreign index was used."
		);

		if index == self.entries.len() {
			self.entries.push(Some(value));
			self.len += 1;
			return None;
		}

		let previous = self.entries[index].replace(value);
		if previous.is_none() {
			self.remove_vacant_index(index);
			self.len += 1;
		}
		previous
	}

	pub fn get(&self, index: usize) -> Option<&T> {
		self.entries.get(index).and_then(Option::as_ref)
	}

	pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
		self.entries.get_mut(index).and_then(Option::as_mut)
	}

	pub fn contains_index(&self, index: usize) -> bool {
		self.get(index).is_some()
	}

	/// Removes a value from an exact slot without shifting any remaining indices.
	pub fn remove(&mut self, index: usize) -> Option<T> {
		let entry = self.entries.get_mut(index)?;
		let value = entry.take()?;

		self.vacant_indices.push(index);
		self.len -= 1;

		Some(value)
	}

	/// Removes the last occupied value without shifting any remaining indices.
	pub fn pop(&mut self) -> Option<T> {
		let index = self.entries.iter().rposition(Option::is_some)?;
		self.remove(index)
	}

	pub fn clear(&mut self) {
		self.entries.clear();
		self.vacant_indices.clear();
		self.len = 0;
	}

	pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
		self.entries.iter().filter_map(Option::as_ref)
	}

	pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
		self.entries.iter_mut().filter_map(Option::as_mut)
	}

	pub fn indexed_iter(&self) -> impl DoubleEndedIterator<Item = (usize, &T)> {
		self.entries
			.iter()
			.enumerate()
			.filter_map(|(index, entry)| entry.as_ref().map(|value| (index, value)))
	}

	pub fn indexed_iter_mut(&mut self) -> impl DoubleEndedIterator<Item = (usize, &mut T)> {
		self.entries
			.iter_mut()
			.enumerate()
			.filter_map(|(index, entry)| entry.as_mut().map(|value| (index, value)))
	}

	fn remove_vacant_index(&mut self, index: usize) {
		if let Some(position) = self.vacant_indices.iter().position(|&vacant| vacant == index) {
			// The free list is unordered, so swap removal avoids shifting unrelated vacant slots.
			self.vacant_indices.swap_remove(position);
		}
	}
}

impl<T> Default for StableVec<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T> Index<usize> for StableVec<T> {
	type Output = T;

	fn index(&self, index: usize) -> &Self::Output {
		self.get(index).expect(
			"StableVec index does not contain a value. The most likely cause is that the slot was removed or never inserted.",
		)
	}
}

impl<T> IndexMut<usize> for StableVec<T> {
	fn index_mut(&mut self, index: usize) -> &mut Self::Output {
		self.get_mut(index).expect(
			"StableVec index does not contain a value. The most likely cause is that the slot was removed or never inserted.",
		)
	}
}

impl<T> FromIterator<T> for StableVec<T> {
	fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
		let entries = iter.into_iter().map(Some).collect::<Vec<_>>();
		let len = entries.len();

		Self {
			entries,
			vacant_indices: Vec::new(),
			len,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::StableVec;

	#[test]
	fn push_returns_stable_indices() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");

		assert_eq!(first, 0);
		assert_eq!(second, 1);
		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), Some(&"second"));
	}

	#[test]
	fn remove_preserves_other_indices() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		let third = values.push("third");

		assert_eq!(values.remove(second), Some("second"));
		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), None);
		assert_eq!(values.get(third), Some(&"third"));
	}

	#[test]
	fn push_reuses_removed_indices() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		let third = values.push("third");

		assert_eq!(values.remove(second), Some("second"));

		let reused = values.push("replacement");

		assert_eq!(reused, second);
		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(reused), Some(&"replacement"));
		assert_eq!(values.get(third), Some(&"third"));
	}

	#[test]
	fn insert_uses_exact_index_without_shifting() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		let third = values.push("third");

		assert_eq!(values.remove(second), Some("second"));
		assert_eq!(values.insert(second, "replacement"), None);

		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), Some(&"replacement"));
		assert_eq!(values.get(third), Some(&"third"));
	}

	#[test]
	fn insert_replaces_occupied_slot() {
		let mut values = StableVec::new();

		let index = values.push("first");

		assert_eq!(values.insert(index, "replacement"), Some("first"));
		assert_eq!(values.len(), 1);
		assert_eq!(values.get(index), Some(&"replacement"));
	}

	#[test]
	fn pop_removes_last_occupied_value_without_shifting() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		let third = values.push("third");

		assert_eq!(values.remove(second), Some("second"));
		assert_eq!(values.pop(), Some("third"));

		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), None);
		assert_eq!(values.get(third), None);
	}

	#[test]
	fn indexed_iter_skips_vacant_slots() {
		let mut values = StableVec::new();

		values.push("first");
		let second = values.push("second");
		values.push("third");
		values.remove(second);

		let entries = values.indexed_iter().collect::<Vec<_>>();

		assert_eq!(entries, vec![(0, &"first"), (2, &"third")]);
	}
}
