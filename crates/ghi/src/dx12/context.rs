/// The `Device` struct exists to own DX12 GPU resources for the shared GHI device API.
pub struct Device {
	device: ID3D12Device,
	// Descriptor strides are immutable for the lifetime of an ID3D12Device, so query them once.
	descriptor_handle_increment_sizes: [u32; 4],
	settings: Features,
	native_16_bit_shader_ops_supported: bool,
	info_queue: Option<ID3D12InfoQueue>,
	debug_log_function: fn(&str),
	debug_log_count: AtomicU64,
	debugger: RenderDebugger,
	pub(crate) frames: u8,

	queues: Vec<StoredQueue>,
	command_buffers: Vec<CommandBuffer>,
	buffers: Vec<Buffer>,
	dynamic_buffers: Vec<Buffer>,
	images: Vec<Image>,
	samplers: Vec<Sampler>,
	descriptor_sets: Vec<DescriptorSet>,
	descriptor_materializations: HashMap<DescriptorMaterializationKey, DescriptorMaterialization>,
	pipeline_layouts: Vec<PipelineLayout>,
	pipeline_root_signatures: Vec<Option<ID3D12RootSignature>>,
	pipeline_root_tables: Vec<Vec<RootDescriptorTable>>,
	pipeline_root_constants: Vec<Vec<RootConstantRange>>,
	pipeline_layout_indices: HashMap<PipelineLayout, PipelineLayoutHandle>,
	pub(crate) pipelines: Vec<Pipeline>,
	indirect_dispatch_signature: Option<ID3D12CommandSignature>,
	shaders: Vec<Shader>,
	meshes: Vec<Mesh>,
	pub(crate) swapchains: Vec<Swapchain>,
	synchronizers: Vec<Synchronizer>,
	top_level_acceleration_structures: Vec<AccelerationStructure>,
	bottom_level_acceleration_structures: Vec<AccelerationStructure>,
	allocations: Vec<Allocation>,
	texture_readbacks: crate::context::TextureReadbackRegistry<TextureReadback>,
	gpu_uploaded_images: HashSet<crate::BaseImageHandle>,
	pending_texture_syncs: Vec<(crate::BaseImageHandle, u8)>,
	present_transitions: HashMap<CommandBufferHandle, Vec<ID3D12Resource>>,
	render_target_views: HashMap<AttachmentViewKey, CpuDescriptorView>,
	depth_stencil_views: HashMap<AttachmentViewKey, CpuDescriptorView>,
	retained_clear_uav_descriptors: HashMap<usize, RetainedCpuDescriptor>,
	clear_uav_descriptor_pages: Vec<DescriptorHeapArena>,
	free_clear_uav_descriptor_slots: Vec<(usize, u32)>,
	buffer_states: HashMap<usize, D3D12_RESOURCE_STATES>,
	image_states: HashMap<usize, D3D12_RESOURCE_STATES>,
	render_target_view_allocation_count: usize,
	depth_stencil_view_allocation_count: usize,
	texture_copy_count: usize,
	buffer_copy_count: usize,
	buffer_clear_count: usize,
	clear_descriptor_copy_call_count: usize,
	native_command_list_execute_count: usize,
	empty_command_list_skip_count: usize,
	root_signature_bind_count: usize,
	descriptor_heap_bind_count: usize,
	descriptor_table_bind_count: usize,
	#[cfg(test)]
	descriptor_table_bind_records: Vec<DescriptorTableBindRecord>,
	push_constant_write_count: usize,
	#[cfg(test)]
	push_constant_write_records: Vec<PushConstantWriteRecord>,
	descriptor_write_count: usize,
	image_srv_descriptor_write_count: usize,
	image_uav_descriptor_write_count: usize,
	acceleration_structure_descriptor_write_count: usize,
	#[cfg(test)]
	sampler_descriptor_write_records: Vec<SamplerDescriptorWriteRecord>,
	pipeline_state_bind_count: usize,
	compute_pipeline_state_create_attempt_count: usize,
	graphics_pipeline_state_create_attempt_count: usize,
	graphics_pipeline_state_last_error: Option<i32>,
	hlsl_specialization_compile_count: usize,
	ray_tracing_state_object_create_attempt_count: usize,
	compute_dispatch_encode_count: usize,
	indirect_dispatch_encode_count: usize,
	trace_rays_record_count: usize,
	mesh_dispatch_encode_count: usize,
	vertex_buffer_bind_count: usize,
	index_buffer_bind_count: usize,
	draw_encode_count: usize,
	draw_indexed_encode_count: usize,
	render_target_bind_count: usize,
	render_target_clear_count: usize,
	render_pass_end_count: usize,
	depth_stencil_bind_count: usize,
	depth_stencil_clear_count: usize,
	viewport_set_count: usize,
	scissor_set_count: usize,
	primitive_topology_set_count: usize,
	swapchain_backbuffer_bind_count: usize,
	swapchain_present_transition_count: usize,
	uav_barrier_count: usize,
	acceleration_structure_resource_count: usize,
	native_acceleration_structure_resource_count: usize,
	acceleration_structure_instance_write_count: usize,
	shader_binding_table_write_count: usize,
	top_level_acceleration_structure_build_record_count: usize,
	bottom_level_acceleration_structure_build_record_count: usize,
	native_top_level_acceleration_structure_build_encode_count: usize,
	native_bottom_level_acceleration_structure_build_encode_count: usize,
	texture_readback_resolve_count: usize,
	debug_region_begin_count: Cell<usize>,
	debug_region_end_count: Cell<usize>,
}

