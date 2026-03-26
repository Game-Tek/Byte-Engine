use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::ptr::NonNull;

use ::utils::hash::HashSet;
use objc2::ClassType;
use objc2_foundation::NSString;
use objc2_metal::{MTLBuffer, MTLCommandQueue, MTLDevice, MTLResource, MTLTexture};

use super::*;
use crate::{
	buffer as buffer_builder, image as image_builder, pipelines::raster as raster_pipeline, sampler as sampler_builder, window,
	Size,
};

pub struct Device {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) frames: u8,
	pub(crate) queues: Vec<queue::Queue>,
	pub(crate) buffers: Vec<buffer::Buffer>,
	pub(crate) images: Vec<image::Image>,
	pub(crate) samplers: Vec<sampler::Sampler>,
	pub(crate) allocations: Vec<Allocation>,
	pub(crate) descriptor_sets_layouts: Vec<DescriptorSetLayout>,
	pub(crate) pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
	pub(crate) vertex_layouts: Vec<VertexLayout>,
	vertex_layout_indices: HashMap<VertexLayoutKey, VertexLayoutHandle>,
	pub(crate) bindings: Vec<binding::Binding>,
	pub(crate) descriptor_sets: Vec<descriptor_set::DescriptorSet>,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) acceleration_structures: Vec<AccelerationStructure>,
	pub(crate) shaders: Vec<Shader>,
	pub(crate) pipelines: Vec<Pipeline>,
	pub(crate) command_buffers: Vec<CommandBuffer>,
	pub(crate) synchronizers: Vec<synchronizer::Synchronizer>,
	pub(crate) swapchains: Vec<swapchain::Swapchain>,

	pub(crate) descriptors: HashMap<descriptor_set::DescriptorSetHandle, HashMap<u32, HashMap<u32, Descriptor>>>,
	pub(crate) resource_to_descriptor: HashMap<Handle, HashSet<(binding::DescriptorSetBindingHandle, u32)>>,
	pub(crate) descriptor_set_to_resource: HashMap<(descriptor_set::DescriptorSetHandle, u32), HashSet<Handle>>,

	pub settings: crate::device::Features,
	pub(crate) states: HashMap<Handle, TransitionState>,
	pub(crate) pending_buffer_syncs: VecDeque<buffer::BufferHandle>,
	pub(crate) pending_image_syncs: VecDeque<image::ImageHandle>,
	pub(crate) tasks: Vec<Task>,
	pub(crate) texture_copies: Vec<Vec<u8>>,

	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handle, String>,
}

impl Device {
	pub fn new(
		settings: crate::device::Features,
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		queues: &mut [(
			graphics_hardware_interface::QueueSelection,
			&mut Option<graphics_hardware_interface::QueueHandle>,
		)],
	) -> Result<Device, &'static str> {
		let mut created_queues = Vec::with_capacity(queues.len());

		for (_selection, output_handle) in queues.iter_mut() {
			let queue = device.newCommandQueue().ok_or(
				"Metal command queue creation failed. The most likely cause is that the device ran out of command queue resources.",
			)?;
			let handle = graphics_hardware_interface::QueueHandle(created_queues.len() as u64);

			created_queues.push(queue::Queue { queue });

			**output_handle = Some(handle);
		}

