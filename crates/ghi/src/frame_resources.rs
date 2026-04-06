//! Storage for backend resources that may need one private representation per frame in flight.
//!
//! A `ResourceCollection` starts each resource chain from a public master handle and stores the
//! backend-private entries in a contiguous vector. Resources that need distinct per-frame
//! representations are linked together through each entry's `next` pointer, so looking up the
//! `n`th frame representation means walking that chain from the master resource.
//!
//! Resources that do not need per-frame duplication stay as a single unchained entry. In that
//! case every frame lookup resolves to the same first private handle because no `next` link is
//! present.
//!
//! Master handles are the stable public identifiers exposed to the rest of the GHI. Private
//! handles identify one concrete backend allocation inside this storage, which lets the backend
//! address the exact per-frame resource instance selected from a master handle's chain.

use std::marker::PhantomData;

use smallvec::SmallVec;

use crate::{MasterHandle, PrivateHandle};

#[derive(Debug)]
/// The `MasterFrameResource` struct stores one backend-private resource and an optional link to the next frame-specific representation.
pub struct MasterFrameResource<T, PH> {
	next: Option<PH>,
	resource: T,
}

impl<T, PH: Copy> MasterFrameResource<T, PH> {
	/// Returns the backend-private resource stored in this chain entry.
	pub fn resource(&self) -> &T {
		&self.resource
	}

	/// Returns mutable access to the backend-private resource stored in this chain entry.
	pub fn resource_mut(&mut self) -> &mut T {
		&mut self.resource
	}

	/// Extracts the backend-private resource from this chain entry.
	pub fn into_inner(self) -> T {
		self.resource
	}
}

#[derive(Debug)]
/// The `ResourceCollection` struct stores master-addressed resources whose private representations may form per-frame chains.
pub struct ResourceCollection<T, MH, PH> {
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
	pub fn new() -> Self {
		Self {
			resources: Vec::new(),
			master_handle_type: PhantomData,
		}
	}

	/// Creates empty storage with capacity for the requested number of private resources.
	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			resources: Vec::with_capacity(capacity),
			master_handle_type: PhantomData,
		}
	}

	/// Adds a resource as both the public master entry and its first private representation.
	pub fn add(&mut self, resource: T) -> (MH, PH) {
		let i = self.resources.len() as u64;

		let master_handle = MH::new(i);

		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource { next: None, resource });

		(master_handle, private_handle)
	}

	/// Adds another private representation to an existing master handle's resource chain.
	pub fn add_with_master(&mut self, resource: T, master_handle: MH) -> PH {
		let i = master_handle.index();

		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource { next: None, resource });

		private_handle
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
	pub fn resource(&self, private_handle: PH) -> &T {
		&self.entry(private_handle).resource
	}

	/// Returns mutable access to the backend-private resource addressed by the provided private handle.
	pub fn resource_mut(&mut self, private_handle: PH) -> &mut T {
		&mut self.entry_mut(private_handle).resource
	}

	/// Returns the first resource for a master handle without walking any per-frame chain.
	///
	/// This is useful for resources that only have a single representation.
	pub fn get_single(&self, handle: MH) -> Option<&T> {
		self.resources.get(handle.index() as usize).map(|r| &r.resource)
	}

	/// Returns the resource for the requested frame offset by walking the master's private chain.
	pub(crate) fn get_nth(&self, handle: MH, frame_offset: usize) -> Option<&T> {
		self.nth_handle(handle, frame_offset).map(|handle| self.resource(handle))
	}

	/// Returns the private handle for the requested frame offset within a master's chain.
	///
	/// If the chain is shorter than the requested offset, the last available private handle is
	/// returned. This allows single-entry resources to resolve to the same representation for every
	/// frame.
	pub(crate) fn nth_handle(&self, handle: MH, frame_offset: usize) -> Option<PH> {
		let mut current = PH::new(handle.index());

		{
			let mut i = 0;

			while i <= frame_offset {
				if let Some(next) = self.entry(current).next {
					current = next;
				} else {
					break;
				}

				i += 1;
			}
		}

		Some(current)
	}

	/// Iterates over all stored private resources in insertion order.
	pub fn iter(&self) -> impl Iterator<Item = &T> {
		self.resources.iter().map(|r| &r.resource)
	}

	/// Starts building a chained resource sequence that will share one master handle.
	pub fn creator<'a>(&'a mut self) -> Creator<'a, T, MH, PH> {
		let mh = MH::new(self.resources.len() as _);
		Creator::new(&mut self.resources, mh)
	}

	/// Returns the master handle that would be assigned to the next created resource chain.
	pub fn master(&self) -> MH {
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
		self.resources.push(MasterFrameResource { next: None, resource });

		{
			let mut current = PH::new(self.handle.index());

			while let Some(last) = self.resources.get_mut(current.index() as usize).unwrap().next {
				current = last;
			}

			self.resources.get_mut(current.index() as usize).unwrap().next = Some(private_handle);
		}

		private_handle
	}

	/// Finishes chain creation and returns the shared master handle.
	pub fn into(self) -> MH {
		self.handle
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	struct MasterHandle(u64);
}
