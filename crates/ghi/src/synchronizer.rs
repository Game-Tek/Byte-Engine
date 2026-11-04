#[cfg(not(target_os = "macos"))]
use crate::{HandleLike, Next, Synchronizer};
use crate::{PrivateHandle, PrivateHandles};

crate::handle::index_handle! {
	#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
	pub(crate) struct SynchronizerHandle;
}

impl From<SynchronizerHandle> for PrivateHandles {
	fn from(val: SynchronizerHandle) -> Self {
		PrivateHandles::Synchronizer(val)
	}
}

impl PrivateHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self::from_index(i)
	}

	fn index(&self) -> u64 {
		SynchronizerHandle::index(*self)
	}
}

// Metal keeps its synchronizer chains in a `ResourceCollection`, so only the other backends link synchronizers themselves.
#[cfg(not(target_os = "macos"))]
impl HandleLike for SynchronizerHandle {
	type Item = Synchronizer;

	fn build(value: u64) -> Self {
		SynchronizerHandle::from_index(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Synchronizer {
		&collection[self.index() as usize]
	}
}

#[cfg(not(target_os = "macos"))]
impl Next for Synchronizer {
	type Handle = SynchronizerHandle;

	fn next(&self) -> Option<SynchronizerHandle> {
		self.next
	}
}
