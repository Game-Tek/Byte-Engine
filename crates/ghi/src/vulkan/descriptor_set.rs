use smallvec::SmallVec;

use crate::descriptors::DescriptorSetHandle;

/// The `RetainedDescriptor` struct keeps one write to a set element, so each frame-local copy of the set resolves it
/// to the resources of its own frame sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RetainedDescriptor {
	pub(crate) descriptor: crate::descriptors::WriteData,
	pub(crate) frame_offset: i32,
}

/// The `DescriptorSet` struct is a descriptor set that lives in the context's descriptor heaps, where shaders read it.
///
/// It keeps the writes it was given, and one copy in the heaps for each frame in flight, which commands of that
/// frame sequence bind. A copy rewrites only the elements whose writes, or whose resources, changed since commands
/// last bound it. See [`crate::vulkan::Context::bind_descriptor_set`].
pub struct DescriptorSet {
	/// The set that follows this one in its chain, which is always `None`: a set holds its frame-local copies itself.
	pub next: Option<DescriptorSetHandle>,
	pub(crate) layout: crate::shader::DescriptorSetLayout,
	/// For each resource of the layout, how many samplers the resources before it hold. See
	/// [`crate::vulkan::holds_samplers`].
	pub(crate) sampler_bases: Box<[u32]>,
	/// How many samplers the set's resources hold.
	pub(crate) sampler_count: u32,
	/// The write of each array element, in layout order, or `None` for an element that was never written.
	pub(crate) descriptors: Box<[Option<RetainedDescriptor>]>,
	/// One copy per frame sequence.
	pub(crate) copies: SmallVec<[SetCopy; super::MAX_FRAMES_IN_FLIGHT]>,
}

impl DescriptorSet {
	/// Returns the layout resource that holds array element `element`, and the element's index in that resource.
	pub(crate) fn resource_of(&self, element: u32) -> (usize, u32) {
		let resources = self.layout.resources().len();
		// Element bases grow with the resource index, so the holder is the last resource that starts at or before it.
		let mut low = 0;
		let mut high = resources;
		while high - low > 1 {
			let middle = (low + high) / 2;
			if self.layout.element_base(middle) <= element {
				low = middle;
			} else {
				high = middle;
			}
		}
		(low, element - self.layout.element_base(low))
	}
}

/// The `SetCopy` struct is the part of the descriptor heaps one frame sequence's commands read a set from.
pub(crate) struct SetCopy {
	/// The resource heap slot of the set's first descriptor.
	pub(crate) resource_base: u32,
	/// The sampler heap index of the set's first sampler.
	pub(crate) sampler_base: u32,
	/// The elements to write into the heaps before commands bind the copy again. It may repeat an element.
	pub(crate) dirty: Vec<u32>,
	/// The resources the copy makes commands access, merged by resource and layout.
	pub(crate) consumptions: Vec<SetConsumption>,
	/// The frame whose commands last bound the copy. Until that frame completes, a change moves the copy to new
	/// heap ranges instead of writing over the ones those commands read.
	pub(crate) bound_frame: Option<u64>,
}

/// The `SetConsumption` struct is one resource that a bound set makes commands access, and which of the set's
/// resources name it, so each pipeline consumes it with the stages that read those resources.
#[derive(Clone, Copy)]
pub(crate) struct SetConsumption {
	pub(crate) handle: super::Handles,
	/// The layout an image is accessed in, and [`crate::Layouts::General`] for other resources.
	pub(crate) layout: crate::Layouts,
	/// One bit per layout resource whose elements name the resource.
	pub(crate) resources: u64,
}
