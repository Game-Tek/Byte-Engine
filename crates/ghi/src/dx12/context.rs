/// The `Device` struct exists to own DX12 GPU resources for the shared GHI device API.
pub struct Device {
	// Initialization requests the interface needed by every modern resource path, so later operations do not branch on it.
	device: ID3D12Device10,
	// Independent devices serialize root signatures through their device-scoped configuration interface.
	device_configuration: ID3D12DeviceConfiguration,
	// Reuse one validated compiler and cache identity for every runtime HLSL compilation.
	dxc_compiler: DxcCompiler,
	// DirectStorage is optional, so a missing runtime is kept as the load error and reported when a resource I/O queue is created.
	direct_storage_runtime: Result<crate::dx12::io::DirectStorageRuntime, String>,
	// Descriptor strides are immutable for the lifetime of an ID3D12Device, so query them once.
	descriptor_handle_increment_sizes: [u32; 4],
	// Native format support is immutable for a device, so creation queries every DXGI format GHI maps to once and
	// image validation reads the table. Sorted by format, for a binary search.
	format_support: Box<[windows::Win32::Graphics::Direct3D12::D3D12_FEATURE_DATA_FORMAT_SUPPORT]>,
	settings: Features,
	info_queue: Option<ID3D12InfoQueue>,
	debug_log_function: fn(&str),
	debug_log_count: AtomicU64,
	pub(crate) frames: u8,

	queues: Vec<StoredQueue>,
	command_buffers: Vec<CommandBuffer>,
	active_command_buffer: Option<CommandBufferHandle>,
	/// The command buffer each queue records workloads into; see [`Device::record_workload`].
	workload_command_buffers: SmallVec<[(QueueHandle, CommandBufferHandle); 2]>,
	/// The synchronizer every workload signals and waits for, created by the first workload.
	workload_synchronizer: Option<SynchronizerHandle>,
	deferred_tasks: [Vec<DeferredTask>; crate::MAX_FRAMES_IN_FLIGHT],
	buffers: Vec<Buffer>,
	images: Vec<Image>,
	samplers: Vec<Sampler>,
	descriptor_sets: Vec<DescriptorSet>,
	/// The resource and sampler heaps shaders index directly. Every command list binds them once, when it opens.
	shader_visible_heaps: ShaderVisibleHeaps,
	pipeline_layouts: Vec<NativePipelineLayout>,
	/// Root signatures by shape, so pipelines whose stages read the same number of sets share one.
	root_signatures: HashMap<RootSignatureKey, ID3D12RootSignature>,
	pub(crate) pipelines: Vec<Pipeline>,
	indirect_dispatch_signature: Option<ID3D12CommandSignature>,
	indirect_draw_signature: Option<ID3D12CommandSignature>,
	indirect_mesh_dispatch_signature: Option<ID3D12CommandSignature>,
	shaders: Vec<Shader>,
	meshes: Vec<Mesh>,
	pub(crate) swapchains: crate::swapchain::Swapchains<Swapchain>,
	synchronizers: Vec<Synchronizer>,
	synchronizer_masters: Vec<SynchronizerHandle>,
	last_frame_synchronizers: [Option<SynchronizerHandle>; crate::MAX_FRAMES_IN_FLIGHT],
	texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadback>,
	counters: crate::counters::Counters,
	/// The timestamp query heap and the mapped readback buffer it resolves into, created by the first counter write.
	/// Stays `None` when counters are disabled, because no counter writes.
	counter_storage: Option<QueryStorage>,
	/// The pipeline statistics query heap and its mapped readback buffer, created by the first statistics counter.
	statistics_storage: Option<QueryStorage>,
	/// The statistics counters a recording opened and hasn't ended, with the command buffer each began in, since a
	/// statistics query must begin and end in one command list.
	open_statistics: SmallVec<[(crate::CounterHandle, CommandBufferHandle); 4]>,
	/// The timestamp frequency of the queue each frame sequence last recorded counters on.
	counter_frequencies: [u64; crate::MAX_FRAMES_IN_FLIGHT],
	untracked_present_work: bool,
	/// The RTVs or DSVs of each attachment image, by native resource and format; see
	/// [`Device::materialize_attachment_views`].
	attachment_views: HashMap<AttachmentViewKey, DescriptorHeap>,
	retained_clear_uav_descriptors: HashMap<usize, RetainedCpuDescriptor>,
	clear_uav_descriptor_pages: Vec<DescriptorHeapArena>,
	free_clear_uav_descriptor_slots: Vec<(usize, u32)>,
	buffer_states: HashMap<usize, BufferBarrierState>,
	image_states: HashMap<usize, TextureBarrierState>,
	pub(crate) image_groups: crate::image_group::ImageGroups,
	/// The image-group members whose memory an image reused when it got new contents, by the image's native resource
	/// key. The image's next transition deactivates each of them, flushing their writes, before it activates the image.
	image_deactivations: HashMap<usize, SmallVec<[ID3D12Resource; 4]>>,
	/// The native resource keys transitioned so far in the open unordered group of
	/// [`crate::command_buffer::CommandBufferRecording::unordered`], while one is open. A resource's later
	/// unordered-access uses in the group get no UAV barrier, because the group's caller promised its commands touch
	/// disjoint memory.
	pub(super) unordered_group: Option<SmallVec<[usize; 16]>>,
	/// Whether the unordered accesses of each resource since its last barrier in the open recording include a write,
	/// by native resource key. Two reads in a row need no UAV barrier between them, so a use only waits on a write.
	/// A resource the recording has not touched is missing, and counts as written, since an earlier recording may
	/// have left a write unordered. Cleared when a recording starts.
	pub(super) unordered_writes: HashMap<usize, bool>,
	/// The heaps backing each image group's members, indexed by group.
	image_group_heaps: Vec<SmallVec<[ID3D12Heap; 2]>>,
	/// Whether the device was created with the ray tracing feature on a GPU with DXR tier 1.1.
	ray_tracing_supported: bool,
	acceleration_structures: crate::dx12::acceleration_structures::AccelerationStructures,
	shader_binding_tables: Vec<crate::dx12::ray_tracing::ShaderBindingTable>,
}

impl Drop for Device {
	fn drop(&mut self) {
		// Device teardown owns every remaining recording, so abandon speculative state before enforcing the queue-idle precondition.
		for index in (0..self.command_buffers.len()).rev() {
			let command_buffer_handle = CommandBufferHandle::from_index(index as u64);
			let frame_count = self.command_buffers[index].frames.len();
			for sequence_index in (0..frame_count).rev() {
				self.command_buffers[index].activate_sequence(sequence_index as u8);
				self.discard_command_buffer_recording(command_buffer_handle);
			}
		}
		if self.wait_for_all_queues_idle().is_err() {
			(self.debug_log_function)(
				"Failed to prove that the DX12 queues stopped during device destruction. The most likely cause is that the device was removed or rejected the terminal fence signal.",
			);
			self.retain_gpu_state_after_failed_shutdown();
		}
	}
}

impl Device {
	/// Retains every object that queued GPU work may still reference when shutdown cannot prove completion.
	fn retain_gpu_state_after_failed_shutdown(&mut self) {
		// Child objects do not make the application's resource lifetime implicit. Keep one device reference and
		// every potentially referenced allocation alive until process teardown instead of risking a GPU use-after-free.
		std::mem::forget(self.device.clone());

		macro_rules! retain_field {
			($field:ident) => {
				std::mem::forget(std::mem::take(&mut self.$field));
			};
		}

		retain_field!(queues);
		retain_field!(command_buffers);
		retain_field!(deferred_tasks);
		retain_field!(buffers);
		retain_field!(images);
		retain_field!(samplers);
		retain_field!(descriptor_sets);
		retain_field!(pipeline_layouts);
		retain_field!(root_signatures);
		retain_field!(pipelines);
		retain_field!(indirect_dispatch_signature);
		retain_field!(indirect_draw_signature);
		retain_field!(indirect_mesh_dispatch_signature);
		retain_field!(shaders);
		retain_field!(meshes);
		retain_field!(swapchains);
		retain_field!(synchronizers);
		retain_field!(attachment_views);
		retain_field!(retained_clear_uav_descriptors);
		retain_field!(clear_uav_descriptor_pages);
		retain_field!(counter_storage);
		retain_field!(statistics_storage);
		// The heaps have no default, and dropping them would free descriptors queued work still reads.
		std::mem::forget(self.shader_visible_heaps.resources.native.clone());
		std::mem::forget(self.shader_visible_heaps.samplers.native.clone());

		let readbacks = std::mem::replace(&mut self.texture_readbacks, crate::context::TextureReadbackRegistry::new());
		std::mem::forget(readbacks);
	}
}

mod acceleration_structures;
mod commands;
mod counters;
use counters::QueryStorage;
mod descriptors;
mod dred;
pub(crate) use dred::device_removed_report;
mod initialization;
// Named apart from the `crate::pipelines` import that this module and its children use.
#[path = "context/pipelines.rs"]
mod device_pipelines;
mod presentation;
mod ray_tracing;
mod resources;
mod transfers;

/// The `StoredQueue` struct preserves a native queue and the workloads requested for its GHI handle.
#[derive(Clone)]
pub(crate) struct StoredQueue {
	queue: ID3D12CommandQueue,
	workloads: WorkloadTypes,
	/// GPU timestamp ticks per second on this queue, which converts counter slots into durations.
	timestamp_frequency: u64,
}

/// Validates that a queue workload can use the backend's unified DIRECT command-list path, which serves raster,
/// ray-tracing, compute, and transfer work. Enhanced layouts persist across submissions, and DIRECT lists can legally
/// consume layouts left by any of them.
pub(crate) fn validate_d3d12_workloads(requested: WorkloadTypes) -> Result<(), &'static str> {
	if requested.is_empty() {
		Err("Invalid DX12 workload selection. The most likely cause is that the queue was created without a workload.")
	} else if requested.intersects(WorkloadTypes::VIDEO) {
		Err(
			"DX12 video workloads are unavailable. The most likely cause is that this backend's command-buffer path does not expose video queues.",
		)
	} else if requested.intersects(WorkloadTypes::IO) {
		Err(
			"DX12 IO workloads are unavailable. The most likely cause is that this backend's command-buffer path does not expose IO queues.",
		)
	} else {
		Ok(())
	}
}

/// The `CommandBufferFrame` struct owns native recording state that can be reused after one frame sequence completes.
struct CommandBufferFrame {
	lifecycle: CommandBufferLifecycle,
	allocator: ID3D12CommandAllocator,
	command_list: ID3D12GraphicsCommandList7,
	/// The attachment view heaps the recording uses, kept until its submission completes.
	retained_descriptor_heaps: Vec<ID3D12DescriptorHeap>,
	retained_resources: Vec<ID3D12Resource>,
	retained_resource_keys: HashSet<usize>,
	descriptor_sync_scratch: SmallVec<[(ShaderResourceDescriptor, RetainedDescriptor); 32]>,
	present_resources: SmallVec<[ID3D12Resource; 2]>,
	recorded_readbacks: SmallVec<[TextureCopyHandle; 4]>,
	/// What the recording did to acceleration structures, which the context settles when the recording executes.
	recorded_builds: crate::dx12::acceleration_structures::RecordedBuilds,
	/// The vertex buffer views the recording bound, kept so binding a raster pipeline afterwards can give them its
	/// strides.
	vertex_buffer_views: SmallVec<[D3D12_VERTEX_BUFFER_VIEW; 4]>,
	/// The vertex strides of the raster pipeline the recording bound last, by input slot.
	vertex_strides: SmallVec<[u32; 4]>,
	original_buffer_states: SmallVec<[(usize, Option<BufferBarrierState>); 16]>,
	original_image_states: SmallVec<[(usize, Option<TextureBarrierState>); 16]>,
	/// The range of the shader-visible resource heap that this frame's UAV clears take their descriptors from.
	clear_descriptors: Option<DescriptorLease>,
	/// Clear descriptor ranges outgrown during the recording, given back once its submission completes.
	retired_clear_descriptors: SmallVec<[std::ops::Range<usize>; 2]>,
	/// Descriptor set ranges this recording read before their set moved away from them, as (sampler heap, range).
	/// They go back once the recording's submission completes.
	retired_set_descriptors: SmallVec<[(bool, std::ops::Range<usize>); 2]>,
	/// Counts the recordings this frame started, so a set copy can tell whether the recording that bound it finished:
	/// a frame starts a new recording only after its previous submission completed.
	generation: u64,
	is_open: bool,
	recorded_work: bool,
	sequence_index: u8,
	frame_synchronizer: Option<SynchronizerHandle>,
	last_submission: Option<(SynchronizerHandle, u8)>,
}

