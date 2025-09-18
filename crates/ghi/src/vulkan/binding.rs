use ash::vk;

use crate::vulkan::{DescriptorSetHandle, HandleLike, Next};

#[derive(Clone)]
pub(crate) struct Binding {
	pub next: Option<DescriptorSetBindingHandle>,
	pub descriptor_set_handle: DescriptorSetHandle,
	pub descriptor_type: vk::DescriptorType,
	pub index: u32,
	pub count: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DescriptorSetBindingHandle(pub u64);

impl HandleLike for DescriptorSetBindingHandle {
	type Item = Binding;

	fn build(value: u64) -> Self {
		DescriptorSetBindingHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Binding {
		&collection[self.0 as usize]
	}
}

impl Next for Binding {
	type Handle = DescriptorSetBindingHandle;

	fn next(&self) -> Option<DescriptorSetBindingHandle> {
		self.next
	}
}
