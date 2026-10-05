use ::utils::{Extent, hash::HashMap};
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSAutoreleasePool, NSRange, NSString};
use objc2_metal::{
	MTL4ArgumentTable, MTL4CommandEncoder, MTL4ComputeCommandEncoder, MTL4RenderCommandEncoder, MTLBuffer, MTLDevice,
	MTLTexture,
};
use smallvec::SmallVec;

use super::*;
use crate::{
	ImageOrSwapchain, ResourceCollection,
	command_buffer::{
		BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
		CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode, RasterizationRenderPassMode,
	},
	descriptors::DescriptorSetHandle,
};

const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
pub(super) const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;
const ARGUMENT_TABLE_BUFFER_COUNT: usize = 17;
/// Upload ranges start at the texture copy alignment, so any range can be the source of a buffer-to-texture copy.
pub(super) const UPLOAD_ALIGNMENT: usize = crate::TEXTURE_COPY_PITCH_ALIGNMENT;
const UPLOAD_PAGE_SIZE: usize = 256 * 1024;

/// The `AppliedDescriptorBinding` struct records which argument-buffer snapshot the active native encoder references.
struct AppliedDescriptorBinding {
	key: DescriptorBindingKey,
	snapshot: AppliedSnapshot,
	settled: synchronization::SettledDescriptors,
}

/// The `AppliedSnapshot` enum locates the resource uses of an applied snapshot without copying them.
enum AppliedSnapshot {
	/// A snapshot that descriptor set `owner` retains at `index` of its snapshot list. The list only grows or
	/// replaces entries in place, so the location stays valid for the whole recording.
	Retained { owner: DescriptorSetHandle, index: usize },
	/// A snapshot encoded for this command alone, which owns its uses.
	Transient(synchronization::DescriptorUses),
}

impl AppliedSnapshot {
	/// Returns the snapshot's resource uses.
	fn uses<'s>(&'s self, descriptor_sets: &'s context::DescriptorSets) -> &'s synchronization::DescriptorUses {
		match self {
			Self::Retained { owner, index } => &descriptor_sets.resource(*owner).argument_buffers[*index].resource_uses,
			Self::Transient(uses) => uses,
		}
	}
}

/// The panic message for a surface a command needs but cannot resolve.
pub(super) const MISSING_SURFACE: &str = "Missing Metal surface. The most likely cause is that an image handle came from another context, or that a swapchain was used before this frame acquired it at a nonzero extent.";

/// The `Surface` struct is the texture a command reaches through an image or swapchain handle.
///
/// It is a frame image or a swapchain's image for the frame sequence; commands never reach a drawable directly.
/// Resolve one with [`CommandBufferRecording::surface`] and describe each access to its `image` with
/// [`MetalResourceUse::image`](synchronization::MetalResourceUse::image); consuming that use retains the texture.
pub(super) struct Surface {
	/// The image behind the texture.
	pub(super) image: ImageHandle,
	pub(super) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
	pub(super) format: crate::Formats,
	pub(super) extent: Extent,
	pub(super) array_layers: u32,
	/// The uses the texture was created with. A swapchain's surfaces report the swapchain's uses.
	pub(super) uses: crate::Uses,
}

/// The `ActiveEncoder` enum holds the one native encoder a recording writes into at a time.
enum ActiveEncoder {
	Compute(Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>>),
	Render(Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>),
}

impl ActiveEncoder {
	/// Returns the protocol both encoder kinds share, which barriers, debug groups, and labels go through.
	fn common(&self) -> &ProtocolObject<dyn mtl::MTL4CommandEncoder> {
		match self {
			Self::Compute(encoder) => ProtocolObject::from_ref(&**encoder),
			Self::Render(encoder) => ProtocolObject::from_ref(&**encoder),
		}
	}
}

/// The `EncoderState` struct keeps what is local to the active native encoder, so ending the encoder drops all of it.
///
/// [`CommandBufferRecording::begin_encoder`] is the only place that builds it.
struct EncoderState {
	encoder: ActiveEncoder,
	/// The identity hazard tracking gives this encoder.
	scope: synchronization::MetalEncoderScope,
	/// The pipeline whose native state this encoder has set.
	pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	/// The argument-buffer snapshot this encoder's tables reference.
	descriptors: Option<AppliedDescriptorBinding>,
	/// The stages whose shared argument table this encoder already holds, one bit per `ArgumentTableStage as usize`.
	bound_argument_tables: u8,
	push_constants_dirty: bool,
	/// The counter slot of the last timestamp this encoder wrote, while no command followed it.
	///
	/// Metal drops a precise timestamp that nothing in the encoder follows, so ending the encoder writes that slot
	/// again from the command buffer, at the encoder boundary, where it belongs.
	tail_timestamp: Option<u32>,
	/// How many logical debug regions this encoder mirrors, which it pops before it ends.
	#[cfg(debug_assertions)]
	debug_region_depth: usize,
}