impl CommandBufferFrame {
	/// Releases ownership recorded by the previous command list before this frame slot is reused.
	fn clear_recording_state(&mut self) {
		self.recorded_work = false;
		self.frame_synchronizer = None;
		self.last_submission = None;
		self.retained_descriptor_heaps.clear();
		self.retained_resources.clear();
		self.retained_resource_keys.clear();
		self.descriptor_sync_scratch.clear();
		self.present_resources.clear();
		self.recorded_readbacks.clear();
		self.recorded_builds = Default::default();
		self.vertex_buffer_views.clear();
		self.vertex_strides.clear();
		self.original_buffer_states.clear();
		self.original_image_states.clear();
	}

	/// Starts the clear descriptor range over and gives back the ranges it outgrew, once no submission reads them.
	fn rewind_clear_descriptors(&mut self, heaps: &mut ShaderVisibleHeaps) {
		if let Some(lease) = self.clear_descriptors.as_mut() {
			lease.used = 0;
		}
		for range in self.retired_clear_descriptors.drain(..) {
			heaps.free_resources.give_back(range);
		}
		for (sampler_heap, range) in self.retired_set_descriptors.drain(..) {
			if sampler_heap {
				heaps.free_samplers.give_back(range);
			} else {
				heaps.free_resources.give_back(range);
			}
		}
		self.generation += 1;
	}

	/// Gives back every clear descriptor range before changing the resource topology.
	fn release_clear_descriptors(&mut self, heaps: &mut ShaderVisibleHeaps) {
		self.rewind_clear_descriptors(heaps);
		if let Some(lease) = self.clear_descriptors.take() {
			heaps.free_resources.give_back(lease.range);
		}
	}
}

/// The `CommandBuffer` struct keeps one independently reusable native recording for each frame in flight.
struct CommandBuffer {
	queue_handle: QueueHandle,
	frames: Vec<CommandBufferFrame>,
	active_sequence_index: u8,
}

impl CommandBuffer {
	/// Selects the native recording state owned by the requested frame sequence.
	fn activate_sequence(&mut self, sequence_index: u8) {
		assert!(
			(sequence_index as usize) < self.frames.len(),
			"Missing DX12 command-buffer frame. The most likely cause is that the command buffer was not extended after the frames-in-flight count increased."
		);
		self.active_sequence_index = sequence_index;
	}

	/// Returns whether any frame-local native recording matches the requested lifecycle predicate.
	fn frames_any(&self, predicate: impl Fn(CommandBufferLifecycle) -> bool) -> bool {
		self.frames.iter().any(|frame| predicate(frame.lifecycle))
	}
}

impl std::ops::Deref for CommandBuffer {
	type Target = CommandBufferFrame;

	fn deref(&self) -> &Self::Target {
		&self.frames[self.active_sequence_index as usize]
	}
}

impl std::ops::DerefMut for CommandBuffer {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.frames[self.active_sequence_index as usize]
	}
}

/// The `CommandBufferLifecycle` enum prevents a reusable native command list from being reset while an execution owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandBufferLifecycle {
	Idle,
	Recording,
	Scheduled,
	Submitted,
	Poisoned,
}

/// The `BufferBarrierState` struct preserves the synchronization and access contract for one buffer use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BufferBarrierState {
	sync: D3D12_BARRIER_SYNC,
	access: D3D12_BARRIER_ACCESS,
}

impl BufferBarrierState {
	pub(crate) const COMMON: Self = Self::new(D3D12_BARRIER_SYNC_ALL, D3D12_BARRIER_ACCESS_COMMON);
	pub(crate) const COPY_SOURCE: Self = Self::new(D3D12_BARRIER_SYNC_COPY, D3D12_BARRIER_ACCESS_COPY_SOURCE);
	pub(crate) const COPY_DESTINATION: Self = Self::new(D3D12_BARRIER_SYNC_COPY, D3D12_BARRIER_ACCESS_COPY_DEST);
	pub(crate) const VERTEX_BUFFER: Self = Self::new(D3D12_BARRIER_SYNC_VERTEX_SHADING, D3D12_BARRIER_ACCESS_VERTEX_BUFFER);
	pub(crate) const INDEX_BUFFER: Self = Self::new(D3D12_BARRIER_SYNC_INDEX_INPUT, D3D12_BARRIER_ACCESS_INDEX_BUFFER);
	pub(crate) const INDIRECT_ARGUMENT: Self =
		Self::new(D3D12_BARRIER_SYNC_EXECUTE_INDIRECT, D3D12_BARRIER_ACCESS_INDIRECT_ARGUMENT);
	pub(crate) const ACCELERATION_STRUCTURE_INPUT: Self =
		Self::new(D3D12_BARRIER_SYNC_ALL, D3D12_BARRIER_ACCESS_SHADER_RESOURCE);

	pub(crate) const fn new(sync: D3D12_BARRIER_SYNC, access: D3D12_BARRIER_ACCESS) -> Self {
		Self { sync, access }
	}

	pub(crate) const fn shader_resource(sync: D3D12_BARRIER_SYNC) -> Self {
		Self::new(sync, D3D12_BARRIER_ACCESS_SHADER_RESOURCE)
	}

	pub(crate) const fn constant_buffer(sync: D3D12_BARRIER_SYNC) -> Self {
		Self::new(sync, D3D12_BARRIER_ACCESS_CONSTANT_BUFFER)
	}

	pub(crate) const fn unordered_access(sync: D3D12_BARRIER_SYNC) -> Self {
		Self::new(sync, D3D12_BARRIER_ACCESS_UNORDERED_ACCESS)
	}
}

/// The `TextureBarrierState` struct preserves synchronization, access, and layout for one texture use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextureBarrierState {
	sync: D3D12_BARRIER_SYNC,
	access: D3D12_BARRIER_ACCESS,
	layout: D3D12_BARRIER_LAYOUT,
}

impl TextureBarrierState {
	pub(crate) const COMMON: Self = Self::new(
		D3D12_BARRIER_SYNC_ALL,
		D3D12_BARRIER_ACCESS_COMMON,
		D3D12_BARRIER_LAYOUT_COMMON,
	);
	pub(crate) const PRESENT: Self = Self::new(
		D3D12_BARRIER_SYNC_NONE,
		D3D12_BARRIER_ACCESS_NO_ACCESS,
		D3D12_BARRIER_LAYOUT_PRESENT,
	);
	pub(crate) const COPY_SOURCE: Self = Self::new(
		D3D12_BARRIER_SYNC_COPY,
		D3D12_BARRIER_ACCESS_COPY_SOURCE,
		D3D12_BARRIER_LAYOUT_COMMON,
	);
	pub(crate) const COPY_DESTINATION: Self = Self::new(
		D3D12_BARRIER_SYNC_COPY,
		D3D12_BARRIER_ACCESS_COPY_DEST,
		D3D12_BARRIER_LAYOUT_COMMON,
	);
	pub(crate) const RENDER_TARGET: Self = Self::new(
		D3D12_BARRIER_SYNC_RENDER_TARGET,
		D3D12_BARRIER_ACCESS_RENDER_TARGET,
		D3D12_BARRIER_LAYOUT_RENDER_TARGET,
	);
	pub(crate) const DEPTH_WRITE: Self = Self::new(
		D3D12_BARRIER_SYNC_DEPTH_STENCIL,
		D3D12_BARRIER_ACCESS_DEPTH_STENCIL_WRITE,
		D3D12_BARRIER_LAYOUT_DEPTH_STENCIL_WRITE,
	);

	pub(crate) const fn new(sync: D3D12_BARRIER_SYNC, access: D3D12_BARRIER_ACCESS, layout: D3D12_BARRIER_LAYOUT) -> Self {
		Self { sync, access, layout }
	}

	pub(crate) const fn shader_resource(sync: D3D12_BARRIER_SYNC) -> Self {
		Self::new(
			sync,
			D3D12_BARRIER_ACCESS_SHADER_RESOURCE,
			D3D12_BARRIER_LAYOUT_SHADER_RESOURCE,
		)
	}

	pub(crate) const fn unordered_access(sync: D3D12_BARRIER_SYNC) -> Self {
		Self::new(
			sync,
			D3D12_BARRIER_ACCESS_UNORDERED_ACCESS,
			D3D12_BARRIER_LAYOUT_UNORDERED_ACCESS,
		)
	}
}

/// The `EnhancedBarrierBatch` struct owns grouped barriers and their retained resource references until submission.
#[derive(Default)]
struct EnhancedBarrierBatch {
	global: SmallVec<[D3D12_GLOBAL_BARRIER; 4]>,
	buffer: SmallVec<[D3D12_BUFFER_BARRIER; 32]>,
	texture: SmallVec<[D3D12_TEXTURE_BARRIER; 32]>,
}

impl EnhancedBarrierBatch {
	/// Submits at most one native group for each enhanced barrier type.
	fn submit(&self, command_list: &ID3D12GraphicsCommandList7) {
		let mut groups = SmallVec::<[D3D12_BARRIER_GROUP; 3]>::new();
		if !self.global.is_empty() {
			groups.push(D3D12_BARRIER_GROUP {
				Type: D3D12_BARRIER_TYPE_GLOBAL,
				NumBarriers: self.global.len() as u32,
				Anonymous: D3D12_BARRIER_GROUP_0 {
					pGlobalBarriers: self.global.as_ptr(),
				},
			});
		}
		if !self.buffer.is_empty() {
			groups.push(D3D12_BARRIER_GROUP {
				Type: D3D12_BARRIER_TYPE_BUFFER,
				NumBarriers: self.buffer.len() as u32,
				Anonymous: D3D12_BARRIER_GROUP_0 {
					pBufferBarriers: self.buffer.as_ptr(),
				},
			});
		}
		if !self.texture.is_empty() {
			groups.push(D3D12_BARRIER_GROUP {
				Type: D3D12_BARRIER_TYPE_TEXTURE,
				NumBarriers: self.texture.len() as u32,
				Anonymous: D3D12_BARRIER_GROUP_0 {
					pTextureBarriers: self.texture.as_ptr(),
				},
			});
		}
		if groups.is_empty() {
			return;
		}

		// SAFETY: Each group points into this live batch, and the synchronous call only borrows the arrays.
		unsafe { command_list.Barrier(&groups) };
	}
}

