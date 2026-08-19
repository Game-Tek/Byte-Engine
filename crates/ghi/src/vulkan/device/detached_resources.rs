use super::*;

impl crate::device::Device for Device {
	type Context = Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type Image = FactoryImage;
	type Sampler = FactorySampler;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		self.inner.as_ref().is_some_and(InnerDevice::has_errors)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Context::new(&self)
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<crate::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
				if spirv.as_ptr().is_aligned_to(std::mem::align_of::<u32>()) {
					Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
				} else {
					let mut words = Vec::with_capacity(spirv.len() / 4);
					for chunk in spirv.chunks_exact(4) {
						words.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
					}
					Cow::Owned(words)
				}
			}
			crate::shader::Sources::DXIL(_)
			| crate::shader::Sources::HLSL { .. }
			| crate::shader::Sources::MTL { .. }
			| crate::shader::Sources::MTLB { .. } => return Err(()),
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);
		let shader_module = unsafe {
			self.device
				.create_shader_module(&shader_module_create_info, None)
				.map_err(|_| ())?
		};
		let handle = crate::ShaderHandle(self.shaders.len() as u64);

		self.shaders.push(crate::vulkan::Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
		});

		Ok(handle)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		// Detached builders borrow caller data, so retain owned state until the render frame interns the pipeline.
		RasterPipeline {
			name: crate::debug_name(builder.name),
			push_constant_ranges: builder.push_constant_ranges.into_owned(),
			vertex_elements: builder
				.vertex_elements
				.iter()
				.map(|element| FactoryVertexElement {
					name: element.name.to_owned(),
					format: element.format,
					binding: element.binding,
				})
				.collect(),
			shaders: builder
				.shaders
				.iter()
				.map(|shader| FactoryShaderParameter {
					handle_index: shader.handle.0 as usize,
					stage: shader.stage,
					specialization_map: shader.specialization_map.to_vec(),
				})
				.collect(),
			render_targets: builder.render_targets.into_owned(),
			face_winding: builder.face_winding,
			cull_mode: builder.cull_mode,
			depth_write: builder.depth_write,
			factory_shaders: self.shaders.clone(),
		}
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		self.create_compute_pipeline_with_resources(builder, &self.shaders)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> Self::Image {
		FactoryImage {
			name: crate::debug_name(builder.name),
			extent: builder.extent,
			format: builder.format,
			resource_uses: builder.resource_uses,
			device_accesses: builder.device_accesses,
			use_case: builder.use_case,
			array_layers: builder.array_layers,
			cube_compatible: builder.cube_compatible,
			cube_array_compatible: builder.cube_array_compatible,
		}
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> Self::Sampler {
		FactorySampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		}
	}
}

impl InnerDevice {
	#[inline]
	pub(crate) fn start_frame_capture(&mut self) {
		// #[cfg(debug_assertions)]
		// self.debugger.start_frame_capture();
	}

	#[inline]
	pub(crate) fn end_frame_capture(&mut self) {
		// #[cfg(debug_assertions)]
		// self.debugger.end_frame_capture();
	}

	pub(crate) fn wait(&self) {
		unsafe {
			self.device.device_wait_idle().unwrap();
		}
	}

	/// Creates a Vulkan buffer and reports the memory requirements needed to bind it.
	pub(crate) fn create_vulkan_buffer(
		&self,
		name: Option<&str>,
		size: usize,
		usage: vk::BufferUsageFlags,
	) -> MemoryBackedResourceCreationResult<vk::Buffer> {
		let buffer_create_info = vk::BufferCreateInfo::default()
			.size(size as u64)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(usage);

		let buffer = unsafe { self.device.create_buffer(&buffer_create_info, None).expect("No buffer") };

		self.set_name(buffer, name);

		let memory_requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };

		MemoryBackedResourceCreationResult {
			resource: buffer,
			size: memory_requirements.size as usize,
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	/// Creates a Vulkan image and reports the memory requirements needed to bind it.
	pub(crate) fn create_vulkan_texture(
		&self,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		mip_levels: u32,
		array_layers: Option<NonZeroU32>,
		cube_compatible: bool,
		cube_array_compatible: bool,
	) -> MemoryBackedResourceCreationResult<vk::Image> {
		if cube_compatible {

			assert!(
				array_layers.is_some_and(|layers| layers.get() == 6)
					&& extent.width() == extent.height()
					&& extent.depth().max(1) == 1,
				"Invalid Vulkan cubemap image. The most likely cause is that cube compatibility was requested for a non-square image or an image without six faces."
			);
		}
		if cube_array_compatible {

			assert!(
				array_layers.is_some_and(|layers| layers.get().is_multiple_of(6))
					&& extent.width() == extent.height()
					&& extent.depth().max(1) == 1,
				"Invalid Vulkan cubemap-array image. The most likely cause is that cube-array compatibility was requested for a non-square image or an array layer count not divisible by six."
			);
		}
		let image_create_info = vk::ImageCreateInfo::default()
			.flags(if cube_compatible || cube_array_compatible {
				vk::ImageCreateFlags::CUBE_COMPATIBLE
			} else {
				vk::ImageCreateFlags::empty()
			})
			.image_type(image_type_from_extent(extent).expect("Failed to get VkImageType from extent"))
			.format(to_format(format))
			.extent(extent_into_vk_extent(extent))
			.mip_levels(mip_levels)
			.array_layers(array_layers.map(|e| e.get()).unwrap_or(1))
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(vk::ImageTiling::OPTIMAL)
			.usage(into_vk_image_usage_flags(resource_uses, format))
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED);

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };

		let memory_requirements = unsafe { self.device.get_image_memory_requirements(image) };

		self.set_name(image, name);

		MemoryBackedResourceCreationResult {
			resource: image.to_owned(),
			size: memory_requirements.size as usize,
			memory_flags: memory_requirements.memory_type_bits,
		}
	}

	/// Creates a Vulkan fence with the requested initial signal state.
	pub(crate) fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
		let fence_create_info = vk::FenceCreateInfo::default().flags(
			vk::FenceCreateFlags::empty()
				| if signaled {
					vk::FenceCreateFlags::SIGNALED
				} else {
					vk::FenceCreateFlags::empty()
				},
		);
		unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") }
	}

	/// Assigns a Vulkan debug name when debug utilities are available.
	pub(crate) fn set_name<T: vk::Handle>(&self, handle: T, name: Option<&str>) {
		#[cfg(debug_assertions)]
		if let Some(name) = name {
			let name = std::ffi::CString::new(name).unwrap();
			let name = name.as_c_str();
			unsafe {
				if let Some(debug_utils) = &self.debug_utils {
					debug_utils
						.set_debug_utils_object_name(
							&vk::DebugUtilsObjectNameInfoEXT::default()
								.object_handle(handle)
								.object_name(name),
						)
						.ok();
					// Ignore errors, if the name can't be set, it's not a big deal.
				}
			}
		}
	}

	/// Creates a Vulkan semaphore and assigns its debug name.
	pub(crate) fn create_vulkan_semaphore(&self, name: Option<&str>, _: bool) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default();
		let handle = unsafe {
			self.device
				.create_semaphore(&semaphore_create_info, None)
				.expect("No semaphore")
		};

		self.set_name(handle, name);

		handle
	}

	/// Creates a Vulkan image view for images with view-capable usage flags.
	pub(crate) fn create_vulkan_image_view(
		&self,
		name: Option<&str>,
		texture: &vk::Image,
		format: crate::Formats,
		usage: vk::ImageUsageFlags,
		mip_levels: u32,
		base_layer: u32,
		layer_count: Option<NonZeroU32>,
	) -> vk::ImageView {
		if !Self::image_usage_allows_views(usage) {
			return vk::ImageView::null();
		}

		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(if layer_count.is_none() {
				vk::ImageViewType::TYPE_2D
			} else {
				vk::ImageViewType::TYPE_2D_ARRAY
			})
			.format(to_format(format))
			.components(vk::ComponentMapping {
				r: vk::ComponentSwizzle::IDENTITY,
				g: vk::ComponentSwizzle::IDENTITY,
				b: vk::ComponentSwizzle::IDENTITY,
				a: vk::ComponentSwizzle::IDENTITY,
			})
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if !format.is_depth() {
					vk::ImageAspectFlags::COLOR
				} else {
					vk::ImageAspectFlags::DEPTH
				},
				base_mip_level: 0,
				level_count: mip_levels,
				base_array_layer: base_layer,
				layer_count: layer_count.map(|e| e.get()).unwrap_or(1),
			});

		let vk_image_view = unsafe {
			self.device
				.create_image_view(&image_view_create_info, None)
				.expect("No image view")
		};

		self.set_name(vk_image_view, name);

		vk_image_view
	}

	pub(crate) fn image_usage_allows_views(usage: vk::ImageUsageFlags) -> bool {
		usage.intersects(
			vk::ImageUsageFlags::SAMPLED
				| vk::ImageUsageFlags::STORAGE
				| vk::ImageUsageFlags::COLOR_ATTACHMENT
				| vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
				| vk::ImageUsageFlags::TRANSIENT_ATTACHMENT
				| vk::ImageUsageFlags::INPUT_ATTACHMENT
				| vk::ImageUsageFlags::FRAGMENT_SHADING_RATE_ATTACHMENT_KHR
				| vk::ImageUsageFlags::FRAGMENT_DENSITY_MAP_EXT,
		)
	}
}

