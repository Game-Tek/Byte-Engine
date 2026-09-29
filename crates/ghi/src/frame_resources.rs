//! Storage for backend resources that may need one private representation per frame in flight.
//!
//! A `ResourceCollection` starts each resource chain at a public master handle.
//! It keeps backend-private entries in a contiguous vector. Resources that need
//! distinct per-frame representations use each entry's `next` pointer. A lookup
//! walks this chain from the master resource to the requested frame.
//!
//! A resource that does not need per-frame duplication has one unchained entry.
//! Every frame lookup for this resource resolves to the first private handle.
//!
//! Master handles are stable public identifiers. Private handles identify concrete
//! backend allocations and select a frame-specific resource from a master chain.

use std::marker::PhantomData;

use crate::{MasterHandle, PrivateHandle};

/// Resolves a signed frame offset, where negative values select earlier frames.
pub(crate) fn frame_index_with_offset(sequence_index: usize, frame_offset: i32, frame_count: usize) -> usize {
	let frame_count = frame_count.max(1) as i32;
	(sequence_index as i32 + frame_offset).rem_euclid(frame_count) as usize
}

#[derive(Debug)]
/// The `MasterFrameResource` struct links one backend resource to its next frame-specific representation.
pub(crate) struct MasterFrameResource<T, PH> {
	next: Option<PH>,
	/// `None` after [`ResourceCollection::take`] moved the resource out. The slot stays so no handle is reused.
	resource: Option<T>,
}

const TAKEN_RESOURCE: &str = "Resource was moved out of its context. The most likely cause is that a handle was used after its image was exported to another context.";

#[derive(Debug)]
/// The `ResourceCollection` struct provides master-handle lookup across per-frame resource chains.
pub(crate) struct ResourceCollection<T, MH, PH> {
	resources: Vec<MasterFrameResource<T, PH>>,
	master_handle_type: PhantomData<MH>,
}

impl<T, MH, PH> Default for ResourceCollection<T, MH, PH> {
	fn default() -> Self {
		Self {
			resources: Vec::new(),
			master_handle_type: PhantomData,
		}
	}
}

impl<T, MH: MasterHandle, PH: PrivateHandle> ResourceCollection<T, MH, PH> {
	/// Creates empty storage with capacity for the requested number of private resources.
	pub(crate) fn with_capacity(capacity: usize) -> Self {
		Self {
			resources: Vec::with_capacity(capacity),
			master_handle_type: PhantomData,
		}
	}

	/// Adds a resource as both the public master entry and its first private representation.
	pub(crate) fn add(&mut self, resource: T) -> (MH, PH) {
		let i = self.resources.len() as u64;
		let master_handle = MH::new(i);
		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource {
			next: None,
			resource: Some(resource),
		});