impl Drop for EnhancedBarrierBatch {
	fn drop(&mut self) {
		// Windows models barrier resource pointers as ManuallyDrop COM options. Release the clones retained by this batch.
		for barrier in &mut self.buffer {
			// SAFETY: Each resource option was initialized once by `buffer_barrier` and is dropped exactly here.
			unsafe { std::mem::ManuallyDrop::drop(&mut barrier.pResource) };
		}
		for barrier in &mut self.texture {
			// SAFETY: Each resource option was initialized once by `texture_barrier` and is dropped exactly here.
			unsafe { std::mem::ManuallyDrop::drop(&mut barrier.pResource) };
		}
	}
}

/// The `DescriptorHeap` struct caches native heap starts for allocation-free handle arithmetic.
#[derive(Clone)]
struct DescriptorHeap {
	native: ID3D12DescriptorHeap,
	cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
	gpu_start: Option<D3D12_GPU_DESCRIPTOR_HANDLE>,
}

/// The `ShaderVisibleHeaps` struct owns the two descriptor heaps that shaders index directly, and keeps track of
/// the ranges of them that descriptor sets and UAV clears lease.
///
/// The sampler heap starts with the [`FIXED_SAMPLER_COUNT`] fixed samplers, which shaders index by their state; set
/// ranges come after them.
struct ShaderVisibleHeaps {
	resources: DescriptorHeap,
	samplers: DescriptorHeap,
	free_resources: ::utils::RangeAllocator,
	free_samplers: ::utils::RangeAllocator,
	/// How many descriptors `resources` holds.
	resource_capacity: u32,
}

/// How many CBV, SRV, and UAV descriptors the shader-visible resource heap holds, across every descriptor set and
/// frame copy of a rendering context.
pub(crate) const RESOURCE_HEAP_CAPACITY: u32 = 1 << 18;

/// How many descriptors a factory's resource heap holds. A factory only compiles, so it never leases one, and this is
/// the least D3D12 creates.
pub(crate) const FACTORY_RESOURCE_HEAP_CAPACITY: u32 = 1;

/// How many samplers the shader-visible sampler heap holds, the most a D3D12 shader-visible sampler heap may hold.
const SAMPLER_HEAP_CAPACITY: u32 = 2048;

/// The `DescriptorLease` struct is one range of the shader-visible resource heap that a command-buffer frame fills
/// from its start, one descriptor at a time.
struct DescriptorLease {
	range: std::ops::Range<usize>,
	used: u32,
}

/// The `DescriptorHeapArena` struct exists to reuse descriptor slots across command-buffer recordings.
struct DescriptorHeapArena {
	heap: DescriptorHeap,
	capacity: u32,
	used: u32,
}

/// The `RetainedCpuDescriptor` struct identifies a stable CPU descriptor slot owned by the device pool.
#[derive(Clone)]
struct RetainedCpuDescriptor {
	heap: DescriptorHeap,
	page_index: usize,
	slot: u32,
}

/// The `AttachmentViewKey` struct identifies a retained CPU descriptor for one native image view.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AttachmentViewKey {
	resource: usize,
	format: i32,
}

/// The `RenderTargetAttachment` struct carries one resolved color attachment through native binding.
struct RenderTargetAttachment {
	image_handle: Option<crate::BaseImageHandle>,
	resource: ID3D12Resource,
	format: Formats,
	array_layers: u32,
	/// The first layer the attachment covers, which a render-target index of zero writes.
	first_layer: u32,
	layer_count: u32,
	/// The value the pass clears the target to, or `None` when it loads or discards the contents.
	clear: Option<ClearValue>,
}

/// The `Buffer` struct keeps one logical DX12 buffer: the storage of each of its copies, which GPU work and the CPU
/// both use directly.
///
/// A buffer with [`crate::Copies::PerFrame`] has one copy for each frame sequence, all created with the buffer, so a
/// frame never shares storage with another frame in flight.
pub(crate) struct Buffer {
	/// The storage of copy 0, which every sequence uses when the buffer has one copy.
	memory: BufferMemory,
	/// The storage of copies 1 and up, one for each other frame sequence. Empty for a buffer with one copy.
	frame_copies: Vec<BufferMemory>,
	size: usize,
	/// The alignment of the buffer's contents, which a zero-sized buffer's dangling CPU pointer must still have.
	align: usize,
	uses: Uses,
	host_access: crate::HostAccess,
	copies: crate::Copies,
	/// The debug name each native copy gets, kept only while debug labels are on.
	name: Option<Box<str>>,
}

impl Buffer {
	/// Returns the storage that `sequence_index` uses: its own copy, or copy 0 for a buffer with one copy.
	fn memory(&self, sequence_index: u8) -> &BufferMemory {
		match sequence_index {
			0 => &self.memory,
			index => self.frame_copies.get(index as usize - 1).unwrap_or(&self.memory),
		}
	}

	/// Returns the typed CPU view of the copy `sequence_index` uses.
	///
	/// # Panics
	///
	/// Panics when the copy has no CPU mapping, which only buffers with host access have.
	fn typed_pointer<T: ?Sized + crate::buffer::BufferContents>(&self, sequence_index: u8) -> *mut T {
		let memory = self.memory(sequence_index);
		let data = if self.size == 0 {
			// A zero-sized view still needs a non-null, aligned pointer, which is never read through.
			std::ptr::without_provenance_mut(self.align)
		} else {
			assert!(
				!memory.mapped.is_null(),
				"DX12 buffer has no CPU mapping. The most likely cause is a buffer built without host access. Build it with HostAccess::Write or HostAccess::Read."
			);
			memory.mapped
		};
		<T as crate::buffer::BufferContents>::from_raw_parts(data, self.size).expect(
			"Failed to view a typed DX12 buffer. The most likely cause is that the buffer was built for another type. Use the handle with the type it was built with.",
		)
	}
}

/// The `BufferMemory` struct is the native resource of one copy of a buffer, and its persistent CPU mapping when the
/// buffer has host access.
pub(crate) struct BufferMemory {
	resource: Option<ID3D12Resource>,
	/// The CPU address of the whole resource, mapped for its lifetime, or null for GPU-local memory.
	mapped: *mut u8,
	heap_kind: BufferHeapKind,
}

struct BufferCopyInfo {
	resource: ID3D12Resource,
	size: usize,
}

/// The `ResourceIoBufferDestination` struct carries a static DX12 buffer into a DirectStorage request.
pub(crate) struct ResourceIoBufferDestination {
	pub(crate) resource: ID3D12Resource,
	pub(crate) size: usize,
	pub(crate) common_state: bool,
	pub(crate) direct_storage_compatible: bool,
}

/// The `ResourceIoImageDestination` struct carries a static DX12 image into a DirectStorage request.
pub(crate) struct ResourceIoImageDestination {
	pub(crate) resource: ID3D12Resource,
	pub(crate) extent: Extent,
	pub(crate) format: Formats,
	pub(crate) array_layers: u32,
	pub(crate) mip_levels: u32,
	pub(crate) common_state: bool,
}

/// The `TextureReadback` struct keeps one DX12 transfer result and optional native staging alive until consumption.
struct TextureReadback {
	completion: Option<(crate::synchronizer::SynchronizerHandle, u64)>,
	resource: Option<ID3D12Resource>,
	sequence_index: u8,
	row_pitch: usize,
	height: usize,
	depth: usize,
	size: usize,
	mapping_failed: bool,
	/// The compact result, whose `bytes` fill in once the copy completes.
	data: MappedTextureReadback,
}

/// The `BufferHeapKind` enum names the native heap a buffer lives in, which its CPU access and uses decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferHeapKind {
	/// GPU-local memory the CPU can't map, for buffers with [`crate::HostAccess::None`].
	Default,
	/// Write-combined system memory the CPU writes, for buffers with [`crate::HostAccess::Write`]. It's the memory an
	/// upload heap uses on a discrete GPU, in a custom heap, so shaders and copies may also write it, which an upload
	/// heap allows neither of.
	Upload,
	/// CPU-cached system memory the CPU reads, for buffers with [`crate::HostAccess::Read`]. A custom heap rather than
	/// a readback heap, so shaders may write it and copies may read it; GPU access to it crosses the bus.
	Readback,
}

// SAFETY: `mapped` points into the mapping of the value's own resource, which it unmaps on drop and shares with no
// other value, so moving the memory to another thread is sound.
unsafe impl Send for BufferMemory {}

impl Drop for BufferMemory {
	fn drop(&mut self) {
		if !self.mapped.is_null()
			&& let Some(resource) = self.resource.as_ref()
		{
			unsafe {
				resource.Unmap(0, None);
			}
		}
	}
}

pub(crate) struct Image {
	extent: Extent,
	is_3d: bool,
	format: Formats,
	uses: Uses,
	array_layers: u32,
	mip_levels: u32,
	resource: Option<ID3D12Resource>,
	/// The texture of each frame sequence for an image with a copy per frame, created by the sequence's first use.
	frame_resources: Option<Vec<Option<ID3D12Resource>>>,
	optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
	/// The debug name each native copy gets, kept only while debug labels are on.
	name: Option<Box<str>>,
}

struct Sampler {
	filtering_mode: FilteringModes,
	reduction_mode: SamplingReductionModes,
	mip_map_mode: FilteringModes,
	addressing_mode: SamplerAddressingModes,
	anisotropy: Option<f32>,
	min_lod: f32,
	max_lod: f32,
}

/// The `DescriptorSet` struct is one frame's copy of a descriptor set: the retained writes of its layout's
/// descriptors, and the ranges of the shader-visible heaps that hold their native descriptors.
///
/// A shader finds the set through two root constants, its resource and sampler bases, so binding the set writes only
/// those. A write changes the retained descriptor and queues its element in `stale`; the native descriptor follows at
/// the copy's next bind, which records for the copy's own frame sequence, so the heap never changes under queued work.
pub(crate) struct DescriptorSet {
	pub(crate) next: Option<crate::descriptors::DescriptorSetHandle>,
	sequence_index: u8,
	/// The recording that bound this copy last. While its work may still run, a changed copy moves to new heap ranges
	/// at its next bind instead of overwriting the ones that work reads.
	reader: Option<SetReader>,
	layout: crate::shader::DescriptorSetLayout,
	/// The heap index of the set's first descriptor; the set holds `layout.element_count()` of them.
	resource_base: u32,
	/// The heap index of the set's first application-bound sampler; see [`Device::layout_sampler_count`].
	sampler_base: u32,
	/// The retained write of each descriptor, at `layout.element_base(resource) + array_element`.
	descriptors: Box<[Option<RetainedDescriptor>]>,
	/// Elements whose native descriptor no longer matches what they reference. Kept between binds for its capacity.
	stale: Vec<u32>,
}

/// The `SetReader` struct names one recording of a command buffer frame, so a descriptor set copy can tell whether
/// work that reads its heap ranges may still run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct SetReader {
	command_buffer: CommandBufferHandle,
	frame: u8,
	/// The frame's recording generation; see [`CommandBufferFrame::generation`].
	generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RetainedDescriptor {
	descriptor: WriteData,
	frame_offset: i32,
}

/// The `PipelineSet` struct is one descriptor set layout a pipeline's stages read, which the bound set created from
/// the same layout fills.
struct PipelineSet {
	layout: crate::shader::DescriptorSetLayout,
	/// The layout's resources any stage uses, as a mask over [`crate::shader::DescriptorSetLayout::resources`].
	used: u64,
}

/// The `StageSets` struct is the root-constant parameter that gives one shader stage the bases of its sets.
struct StageSets {
	root_parameter: u32,
	/// For each set position of the stage's shader, up to the last one it reads, the set's index in
	/// [`NativePipelineLayout::sets`], or `None` when the stage reads nothing from it.
	sets: SmallVec<[Option<u8>; 8]>,
}