		Ok(Device {
			device,
			frames: MAX_FRAMES_IN_FLIGHT as u8,
			queues: created_queues,
			buffers: Vec::new(),
			images: Vec::new(),
			samplers: Vec::new(),
			allocations: Vec::new(),
			descriptor_sets_layouts: Vec::new(),
			pipeline_layouts: Vec::new(),
			pipeline_layout_indices: HashMap::default(),
			vertex_layouts: Vec::new(),
			vertex_layout_indices: HashMap::default(),
			bindings: Vec::new(),
			descriptor_sets: Vec::new(),
			meshes: Vec::new(),
			acceleration_structures: Vec::new(),
			shaders: Vec::new(),
			pipelines: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers: Vec::new(),
			swapchains: Vec::new(),
			descriptors: HashMap::default(),
			resource_to_descriptor: HashMap::default(),
			descriptor_set_to_resource: HashMap::default(),
			settings,
			states: HashMap::default(),
			pending_buffer_syncs: VecDeque::new(),
			pending_image_syncs: VecDeque::new(),
			tasks: Vec::new(),
			texture_copies: Vec::new(),

			#[cfg(debug_assertions)]
			names: HashMap::default(),
		})
	}

	fn create_buffer_internal(
		&mut self,
		next: Option<buffer::BufferHandle>,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> buffer::BufferHandle {
		let options = utils::resource_options_from_access(device_accesses);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal buffer creation failed. The most likely cause is that the device is out of memory.");

		if let Some(name) = name {
			buffer.setLabel(Some(&NSString::from_str(name)));
		}

		let pointer = buffer.contents().as_ptr() as *mut u8;
		let gpu_address = buffer.gpuAddress() as u64;

		let handle = buffer::BufferHandle(self.buffers.len() as u64);
		self.buffers.push(buffer::Buffer {
			next,
			staging: Some(handle),
			buffer,
			size,
			gpu_address,
			pointer,
			uses: resource_uses,
			access: device_accesses,
		});

		handle
	}

	pub(super) fn create_image_internal(
		&mut self,
		next: Option<image::ImageHandle>,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		array_layers: u32,
	) -> image::ImageHandle {
		let pixel_format = utils::to_pixel_format(format);

		let width = extent.width().max(1);
		let height = extent.height().max(1);
		let mipmapped = false;

		let descriptor = unsafe {
			mtl::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
				pixel_format,
				width as _,
				height as _,
				mipmapped,
			)
		};
		if extent.depth() > 1 {
			descriptor.setTextureType(mtl::MTLTextureType::Type3D);
		} else if array_layers > 1 {
			descriptor.setTextureType(mtl::MTLTextureType::Type2DArray);
		}
		descriptor.setUsage(utils::texture_usage_from_uses(resource_uses));
		descriptor.setStorageMode(mtl::MTLStorageMode::Shared);
		unsafe {
			descriptor.setArrayLength(array_layers as _);
		}

		let texture = self
			.device
			.newTextureWithDescriptor(&descriptor)
			.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

		if let Some(name) = name {
			texture.setLabel(Some(&NSString::from_str(name)));
		}

		let staging = utils::bytes_per_pixel(format).map(|bytes_per_pixel| {
			let depth = extent.depth().max(1) as usize;
			let size = width as usize * height as usize * depth * bytes_per_pixel * array_layers as usize;
			vec![0u8; size]
		});

		let handle = image::ImageHandle(self.images.len() as u64);
		self.images.push(image::Image {
			next,
			texture,
			extent,
			format,
			uses: resource_uses,
			access: device_accesses,
			array_layers,
			staging,
		});

		handle
	}

	fn update_descriptor_for_binding(
		&mut self,
		binding_handle: binding::DescriptorSetBindingHandle,
		descriptor: Descriptor,
		array_element: u32,
	) {
		let binding = &self.bindings[binding_handle.0 as usize];
		let set_handle = binding.descriptor_set_handle;

		let bindings = self.descriptors.entry(set_handle).or_default();
		let arrays = bindings.entry(binding.index).or_default();
		arrays.insert(array_element, descriptor);
	}

	pub(super) fn copy_texture_to_cpu(
		&mut self,
		image_handle: image::ImageHandle,
	) -> graphics_hardware_interface::TextureCopyHandle {
		let image = &self.images[image_handle.0 as usize];
		let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
			self.texture_copies.push(Vec::new());
			return graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64);
		};

		let extent = image.extent;
		let width = extent.width() as usize;
		let height = extent.height() as usize;
		let bytes_per_row = width * bytes_per_pixel;
		let size = bytes_per_row * height;

		let mut data = vec![0u8; size];
		let data_ptr = NonNull::new(data.as_mut_ptr() as *mut std::ffi::c_void)
			.expect("Texture readback buffer was null. The most likely cause is an empty allocation.");
		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: mtl::MTLSize {
				width: width as _,
				height: height as _,
				depth: 1,
			},
		};

		unsafe {
			image
				.texture
				.getBytes_bytesPerRow_fromRegion_mipmapLevel(data_ptr, bytes_per_row as _, region, 0);
		}

		self.texture_copies.push(data);
		graphics_hardware_interface::TextureCopyHandle((self.texture_copies.len() - 1) as u64)
	}
}

