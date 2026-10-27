//! The `slots` module keeps resources an application can destroy while copies of their handles live on.

/// The `Slots` struct stores destroyable resources, so a handle to a destroyed resource is caught instead of reaching
/// whatever resource reused its slot.
///
/// A handle packs the slot index in its low 32 bits and the slot's generation in its high 32 bits. Storing a resource
/// advances its slot's generation, so every handle to an earlier resource in the slot stops matching. Generations start
/// at 1, so a zeroed handle never matches.
pub(crate) struct Slots<T> {
	entries: Vec<Slot<T>>,
	/// Slots without a resource, which [`Self::insert`] reuses before growing.
	free: Vec<u32>,
}

/// The `Slot` struct is one entry of [`Slots`] and the generation of the resource it holds or last held.
struct Slot<T> {
	generation: u32,
	value: Option<T>,
}

impl<T> Slots<T> {
	pub(crate) const fn new() -> Self {
		Self {
			entries: Vec::new(),
			free: Vec::new(),
		}
	}

	/// Stores `value` and returns the handle bits that name it.
	pub(crate) fn insert(&mut self, value: T) -> u64 {
		let index = self.free.pop().unwrap_or_else(|| {
			let index = u32::try_from(self.entries.len()).expect(
				"GHI resource slots are exhausted. The most likely cause is that more than u32::MAX resources of one kind exist at once. Destroy resources that are no longer used.",
			);
			self.entries.push(Slot {
				generation: 0,
				value: None,
			});
			index
		});
		let slot = &mut self.entries[index as usize];
		// A slot whose generation reached the maximum is never freed, see `remove`, so this cannot overflow.
		slot.generation += 1;
		slot.value = Some(value);
		Self::handle(index, slot.generation)
	}

	/// Returns the resource `handle` names, or `None` when it was removed.
	pub(crate) fn get(&self, handle: u64) -> Option<&T> {
		let (index, generation) = Self::parts(handle);
		self.entries
			.get(index as usize)
			.filter(|slot| slot.generation == generation)?
			.value
			.as_ref()
	}

	/// Returns the resource `handle` names for changes, or `None` when it was removed.
	pub(crate) fn get_mut(&mut self, handle: u64) -> Option<&mut T> {
		let (index, generation) = Self::parts(handle);
		self.entries
			.get_mut(index as usize)
			.filter(|slot| slot.generation == generation)?
			.value
			.as_mut()
	}

	/// Removes the resource `handle` names and returns it, or `None` when it was already removed.
	pub(crate) fn remove(&mut self, handle: u64) -> Option<T> {
		let (index, generation) = Self::parts(handle);
		let slot = self
			.entries
			.get_mut(index as usize)
			.filter(|slot| slot.generation == generation)?;
		let value = slot.value.take()?;
		// A slot at the last generation would hand out a handle equal to an old one, so it retires instead.
		if slot.generation != u32::MAX {
			self.free.push(index);
		}
		Some(value)
	}

	/// Returns every stored resource.
	pub(crate) fn values(&self) -> impl Iterator<Item = &T> {
		self.entries.iter().filter_map(|slot| slot.value.as_ref())
	}

	/// Returns the slot index of `handle`.
	pub(crate) fn index(handle: u64) -> u32 {
		Self::parts(handle).0
	}

	fn handle(index: u32, generation: u32) -> u64 {
		(u64::from(generation) << 32) | u64::from(index)
	}

	fn parts(handle: u64) -> (u32, u32) {
		(handle as u32, (handle >> 32) as u32)
	}
}

impl<T> Default for Slots<T> {
	fn default() -> Self {
		Self::new()
	}
}
