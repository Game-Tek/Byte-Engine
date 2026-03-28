use std::marker::PhantomData;

use smallvec::SmallVec;

use crate::{MasterHandle, PrivateHandle};

#[derive(Debug)]
/// The `MasterFrameResource` struct stores a backend-private resource together with the public handle that owns it.
pub struct MasterFrameResource<T, PH> {
	next: Option<PH>,
	resource: T,
}

impl<T, PH: Copy> MasterFrameResource<T, PH> {
	pub fn resource(&self) -> &T {
		&self.resource
	}

	pub fn resource_mut(&mut self) -> &mut T {
		&mut self.resource
	}

	pub fn into_inner(self) -> T {
		self.resource
	}
}

#[derive(Debug)]
/// The `MasterFrameResources` struct centralizes public master handles and their per-frame private resources.
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
	pub fn new() -> Self {
		Self {
			resources: Vec::new(),
			master_handle_type: PhantomData,
		}
	}

	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			resources: Vec::with_capacity(capacity),
			master_handle_type: PhantomData,
		}
	}

	pub fn add(&mut self, resource: T) -> (MH, PH) {
		let i = self.resources.len() as u64;

		let master_handle = MH::new(i);

		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource { next: None, resource });

		(master_handle, private_handle)
	}

	pub fn add_with_master(&mut self, resource: T, master_handle: MH) -> PH {
		let i = master_handle.index();

		let private_handle = PH::new(i);

		self.resources.push(MasterFrameResource { next: None, resource });

		private_handle
	}

	pub(crate) fn set_next(&mut self, private_handle: PH, next: Option<PH>) {
		self.entry_mut(private_handle).next = next;
	}

	fn entry(&self, private_handle: PH) -> &MasterFrameResource<T, PH> {
		self.resources
			.get(private_handle.index() as usize)
			.expect("Invalid private handle. The most likely cause is that the handle was not created by this storage.")
	}

	fn entry_mut(&mut self, private_handle: PH) -> &mut MasterFrameResource<T, PH> {
		self.resources
			.get_mut(private_handle.index() as usize)
			.expect("Invalid private handle. The most likely cause is that the handle was not created by this storage.")
	}

	pub fn resource(&self, private_handle: PH) -> &T {
		&self.entry(private_handle).resource
	}

	pub fn resource_mut(&mut self, private_handle: PH) -> &mut T {
		&mut self.entry_mut(private_handle).resource
	}

	pub fn get_single(&self, handle: MH) -> Option<&T> {
		self.resources.get(handle.index() as usize).map(|r| &r.resource)
	}

	pub(crate) fn get_nth(&self, handle: MH, frame_offset: i64) -> Option<&T> {
		self.nth_handle(handle, frame_offset).map(|handle| self.resource(handle))
	}

	pub(crate) fn nth_handle(&self, handle: MH, frame_offset: i64) -> Option<PH> {
		let frame_offset = usize::try_from(frame_offset).ok()?;
		let mut current = PH::new(handle.index());

		for _ in 0..frame_offset {
			current = self.entry(current).next?;
		}

		Some(current)
	}

	pub fn iter(&self) -> impl Iterator<Item = &T> {
		self.resources.iter().map(|r| &r.resource)
	}

	pub fn creator<'a>(&'a mut self) -> Creator<'a, T, MH, PH> {
		let mh = MH::new(self.resources.len() as _);
		Creator::new(&mut self.resources, mh)
	}

	pub fn master(&self) -> MH {
		MH::new(self.resources.len() as _)
	}
}

pub(crate) struct Creator<'a, T, MH, PH> {
	resources: &'a mut Vec<MasterFrameResource<T, PH>>,
	handle: MH,
}

impl<'a, T, MH: MasterHandle, PH: PrivateHandle> Creator<'a, T, MH, PH> {
	pub(crate) fn new(resources: &'a mut Vec<MasterFrameResource<T, PH>>, handle: MH) -> Self {
		Self { resources, handle }
	}

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
