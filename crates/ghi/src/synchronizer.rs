#[cfg(not(target_os = "macos"))]
use crate::{HandleLike, Next, Synchronizer};
use crate::{PrivateHandle, PrivateHandles};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SynchronizerHandle(pub(crate) u64);

impl From<SynchronizerHandle> for PrivateHandles {
	fn from(val: SynchronizerHandle) -> Self {
		PrivateHandles::Synchronizer(val)
	}
}

impl PrivateHandle for SynchronizerHandle {
	fn new(i: u64) -> Self {
		Self(i)
	}

	fn index(&self) -> u64 {
		self.0
	}
}

// Metal keeps its synchronizer chains in a `ResourceCollection`, so only the other backends link synchronizers themselves.
#[cfg(not(target_os = "macos"))]
impl HandleLike for SynchronizerHandle {
	type Item = Synchronizer;

	fn build(value: u64) -> Self {
		SynchronizerHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Synchronizer {
		&collection[self.0 as usize]
	}
}

#[cfg(not(target_os = "macos"))]
impl Next for Synchronizer {
	type Handle = SynchronizerHandle;

	fn next(&self) -> Option<SynchronizerHandle> {
		self.next
	}
}