impl Device {
	/// Creates a detached compute pipeline whose flat bindings map directly into descriptor heaps.
	pub(crate) fn create_compute_pipeline_with_resources(
		&self,
		builder: crate::pipelines::compute::Builder,
		shaders: &[crate::vulkan::Shader],
	) -> ComputePipeline {
		let shader_parameter = builder.shader;
		let shader = &shaders[shader_parameter.handle.0 as usize];
		let stage_resources = [(shader.stage, shader.shader_resource_descriptors.clone())];
		let layout = crate::vulkan::build_pipeline_layout(
			&stage_resources,
			builder.push_constant_ranges,
			&self.descriptor_heap_properties,
		);
		let mappings = crate::vulkan::build_shader_mappings(&layout, &shader.shader_resource_descriptors);
		let mut mapping_info = vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings);
		let (specialization_entries_buffer, specialization_map_entries) =
			build_specialization_entries(shader_parameter.specialization_map);
		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);
		let stage = vk::PipelineShaderStageCreateInfo::default()
			.push(&mut mapping_info)
			.stage(vk::ShaderStageFlags::COMPUTE)
			.module(shader.shader)
			.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			.specialization_info(&specialization_info);
		let mut flags = vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.push(&mut flags)
			.stage(stage)
			.layout(vk::PipelineLayout::null())];
		let pipeline = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("Vulkan descriptor-heap compute pipeline creation failed. The most likely cause is an invalid shader resource mapping or specialization constant.")[0]
		};
		let mut shader_handles = HashMap::default();
		shader_handles.insert(*shader_parameter.handle, [0; 32]);

		ComputePipeline {
			pipeline,
			layout,
			shader_handles,
		}
	}
}

fn build_specialization_entries(
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
	let mut data = Vec::<u8>::with_capacity(256);
	let mut entries = Vec::with_capacity(48);

	for specialization_map_entry in specialization_map {
		let scalar_count = match specialization_map_entry.get_type().as_str() {
			"bool" | "u32" | "f32" => 1,
			"vec2f" => 2,
			"vec3f" => 3,
			"vec4f" => 4,
			_ => panic!(
				"Unsupported Vulkan specialization constant type. The most likely cause is that the Vulkan backend was not updated for a new specialization entry type."
			),
		};
		let offset = data.len() as u32;
		for i in 0..scalar_count {
			entries.push(
				vk::SpecializationMapEntry::default()
					.constant_id(specialization_map_entry.get_constant_id() + i)
					.offset(offset + i * 4)
					.size(4),
			);
		}
		data.extend_from_slice(specialization_map_entry.get_data());
	}

	(data, entries)
}
