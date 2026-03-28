use ash::vk;

use crate::{binding::DescriptorSetBindingHandle, descriptors::DescriptorSetHandle};

#[derive(Clone)]
pub(crate) struct Binding {
	pub next: Option<DescriptorSetBindingHandle>,
	pub descriptor_set_handle: DescriptorSetHandle,
	pub descriptor_type: vk::DescriptorType,
	pub index: u32,
	pub _count: u32,
}
