use std::alloc::{self, Layout};

use ::utils::hash::{HashMap, HashSet};
use ::utils::Extent;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::{D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_12_0};
use windows::Win32::Graphics::Direct3D12::{
	D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList,
	D3D12_COMMAND_LIST_TYPE, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAGS, D3D12_FENCE_FLAGS,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
	CreateDXGIFactory2, IDXGIFactory4, IDXGISwapChain3, DXGI_CREATE_FACTORY_FLAGS, DXGI_MWA_NO_ALT_ENTER, DXGI_SCALING_STRETCH,
	DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::{
	core::{IUnknown, Interface},
	Win32::Graphics::{
		Direct3D12::{D3D12_COMMAND_LIST_TYPE_COMPUTE, D3D12_COMMAND_LIST_TYPE_COPY, D3D12_COMMAND_LIST_TYPE_DIRECT},
		Dxgi::{DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG},
	},
};

use crate::{
	buffer,
	command_buffer::CommandBufferType,
	descriptors::{DescriptorType, Write as DescriptorWrite, WriteData},
	device::Features,
	image,
	pipelines::{self, PushConstantRange, ShaderParameter, VertexElement},
	rt, sampler,
	shader::{BindingDescriptor, Sources},
	window, AllocationHandle, BaseBufferHandle, BindingConstructor, BottomLevelAccelerationStructure,
	BottomLevelAccelerationStructureHandle, BufferHandle, CommandBufferHandle, DescriptorSetBindingHandle,
	DescriptorSetBindingTemplate, DescriptorSetHandle, DescriptorSetTemplateHandle, DeviceAccesses, DynamicBufferHandle,
	FilteringModes, Formats, Handle, ImageHandle, MeshHandle, PipelineHandle, PipelineLayoutHandle, PresentKey,
	PresentationModes, QueueHandle, QueueSelection, RGBAu8, SamplerAddressingModes, SamplerHandle, SamplingReductionModes,
	ShaderHandle, ShaderTypes, SwapchainHandle, SynchronizerHandle, TextureCopyHandle, TopLevelAccelerationStructureHandle,
	Uses,
};

use super::utils;

pub struct Device {
	device: ID3D12Device,
	settings: Features,
	frames: u8,

	queues: Vec<Queue>,
	command_buffers: Vec<CommandBuffer>,
	buffers: Vec<Buffer>,
	dynamic_buffers: Vec<Buffer>,
	images: Vec<Image>,
	samplers: Vec<Sampler>,
	descriptor_set_templates: Vec<DescriptorSetTemplate>,
	descriptor_sets: Vec<DescriptorSet>,
	descriptor_bindings: Vec<DescriptorSetBinding>,
	descriptors: HashMap<DescriptorSetHandle, HashMap<u32, HashMap<u32, WriteData>>>,
	resource_to_descriptor: HashMap<Handle, HashSet<(DescriptorSetBindingHandle, u32)>>,
	descriptor_set_to_resource: HashMap<(DescriptorSetHandle, u32), HashSet<Handle>>,
	pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayout, PipelineLayoutHandle>,
	pipelines: Vec<Pipeline>,
	shaders: Vec<Shader>,
	meshes: Vec<Mesh>,
	swapchains: Vec<Swapchain>,
	synchronizers: Vec<Synchronizer>,
	top_level_acceleration_structures: Vec<()>,
	bottom_level_acceleration_structures: Vec<()>,
	texture_copies: Vec<Vec<u8>>,
	allocations: Vec<Allocation>,
}

struct Queue {
	queue: ID3D12CommandQueue,
	queue_type: D3D12_COMMAND_LIST_TYPE,
}

struct CommandBuffer {
	queue_handle: QueueHandle,
	allocator: Option<ID3D12CommandAllocator>,
	command_list: Option<ID3D12GraphicsCommandList>,
}

struct Buffer {
	data: *mut u8,
	layout: Layout,
	size: usize,
	uses: Uses,
	access: DeviceAccesses,
}

impl Drop for Buffer {
	fn drop(&mut self) {
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

struct Image {
	extent: Extent,
	format: Formats,
	uses: Uses,
	access: DeviceAccesses,
	array_layers: u32,
	data: Option<Vec<u8>>,
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

struct DescriptorSetTemplate {
	bindings: Vec<DescriptorSetBindingTemplate>,
}

struct DescriptorSet {
	next: Option<DescriptorSetHandle>,
	template: DescriptorSetTemplateHandle,
	bindings: Vec<DescriptorSetBindingHandle>,
}

struct DescriptorSetBinding {
	next: Option<DescriptorSetBindingHandle>,
	descriptor_set: DescriptorSetHandle,
	descriptor_type: DescriptorType,
	binding_index: u32,
	count: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineLayout {
	descriptor_set_templates: Vec<DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<PushConstantRange>,
}

struct Pipeline {
	layout: PipelineLayoutHandle,
	shaders: Vec<ShaderHandle>,
	kind: PipelineKind,
}

enum PipelineKind {
	Raster,
	Compute,
	RayTracing,
}

struct Shader {
	stage: ShaderTypes,
	spirv: Option<Vec<u8>>,
	bindings: Vec<BindingDescriptor>,
}

struct Mesh {
	vertex_count: u32,
	index_count: u32,
	vertices: Vec<u8>,
	indices: Vec<u8>,
}

struct Swapchain {
	handles: window::Handles,
	swapchain: IDXGISwapChain3,
	extent: Extent,
	image_count: u8,
	next_image_index: u8,
	present_mode: PresentationModes,
}

struct Synchronizer {
	fence: ID3D12Fence,
	value: u64,
}

struct Allocation {
	data: Vec<u8>,
}

impl Device {
	/// Creates a DX12 device and initializes command queues for the requested queue types.
	pub fn new(settings: Features, queues: &mut [(QueueSelection, &mut Option<QueueHandle>)]) -> Result<Self, &'static str> {
		let adapter: Option<&IUnknown> = None;
		let mut device: Option<ID3D12Device> = None;
		unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_12_0, &mut device) }
			.or_else(|_| unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_11_0, &mut device) })
			.map_err(|_| "Failed to create a D3D12 device. The most likely cause is that the GPU or driver does not support the required feature level.")?;
		let device = device.ok_or(
			"Failed to acquire a D3D12 device. The most likely cause is that the D3D12CreateDevice call returned no device instance.",
		)?;

		let mut queue_storage = Vec::with_capacity(queues.len());

		for (selection, handle) in queues.iter_mut() {
			let queue_type = match selection.r#type {
				CommandBufferType::GRAPHICS => D3D12_COMMAND_LIST_TYPE_DIRECT,
				CommandBufferType::COMPUTE => D3D12_COMMAND_LIST_TYPE_COMPUTE,
				CommandBufferType::TRANSFER => D3D12_COMMAND_LIST_TYPE_COPY,
			};

			let desc = D3D12_COMMAND_QUEUE_DESC {
				Type: queue_type,
				Priority: 0,
				Flags: D3D12_COMMAND_QUEUE_FLAGS(0),
				NodeMask: 0,
			};

			let queue = unsafe { device.CreateCommandQueue(&desc) }
				.map_err(|_| "Failed to create a D3D12 command queue. The most likely cause is that the device does not support the requested queue type.")?;

			let index = queue_storage.len() as u64;
			queue_storage.push(Queue { queue, queue_type });
			**handle = Some(QueueHandle(index));
		}

		Ok(Self {
			device,
			settings,
			frames: 1,

			queues: queue_storage,
			command_buffers: Vec::new(),
			buffers: Vec::new(),
			dynamic_buffers: Vec::new(),
			images: Vec::new(),
			samplers: Vec::new(),
			descriptor_set_templates: Vec::new(),
			descriptor_sets: Vec::new(),
			descriptor_bindings: Vec::new(),
			descriptors: HashMap::default(),
			resource_to_descriptor: HashMap::default(),
			descriptor_set_to_resource: HashMap::default(),
			pipeline_layouts: Vec::new(),
			pipeline_layout_indices: HashMap::default(),
			pipelines: Vec::new(),
			shaders: Vec::new(),
			meshes: Vec::new(),
			swapchains: Vec::new(),
			synchronizers: Vec::new(),
			top_level_acceleration_structures: Vec::new(),
			bottom_level_acceleration_structures: Vec::new(),
			texture_copies: Vec::new(),
			allocations: Vec::new(),
		})
	}

	#[cfg(debug_assertions)]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		self.frames = frames.max(1);
		let image_count = self.frames.max(2);

		for swapchain in &mut self.swapchains {
			if swapchain.image_count != image_count && swapchain.extent.width() > 0 && swapchain.extent.height() > 0 {
				let result = unsafe {
					swapchain.swapchain.ResizeBuffers(
						image_count as u32,
						swapchain.extent.width(),
						swapchain.extent.height(),
						DXGI_FORMAT_B8G8R8A8_UNORM,
						DXGI_SWAP_CHAIN_FLAG(0),
					)
				};

				if result.is_err() {
					panic!(
						"Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed."
					);
				}
			}

			swapchain.image_count = image_count;
			swapchain.next_image_index %= image_count;
		}
	}

	pub fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: Uses,
		_resource_device_accesses: DeviceAccesses,
	) -> AllocationHandle {
		self.allocations.push(Allocation { data: vec![0u8; size] });
		AllocationHandle((self.allocations.len() - 1) as u64)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		_vertex_layout: &[VertexElement],
	) -> MeshHandle {
		// Stores mesh data in CPU memory for later draw calls.
		self.meshes.push(Mesh {
			vertex_count,
			index_count,
			vertices: vertices.to_vec(),
			indices: indices.to_vec(),
		});
		MeshHandle((self.meshes.len() - 1) as u64)
	}

	pub fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = BindingDescriptor>,
	) -> Result<ShaderHandle, ()> {
		// Stores shader metadata and bytecode without compiling to DXIL.
		let spirv = match shader_source_type {
			Sources::SPIRV(bytes) => Some(bytes.to_vec()),
		};

		self.shaders.push(Shader {
			stage,
			spirv,
			bindings: shader_binding_descriptors.into_iter().collect(),
		});

		// DX12 expects DXIL or precompiled HLSL bytecode, so SPIR-V is stored but not consumed.
		Ok(ShaderHandle((self.shaders.len() - 1) as u64))
	}

	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		binding_templates: &[DescriptorSetBindingTemplate],
	) -> DescriptorSetTemplateHandle {
		self.descriptor_set_templates.push(DescriptorSetTemplate {
			bindings: binding_templates.to_vec(),
		});
		DescriptorSetTemplateHandle((self.descriptor_set_templates.len() - 1) as u64)
	}

	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		descriptor_set_template_handle: &DescriptorSetTemplateHandle,
	) -> DescriptorSetHandle {
		// Creates per-frame descriptor set records for the template.
		let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(DescriptorSet {
				next: None,
				template: *descriptor_set_template_handle,
				bindings: Vec::new(),
			});

			if let Some(previous) = previous {
				self.descriptor_sets[previous.0 as usize].next = Some(handle);
			}

			previous = Some(handle);
		}

		handle
	}

	pub fn create_descriptor_binding(
		&mut self,
		descriptor_set: DescriptorSetHandle,
		binding_constructor: BindingConstructor,
	) -> DescriptorSetBindingHandle {
		// Records a descriptor binding while deferring DX12 descriptor heap setup.
		let template = binding_constructor.descriptor_set_binding_template;
		let descriptor_type = template.descriptor_type;
		let binding_index = template.binding;
		let count = template.descriptor_count;

		let descriptor_set_handles = self.collect_descriptor_set_handles(descriptor_set);
		let mut next = None;

		for (frame_index, descriptor_set_handle) in descriptor_set_handles.iter().enumerate().rev() {
			let binding_handle = DescriptorSetBindingHandle(self.descriptor_bindings.len() as u64);

			self.descriptor_bindings.push(DescriptorSetBinding {
				next,
				descriptor_set: *descriptor_set_handle,
				descriptor_type,
				binding_index,
				count,
			});

			if let Some(set) = self.descriptor_sets.get_mut(descriptor_set_handle.0 as usize) {
				set.bindings.push(binding_handle);
			}

			let descriptor = self.resolve_descriptor_for_frame(
				binding_constructor.descriptor,
				frame_index,
				binding_constructor.frame_offset.map(|offset| offset as i32),
			);
			self.update_descriptor_for_binding(binding_handle, descriptor, binding_constructor.array_element);

			next = Some(binding_handle);
		}

		// DX12 uses descriptor heaps and root signatures, so descriptor set bindings are stored but not bound yet.
		DescriptorSetBindingHandle(next.expect("No next binding").0)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		descriptor_set_template_handles: &[DescriptorSetTemplateHandle],
		push_constant_ranges: &[PushConstantRange],
	) -> PipelineLayoutHandle {
		let layout = PipelineLayout {
			descriptor_set_templates: descriptor_set_template_handles.to_vec(),
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&layout) {
			return *handle;
		}

		self.pipeline_layouts.push(layout.clone());
		let handle = PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
		self.pipeline_layout_indices.insert(layout, handle);
		handle
	}

	pub fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		// Records raster pipeline metadata without constructing a DX12 pipeline state.
		let shaders = builder.shaders.iter().map(|s| *s.handle).collect();
		self.pipelines.push(Pipeline {
			layout,
			shaders,
			kind: PipelineKind::Raster,
		});

		// DX12 pipeline state creation is deferred because shader translation to DXIL is not implemented.
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_parameter = builder.shader;
		// Records compute pipeline metadata without constructing a DX12 pipeline state.
		self.pipelines.push(Pipeline {
			layout,
			shaders: vec![*shader_parameter.handle],
			kind: PipelineKind::Compute,
		});
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let shaders = builder.shaders;
		// Records ray tracing pipeline metadata without constructing a DX12 state object.
		self.pipelines.push(Pipeline {
			layout,
			shaders: shaders.iter().map(|s| *s.handle).collect(),
			kind: PipelineKind::RayTracing,
		});

		// DX12 ray tracing state objects are not built yet because shader tables are not mapped.
		PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	/// Creates a command buffer and initializes a matching command allocator and list.
	pub fn create_command_buffer(&mut self, _name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle {
		let queue = &self.queues[queue_handle.0 as usize];
		let allocator = unsafe { self.device.CreateCommandAllocator(queue.queue_type) }.ok();
		let command_list = if let Some(allocator) = allocator.as_ref() {
			unsafe { self.device.CreateCommandList(0, queue.queue_type, allocator, None) }.ok()
		} else {
			None
		};

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			allocator,
			command_list,
		});

		CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		super::CommandBufferRecording::new(self, command_buffer_handle, None)
	}

	pub fn build_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> BufferHandle<T> {
		let handle = Self::create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			&mut self.buffers,
		);
		BufferHandle(handle, std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: Copy>(&mut self, builder: buffer::Builder) -> DynamicBufferHandle<T> {
		let handle = Self::create_buffer_with_layout(
			Layout::new::<T>(),
			builder.resource_uses,
			builder.device_accesses,
			&mut self.dynamic_buffers,
		);
		DynamicBufferHandle(handle, std::marker::PhantomData)
	}

	pub fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));
		crate::DynamicImageHandle(handle.0)
	}

	pub fn get_buffer_address(&self, _buffer_handle: BaseBufferHandle) -> u64 {
		// TODO: Map buffers to ID3D12Resource instances and return GPU virtual addresses.
		0
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &T {
		let buffer = &self.buffers[buffer_handle.0 as usize];
		unsafe { &*(buffer.data as *const T) }
	}

	pub fn get_mut_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: BufferHandle<T>) -> &'a mut T {
		let buffer = &self.buffers[buffer_handle.0 as usize];
		unsafe { &mut *(buffer.data as *mut T) }
	}

	pub fn get_texture_slice_mut(&mut self, _texture_handle: ImageHandle) -> &'static mut [u8] {
		// TODO: DX12 images are not mapped to CPU-visible resources yet.
		&mut []
	}

	pub fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		// Writes into CPU-side staging storage when available.
		let Some(image) = self.images.get_mut(texture_handle.0 as usize) else {
			return;
		};

		let Some(staging) = image.data.as_mut() else {
			// TODO: DX12 upload to textures requires staging resources and command lists.
			return;
		};

		f(staging);
	}

	pub fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		let size = utils::bytes_per_pixel(builder.format).map(|bpp| {
			bpp * builder.extent.width() as usize * builder.extent.height() as usize * builder.extent.depth() as usize
		});

		self.images.push(Image {
			extent: builder.extent,
			format: builder.format,
			uses: builder.resource_uses,
			access: builder.device_accesses,
			array_layers: builder.array_layers.map(|v| v.get()).unwrap_or(1),
			data: size.map(|bytes| vec![0u8; bytes]),
		});

		ImageHandle((self.images.len() - 1) as u64)
	}

	pub fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
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
		SamplerHandle((self.samplers.len() - 1) as u64)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		_name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle {
		// Allocates a CPU-side buffer sized for instance descriptors.
		let size = max_instance_count as usize * 64;
		let handle = Self::create_buffer_with_layout(
			Layout::from_size_align(size, 16).unwrap(),
			Uses::Storage,
			DeviceAccesses::DeviceOnly,
			&mut self.buffers,
		);
		BaseBufferHandle(handle)
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		// TODO: DXR top-level acceleration structure creation is not implemented yet.
		self.top_level_acceleration_structures.push(());
		TopLevelAccelerationStructureHandle((self.top_level_acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		_description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		// TODO: DXR bottom-level acceleration structure creation is not implemented yet.
		self.bottom_level_acceleration_structures.push(());
		BottomLevelAccelerationStructureHandle((self.bottom_level_acceleration_structures.len() - 1) as u64)
	}

	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		// Updates descriptor binding records without touching DX12 descriptor heaps.
		for write in descriptor_set_writes {
			let binding_handles = self.collect_descriptor_binding_handles(write.binding_handle);
			for (frame_index, binding_handle) in binding_handles.iter().enumerate() {
				let descriptor = self.resolve_descriptor_for_frame(write.descriptor, frame_index, write.frame_offset);
				self.update_descriptor_for_binding(*binding_handle, descriptor, write.array_element);
			}
		}

		// DX12 descriptor heap updates are not wired yet because root signatures are not created.
	}

	pub fn write_instance(
		&mut self,
		_instances_buffer_handle: BaseBufferHandle,
		_instance_index: usize,
		_transform: [[f32; 4]; 3],
		_custom_index: u16,
		_mask: u8,
		_sbt_record_offset: usize,
		_acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
		// TODO: DXR instance buffer writes require D3D12_RAYTRACING_INSTANCE_DESC layout.
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: PipelineHandle,
		_shader_handle: ShaderHandle,
	) {
		// TODO: DXR shader binding table packing is not implemented yet.
	}

	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
	) -> SwapchainHandle {
		let extent = Self::query_window_extent(window_os_handles, fallback_extent);
		let image_count = self.frames.max(2);

		let queue = self
			.queues
			.iter()
			.find(|queue| queue.queue_type == D3D12_COMMAND_LIST_TYPE_DIRECT)
			.or_else(|| self.queues.first())
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");

		let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI factory. The most likely cause is that the DXGI runtime is unavailable.");
		});

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
			Flags: 0,
		};

		let swapchain = unsafe { factory.CreateSwapChainForHwnd(&queue.queue, window_os_handles.hwnd, &swapchain_desc, None, None) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI swapchain. The most likely cause is that the window handle is invalid or the device does not support the swapchain format.");
		});

		let swapchain: IDXGISwapChain3 = swapchain.cast().unwrap_or_else(|_| {
			panic!(
				"Failed to upgrade the DXGI swapchain. The most likely cause is that the DXGI runtime does not support IDXGISwapChain3."
			);
		});

		let _ = unsafe { factory.MakeWindowAssociation(window_os_handles.hwnd, DXGI_MWA_NO_ALT_ENTER) };

		self.swapchains.push(Swapchain {
			handles: window::Handles {
				hinstance: window_os_handles.hinstance,
				hwnd: window_os_handles.hwnd,
			},
			swapchain,
			extent,
			image_count,
			next_image_index: 0,
			present_mode: presentation_mode,
		});

		SwapchainHandle((self.swapchains.len() - 1) as u64)
	}

	pub fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
		self.texture_copies
			.get(texture_copy_handle.0 as usize)
			.map(|v| v.as_slice())
			.unwrap_or(&[])
	}

	pub fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let initial_value = if signaled { 1 } else { 0 };
		let fence = unsafe { self.device.CreateFence(initial_value, D3D12_FENCE_FLAGS(0)) }
			.expect("Failed to create a D3D12 fence. The most likely cause is that the device does not support fences.");
		self.synchronizers.push(Synchronizer {
			fence,
			value: initial_value,
		});
		SynchronizerHandle((self.synchronizers.len() - 1) as u64)
	}

	pub fn start_frame<'a>(&'a mut self, index: u32, _synchronizer_handle: SynchronizerHandle) -> super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % self.frames as u32) as u8,
		};
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer(&mut self, buffer_handle: BaseBufferHandle, size: usize) {
		// Resizes CPU-side buffer storage while discarding previous contents.
		let buffer = &mut self.buffers[buffer_handle.0 as usize];
		if buffer.size >= size {
			return;
		}

		let layout = Layout::from_size_align(size, buffer.layout.align()).unwrap();
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to resize buffer storage. The most likely cause is that the system is out of memory.");
		}

		if buffer.layout.size() != 0 && !buffer.data.is_null() {
			unsafe {
				alloc::dealloc(buffer.data, buffer.layout);
			}
		}

		buffer.data = data;
		buffer.layout = layout;
		buffer.size = size;
	}

	pub fn start_frame_capture(&self) {
		// TODO: Integrate with PIX or other DX12 capture tooling.
	}

	pub fn end_frame_capture(&self) {
		// TODO: Integrate with PIX or other DX12 capture tooling.
	}

	pub fn wait(&self) {
		// TODO: Wait on outstanding fences once command submission is implemented.
	}

	pub(crate) fn copy_image_to_cpu(&mut self, image_handle: ImageHandle) -> TextureCopyHandle {
		// Copies stored image data into a new staging buffer for CPU reads.
		let image = &self.images[image_handle.0 as usize];
		let data = image.data.clone().unwrap_or_default();
		self.texture_copies.push(data);
		TextureCopyHandle((self.texture_copies.len() - 1) as u64)
	}

	pub(crate) fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]) {
		// Writes CPU-side image data for formats with staging storage.
		let image = &mut self.images[image_handle.0 as usize];
		let Some(staging) = image.data.as_mut() else {
			return;
		};

		let bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
		let length = staging.len().min(bytes.len());
		staging[..length].copy_from_slice(&bytes[..length]);
	}

	pub(crate) fn dynamic_buffer_slice_mut<'a, T: Copy>(&'a self, buffer_handle: DynamicBufferHandle<T>) -> &'a mut T {
		let buffer = &self.dynamic_buffers[buffer_handle.0 as usize];
		unsafe { &mut *(buffer.data as *mut T) }
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent) {
		// Resizes CPU-side image storage without emitting GPU commands.
		let image = &mut self.images[image_handle.0 as usize];
		if image.extent == extent {
			return;
		}

		image.extent = extent;
		image.data = utils::bytes_per_pixel(image.format).map(|bpp| {
			let size = bpp * extent.width() as usize * extent.height() as usize * extent.depth() as usize;
			vec![0u8; size]
		});
	}

	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle) -> Extent {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.0 as usize) else {
			return Extent::rectangle(0, 0);
		};

		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		if extent != swapchain.extent && extent.width() > 0 && extent.height() > 0 {
			let result = unsafe {
				swapchain.swapchain.ResizeBuffers(
					swapchain.image_count as u32,
					extent.width(),
					extent.height(),
					DXGI_FORMAT_B8G8R8A8_UNORM,
					DXGI_SWAP_CHAIN_FLAG(0),
				)
			};

			if result.is_err() {
				panic!(
					"Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed."
				);
			}

			swapchain.extent = extent;
		}
		extent
	}

	pub(crate) fn next_swapchain_image_index(&mut self, swapchain_handle: SwapchainHandle) -> u8 {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.0 as usize) else {
			return 0;
		};

		let index = unsafe { swapchain.swapchain.GetCurrentBackBufferIndex() } as u8;
		let image_count = swapchain.image_count.max(1);
		swapchain.next_image_index = (index + 1) % image_count;
		index
	}

	pub(crate) fn present_swapchain(&mut self, present_key: PresentKey) {
		let Some(swapchain) = self.swapchains.get_mut(present_key.swapchain.0 as usize) else {
			return;
		};

		let sync_interval = match swapchain.present_mode {
			PresentationModes::FIFO => 1,
			PresentationModes::Mailbox | PresentationModes::Inmediate => 0,
		};

		let result = unsafe { swapchain.swapchain.Present(sync_interval, DXGI_PRESENT(0)) };
		if result.is_err() {
			panic!(
				"Failed to present the DXGI swapchain. The most likely cause is that the device was removed or the swapchain became invalid."
			);
		}
	}

	/// Collects the per-frame descriptor set handles chained from the root handle.
	fn collect_descriptor_set_handles(&self, handle: DescriptorSetHandle) -> Vec<DescriptorSetHandle> {
		let mut handles = Vec::new();
		let mut current = Some(handle);

		while let Some(handle) = current {
			let Some(set) = self.descriptor_sets.get(handle.0 as usize) else {
				break;
			};
			handles.push(handle);
			current = set.next;
		}

		handles
	}

	fn query_window_extent(handles: &window::Handles, fallback_extent: Extent) -> Extent {
		let mut rect = RECT::default();
		let ok = unsafe { GetClientRect(handles.hwnd, &mut rect) }.is_ok();

		if !ok {
			return fallback_extent;
		}

		let width = (rect.right - rect.left).max(0) as u32;
		let height = (rect.bottom - rect.top).max(0) as u32;

		if width == 0 || height == 0 {
			fallback_extent
		} else {
			Extent::rectangle(width, height)
		}
	}

	/// Collects the per-frame descriptor binding handles chained from the root handle.
	fn collect_descriptor_binding_handles(&self, handle: DescriptorSetBindingHandle) -> Vec<DescriptorSetBindingHandle> {
		let mut handles = Vec::new();
		let mut current = Some(handle);

		while let Some(handle) = current {
			let Some(binding) = self.descriptor_bindings.get(handle.0 as usize) else {
				break;
			};
			handles.push(handle);
			current = binding.next;
		}

		handles
	}

	/// Resolves a frame-aware index using the optional frame offset.
	fn frame_index_with_offset(&self, frame_index: usize, frame_offset: Option<i32>, total_frames: usize) -> usize {
		let total = (total_frames.max(1)) as i32;
		let offset = frame_offset.unwrap_or(0);
		(frame_index as i32 - offset).rem_euclid(total) as usize
	}

	/// Resolves per-frame descriptor resources, falling back to single-resource handles for DX12.
	fn resolve_descriptor_for_frame(&self, descriptor: WriteData, frame_index: usize, frame_offset: Option<i32>) -> WriteData {
		let _sequence_index = self.frame_index_with_offset(frame_index, frame_offset, self.frames as usize);

		match descriptor {
			WriteData::Buffer { handle, size } => WriteData::Buffer { handle, size },
			WriteData::Image { handle, layout } => WriteData::Image { handle, layout },
			WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			} => WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			},
			_ => descriptor,
		}
	}

	/// Updates descriptor tracking and reverse lookup maps for a binding write.
	fn update_descriptor_for_binding(
		&mut self,
		binding_handle: DescriptorSetBindingHandle,
		descriptor: WriteData,
		array_element: u32,
	) {
		let Some(binding) = self.descriptor_bindings.get(binding_handle.0 as usize) else {
			return;
		};

		let descriptor_set_handle = binding.descriptor_set;
		let binding_index = binding.binding_index;

		let bindings = self.descriptors.entry(descriptor_set_handle).or_default();
		let arrays = bindings.entry(binding_index).or_default();
		arrays.insert(array_element, descriptor);

		let mut record_resource = |resource: Handle| {
			self.descriptor_set_to_resource
				.entry((descriptor_set_handle, binding_index))
				.or_default()
				.insert(resource);
			self.resource_to_descriptor
				.entry(resource)
				.or_default()
				.insert((binding_handle, array_element));
		};

		match descriptor {
			WriteData::Buffer { handle, .. } => {
				record_resource(handle.into());
			}
			WriteData::Image { handle, .. } => {
				record_resource(handle.into());
			}
			WriteData::CombinedImageSampler { image_handle, .. } => {
				record_resource(image_handle.into());
			}
			_ => {}
		}
	}

	fn create_buffer_with_layout(
		layout: Layout,
		resource_uses: Uses,
		device_accesses: DeviceAccesses,
		storage: &mut Vec<Buffer>,
	) -> u64 {
		// Allocates CPU storage for a buffer with the requested layout.
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to allocate buffer storage. The most likely cause is that the system is out of memory.");
		}

		storage.push(Buffer {
			data,
			layout,
			size: layout.size(),
			uses: resource_uses,
			access: device_accesses,
		});

		(storage.len() - 1) as u64
	}
}
