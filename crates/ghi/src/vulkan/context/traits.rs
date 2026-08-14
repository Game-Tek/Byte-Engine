use super::*;

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

impl crate::context::Context for Context {
	type Queue = crate::vulkan::queue::Queue;
	type QueueReference<'a>
		= crate::vulkan::queue::QueueReference<'a>
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

	fn queue(&mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue {
		let queue = &self.queues[queue_handle.0 as usize];
		let vk_queue = queue.vk_queue.clone();
		let queue_family_index = queue.queue_family_index;
		let queue_index = queue._queue_index;
		crate::vulkan::queue::Queue {
			device: std::ptr::NonNull::from(self),
			queue_handle,
			vk_queue,
			queue_family_index,
			_queue_index: queue_index,
		}
	}

	fn queue_reference<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::QueueReference<'a> {
		crate::vulkan::queue::QueueReference {
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

		if frames > MAX_FRAMES_IN_FLIGHT as u8 {
			panic!("Cannot set frames in flight to more than {}", MAX_FRAMES_IN_FLIGHT);
		}

		let current_frames = self.frames;
		let target_frames = frames;
		let delta_frames = target_frames as i8 - current_frames as i8;

		if delta_frames > 0 {
			let to_extend = self
				.images
				.iter()
				.filter_map(|image| {
					let next = image.next?;

					let mut handle = next;

					while let Some(h) = self.images[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

			for image_handle in to_extend {
				let current_image = &self.images[image_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name: Option<&str> = None;

				#[cfg(not(debug_assertions))]
				let name = None;

				let next = current_image.next;
				let format = current_image.format_;
				let access = current_image.access;
				let array_layers = current_image.layers;
				let cube_compatible = current_image.cube_compatible;
				let cube_array_compatible = current_image.cube_array_compatible;
				let extent = current_image.extent;
				let resource_uses = current_image.uses;
				let mip_levels = current_image.mip_levels;

				let new_image = self.create_image_internal(
					next,
					None,
					name,
					format,
					access,
					array_layers,
					cube_compatible,
					cube_array_compatible,
					extent,
					resource_uses,
					mip_levels,
				);

				let current_image = &mut self.images[image_handle.0 as usize];
				current_image.next = Some(new_image);
			}

			let to_extend = self
				.synchronizers
				.iter()
				.filter_map(|synchronizer| {
					let next = synchronizer.next?;

					let mut handle = next;

					while let Some(h) = self.synchronizers[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

			for synchronizer_handle in to_extend {
				let current_synchronizer = &self.synchronizers[synchronizer_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name_owned = self
					.names
					.get(
						&graphics_hardware_interface::SynchronizerHandle(synchronizer_handle.root(&self.synchronizers).0)
							.into(),
					)
					.cloned();

				#[cfg(not(debug_assertions))]
				let name_owned: Option<String> = None;

				let name = name_owned.as_deref();
				let signaled = current_synchronizer.signaled;

				let new_synchronizer = self.create_synchronizer_internal(name, signaled);

				let current_synchronizer = &mut self.synchronizers[synchronizer_handle.0 as usize];
				current_synchronizer.next = Some(new_synchronizer);
			}

			for command_buffer in &mut self.command_buffers {
				let queue = &self.queues[command_buffer.queue_handle.0 as usize];
				let vk_queue = queue.vk_queue.clone();
				let command_pool_create_info =
					vk::CommandPoolCreateInfo::default().queue_family_index(queue.queue_family_index);

				let command_pool = unsafe {
					self.device
						.create_command_pool(&command_pool_create_info, None)
						.expect("No command pool")
				};

				let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
					.command_pool(command_pool)
					.level(vk::CommandBufferLevel::PRIMARY)
					.command_buffer_count(1);

				let command_buffers = unsafe {
					self.device
						.allocate_command_buffers(&command_buffer_allocate_info)
						.expect("No command buffer")
				};

				let vk_command_buffer = command_buffers[0];

				// self.set_name(vk_command_buffer, name);

				command_buffer.frames.push(CommandBufferInternal {
					vk_queue: vk_queue.clone(),
					command_pool,
					command_buffer: vk_command_buffer,
				});
			}
		} else {
			unimplemented!()
		}

		self.frames = target_frames;
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.get_buffer_address(buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		self.get_buffer_slice(buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		self.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
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

	fn get_image_data<'a>(&'a mut self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		Context::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>, size: usize) {
		self.resize_buffer(buffer_handle, size);
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

	fn wait(&self) {
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
		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(
			None,
			buffer_size,
			vk::BufferUsageFlags::VERTEX_BUFFER
				| vk::BufferUsageFlags::INDEX_BUFFER
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, pointer) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		unsafe {
			let vertex_buffer_pointer = pointer.expect("No pointer");
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size);
			let index_buffer_pointer = vertex_buffer_pointer.add(vertex_buffer_size.next_multiple_of(16));
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size);
		}

		let mesh_handle = graphics_hardware_interface::MeshHandle(self.meshes.len() as u64);

		self.meshes.push(Mesh {
			buffer: buffer_creation_result.resource,
			vertex_count,
			index_count,
			vertex_size: vertex_layout.size(),
		});

		mesh_handle
	}

	/// Creates a shader.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
				if !spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
					return Err(());
				}

				// SAFETY: shader was checked to be aligned to 4 bytes.
				Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
			}
			crate::shader::Sources::DXIL(_)
			| crate::shader::Sources::HLSL { .. }
			| crate::shader::Sources::MTL { .. }
			| crate::shader::Sources::MTLB { .. } => return Err(()),
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).unwrap() };

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
			descriptors: HashMap::new(),
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
		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);

		let mut specialization_map_entries = Vec::with_capacity(48);

		for specialization_map_entry in shader_parameter.specialization_map {
			// TODO: accumulate offset
			match specialization_map_entry.get_type().as_str() {
				"bool" | "u32" | "f32" => {
					specialization_map_entries.push(
						vk::SpecializationMapEntry::default()
							.constant_id(specialization_map_entry.get_constant_id())
							.offset(specialization_entries_buffer.len() as u32)
							.size(4),
					);

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec2f" => {
					for i in 0..2 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec3f" => {
					for i in 0..3 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec4f" => {
					for i in 0..4 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					assert_eq!(specialization_map_entry.get_size(), 16);

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				_ => {
					panic!("Unknown specialization map entry type");
				}
			}
		}

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
			.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			.specialization_info(&specialization_info);
		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.push(&mut descriptor_heap_flags)
			.stage(stage)
			.layout(vk::PipelineLayout::null())];

		let pipeline_handle = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("No compute pipeline")[0]
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: HashMap::new(),
		});

		handle
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let shaders = builder.shaders;
		let mut groups = Vec::with_capacity(1024);

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
				let shader = &self.shaders[stage.handle.0 as usize];

				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		for (i, shader) in shaders.iter().enumerate() {
			match shader.stage {
				crate::ShaderTypes::RayGen | crate::ShaderTypes::Miss | crate::ShaderTypes::Callable => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
							.general_shader(i as u32)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::ClosestHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(i as u32)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::AnyHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(i as u32)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::Intersection => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(i as u32),
					);
				}
				_ => {
					// warn!("Fed shader of type '{:?}' to ray tracing pipeline", shader.stage)
				}
			}
		}

		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.push(&mut descriptor_heap_flags)
			.layout(vk::PipelineLayout::null())
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let mut handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]> = HashMap::with_capacity(shaders.len());