impl Device {
	#[cfg(debug_assertions)]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		self.frames = frames.max(1);
		// TODO: Rebuild dynamic resources for new frame count.
	}

	pub fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		let options = utils::resource_options_from_access(device_accesses);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal allocation failed. The most likely cause is that the device is out of memory.");
		let pointer = buffer.contents().as_ptr() as *mut u8;

		self.allocations.push(Allocation { buffer, pointer, size });
		graphics_hardware_interface::AllocationHandle((self.allocations.len() - 1) as u64)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		let options = mtl::MTLResourceOptions::StorageModeShared;
		let vertex_ptr = NonNull::new(vertices.as_ptr() as *mut std::ffi::c_void)
			.expect("Vertex data pointer was null. The most likely cause is an empty vertex slice.");
		let index_ptr = NonNull::new(indices.as_ptr() as *mut std::ffi::c_void)
			.expect("Index data pointer was null. The most likely cause is an empty index slice.");
		let vertex_buffer = unsafe {
			self.device
				.newBufferWithBytes_length_options(vertex_ptr, vertices.len() as _, options)
		}
		.expect("Metal vertex buffer creation failed. The most likely cause is that the device is out of memory.");
		let index_buffer = unsafe {
			self.device
				.newBufferWithBytes_length_options(index_ptr, indices.len() as _, options)
		}
		.expect("Metal index buffer creation failed. The most likely cause is that the device is out of memory.");
		let vertex_size = utils::vertex_layout_size(vertex_layout);

		self.meshes.push(Mesh {
			vertex_buffer,
			index_buffer,
			vertex_count,
			index_count,
			vertex_size,
		});

		graphics_hardware_interface::MeshHandle((self.meshes.len() - 1) as u64)
	}

	pub fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = crate::shader::BindingDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let spirv = match shader_source_type {
			crate::shader::Sources::SPIRV(data) => Some(data.to_vec()),
		};

		let stages = match stage {
			crate::ShaderTypes::Vertex => crate::Stages::VERTEX,
			crate::ShaderTypes::Fragment => crate::Stages::FRAGMENT,
			crate::ShaderTypes::Compute => crate::Stages::COMPUTE,
			crate::ShaderTypes::RayGen => crate::Stages::RAYGEN,
			crate::ShaderTypes::Intersection => crate::Stages::INTERSECTION,
			crate::ShaderTypes::AnyHit => crate::Stages::ANY_HIT,
			crate::ShaderTypes::ClosestHit => crate::Stages::CLOSEST_HIT,
			crate::ShaderTypes::Miss => crate::Stages::MISS,
			crate::ShaderTypes::Callable => crate::Stages::CALLABLE,
			crate::ShaderTypes::Task => crate::Stages::TASK,
			crate::ShaderTypes::Mesh => crate::Stages::MESH,
		};

		self.shaders.push(Shader {
			stage: stages,
			shader_binding_descriptors: shader_binding_descriptors.into_iter().collect(),
			metal_function: None,
			spirv,
		});

		// TODO: Compile SPIR-V to MSL and create MTLLibrary/MTLFunction.
		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		binding_templates: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
	) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		let bindings = binding_templates
			.iter()
			.map(|template| (template.descriptor_type, template.descriptor_count))
			.collect();
		self.descriptor_sets_layouts.push(DescriptorSetLayout { bindings });
		graphics_hardware_interface::DescriptorSetTemplateHandle((self.descriptor_sets_layouts.len() - 1) as u64)
	}

	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		descriptor_set_template_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		self.descriptor_sets.push(descriptor_set::DescriptorSet {
			next: None,
			descriptor_set_layout: *descriptor_set_template_handle,
		});
		graphics_hardware_interface::DescriptorSetHandle((self.descriptor_sets.len() - 1) as u64)
	}

	pub fn create_descriptor_binding(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		binding_constructor: graphics_hardware_interface::BindingConstructor,
	) -> graphics_hardware_interface::DescriptorSetBindingHandle {
		let descriptor_type = binding_constructor.descriptor_set_binding_template.descriptor_type;
		let binding_index = binding_constructor.descriptor_set_binding_template.binding;
		let count = binding_constructor.descriptor_set_binding_template.descriptor_count;

		self.bindings.push(binding::Binding {
			next: None,
			descriptor_set_handle: descriptor_set::DescriptorSetHandle(descriptor_set.0),
			descriptor_type,
			index: binding_index,
			count,
		});

		let binding_handle = binding::DescriptorSetBindingHandle((self.bindings.len() - 1) as u64);

		match binding_constructor.descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => {
				self.update_descriptor_for_binding(
					binding_handle,
					Descriptor::Buffer {
						buffer: buffer::BufferHandle(handle.0),
						size,
					},
					binding_constructor.array_element,
				);
			}
			crate::descriptors::WriteData::Image { handle, layout } => {
				self.update_descriptor_for_binding(
					binding_handle,
					Descriptor::Image {
						image: image::ImageHandle(handle.0),
						layout,
					},
					binding_constructor.array_element,
				);
			}
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				..
			} => {
				self.update_descriptor_for_binding(
					binding_handle,
					Descriptor::CombinedImageSampler {
						image: image::ImageHandle(image_handle.0),
						sampler: sampler::SamplerHandle(sampler_handle.0),
						layout,
					},
					binding_constructor.array_element,
				);
			}
			crate::descriptors::WriteData::Sampler(handle) => {
				self.update_descriptor_for_binding(
					binding_handle,
					Descriptor::Sampler {
						sampler: sampler::SamplerHandle(handle.0),
					},
					binding_constructor.array_element,
				);
			}
			_ => {
				// TODO: Map acceleration structures, swapchains, and static samplers to Metal argument buffers.
			}
		}

		graphics_hardware_interface::DescriptorSetBindingHandle(binding_handle.0)
	}

	fn get_or_create_pipeline_layout(
		&mut self,
		descriptor_set_template_handles: &[graphics_hardware_interface::DescriptorSetTemplateHandle],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let key = PipelineLayoutKey {
			descriptor_set_templates: descriptor_set_template_handles.to_vec(),
			push_constant_ranges: push_constant_ranges.to_vec(),
		};

		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		let descriptor_set_template_indices = descriptor_set_template_handles
			.iter()
			.enumerate()
			.map(|(i, handle)| (*handle, i as u32))
			.collect();
		self.pipeline_layouts.push(PipelineLayout {
			descriptor_set_template_indices,
		});
		let handle = graphics_hardware_interface::PipelineLayoutHandle((self.pipeline_layouts.len() - 1) as u64);
		self.pipeline_layout_indices.insert(key, handle);
		handle
	}

	fn get_or_create_vertex_layout(&mut self, vertex_elements: &[crate::pipelines::VertexElement]) -> VertexLayoutHandle {
		let elements = vertex_elements
			.iter()
			.map(|element| VertexElementDescriptor {
				name: element.name.to_owned(),
				format: element.format,
				binding: element.binding,
			})
			.collect::<Vec<_>>();
		let key = VertexLayoutKey {
			elements: elements.clone(),
		};

		if let Some(handle) = self.vertex_layout_indices.get(&key) {
			return *handle;
		}

		let max_binding = elements
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);
		let mut strides = vec![0; max_binding];

		for element in &elements {
			strides[element.binding as usize] += element.format.size() as u32;
		}

		self.vertex_layouts.push(VertexLayout { elements, strides });
		let handle = VertexLayoutHandle((self.vertex_layouts.len() - 1) as u64);
		self.vertex_layout_indices.insert(key, handle);
		handle
	}

	pub fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		let vertex_layout = self.get_or_create_vertex_layout(builder.vertex_elements.as_ref());

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Raster(None),
			layout,
			vertex_layout: Some(vertex_layout),
			shader_handles: HashMap::default(),
			resource_access: Vec::new(),
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		});

		let rpd = mtl::MTL4RenderPipelineDescriptor::new();
		rpd.setLabel(Some(&NSString::from_str("raster_pipeline")));
		// rpd.setVertexFunctionDescriptor(vertex_function_descriptor);
		// rpd.setFragmentFunctionDescriptor(fragment_function_descriptor);
		// rpd.colorAttachments().object
		//
		// self.device.newRenderPipelineStateWithDescriptor_error(&rpd);

		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(builder.descriptor_set_templates, builder.push_constant_ranges);
		let shader_handle = *builder.shader.handle;
		let compute_pipeline_state = {
			let shader = &self.shaders[shader_handle.0 as usize];
			assert!(
				shader.stage == crate::Stages::COMPUTE,
				"Metal compute pipeline creation requires a compute shader. The most likely cause is that a non-compute shader was passed to compute::Builder.",
			);

			shader.metal_function.as_ref().map(|function| {
				self.device
					.newComputePipelineStateWithFunction_error(function)
					.expect("Metal compute pipeline creation failed. The most likely cause is that the shader function was invalid for compute pipeline creation.")
			})
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			layout,
			vertex_layout: None,
			shader_handles,
			resource_access: Vec::new(),
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout = self.get_or_create_pipeline_layout(
			builder.descriptor_set_templates.as_ref(),
			builder.push_constant_ranges.as_ref(),
		);
		self.pipelines.push(Pipeline {
			pipeline: PipelineState::RayTracing,
			layout,
			vertex_layout: None,
			shader_handles: HashMap::default(),
			resource_access: Vec::new(),
			face_winding: crate::pipelines::raster::FaceWinding::Clockwise,
			cull_mode: crate::pipelines::raster::CullMode::Back,
		});
		// TODO: Metal ray tracing pipeline mapping.
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	pub fn create_command_buffer(
		&mut self,
		_name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		self.command_buffers.push(CommandBuffer { queue_handle });
		graphics_hardware_interface::CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
		let queue = &self.queues[command_buffer.queue_handle.0 as usize];
		let mtl_command_buffer = queue.queue.commandBuffer().expect(
			"Metal command buffer creation failed. The most likely cause is that the command queue did not provide a command buffer.",
		);

		super::CommandBufferRecording::new(self, command_buffer_handle, mtl_command_buffer, None)
	}

	pub fn build_buffer<T: Copy>(&mut self, builder: buffer_builder::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();
		let buffer_handle =
			self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
		graphics_hardware_interface::BufferHandle::<T>(buffer_handle.0, std::marker::PhantomData)
	}

	pub fn build_dynamic_buffer<T: Copy>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();
		let mut first_handle: Option<buffer::BufferHandle> = None;
		let mut previous_handle: Option<buffer::BufferHandle> = None;

		for _ in 0..self.frames {
			let handle = self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
			if let Some(previous) = previous_handle {
				self.buffers[previous.0 as usize].next = Some(handle);
			} else {
				first_handle = Some(handle);
			}
			previous_handle = Some(handle);
		}

		let master =
			first_handle.expect("Dynamic buffer creation failed. The most likely cause is that no buffers were allocated.");
		graphics_hardware_interface::DynamicBufferHandle::<T>(master.0, std::marker::PhantomData)
	}

	pub fn build_dynamic_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
		let mut first_handle: Option<image::ImageHandle> = None;
		let mut previous_handle: Option<image::ImageHandle> = None;

		for _ in 0..self.frames {
			let handle = self.create_image_internal(
				None,
				builder.get_name(),
				builder.extent,
				builder.format,
				builder.resource_uses,
				builder.device_accesses,
				layers,
			);

			if let Some(previous) = previous_handle {
				self.images[previous.0 as usize].next = Some(handle);
			} else {
				first_handle = Some(handle);
			}

			previous_handle = Some(handle);
		}

		let master =
			first_handle.expect("Dynamic image creation failed. The most likely cause is that no images were allocated.");
		graphics_hardware_interface::DynamicImageHandle(master.0)
	}

	pub fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers[buffer_handle.0 as usize].gpu_address
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = &self.buffers[buffer_handle.0 as usize];
		unsafe { &*(buffer.pointer as *const T) }
	}

	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		let buffer = &self.buffers[buffer_handle.0 as usize];
		unsafe { std::mem::transmute(buffer.pointer) }
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let handle = buffer::BufferHandle(buffer_handle.into().0);
		self.pending_buffer_syncs.push_back(handle);
	}

	pub fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let image = &self.images[texture_handle.0 as usize];
		let Some(staging) = image.staging.as_ref() else {
			return &mut [];
		};

		unsafe { std::slice::from_raw_parts_mut(staging.as_ptr() as *mut u8, staging.len()) }
	}

	pub fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image = &mut self.images[texture_handle.0 as usize];
		let Some(staging) = image.staging.as_mut() else {
			return;
		};

		f(staging);

		let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
			return;
		};

		let extent = image.extent;
		let width = extent.width() as usize;
		let height = extent.height() as usize;
		let bytes_per_row = width * bytes_per_pixel;

		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: mtl::MTLSize {
				width: width as _,
				height: height as _,
				depth: 1,
			},
		};

		let staging_ptr = NonNull::new(staging.as_ptr() as *mut std::ffi::c_void)
			.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");
		unsafe {
			image
				.texture
				.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
		}
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		let handle = image::ImageHandle(image_handle.0);
		self.pending_image_syncs.push_back(handle);
	}

	pub fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
		let image_handle = self.create_image_internal(
			None,
			builder.get_name(),
			builder.extent,
			builder.format,
			builder.resource_uses,
			builder.device_accesses,
			layers,
		);
		graphics_hardware_interface::ImageHandle(image_handle.0)
	}

	pub fn build_sampler(&mut self, builder: sampler_builder::Builder) -> graphics_hardware_interface::SamplerHandle {
		let descriptor = mtl::MTLSamplerDescriptor::new();
		descriptor.setMinFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
		descriptor.setMagFilter(utils::sampler_min_mag_filter(builder.filtering_mode));
		descriptor.setMipFilter(utils::sampler_mip_filter(builder.mip_map_mode));
		descriptor.setSAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setTAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setRAddressMode(utils::sampler_address_mode(builder.addressing_mode));
		descriptor.setLodMinClamp(builder.min_lod);
		descriptor.setLodMaxClamp(builder.max_lod);

		if let Some(anisotropy) = builder.anisotropy {
			descriptor.setMaxAnisotropy(anisotropy as _);
		}

		let sampler_state = self
			.device
			.newSamplerStateWithDescriptor(&descriptor)
			.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.");
		self.samplers.push(super::sampler::Sampler { sampler: sampler_state });
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let size = max_instance_count as usize * std::mem::size_of::<mtl::MTLAccelerationStructureInstanceDescriptor>();
		let handle = self.create_buffer_internal(
			None,
			name,
			size,
			crate::Uses::AccelerationStructure,
			crate::DeviceAccesses::DeviceOnly,
		);
		graphics_hardware_interface::BaseBufferHandle(handle.0)
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		_max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure and backing buffer.
		graphics_hardware_interface::TopLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		_description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		self.acceleration_structures.push(AccelerationStructure {
			structure: None,
			buffer: None,
		});
		// TODO: Build MTLAccelerationStructure for mesh or AABB.
		graphics_hardware_interface::BottomLevelAccelerationStructureHandle((self.acceleration_structures.len() - 1) as u64)
	}

	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::Write]) {
		for write in descriptor_set_writes {
			let binding_handle = binding::DescriptorSetBindingHandle(write.binding_handle.0);
			let array_element = write.array_element;

			match write.descriptor {
				crate::descriptors::WriteData::Buffer { handle, size } => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Buffer {
							buffer: buffer::BufferHandle(handle.0),
							size,
						},
						array_element,
					);
				}
				crate::descriptors::WriteData::Image { handle, layout } => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Image {
							image: image::ImageHandle(handle.0),
							layout,
						},
						array_element,
					);
				}
				crate::descriptors::WriteData::CombinedImageSampler {
					image_handle,
					sampler_handle,
					layout,
					..
				} => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::CombinedImageSampler {
							image: image::ImageHandle(image_handle.0),
							sampler: sampler::SamplerHandle(sampler_handle.0),
							layout,
						},
						array_element,
					);
				}
				crate::descriptors::WriteData::Sampler(handle) => {
					self.update_descriptor_for_binding(
						binding_handle,
						Descriptor::Sampler {
							sampler: sampler::SamplerHandle(handle.0),
						},
						array_element,
					);
				}
				_ => {
					// TODO: Implement descriptor writes for Metal acceleration structures and swapchains.
				}
			}
		}
	}

	pub fn write_instance(
		&mut self,
		_instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_instance_index: usize,
		_transform: [[f32; 4]; 3],
		_custom_index: u16,
		_mask: u8,
		_sbt_record_offset: usize,
		_acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		// TODO: Populate MTLAccelerationStructureInstanceDescriptor buffer.
	}

	pub fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: graphics_hardware_interface::PipelineHandle,
		_shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		// TODO: Metal ray tracing shader binding table mapping.
	}

	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		_presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		_uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();
		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(mtl::MTLPixelFormat::BGRA8Unorm);
		layer.setFramebufferOnly(false);

		window_os_handles.view.setWantsLayer(true);
		window_os_handles.view.setLayer(Some(layer.as_super()));
		let extent = update_layer_extent(&layer, &window_os_handles.view);

		self.swapchains.push(swapchain::Swapchain::new(
			layer,
			window_os_handles.view.clone(),
			if extent.width() == 0 || extent.height() == 0 {
				fallback_extent
			} else {
				extent
			},
			mtl::MTLPixelFormat::BGRA8Unorm,
		));
		graphics_hardware_interface::SwapchainHandle((self.swapchains.len() - 1) as u64)
	}

	pub fn get_image_data<'a>(&'a self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		self.texture_copies
			.get(texture_copy_handle.0 as usize)
			.map(|data| data.as_slice())
			.unwrap_or(&[])
	}

	pub fn create_synchronizer(
		&mut self,
		_name: Option<&str>,
		signaled: bool,
	) -> graphics_hardware_interface::SynchronizerHandle {
		self.synchronizers.push(synchronizer::Synchronizer { next: None, signaled });
		graphics_hardware_interface::SynchronizerHandle((self.synchronizers.len() - 1) as u64)
	}

	pub fn start_frame<'a>(
		&'a mut self,
		index: u32,
		_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> super::Frame<'a> {
		let frame_key = graphics_hardware_interface::FrameKey {
			frame_index: index,
			sequence_index: (index % self.frames as u32) as u8,
		};
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let handle = buffer::BufferHandle(buffer_handle.0);
		let buffer = &self.buffers[handle.0 as usize];

		if buffer.size >= size {
			return;
		}

		let name = buffer.buffer.label().map(|l| l.to_string());
		let new_handle = self.create_buffer_internal(None, name.as_deref(), size, buffer.uses, buffer.access);
		self.buffers[handle.0 as usize] = self.buffers[new_handle.0 as usize].clone();
	}

	pub fn start_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn end_frame_capture(&self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	pub fn wait(&self) {
		// TODO: Track pending command buffers and wait for completion.
	}
}
