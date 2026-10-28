use crate::{Binding, HandleLike, Next};

crate::handle::index_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
	pub struct DescriptorSetBindingHandle;
}

impl HandleLike for DescriptorSetBindingHandle {
	type Item = Binding;

	fn build(value: u64) -> Self {
		DescriptorSetBindingHandle::from_index(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Binding {
		&collection[self.index() as usize]
	}
}

impl Next for Binding {
	type Handle = DescriptorSetBindingHandle;

	fn next(&self) -> Option<DescriptorSetBindingHandle> {
		self.next
	}
}