		let pipeline_handle = unsafe {
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

			for (i, shader) in shaders.iter().enumerate() {
				let mut h = [0u8; 32];
				h.copy_from_slice(&handle_buffer[i * 32..(i + 1) * 32]);

				handles.insert(*shader.handle, h);
			}

			pipeline
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: handles,
		});

		handle
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
		let root_image_handle = self.create_image_internal(
			None,
			None,
			builder.name,
			builder.format,
			builder.device_accesses,
			builder.array_layers,
			builder.cube_compatible,
			builder.cube_array_compatible,
			builder.extent,
			builder.resource_uses,
			builder.mip_levels,
		);

		let handle =
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_image_handle.0));

		let instances = match builder.use_case {
			crate::UseCases::DYNAMIC => self.frames,
			crate::UseCases::STATIC => 1,
		};

		let mut previous = root_image_handle;
		for _ in 1..instances {
			previous = self.create_image_internal(
				None,
				Some(previous),
				builder.name,
				builder.format,
				builder.device_accesses,
				builder.array_layers,
				builder.cube_compatible,
				builder.cube_array_compatible,
				builder.extent,
				builder.resource_uses,
				builder.mip_levels,
			);
		}

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
		let size = max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

		let buffer_creation_result = self.create_vulkan_buffer(
			name,
			size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let (buffer_handle, _) = self.buffers.add(Buffer {
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size: buffer_creation_result.size,
			device_address: address,
			pointer,
			uses: crate::Uses::empty(),
			access: crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		});

		buffer_handle
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
			});

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				Some(&[max_instance_count]),
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) =
			self.create_allocation_internal(buffer.size, buffer.memory_flags.into(), crate::DeviceAccesses::GpuWrite);

		let (..) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);

		let handle =
			graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer.resource,
			});

			self.set_name(handle, name);
		}

		handle
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

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				Some(&[primitive_count]),
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer_descriptor = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_descriptor.size,
			buffer_descriptor.memory_flags.into(),
			crate::DeviceAccesses::GpuWrite,
		);

		let (..) = self.bind_vulkan_buffer_memory(&buffer_descriptor, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer_descriptor.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

		let handle =
			graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer_descriptor.resource,
			});
		}

		handle
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::BufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData::<T> {},
		);

		return handle;
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData::<T> {},
		);

		if super::buffer::PERSISTENT_WRITE
			&& builder.device_accesses.intersects(crate::DeviceAccesses::CpuWrite)
			&& !Self::uses_only_host_access(builder.device_accesses)
		{
			// The master buffer's existing staging buffer becomes the shared, persistent
			// CPU-writable source buffer. We create a new per-frame staging buffer for
			// frame 0 and store the source handle on the master buffer.

			let source_handle = self
				.buffers
				.resource(buffer_handle)
				.staging
				.expect("CpuWrite dynamic buffer must have a staging buffer");

			// Create a new per-frame staging buffer for frame 0
			let frame0_staging = self.create_staging_buffer(builder.name, size);

			// Reassign: the master's staging now points to the new per-frame staging,
			// and source points to the original (persistent) CPU-writable buffer.
			let buffer = self.buffers.resource_mut(buffer_handle);
			buffer.staging = Some(frame0_staging);
			buffer.source = Some(source_handle);

			// Track this dynamic buffer for automatic per-frame memcpy
			self.persistent_write_dynamic_buffers.push(handle.into());

			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: Some(source_handle),
					}),
					Some(i),
				));
			}
		} else {
			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: None,
					}),
					Some(i),
				));
			}
		}

		handle
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));

		crate::DynamicImageHandle(handle.0)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		let synchronizer_handle = graphics_hardware_interface::SynchronizerHandle(self.synchronizers.len() as u64);

		{
			let mut previous: Option<SynchronizerHandle> = None;

			for _ in 0..self.frames {
				let synchronizer_handle = self.create_synchronizer_internal(name, signaled);

				if let Some(pr) = previous {
					self.synchronizers[pr.0 as usize].next = Some(synchronizer_handle);
				}

				previous = Some(synchronizer_handle);
			}
		}

		self.set_object_debug_name(name, synchronizer_handle.into());

		synchronizer_handle
	}
}
