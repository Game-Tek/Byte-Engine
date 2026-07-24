/// The `StableVecHandle` struct provides generation-aware access to a live [`StableVec`] slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StableVecHandle {
	index: u32,
	generation: u32,
}

impl StableVecHandle {
	pub fn index(self) -> usize {
		self.index as usize
	}

	pub fn generation(self) -> u32 {
		self.generation
	}

	fn new(index: usize, generation: u32) -> Self {
		let index = u32::try_from(index).expect(
			"StableVec handle index exceeds u32::MAX. The most likely cause is that this StableVec grew beyond handle capacity.",
		);

		Self { index, generation }
	}
}

#[derive(Debug, Clone)]
struct Entry<T> {
	value: Option<T>,
	generation: u32,
	next_free: Option<usize>,
}

/// The `StableVec` struct provides vector-like storage whose handles reject stale slot reuse.
#[derive(Debug, Clone)]
pub struct StableVec<T> {
	entries: Vec<Entry<T>>,
	first_free: Option<usize>,
	len: usize,
}

impl<T> StableVec<T> {
	pub fn new() -> Self {
		Self {
			entries: Vec::new(),
			first_free: None,
			len: 0,
		}
	}

	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			entries: Vec::with_capacity(capacity),
			first_free: None,
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

	/// Stores a value in the first available slot and returns its handle.
	pub fn push(&mut self, value: T) -> StableVecHandle {
		if let Some(index) = self.first_free {
			let entry = &mut self.entries[index];
			self.first_free = entry.next_free;
			entry.next_free = None;
			entry.value = Some(value);
			self.len += 1;

			return StableVecHandle::new(index, entry.generation);
		}

		let index = self.entries.len();
		self.entries.push(Entry {
			value: Some(value),
			generation: 0,
			next_free: None,
		});
		self.len += 1;

		StableVecHandle::new(index, 0)
	}

	/// Replaces the value for a live handle and returns the previous value.
	pub fn insert(&mut self, handle: StableVecHandle, value: T) -> Option<T> {
		let entry = self.valid_entry_mut(handle)?;
		entry.value.replace(value)
	}

	pub fn get(&self, handle: StableVecHandle) -> Option<&T> {
		self.valid_entry(handle).and_then(|entry| entry.value.as_ref())
	}

	pub fn get_mut(&mut self, handle: StableVecHandle) -> Option<&mut T> {
		self.valid_entry_mut(handle).and_then(|entry| entry.value.as_mut())
	}

	pub fn contains_handle(&self, handle: StableVecHandle) -> bool {
		self.get(handle).is_some()
	}

	/// Returns a live value by raw slot for algorithms that use temporary slot IDs.
	pub fn get_slot(&self, index: usize) -> Option<&T> {
		self.entries.get(index).and_then(|entry| entry.value.as_ref())
	}

	/// Returns a mutable live value by raw slot for algorithms that use temporary slot IDs.
	pub fn get_slot_mut(&mut self, index: usize) -> Option<&mut T> {
		self.entries.get_mut(index).and_then(|entry| entry.value.as_mut())
	}

	/// Removes a value addressed by a live handle without shifting any remaining slots.
	pub fn remove(&mut self, handle: StableVecHandle) -> Option<T> {
		let index = handle.index();
		let first_free = self.first_free;
		let entry = self.valid_entry_mut(handle)?;
		let value = entry.value.take()?;

		entry.generation = entry.generation.wrapping_add(1);
		entry.next_free = first_free;
		self.first_free = Some(index);
		self.len -= 1;

		Some(value)
	}

	/// Removes the last occupied value without shifting any remaining slots.
	pub fn pop(&mut self) -> Option<T> {
		let (index, entry) = self.entries.iter().enumerate().rfind(|(_, entry)| entry.value.is_some())?;
		let handle = StableVecHandle::new(index, entry.generation);
		self.remove(handle)
	}

	pub fn clear(&mut self) {
		self.entries.clear();
		self.first_free = None;
		self.len = 0;
	}

	pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
		self.entries.iter().filter_map(|entry| entry.value.as_ref())
	}

	pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
		self.entries.iter_mut().filter_map(|entry| entry.value.as_mut())
	}

	pub fn indexed_iter(&self) -> impl DoubleEndedIterator<Item = (usize, &T)> {
		self.entries
			.iter()
			.enumerate()
			.filter_map(|(index, entry)| entry.value.as_ref().map(|value| (index, value)))
	}

	pub fn indexed_iter_mut(&mut self) -> impl DoubleEndedIterator<Item = (usize, &mut T)> {
		self.entries
			.iter_mut()
			.enumerate()
			.filter_map(|(index, entry)| entry.value.as_mut().map(|value| (index, value)))
	}

	pub fn handled_iter(&self) -> impl DoubleEndedIterator<Item = (StableVecHandle, &T)> {
		self.entries.iter().enumerate().filter_map(|(index, entry)| {
			entry
				.value
				.as_ref()
				.map(|value| (StableVecHandle::new(index, entry.generation), value))
		})
	}

	pub fn handled_iter_mut(&mut self) -> impl DoubleEndedIterator<Item = (StableVecHandle, &mut T)> {
		self.entries.iter_mut().enumerate().filter_map(|(index, entry)| {
			entry
				.value
				.as_mut()
				.map(|value| (StableVecHandle::new(index, entry.generation), value))
		})
	}

	fn valid_entry(&self, handle: StableVecHandle) -> Option<&Entry<T>> {
		let entry = self.entries.get(handle.index())?;
		(entry.generation == handle.generation).then_some(entry)
	}

	fn valid_entry_mut(&mut self, handle: StableVecHandle) -> Option<&mut Entry<T>> {
		let entry = self.entries.get_mut(handle.index())?;
		(entry.generation == handle.generation).then_some(entry)
	}
}

impl<T> Default for StableVec<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T> std::ops::Index<StableVecHandle> for StableVec<T> {
	type Output = T;

	fn index(&self, handle: StableVecHandle) -> &Self::Output {
		self.get(handle).expect(
			"StableVec handle does not contain a value. The most likely cause is that the handle was removed or belongs to another StableVec.",
		)
	}
}

impl<T> std::ops::IndexMut<StableVecHandle> for StableVec<T> {
	fn index_mut(&mut self, handle: StableVecHandle) -> &mut Self::Output {
		self.get_mut(handle).expect(
			"StableVec handle does not contain a value. The most likely cause is that the handle was removed or belongs to another StableVec.",
		)
	}
}

impl<T> FromIterator<T> for StableVec<T> {
	fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
		let entries = iter
			.into_iter()
			.map(|value| Entry {
				value: Some(value),
				generation: 0,
				next_free: None,
			})
			.collect::<Vec<_>>();
		let len = entries.len();

		Self {
			entries,
			first_free: None,
			len,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{StableVec, StableVecHandle};

	#[test]
	fn push_returns_stable_handles() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");

		assert_eq!(first.index(), 0);
		assert_eq!(second.index(), 1);
		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), Some(&"second"));
	}

	#[test]
	fn remove_preserves_other_handles() {
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
	fn push_reuses_removed_slots_with_new_generation() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		let third = values.push("third");

		assert_eq!(values.remove(second), Some("second"));

		let reused = values.push("replacement");

		assert_eq!(reused.index(), second.index());
		assert_ne!(reused.generation(), second.generation());
		assert_eq!(values.get(first), Some(&"first"));
		assert_eq!(values.get(second), None);
		assert_eq!(values.get(reused), Some(&"replacement"));
		assert_eq!(values.get(third), Some(&"third"));
	}

	#[test]
	fn insert_replaces_occupied_slot() {
		let mut values = StableVec::new();

		let handle = values.push("first");

		assert_eq!(values.insert(handle, "replacement"), Some("first"));
		assert_eq!(values.len(), 1);
		assert_eq!(values.get(handle), Some(&"replacement"));
	}

	#[test]
	fn insert_rejects_stale_handle() {
		let mut values = StableVec::new();

		let handle = values.push("first");
		assert_eq!(values.remove(handle), Some("first"));

		assert_eq!(values.insert(handle, "stale"), None);
		assert_eq!(values.len(), 0);
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

	#[test]
	fn handled_iter_returns_live_handles() {
		let mut values = StableVec::new();

		let first = values.push("first");
		let second = values.push("second");
		values.remove(first);

		let entries = values.handled_iter().collect::<Vec<_>>();

		assert_eq!(entries, vec![(second, &"second")]);
	}

	#[test]
	fn stale_foreign_generation_is_rejected() {
		let mut values = StableVec::new();
		let handle = values.push("first");
		let stale = StableVecHandle::new(handle.index(), handle.generation().wrapping_add(1));

		assert_eq!(values.get(stale), None);
		assert_eq!(values.remove(stale), None);
		assert_eq!(values.get(handle), Some(&"first"));
	}
}
