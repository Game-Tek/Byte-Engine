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
	/// Creates empty storage for master-addressed resources and their private frame representations.
	pub(crate) fn new() -> Self {
		Self {
			resources: Vec::new(),
			master_handle_type: PhantomData,
		}
	}

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

	/// Adds another private representation to an existing master handle's resource chain.
	pub(crate) fn add_with_master(&mut self, resource: T, master_handle: MH) -> PH {
		let i = master_handle.index();
		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource {
			next: None,
			resource: Some(resource),
		});

		private_handle
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

	/// Returns the resource for the requested frame offset by walking the master's private chain.
	pub(crate) fn get_nth(&self, handle: MH, frame_offset: usize) -> Option<&T> {
		self.nth_handle(handle, frame_offset).map(|handle| self.resource(handle))
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

	/// Iterates over all stored private resources in insertion order, skipping resources that were taken.
	pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
		self.resources.iter().filter_map(|r| r.resource.as_ref())
	}

	/// Iterates mutably over all stored private resources in insertion order, skipping resources that were taken.
	pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
		self.resources.iter_mut().filter_map(|r| r.resource.as_mut())
	}

	/// Starts building a chained resource sequence that will share one master handle.
	pub(crate) fn creator<'a>(&'a mut self) -> Creator<'a, T, MH, PH> {
		let mh = MH::new(self.resources.len() as _);
		Creator::new(&mut self.resources, mh)
	}

	/// Returns the master handle that would be assigned to the next created resource chain.
	pub(crate) fn master(&self) -> MH {
		MH::new(self.resources.len() as _)
	}
}

/// The `Creator` struct incrementally builds one master resource chain from multiple private frame representations.
pub(crate) struct Creator<'a, T, MH, PH> {
	resources: &'a mut Vec<MasterFrameResource<T, PH>>,
	handle: MH,
}

impl<'a, T, MH: MasterHandle, PH: PrivateHandle> Creator<'a, T, MH, PH> {
	/// Creates a chain builder for a new master handle.
	pub(crate) fn new(resources: &'a mut Vec<MasterFrameResource<T, PH>>, handle: MH) -> Self {
		Self { resources, handle }
	}

	/// Appends a private resource to the end of the current master handle's chain.
	pub(crate) fn add(&mut self, resource: T) -> PH {
		let private_handle = PH::new(self.resources.len() as u64);
		self.resources.push(MasterFrameResource {
			next: None,
			resource: Some(resource),
		});

		let mut current = PH::new(self.handle.index());
		while let Some(last) = self.resources.get_mut(current.index() as usize).unwrap().next {
			current = last;
		}

		self.resources.get_mut(current.index() as usize).unwrap().next = Some(private_handle);

		private_handle
	}

	/// Finishes chain creation and returns the shared master handle.
	pub(crate) fn into(self) -> MH {
		self.handle
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
		let mut resources = super::ResourceCollection::<&'static str, TestMasterHandle, TestPrivateHandle>::new();
		let (master, first) = resources.add("frame 0");
		let (_, second) = resources.add("frame 1");
		resources.set_next(first, Some(second));

		assert_eq!(resources.get_nth(master, 0), Some(&"frame 0"));
		assert_eq!(resources.get_nth(master, 1), Some(&"frame 1"));
		assert_eq!(resources.get_nth(master, 3), Some(&"frame 1"));
	}
}