#[path = "context/commands.rs"]
mod device_commands;
#[path = "context/descriptors.rs"]
mod device_descriptors;
#[path = "context/initialization.rs"]
mod device_initialization;
#[path = "context/pipelines.rs"]
mod device_pipelines;
#[path = "context/presentation.rs"]
mod device_presentation;
#[path = "context/resources.rs"]
mod device_resources;
#[path = "context/transfers.rs"]
mod device_transfers;

const DYNAMIC_BUFFER_HANDLE_FLAG: u64 = 1 << 63;

#[derive(Clone)]
pub(crate) struct StoredQueue {
	queue: ID3D12CommandQueue,
	queue_type: D3D12_COMMAND_LIST_TYPE,
}

pub(crate) fn select_d3d12_command_list_type(requested: WorkloadTypes) -> Result<D3D12_COMMAND_LIST_TYPE, &'static str> {
	if requested.is_empty() {
		return Err("Invalid workload type");
	}

	if requested.intersects(WorkloadTypes::VIDEO) {
		return Err("D3D12 video queues are not exposed through this backend command-buffer path.");
	}

	if requested.intersects(WorkloadTypes::IO) {
		return Err("D3D12 IO queues are not exposed through this backend command-buffer path.");
	}

	if requested.intersects(WorkloadTypes::RASTER | WorkloadTypes::RAY_TRACING) {
		return Ok(D3D12_COMMAND_LIST_TYPE_DIRECT);
	}

	if requested.intersects(WorkloadTypes::COMPUTE) {
		return Ok(D3D12_COMMAND_LIST_TYPE_COMPUTE);
	}

	if requested.intersects(WorkloadTypes::TRANSFER) {
		return Ok(D3D12_COMMAND_LIST_TYPE_COPY);
	}

	Err("Invalid workload type")
}

/// The `CommandBuffer` struct owns reusable native recording state for one shared command-buffer handle.
struct CommandBuffer {
	queue_handle: QueueHandle,
	allocator: Option<ID3D12CommandAllocator>,
	command_list: Option<ID3D12GraphicsCommandList>,
	pending_clear_descriptor_copies: Vec<PendingDescriptorCopy>,
	prepared_clear_descriptors: Vec<PreparedClearDescriptor>,
	retained_descriptor_heaps: Vec<ID3D12DescriptorHeap>,
	retained_resources: Vec<ID3D12Resource>,
	retained_upload_resource_count: usize,
	cbv_srv_uav_staging_heap: Option<DescriptorHeapArena>,
	sampler_staging_heap: Option<DescriptorHeapArena>,
	is_open: bool,
	recorded_work: bool,
	sequence_index: u8,
	last_submission: Option<(SynchronizerHandle, u8)>,
}

/// The `PendingDescriptorCopy` struct identifies one deferred retained-to-visible descriptor write.
struct PendingDescriptorCopy {
	destination: D3D12_CPU_DESCRIPTOR_HANDLE,
	source: D3D12_CPU_DESCRIPTOR_HANDLE,
}

/// The `PreparedClearDescriptor` struct carries matching CPU and GPU handles into one recorded UAV clear.
struct PreparedClearDescriptor {
	resource: usize,
	cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
	gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
}