		(master_handle, private_handle)
	}

	/// Adds one resource chain, with an entry per item of `resources` in frame-sequence order, and returns its public
	/// handle.
	///
	/// Use it for resources that need a private copy per frame in flight, such as synchronizers, descriptor sets, and
	/// dynamic buffers and images. Every copy exists from the start, so no frame ever shares a copy with another frame
	/// in flight. Resolve a frame's copy with [`Self::nth_handle`].
	///
	/// # Panics
	///
	/// Panics when `resources` is empty.
	pub(crate) fn add_chain(&mut self, resources: impl IntoIterator<Item = T>) -> MH {
		let mut resources = resources.into_iter();
		let first = resources.next().expect(
			"Empty resource chain. The most likely cause is that a per-frame resource was created with zero frames in flight.",
		);
		let (master, mut previous) = self.add(first);
		for resource in resources {
			let (_, next) = self.add(resource);
			self.set_next(previous, Some(next));
			previous = next;
		}
		master
	}

	/// Returns the resource a master handle names when it has a single representation for every frame.
	///
	/// Returns `None` when the handle is unknown, was taken, or names a chain with one representation per frame.
	pub(crate) fn get_unique(&self, handle: MH) -> Option<&T> {
		self.resources
			.get(handle.index() as usize)
			.filter(|entry| entry.next.is_none())?
			.resource
			.as_ref()
	}

	/// Moves a resource with a single representation out and leaves its slot empty.
	///
	/// Use this to hand a resource to another owner. The master handle stays reserved, so it can never name a
	/// different resource later. Returns `None` in the same cases as [`Self::get_unique`].
	pub(crate) fn take(&mut self, handle: MH) -> Option<T> {
		self.resources
			.get_mut(handle.index() as usize)
			.filter(|entry| entry.next.is_none())?
			.resource
			.take()
	}

	/// Updates the next link for one private resource in the chain.
	pub(crate) fn set_next(&mut self, private_handle: PH, next: Option<PH>) {
		self.entry_mut(private_handle).next = next;
	}

	/// Returns the chain entry addressed by the provided private handle.
	fn entry(&self, private_handle: PH) -> &MasterFrameResource<T, PH> {
		self.resources
			.get(private_handle.index() as usize)
			.expect("Invalid private handle. The most likely cause is that the handle was not created by this storage.")
	}

	/// Returns mutable access to the chain entry addressed by the provided private handle.
	fn entry_mut(&mut self, private_handle: PH) -> &mut MasterFrameResource<T, PH> {
		self.resources
			.get_mut(private_handle.index() as usize)
			.expect("Invalid private handle. The most likely cause is that the handle was not created by this storage.")
	}

	/// Returns the backend-private resource addressed by the provided private handle.
	pub(crate) fn resource(&self, private_handle: PH) -> &T {
		self.entry(private_handle).resource.as_ref().expect(TAKEN_RESOURCE)
	}

	/// Returns mutable access to the backend-private resource addressed by the provided private handle.
	pub(crate) fn resource_mut(&mut self, private_handle: PH) -> &mut T {
		self.entry_mut(private_handle).resource.as_mut().expect(TAKEN_RESOURCE)
	}

	/// Returns the first resource for a master handle without walking any per-frame chain.
	///
	/// Use this method for a resource with one representation.
	pub(crate) fn get_single(&self, handle: MH) -> Option<&T> {
		self.resources.get(handle.index() as usize).and_then(|r| r.resource.as_ref())
	}

	/// Returns the private handle for the requested frame offset within a master's chain.
	///
	/// If the chain is shorter than the requested offset, this method returns its
	/// last private handle. A single-entry resource therefore resolves to the same
	/// representation for every frame.
	pub(crate) fn nth_handle(&self, handle: MH, frame_offset: usize) -> Option<PH> {
		let mut current = PH::new(handle.index());

		let mut i = 0;
		while i < frame_offset {
			if let Some(next) = self.entry(current).next {
				current = next;
			} else {
				break;
			}

			i += 1;
		}

		Some(current)
	}

	/// Returns every private handle of a master's chain, in frame order, walking the chain once.
	///
	/// The chain must end: chains built with [`Self::add_chain`], or with [`Self::set_next`] onto new entries, always do.
	pub(crate) fn chain(&self, handle: MH) -> impl Iterator<Item = PH> + '_ {
		std::iter::successors(Some(PH::new(handle.index())), |&current| self.entry(current).next)
	}

	/// Iterates over all stored private resources in insertion order, skipping resources that were taken.
	pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
		self.resources.iter().filter_map(|r| r.resource.as_ref())
	}

	/// Iterates mutably over all stored private resources in insertion order, skipping resources that were taken.
	pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
		self.resources.iter_mut().filter_map(|r| r.resource.as_mut())
	}
}

#[cfg(test)]
mod tests {
	use super::frame_index_with_offset;
	use crate::{MasterHandle, PrivateHandle};

	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	struct TestMasterHandle(u64);

	impl MasterHandle for TestMasterHandle {
		fn new(i: u64) -> Self {
			Self(i)
		}

		fn index(&self) -> u64 {
			self.0
		}
	}

	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	struct TestPrivateHandle(u64);

	impl PrivateHandle for TestPrivateHandle {
		fn new(i: u64) -> Self {
			Self(i)
		}

		fn index(&self) -> u64 {
			self.0
		}
	}

	#[test]
	fn signed_frame_offsets_select_relative_frames_and_wrap() {
		assert_eq!(frame_index_with_offset(1, -1, 3), 0);
		assert_eq!(frame_index_with_offset(1, 1, 3), 2);
		assert_eq!(frame_index_with_offset(0, -1, 3), 2);
		assert_eq!(frame_index_with_offset(2, 1, 3), 0);
	}

	#[test]
	fn nth_handle_follows_the_frame_chain_and_reuses_its_last_resource() {
		let mut resources = super::ResourceCollection::<&'static str, TestMasterHandle, TestPrivateHandle>::default();
		let (master, first) = resources.add("frame 0");
		let (_, second) = resources.add("frame 1");
		resources.set_next(first, Some(second));

		let nth = |offset| resources.nth_handle(master, offset).map(|handle| *resources.resource(handle));
		assert_eq!(nth(0), Some("frame 0"));
		assert_eq!(nth(1), Some("frame 1"));
		assert_eq!(nth(3), Some("frame 1"));
	}

	#[test]
	fn chains_give_every_frame_sequence_its_own_resource() {
		let mut resources = super::ResourceCollection::<&'static str, TestMasterHandle, TestPrivateHandle>::default();
		let first = resources.add_chain(["first 0", "first 1", "first 2"]);
		let second = resources.add_chain(["second 0", "second 1"]);

		let nth = |master, offset| resources.nth_handle(master, offset).map(|handle| *resources.resource(handle));
		assert_eq!(nth(first, 0), Some("first 0"));
		assert_eq!(nth(first, 1), Some("first 1"));
		assert_eq!(nth(first, 2), Some("first 2"));
		assert_eq!(nth(second, 0), Some("second 0"));
		assert_eq!(nth(second, 1), Some("second 1"));
		assert_eq!(resources.chain(first).count(), 3);
		assert_eq!(resources.chain(second).count(), 2);
	}
}
