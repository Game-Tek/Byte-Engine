use super::*;

impl crate::device::Device for Device {
	type Context = Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type RayTracingPipeline = crate::vulkan::factory::RayTracingPipeline;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	fn has_errors(&self) -> bool {
		self.inner.as_ref().is_some_and(InnerDevice::has_errors)
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Context::new(self)
	}

	fn create_shader(
		&mut self,
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<crate::ShaderHandle, ()> {
		let shader = crate::vulkan::Shader::new(shader_source_type, stage, descriptor_sets, self.pipeline_binaries.as_deref())?;
		self.shaders.push(shader);
		Ok(crate::ShaderHandle(self.shaders.len() as u64 - 1))
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		let (pipeline, layout) = crate::vulkan::context::build_raster_pipeline(
			&self.device,
			&self.descriptor_heap_properties,
			self.pipeline_binaries.as_deref(),
			&self.shaders,
			builder,
		);
		RasterPipeline { pipeline, layout }
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		crate::vulkan::context::build_compute_pipeline(
			&self.device,
			&self.descriptor_heap_properties,
			self.pipeline_binaries.as_deref(),
			&self.shaders,
			builder,
		)
	}

	/// Links a ray tracing pipeline on the calling thread, compiling only the shaders this factory didn't compile for
	/// an earlier version of it.
	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> Self::RayTracingPipeline {
		let ray_tracing = self.ray_tracing.as_deref().expect(
			"Vulkan ray tracing pipeline creation failed. The most likely cause is that the device was created without the ray tracing feature, or that its GPU does not support ray tracing. Check Context::supports_ray_tracing first, and request Features::ray_tracing when you create the device.",
		);
		crate::vulkan::ray_tracing::build_ray_tracing_pipeline(
			&self.device,
			ray_tracing,
			&self.descriptor_heap_properties,
			self.pipeline_binaries.as_deref(),
			&self.shaders,
			&mut self.ray_tracing_libraries,
			builder,
		)
	}
}

/// Returns whether a resource should get its own allocation: the driver either requires it or runs faster with it.
fn wants_dedicated_allocation(requirements: &vk::MemoryDedicatedRequirements) -> bool {
	requirements.prefers_dedicated_allocation != vk::FALSE || requirements.requires_dedicated_allocation != vk::FALSE
}

impl InnerDevice {
	/// Waits until every queue of the device is idle.
	///
	/// Vulkan forbids submitting on any queue during the wait, so this holds the lock of every queue, in index order.
	/// Submission paths hold one queue lock at a time, so the order cannot deadlock.
	pub(crate) fn wait(&self) {
		let _queues = self
			.vk_queues
			.iter()
			.map(|queue| {
				queue.lock().expect(
					"Failed to lock a Vulkan queue for a device wait. The most likely cause is that another thread panicked while holding the queue lock.",
				)
			})
			.collect::<smallvec::SmallVec<[_; 4]>>();
		unsafe { self.device.device_wait_idle() }
			.expect("Failed to wait for the Vulkan device. The most likely cause is that the device was lost.");
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

		let (memory_requirements, dedicated) = self.buffer_memory_requirements(buffer);

		MemoryBackedResourceCreationResult {
			resource: buffer,
			size: memory_requirements.size as usize,
			memory_flags: memory_requirements.memory_type_bits,
			alignment: memory_requirements.alignment,
			dedicated,
		}
	}

	/// Reports a buffer's memory requirements and whether the driver prefers or requires a dedicated allocation for it.
	pub(crate) fn buffer_memory_requirements(&self, buffer: vk::Buffer) -> (vk::MemoryRequirements, bool) {
		let mut dedicated_requirements = vk::MemoryDedicatedRequirements::default();
		let mut memory_requirements = vk::MemoryRequirements2::default().push(&mut dedicated_requirements);
		let info = vk::BufferMemoryRequirementsInfo2::default().buffer(buffer);
		unsafe { self.device.get_buffer_memory_requirements2(&info, &mut memory_requirements) };
		let memory_requirements = memory_requirements.memory_requirements;
		(memory_requirements, wants_dedicated_allocation(&dedicated_requirements))
	}