/// The `DescriptorHeap` struct caches native heap starts for allocation-free handle arithmetic.
#[derive(Clone)]
struct DescriptorHeap {
	native: ID3D12DescriptorHeap,
	cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
	gpu_start: Option<D3D12_GPU_DESCRIPTOR_HANDLE>,
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

/// The `CpuDescriptorView` struct retains native attachment descriptors for reuse across frames.
struct CpuDescriptorView {
	heap: DescriptorHeap,
}

/// The `RenderTargetAttachment` struct carries one resolved color attachment through native binding.
struct RenderTargetAttachment {
	image_handle: Option<crate::BaseImageHandle>,
	resource: ID3D12Resource,
	format: Formats,
	array_layers: u32,
	layer: Option<u32>,
	layer_count: u32,
	load: bool,
	clear: ClearValue,
	swapchain_backbuffer: bool,
}

pub(crate) struct Buffer {
	data: *mut u8,
	layout: Layout,
	size: usize,
	uses: Uses,
	access: DeviceAccesses,
	resource: Option<ID3D12Resource>,
	mapped: *mut u8,
	heap_kind: BufferHeapKind,
	frame_resources: Option<Vec<Option<BufferFrameStorage>>>,
}

/// The `BufferFrameStorage` struct provides lazy frame-local backing storage for dynamic DX12 buffers.
struct BufferFrameStorage {
	data: *mut u8,
	layout: Layout,
	resource: Option<ID3D12Resource>,
	mapped: *mut u8,
	heap_kind: BufferHeapKind,
}

enum BufferStorage {
	Static,
	Dynamic,
}

struct BufferCopyInfo {
	resource: ID3D12Resource,
	access: DeviceAccesses,
	heap_kind: BufferHeapKind,
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

/// The `TextureReadbackData` struct owns one completed DX12 texture-transfer result.
struct TextureReadbackData {
	bytes: Vec<u8>,
	extent: Extent,
	format: Formats,
	bytes_per_row: usize,
	bytes_per_image: usize,
}

/// The `TextureReadback` struct keeps one DX12 transfer result and optional native staging alive until consumption.
struct TextureReadback {
	command_buffer_handle: Option<CommandBufferHandle>,
	completion: Option<(crate::synchronizer::SynchronizerHandle, u64)>,
	resource: Option<ID3D12Resource>,
	sequence_index: u8,
	row_pitch: usize,
	row_bytes: usize,
	height: usize,
	depth: usize,
	size: usize,
	mapping_failed: bool,
	data: TextureReadbackData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferHeapKind {
	Default,
	Upload,
	Readback,
}

impl Drop for Buffer {
	fn drop(&mut self) {
		if self.heap_kind != BufferHeapKind::Default && !self.mapped.is_null() {
			if let Some(resource) = self.resource.as_ref() {
				unsafe {
					resource.Unmap(0, None);
				}
			}
		}
		if self.layout.size() == 0 {
			return;
		}
		if !self.data.is_null() {
			unsafe {
				alloc::dealloc(self.data, self.layout);
			}
		}
	}
}

impl Drop for BufferFrameStorage {
	fn drop(&mut self) {
		if self.heap_kind != BufferHeapKind::Default && !self.mapped.is_null() {
			if let Some(resource) = self.resource.as_ref() {
				unsafe {
					resource.Unmap(0, None);
				}
			}
		}
		if self.layout.size() == 0 {
			return;
		}
		if !self.data.is_null() {
			unsafe {
				alloc::dealloc(self.data, self.layout);
			}
		}
	}
}

pub(crate) struct Image {
	extent: Extent,
	format: Formats,
	uses: Uses,
	access: DeviceAccesses,
	array_layers: u32,
	mip_levels: u32,
	resource: Option<ID3D12Resource>,
	data: Option<Vec<u8>>,
	frame_data: Option<Vec<Vec<u8>>>,
	frame_resources: Option<Vec<Option<ID3D12Resource>>>,
	optimized_clear_value: Option<D3D12_CLEAR_VALUE>,
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

/// The `DescriptorSet` struct retains one frame's logical resource writes and native snapshot version.
pub(crate) struct DescriptorSet {
	pub(crate) next: Option<crate::descriptors::DescriptorSetHandle>,
	version: u64,
	descriptors: HashMap<ResourceSlot, HashMap<u32, RetainedDescriptor>>,
}

/// The `DescriptorMaterializationKey` struct identifies one frame-resolved set union for a pipeline layout.
#[derive(Clone, PartialEq, Eq, Hash)]
struct DescriptorMaterializationKey {
	layout: PipelineLayoutHandle,
	descriptor_sets: SmallVec<[DescriptorSetHandle; 8]>,
	sequence_index: u8,
}

/// The `DescriptorMaterialization` struct retains immutable shader-visible heaps until its logical sets change.
#[derive(Clone)]
struct DescriptorMaterialization {
	versions: SmallVec<[u64; 8]>,
	cbv_srv_uav_heap: Option<DescriptorHeap>,
	sampler_heap: Option<DescriptorHeap>,
}

/// The `Binding` struct preserves the private handle item required by shared legacy exports.
pub(crate) struct Binding {
	pub(crate) next: Option<crate::binding::DescriptorSetBindingHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct RetainedDescriptor {
	descriptor: WriteData,
	frame_offset: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineResource {
	descriptor: ShaderResourceDescriptor,
	cbv_srv_uav_offset: Option<u32>,
	sampler_offset: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayout {
	resources: Vec<PipelineResource>,
	cbv_srv_uav_descriptor_count: u32,
	sampler_descriptor_count: u32,
	push_constant_ranges: Vec<PushConstantRange>,
}

#[derive(Clone)]
struct RootDescriptorTable {
	root_parameter_index: u32,
	sampler_heap: bool,
}

#[derive(Clone, Copy)]
struct RootConstantRange {
	root_parameter_index: u32,
	offset: u32,
	size: u32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorTableBindRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) set_index: usize,
	pub(crate) binding_index: u32,
	pub(crate) sampler_heap: bool,
	pub(crate) heap_slot: u32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PushConstantWriteRecord {
	pub(crate) root_parameter_index: u32,
	pub(crate) offset: u32,
	pub(crate) size: u32,
	pub(crate) compute_root: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SamplerDescriptorWriteRecord {
	pub(crate) filter: D3D12_FILTER,
	pub(crate) address_mode: D3D12_TEXTURE_ADDRESS_MODE,
	pub(crate) max_anisotropy: u32,
	pub(crate) min_lod: f32,
	pub(crate) max_lod: f32,
}

pub(crate) struct Pipeline {
	pub(crate) layout: PipelineLayoutHandle,
	shaders: Vec<ShaderHandle>,
	kind: PipelineKind,
	pipeline_state: Option<ID3D12PipelineState>,
	ray_tracing_state_object: Option<ID3D12StateObject>,
	ray_tracing_shader_identifiers: HashMap<ShaderHandle, [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize]>,
	has_mesh_shader: bool,
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

#[derive(Clone, Copy)]
enum PipelineKind {
	Raster,
	Compute,
	RayTracing,
}

struct Shader {
	stage: ShaderTypes,
	spirv: Option<Vec<u8>>,
	dxil: Option<Vec<u8>>,
	hlsl: Option<HlslSource>,
	resources: Vec<ShaderResourceDescriptor>,
}

#[derive(Clone)]
struct HlslSource {
	name: Option<String>,
	source: String,
	entry_point: String,
}

struct Mesh {
	vertex_count: u32,
	index_count: u32,
	vertices: Vec<u8>,
	indices: Vec<u8>,
	vertex_size: usize,
	vertex_resource: Option<ID3D12Resource>,
	index_resource: Option<ID3D12Resource>,
}

pub(crate) struct Swapchain {
	handles: window::Handles,
	swapchain: IDXGISwapChain3,
	extent: Extent,
	image_count: u8,
	next_image_index: u8,
	present_mode: PresentationModes,
	images: [Option<ImageHandle>; 8],
	proxy_uses: [Uses; 8],
	backbuffers: [Option<ID3D12Resource>; 8],
	pub(crate) acquired_image_indices: [u8; 8],
}

pub(crate) struct Synchronizer {
	pub(crate) next: Option<crate::synchronizer::SynchronizerHandle>,
	fence: ID3D12Fence,
	value: u64,
}

struct Allocation {
	data: Vec<u8>,
}

struct AccelerationStructure {
	resource: Option<ID3D12Resource>,
	size: usize,
	native_resource: bool,
}

fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
	(c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])
}

fn wide_null(value: &str) -> Vec<u16> {
	value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The `Execution` struct exists to collect frame-scoped DX12 command recordings for a queue submission.
pub struct Execution<'a> {
	pub(crate) frame: Option<super::Frame<'a>>,
	pub(crate) completed_frame: Option<crate::FrameKey>,
	pub(crate) command_buffers: smallvec::SmallVec<[CommandBufferHandle; 4]>,
}

impl Drop for Execution<'_> {
	fn drop(&mut self) {
		let Some(frame) = self.frame.as_mut() else {
			return;
		};
		for &command_buffer in &self.command_buffers {
			frame
				.device_mut()
				.abandon_texture_readbacks_for_command_buffer(command_buffer);
		}
	}
}

/// The `CommandBufferReference` struct exists to start DX12 command-buffer recordings from a command-buffer handle.
pub struct CommandBufferReference<'a> {
	device: &'a mut Device,
	command_buffer_handle: CommandBufferHandle,
}

impl crate::command_buffer::CommandBuffer for CommandBufferReference<'_> {
	fn create_command_buffer_recording(
		&mut self,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		self.device.create_command_buffer_recording(self.command_buffer_handle)
	}
}

impl crate::device::Device for Device {
	type Context = Device;
	type Allocator = std::alloc::Global;
	type RasterPipeline = crate::dx12::factory::RasterPipeline;
	type ComputePipeline = crate::dx12::factory::ComputePipeline;
	type Image = crate::dx12::factory::FactoryImage;
	type Sampler = crate::dx12::factory::FactorySampler;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Ok(Device::from_native_parts(
			self.device.clone(),
			self.settings,
			self.info_queue.clone(),
			self.debug_log_function,
			self.queues.clone(),
		))
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		_shader_source_type: Sources,
		_stage: ShaderTypes,
		_shader_resource_descriptors: impl IntoIterator<Item = ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()> {
		panic!(
			"DX12 detached shader creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn create_raster_pipeline(&mut self, _builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		panic!(
			"DX12 detached raster pipeline creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn create_compute_pipeline(&mut self, _builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		panic!(
			"DX12 detached compute pipeline creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn build_image(&mut self, _builder: crate::image::Builder) -> Self::Image {
		panic!(
			"DX12 detached image creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}

	fn build_sampler(&mut self, _builder: crate::sampler::Builder) -> Self::Sampler {
		panic!(
			"DX12 detached sampler creation requires a detached device. The most likely cause is using the primary device after moving resource creation into the Device trait."
		)
	}
}

impl crate::context::ContextCreate for Device {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: Uses,
		resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle {
		Device::create_allocation(self, size, resource_uses, resource_device_accesses)
	}
	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[VertexElement],
	) -> MeshHandle {
		Device::add_mesh_from_vertices_and_indices(self, vertex_count, index_count, vertices, indices, vertex_layout)
	}
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()> {
		Device::create_shader(self, name, shader_source_type, stage, shader_resource_descriptors)
	}
	fn create_descriptor_set(&mut self, name: Option<&str>) -> DescriptorSetHandle {
		Device::create_descriptor_set(self, name)
	}
	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> PipelineHandle {
		Device::create_raster_pipeline(self, builder)
	}
	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> PipelineHandle {
		Device::create_compute_pipeline(self, builder)
	}
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> PipelineHandle {
		Device::create_ray_tracing_pipeline(self, builder)
	}
	fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		Device::build_buffer(self, builder)
	}
	fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T> {
		Device::build_dynamic_buffer(self, builder)
	}
	fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		Device::build_dynamic_image(self, builder)
	}
	fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		Device::build_image(self, builder)
	}
	fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
		Device::build_sampler(self, builder)
	}
	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle {
		Device::create_acceleration_structure_instance_buffer(self, name, max_instance_count)
	}
	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		Device::create_top_level_acceleration_structure(self, name, max_instance_count)
	}
	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		Device::create_bottom_level_acceleration_structure(self, description)
	}
	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		Device::create_synchronizer(self, name, signaled)
	}
}