/// The `RootSignatureKey` struct is the shape of a root signature, which is all that tells two apart.
#[derive(Clone, PartialEq, Eq, Hash)]
struct RootSignatureKey {
	push_constant_dwords: u32,
	/// The visibility and root-constant count of each stage's set bases, in root parameter order.
	stages: SmallVec<[(i32, u32); 3]>,
}

/// The `NativePipelineLayout` struct is how one pipeline finds its descriptor sets: its root signature, the set
/// layouts its stages read, and where each stage's set bases go.
pub(crate) struct NativePipelineLayout {
	root_signature: ID3D12RootSignature,
	sets: SmallVec<[PipelineSet; 4]>,
	stages: SmallVec<[StageSets; 3]>,
	push_constant_root: Option<u32>,
	push_constant_ranges: Box<[PushConstantRange]>,
}

pub(crate) struct Pipeline {
	pub(crate) layout: PipelineLayoutHandle,
	pub(crate) kind: PipelineKind,
	native: NativePipeline,
	has_mesh_shader: bool,
	/// The bytes one vertex takes in each input slot, by slot. D3D12 reads the stride from the vertex buffer view, not
	/// from the pipeline, so binding the pipeline puts these in the bound views.
	vertex_strides: SmallVec<[u32; 4]>,
}

/// The `NativePipeline` enum is the native object that binding a pipeline sets on a command list.
///
/// A pipeline can't exist without one, so a bound raster or compute pipeline always records its draws and dispatches.
/// Creation panics instead of producing a pipeline with no native object; see [`Device::create_compute_pipeline`].
pub(crate) enum NativePipeline {
	/// The pipeline state of a raster, mesh, or compute pipeline.
	State(ID3D12PipelineState),
	/// The state object and shader identifiers of a ray tracing pipeline.
	RayTracing(Box<crate::dx12::ray_tracing::RayTracingState>),
}

/// The `CompiledPipeline` struct is a pipeline no context owns yet: its layout and its native state, which a factory
/// thread or the owning context compiled. Compiled objects belong to the device, so any context of it can intern one.
pub(crate) struct CompiledPipeline {
	layout: NativePipelineLayout,
	kind: PipelineKind,
	native: NativePipeline,
	has_mesh_shader: bool,
	vertex_strides: SmallVec<[u32; 4]>,
}

#[repr(C, align(8))]
struct PipelineStateStreamSubobject<T> {
	subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE,
	value: T,
}

#[repr(C)]
struct MeshPipelineStateStream {
	root_signature: PipelineStateStreamSubobject<Option<ID3D12RootSignature>>,
	amplification_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	mesh_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	pixel_shader: PipelineStateStreamSubobject<D3D12_SHADER_BYTECODE>,
	blend: PipelineStateStreamSubobject<D3D12_BLEND_DESC>,
	sample_mask: PipelineStateStreamSubobject<u32>,
	rasterizer: PipelineStateStreamSubobject<D3D12_RASTERIZER_DESC>,
	depth_stencil: PipelineStateStreamSubobject<D3D12_DEPTH_STENCIL_DESC>,
	depth_stencil_format: PipelineStateStreamSubobject<DXGI_FORMAT>,
	render_targets: PipelineStateStreamSubobject<D3D12_RT_FORMAT_ARRAY>,
	sample_desc: PipelineStateStreamSubobject<DXGI_SAMPLE_DESC>,
	node_mask: PipelineStateStreamSubobject<u32>,
	flags: PipelineStateStreamSubobject<D3D12_PIPELINE_STATE_FLAGS>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipelineKind {
	Raster,
	Compute,
	/// A DXR pipeline, which binds through the compute root signature.
	RayTracing,
}

struct Shader {
	/// The name the shader was created with, for errors that concern it.
	name: Option<Box<str>>,
	stage: ShaderTypes,
	/// Shared so each pipeline that uses the shader borrows its bytecode instead of copying it.
	dxil: Option<std::sync::Arc<[u8]>>,
	/// The DXIL of each specialization of the shader, by its specialization map; see [`crate::shader::Sources::DXIL`].
	variants: Box<[(Box<[pipelines::SpecializationMapEntry]>, std::sync::Arc<[u8]>)]>,
	/// The sets the shader declares, at the positions its source reads them.
	descriptor_sets: Box<[crate::shader::ShaderDescriptorSet]>,
}

/// The `DxcCompiler` struct keeps the validated compiler and cache identity available to runtime HLSL compilation.
///
/// Each value owns its own native compiler, so a clone given to another context never shares one with it.
pub(crate) struct DxcCompiler {
	native: IDxcCompiler3,
	identity: Box<str>,
}

impl DxcCompiler {
	/// Loads DXC and validates the version that every runtime HLSL compilation will use.
	pub(crate) fn load() -> Result<Self, String> {
		// SAFETY: DXC owns the registered compiler class and returns a typed COM interface on success.
		let native = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }.map_err(|error| {
			format!(
				"Failed to create the DirectX Shader Compiler. The most likely cause is that dxcompiler.dll is missing or incompatible. Error: {error:?}"
			)
		})?;
		let identity = Device::dxc_identity(&native)?.into();
		Ok(Self { native, identity })
	}
}

impl Clone for DxcCompiler {
	fn clone(&self) -> Self {
		// SAFETY: DXC owns the registered compiler class and returns a typed COM interface on success.
		let native = unsafe { DxcCreateInstance::<IDxcCompiler3>(&CLSID_DxcCompiler) }
			.expect("Failed to create the DirectX Shader Compiler. The most likely cause is that dxcompiler.dll was unloaded.");
		Self {
			native,
			identity: self.identity.clone(),
		}
	}
}

// SAFETY: DXC compilers are free-threaded COM objects, and each value owns its own instance (see `Clone`), so moving
// one to another thread never lets two threads use the same compiler at once.
unsafe impl Send for DxcCompiler {}

struct Mesh {
	index_count: u32,
	/// Byte sizes of the vertex and index buffers, which the draw's buffer views cover.
	vertex_bytes: u32,
	index_bytes: u32,
	vertex_size: usize,
	vertex_resource: Option<ID3D12Resource>,
	index_resource: Option<ID3D12Resource>,
}

/// Returns how many DXGI buffers a swapchain holds: the desired count, but never fewer than one more than the frames in
/// flight, nor more than the eight tracked backbuffers.
///
/// Each frame sequence may queue a present while another buffer stays on screen, so the extra buffer lets the
/// swapchain's frame latency match the frames in flight without `Present` blocking. See [`super::pacing::frame_latency`].
pub(crate) fn swapchain_image_count(frames: u8, desired_image_count: Option<u8>) -> u8 {
	let minimum = (frames + 1).max(2);
	desired_image_count.unwrap_or(minimum).clamp(minimum, 8)
}

pub(crate) struct Swapchain {
	/// The window, which the swapchain only queries for its client extent.
	handles: window::Handles,
	swapchain: IDXGISwapChain3,
	extent: Extent,
	image_count: u8,
	/// The image count requested when binding the window; `None` follows the frames in flight.
	desired_image_count: Option<u8>,
	next_image_index: u8,
	present_mode: PresentationModes,
	/// The proxy image that the engine renders into; each frame sequence resolves it to its own native resource.
	image: Option<ImageHandle>,
	/// The uses requested for the proxy image so far.
	proxy_uses: Uses,
	proxy_resource_uses: Uses,
	backbuffers: [Option<ID3D12Resource>; 8],
	pub(crate) acquired_image_indices: [u8; 8],
	pub(crate) acquired_sequences: [bool; 8],
	queue_handle: QueueHandle,
	/// The minimum time between presented frames; `None` presents on the next refresh.
	pub(crate) present_interval: Option<std::time::Duration>,
	/// The earliest time the next acquisition may start when `present_interval` is set and no sync interval can hold
	/// it.
	pub(crate) next_present_slot: Option<std::time::Instant>,
	/// The flags the swapchain was created with, which every `ResizeBuffers` call must repeat.
	flags: DXGI_SWAP_CHAIN_FLAG,
	/// Whether presents may tear, which [`PresentationModes::Inmediate`] uses when the display supports it.
	allows_tearing: bool,
	/// Signals when the swapchain's present queue has room for another frame. Acquisition waits on it, so each tick
	/// starts when the display takes a frame instead of blocking inside `Present` after the tick simulated.
	latency_waitable: windows::core::Owned<HANDLE>,
	/// The maximum frame latency last set on the swapchain.
	frame_latency: u32,
	/// The sync interval the next present uses, chosen at acquisition from the mode and `present_interval`.
	sync_interval: u32,
	/// The refresh interval of the window's monitor, with the monitor and when it was read; see
	/// [`Self::refresh_interval`].
	refresh: Option<(HMONITOR, std::time::Instant, Option<std::time::Duration>)>,
	/// The `SyncQPCTime` last converted to an `Instant`, with the result. Converting reads two clocks, so converting an
	/// unchanged time again would land on a slightly different instant.
	shown_instant: Option<(i64, std::time::Instant)>,
	/// The time the previous acquisition predicted its frame would be shown.
	last_target: Option<std::time::Instant>,
}

impl Swapchain {
	/// Releases the backbuffers, which DXGI requires before `ResizeBuffers`, and resizes the swapchain to
	/// `image_count` buffers of `extent`.
	///
	/// # Panics
	///
	/// Panics when DXGI rejects the resize.
	fn resize_buffers(&mut self, image_count: u8, extent: Extent) {
		self.backbuffers = std::array::from_fn(|_| None);
		self.acquired_sequences = [false; 8];
		// SAFETY: The swapchain holds no backbuffer reference, and its flags repeat the ones it was created with.
		unsafe {
			self.swapchain.ResizeBuffers(
				image_count as u32,
				extent.width(),
				extent.height(),
				DXGI_FORMAT_B8G8R8A8_UNORM,
				self.flags,
			)
		}
		.expect("Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed.");
		self.extent = extent;
		self.image_count = image_count;
		self.next_image_index %= image_count;
	}

	/// How often the refresh interval of an unchanged monitor is read again, so a display mode change is picked up.
	const REFRESH_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

	/// Returns the refresh interval of the monitor that shows most of the window.
	///
	/// Reading a display mode is slow, so the result is kept until the window moves to another monitor or
	/// [`Self::REFRESH_RECHECK`] passes.
	fn refresh_interval(&mut self) -> Option<std::time::Duration> {
		// SAFETY: `MonitorFromWindow` only reads the window's position; an invalid window yields the nearest monitor.
		let monitor = unsafe { MonitorFromWindow(self.handles.hwnd, MONITOR_DEFAULTTONEAREST) };
		let now = std::time::Instant::now();
		match self.refresh {
			Some((cached, read_at, interval)) if cached == monitor && now - read_at < Self::REFRESH_RECHECK => interval,
			_ => {
				let interval = crate::window::os::win32::monitor_refresh_interval(monitor);
				self.refresh = Some((monitor, now, interval));
				interval
			}
		}
	}
}

// SAFETY: The window handles are identifiers, and the only call made with them, `GetClientRect`, is valid from any
// thread. Every other field is a free-threaded D3D12/DXGI interface or plain data.
unsafe impl Send for Swapchain {}

pub(crate) struct Synchronizer {
	pub(crate) next: Option<crate::synchronizer::SynchronizerHandle>,
	fence: ID3D12Fence,
	value: u64,
	last_signal_queue: Option<QueueHandle>,
}

/// The `DeferredTask` enum keeps DX12 work and resources with the frame sequence that can safely process them.
pub(crate) enum DeferredTask {
	RetireResource(ID3D12Resource),
	ResizeImage { handle: ImageHandle, extent: Extent },
}

