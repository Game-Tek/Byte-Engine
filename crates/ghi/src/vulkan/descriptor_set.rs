use ash::vk;

use crate::{descriptors::DescriptorSetHandle, graphics_hardware_interface};

#[derive(Clone)]
pub(crate) struct DescriptorSet {
	pub next: Option<DescriptorSetHandle>,
	pub descriptor_set: vk::DescriptorSet,
	pub descriptor_set_layout: graphics_hardware_interface::DescriptorSetTemplateHandle,
}