impl crate::context::Context for Device {
	type Queue<'a> = super::queue::Queue<'a>;
	type CommandBuffer<'a> = CommandBufferReference<'a>;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		Device::has_errors(self)
	}

	fn supports_bc_texture_compression(&self) -> bool {
		true
	}

	fn queue<'a>(&'a mut self, queue_handle: QueueHandle) -> Self::Queue<'a> {
		super::queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn command_buffer<'a>(&'a mut self, command_buffer_handle: CommandBufferHandle) -> Self::CommandBuffer<'a> {
		CommandBufferReference {
			device: self,
			command_buffer_handle,
		}
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		Device::set_frames_in_flight(self, frames);
	}

	fn get_buffer_address(&self, buffer_handle: BaseBufferHandle) -> u64 {
		Device::get_buffer_address(self, buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		Device::get_buffer_slice(self, buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &mut T {
		Device::get_mut_buffer_slice(self, buffer_handle)
	}

	unsafe fn transfer_buffer_mapping<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> crate::buffer::Mapping {
		unsafe { Device::transfer_buffer_mapping(self, buffer_handle) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>) {
		Device::sync_buffer(self, buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: ImageHandle) -> &mut [u8] {
		self.texture_slice_mut_static(texture_handle.0)
	}

	fn sync_texture(&mut self, image_handle: ImageHandle) {
		self.queue_texture_sync_for_sequence(image_handle.0, 0);
	}

	fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		Device::write_texture(self, texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		Device::write(self, descriptor_set_writes);
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
		Device::write_instance(
			self,
			instances_buffer_handle,
			instance_index,
			transform,
			custom_index,
			mask,
			sbt_record_offset,
			acceleration_structure,
		);
	}

	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	) {
		Device::write_sbt_entry(self, sbt_buffer_handle, sbt_record_offset, pipeline_handle, shader_handle);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		Device::bind_to_window(self, window_os_handles, presentation_mode, fallback_extent, _uses)
	}

	fn get_image_data(
		&mut self,
		texture_copy_handle: TextureCopyHandle,
	) -> Result<MappedTextureReadback, TextureTransferError> {
		self.wait_for_texture_copy_readback(texture_copy_handle);
		self.refresh_readback_texture_copies(None);
		Device::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		Device::resize_buffer(self, buffer_handle, size);
	}

	fn start_frame_capture(&mut self) {
		Device::start_frame_capture(self);
	}

	fn end_frame_capture(&mut self) {
		Device::end_frame_capture(self);
	}

	fn wait_for_synchronizer(&mut self, synchronizer: SynchronizerHandle) {
		Device::wait_for_synchronizer(self, synchronizer);
	}

	fn wait(&mut self) {
		Device::wait(self);
	}
}

use std::{
	alloc::{self, Layout},
	cell::Cell,
	sync::atomic::{AtomicU64, Ordering},
};

use ::utils::Extent;
use ::utils::hash::{HashMap, HashSet};
use smallvec::SmallVec;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Dxc::{
	CLSID_DxcCompiler, DXC_CP_UTF8, DXC_OUT_ERRORS, DXC_OUT_OBJECT, DXC_OUT_PDB, DxcBuffer, DxcCreateInstance, IDxcBlob,
	IDxcCompiler3, IDxcIncludeHandler, IDxcResult,
};
use windows::Win32::Graphics::Direct3D::{
	D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_SHADER_MACRO, Fxc::D3DCompile,
	ID3DInclude,
};
use windows::Win32::Graphics::Direct3D12::{
	D3D_ROOT_SIGNATURE_VERSION_1_0, D3D12_BLEND_DESC, D3D12_BLEND_INV_SRC_ALPHA, D3D12_BLEND_ONE, D3D12_BLEND_OP_ADD,
	D3D12_BLEND_SRC_ALPHA, D3D12_BLEND_ZERO, D3D12_BUFFER_SRV, D3D12_BUFFER_SRV_FLAG_NONE, D3D12_BUFFER_UAV,
	D3D12_BUFFER_UAV_FLAG_NONE, D3D12_BUFFER_UAV_FLAG_RAW, D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC,
	D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS, D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0,
	D3D12_CACHED_PIPELINE_STATE, D3D12_CLEAR_FLAG_DEPTH, D3D12_CLEAR_VALUE, D3D12_CLEAR_VALUE_0, D3D12_COLOR_WRITE_ENABLE_ALL,
	D3D12_COMMAND_LIST_TYPE, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAGS, D3D12_COMMAND_SIGNATURE_DESC,
	D3D12_COMPARISON_FUNC_ALWAYS, D3D12_COMPARISON_FUNC_GREATER_EQUAL, D3D12_COMPARISON_FUNC_NEVER,
	D3D12_COMPUTE_PIPELINE_STATE_DESC, D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF, D3D12_CONSTANT_BUFFER_VIEW_DESC,
	D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_BACK, D3D12_CULL_MODE_FRONT,
	D3D12_CULL_MODE_NONE, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING, D3D12_DEPTH_STENCIL_DESC, D3D12_DEPTH_STENCIL_VALUE,
	D3D12_DEPTH_STENCIL_VIEW_DESC, D3D12_DEPTH_STENCIL_VIEW_DESC_0, D3D12_DEPTH_STENCILOP_DESC, D3D12_DEPTH_WRITE_MASK_ALL,
	D3D12_DEPTH_WRITE_MASK_ZERO, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
	D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_DSV, D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
	D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE, D3D12_DESCRIPTOR_RANGE_TYPE, D3D12_DESCRIPTOR_RANGE_TYPE_CBV,
	D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, D3D12_DESCRIPTOR_RANGE_TYPE_SRV, D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
	D3D12_DISPATCH_RAYS_DESC, D3D12_DSV_DIMENSION_TEXTURE2D, D3D12_DSV_DIMENSION_TEXTURE2DARRAY, D3D12_DSV_FLAG_NONE,
	D3D12_DXIL_LIBRARY_DESC, D3D12_ELEMENTS_LAYOUT_ARRAY, D3D12_EXPORT_DESC, D3D12_EXPORT_FLAG_NONE,
	D3D12_FEATURE_D3D12_OPTIONS1, D3D12_FEATURE_D3D12_OPTIONS4, D3D12_FEATURE_D3D12_OPTIONS5, D3D12_FEATURE_D3D12_OPTIONS7,
	D3D12_FEATURE_DATA_D3D12_OPTIONS1, D3D12_FEATURE_DATA_D3D12_OPTIONS4, D3D12_FEATURE_DATA_D3D12_OPTIONS5,
	D3D12_FEATURE_DATA_D3D12_OPTIONS7, D3D12_FENCE_FLAGS, D3D12_FILL_MODE_SOLID, D3D12_FILTER, D3D12_FILTER_ANISOTROPIC,
	D3D12_FILTER_MAXIMUM_ANISOTROPIC, D3D12_FILTER_MIN_MAG_MIP_LINEAR, D3D12_FILTER_MINIMUM_ANISOTROPIC,
	D3D12_GLOBAL_ROOT_SIGNATURE, D3D12_GPU_DESCRIPTOR_HANDLE, D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE,
	D3D12_GPU_VIRTUAL_ADDRESS_RANGE, D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE, D3D12_GRAPHICS_PIPELINE_STATE_DESC,
	D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_HEAP_TYPE_READBACK, D3D12_HEAP_TYPE_UPLOAD,
	D3D12_HIT_GROUP_DESC, D3D12_HIT_GROUP_TYPE_PROCEDURAL_PRIMITIVE, D3D12_HIT_GROUP_TYPE_TRIANGLES,
	D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED, D3D12_INDEX_BUFFER_VIEW, D3D12_INDIRECT_ARGUMENT_DESC,
	D3D12_INDIRECT_ARGUMENT_DESC_0, D3D12_INDIRECT_ARGUMENT_TYPE_DISPATCH, D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
	D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC, D3D12_LOGIC_OP_NOOP, D3D12_MEMORY_POOL_UNKNOWN,
	D3D12_MESH_SHADER_TIER_NOT_SUPPORTED, D3D12_MESSAGE, D3D12_MESSAGE_SEVERITY_CORRUPTION, D3D12_MESSAGE_SEVERITY_ERROR,
	D3D12_PIPELINE_STATE_FLAG_NONE, D3D12_PIPELINE_STATE_FLAGS, D3D12_PIPELINE_STATE_STREAM_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_BLEND,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_DEPTH_STENCIL_FORMAT,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_FLAGS, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_NODE_MASK, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_PS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RASTERIZER, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_RENDER_TARGET_FORMATS,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_ROOT_SIGNATURE, D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_DESC,
	D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_SAMPLE_MASK, D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
	D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RANGE, D3D12_RASTERIZER_DESC,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV,
	D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL, D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
	D3D12_RAYTRACING_GEOMETRY_AABBS_DESC, D3D12_RAYTRACING_GEOMETRY_DESC, D3D12_RAYTRACING_GEOMETRY_DESC_0,
	D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE, D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC,
	D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS, D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
	D3D12_RAYTRACING_INSTANCE_DESC, D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE, D3D12_RAYTRACING_PIPELINE_CONFIG,
	D3D12_RAYTRACING_SHADER_CONFIG, D3D12_RAYTRACING_TIER_NOT_SUPPORTED, D3D12_RENDER_TARGET_BLEND_DESC,
	D3D12_RENDER_TARGET_VIEW_DESC, D3D12_RENDER_TARGET_VIEW_DESC_0, D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0,
	D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES, D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
	D3D12_RESOURCE_BARRIER_TYPE_UAV, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
	D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
	D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_FLAGS, D3D12_RESOURCE_STATE_COMMON,
	D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_DEPTH_WRITE,
	D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATE_INDEX_BUFFER, D3D12_RESOURCE_STATE_INDIRECT_ARGUMENT,
	D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_PRESENT,
	D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_RESOURCE_STATE_RENDER_TARGET,
	D3D12_RESOURCE_STATE_UNORDERED_ACCESS, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER, D3D12_RESOURCE_STATES,
	D3D12_RESOURCE_TRANSITION_BARRIER, D3D12_RESOURCE_UAV_BARRIER, D3D12_ROOT_CONSTANTS, D3D12_ROOT_DESCRIPTOR_TABLE,
	D3D12_ROOT_PARAMETER, D3D12_ROOT_PARAMETER_0, D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
	D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, D3D12_ROOT_SIGNATURE_DESC,
	D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT, D3D12_RT_FORMAT_ARRAY, D3D12_RTV_DIMENSION_TEXTURE2D,
	D3D12_RTV_DIMENSION_TEXTURE2DARRAY, D3D12_SAMPLER_DESC, D3D12_SHADER_BYTECODE, D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES,
	D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0, D3D12_SHADER_VISIBILITY_ALL,
	D3D12_SRV_DIMENSION_BUFFER, D3D12_SRV_DIMENSION_RAYTRACING_ACCELERATION_STRUCTURE, D3D12_SRV_DIMENSION_TEXTURE2D,
	D3D12_SRV_DIMENSION_TEXTURE2DARRAY, D3D12_SRV_DIMENSION_TEXTURE3D, D3D12_SRV_DIMENSION_TEXTURECUBE,
	D3D12_SRV_DIMENSION_TEXTURECUBEARRAY, D3D12_STATE_OBJECT_DESC, D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
	D3D12_STATE_SUBOBJECT, D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY, D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
	D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP, D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
	D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG, D3D12_STATE_SUBOBJECT_TYPE_SUBOBJECT_TO_EXPORTS_ASSOCIATION,
	D3D12_STENCIL_OP_KEEP, D3D12_SUBOBJECT_TO_EXPORTS_ASSOCIATION, D3D12_SUBRESOURCE_FOOTPRINT, D3D12_TEX2D_ARRAY_DSV,
	D3D12_TEX2D_ARRAY_RTV, D3D12_TEX2D_ARRAY_SRV, D3D12_TEX2D_ARRAY_UAV, D3D12_TEX2D_DSV, D3D12_TEX2D_RTV, D3D12_TEX2D_SRV,
	D3D12_TEX2D_UAV, D3D12_TEX3D_SRV, D3D12_TEX3D_UAV, D3D12_TEXCUBE_ARRAY_SRV, D3D12_TEXCUBE_SRV, D3D12_TEXTURE_ADDRESS_MODE,
	D3D12_TEXTURE_ADDRESS_MODE_BORDER, D3D12_TEXTURE_ADDRESS_MODE_CLAMP, D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
	D3D12_TEXTURE_ADDRESS_MODE_WRAP, D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
	D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX, D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
	D3D12_TEXTURE_LAYOUT_UNKNOWN, D3D12_UAV_DIMENSION_BUFFER, D3D12_UAV_DIMENSION_TEXTURE2D,
	D3D12_UAV_DIMENSION_TEXTURE2DARRAY, D3D12_UAV_DIMENSION_TEXTURE3D, D3D12_UNORDERED_ACCESS_VIEW_DESC,
	D3D12_UNORDERED_ACCESS_VIEW_DESC_0, D3D12_VERTEX_BUFFER_VIEW, D3D12_VIEWPORT, D3D12CreateDevice,
	D3D12SerializeRootSignature, ID3D12CommandAllocator, ID3D12CommandList, ID3D12CommandQueue, ID3D12CommandSignature,
	ID3D12DescriptorHeap, ID3D12Device, ID3D12Device2, ID3D12Device5, ID3D12Fence, ID3D12GraphicsCommandList,
	ID3D12GraphicsCommandList4, ID3D12GraphicsCommandList6, ID3D12InfoQueue, ID3D12PipelineState, ID3D12Resource,
	ID3D12RootSignature, ID3D12StateObject, ID3D12StateObjectProperties,
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
	CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_MWA_NO_ALT_ENTER, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
	DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIFactory4, IDXGISwapChain3,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{BOOL, PCSTR, PCWSTR};
use windows::{
	Win32::Graphics::{
		Direct3D12::{D3D12_COMMAND_LIST_TYPE_COMPUTE, D3D12_COMMAND_LIST_TYPE_COPY, D3D12_COMMAND_LIST_TYPE_DIRECT},
		Dxgi::{DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG},
	},
	core::{IUnknown, Interface},
};

use super::utils;
use crate::WorkloadTypes;
use crate::{
	AllocationHandle, AttachmentInformation, BaseBufferHandle, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferHandle, BufferStridedRange, ClearValue,
	CommandBufferHandle, DataTypes, DescriptorSetHandle, DeviceAccesses, DispatchExtent, DynamicBufferHandle, FilteringModes,
	Formats, HandleLike as _, ImageHandle, ImageOrSwapchain, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey,
	PresentationModes, QueueHandle, QueueSelection, RGBAu8, SamplerAddressingModes, SamplerHandle, SamplingReductionModes,
	ShaderHandle, ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle,
	TextureReadback as MappedTextureReadback, TextureTransferError, TextureViewTypes, TopLevelAccelerationStructureHandle,
	UseCases, Uses, buffer,
	descriptors::{DescriptorWrite, WriteData},
	device::Features,
	image,
	pipelines::{self, PushConstantRange, VertexElement},
	render_debugger::RenderDebugger,
	sampler,
	shader::{ResourceKind, ResourceSlot, ShaderResourceDescriptor, Sources},
	window,
};