fn wide_null(value: &str) -> Vec<u16> {
	value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The `Execution` struct exists to collect frame-scoped DX12 command recordings for a queue submission.
pub struct Execution<'a> {
	pub(crate) frame: Option<super::Frame<'a>>,
	pub(crate) completed_frame: Option<crate::FrameKey>,
	pub(crate) command_buffers: smallvec::SmallVec<[CommandBufferHandle; 4]>,
	pub(crate) prepared_present_keys: smallvec::SmallVec<[PresentKey; 4]>,
	pub(crate) queue_handle: QueueHandle,
}

impl Drop for Execution<'_> {
	fn drop(&mut self) {
		let Some(frame) = self.frame.as_mut() else {
			return;
		};
		// Recordings update state in order, so abandon them in reverse to restore the committed state.
		for &command_buffer in self.command_buffers.iter().rev() {
			frame.device().discard_command_buffer_recording(command_buffer);
		}
	}
}

impl crate::device::Device for Device {
	type Context = Device;
	type Allocator = std::alloc::Global;
	type RasterPipeline = crate::dx12::factory::RasterPipeline;
	type ComputePipeline = crate::dx12::factory::ComputePipeline;
	type RayTracingPipeline = crate::dx12::factory::RayTracingPipeline;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Ok(Device::from_native_parts(
			self.device.clone(),
			self.device_configuration.clone(),
			self.dxc_compiler.clone(),
			self.settings,
			self.info_queue.clone(),
			self.debug_log_function,
			self.queues.clone(),
			RESOURCE_HEAP_CAPACITY,
		))
	}

	// The device creates detached shaders and pipelines as a factory does, so a frame interns them alike.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<ShaderHandle, ()> {
		Device::create_shader(self, name, shader_source_type, stage, descriptor_sets)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		crate::dx12::factory::RasterPipeline(self.compile_raster_pipeline(builder))
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		crate::dx12::factory::ComputePipeline(self.compile_compute_pipeline(builder))
	}

	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> Self::RayTracingPipeline {
		crate::dx12::factory::RayTracingPipeline(self.compile_ray_tracing_pipeline(builder))
	}
}

impl crate::context::ContextCreate for Device {
	fn add_mesh_from_vertices_and_indices(
		&mut self,
		_vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle {
		let vertex_size = vertex_layout.iter().map(|element| element.format.size()).sum::<usize>();
		let (vertex_resource, vertex_pointer, _) = self.create_buffer_resource(vertices.len(), crate::HostAccess::Write);
		let (index_resource, index_pointer, _) = self.create_buffer_resource(indices.len(), crate::HostAccess::Write);
		if !vertex_pointer.is_null() {
			unsafe {
				std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_pointer, vertices.len());
			}
		}
		if !index_pointer.is_null() {
			unsafe {
				std::ptr::copy_nonoverlapping(indices.as_ptr(), index_pointer, indices.len());
			}
		}

		self.meshes.push(Mesh {
			index_count,
			vertex_bytes: u32::try_from(vertices.len()).expect(
				"Failed to add a DX12 mesh: its vertex data is 4 GiB or larger. The most likely cause is a mesh that was not split. Split the mesh into parts under 4 GiB.",
			),
			index_bytes: u32::try_from(indices.len()).expect(
				"Failed to add a DX12 mesh: its index data is 4 GiB or larger. The most likely cause is a mesh that was not split. Split the mesh into parts under 4 GiB.",
			),
			vertex_size,
			vertex_resource,
			index_resource,
		});
		MeshHandle::from_index((self.meshes.len() - 1) as u64)
	}
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<ShaderHandle, ()> {
		Device::create_shader(self, name, shader_source_type, stage, descriptor_sets)
	}
	/// Creates a descriptor set from `layout`, with one copy for each frame in flight.
	///
	/// Each copy leases its ranges of the shader-visible heaps now and holds them for its lifetime, so binding it
	/// only writes two root constants. Next, fill it with [`Self::write`] and bind it with a pipeline whose shaders
	/// declare `layout`.
	fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		layout: &crate::shader::DescriptorSetLayout,
	) -> DescriptorSetHandle {
		let handle = DescriptorSetHandle::from_index(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for sequence_index in 0..self.frames {
			let descriptors = vec![None; layout.element_count() as usize].into_boxed_slice();
			let frame_handle = self.push_descriptor_set_copy(layout, sequence_index, descriptors);
			if let Some(previous) = previous {
				self.descriptor_sets[previous.index() as usize].next =
					Some(crate::descriptors::DescriptorSetHandle::from_index(frame_handle.index()));
			}
			previous = Some(frame_handle);
		}

		handle
	}
	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let pipeline = self.compile_raster_pipeline(builder);
		self.intern_pipeline(pipeline)
	}
	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> PipelineHandle {
		let pipeline = self.compile_compute_pipeline(builder);
		self.intern_pipeline(pipeline)
	}
	/// Links a ray tracing pipeline from this context's shaders and returns the handle commands bind it with.
	fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let pipeline = self.compile_ray_tracing_pipeline(builder);
		self.intern_pipeline(pipeline)
	}
	/// Creates a shader binding table that holds no records.
	fn create_shader_binding_table(&mut self, name: Option<&str>) -> crate::ShaderBindingTableHandle {
		self.require_ray_tracing("create a shader binding table");
		self.shader_binding_tables.push(ShaderBindingTable {
			name: name.map(str::to_owned),
			written: None,
		});
		crate::ShaderBindingTableHandle::from_index(self.shader_binding_tables.len() as u64 - 1)
	}
	/// Fills a new buffer with one record per entry of `hit_groups` for `pipeline`, and makes it the table's.
	///
	/// Recordings that traced with the table's old buffer keep it alive until they complete, so their traces keep their
	/// records.
	fn write_shader_binding_table(
		&mut self,
		table: crate::ShaderBindingTableHandle,
		pipeline: PipelineHandle,
		hit_groups: &[u32],
	) {
		self.require_ray_tracing("write a shader binding table");
		if table.index() as usize >= self.shader_binding_tables.len() {
			crate::pipelines::ray_tracing::unknown_table_write("DX12");
		}
		crate::pipelines::ray_tracing::check_records(
			"DX12",
			self.ray_tracing_state(pipeline, "write_shader_binding_table")
				.hit_groups
				.len(),
			hit_groups,
		);
		let size = table_size(hit_groups.len());
		let name = self.shader_binding_tables[table.index() as usize].name.clone();
		// The GPU reads a record each time a closest hit runs, from upload memory, which the CPU writes once.
		let resource = self.create_ray_tracing_buffer(name.as_deref(), size, D3D12_HEAP_TYPE_UPLOAD, D3D12_RESOURCE_FLAG_NONE);
		let mut mapped = std::ptr::null_mut();
		// SAFETY: Upload buffers map for writing, and the empty read range says the CPU reads nothing.
		unsafe { resource.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped)) }.expect(
			"DX12 shader binding table mapping failed. The most likely cause is that the device was removed. Check the device removed reason.",
		);
		// SAFETY: The mapping covers the buffer's `size` bytes, and only this write uses it.
		let memory = unsafe { std::slice::from_raw_parts_mut(mapped.cast::<u8>(), size as usize) };
		fill_shader_binding_table(
			self.ray_tracing_state(pipeline, "write_shader_binding_table"),
			hit_groups,
			memory,
		);
		// SAFETY: The CPU wrote every record it will, and the buffer stays mapped nowhere else.
		unsafe { resource.Unmap(0, None) };
		// SAFETY: The resource is a live buffer.
		let address = unsafe { resource.GetGPUVirtualAddress() };
		self.shader_binding_tables[table.index() as usize].written = Some(WrittenTable {
			pipeline,
			resource,
			ray_generation: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
				StartAddress: address,
				SizeInBytes: IDENTIFIER_SIZE as u64,
			},
			callable: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
				StartAddress: address + CALLABLE_OFFSET,
				SizeInBytes: RECORD_STRIDE * hit_groups.len() as u64,
				StrideInBytes: RECORD_STRIDE,
			},
		});
	}
	/// Builds every copy of a buffer; see [`crate::context::ContextCreate::build_buffer`].
	fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = self.create_buffer(
			T::layout(builder.length),
			builder.resource_uses,
			builder.host_access,
			builder.copies,
			builder.name,
		);
		BufferHandle(BaseBufferHandle::from_index(handle), std::marker::PhantomData)
	}
	/// Builds a buffer the CPU writes and writes every copy with `initialize`; see
	/// [`crate::context::ContextCreate::build_buffer_with`].
	fn build_buffer_with<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: buffer::Builder,
		initialize: impl Fn(&mut T),
	) -> BufferHandle<T> {
		crate::buffer::assert_initializable(builder.host_access);
		let handle = self.build_buffer::<T>(builder);
		let buffer = self.buffer(handle.into());
		for copy in 0..buffer.copies.count(self.frames) {
			// SAFETY: The copy is mapped storage of `T`'s layout, and no command has used the new buffer yet.
			initialize(unsafe { &mut *buffer.typed_pointer::<T>(copy) });
		}
		handle
	}
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		if builder.group.is_some() {
			crate::image_group::ImageGroups::validate_member(&builder);
		}
		// Reject unsupported view contracts before a logical handle can escape to callers.
		let array_layers = builder.array_layers.map(|layers| layers.get()).unwrap_or(1);
		let is_3d = builder.extent.depth() > 1;
		Self::validate_image_dimension(
			builder.extent,
			is_3d,
			array_layers,
			builder.cube_compatible || builder.cube_array_compatible,
		);
		self.validate_image_format_support(builder.format, builder.resource_uses, is_3d);
		// An image with a copy per frame creates each sequence's texture at that sequence's first use.
		let per_frame = builder.copies == crate::Copies::PerFrame;
		let flags = Self::image_resource_flags(builder.format, builder.resource_uses);
		let optimized_clear_value = builder
			.optimized_clear_value
			.and_then(|clear| Self::optimized_image_clear_value(builder.format, flags, clear));
		let resource = if per_frame {
			None
		} else {
			self.create_image_resource(
				builder.extent,
				is_3d,
				builder.format,
				builder.resource_uses,
				array_layers,
				builder.mip_levels,
				optimized_clear_value,
				builder.name,
			)
		};
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, builder.format, builder.resource_uses, array_layers);
		}
		let frame_resources = per_frame.then(|| vec![None; self.frames as usize]);

		self.images.push(Image {
			extent: builder.extent,
			is_3d,
			format: builder.format,
			uses: builder.resource_uses,
			array_layers,
			mip_levels: builder.mip_levels,
			resource,
			frame_resources,
			optimized_clear_value,
			name: self.debug_name(builder.name),
		});

		let handle = crate::BaseImageHandle::from_index((self.images.len() - 1) as u64);
		// A member built without an extent has no resource until its group is placed.
		if let Some(group) = builder.group {
			self.image_groups.add_member(group, handle);
		}

		ImageHandle(handle)
	}
	fn create_image_group(&mut self, name: Option<&str>) -> crate::ImageGroupHandle {
		self.image_group_heaps.push(SmallVec::new());
		self.image_groups.create(name)
	}
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
		// Stores sampler parameters without creating a DX12 descriptor.
		self.samplers.push(Sampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		});
		SamplerHandle::from_index((self.samplers.len() - 1) as u64)
	}
	/// Creates a bottom-level structure over `geometries`, sized for their build, after validating each geometry.
	fn create_bottom_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		geometries: &[rt::TriangleGeometry],
	) -> BottomLevelAccelerationStructureHandle {
		self.require_ray_tracing("create a bottom-level acceleration structure");
		rt::check_bottom_level_geometries("DX12", geometries);
		for geometry in geometries {
			// Builds resolve the same ranges later, so only creation checks them.
			self.check_geometry_range(&geometry.positions.buffer_offset, geometry.positions.size, 4);
			self.check_geometry_range(
				&geometry.indices,
				rt::index_bytes(geometry),
				rt::index_size(geometry.index_format),
			);
		}
		// Sizing ignores addresses, so the descriptions name none.
		let descriptions = geometries
			.iter()
			.map(|geometry| triangle_geometry(geometry, 0, 0))
			.collect::<SmallVec<[_; 8]>>();
		let info = self.prebuild_info(&build_inputs(&descriptions, 0, 0));
		let (resource, address) = self.create_structure_storage(name, info.ResultDataMaxSizeInBytes);
		let handle = self.acceleration_structures.bottom_levels.insert(BottomLevel {
			resource,
			address,
			size: info.ResultDataMaxSizeInBytes,
			geometries: geometries.into(),
			build_scratch_size: info.ScratchDataSizeInBytes,
			build_state: BuildState::Unbuilt,
		});
		BottomLevelAccelerationStructureHandle::from_bits(handle)
	}
	/// Creates a top-level structure with one copy for each frame in flight, each sized for `max_instance_count`
	/// instances and with instance records of its own.
	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		self.require_ray_tracing("create a top-level acceleration structure");
		rt::check_max_instance_count("DX12", max_instance_count);
		// Sizing ignores the address of the instance records; builds name their own.
		let info = self.prebuild_info(&build_inputs(&[], 0, max_instance_count));
		let mut copies = SmallVec::new();
		for _ in 0..self.frames {
			let (resource, address) = self.create_structure_storage(name, info.ResultDataMaxSizeInBytes);
			let instances = self.create_ray_tracing_buffer(
				name,
				u64::from(max_instance_count) * INSTANCE_SIZE,
				D3D12_HEAP_TYPE_UPLOAD,
				D3D12_RESOURCE_FLAG_NONE,
			);
			let mut mapped = std::ptr::null_mut();
			// SAFETY: Upload buffers map for writing, and the empty read range says the CPU reads nothing.
			unsafe { instances.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped)) }.expect(
				"DX12 instance buffer mapping failed. The most likely cause is that the device was removed. Check the device removed reason.",
			);
			copies.push(TopLevelCopy {
				resource,
				address,
				instances,
				instance_records: mapped.cast(),
				recorded_frame: None,
				built_epoch: None,
			});
		}
		let handle = self.acceleration_structures.top_levels.insert(TopLevel {
			copies,
			max_instance_count,
			build_scratch_size: info.ScratchDataSizeInBytes,
		});
		TopLevelAccelerationStructureHandle::from_bits(handle)
	}
	/// Destroys the bottom-level structure `handle` names once the frames recorded before this call complete.
	fn destroy_bottom_level_acceleration_structure(&mut self, handle: BottomLevelAccelerationStructureHandle) {
		let bottom_level = self.acceleration_structures.bottom_levels.remove(handle.bits()).unwrap_or_else(|| {
			panic!(
				"The bottom-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		self.retire_after_recorded_frames(bottom_level.resource);
	}
	/// Destroys the top-level structure `handle` names, and every copy, once the frames recorded before this call
	/// complete.
	fn destroy_top_level_acceleration_structure(&mut self, handle: TopLevelAccelerationStructureHandle) {
		let top_level = self.acceleration_structures.top_levels.remove(handle.bits()).unwrap_or_else(|| {
			panic!(
				"The top-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		for copy in top_level.copies {
			self.retire_after_recorded_frames(copy.resource);
			self.retire_after_recorded_frames(copy.instances);
		}
	}
	/// DX12 counters share one timestamp heap and one statistics heap, so a counter is only a slot owner and needs no
	/// native object.
	fn create_counter(&mut self, _name: Option<&str>, kind: crate::CounterKind) -> crate::CounterHandle {
		self.counters.create(kind)
	}

	fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let master = SynchronizerHandle::from_index(self.synchronizers.len() as u64);
		let mut previous: Option<crate::synchronizer::SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.index() as usize].next = Some(handle);
			}
			previous = Some(handle);
		}
		self.synchronizer_masters.push(master);
		master
	}
}

