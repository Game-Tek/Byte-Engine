use crate::{HandleLike, Next, PrivateHandles, Synchronizer};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SynchronizerHandle(pub(crate) u64);

impl Into<PrivateHandles> for SynchronizerHandle {
	fn into(self) -> PrivateHandles {
		PrivateHandles::Synchronizer(self)
	}
}

impl HandleLike for SynchronizerHandle {
	type Item = Synchronizer;

	fn build(value: u64) -> Self {
		SynchronizerHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Synchronizer {
		&collection[self.0 as usize]
	}
}

impl Next for Synchronizer {
	type Handle = SynchronizerHandle;

	fn next(&self) -> Option<SynchronizerHandle> {
		self.next
	}
}