/// Creates a 2D view of one mip level and array layer, for attachments and descriptors that select a subresource.
fn texture_view_2d(
	texture: &ProtocolObject<dyn mtl::MTLTexture>,
	format: crate::Formats,
	mip_level: u32,
	layer: u32,
) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
	// SAFETY: Callers validate the mip level and layer against the image before recording the view.
	let view = unsafe {
		texture.newTextureViewWithPixelFormat_textureType_levels_slices(
			utils::to_pixel_format(format),
			mtl::MTLTextureType::Type2D,
			NSRange::new(mip_level as usize, 1),
			NSRange::new(layer as usize, 1),
		)
	}
	.expect(
		"Metal texture view creation failed. The most likely cause is that the selected mip level or array layer does not exist in the image.",
	);
	// Images are labeled only when debug labels are enabled, so views follow the same setting.
	#[cfg(debug_assertions)]
	if let Some(label) = texture.label() {
		view.setLabel(Some(&NSString::from_str(&format!(
			"{label} (mip {mip_level}, layer {layer})"
		))));
	}
	view
}

#[cfg(test)]
mod tests {
	#[test]
	fn upload_ranges_are_aligned_and_do_not_overlap() {
		assert_eq!(super::upload_offset(0, 4, 1024), Some(0));
		assert_eq!(super::upload_offset(4, 4, 1024), Some(256));
		assert_eq!(super::upload_offset(260, 4, 1024), Some(512));
		assert_eq!(super::upload_offset(1020, 8, 1024), None);
	}
}

/// The `RecordingDevice` struct provides command recording with immutable access to backend resources.
pub(super) struct RecordingDevice<'a> {
	pub(super) metal_device: &'a ProtocolObject<dyn mtl::MTLDevice>,
	pub(super) buffers: &'a ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(super) images: &'a ResourceCollection<Image, graphics_hardware_interface::BaseImageHandle, ImageHandle>,
	pub(super) samplers: &'a [Retained<ProtocolObject<dyn mtl::MTLSamplerState>>],
	pub(super) acceleration_structures: &'a [AccelerationStructure],
	pub(super) meshes: &'a [Mesh],
	pub(super) pipelines: &'a [Pipeline],
	pub(super) swapchains: &'a [Swapchain],
	/// The number of frames in flight, which resolves frame offsets into per-frame resource copies.
	pub(super) frames: u8,
	pub(super) debug_labels: bool,
	/// The context's timestamp heaps, one per frame sequence, which counter starts and ends write into.
	pub(super) counter_heaps: &'a [Retained<ProtocolObject<dyn mtl::MTL4CounterHeap>>; MAX_FRAMES_IN_FLIGHT],
}

/// The `RecordingCommit` struct carries recording results back into the owning device after encoding ends.
pub(super) struct RecordingCommit<'a> {
	pub(super) queue_handle: graphics_hardware_interface::QueueHandle,
	pub(super) queue: &'a mut queue::StoredQueue,
	pub(super) synchronizers: &'a mut ResourceCollection<
		Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
	pub(super) texture_readbacks: &'a mut crate::context::TextureReadbackRegistry<context::TextureReadbackStorage>,
	/// Frame-local sets are mutable so a recording can retain the argument buffers it encodes from them.
	pub(super) descriptor_sets: &'a mut context::DescriptorSets,
	/// Upload pages owned by this recording's frame, or the transient arena for detached recordings.
	pub(super) upload_arena: &'a mut UploadArena,
	pub(super) argument_tables: &'a mut CommandArgumentTables,
	pub(super) image_groups: &'a mut crate::image_group::ImageGroups,
	/// Hands out the timestamp slots counters write; see [`crate::counters::Counters`].
	pub(super) counters: &'a mut crate::counters::Counters,
}

/// The `NativeCommandSlot` struct permits a recording guard to move its uniquely owned command into the next lifecycle stage.
struct NativeCommandSlot(Option<queue::NativeCommand>);

impl NativeCommandSlot {
	fn take(&mut self) -> queue::NativeCommand {
		self.0.take().expect(
			"Metal native command is missing. The most likely cause is that one recording was finalized more than once.",
		)
	}
}

impl std::ops::Deref for NativeCommandSlot {
	type Target = queue::NativeCommand;