impl crate::context::Context for Device {
	type Queue<'a> = super::queue::Queue<'a>;
	type Recording<'a> = super::CommandBufferRecording<'a>;

	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn counter_duration(&self, counter: crate::CounterHandle) -> Option<std::time::Duration> {
		self.counters.duration(counter)
	}

	fn supports_pipeline_statistics(&self) -> bool {
		true
	}

	fn counter_statistics(&self, counter: crate::CounterHandle) -> Option<crate::PipelineStatistics> {
		self.counters.pipeline_statistics(counter)
	}

	fn supports_ray_tracing(&self) -> bool {
		self.ray_tracing_supported
	}

	/// Presentation creates UNORM back buffers, as `presentation.rs` builds them.
	fn swapchain_format(&self) -> Formats {
		Formats::BGRAu8
	}

	/// Returns the bytes the bottom-level structure `handle` occupies.
	fn bottom_level_acceleration_structure_size(&self, handle: BottomLevelAccelerationStructureHandle) -> usize {
		self.acceleration_structures.bottom_level(handle).size as usize
	}

	fn queue<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::Queue<'a> {
		super::queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn record<'a, R>(
		&'a mut self,
		queue: QueueHandle,
		name: Option<&str>,
		record: impl FnOnce(&mut Self::Recording<'a>) -> R,
	) -> R {
		self.record_workload(queue, name, record)
	}

