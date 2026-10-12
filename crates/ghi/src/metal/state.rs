use std::sync::Arc;

use objc2_metal::{MTLSamplerState as _, MTLTexture as _};

use super::*;
use crate::metal::queue::{StoredQueue, SubmittedBatch};
use crate::{DeviceAccesses, Formats, ResourceCollection, Uses};

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
/// [`Context::export_image`] moves one out of a context as a [`DetachedImage`], and [`Frame::intern_image`] hands it
/// to another context of the same device.
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
/// Recording uses it to track the member by the bytes it occupies, so hazards between members that share memory
/// are ordered like hazards on one buffer.
#[derive(Clone)]
pub(crate) struct GroupSlot {
	/// Identifies the heap for hazard tracking. It is never reused, so a replaced heap's history cannot leak into
	/// its replacement.
	pub(crate) heap_serial: u64,
	pub(crate) offset: usize,
	pub(crate) size: usize,
}

// SAFETY: The image owns a retained Metal texture, which Metal documents as usable from any thread, and plain
// metadata. Moving it between threads adds no shared state.
unsafe impl Send for Image {}

/// The allocations one descriptor set element keeps resident, by object address: its buffer, texture or
/// acceleration structure, and the view when it binds one mip of a texture. Zero is no allocation.
pub(crate) type ElementPins = [usize; 2];

/// The `DescriptorSet` struct holds one frame sequence's copy of a descriptor set: the resources written to it and
/// the argument buffer every pipeline that declares its layout reads them from.
///
/// A write stores the resource, patches its entry in `words`, and pins what the entry names on every queue, see
/// [`queue::QueueResidency::pin`]. The set therefore keeps its own resources alive and resident, and a command that
/// binds it only has to keep the argument buffer: binding a set costs the same whatever its size.
pub(crate) struct DescriptorSet {
	pub(crate) layout: crate::shader::DescriptorSetLayout,
	/// Advances with every write that changes the set, and whenever a resource it holds is replaced.
	pub(crate) version: u64,
	/// The resource of each array element, in layout order; see `DescriptorSetLayout::element_base`.
	pub(crate) descriptors: Vec<Option<Descriptor>>,
	/// The first argument ID of each layout resource; see [`argument_bases`].
	pub(crate) argument_bases: Box<[u32]>,
	/// The frame sequence this copy belongs to. Its swapchain entries name the images that sequence renders into.
	sequence: u8,
	/// The argument buffer contents: one GPU address or resource ID per argument ID. Entries without a resource are
	/// zero.
	pub(crate) words: Vec<u64>,
	/// What each array element pins, in the order of `descriptors`.
	pins: Vec<ElementPins>,
	/// The single-mip texture views the set binds, by array element. Argument buffers do not retain them.
	texture_views: SmallVec<[(u32, Retained<ProtocolObject<dyn mtl::MTLTexture>>); 2]>,
	/// The buffer that holds `words` as they were at the version it records. Commands that read it share it, and
	/// the set writes over it only while none does.
	pub(crate) argument_buffer: Option<std::sync::Arc<ArgumentBufferSnapshot>>,
	/// The recording that last retained `argument_buffer`, by queue and [`queue::NativeCommand::serial`], so binding
	/// the set again in that recording retains nothing.
	pub(crate) argument_buffer_reader: Option<(graphics_hardware_interface::QueueHandle, u64)>,
	/// The set's resource uses, one entry for each way a pipeline uses the set.
	pub(crate) uses: Vec<SetUses>,
}

impl DescriptorSet {
	/// Creates the empty copy of a set for `layout` that the frame sequence `sequence` binds.
	pub(crate) fn new(layout: &crate::shader::DescriptorSetLayout, sequence: u8) -> Self {
		let (argument_bases, argument_count) = argument_bases(layout);
		Self {
			layout: layout.clone(),
			version: 0,
			descriptors: vec![None; layout.element_count() as usize],
			argument_bases,
			sequence,
			words: vec![0; argument_count as usize],
			pins: vec![ElementPins::default(); layout.element_count() as usize],
			texture_views: SmallVec::new(),
			argument_buffer: None,
			argument_buffer_reader: None,
			uses: Vec::new(),
		}
	}

