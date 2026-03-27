use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::ptr::NonNull;

use ::utils::hash::{HashMap, HashSet};
use objc2::ClassType;
use objc2_foundation::{NSArray, NSString};
use objc2_metal::{MTLArgumentEncoder, MTLBuffer, MTLCommandQueue, MTLDevice, MTLLibrary, MTLResource, MTLTexture};

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

	pub(crate) resource_to_descriptor: HashMap<Handle, HashSet<(binding::DescriptorSetBindingHandle, u32, u8)>>,
	pub(crate) descriptor_set_to_resource: HashMap<(descriptor_set::DescriptorSetHandle, u32, u32, u8), HashSet<Handle>>,

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
		let handle = buffer::BufferHandle(self.buffers.len() as u64);
		let buffer = self.create_buffer_resource(next, name, size, resource_uses, device_accesses, handle);

		self.buffers.push(buffer);

		handle
	}

	fn create_buffer_resource(
		&self,
		next: Option<buffer::BufferHandle>,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		handle: buffer::BufferHandle,
	) -> buffer::Buffer {
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

		buffer::Buffer {
			next,
			staging: Some(handle),
			buffer,
			size,
			gpu_address,
			pointer,
			uses: resource_uses,
			access: device_accesses,
		}
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
		let handle = image::ImageHandle(self.images.len() as u64);
		let image = self.create_image_resource(next, name, extent, format, resource_uses, device_accesses, array_layers);

		self.images.push(image);

		handle
	}

	pub(super) fn create_image_resource(
		&self,
		next: Option<image::ImageHandle>,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		array_layers: u32,
	) -> image::Image {
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
		descriptor.setStorageMode(utils::storage_mode_from_access(device_accesses));
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

		image::Image {
			next,
			texture,
			extent,
			format,
			uses: resource_uses,
			access: device_accesses,
			array_layers,
			staging,
		}
	}

	fn update_descriptor_for_binding(
		&mut self,
		binding_handle: binding::DescriptorSetBindingHandle,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let binding = self.bindings[binding_handle.0 as usize].clone();
		let set_handle = binding.descriptor_set_handle;
		let binding_index = binding.index;

		self.clear_descriptor_tracking(set_handle, binding_handle, binding_index, array_element, frame_index);

		{
			let frame_state = &mut self.descriptor_sets[set_handle.0 as usize].frames[frame_index as usize];
			let bindings = frame_state.descriptors.entry(binding_index).or_default();
			bindings.insert(array_element, descriptor);
		}

		self.encode_descriptor_binding(set_handle, binding_index, descriptor, frame_index, array_element);
		self.register_descriptor_tracking(
			set_handle,
			binding_handle,
			binding_index,
			descriptor,
			array_element,
			frame_index,
		);
	}

	fn clear_descriptor_tracking(
		&mut self,
		set_handle: descriptor_set::DescriptorSetHandle,
		binding_handle: binding::DescriptorSetBindingHandle,
		binding_index: u32,
		array_element: u32,
		frame_index: u8,
	) {
		let key = (set_handle, binding_index, array_element, frame_index);
		let Some(resources) = self.descriptor_set_to_resource.remove(&key) else {
			return;
		};

		for resource in resources {
			let should_remove = if let Some(descriptor_bindings) = self.resource_to_descriptor.get_mut(&resource) {
				descriptor_bindings.remove(&(binding_handle, array_element, frame_index));
				descriptor_bindings.is_empty()
			} else {
				false
			};

			if should_remove {
				self.resource_to_descriptor.remove(&resource);
			}
		}
	}

	fn register_descriptor_tracking(
		&mut self,
		set_handle: descriptor_set::DescriptorSetHandle,
		binding_handle: binding::DescriptorSetBindingHandle,
		binding_index: u32,
		descriptor: Descriptor,
		array_element: u32,
		frame_index: u8,
	) {
		let Some(resource) = descriptor.tracked_resource() else {
			return;
		};

		self.descriptor_set_to_resource
			.entry((set_handle, binding_index, array_element, frame_index))
			.or_default()
			.insert(resource);
		self.resource_to_descriptor
			.entry(resource)
			.or_default()
			.insert((binding_handle, array_element, frame_index));
	}

	fn encode_descriptor_binding(
		&mut self,
		set_handle: descriptor_set::DescriptorSetHandle,
		binding_index: u32,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let descriptor_set_layout_handle = self.descriptor_sets[set_handle.0 as usize].descriptor_set_layout;
		let (argument_encoder, layout_binding) = {
			let layout = &self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize];
			(
				layout.argument_encoder.clone(),
				layout.binding(binding_index).cloned().expect(
					"Descriptor set binding not found in Metal layout. The most likely cause is that a descriptor write targeted a binding that was not declared in the descriptor set template.",
				),
			)
		};
		let frame_state = &mut self.descriptor_sets[set_handle.0 as usize].frames[frame_index as usize];

		unsafe {
			argument_encoder.setArgumentBuffer_offset(Some(frame_state.argument_buffer.as_ref()), 0);
		}

		match (layout_binding.slot_for_array_element(array_element), descriptor) {
			(DescriptorBindingSlot::Buffer(slot), Descriptor::Buffer { buffer, .. }) => unsafe {
				let buffer = &self.buffers[buffer.0 as usize];
				argument_encoder.setBuffer_offset_atIndex(Some(buffer.buffer.as_ref()), 0, slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::Image { image, .. }) => unsafe {
				let image = &self.images[image.0 as usize];
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Texture(slot), Descriptor::CombinedImageSampler { image, .. }) => unsafe {
				let image = &self.images[image.0 as usize];
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), slot as _);
			},
			(DescriptorBindingSlot::Sampler(slot), Descriptor::Sampler { sampler }) => unsafe {
				let sampler = &self.samplers[sampler.0 as usize];
				argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
			},
			(
				DescriptorBindingSlot::CombinedImageSampler { texture, sampler },
				Descriptor::CombinedImageSampler {
					image,
					sampler: sampler_handle,
					..
				},
			) => unsafe {
				let image = &self.images[image.0 as usize];
				let sampler_state = &self.samplers[sampler_handle.0 as usize];
				argument_encoder.setTexture_atIndex(Some(image.texture.as_ref()), texture as _);
				argument_encoder.setSamplerState_atIndex(Some(sampler_state.sampler.as_ref()), sampler as _);
			},
			_ => panic!(
				"Descriptor write does not match the Metal descriptor set layout. The most likely cause is that a descriptor type was written to a binding declared with a different descriptor type."
			),
		}
	}

	fn encode_immutable_samplers(&mut self, set_handle: descriptor_set::DescriptorSetHandle) {
		let descriptor_set_layout_handle = self.descriptor_sets[set_handle.0 as usize].descriptor_set_layout;
		let (argument_encoder, bindings) = {
			let layout = &self.descriptor_sets_layouts[descriptor_set_layout_handle.0 as usize];
			(layout.argument_encoder.clone(), layout.bindings.clone())
		};

		for frame_state in &mut self.descriptor_sets[set_handle.0 as usize].frames {
			unsafe {
				argument_encoder.setArgumentBuffer_offset(Some(frame_state.argument_buffer.as_ref()), 0);
			}

			for binding in &bindings {
				let Some(immutable_samplers) = &binding.immutable_samplers else {
					continue;
				};

				for (array_element, sampler_handle) in immutable_samplers.iter().enumerate() {
					let slot = binding.slot_for_array_element(array_element as u32);
					let sampler = &self.samplers[sampler::SamplerHandle(sampler_handle.0).0 as usize];

					match slot {
						DescriptorBindingSlot::Sampler(slot) => unsafe {
							argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), slot as _);
						},
						DescriptorBindingSlot::CombinedImageSampler {
							sampler: sampler_slot, ..
						} => unsafe {
							argument_encoder.setSamplerState_atIndex(Some(sampler.sampler.as_ref()), sampler_slot as _);
						},
						_ => {}
					}
				}
			}
		}
	}

	fn resolve_descriptor_for_frame(
		&self,
		descriptor: crate::descriptors::WriteData,
		sequence_index: u8,
		frame_offset: i32,
	) -> Option<Descriptor> {
		match descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => {
				let handles = buffer::BufferHandle(handle.0).get_all(&self.buffers);
				let index = (sequence_index as i32 - frame_offset).rem_euclid(handles.len() as i32) as usize;
				Some(Descriptor::Buffer {
					buffer: handles[index],
					size,
				})
			}
			crate::descriptors::WriteData::Image { handle, layout } => {
				let handles = image::ImageHandle(handle.0).get_all(&self.images);
				let index = (sequence_index as i32 - frame_offset).rem_euclid(handles.len() as i32) as usize;
				Some(Descriptor::Image {
					image: handles[index],
					layout,
				})
			}
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				..
			} => {
				let handles = image::ImageHandle(image_handle.0).get_all(&self.images);
				let index = (sequence_index as i32 - frame_offset).rem_euclid(handles.len() as i32) as usize;
				Some(Descriptor::CombinedImageSampler {
					image: handles[index],
					sampler: sampler::SamplerHandle(sampler_handle.0),
					layout,
				})
			}
			crate::descriptors::WriteData::Sampler(handle) => Some(Descriptor::Sampler {
				sampler: sampler::SamplerHandle(handle.0),
			}),
			crate::descriptors::WriteData::StaticSamplers => None,
			crate::descriptors::WriteData::CombinedImageSamplerArray => None,
			crate::descriptors::WriteData::AccelerationStructure { .. } => None,
			crate::descriptors::WriteData::Swapchain(_) => None,
		}
	}

	fn apply_descriptor_write_for_frame(
		&mut self,
		binding_handle: binding::DescriptorSetBindingHandle,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
		sequence_index: u8,
	) {
		if let Some(descriptor) = self.resolve_descriptor_for_frame(descriptor, sequence_index, frame_offset) {
			self.update_descriptor_for_binding(binding_handle, descriptor, sequence_index, array_element);
		}
	}

	fn apply_descriptor_write_to_all_frames(
		&mut self,
		binding_handle: binding::DescriptorSetBindingHandle,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
	) {
		for sequence_index in 0..self.frames {
			self.apply_descriptor_write_for_frame(binding_handle, descriptor, array_element, frame_offset, sequence_index);
		}
	}

	pub(crate) fn rewrite_descriptors_for_handle(&mut self, handle: Handle) {
		let Some(descriptor_bindings) = self.resource_to_descriptor.get(&handle).cloned() else {
			return;
		};

		for (binding_handle, array_element, frame_index) in descriptor_bindings {
			let binding = self.bindings[binding_handle.0 as usize].clone();
			let set_handle = binding.descriptor_set_handle;
			let descriptor = self.descriptor_sets[set_handle.0 as usize].frames[frame_index as usize]
				.descriptors
				.get(&binding.index)
				.and_then(|descriptors| descriptors.get(&array_element))
				.copied();

			if let Some(descriptor) = descriptor {
				self.encode_descriptor_binding(set_handle, binding.index, descriptor, frame_index, array_element);
			}
		}
	}

	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let mut tasks = self.tasks.split_off(0);

		tasks.retain(|task| {
			if let Some(frame) = task.frame() {
				if frame != sequence_index {
					return true;
				}
			}

			match task.task() {
				Tasks::UpdateBufferDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(Handle::Buffer(*handle));
				}
				Tasks::UpdateImageDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(Handle::Image(*handle));
				}
				Tasks::UpdateDescriptor { descriptor_write } => {
					self.apply_descriptor_write_for_frame(
						binding::DescriptorSetBindingHandle(descriptor_write.binding_handle.0),
						descriptor_write.descriptor,
						descriptor_write.array_element,
						descriptor_write.frame_offset.unwrap_or(0),
						sequence_index,
					);
				}
				Tasks::WriteDescriptor {
					binding_handle,
					descriptor,
				} => match descriptor {
					Descriptors::Buffer { handle, size } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Buffer {
							buffer: *handle,
							size: *size,
						},
						sequence_index,
						0,
					),
					Descriptors::Image { handle, layout } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Image {
							image: *handle,
							layout: *layout,
						},
						sequence_index,
						0,
					),
					Descriptors::CombinedImageSampler {
						image_handle,
						sampler_handle,
						layout,
						..
					} => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::CombinedImageSampler {
							image: *image_handle,
							sampler: *sampler_handle,
							layout: *layout,
						},
						sequence_index,
						0,
					),
					Descriptors::Sampler { handle } => self.update_descriptor_for_binding(
						*binding_handle,
						Descriptor::Sampler { sampler: *handle },
						sequence_index,
						0,
					),
					Descriptors::CombinedImageSamplerArray => {}
				},
				Tasks::DeleteMetalTexture { .. }
				| Tasks::DeleteMetalBuffer { .. }
				| Tasks::ResizeImage { .. }
				| Tasks::BuildImage(_)
				| Tasks::BuildBuffer(_) => {}
			}

			false
		});

		self.tasks = tasks;
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
		let (spirv, metal_function) = match shader_source_type {
			crate::shader::Sources::SPIRV(data) => (Some(data.to_vec()), None),
			crate::shader::Sources::MTL { source, entry_point } => {
				let compile_options = mtl::MTLCompileOptions::new();
				let source = NSString::from_str(source);
				let library = self
					.device
					.newLibraryWithSource_options_error(&source, Some(&compile_options))
					.map_err(|_| ())?;
				let entry_point = NSString::from_str(entry_point);
				let function = library.newFunctionWithName(&entry_point).ok_or(())?;

				(None, Some(function))
			}
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
			metal_function,
			spirv,
		});

		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	pub fn create_descriptor_set_template(
		&mut self,
		_name: Option<&str>,
		binding_templates: &[graphics_hardware_interface::DescriptorSetBindingTemplate],
	) -> graphics_hardware_interface::DescriptorSetTemplateHandle {
		let mut next_argument_index = 0u32;
		let mut metal_argument_descriptors = Vec::new();
		let bindings = binding_templates
			.iter()
			.map(|template| {
				assert_ne!(
					template.descriptor_count, 0,
					"Metal descriptor set bindings must contain at least one descriptor. The most likely cause is that a descriptor set template declared a binding with descriptor_count = 0.",
				);

				let access = match template.descriptor_type {
					crate::descriptors::DescriptorType::UniformBuffer
					| crate::descriptors::DescriptorType::SampledImage
					| crate::descriptors::DescriptorType::InputAttachment
					| crate::descriptors::DescriptorType::Sampler
					| crate::descriptors::DescriptorType::CombinedImageSampler => mtl::MTLBindingAccess::ReadOnly,
					crate::descriptors::DescriptorType::StorageBuffer
					| crate::descriptors::DescriptorType::StorageImage
					| crate::descriptors::DescriptorType::AccelerationStructure => mtl::MTLBindingAccess::ReadWrite,
				};

				let mut build_slots = |data_type: mtl::MTLDataType| {
					(0..template.descriptor_count)
						.map(|_| {
							let descriptor = mtl::MTLArgumentDescriptor::argumentDescriptor();
							descriptor.setDataType(data_type);
							descriptor.setIndex(next_argument_index as _);
							descriptor.setAccess(access);
							if data_type == mtl::MTLDataType::Texture {
								descriptor.setTextureType(mtl::MTLTextureType::Type2D);
							}
							metal_argument_descriptors.push(descriptor);
							let slot = next_argument_index;
							next_argument_index += 1;
							slot
						})
						.collect::<Vec<_>>()
				};

				let argument_slots = match template.descriptor_type {
					crate::descriptors::DescriptorType::UniformBuffer
					| crate::descriptors::DescriptorType::StorageBuffer => {
						ArgumentBindingSlots::Buffer(build_slots(mtl::MTLDataType::Pointer))
					}
					crate::descriptors::DescriptorType::SampledImage
					| crate::descriptors::DescriptorType::StorageImage
					| crate::descriptors::DescriptorType::InputAttachment => {
						ArgumentBindingSlots::Texture(build_slots(mtl::MTLDataType::Texture))
					}
					crate::descriptors::DescriptorType::Sampler => {
						ArgumentBindingSlots::Sampler(build_slots(mtl::MTLDataType::Sampler))
					}
					crate::descriptors::DescriptorType::CombinedImageSampler => ArgumentBindingSlots::CombinedImageSampler {
						textures: build_slots(mtl::MTLDataType::Texture),
						samplers: build_slots(mtl::MTLDataType::Sampler),
					},
					crate::descriptors::DescriptorType::AccelerationStructure => {
						ArgumentBindingSlots::Buffer(build_slots(mtl::MTLDataType::Pointer))
					}
				};

				DescriptorSetLayoutBinding {
					binding: template.binding,
					descriptor_type: template.descriptor_type,
					descriptor_count: template.descriptor_count,
					stages: template.stages,
					immutable_samplers: template.immutable_samplers.clone(),
					argument_slots,
				}
			})
			.collect::<Vec<_>>();
		let argument_descriptor_refs = metal_argument_descriptors
			.iter()
			.map(|descriptor| descriptor.as_ref())
			.collect::<Vec<_>>();
		let argument_descriptors = NSArray::from_slice(&argument_descriptor_refs);
		let argument_encoder = self.device.newArgumentEncoderWithArguments(&argument_descriptors).expect(
			"Metal argument encoder creation failed. The most likely cause is that the descriptor set template described an unsupported argument layout.",
		);
		self.descriptor_sets_layouts.push(DescriptorSetLayout {
			bindings,
			encoded_length: argument_encoder.encodedLength() as usize,
			argument_encoder,
		});
		graphics_hardware_interface::DescriptorSetTemplateHandle((self.descriptor_sets_layouts.len() - 1) as u64)
	}

	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		descriptor_set_template_handle: &graphics_hardware_interface::DescriptorSetTemplateHandle,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		let encoded_length = self.descriptor_sets_layouts[descriptor_set_template_handle.0 as usize]
			.encoded_length
			.max(1);
		let frames = (0..self.frames)
			.map(|_| {
				let argument_buffer = self
					.device
					.newBufferWithLength_options(encoded_length as _, mtl::MTLResourceOptions::StorageModeShared)
					.expect(
						"Metal argument buffer allocation failed. The most likely cause is that the device is out of memory.",
					);

				DescriptorSetFrameState {
					argument_buffer,
					descriptors: HashMap::default(),
				}
			})
			.collect::<Vec<_>>();

		self.descriptor_sets.push(descriptor_set::DescriptorSet {
			next: None,
			descriptor_set_layout: *descriptor_set_template_handle,
			frames,
		});
		let handle = descriptor_set::DescriptorSetHandle((self.descriptor_sets.len() - 1) as u64);
		self.encode_immutable_samplers(handle);
		graphics_hardware_interface::DescriptorSetHandle(handle.0)
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
		self.apply_descriptor_write_to_all_frames(
			binding_handle,
			binding_constructor.descriptor,
			binding_constructor.array_element,
			binding_constructor.frame_offset.map(i32::from).unwrap_or(0),
		);

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
		let push_constant_size = push_constant_ranges
			.iter()
			.map(|range| range.offset as usize + range.size as usize)
			.max()
			.unwrap_or(0);
		self.pipeline_layouts.push(PipelineLayout {
			descriptor_set_template_indices,
			push_constant_ranges: push_constant_ranges.to_vec(),
			push_constant_size,
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
		let vertex_descriptor = mtl::MTLVertexDescriptor::vertexDescriptor();
		let mut binding_offsets = vec![0usize; max_binding];

		for (attribute_index, element) in elements.iter().enumerate() {
			strides[element.binding as usize] += element.format.size() as u32;

			let offset = binding_offsets[element.binding as usize];
			let attribute = unsafe { vertex_descriptor.attributes().objectAtIndexedSubscript(attribute_index as _) };
			attribute.setFormat(utils::vertex_format(element.format));
			unsafe {
				attribute.setOffset(offset as _);
				attribute.setBufferIndex(element.binding as _);
			}

			binding_offsets[element.binding as usize] += utils::data_type_size(element.format);
		}

		for (binding, stride) in strides.iter().copied().enumerate() {
			let layout = unsafe { vertex_descriptor.layouts().objectAtIndexedSubscript(binding as _) };
			unsafe {
				layout.setStride(stride as _);
				layout.setStepRate(1);
			}
			layout.setStepFunction(mtl::MTLVertexStepFunction::PerVertex);
		}

		self.vertex_layouts.push(VertexLayout {
			elements,
			strides,
			vertex_descriptor,
		});
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
		let mut shader_handles = HashMap::default();
		let mut vertex_function = None;
		let mut fragment_function = None;
		let resource_access = builder
			.shaders
			.iter()
			.flat_map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				shader_handles.insert(*shader_parameter.handle, [0; 32]);
				match shader_parameter.stage {
					crate::ShaderTypes::Vertex => vertex_function = shader.metal_function.clone(),
					crate::ShaderTypes::Fragment => fragment_function = shader.metal_function.clone(),
					_ => {}
				}
				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| {
						(
							(descriptor.set, descriptor.binding),
							(shader_parameter.stage.into(), descriptor.access),
						)
					})
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();

		let raster_pipeline_state = if let Some(vertex_function) = vertex_function.as_ref() {
			let descriptor = mtl::MTLRenderPipelineDescriptor::new();
			descriptor.setLabel(Some(&NSString::from_str("raster_pipeline")));
			descriptor.setVertexFunction(Some(vertex_function.as_ref()));
			descriptor.setFragmentFunction(fragment_function.as_ref().map(|function| function.as_ref()));
			descriptor.setVertexDescriptor(Some(&self.vertex_layouts[vertex_layout.0 as usize].vertex_descriptor));

			for (index, attachment) in builder.render_targets.iter().enumerate() {
				if attachment.format.channel_layout() == crate::ChannelLayout::Depth {
					descriptor.setDepthAttachmentPixelFormat(utils::to_pixel_format(attachment.format));
				} else {
					let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(index as _) };
					color_attachment.setPixelFormat(utils::to_pixel_format(attachment.format));
					match attachment.blend {
						crate::pipelines::raster::BlendMode::None => color_attachment.setBlendingEnabled(false),
						crate::pipelines::raster::BlendMode::Alpha => {
							color_attachment.setBlendingEnabled(true);
							color_attachment.setRgbBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setAlphaBlendOperation(mtl::MTLBlendOperation::Add);
							color_attachment.setSourceRGBBlendFactor(mtl::MTLBlendFactor::SourceAlpha);
							color_attachment.setDestinationRGBBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
							color_attachment.setSourceAlphaBlendFactor(mtl::MTLBlendFactor::One);
							color_attachment.setDestinationAlphaBlendFactor(mtl::MTLBlendFactor::OneMinusSourceAlpha);
						}
					}
				}
			}

			self.device.newRenderPipelineStateWithDescriptor_error(&descriptor).ok()
		} else {
			None
		};

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Raster(raster_pipeline_state),
			layout,
			vertex_layout: Some(vertex_layout),
			shader_handles,
			resource_access,
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
		});

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
			let function = shader.metal_function.as_ref().expect(
				"Metal compute pipeline creation requires a Metal shader function. The most likely cause is that this compute shader was created from SPIR-V, which this backend does not translate to MSL.",
			);

			Some(
				self.device
					.newComputePipelineStateWithFunction_error(function)
					.expect("Metal compute pipeline creation failed. The most likely cause is that the shader function was invalid for compute pipeline creation."),
			)
		};

		let mut shader_handles = HashMap::default();
		shader_handles.insert(shader_handle, [0; 32]);
		let resource_access = self.shaders[shader_handle.0 as usize]
			.shader_binding_descriptors
			.iter()
			.map(|descriptor| {
				(
					(descriptor.set, descriptor.binding),
					(crate::Stages::COMPUTE, descriptor.access),
				)
			})
			.collect::<Vec<_>>();

		self.pipelines.push(Pipeline {
			pipeline: PipelineState::Compute(compute_pipeline_state),
			layout,
			vertex_layout: None,
			shader_handles,
			resource_access,
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
		let resource_access = builder
			.shaders
			.iter()
			.flat_map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				shader
					.shader_binding_descriptors
					.iter()
					.map(|descriptor| ((descriptor.set, descriptor.binding), (shader.stage, descriptor.access)))
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();
		self.pipelines.push(Pipeline {
			pipeline: PipelineState::RayTracing,
			layout,
			vertex_layout: None,
			shader_handles: HashMap::default(),
			resource_access,
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
		self.create_command_buffer_recording_with_frame_key(command_buffer_handle, None)
	}

	pub fn create_command_buffer_recording_with_frame_key<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) -> super::CommandBufferRecording<'a> {
		let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
		let queue = &self.queues[command_buffer.queue_handle.0 as usize];
		let mtl_command_buffer = queue.queue.commandBuffer().expect(
			"Metal command buffer creation failed. The most likely cause is that the command queue did not provide a command buffer.",
		);

		super::CommandBufferRecording::new(self, command_buffer_handle, mtl_command_buffer, frame_key)
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
		descriptor.setSupportArgumentBuffers(true);

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
			self.apply_descriptor_write_to_all_frames(
				binding::DescriptorSetBindingHandle(write.binding_handle.0),
				write.descriptor,
				write.array_element,
				write.frame_offset.unwrap_or(0),
			);
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

	pub fn get_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		uses: crate::Uses,
	) -> (graphics_hardware_interface::ImageHandle, crate::Formats) {
		let (extent, format) = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			let format = match swapchain.pixel_format {
				mtl::MTLPixelFormat::BGRA8Unorm => crate::Formats::BGRAu8,
				mtl::MTLPixelFormat::BGRA8Unorm_sRGB => crate::Formats::BGRAsRGB,
				_ => panic!(
					"Unsupported Metal swapchain pixel format. The most likely cause is that the layer pixel format does not have a matching GHI format."
				),
			};
			(swapchain.extent, format)
		};

		let needs_new_proxy = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			let proxy_matches_extent = swapchain.images[0]
				.map(|image_handle| self.images[image_handle.0 as usize].extent == extent)
				.unwrap_or(false);

			!proxy_matches_extent || !swapchain.proxy_uses[0].contains(uses)
		};

		if needs_new_proxy {
			let existing_proxies = self.swapchains[swapchain_handle.0 as usize].images;
			let mut proxies = existing_proxies;
			for image_index in 0..super::MAX_SWAPCHAIN_IMAGES {
				let proxy = self.create_image_resource(
					None,
					Some("Swapchain Proxy Image"),
					extent,
					format,
					uses | crate::Uses::BlitSource,
					crate::DeviceAccesses::DeviceOnly,
					1,
				);

				if let Some(handle) = existing_proxies[image_index] {
					self.images[handle.0 as usize] = proxy;
					proxies[image_index] = Some(handle);
				} else {
					let handle = image::ImageHandle(self.images.len() as u64);
					self.images.push(proxy);
					proxies[image_index] = Some(handle);
				}
			}
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			swapchain.images = proxies;
			swapchain.proxy_uses = [uses; super::MAX_SWAPCHAIN_IMAGES];
		}

		let image = self.swapchains[swapchain_handle.0 as usize].images[0].expect(
			"Missing Metal swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
		);

		(graphics_hardware_interface::ImageHandle(image.0), format)
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
		self.process_tasks(frame_key.sequence_index);
		super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer(&mut self, buffer_handle: graphics_hardware_interface::BaseBufferHandle, size: usize) {
		let handle = buffer::BufferHandle(buffer_handle.0);
		let buffer = &self.buffers[handle.0 as usize];

		if buffer.size >= size {
			return;
		}

		let next = buffer.next;
		let uses = buffer.uses;
		let access = buffer.access;
		let name = buffer.buffer.label().map(|l| l.to_string());
		let replacement = self.create_buffer_resource(next, name.as_deref(), size, uses, access, handle);
		self.buffers[handle.0 as usize] = replacement;
		self.rewrite_descriptors_for_handle(Handle::Buffer(handle));
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
