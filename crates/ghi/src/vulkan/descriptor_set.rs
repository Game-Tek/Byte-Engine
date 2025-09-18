use ash::vk;

use crate::{graphics_hardware_interface, vulkan::{HandleLike, Next}};

#[derive(Clone)]
pub(crate) struct DescriptorSet {
	pub next: Option<DescriptorSetHandle>,
	pub descriptor_set: vk::DescriptorSet,
	pub descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,
}

impl Next for DescriptorSet {
	type Handle = DescriptorSetHandle;

	fn next(&self) -> Option<DescriptorSetHandle> {
		self.next
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetHandle(pub u64);

impl HandleLike for DescriptorSetHandle {
	type Item = DescriptorSet;

	fn build(value: u64) -> Self {
		DescriptorSetHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a DescriptorSet {
		&collection[self.0 as usize]
	}
}