	/// Writes the argument-buffer entries of one array element from the resource the element holds, and makes that
	/// resource resident on `queues` in place of the one the entries named before.
	///
	/// Call it after the element's resource changes, or after the native object behind that resource is replaced.
	/// The caller advances [`Self::version`].
	pub(crate) fn encode_element(
		&mut self,
		resource_index: usize,
		array_element: u32,
		resources: ArgumentResources<'_>,
		queues: &mut [queue::StoredQueue],
	) {
		/// Names a Metal object as the allocation residency tracks.
		fn allocation<T: objc2::Message + 'static>(object: &T) -> &ProtocolObject<dyn mtl::MTLAllocation>
		where
			dyn mtl::MTLAllocation: objc2::runtime::ImplementedBy<T>,
		{
			ProtocolObject::from_ref(object)
		}

		let count = self.layout.resources()[resource_index].count();
		let element = self.layout.element_base(resource_index) + array_element;
		let word = (self.argument_bases[resource_index] + array_element) as usize;

		// A view belongs to the resource the element held before.
		self.texture_views.retain(|(viewed, _)| *viewed != element);

		let mut pins = ElementPins::default();
		let mut pinned = 0;
		let mut pin = |allocation: &ProtocolObject<dyn mtl::MTLAllocation>| {
			for queue in queues.iter_mut() {
				queue.residency.pin(allocation);
			}
			pins[pinned] = std::ptr::from_ref(allocation).cast::<()>() as usize;
			pinned += 1;
		};
		match self.descriptors[element as usize] {
			None => self.words[word] = 0,
			Some(Descriptor::Buffer { buffer, .. }) => {
				let buffer = resources.buffers.resource(buffer);
				self.words[word] = buffer.gpu_address;
				pin(allocation(&*buffer.buffer));
			}
			Some(Descriptor::Image { image, mip_level, .. }) => {
				let image = resources.images.resource(image);
				pin(allocation(&*image.texture));
				self.words[word] = match mip_level {
					Some(mip_level) => {
						let view = command_buffer::texture_view_2d(&image.texture, image.description.format, mip_level, 0);
						let id = resource_id(view.gpuResourceID());
						pin(allocation(&*view));
						self.texture_views.push((element, view));
						id
					}
					None => resource_id(image.texture.gpuResourceID()),
				};
			}
			Some(Descriptor::CombinedImageSampler { image, sampler, .. }) => {
				let image = resources.images.resource(image);
				pin(allocation(&*image.texture));
				self.words[word] = resource_id(image.texture.gpuResourceID());
				// A combined image sampler's samplers follow its textures.
				self.words[word + count as usize] = resource_id(resources.samplers[sampler.0 as usize].gpuResourceID());
			}
			Some(Descriptor::Sampler { sampler }) => {
				self.words[word] = resource_id(resources.samplers[sampler.0 as usize].gpuResourceID());
			}
			Some(Descriptor::AccelerationStructure { handle }) => {
				let structure = &resources.acceleration_structures[handle.0 as usize].structure;
				self.words[word] = resource_id(structure.gpuResourceID());
				pin(allocation(&**structure));
			}
			// Each frame sequence renders into its own swapchain image. A sequence has none until it acquires the
			// swapchain, which encodes the entry again.
			Some(Descriptor::Swapchain { handle }) => {
				self.words[word] = match resources.swapchains[handle.0 as usize].images[self.sequence as usize] {
					Some(image) => {
						let image = resources.images.resource(image);
						pin(allocation(&*image.texture));
						resource_id(image.texture.gpuResourceID())
					}
					None => 0,
				};
			}
		}

		// The new allocations were pinned first, so one the element keeps never stops being held in between.
		for address in std::mem::replace(&mut self.pins[element as usize], pins) {
			if address != 0 {
				for queue in queues.iter_mut() {
					queue.residency.unpin(address);
				}
			}
		}
	}

