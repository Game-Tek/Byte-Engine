use ::utils::Extent;
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
		BoundTilePipelineMode, CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode,
		RasterizationRenderPassMode,
	},
	descriptors::DescriptorSetHandle,
};

/// The buffer index of a stage's first descriptor set. Each further set the stage uses takes the next index, as
/// the MSL generator in resource-management declares them.
pub(super) const ARGUMENT_BUFFER_BINDING_BASE: u32 = 16;
pub(super) const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;
/// The buffer index of the visible function table a ray generation kernel calls closest hits through: the records of
/// the shader binding table its trace names. Only compute encoders bind it, so it doesn't collide with vertex buffers.
pub(super) const SHADER_BINDING_TABLE_BINDING_INDEX: u32 = 14;
/// The buffer index of the extent a tile function's outputs have, which the MSL generator in resource-management
/// declares. Only the tile stage's table binds it, so it doesn't collide with vertex buffers.
pub(super) const TILE_EXTENT_BINDING_INDEX: u32 = 13;
/// Every buffer index Metal offers: vertex buffers, the push constants, and up to fifteen descriptor sets.
const ARGUMENT_TABLE_BUFFER_COUNT: usize = 31;
/// Upload ranges start at the texture copy alignment, so any range can be the source of a buffer-to-texture copy.
pub(super) const UPLOAD_ALIGNMENT: usize = crate::TEXTURE_COPY_PITCH_ALIGNMENT;
const UPLOAD_PAGE_SIZE: usize = 256 * 1024;

/// The `AppliedDescriptorBinding` struct records which descriptor sets the active native encoder's tables reference,
/// and the resource accesses of the pipeline that reads them, one table per set.
struct AppliedDescriptorBinding {
	key: DescriptorBindingKey,
	/// The table of each set the pipeline declares, as an index into [`EncoderState::tables`].
	tables: SmallVec<[u32; 4]>,
	/// One table of every set's uses, present when two of the sets reach the same memory and one writes it. Such
	/// sets cannot settle on their own, so the command plans them as one table.
	merged: Option<synchronization::TrackedTable>,
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
	/// Records dispatches and copies until another kind of command needs another encoder.
	Compute(Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>>),
	/// A render pass, started by `start_render_pass`, `start_tile_pass`, or an image clear. Only ending the pass ends
	/// it, so a dispatch or copy recorded inside it panics instead of silently ending it.
	Render(RenderPass),
}

impl ActiveEncoder {
	/// Returns the protocol both encoder kinds share, which barriers, debug groups, and labels go through.
	fn common(&self) -> &ProtocolObject<dyn mtl::MTL4CommandEncoder> {
		match self {
			Self::Compute(encoder) => ProtocolObject::from_ref(&**encoder),
			Self::Render(pass) => ProtocolObject::from_ref(&*pass.encoder),
		}
	}
}

/// The `RenderPass` struct keeps what is local to one open render pass, so ending the pass drops all of it.
struct RenderPass {
	encoder: Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>,
	/// The pass's extent, which scissors are clamped to.
	extent: Extent,
	/// The attachments' first accesses, which the pass's first command consumes with its own uses, so one barrier
	/// orders both. A pass that ends with no command consumes them as it ends.
	attachment_uses: SmallVec<[synchronization::MetalResourceUse; 8]>,
	/// The attachment writes every command of the pass records, consolidated once as the pass begins.
	attachment_writes: SmallVec<[synchronization::MetalResourceUse; 8]>,
	/// Whether the bound vertex buffers changed since the pass's vertex argument table last held them.
	vertex_buffers_dirty: bool,
	/// How many leading vertex bindings the pass's argument table holds.
	encoded_vertex_buffer_count: usize,
}

