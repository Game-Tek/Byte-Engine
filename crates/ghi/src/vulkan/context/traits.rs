use super::*;
use crate::Next as _;

impl std::ops::Deref for Context {
	type Target = InnerDevice;

	fn deref(&self) -> &Self::Target {
		&self.device
	}
}

impl std::ops::DerefMut for Context {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.device
	}
}

/// Returns the last handle of the chain that follows every resource with a successor.
fn chain_tails<H: HandleLike>(collection: &[H::Item]) -> Vec<H> {
	collection
		.iter()
		.filter_map(|item| {
			let mut handle = item.next()?;
			while let Some(next) = handle.access(collection).next() {
				handle = next;
			}
			Some(handle)
		})
		.collect()
}

impl Context {
	/// Creates an acceleration structure sized for `build_info` and returns its index.
	fn create_acceleration_structure(
		&mut self,
		name: Option<&str>,
		build_info: &vk::AccelerationStructureBuildGeometryInfoKHR,
		primitive_count: u32,
	) -> u64 {
		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				build_info,
				Some(&[primitive_count]),
				&mut size_info,
			);
		}

		let (buffer, ..) = self.create_dedicated_buffer(
			None,
			size_info.acceleration_structure_size as usize,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::GpuWrite,
		);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(size_info.acceleration_structure_size)
			.offset(0)
			.ty(build_info.ty);
		let acceleration_structure = unsafe {
			self.acceleration_structure
				.create_acceleration_structure(&create_info, None)
				.expect("No acceleration structure")
		};
		self.set_name(acceleration_structure, name);

		self.acceleration_structures.push(AccelerationStructure {
			acceleration_structure,
			buffer: buffer.resource,
		});
		(self.acceleration_structures.len() - 1) as u64
	}
}

impl crate::context::Context for Context {
	type Queue<'a>
		= crate::vulkan::queue::Queue<'a>
	where
		Self: 'a;
	type CommandBuffer<'a>
		= crate::vulkan::command_buffer::CommandBufferReference<'a>
	where
		Self: 'a;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		self.device.has_errors()
	}

	fn supports_bc_texture_compression(&self) -> bool {
		true
	}

	fn queue<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue<'a> {
		crate::vulkan::queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CommandBuffer<'a> {
		crate::vulkan::command_buffer::CommandBufferReference {
			device: self,
			command_buffer_handle,
		}
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		if self.frames == frames {
			return;
		}

		assert!(
			frames <= MAX_FRAMES_IN_FLIGHT as u8,
			"Cannot set frames in flight to more than {MAX_FRAMES_IN_FLIGHT}"
		);
		assert!(
			frames > self.frames,
			"Failed to reduce the frames in flight. The most likely cause is that shrinking per-frame resources is not implemented."
		);

		for image_handle in chain_tails::<ImageHandle>(&self.images) {
			let image = &self.images[image_handle.0 as usize];
			let new_image = self.create_image_internal(
				image.next,
				None,
				None,
				image.format_,
				image.access,
				image.layers,
				image.cube_compatible,
				image.cube_array_compatible,
				image.extent,
				image.uses,
				image.mip_levels,
			);
			self.images[image_handle.0 as usize].next = Some(new_image);
		}

		for synchronizer_handle in chain_tails::<SynchronizerHandle>(&self.synchronizers) {
			let root = synchronizer_handle.root(&self.synchronizers);
			let name = self.get_object_debug_name(graphics_hardware_interface::SynchronizerHandle(root.0).into());
			let signaled = self.synchronizers[synchronizer_handle.0 as usize].signaled;
			let new_synchronizer = self.create_synchronizer_internal(name.as_deref(), signaled);
			self.synchronizers[synchronizer_handle.0 as usize].next = Some(new_synchronizer);
		}

		for i in 0..self.command_buffers.len() {
			let queue_handle = self.command_buffers[i].queue_handle;
			let frame = self.create_command_buffer_frame(queue_handle, vk::CommandPoolCreateFlags::empty(), None);
			self.command_buffers[i].frames.push(frame);
		}

		self.frames = frames;
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.get_buffer_address(buffer_handle)
	}

	fn get_buffer_slice<T: crate::Pod>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		// SAFETY: Typed handles preserve the allocation's type and the buffer remains mapped while the context lives.
		unsafe { &*self.typed_buffer_pointer(buffer_handle) }
	}

	fn get_mut_buffer_slice<T: crate::Pod>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &mut T {
		self.get_mut_buffer_slice(buffer_handle)
	}

	unsafe fn transfer_buffer_mapping<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let buffer = self.host_visible_buffer(buffer_handle.into());
		let pointer = if std::mem::size_of::<T>() == 0 {
			std::ptr::NonNull::<T>::dangling().as_ptr().cast::<u8>()
		} else {
			buffer.pointer.0
		};
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer, std::mem::size_of::<T>()) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
		self.get_texture_slice_mut(texture_handle)
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		self.sync_texture(image_handle);
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		self.write_texture(texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		Context::write(self, descriptor_set_writes);
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		self.write_instance(
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
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		self.write_sbt_entry(sbt_buffer_handle, sbt_record_offset, pipeline_handle, shader_handle);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		self.bind_to_window(window_os_handles, presentation_mode, fallback_extent, uses)
	}

	fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		Context::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
		size: usize,
	) {
		let buffer_handle: graphics_hardware_interface::BaseBufferHandle = buffer_handle.into();
		self.resize_buffer_internal(BufferHandle(buffer_handle.0), size);
	}

	fn start_frame_capture(&mut self) {
		self.device.start_frame_capture();
	}

	fn end_frame_capture(&mut self) {
		self.device.end_frame_capture();
	}

	fn wait_for_synchronizer(&mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		Context::wait_for_synchronizer(self, synchronizer);
	}

	fn wait(&mut self) {
		self.device.wait();
	}
}

