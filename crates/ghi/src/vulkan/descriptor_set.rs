use utils::hash::HashMap;

use crate::descriptors::DescriptorSetHandle;

/// The `RetainedDescriptor` struct preserves one logical write until a pipeline materializes it into descriptor heaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RetainedDescriptor {
	pub(crate) descriptor: crate::descriptors::WriteData,
	pub(crate) frame_offset: i32,
}

/// The `DescriptorSet` struct provides retained Vulkan descriptor state without allocating native descriptor sets.
#[derive(Clone)]
pub struct DescriptorSet {
	pub next: Option<DescriptorSetHandle>,
	pub(crate) version: u64,
	pub(crate) sequence_versions: [u64; super::MAX_FRAMES_IN_FLIGHT],
	pub(crate) descriptors: HashMap<crate::shader::ResourceSlot, HashMap<u32, RetainedDescriptor>>,
}