	fn deref(&self) -> &Self::Target {
		self.0
			.as_ref()
			.expect("Metal native command is missing. The most likely cause is that recording continued after finalization.")
	}
}

impl std::ops::DerefMut for NativeCommandSlot {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.0
			.as_mut()
			.expect("Metal native command is missing. The most likely cause is that recording continued after finalization.")
	}
}

/// The `ArgumentTableStage` enum names the shader stage whose shared argument table a binding targets.
///
/// Its declaration order indexes [`CommandArgumentTables`], so `stage as usize` selects the stage's table.
#[derive(Clone, Copy)]
pub(super) enum ArgumentTableStage {
	Compute,
	Vertex,
	Fragment,
	Object,
	Mesh,
}

impl ArgumentTableStage {
	/// Names this stage's argument table in capture tools.
	#[cfg(debug_assertions)]
	fn label(self) -> &'static str {
		match self {
			Self::Compute => "Compute Argument Table",
			Self::Vertex => "Vertex Argument Table",
			Self::Fragment => "Fragment Argument Table",
			Self::Object => "Object Argument Table",
			Self::Mesh => "Mesh Argument Table",
		}
	}

	fn render_stage(self) -> mtl::MTLRenderStages {
		match self {
			Self::Vertex => mtl::MTLRenderStages::Vertex,
			Self::Fragment => mtl::MTLRenderStages::Fragment,
			Self::Object => mtl::MTLRenderStages::Object,
			Self::Mesh => mtl::MTLRenderStages::Mesh,
			Self::Compute => unreachable!(
				"Invalid Metal render argument-table stage. The most likely cause is that the compute table was bound to a render encoder."
			),
		}
	}
}

/// The `CommandArgumentTables` type keeps one mutable Metal 4 binding table per shader stage.
///
/// Draws and dispatches snapshot table contents when they are encoded, so one
/// set of tables serves every recording; each command retains the tables it
/// snapshots until completion.
pub(crate) type CommandArgumentTables = [Option<Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>>; 5];

/// The `UploadPage` struct keeps immutable upload snapshots in one shared Metal buffer.
///
/// [`UploadArena::allocate`] hands out ranges of it; callers write through `contents` and bind through `gpu_address`.
pub(crate) struct UploadPage {
	pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	/// The buffer's length, CPU mapping, and GPU address, which never change, read once so allocations ask Metal
	/// for none of them.
	capacity: usize,
	pub(crate) contents: *mut u8,
	pub(crate) gpu_address: mtl::MTLGPUAddress,
	cursor: usize,
	/// Sized for one oversized request; released at the next reset instead of being kept resident.
	dedicated: bool,
}

/// The `UploadArena` struct suballocates aligned, non-overlapping upload ranges from retained shared pages.
///
/// A frame owns one arena per sequence index and resets it once that sequence's
/// commands have completed, so pages are reused instead of reallocated. Every
/// range handed out is immutable until the reset, which keeps push-constant and
/// texture-upload snapshots valid for the commands that read them.
#[derive(Default)]
pub(crate) struct UploadArena {
	pages: Vec<UploadPage>,
	debug_labels: bool,
}

impl UploadArena {
	/// Creates an empty arena whose pages get capture labels when `debug_labels` is set.
	pub(crate) fn new(debug_labels: bool) -> Self {
		Self {
			pages: Vec::new(),
			debug_labels,
		}
	}

	/// Rewinds every resident page; the caller guarantees no in-flight command still reads them.
	pub(crate) fn reset(&mut self) {
		self.pages.retain(|page| !page.dedicated);
		for page in &mut self.pages {
			page.cursor = 0;
		}
	}

	/// Drops every page so the next allocation starts fresh; commands that retained old pages keep them alive.
	pub(crate) fn discard(&mut self) {
		self.pages.clear();
	}

	#[cfg(test)]
	pub(crate) fn page_count(&self) -> usize {
		self.pages.len()
	}