	/// Returns the resources written for the layout resource at `resource_index`, one per array element.
	pub(crate) fn resource_descriptors(&self, resource_index: usize) -> &[Option<Descriptor>] {
		let resource = self.layout.resources()[resource_index];
		let base = self.layout.element_base(resource_index) as usize;
		&self.descriptors[base..base + resource.count() as usize]
	}
}

/// The `ArgumentResources` struct lends a descriptor set the backend resources its argument-buffer entries name.
#[derive(Clone, Copy)]
pub(crate) struct ArgumentResources<'a> {
	pub(crate) buffers: &'a ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(crate) images: &'a ResourceCollection<Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(crate) samplers: &'a [Retained<ProtocolObject<dyn mtl::MTLSamplerState>>],
	pub(crate) acceleration_structures: &'a [AccelerationStructure],
	pub(crate) swapchains: &'a [Swapchain],
}

/// Returns the 64-bit value Metal reads a texture, sampler, or acceleration structure from in an argument buffer.
fn resource_id(id: mtl::MTLResourceID) -> u64 {
	// SAFETY: `MTLResourceID` is a C struct with one 64-bit member, the value argument buffers store.
	unsafe { std::mem::transmute::<mtl::MTLResourceID, u64>(id) }
}

/// The `ArgumentBufferSnapshot` struct is one encoding of a descriptor set, which the commands that hold it keep
/// reading after the set changes.
pub(crate) struct ArgumentBufferSnapshot {
	/// The set version the buffer was encoded from.
	pub(crate) version: u64,
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) address: mtl::MTLGPUAddress,
}

/// The `Synchronizer` struct owns the Metal workloads associated with one GHI synchronization point.
///
/// A synchronizer with no pending workloads is signaled.
#[derive(Default)]
pub(crate) struct Synchronizer {
	workloads: SmallVec<[SubmittedBatch; 4]>,
}

impl Synchronizer {
	pub(crate) fn signal(&mut self, workload: SubmittedBatch) {
		self.workloads.push(workload);
	}

	/// Finishes every batch Metal already completed without blocking, and returns whether no batch is still running.
	///
	/// Panics with the first GPU error among the finished batches, after finishing all of them.
	pub(crate) fn poll(&mut self, queues: &mut [StoredQueue]) -> bool {
		let mut first_error = None;
		for workload in std::mem::take(&mut self.workloads) {
			// `Option::or` evaluates its argument first, so every finished batch is still finished.
			match workload.try_feedback() {
				Some(feedback) => first_error = first_error.or(workload.finish(feedback, queues)),
				None => self.workloads.push(workload),
			}
		}
		if let Some(error) = first_error {
			panic!("{error}");
		}
		self.workloads.is_empty()
	}

	/// Waits for every submitted batch, returns its commands to `queues` for reuse, and reports the first GPU error.
	pub(crate) fn wait(&mut self, queues: &mut [StoredQueue]) -> Option<String> {
		let mut first_error = None;
		for workload in self.workloads.drain(..) {
			first_error = first_error.or(workload.wait(queues));
		}
		first_error
	}
}

#[derive(Clone)]
pub(crate) struct Swapchain {
	pub layer: Retained<CAMetalLayer>,
	pub view: Retained<NSView>,
	/// The image each frame sequence renders into, which presentation copies to a drawable. A sequence gets its
	/// image the first time it acquires the swapchain at a nonzero extent.
	pub images: [Option<ImageHandle>; MAX_FRAMES_IN_FLIGHT],
	pub uses: crate::Uses,
	pub extent: Extent,
	/// The `presentedTime` of the last drawable shown on screen, as `f64` bits. Zero means nothing was presented yet.
	/// Metal publishes it through a presented handler on an arbitrary thread shortly after the display shows the
	/// frame, so the value is shared atomically and may lag one acquisition behind.
	pub last_presented_time: Arc<AtomicU64>,
	/// The `last_presented_time` bits last converted to an `Instant`, with the result. Converting reads two clocks,
	/// so converting an unchanged presented time again would land on a slightly different instant.
	pub presented_instant: Option<(u64, std::time::Instant)>,
	/// The minimum time between presented frames; `None` presents on the next refresh.
	pub present_interval: Option<std::time::Duration>,
}
