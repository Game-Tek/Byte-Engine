use super::*;

pub mod buffer {
	use super::*;
	use crate::{DeviceAccesses, Uses};

	#[derive(Clone)]
	pub(crate) struct Buffer {
		pub(crate) name: Option<String>,
		pub(crate) staging: Option<BufferHandle>,
		pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub(crate) size: usize,
		pub(crate) gpu_address: u64,
		pub(crate) pointer: *mut u8,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
	}
}

pub mod image {
	use super::*;
	use crate::{DeviceAccesses, Formats, Uses};

	/// The `ImageDescription` struct keeps the creation parameters that rebuilding an image, for example at a new
	/// extent, reuses.
	#[derive(Clone, Copy)]
	pub(crate) struct ImageDescription {
		pub(crate) extent: Extent,
		pub(crate) format: Formats,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
		pub(crate) array_layers: u32,
		pub(crate) cube_compatible: bool,
		pub(crate) cube_array_compatible: bool,
		pub(crate) mip_levels: u32,
	}

	impl ImageDescription {
		pub(crate) fn new(builder: &crate::image::Builder) -> Self {
			Self {
				extent: builder.extent,
				format: builder.format,
				uses: builder.resource_uses,
				access: builder.device_accesses,
				array_layers: builder.array_layers.map_or(1, std::num::NonZeroU32::get),
				cube_compatible: builder.cube_compatible,
				cube_array_compatible: builder.cube_array_compatible,
				mip_levels: builder.mip_levels,
			}
		}
	}

	/// The `Image` struct owns one Metal texture, the description it was created from, and its CPU staging bytes.
	///
	/// A [`Factory`] can build one away from the render thread; [`Frame::intern_image`] hands it to a context.
	pub struct Image {
		pub(crate) name: Option<String>,
		pub(crate) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
		pub(crate) description: ImageDescription,
		/// Present only for images the CPU may access.
		pub(crate) staging: Option<Vec<u8>>,
		/// Present only for image-group members after their group is placed.
		pub(crate) slot: Option<GroupSlot>,
	}

	/// The `GroupSlot` struct locates an image-group member inside a group heap.
	///
	/// Recording uses it to make the heap resident and to track the member by the bytes it occupies, so hazards
	/// between members that share memory are ordered like hazards on one buffer.
	#[derive(Clone)]
	pub(crate) struct GroupSlot {
		pub(crate) heap: Retained<ProtocolObject<dyn mtl::MTLHeap>>,
		/// Identifies the heap for hazard tracking. It is never reused, so a replaced heap's history cannot leak into
		/// its replacement.
		pub(crate) heap_serial: u64,
		pub(crate) offset: usize,
		pub(crate) size: usize,
	}

	// SAFETY: The image owns a retained Metal texture, which Metal documents as usable from any thread, and plain
	// metadata. Moving it between threads adds no shared state.
	unsafe impl Send for Image {}
}

pub mod sampler {
	use super::*;

	/// The `Sampler` struct owns one Metal sampler state; a [`Factory`] can build it for later interning.
	pub struct Sampler {
		pub(crate) sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>,
	}
}

pub mod descriptor_set {
	use super::*;
	use crate::descriptors::DescriptorSetHandle;

	/// The `DescriptorSet` struct provides Metal descriptor state for one frame.
	#[derive(Clone)]
	pub(crate) struct DescriptorSet {
		pub next: Option<DescriptorSetHandle>,
		pub version: u64,
		pub descriptors: HashMap<crate::shader::ResourceSlot, HashMap<u32, Descriptor>>,
		/// Argument-buffer snapshots encoded with this set bound first, one per pipeline layout and set union.
		pub argument_buffers: Vec<Materialization>,
	}
}

pub mod synchronizer {
	use super::*;
	use crate::metal::queue::{StoredQueue, SubmittedBatch};
	use crate::synchronizer::SynchronizerHandle;

	/// The `Synchronizer` struct owns the Metal workloads associated with one GHI synchronization point.
	///
	/// A synchronizer with no pending workloads is signaled.
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		workloads: SmallVec<[SubmittedBatch; 4]>,
	}

	impl Synchronizer {
		pub(crate) fn new() -> Self {
			Self {
				next: None,
				workloads: SmallVec::new(),
			}
		}

		pub(crate) fn signal(&mut self, workload: SubmittedBatch) {
			self.workloads.push(workload);
		}

		/// Waits for every submitted batch, returns its commands to `queues` for reuse, and reports the first GPU error.
		pub(crate) fn wait(&mut self, queues: &mut [StoredQueue]) -> Option<String> {
			let mut first_error = None;
			for workload in self.workloads.drain(..) {
				if let Some(error) = workload.wait(queues) {
					first_error.get_or_insert(error);
				}
			}
			first_error
		}
	}
}

pub mod swapchain {
	use std::sync::Arc;

	use super::*;
	use crate::image::ImageHandle;

	#[derive(Clone)]
	pub(crate) struct Swapchain {
		pub layer: Retained<CAMetalLayer>,
		pub view: Retained<NSView>,
		/// One proxy image per frame sequence, present only when the declared uses cannot be applied to a drawable texture.
		pub images: [Option<ImageHandle>; MAX_FRAMES_IN_FLIGHT],
		pub uses_proxy: bool,
		pub uses: crate::Uses,
		pub extent: Extent,
		/// The drawable acquired for the next presentation, held between acquisition and submission.
		pub pending_drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
		/// The `presentedTime` of the last drawable shown on screen, as `f64` bits. Zero means nothing was presented yet.
		/// Metal publishes it through a presented handler on an arbitrary thread shortly after the display shows the
		/// frame, so the value is shared atomically and may lag one acquisition behind.
		pub last_presented_time: Arc<AtomicU64>,
		/// The minimum time between presented frames; `None` presents on the next refresh.
		pub present_interval: Option<std::time::Duration>,
	}
}