	/// Reports an image's memory requirements and whether the driver prefers or requires a dedicated allocation for it.
	pub(crate) fn image_memory_requirements(&self, image: vk::Image) -> (vk::MemoryRequirements, bool) {
		let mut dedicated_requirements = vk::MemoryDedicatedRequirements::default();
		let mut memory_requirements = vk::MemoryRequirements2::default().push(&mut dedicated_requirements);
		let info = vk::ImageMemoryRequirementsInfo2::default().image(image);
		unsafe { self.device.get_image_memory_requirements2(&info, &mut memory_requirements) };
		let memory_requirements = memory_requirements.memory_requirements;
		(memory_requirements, wants_dedicated_allocation(&dedicated_requirements))
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
		let square_2d = extent.width() == extent.height() && extent.depth().max(1) == 1;
		assert!(
			!cube_compatible || square_2d && array_layers.is_some_and(|layers| layers.get() == 6),
			"Invalid Vulkan cubemap image. The most likely cause is that cube compatibility was requested for a non-square image or an image without six faces."
		);
		assert!(
			!cube_array_compatible || square_2d && array_layers.is_some_and(|layers| layers.get().is_multiple_of(6)),
			"Invalid Vulkan cubemap-array image. The most likely cause is that cube-array compatibility was requested for a non-square image or an array layer count not divisible by six."
		);
		let cube = cube_compatible || cube_array_compatible;
		let image_create_info = vk::ImageCreateInfo::default()
			.flags(flag_if(cube, vk::ImageCreateFlags::CUBE_COMPATIBLE))
			.image_type(image_type_from_extent(extent).expect("Failed to get VkImageType from extent"))
			.format(to_format(format))
			.extent(extent_into_vk_extent(extent))
			.mip_levels(mip_levels)
			.array_layers(array_layers.map_or(1, NonZeroU32::get))
			.samples(vk::SampleCountFlags::TYPE_1)
			.tiling(vk::ImageTiling::OPTIMAL)
			.usage(into_vk_image_usage_flags(resource_uses, format))
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.initial_layout(vk::ImageLayout::UNDEFINED);

		let image = unsafe { self.device.create_image(&image_create_info, None).expect("No image") };
		let (memory_requirements, dedicated) = self.image_memory_requirements(image);
		self.set_name(image, name);

		MemoryBackedResourceCreationResult {
			resource: image,
			size: memory_requirements.size as usize,
			memory_flags: memory_requirements.memory_type_bits,
			alignment: memory_requirements.alignment,
			dedicated,
		}
	}

	/// Creates a Vulkan fence with the requested initial signal state.
	pub(crate) fn create_vulkan_fence(&self, signaled: bool) -> vk::Fence {
		let fence_create_info = vk::FenceCreateInfo::default().flags(flag_if(signaled, vk::FenceCreateFlags::SIGNALED));
		unsafe { self.device.create_fence(&fence_create_info, None).expect("No fence") }
	}

	/// Assigns a Vulkan debug name when debug utilities are available.
	#[cfg_attr(not(debug_assertions), allow(unused_variables))]
	pub(crate) fn set_name<T: vk::Handle>(&self, handle: T, name: Option<&str>) {
		#[cfg(debug_assertions)]
		if let (Some(name), Some(debug_utils)) = (name, &self.debug_utils) {
			let name = std::ffi::CString::new(name).unwrap();
			let name_info = vk::DebugUtilsObjectNameInfoEXT::default()
				.object_handle(handle)
				.object_name(&name);
			// Names are only a debugging aid, so failing to set one is ignored.
			unsafe { debug_utils.set_debug_utils_object_name(&name_info) }.ok();
		}
	}

	/// Creates a Vulkan semaphore and assigns its debug name.
	pub(crate) fn create_vulkan_semaphore(&self, name: Option<&str>) -> vk::Semaphore {
		let semaphore_create_info = vk::SemaphoreCreateInfo::default();
		let handle = unsafe { self.device.create_semaphore(&semaphore_create_info, None) }.expect("No semaphore");
		self.set_name(handle, name);
		handle
	}

	/// Creates a Vulkan image view for images with view-capable usage flags.
	///
	/// The view matches the image's dimensionality; `layer_count` selects an arrayed view of 1D or 2D images.
	pub(crate) fn create_vulkan_image_view(
		&self,
		name: Option<&str>,
		texture: &vk::Image,
		image_type: vk::ImageType,
		format: crate::Formats,
		usage: vk::ImageUsageFlags,
		mip_levels: u32,
		base_layer: u32,
		layer_count: Option<NonZeroU32>,
	) -> vk::ImageView {
		if !Self::image_usage_allows_views(usage) {
			return vk::ImageView::null();
		}

		// The default component mapping is the identity swizzle.
		let image_view_create_info = vk::ImageViewCreateInfo::default()
			.image(*texture)
			.view_type(crate::vulkan::utils::image_view_type(image_type, layer_count.is_some()))
			.format(to_format(format))
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if format.is_depth() {
					vk::ImageAspectFlags::DEPTH
				} else {
					vk::ImageAspectFlags::COLOR
				},
				base_mip_level: 0,
				level_count: mip_levels,
				base_array_layer: base_layer,
				layer_count: layer_count.map_or(1, NonZeroU32::get),
			});

		let vk_image_view = unsafe { self.device.create_image_view(&image_view_create_info, None) }.expect("No image view");
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
