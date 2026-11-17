use std::time::{Duration, Instant};

use crate::{PrivateHandle, PrivateHandles};

/// Sleeps until the next paced present slot and advances it by `interval`.
///
/// Backends without a timed present call this before acquisition so the frame cannot start earlier than the
/// interval allows; the following FIFO present then lands on the next refresh. A missed slot restarts from now
/// instead of catching up. Without an interval the slot is cleared and the call returns immediately.
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "Only the Vulkan and DX12 backends use it."))]
pub(crate) fn pace_present(next_slot: &mut Option<Instant>, interval: Option<Duration>) {
	let Some(interval) = interval else {
		*next_slot = None;
		return;
	};
	let now = Instant::now();
	let slot = match *next_slot {
		Some(slot) if slot > now => {
			std::thread::sleep(slot - now);
			slot
		}
		_ => now,
	};
	*next_slot = Some(slot + interval);
}

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub(crate) struct SwapchainHandle;
}

impl From<SwapchainHandle> for PrivateHandles {
	fn from(val: SwapchainHandle) -> Self {
		PrivateHandles::Swapchain(val)
	}
}

impl PrivateHandle for SwapchainHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		SwapchainHandle::index(*self)
	}
}

/// The `Swapchains` struct stores a backend's swapchains by handle index, so destroying one leaves every other
/// handle valid. A destroyed swapchain's index is never reused, and reaching it through a handle panics.
pub(crate) struct Swapchains<T> {
	entries: Vec<Option<T>>,
}

impl<T> Default for Swapchains<T> {
	fn default() -> Self {
		Self { entries: Vec::new() }
	}
}

#[allow(dead_code, reason = "Each backend uses a different part of the store.")]
impl<T> Swapchains<T> {
	/// Stores `swapchain` and returns its handle index.
	pub(crate) fn push(&mut self, swapchain: T) -> u64 {
		self.entries.push(Some(swapchain));
		(self.entries.len() - 1) as u64
	}

	/// The number of handle indices given out, destroyed swapchains included.
	pub(crate) fn len(&self) -> usize {
		self.entries.len()
	}

	/// Returns the swapchain at `index`, or `None` when there's none or it was destroyed.
	pub(crate) fn get(&self, index: usize) -> Option<&T> {
		self.entries.get(index)?.as_ref()
	}

	/// Returns the swapchain at `index` for changes, or `None` when there's none or it was destroyed.
	pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
		self.entries.get_mut(index)?.as_mut()
	}

	/// Takes the swapchain at `index` out of the store, which leaves its index empty for good.
	pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
		self.entries.get_mut(index)?.take()
	}

	/// Returns every swapchain that wasn't destroyed.
	pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
		self.entries.iter().flatten()
	}

	/// Returns every swapchain that wasn't destroyed, with its handle index.
	pub(crate) fn indexed(&self) -> impl Iterator<Item = (usize, &T)> {
		self.entries
			.iter()
			.enumerate()
			.filter_map(|(index, swapchain)| Some((index, swapchain.as_ref()?)))
	}

	/// Returns every swapchain that wasn't destroyed, for changes.
	pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
		self.entries.iter_mut().flatten()
	}
}

impl<T> std::ops::Index<usize> for Swapchains<T> {
	type Output = T;

	fn index(&self, index: usize) -> &T {
		self.get(index).expect(DESTROYED)
	}
}

impl<T> std::ops::IndexMut<usize> for Swapchains<T> {
	fn index_mut(&mut self, index: usize) -> &mut T {
		self.get_mut(index).expect(DESTROYED)
	}
}

impl<'a, T> IntoIterator for &'a Swapchains<T> {
	type Item = &'a T;
	type IntoIter = std::iter::Flatten<std::slice::Iter<'a, Option<T>>>;

	fn into_iter(self) -> Self::IntoIter {
		self.entries.iter().flatten()
	}
}

impl<'a, T> IntoIterator for &'a mut Swapchains<T> {
	type Item = &'a mut T;
	type IntoIter = std::iter::Flatten<std::slice::IterMut<'a, Option<T>>>;

	fn into_iter(self) -> Self::IntoIter {
		self.entries.iter_mut().flatten()
	}
}

const DESTROYED: &str = "The swapchain was destroyed. The most likely cause is that a handle was used after `destroy_swapchain`. Stop presenting to a swapchain before destroying it.";