impl RenderPass {
	/// Opens a pass over `extent` that records into `encoder`; [`CommandBufferRecording::begin_encoder`] marks the
	/// bound vertex buffers for it.
	fn new(encoder: Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>, extent: Extent) -> Self {
		Self {
			encoder,
			extent,
			attachment_uses: SmallVec::new(),
			attachment_writes: SmallVec::new(),
			vertex_buffers_dirty: false,
			encoded_vertex_buffer_count: 0,
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
	/// The descriptor sets this encoder's tables reference.
	descriptors: Option<AppliedDescriptorBinding>,
	/// Every descriptor table a command of this encoder applied, with whether its reads are still to be planned.
	/// Bindings index it, so a table keeps that state while the pipeline changes around the set it came from.
	tables: SmallVec<[synchronization::TrackedTable; 8]>,
	/// The stages whose shared argument table this encoder already holds, one bit per `ArgumentTableStage as usize`.
	bound_argument_tables: u8,
	push_constants_dirty: bool,
	/// The counter slots of the timestamps this encoder wrote since its last command, in order.
	///
	/// Metal drops a precise timestamp that no command in the encoder follows, even when another timestamp does, so
	/// ending the encoder writes every one of these slots again from the command buffer, at the encoder boundary,
	/// where they belong. Counters that end together, such as a stage's and its scene's, leave several.
	tail_timestamps: SmallVec<[u32; 4]>,
	/// Whether this encoder recorded bottom-level builds or compactions that no timestamp has waited for yet.
	///
	/// A compute encoder's timestamp doesn't wait for the acceleration structure stage, so the next timestamp ends the
	/// encoder and is written between encoders, where it waits for everything. Otherwise a counter around the builds
	/// would read nothing and a later counter would get their time. Top-level builds don't set it: each frame records
	/// one, and splitting the encoder for it would cost more than the build.
	unwaited_bottom_level_work: bool,
	/// How many logical debug regions this encoder mirrors, which it pops before it ends.
	debug_region_depth: usize,
}

/// Creates a view of mip `mip_level` of `layers` of `texture`, for attachments and descriptors that select a
/// subresource: a 2D view of one layer, or with `array`, a 2D array view, as layered rendering into part of an array
/// image and a `StorageImageArray` descriptor that binds one mip of every layer need.
pub(super) fn texture_view(
	texture: &ProtocolObject<dyn mtl::MTLTexture>,
	format: crate::Formats,
	mip_level: u32,
	layers: std::ops::Range<u32>,
	array: bool,
) -> Retained<ProtocolObject<dyn mtl::MTLTexture>> {
	let texture_type = if array {
		mtl::MTLTextureType::Type2DArray
	} else {
		mtl::MTLTextureType::Type2D
	};
	// SAFETY: Callers validate the mip level and layer range against the image before recording the view.
	let view = unsafe {
		texture.newTextureViewWithPixelFormat_textureType_levels_slices(
			utils::to_pixel_format(format),
			texture_type,
			NSRange::new(mip_level as usize, 1),
			NSRange::new(layers.start as usize, layers.len()),
		)
	}
	.expect(
		"Metal texture view creation failed. The most likely cause is that the selected mip level or array layers do not exist in the image.",
	);
	// Images are labeled only when debug labels are enabled, so views follow the same setting.
	if let Some(label) = texture.label() {
		view.setLabel(Some(&NSString::from_str(&format!(
			"{label} (mip {mip_level}, layers {layers:?})"
		))));
	}
	view
}

/// Attaches `texture` to the next attachment of a render pass `descriptor`: the depth attachment for a depth format,
/// and otherwise color attachment `*color_index`, which it advances. `clear` is the value a clearing `load` writes.
pub(super) fn attach(
	descriptor: &mtl::MTL4RenderPassDescriptor,
	color_index: &mut usize,
	texture: &ProtocolObject<dyn mtl::MTLTexture>,
	format: crate::Formats,
	clear: crate::ClearValue,
	load: mtl::MTLLoadAction,
	store: mtl::MTLStoreAction,
) {
	let attachment: Retained<mtl::MTLRenderPassAttachmentDescriptor> = if format.is_depth() {
		let depth = descriptor.depthAttachment();
		depth.setClearDepth(utils::clear_depth(clear));
		Retained::into_super(depth)
	} else {
		// SAFETY: `color_index` counts only color attachments, which callers keep within Metal's eight.
		let color = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(*color_index) };
		*color_index += 1;
		color.setClearColor(utils::clear_color(clear));
		Retained::into_super(color)
	};
	attachment.setTexture(Some(texture));
	attachment.setLoadAction(load);
	attachment.setStoreAction(store);
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
	pub(super) meshes: &'a [Mesh],
	pub(super) pipelines: &'a [Pipeline],
	pub(super) shader_binding_tables: &'a [ShaderBindingTable],
	pub(super) swapchains: &'a crate::swapchain::Swapchains<Swapchain>,
	/// The number of frames in flight, which resolves frame offsets into per-frame resource copies.
	pub(super) frames: u8,
	pub(super) debug_labels: bool,
	/// The context's timestamp heaps, one per frame sequence, which counter starts and ends write into.
	pub(super) counter_heaps: &'a [Retained<ProtocolObject<dyn mtl::MTL4CounterHeap>>],
	/// The context's temporal upscalers, indexed by their handles.
	pub(super) upscalers: &'a [super::upscaler::Upscaler],
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
	/// Build scratch owned by this recording's frame, or the transient arena for detached recordings.
	pub(super) scratch_arena: &'a mut acceleration_structures::ScratchArena,
	/// Where a detached recording's executed bottom-level builds wait to compact.
	pub(super) pending_compactions: &'a mut Vec<acceleration_structures::PendingCompaction>,
	/// Mutable so recorded builds can move bottom-level structures through their build states.
	pub(super) acceleration_structures: &'a mut acceleration_structures::AccelerationStructures,
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
#[derive(Clone, Copy, Debug)]
pub(super) enum ArgumentTableStage {
	Compute,
	Vertex,
	Fragment,
	Object,
	Mesh,
	Tile,
}

impl ArgumentTableStage {
	fn render_stage(self) -> mtl::MTLRenderStages {
		match self {
			Self::Vertex => mtl::MTLRenderStages::Vertex,
			Self::Fragment => mtl::MTLRenderStages::Fragment,
			Self::Object => mtl::MTLRenderStages::Object,
			Self::Mesh => mtl::MTLRenderStages::Mesh,
			Self::Tile => mtl::MTLRenderStages::Tile,
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
pub(crate) type CommandArgumentTables = [Option<Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>>; 6];

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
	debug_regions: Vec<Retained<NSString>, &'a dyn std::alloc::Allocator>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_roots: SmallVec<[graphics_hardware_interface::DescriptorSetHandle; 4]>,
	/// The frame-local sets the roots resolve to, each with the version the next command will read.
	bound_descriptor_sets: SmallVec<[(DescriptorSetHandle, u64); 4]>,
	bound_vertex_buffers: SmallVec<[(graphics_hardware_interface::BaseBufferHandle, usize); 8]>,
	bound_index_buffer: Option<(graphics_hardware_interface::BaseBufferHandle, usize, crate::DataTypes)>,
	push_constant_data: Vec<u8, &'a dyn std::alloc::Allocator>,
	encoder: Option<EncoderState>,
	next_encoder_id: u32,
	resource_tracker: synchronization::MetalResourceTracker,
	/// The uses of the command being prepared, besides those of its descriptor tables. Commands fill it in place and
	/// preparing the command empties it, so no draw or dispatch moves its uses.
	command_uses: SmallVec<[synchronization::MetalResourceUse; 8]>,
	/// Readbacks recorded but not yet handed to submission. Dropping the recording abandons whatever is left.
	texture_readbacks: SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
	/// Acceleration-structure builds recorded but not yet handed to submission. Dropping the recording abandons them.
	builds: acceleration_structures::RecordedBuilds,
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
		std::mem::take(&mut self.builds).abandon(self.commit.acceleration_structures);
	}
}

/// The `FinishedCommandBuffer` struct carries one ended recording to the frame batch that submits it.
pub struct FinishedCommandBuffer {
	/// The queue the recording was made for, which must be the queue that submits it.
	pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	pub(crate) command_buffer: queue::NativeCommand,
	pub(crate) texture_readbacks: SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
	/// The acceleration-structure builds the recording holds, which submitting it executes.
	pub(crate) builds: acceleration_structures::RecordedBuilds,
}

mod encoding;
mod operations;
mod recording;
