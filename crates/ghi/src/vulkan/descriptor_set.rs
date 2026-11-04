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
/// It keeps the writes it was given, and copies in the heaps that commands bind: one for recordings outside frames,
/// and one for each frame in flight, which commands of that frame sequence bind. A copy rewrites only the elements
/// whose writes, or whose resources, changed since commands last bound it. See
/// [`crate::vulkan::Context::bind_descriptor_set`].
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
	/// The copy recordings outside frames bind, then one copy per frame sequence, in the order [`CopyKey::index`]
	/// gives.
	pub(crate) copies: SmallVec<[SetCopy; super::MAX_FRAMES_IN_FLIGHT + 1]>,
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
	/// The work that last bound the copy. Until it completes, a change moves the copy to new heap ranges instead of
	/// writing over the ones its commands read.
	pub(crate) reader: Option<Reader>,
}

/// The `CopyKey` enum names the copy of a set that a recording binds: the one recordings outside frames share, or
/// the one of a frame's sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyKey {
	/// A recording outside frames, such as a GPU worker's, which its synchronizer retires.
	Recording,
	/// Frame `frame`, on frame sequence `sequence`.
	Frame { sequence: u8, frame: u64 },
}

impl CopyKey {
	/// Returns the index of the copy in [`DescriptorSet::copies`].
	pub(crate) fn index(self) -> usize {
		match self {
			CopyKey::Recording => 0,
			CopyKey::Frame { sequence, .. } => 1 + sequence as usize,
		}
	}

	/// Returns the frame sequence whose resources the copy's writes resolve to. Recordings outside frames read the
	/// first sequence's.
	pub(crate) fn sequence(self) -> u8 {
		match self {
			CopyKey::Recording => 0,
			CopyKey::Frame { sequence, .. } => sequence,
		}
	}
}

/// The `Reader` enum is the work that last bound a set copy, which decides whether the copy may still be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reader {
	/// The frame of this index, whose completion its frame sequence's next frame waits for.
	Frame(u64),
	/// A recording outside frames that has not executed yet.
	OpenRecording,
	/// A recording outside frames that executed, and the GPU work it completes with.
	Recording(super::Completion),
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