	/// Rebuilds frame-local chains and resources at a global native queue-idle boundary.
	fn set_frames_in_flight(&mut self, frames: u8) {
		assert!(
			(1..=crate::MAX_FRAMES_IN_FLIGHT as u8).contains(&frames),
			"Invalid frames-in-flight count. The most likely cause is that the requested count is zero or exceeds the engine maximum."
		);
		let previous_frames = self.frames;
		if frames == previous_frames {
			return;
		}
		assert!(
			self.swapchains
				.iter()
				.all(|swapchain| swapchain.acquired_sequences.iter().all(|acquired| !acquired)),
			"DX12 frame topology cannot change while a swapchain image is acquired. The most likely cause is that a present key was retained across set_frames_in_flight."
		);
		self.idle_for_topology_change("changing frames in flight");
		self.resize_command_buffer_frames(frames);
		self.resize_descriptor_set_chains(frames);
		self.frames = frames;
		self.last_frame_synchronizers = [None; crate::MAX_FRAMES_IN_FLIGHT];

		// Keep dormant chain nodes linked so repeated frame-count changes reuse their fences instead of growing device storage.
		for master_index in 0..self.synchronizer_masters.len() {
			let master = self.synchronizer_masters[master_index];
			let handles = self.all_synchronizer_handles(master);
			if frames as usize > handles.len() {
				let mut tail = *handles.last().expect(
					"Missing DX12 synchronizer master. The most likely cause is that its sequence chain was corrupted.",
				);
				for _ in handles.len()..frames as usize {
					let next = self.create_synchronizer_internal(false);
					self.synchronizers[tail.index() as usize].next = Some(next);
					tail = next;
				}
			}
		}

		let mut retired_backbuffer_keys = SmallVec::<[usize; 8]>::new();
		for swapchain in &mut self.swapchains {
			let image_count = swapchain_image_count(frames, swapchain.desired_image_count);
			if swapchain.image_count != image_count && swapchain.extent.width() > 0 && swapchain.extent.height() > 0 {
				retired_backbuffer_keys.extend(swapchain.backbuffers.iter().flatten().map(Self::native_resource_key));
				let extent = swapchain.extent;
				swapchain.resize_buffers(image_count, extent);
			}
		}
		self.forget_native_resources(&retired_backbuffer_keys);

		let retired_image_keys = self
			.images
			.iter()
			.filter_map(|image| image.frame_resources.as_ref())
			.flat_map(|resources| {
				resources
					.iter()
					.skip(frames as usize)
					.flatten()
					.map(Self::native_resource_key)
			})
			.collect::<SmallVec<[usize; 8]>>();
		self.forget_native_resources(&retired_image_keys);
		for image in &mut self.images {
			if let Some(frame_resources) = image.frame_resources.as_mut() {
				frame_resources.resize(frames as usize, None);
			}
		}

		// A buffer with a copy per frame gets copies for new sequences and drops those of removed ones, which no queued
		// work uses now that the queues are idle.
		let mut retired_buffer_keys = SmallVec::<[usize; 8]>::new();
		for index in 0..self.buffers.len() {
			let buffer = &self.buffers[index];
			if buffer.copies != crate::Copies::PerFrame {
				continue;
			}
			let wanted = frames as usize - 1;
			let (size, host_access, uses, name) = (buffer.size, buffer.host_access, buffer.uses, buffer.name.clone());
			let added = (buffer.frame_copies.len()..wanted)
				.map(|_| self.create_buffer_memory(size, host_access, uses, name.as_deref()))
				.collect::<Vec<_>>();
			let frame_copies = &mut self.buffers[index].frame_copies;
			retired_buffer_keys.extend(
				frame_copies
					.drain(wanted.min(frame_copies.len())..)
					.filter_map(|memory| memory.resource.as_ref().map(Self::native_resource_key)),
			);
			frame_copies.extend(added);
		}
		self.forget_native_resources(&retired_buffer_keys);
		// Frame offsets resolve to other sequences under the new count, so every descriptor is written again.
		self.mark_stale_descriptors(|_, _| true);
	}

	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64 {
		let buffer = self.buffer(buffer_handle);
		buffer
			.memory
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0)
	}

	/// Gives the whole mapping of a one-copy buffer to the caller; see
	/// [`crate::context::Context::map_unsynchronized`].
	///
	/// # Safety
	///
	/// The caller must keep the buffer alive and prevent concurrent access for the lifetime of the returned mapping.
	unsafe fn map_unsynchronized<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let buffer = self.buffer(buffer_handle.into());
		crate::buffer::assert_unsynchronized_mappable(buffer.host_access, buffer.copies);
		let pointer = buffer.typed_pointer::<T>(0);
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), T::byte_count(pointer)) }
	}

	/// Applies retained descriptor writes to every frame copy of their sets.
	///
	/// Each copy keeps the write and queues its element; the native descriptor follows at the copy's next bind. Work
	/// recorded before that bind keeps reading the old descriptors: when a recording that may still run read the copy,
	/// the bind moves the copy to new heap ranges instead of overwriting them.
	///
	/// # Panics
	///
	/// Panics when a write doesn't fit its set's layout (see [`crate::descriptors::DescriptorWrite::target`]), or when
	/// a frame offset's magnitude reaches the resource's copy count.
	fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		for write in descriptor_set_writes {
			if let Some(frame_offset) = write.frame_offset {
				crate::descriptors::assert_frame_offset(self.descriptor_resource_copies(write.descriptor), frame_offset);
			}
			let (_, element) = write.target(&self.descriptor_sets[write.descriptor_set.index() as usize].layout);
			let mut current = Some(DescriptorSetHandle::from_index(write.descriptor_set.index()));
			while let Some(set_handle) = current {
				let descriptor_set = &mut self.descriptor_sets[set_handle.index() as usize];
				let retained = RetainedDescriptor {
					descriptor: write.descriptor,
					frame_offset: write.frame_offset.unwrap_or(0),
				};
				let next = descriptor_set
					.next
					.map(|handle| DescriptorSetHandle::from_index(handle.index()));
				if descriptor_set.descriptors[element as usize] != Some(retained) {
					descriptor_set.descriptors[element as usize] = Some(retained);
					descriptor_set.stale.push(element);
					Self::bound_stale_descriptors(descriptor_set);
				}
				self.materialize_descriptor_base_image_resource(set_handle, write.descriptor);
				current = next;
			}
		}
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		desired_image_count: Option<u8>,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		let extent = Self::query_window_extent(window_os_handles, fallback_extent);
		let image_count = swapchain_image_count(self.frames, desired_image_count);

		let (queue_index, queue) = self
			.queues
			.iter()
			.enumerate()
			.find(|(_, queue)| queue.workloads.intersects(WorkloadTypes::RASTER))
			.or_else(|| self.queues.first().map(|queue| (0, queue)))
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");
		let queue_handle = QueueHandle::from_index(queue_index as u64);

		let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI factory. The most likely cause is that the DXGI runtime is unavailable.");
		});

		// Tearing lets a variable-refresh display show each frame as soon as it is ready. Only immediate presentation
		// asks for it, and only where the display and driver support it.
		let allows_tearing = matches!(presentation_mode, PresentationModes::Inmediate) && supports_tearing(&factory);
		// The waitable object moves the wait for queue room from `Present` to acquisition. See `latency_waitable`.
		let flags = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT
			| if allows_tearing {
				DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
			} else {
				DXGI_SWAP_CHAIN_FLAG(0)
			};

		let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
			Width: extent.width(),
			Height: extent.height(),
			Format: DXGI_FORMAT_B8G8R8A8_UNORM,
			Stereo: false.into(),
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
			BufferCount: image_count as u32,
			Scaling: DXGI_SCALING_STRETCH,
			SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
			AlphaMode: DXGI_ALPHA_MODE_IGNORE,
			Flags: flags.0 as u32,
		};

		let swapchain = unsafe { factory.CreateSwapChainForHwnd(&queue.queue, window_os_handles.hwnd, &swapchain_desc, None, None) }.unwrap_or_else(|error| {
			panic!("Failed to create a DXGI swapchain ({error}). The most likely cause is that the window handle is invalid, the device does not support the swapchain format, or the process runs without an interactive desktop, as a service or SSH session does. Create the window from a process in an interactive session.");
		});

		let swapchain: IDXGISwapChain3 = swapchain.cast().unwrap_or_else(|_| {
			panic!(
				"Failed to upgrade the DXGI swapchain. The most likely cause is that the DXGI runtime does not support IDXGISwapChain3."
			);
		});

		let _ = unsafe { factory.MakeWindowAssociation(window_os_handles.hwnd, DXGI_MWA_NO_ALT_ENTER) };

		// SAFETY: The swapchain was created with the waitable flag, which this call requires. The returned handle is
		// owned by the caller and closed when `Owned` drops.
		let latency_waitable = unsafe { windows::core::Owned::new(swapchain.GetFrameLatencyWaitableObject()) };

		self.swapchains.push(Swapchain {
			handles: window::Handles {
				hinstance: window_os_handles.hinstance,
				hwnd: window_os_handles.hwnd,
			},
			swapchain,
			extent,
			image_count,
			desired_image_count,
			next_image_index: 0,
			present_mode: presentation_mode,
			image: None,
			proxy_uses: Uses::empty(),
			proxy_resource_uses: Uses::empty(),
			backbuffers: std::array::from_fn(|_| None),
			acquired_image_indices: [0; 8],
			acquired_sequences: [false; 8],
			queue_handle,
			present_interval: None,
			next_present_slot: None,
			flags,
			allows_tearing,
			latency_waitable,
			// No valid latency is 0, so the first acquisition sets it.
			frame_latency: 0,
			sync_interval: 1,
			refresh: None,
			shown_instant: None,
			last_target: None,
		});

		SwapchainHandle::from_index((self.swapchains.len() - 1) as u64)
	}

	/// Acquires the backbuffer that `frame` will present before the frame is started.
	///
	/// The sequence's fences are waited first so `ResizeBuffers` and backbuffer reuse see released resources,
	/// matching the guarantee `start_frame` gives for in-frame acquisition. Fences are monotonic, so the later
	/// `start_frame` wait returns immediately.
	fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		if let Some(previous) = self.last_frame_synchronizers[sequence_index as usize] {
			self.wait_for_synchronizer_sequence(previous, sequence_index);
		}
		self.wait_for_synchronizer_sequence(frame.synchronizer, sequence_index);
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	/// Waits for every native queue, then releases a swapchain, its backbuffers, and their attachment views.
	fn destroy_swapchain(&mut self, swapchain_handle: SwapchainHandle) {
		let index = swapchain_handle.index() as usize;
		if self.swapchains.get(index).is_none() {
			return;
		}
		self.idle_for_topology_change("destroying a swapchain");
		let retired_backbuffers = self.swapchains[index]
			.backbuffers
			.iter()
			.flatten()
			.map(Self::native_resource_key)
			.collect::<SmallVec<[usize; 8]>>();
		self.forget_native_resources(&retired_backbuffers);
		// Dropping the swapchain releases its backbuffers and the DXGI swapchain itself.
		self.swapchains.remove(index);
	}

	fn set_present_interval(&mut self, swapchain: SwapchainHandle, interval: Option<std::time::Duration>) {
		self.swapchains[swapchain.index() as usize].present_interval = interval;
	}

	/// A DXGI swapchain reads its size from its window, so the window's extent adds nothing.
	fn set_window_extent(&mut self, _swapchain: SwapchainHandle, _extent: Extent) {}

	/// Waits for a recorded texture copy and returns its bytes.
	fn get_image_data(
		&mut self,
		texture_copy_handle: TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		self.wait_for_texture_copy_readback(texture_copy_handle);
		self.refresh_readback_texture_copies(None);
		// Keep native storage alive while submitted work is unresolved; only completed or failed mappings are consumed.
		if self.texture_readbacks.submitted(texture_copy_handle)?.0.resource.is_some() {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		let readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		if readback.mapping_failed {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		Ok(readback.data)
	}

	/// Replaces every copy of an array buffer with zero-initialized copies of `length` elements; see
	/// [`crate::context::Context::resize_buffer`].
	///
	/// The retired copies stay alive until every frame recorded before the resize completes.
	fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>, length: usize) {
		let size = T::layout(Some(length)).size();
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let buffer = self.buffer(buffer_handle);
		let (host_access, uses, name, count) = (
			buffer.host_access,
			buffer.uses,
			buffer.name.clone(),
			buffer.copies.count(self.frames),
		);
		// D3D12 zeroes committed resources, so the new copies start zero-initialized with no clear.
		let mut storages = (0..count)
			.map(|_| self.create_buffer_memory(size, host_access, uses, name.as_deref()))
			.collect::<Vec<_>>();
		let buffer = &mut self.buffers[buffer_handle.index() as usize];
		let first = storages.remove(0);
		let retired = std::iter::once(std::mem::replace(&mut buffer.memory, first))
			.chain(std::mem::replace(&mut buffer.frame_copies, storages))
			.collect::<SmallVec<[BufferMemory; crate::MAX_FRAMES_IN_FLIGHT]>>();
		buffer.size = size;

		let mut retired_state_keys = SmallVec::<[usize; crate::MAX_FRAMES_IN_FLIGHT]>::new();
		for storage in retired {
			// Dropping the storage only unmaps its CPU view; the GPU may still use the resource, which retires below.
			if let Some(resource) = storage.resource.as_ref() {
				retired_state_keys.push(Self::native_resource_key(resource));
				self.retire_resource(resource);
			}
		}
		self.forget_native_resources(&retired_state_keys);
		self.mark_stale_descriptors(
			|descriptor, _| matches!(descriptor, WriteData::Buffer { handle } if handle == buffer_handle),
		);
	}

	fn wait_for_synchronizer(&mut self, synchronizer_handle: SynchronizerHandle) {
		for handle in self.synchronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
		self.refresh_readback_texture_copies(None);
	}

	fn wait(&mut self) {
		for index in 0..self.synchronizers.len() {
			self.wait_for_private_synchronizer(crate::synchronizer::SynchronizerHandle::from_index(index as u64));
		}
	}
}

use std::{
	alloc::Layout,
	sync::atomic::{AtomicU64, Ordering},
};