impl crate::context::ContextCreate for Context {
	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		self.create_allocation_internal(size, None, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		let index_offset = vertices.len().next_multiple_of(16);
		let (buffer, _, _, pointer) = self.create_dedicated_buffer(
			None,
			index_offset + indices.len(),
			vk::BufferUsageFlags::VERTEX_BUFFER
				| vk::BufferUsageFlags::INDEX_BUFFER
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		unsafe {
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), pointer, vertices.len());
			std::ptr::copy_nonoverlapping(indices.as_ptr(), pointer.add(index_offset), indices.len());
		}

		self.meshes.push(Mesh {
			buffer: buffer.resource,
			vertex_count,
			index_count,
			vertex_size: vertex_layout.size(),
		});
		graphics_hardware_interface::MeshHandle(self.meshes.len() as u64 - 1)
	}

	/// Creates a shader.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let crate::shader::Sources::SPIRV(spirv) = shader_source_type else {
			return Err(());
		};
		if !spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
			return Err(());
		}
		// SAFETY: shader was checked to be aligned to 4 bytes.
		let code = unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) };

		let shader_module = unsafe {
			self.device
				.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(code), None)
				.unwrap()
		};

		let handle = graphics_hardware_interface::ShaderHandle(self.shaders.len() as u64);
		self.shaders.push(Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
		});
		self.set_name(shader_module, name);

		Ok(handle)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>) -> graphics_hardware_interface::DescriptorSetHandle {
		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		self.descriptor_sets.push(DescriptorSet {
			next: None,
			version: 0,
			sequence_versions: [0; MAX_FRAMES_IN_FLIGHT],
			descriptors: HashMap::default(),
		});
		self.set_object_debug_name(name, graphics_hardware_interface::Handles::DescriptorSet(handle));
		handle
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_pipeline(builder)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let shader_parameter = builder.shader;
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(std::slice::from_ref(&shader_parameter), builder.push_constant_ranges);
		let (specialization_entries_buffer, specialization_map_entries) =
			crate::vulkan::utils::build_specialization_entries(shader_parameter.specialization_map);

		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let mappings = crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors);
		let mut mapping_info = vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings);
		let stage = vk::PipelineShaderStageCreateInfo::default()
			.push(&mut mapping_info)
			.stage(vk::ShaderStageFlags::COMPUTE)
			.module(shader.shader)
			.name(c"main")
			.specialization_info(&specialization_info);
		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.push(&mut descriptor_heap_flags)
			.stage(stage)
			.layout(vk::PipelineLayout::null())];

		let pipeline = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("No compute pipeline")[0]
		};

		self.pipelines.push(Pipeline {
			pipeline,
			layout: pipeline_layout_handle,
			shader_handles: HashMap::default(),
		});
		graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64 - 1)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let shaders = builder.shaders;

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let stage_mappings = shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];
				crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors)
			})
			.collect::<Vec<_>>();
		let mut mapping_infos = stage_mappings
			.iter()
			.map(|mappings| vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings))
			.collect::<Vec<_>>();
		let stages = shaders
			.iter()
			.zip(mapping_infos.iter_mut())
			.map(|(stage, mapping_info)| {
				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(to_shader_stage_flags(stage.stage))
					.module(self.shaders[stage.handle.0 as usize].shader)
					.name(c"main")
			})
			.collect::<Vec<_>>();

		let groups = shaders
			.iter()
			.enumerate()
			.filter_map(|(i, shader)| {
				let (i, unused) = (i as u32, vk::SHADER_UNUSED_KHR);
				let (ty, general, closest_hit, any_hit, intersection) = match shader.stage {
					crate::ShaderTypes::RayGen | crate::ShaderTypes::Miss | crate::ShaderTypes::Callable => {
						(vk::RayTracingShaderGroupTypeKHR::GENERAL, i, unused, unused, unused)
					}
					crate::ShaderTypes::ClosestHit => (
						vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP,
						unused,
						i,
						unused,
						unused,
					),
					crate::ShaderTypes::AnyHit => (
						vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP,
						unused,
						unused,
						i,
						unused,
					),
					crate::ShaderTypes::Intersection => (
						vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP,
						unused,
						unused,
						unused,
						i,
					),
					_ => return None,
				};
				Some(
					vk::RayTracingShaderGroupCreateInfoKHR::default()
						.ty(ty)
						.general_shader(general)
						.closest_hit_shader(closest_hit)
						.any_hit_shader(any_hit)
						.intersection_shader(intersection),
				)
			})
			.collect::<Vec<_>>();

		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.push(&mut descriptor_heap_flags)
			.layout(vk::PipelineLayout::null())
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let (pipeline, handle_buffer) = unsafe {
			let pipeline = self
				.ray_tracing_pipeline
				.create_ray_tracing_pipelines(
					vk::DeferredOperationKHR::null(),
					vk::PipelineCache::null(),
					&[create_info],
					None,
				)
				.expect("No ray tracing pipeline")[0];
			let handle_buffer = self
				.ray_tracing_pipeline
				.get_ray_tracing_shader_group_handles(pipeline, 0, groups.len() as u32, 32 * groups.len())
				.expect("Could not get ray tracing shader group handles");
			(pipeline, handle_buffer)
		};

		let shader_handles = shaders
			.iter()
			.enumerate()
			.map(|(i, shader)| (*shader.handle, handle_buffer[i * 32..(i + 1) * 32].try_into().unwrap()))
			.collect();

		self.pipelines.push(Pipeline {
			pipeline,
			layout: pipeline_layout_handle,
			shader_handles,
		});
		graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64 - 1)
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
		let create_image = |context: &mut Self, previous| {
			context.create_image_internal(
				None,
				previous,
				builder.name,
				builder.format,
				builder.device_accesses,
				builder.array_layers,
				builder.cube_compatible,
				builder.cube_array_compatible,
				builder.extent,
				builder.resource_uses,
				builder.mip_levels,
			)
		};

		let root_image_handle = create_image(self, None);
		let instances = match builder.use_case {
			crate::UseCases::DYNAMIC => self.frames,
			crate::UseCases::STATIC => 1,
		};
		let mut previous = root_image_handle;
		for _ in 1..instances {
			previous = create_image(self, Some(previous));
		}

		let handle =
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_image_handle.0));
		self.set_object_debug_name(builder.name, handle.into());
		handle
	}

	fn build_sampler(&mut self, builder: sampler::Builder) -> crate::SamplerHandle {
		let filtering_mode = match builder.filtering_mode {
			crate::FilteringModes::Closest => vk::Filter::NEAREST,
			crate::FilteringModes::Linear => vk::Filter::LINEAR,
		};

		let mip_map_filter = match builder.mip_map_mode {
			crate::FilteringModes::Closest => vk::SamplerMipmapMode::NEAREST,
			crate::FilteringModes::Linear => vk::SamplerMipmapMode::LINEAR,
		};

		let address_mode = match builder.addressing_mode {
			crate::SamplerAddressingModes::Repeat => vk::SamplerAddressMode::REPEAT,
			crate::SamplerAddressingModes::Mirror => vk::SamplerAddressMode::MIRRORED_REPEAT,
			crate::SamplerAddressingModes::Clamp => vk::SamplerAddressMode::CLAMP_TO_EDGE,
			crate::SamplerAddressingModes::Border { .. } => vk::SamplerAddressMode::CLAMP_TO_BORDER,
		};

		let reduction_mode = match builder.reduction_mode {
			crate::SamplingReductionModes::WeightedAverage => vk::SamplerReductionMode::WEIGHTED_AVERAGE,
			crate::SamplingReductionModes::Min => vk::SamplerReductionMode::MIN,
			crate::SamplingReductionModes::Max => vk::SamplerReductionMode::MAX,
		};

		let handle = graphics_hardware_interface::SamplerHandle(self.samplers.len() as u64);
		self.samplers.push(Sampler {
			mag_filter: filtering_mode,
			min_filter: filtering_mode,
			mipmap_mode: mip_map_filter,
			address_mode,
			reduction_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		});
		handle
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let access = crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead;
		let (buffer, allocation, device_address, pointer) = self.create_dedicated_buffer(
			name,
			max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>(),
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			access,
		);

		self.buffers
			.add(Buffer {
				staging: None,
				source: None,
				buffer: buffer.resource,
				size: buffer.size,
				device_address,
				pointer: crate::vulkan::MappedMemoryPointer(pointer),
				allocation: Some(allocation),
				uses: crate::Uses::empty(),
				access,
			})
			.0
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometries = [vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
			})];
		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.create_acceleration_structure(
			name,
			&build_info,
			max_instance_count,
		))
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let (geometry, primitive_count) = match &description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				vertex_position_encoding,
				triangle_count,
				index_format,
			} => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_format(match vertex_position_encoding {
								crate::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position format"),
							})
							.max_vertex(*vertex_count - 1)
							.index_type(match index_format {
								crate::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								crate::DataTypes::U16 => vk::IndexType::UINT16,
								crate::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							}),
					}),
				*triangle_count,
			),
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::AABBS)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default(),
					}),
				*transform_count,
			),
		};

		let geometries = [geometry];
		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
			.geometries(&geometries);

		graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.create_acceleration_structure(
			None,
			&build_info,
			primitive_count,
		))
	}

	fn build_buffer<T: crate::Pod>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let buffer_handle = self.create_buffer_internal(
			None,
			None,
			builder.name,
			builder.resource_uses,
			std::mem::size_of::<T>(),
			builder.device_accesses,
		);
		graphics_hardware_interface::BufferHandle(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData,
		)
	}

	fn build_dynamic_buffer<T: crate::Pod>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();
		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData,
		);

		let source = if crate::vulkan::buffer::PERSISTENT_WRITE
			&& builder.device_accesses.intersects(crate::DeviceAccesses::CpuWrite)
			&& !Self::uses_only_host_access(builder.device_accesses)
		{
			// The master buffer's existing staging buffer becomes the shared, persistent CPU-writable source buffer,
			// and a new per-frame staging buffer takes its place for frame 0.
			let source_handle = self
				.buffers
				.resource(buffer_handle)
				.staging
				.expect("CpuWrite dynamic buffer must have a staging buffer");
			let frame0_staging = self.create_staging_buffer(builder.name, size);
			let buffer = self.buffers.resource_mut(buffer_handle);
			buffer.staging = Some(frame0_staging);
			buffer.source = Some(source_handle);

			// Track this dynamic buffer for automatic per-frame memcpy.
			self.persistent_write_dynamic_buffers.push(handle.into());
			Some(source_handle)
		} else {
			None
		};

		for i in 1..self.frames {
			assert!(i < 2, "This does not support more than one deferred buffer!");
			self.tasks.push(Task::new(
				Tasks::BuildBuffer(BuildBuffer {
					previous: buffer_handle,
					master: handle.into(),
					source,
				}),
				Some(i),
			));
		}

		handle
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		crate::DynamicImageHandle(self.build_image(builder.use_case(crate::UseCases::DYNAMIC)).0)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		let synchronizer_handle = graphics_hardware_interface::SynchronizerHandle(self.synchronizers.len() as u64);

		let mut previous: Option<SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(name, signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.0 as usize].next = Some(handle);
			}
			previous = Some(handle);
		}

		self.set_object_debug_name(name, synchronizer_handle.into());
		synchronizer_handle
	}
}