	/// Returns an aligned range of `size` bytes and the page that backs it.
	pub(crate) fn allocate(&mut self, device: &ProtocolObject<dyn mtl::MTLDevice>, size: usize) -> (&UploadPage, usize) {
		assert!(
			size > 0,
			"Empty Metal upload. The most likely cause is that a zero-sized upload was requested."
		);
		let page_index = self
			.pages
			.iter()
			.position(|page| !page.dedicated && upload_offset(page.cursor, size, page.capacity).is_some())
			.unwrap_or_else(|| {
				let dedicated = size > UPLOAD_PAGE_SIZE;
				let capacity = if dedicated {
					size.next_multiple_of(UPLOAD_ALIGNMENT)
				} else {
					UPLOAD_PAGE_SIZE
				};
				let buffer = device
					.newBufferWithLength_options(capacity, mtl::MTLResourceOptions::StorageModeShared)
					.expect(
						"Metal upload page allocation failed. The most likely cause is that the device is out of shared memory.",
					);
				#[cfg(debug_assertions)]
				if self.debug_labels {
					let label = if dedicated { "Dedicated Upload Page" } else { "Upload Page" };
					buffer.setLabel(Some(&NSString::from_str(label)));
				}
				self.pages.push(UploadPage {
					capacity: buffer.length(),
					contents: buffer.contents().as_ptr().cast(),
					gpu_address: buffer.gpuAddress(),
					buffer,
					cursor: 0,
					dedicated,
				});
				self.pages.len() - 1
			});
		let page = &mut self.pages[page_index];
		let offset = upload_offset(page.cursor, size, page.capacity).expect(
			"Metal upload range does not fit. The most likely cause is that the selected page is smaller than the request.",
		);
		page.cursor = offset + size;
		(page, offset)
	}

	/// Copies `bytes` into a fresh range and returns its page and offset.
	pub(crate) fn upload(&mut self, device: &ProtocolObject<dyn mtl::MTLDevice>, bytes: &[u8]) -> (&UploadPage, usize) {
		let (page, offset) = self.allocate(device, bytes.len());
		// SAFETY: `offset` was computed against this page's capacity and leaves `bytes.len()` writable bytes.
		let destination = unsafe { page.contents.add(offset) };
		// SAFETY: Caller bytes and the shared upload page do not overlap and no command reads this range yet.
		unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
		(page, offset)
	}
}

/// Returns the next aligned upload offset when the requested range fits in the page.
fn upload_offset(cursor: usize, size: usize, capacity: usize) -> Option<usize> {
	let aligned = cursor.checked_next_multiple_of(UPLOAD_ALIGNMENT)?;
	(aligned.checked_add(size)? <= capacity).then_some(aligned)
}

/// The `CommandBufferRecording` struct scopes Metal command encoding and its temporary host allocations.
pub struct CommandBufferRecording<'a> {
	device: RecordingDevice<'a>,
	commit: RecordingCommit<'a>,
	frame_key: Option<graphics_hardware_interface::FrameKey>,
	sequence_index: u8,
	command_buffer: NativeCommandSlot,
	#[cfg(debug_assertions)]
	debug_regions: Vec<Retained<NSString>, &'a dyn std::alloc::Allocator>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_roots: SmallVec<[graphics_hardware_interface::DescriptorSetHandle; 4]>,
	/// The frame-local sets the roots resolve to, each with the version the next command will read.
	bound_descriptor_sets: SmallVec<[(DescriptorSetHandle, u64); 4]>,
	bound_vertex_buffers: SmallVec<[(graphics_hardware_interface::BaseBufferHandle, usize); 8]>,
	render_vertex_buffers_dirty: bool,
	encoded_vertex_buffer_count: usize,
	bound_index_buffer: Option<(graphics_hardware_interface::BaseBufferHandle, usize, crate::DataTypes)>,
	push_constant_data: Vec<u8, &'a dyn std::alloc::Allocator>,
	encoder: Option<EncoderState>,
	/// Extent of the render pass being encoded; scissors are clamped to it.
	active_render_extent: Extent,
	next_encoder_id: u32,
	resource_tracker: synchronization::MetalResourceTracker,
	active_render_attachment_uses: SmallVec<[synchronization::MetalResourceUse; 8]>,
	/// Readbacks recorded but not yet handed to submission. Dropping the recording abandons whatever is left.
	texture_readbacks: SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
	_autorelease_pool: Option<Retained<NSAutoreleasePool>>,
}

impl Drop for CommandBufferRecording<'_> {
	fn drop(&mut self) {
		if self.resource_tracker.rollback_recording() {
			self.commit.queue.resource_tracker = std::mem::take(&mut self.resource_tracker);
		}
		for handle in self.texture_readbacks.drain(..) {
			// Dropping the returned storage releases the retained native staging buffer immediately.
			self.commit.texture_readbacks.abandon_recorded(handle);
		}
	}
}

/// The `FinishedCommandBuffer` struct carries one ended recording to the frame batch that submits it.
pub struct FinishedCommandBuffer {
	/// The queue the recording was made for, which must be the queue that submits it.
	pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	pub(crate) command_buffer: queue::NativeCommand,
	pub(crate) texture_readbacks: SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
}

mod encoding;
mod operations;
mod recording;