use ::utils::{
	Extent,
	hash::{HashMap, HashSet},
};
use smallvec::SmallVec;
use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::Dxc::{
	CLSID_DxcCompiler, DXC_CP_UTF8, DXC_OUT_ERRORS, DXC_OUT_OBJECT, DXC_OUT_PDB, DxcBuffer, DxcCreateInstance, IDxcBlob,
	IDxcCompiler3, IDxcIncludeHandler, IDxcResult, IDxcVersionInfo2,
};
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL_12_2, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::Win32::Graphics::Direct3D12::{
	CLSID_D3D12Debug, CLSID_D3D12SDKConfiguration, D3D_ROOT_SIGNATURE_VERSION_1_2, D3D12_BARRIER_ACCESS,
	D3D12_BARRIER_ACCESS_COMMON, D3D12_BARRIER_ACCESS_CONSTANT_BUFFER, D3D12_BARRIER_ACCESS_COPY_DEST,
	D3D12_BARRIER_ACCESS_COPY_SOURCE, D3D12_BARRIER_ACCESS_DEPTH_STENCIL_WRITE, D3D12_BARRIER_ACCESS_INDEX_BUFFER,
	D3D12_BARRIER_ACCESS_INDIRECT_ARGUMENT, D3D12_BARRIER_ACCESS_NO_ACCESS, D3D12_BARRIER_ACCESS_RENDER_TARGET,
	D3D12_BARRIER_ACCESS_SHADER_RESOURCE, D3D12_BARRIER_ACCESS_UNORDERED_ACCESS, D3D12_BARRIER_ACCESS_VERTEX_BUFFER,
	D3D12_BARRIER_GROUP, D3D12_BARRIER_GROUP_0, D3D12_BARRIER_LAYOUT, D3D12_BARRIER_LAYOUT_COMMON,
	D3D12_BARRIER_LAYOUT_DEPTH_STENCIL_WRITE, D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_COMMON, D3D12_BARRIER_LAYOUT_PRESENT,
	D3D12_BARRIER_LAYOUT_RENDER_TARGET, D3D12_BARRIER_LAYOUT_SHADER_RESOURCE, D3D12_BARRIER_LAYOUT_UNDEFINED,
	D3D12_BARRIER_LAYOUT_UNORDERED_ACCESS, D3D12_BARRIER_SUBRESOURCE_RANGE, D3D12_BARRIER_SYNC, D3D12_BARRIER_SYNC_ALL,
	D3D12_BARRIER_SYNC_ALL_SHADING, D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW, D3D12_BARRIER_SYNC_COMPUTE_SHADING,
	D3D12_BARRIER_SYNC_COPY, D3D12_BARRIER_SYNC_DEPTH_STENCIL, D3D12_BARRIER_SYNC_EXECUTE_INDIRECT,
	D3D12_BARRIER_SYNC_INDEX_INPUT, D3D12_BARRIER_SYNC_NONE, D3D12_BARRIER_SYNC_RENDER_TARGET,
	D3D12_BARRIER_SYNC_VERTEX_SHADING, D3D12_BARRIER_TYPE_BUFFER, D3D12_BARRIER_TYPE_GLOBAL, D3D12_BARRIER_TYPE_TEXTURE,
	D3D12_BLEND_DESC, D3D12_BLEND_INV_SRC_ALPHA, D3D12_BLEND_ONE, D3D12_BLEND_OP_ADD, D3D12_BLEND_SRC_ALPHA, D3D12_BLEND_ZERO,
	D3D12_BUFFER_BARRIER, D3D12_BUFFER_SRV, D3D12_BUFFER_SRV_FLAG_NONE, D3D12_BUFFER_UAV, D3D12_BUFFER_UAV_FLAG_NONE,
	D3D12_BUFFER_UAV_FLAG_RAW, D3D12_CACHED_PIPELINE_STATE, D3D12_CLEAR_FLAG_DEPTH, D3D12_CLEAR_VALUE, D3D12_CLEAR_VALUE_0,
	D3D12_COLOR_WRITE_ENABLE_ALL, D3D12_COMMAND_LIST_FLAG_NONE, D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC,
	D3D12_COMMAND_SIGNATURE_DESC, D3D12_COMPARISON_FUNC_ALWAYS, D3D12_COMPARISON_FUNC_GREATER_EQUAL,
	D3D12_COMPARISON_FUNC_NEVER, D3D12_COMPUTE_PIPELINE_STATE_DESC, D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
	D3D12_CONSTANT_BUFFER_VIEW_DESC, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_BACK,
	D3D12_CULL_MODE_FRONT, D3D12_CULL_MODE_NONE, D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT,
	D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, D3D12_DEPTH_STENCIL_DESC,
	D3D12_DEPTH_STENCIL_VALUE, D3D12_DEPTH_STENCIL_VIEW_DESC, D3D12_DEPTH_STENCIL_VIEW_DESC_0, D3D12_DEPTH_STENCILOP_DESC,
	D3D12_DEPTH_WRITE_MASK_ALL, D3D12_DEPTH_WRITE_MASK_ZERO, D3D12_DESCRIPTOR_HEAP_DESC,
	D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
	D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, D3D12_DSV_DIMENSION_TEXTURE2D,
	D3D12_DSV_DIMENSION_TEXTURE2DARRAY, D3D12_DSV_FLAG_NONE, D3D12_FEATURE_D3D12_OPTIONS, D3D12_FEATURE_D3D12_OPTIONS4,
	D3D12_FEATURE_D3D12_OPTIONS12, D3D12_FEATURE_DATA_D3D12_OPTIONS, D3D12_FEATURE_DATA_D3D12_OPTIONS4,
	D3D12_FEATURE_DATA_D3D12_OPTIONS12, D3D12_FEATURE_DATA_SHADER_MODEL, D3D12_FEATURE_SHADER_MODEL, D3D12_FENCE_FLAGS,
	D3D12_FILL_MODE, D3D12_FILL_MODE_SOLID, D3D12_FILL_MODE_WIREFRAME, D3D12_FILTER, D3D12_FILTER_ANISOTROPIC,
	D3D12_FILTER_MAXIMUM_ANISOTROPIC, D3D12_FILTER_MIN_MAG_MIP_LINEAR, D3D12_FILTER_MINIMUM_ANISOTROPIC,
	D3D12_FILTER_REDUCTION_TYPE_SHIFT, D3D12_FLOAT32_MAX, D3D12_GLOBAL_BARRIER, D3D12_GPU_BASED_VALIDATION_FLAGS_NONE,
	D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_GRAPHICS_PIPELINE_STATE_DESC, D3D12_HEAP_DESC,
	D3D12_HEAP_FLAG_ALLOW_ALL_BUFFERS_AND_TEXTURES, D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
	D3D12_HEAP_FLAG_ALLOW_ONLY_RT_DS_TEXTURES, D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT,
	D3D12_HEAP_TYPE_UPLOAD, D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED, D3D12_INDEX_BUFFER_VIEW, D3D12_INDIRECT_ARGUMENT_DESC,
	D3D12_INDIRECT_ARGUMENT_DESC_0, D3D12_INDIRECT_ARGUMENT_TYPE, D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH,
	D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH_MESH, D3D12_INDIRECT_ARGUMENT_TYPE_DRAW, D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
	D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC, D3D12_LOGIC_OP_NOOP, D3D12_MAG_FILTER_SHIFT, D3D12_MEMORY_POOL_UNKNOWN,
	D3D12_MESSAGE, D3D12_MESSAGE_SEVERITY_CORRUPTION, D3D12_MESSAGE_SEVERITY_ERROR, D3D12_MIN_FILTER_SHIFT,
	D3D12_MIP_FILTER_SHIFT, D3D12_PIPELINE_STATE_FLAG_NONE, D3D12_PIPELINE_STATE_FLAGS, D3D12_PIPELINE_STATE_STREAM_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK, D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
	D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RANGE, D3D12_RASTERIZER_DESC, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV,
	D3D12_RENDER_TARGET_BLEND_DESC, D3D12_RENDER_TARGET_VIEW_DESC, D3D12_RENDER_TARGET_VIEW_DESC_0, D3D12_RESOURCE_DESC1,
	D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_DIMENSION_TEXTURE3D,
	D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
	D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_FLAGS, D3D12_RESOURCE_HEAP_TIER,
	D3D12_RESOURCE_HEAP_TIER_1, D3D12_ROOT_CONSTANTS, D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS, D3D12_ROOT_PARAMETER1,
	D3D12_ROOT_PARAMETER1_0, D3D12_ROOT_SIGNATURE_DESC2, D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
	D3D12_ROOT_SIGNATURE_FLAG_CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED, D3D12_ROOT_SIGNATURE_FLAG_SAMPLER_HEAP_DIRECTLY_INDEXED,
	D3D12_RT_FORMAT_ARRAY, D3D12_RTV_DIMENSION_TEXTURE2D, D3D12_RTV_DIMENSION_TEXTURE2DARRAY, D3D12_SAMPLER_DESC,
	D3D12_SHADER_BYTECODE, D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0, D3D12_SHADER_VISIBILITY,
	D3D12_SHADER_VISIBILITY_ALL, D3D12_SHADER_VISIBILITY_AMPLIFICATION, D3D12_SHADER_VISIBILITY_MESH,
	D3D12_SHADER_VISIBILITY_PIXEL, D3D12_SHADER_VISIBILITY_VERTEX, D3D12_SRV_DIMENSION_BUFFER,
	D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_SRV_DIMENSION_TEXTURE2D, D3D12_SRV_DIMENSION_TEXTURE2DARRAY,
	D3D12_SRV_DIMENSION_TEXTURE3D, D3D12_SRV_DIMENSION_TEXTURECUBE, D3D12_SRV_DIMENSION_TEXTURECUBEARRAY,
	D3D12_STENCIL_OP_KEEP, D3D12_TEX2D_ARRAY_DSV, D3D12_TEX2D_ARRAY_RTV, D3D12_TEX2D_ARRAY_SRV, D3D12_TEX2D_ARRAY_UAV,
	D3D12_TEX2D_DSV, D3D12_TEX2D_RTV, D3D12_TEX2D_SRV, D3D12_TEX2D_UAV, D3D12_TEX3D_SRV, D3D12_TEX3D_UAV,
	D3D12_TEXCUBE_ARRAY_SRV, D3D12_TEXCUBE_SRV, D3D12_TEXTURE_ADDRESS_MODE, D3D12_TEXTURE_ADDRESS_MODE_BORDER,
	D3D12_TEXTURE_ADDRESS_MODE_CLAMP, D3D12_TEXTURE_ADDRESS_MODE_MIRROR, D3D12_TEXTURE_ADDRESS_MODE_WRAP,
	D3D12_TEXTURE_BARRIER, D3D12_TEXTURE_BARRIER_FLAG_DISCARD, D3D12_TEXTURE_BARRIER_FLAG_NONE, D3D12_TEXTURE_COPY_LOCATION,
	D3D12_TEXTURE_COPY_LOCATION_0, D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
	D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_UAV_DIMENSION_BUFFER, D3D12_UAV_DIMENSION_TEXTURE2D,
	D3D12_UAV_DIMENSION_TEXTURE2DARRAY, D3D12_UAV_DIMENSION_TEXTURE3D, D3D12_UNORDERED_ACCESS_VIEW_DESC,
	D3D12_UNORDERED_ACCESS_VIEW_DESC_0, D3D12_VERSIONED_ROOT_SIGNATURE_DESC, D3D12_VERSIONED_ROOT_SIGNATURE_DESC_0,
	D3D12_VERTEX_BUFFER_VIEW, D3D12_VIEWPORT, D3D12GetInterface, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue,
	ID3D12CommandSignature, ID3D12Debug, ID3D12Debug3, ID3D12DescriptorHeap, ID3D12Device10, ID3D12DeviceConfiguration,
	ID3D12DeviceFactory, ID3D12Fence, ID3D12GraphicsCommandList7, ID3D12Heap, ID3D12InfoQueue, ID3D12PipelineState,
	ID3D12ProtectedResourceSession, ID3D12Resource, ID3D12RootSignature, ID3D12SDKConfiguration1,
};
use windows::Win32::Graphics::Dxgi::Common::{
	DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_BC5_SNORM, DXGI_FORMAT_BC5_UNORM,
	DXGI_FORMAT_BC7_UNORM, DXGI_FORMAT_BC7_UNORM_SRGB, DXGI_FORMAT_D16_UNORM, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8_SNORM,
	DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_SNORM, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8G8B8A8_SNORM,
	DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R16_FLOAT, DXGI_FORMAT_R16_SNORM,
	DXGI_FORMAT_R16_TYPELESS, DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R16G16_FLOAT, DXGI_FORMAT_R16G16_SNORM,
	DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R16G16B16A16_SNORM, DXGI_FORMAT_R16G16B16A16_UNORM,
	DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32_SINT, DXGI_FORMAT_R32_TYPELESS, DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT,
	DXGI_FORMAT_R32G32_SINT, DXGI_FORMAT_R32G32_UINT, DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32B32_SINT,
	DXGI_FORMAT_R32G32B32_UINT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32A32_SINT, DXGI_FORMAT_R32G32B32A32_UINT,
	DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
	CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_FEATURE_PRESENT_ALLOW_TEARING, DXGI_FRAME_STATISTICS,
	DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
	DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
	DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory4, IDXGIFactory5, IDXGISwapChain3,
};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{BOOL, IUnknown, Interface, PCSTR, PCWSTR};

use self::presentation::supports_tearing;
use super::utils;
use crate::context::{Context as _, ContextCreate as _};
use crate::dx12::acceleration_structures::{
	BottomLevel, BuildState, INSTANCE_SIZE, TopLevel, TopLevelCopy, build_inputs, triangle_geometry,
};
use crate::dx12::ray_tracing::{
	CALLABLE_OFFSET, IDENTIFIER_SIZE, RECORD_STRIDE, ShaderBindingTable, WrittenTable, fill_shader_binding_table, table_size,
};
use crate::rt;
use crate::{
	AttachmentInformation, BaseBufferHandle, BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferHandle,
	ClearValue, CommandBufferHandle, DataTypes, DescriptorSetHandle, DispatchExtent, FilteringModes, Formats, HandleLike as _,
	ImageHandle, ImageOrSwapchain, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey, PresentationModes,
	QueueHandle, QueueSelection, RGBAu8, SamplerAddressingModes, SamplerHandle, SamplingReductionModes, ShaderHandle,
	ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TextureReadback as MappedTextureReadback,
	TextureViewTypes, TopLevelAccelerationStructureHandle, Uses, WorkloadTypes, buffer,
	descriptors::{DescriptorWrite, WriteData},
	device::Features,
	image,
	pipelines::{self, PushConstantRange, VertexElement},
	sampler::{self, FIXED_SAMPLER_COUNT},
	shader::{ResourceKind, ShaderResourceDescriptor, Sources},
	window,
};
