use ash::vk;
use utils::Extent;

use super::{command_buffer::CommandBufferRecording, context::Context};
use crate::{
	FrameKey, HandleLike as _, MasterHandle as _,
	context::ContextCreate as _,
	graphics_hardware_interface,
	vulkan::{BufferCopy, ImageCopy, ImageHandle, Swapchain, Synchronizer, Tasks},
};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut Context,
	acquired_swapchains: Vec<crate::PresentKey>,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut Context, frame_key: FrameKey) -> Self {
		Self {
			frame_key,
			device,
			acquired_swapchains: Vec::new(),
		}
	}

	pub fn device(&self) -> &Context {
		self.device
	}

	pub fn device_mut(&mut self) -> &mut Context {
		self.device
	}

	pub(crate) fn execute_submission(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		states: utils::hash::HashMap<super::Handles, super::TransitionState>,
		buffer_states: utils::hash::HashMap<super::Handles, Vec<super::BufferTransitionState>>,
		texture_readbacks: smallvec::SmallVec<[graphics_hardware_interface::TextureCopyHandle; 4]>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: Option<graphics_hardware_interface::SynchronizerHandle>,
	) {
		let command_buffer = self.device.command_buffers[command_buffer_handle.0 as usize].frames
			[self.frame_key.sequence_index as usize]
			.clone();

		let command_buffers = [command_buffer.command_buffer];

		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffers[0])];

		let wait_for_synchronizer_handles: [graphics_hardware_interface::SynchronizerHandle; 0] = [];

		let wait_semaphores = wait_for_synchronizer_handles
			.iter()
			.map(|&synchronizer| {
				vk::SemaphoreSubmitInfo::default()
					.semaphore(self.get_synchronizer(synchronizer).semaphore)
					.stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE | vk::PipelineStageFlags2::TRANSFER)
			})
			.chain(present_keys.iter().map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let semaphore = swapchain.acquire_synchronizers[present_key.sequence_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;

				vk::SemaphoreSubmitInfo::default()
					.semaphore(semaphore)
					.stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
			}))
			.collect::<Vec<_>>();

		let signal_synchronizer_handles: [graphics_hardware_interface::SynchronizerHandle; 0] = [];

		let signal_semaphores = signal_synchronizer_handles
			.iter()
			.map(|&synchronizer| {
				vk::SemaphoreSubmitInfo::default()
					.semaphore(self.get_synchronizer(synchronizer).semaphore)
					.stage_mask(vk::PipelineStageFlags2::empty())
			})
			.chain(present_keys.iter().map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let presentable_image_handle = self.get_presentable_swapchain_image_handle(*present_key);
				let wait_stage = states
					.get(&super::Handles::Image(presentable_image_handle))
					.map(|state| state.stage)
					.unwrap_or(vk::PipelineStageFlags2::ALL_COMMANDS);

				vk::SemaphoreSubmitInfo::default()
					.semaphore(
						swapchain.submit_synchronizers[present_key.image_index as usize]
							.access(&self.device.synchronizers)
							.semaphore,
					)
					.stage_mask(wait_stage)
			}))
			.collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores);

		let execution_completion_fence = synchronizer
			.map(|synchronizer| self.get_synchronizer(synchronizer).fence)
			.unwrap_or(vk::Fence::null());

		let vk_queue = command_buffer
			.vk_queue
			.lock()
			.expect("Failed to lock Vulkan queue for frame submission. The most likely cause is that another thread panicked while holding the queue lock.");

		unsafe {
			self.device
				.device
				.queue_submit2(*vk_queue, &[submit_info], execution_completion_fence)
				.expect("Failed to submit command buffer.");
		}
		if let Some(synchronizer) = synchronizer {
			self.get_synchronizer_mut(synchronizer).armed = true;
		}
		for handle in texture_readbacks {
			self.device.texture_readbacks.mark_submitted(handle);
		}

		for presentation in present_keys {
			let swapchain = self.get_swapchain(presentation.swapchain);

			// Binary semaphores are consumed by one wait, so each present waits only on its own image's render semaphore.
			let wait_semaphores = [swapchain.submit_synchronizers[presentation.image_index as usize]
				.access(&self.device.synchronizers)
				.semaphore];
			let swapchains = [swapchain.swapchain];
			let image_indices = [presentation.image_index as u32];

			let present_info = vk::PresentInfoKHR::default()
				.swapchains(&swapchains)
				.wait_semaphores(&wait_semaphores)
				.image_indices(&image_indices);

			match unsafe { self.device.swapchain.queue_present(*vk_queue, &present_info) } {
				Ok(false) => {}
				Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
					self.device.swapchains[presentation.swapchain.0 as usize].needs_recreation = true;
				}
				Err(error) => panic!(
					"Failed to present a Vulkan swapchain image ({error:?}). The most likely cause is that the surface or the device was lost."
				),
			}
		}

		for (k, v) in states {
			self.device.states.insert(k, v);
		}
		for (k, v) in buffer_states {
			self.device.buffer_states.insert(k, v);
		}
	}

	pub(crate) fn complete_without_submissions(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		let synchronizer = self.get_synchronizer(synchronizer_handle);
		{
			let queue = self.device.queues[0]
				.vk_queue
				.lock()
				.expect("Failed to lock Vulkan queue for empty frame submission. The most likely cause is that another thread panicked while holding the queue lock.");
			let submit_info = vk::SubmitInfo2::default();

			unsafe {
				self.device
					.device
					.queue_submit2(*queue, &[submit_info], synchronizer.fence)
					.expect("Failed to submit empty Vulkan frame. The most likely cause is that the completion fence is invalid.");
			}
		}
		self.get_synchronizer_mut(synchronizer_handle).armed = true;
	}

	fn get_current_image_handle(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		let handles = ImageHandle(image_handle.index()).get_all(&self.device.images);
		handles[(self.frame_key.sequence_index as usize).rem_euclid(handles.len())]
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> crate::FrameKey {
		self.frame_key
	}

	fn get_mut_buffer_slice<T: crate::Pod>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		self.device
			.get_texture_slice_mut(crate::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(
				self.get_current_image_handle(texture_handle).0,
			)))
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		self.device
			.sync_texture(crate::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(
				self.get_current_image_handle(image_handle).0,
			)));
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	/// Acquires an image, recreating the swapchain when it no longer matches its surface.
	///
	/// Returns a zero extent when no image could be acquired, such as while the window is minimized; callers
	/// must skip rendering and presentation for that swapchain this frame.
	fn acquire_swapchain_image(&mut self, swapchain_handle: crate::SwapchainHandle) -> (crate::PresentKey, utils::Extent) {
		let sequence_index = self.frame_key.sequence_index;
		let unavailable = (
			graphics_hardware_interface::PresentKey {
				image_index: 0,
				sequence_index,
				swapchain: swapchain_handle,
			},
			Extent::rectangle(0, 0),
		);

		let capabilities = self.query_swapchain_capabilities(swapchain_handle);
		let swapchain = self.get_swapchain(swapchain_handle);
		let extent_changed =
			capabilities.current_extent.width != u32::MAX && capabilities.current_extent != swapchain.extent;
		if (swapchain.needs_recreation || extent_changed) && !self.device.recreate_swapchain(swapchain_handle, &capabilities) {
			return unavailable;
		}

		let mut recreated = false;
		let index = loop {
			match self.acquire_next_swapchain_image(swapchain_handle) {
				Ok((index, suboptimal)) => {
					// The acquired image is still presentable, so rebuild on the next acquire instead of discarding it.
					if suboptimal {
						self.device.swapchains[swapchain_handle.0 as usize].needs_recreation = true;
					}
					break index;
				}
				Err(vk::Result::ERROR_OUT_OF_DATE_KHR) if !recreated => {
					recreated = true;
					let capabilities = self.query_swapchain_capabilities(swapchain_handle);
					if !self.device.recreate_swapchain(swapchain_handle, &capabilities) {
						return unavailable;
					}
				}
				Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
					self.device.swapchains[swapchain_handle.0 as usize].needs_recreation = true;
					return unavailable;
				}
				Err(error) => panic!(
					"Failed to acquire a Vulkan swapchain image ({error:?}). The most likely cause is that the surface or the device was lost."
				),
			}
		};

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: index as u8,
			sequence_index,
			swapchain: swapchain_handle,
		};

		if !self.acquired_swapchains.contains(&present_key) {
			self.acquired_swapchains.push(present_key);
		}

		let swapchain = &mut self.device.swapchains[swapchain_handle.0 as usize];
		swapchain.acquired_image_indices[sequence_index as usize] = index as u8;

		(present_key, Extent::rectangle(swapchain.extent.width, swapchain.extent.height))
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		let current_frame = self.frame_key.sequence_index;
		let image_handles = ImageHandle(image_handle.index()).get_all(&self.device.images);
		let handle = image_handles[(current_frame as usize).rem_euclid(image_handles.len())];

		self.device.resize_image_internal(handle, extent, current_frame);

		self.device
			.add_task_to_all_other_frames(Tasks::ResizeImage { handle, extent }, current_frame);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.create_command_buffer_recording_internal(command_buffer_handle, true)
	}

	fn create_command_buffer_recording_without_implicit_sync<'record>(
		&'record mut self,
		command_buffer_handle: crate::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.create_command_buffer_recording_internal(command_buffer_handle, false)
	}

	fn get_mut_dynamic_buffer_slice<T: crate::Pod>(&mut self, buffer_handle: crate::DynamicBufferHandle<T>) -> &mut T {
		let buffers = &self.device.buffers;
		let frame_key = self.frame_key;

		let handle = buffers
			.nth_handle(buffer_handle.into(), frame_key.sequence_index as _)
			.unwrap();
		let buffer = buffers.resource(handle);

		let (pointer, byte_count) = if super::buffer::PERSISTENT_WRITE
			&& let Some(source_handle) = buffer.source
		{
			// The persistent source receives user writes. Frame recording copies it to the current staging buffer.
			let source_buffer = buffers.resource(source_handle);
			(source_buffer.pointer.0, source_buffer.size)
		} else if let Some(staging_handle) = buffer.staging {
			self.device.pending_buffer_syncs.insert(handle);
			let staging_buffer = buffers.resource(staging_handle);
			(staging_buffer.pointer.0, staging_buffer.size)
		} else {
			(buffer.pointer.0, buffer.size)
		};
		let pointer = crate::buffer::typed_buffer_pointer::<T>(pointer, byte_count).expect(
			"Failed to map a typed Vulkan frame buffer. The most likely cause is that the frame-local buffer has no sufficiently large, aligned CPU-visible storage.",
		);
		// SAFETY: The validated pointer addresses initialized POD storage and the frame owns exclusive access to its sequence resource.
		unsafe { &mut *pointer }
	}
}

impl Frame<'_> {
	fn create_command_buffer_recording_internal(
		&mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		include_implicit_sync: bool,
	) -> CommandBufferRecording<'_> {
		let frame_key = self.frame_key;

		// Update descriptors before creating command buffer
		self.device.process_tasks(frame_key.sequence_index);

		// When PERSISTENT_WRITE is enabled, memcpy from each dynamic buffer's
		// persistent source buffer into the current frame's staging buffer, then
		// enqueue the staging→GPU copy. This ensures every frame gets the latest
		// data even if the CPU didn't write this frame.
		if include_implicit_sync && super::buffer::PERSISTENT_WRITE {
			for master_handle in &self.device.persistent_write_dynamic_buffers {
				let frame_buffer_handle = self
					.device()
					.buffers
					.nth_handle(*master_handle, frame_key.sequence_index as _)
					.unwrap();
				let frame_buffer = self.device().buffers.resource(frame_buffer_handle);

				let source_handle = frame_buffer
					.source
					.expect("Persistent write dynamic buffer must have a source");
				let staging_handle = frame_buffer
					.staging
					.expect("Persistent write dynamic buffer must have per-frame staging");

				let source_buffer = self.device().buffers.resource(source_handle);
				let staging_buffer = self.device().buffers.resource(staging_handle);
				let size = frame_buffer.size;

				if size != 0 {
					assert!(
						size <= source_buffer.size
							&& size <= staging_buffer.size
							&& !source_buffer.pointer.0.is_null()
							&& !staging_buffer.pointer.0.is_null(),
						"Failed to copy a persistent Vulkan buffer. The most likely cause is that its source or frame-local staging allocation is missing mapped storage.",
					);
					// SAFETY: The source and staging buffers are distinct live allocations, and `size` is bounded by both.
					unsafe {
						std::ptr::copy_nonoverlapping(source_buffer.pointer.0, staging_buffer.pointer.0, size);
					}
				}

				// Enqueue staging → GPU copy
				self.device.pending_buffer_syncs.insert(frame_buffer_handle);
			}
		}

		let (buffer_copies, image_copies): (Vec<_>, Vec<_>) = if include_implicit_sync {
			let pending_buffers = &mut self.device.pending_buffer_syncs;
			let buffers = &self.device.buffers;

			let buffer_copies = pending_buffers
				.drain()
				.filter_map(|e| {
					let dst_buffer_handle = e;

					let dst_buffer = buffers.resource(dst_buffer_handle);
					let src_buffer_handle = dst_buffer.staging?;

					Some(BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size))
				})
				.collect();

			let pending_images = &mut self.device.pending_image_syncs;
			let images = &self.device.images;

			let image_copies = pending_images
				.drain()
				.map(|e| {
					let dst_image_handle = e;

					let dst_image = &images[dst_image_handle.0 as usize];

					ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
				})
				.collect();

			(buffer_copies, image_copies)
		} else {
			// Explicit transfer command buffers must not consume frame-global pending
			// uploads. Those uploads belong to the normal render recording path, and
			// stealing them here makes helper transfer submissions write render-frame
			// resources such as dynamic view buffers.
			(Vec::new(), Vec::new())
		};

		let mut recording = CommandBufferRecording::new(self.device, command_buffer_handle, frame_key.into());

		recording.sync_buffers(buffer_copies.iter().copied());
		recording.sync_textures(image_copies.iter().copied());

		recording
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}

impl<'a> Frame<'a> {
	/// Interns a factory-built raster pipeline into this frame's device.
	pub fn intern_raster_pipeline(
		&mut self,
		pipeline: crate::implementation::RasterPipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		// Pipelines from one factory share shader modules, and the context destroys each entry once, so reuse interned modules.
		let shader_handles = pipeline
			.factory_shaders
			.into_iter()
			.map(|shader| {
				let index = self
					.device
					.shaders
					.iter()
					.position(|interned| interned.shader == shader.shader)
					.unwrap_or_else(|| {
						self.device.shaders.push(shader);
						self.device.shaders.len() - 1
					});
				graphics_hardware_interface::ShaderHandle(index as u64)
			})
			.collect::<Vec<_>>();
		let vertex_elements = pipeline
			.vertex_elements
			.iter()
			.map(|element| crate::pipelines::VertexElement::new(&element.name, element.format, element.binding))
			.collect::<Vec<_>>();
		let shaders = pipeline
			.shaders
			.iter()
			.map(|shader| {
				let handle = shader_handles.get(shader.handle_index).expect(
					"Missing Vulkan factory shader. The most likely cause is that the detached raster pipeline references a shader from another factory.",
				);
				crate::pipelines::ShaderParameter::new(handle, shader.stage)
					.with_specialization_map(&shader.specialization_map)
			})
			.collect::<Vec<_>>();
		let mut builder = crate::pipelines::raster::Builder::new(
			&pipeline.push_constant_ranges,
			&vertex_elements,
			&shaders,
			&pipeline.render_targets,
		)
		.face_winding(pipeline.face_winding)
		.cull_mode(pipeline.cull_mode)
		.fill_mode(pipeline.fill_mode)
		.depth_write(pipeline.depth_write);
		if let Some(name) = pipeline.name.as_deref() {
			builder = builder.name(name);
		}

		self.device.create_raster_pipeline(builder)
	}

	/// Interns a factory-built compute pipeline into this frame's device.
	pub fn intern_compute_pipeline(
		&mut self,
		pipeline: crate::implementation::ComputePipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		let layout_handle = graphics_hardware_interface::PipelineLayoutHandle(self.device.pipeline_layouts.len() as u64);
		self.device.pipeline_layouts.push(pipeline.layout);
		let handle = graphics_hardware_interface::PipelineHandle(self.device.pipelines.len() as u64);
		self.device.pipelines.push(crate::vulkan::Pipeline {
			pipeline: pipeline.pipeline,
			layout: layout_handle,
			shader_handles: pipeline.shader_handles,
		});

		handle
	}

	/// Interns a factory-built image through this frame's device.
	pub fn intern_image(&mut self, image: crate::implementation::FactoryImage) -> graphics_hardware_interface::ImageHandle {
		self.device.intern_image(image)
	}

	/// Updates retained descriptor-set state before command recording.
	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	/// Interns a factory-built sampler through this frame's device.
	pub fn intern_sampler(
		&mut self,
		sampler: crate::implementation::FactorySampler,
	) -> graphics_hardware_interface::SamplerHandle {
		self.device.intern_sampler(sampler)
	}

	pub(crate) fn get_synchronizer(
		&self,
		syncronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> &Synchronizer {
		&self.device.synchronizers
			[self.device.get_syncronizer_handles(syncronizer_handle)[self.frame_key.sequence_index as usize].0 as usize]
	}

	fn query_swapchain_capabilities(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> vk::SurfaceCapabilitiesKHR {
		let swapchain = self.get_swapchain(swapchain_handle);
		self.device
			.device
			.query_swapchain_surface_capabilities(swapchain.surface, swapchain.vk_present_mode)
	}

	fn acquire_next_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Result<(u32, bool), vk::Result> {
		let swapchain = self.get_swapchain(swapchain_handle);
		let synchronizer_index = swapchain.acquire_synchronizers[self.frame_key.sequence_index as usize].0 as usize;
		let synchronizer = &self.device.synchronizers[synchronizer_index];

		// Only one image can be held at a time when the swapchain has no spare images, so poll instead of blocking in the driver.
		let use_vulkan_timeout = swapchain.max_image_count > swapchain.min_image_count;

		let acquire_info = vk::AcquireNextImageInfoKHR::default()
			.swapchain(swapchain.swapchain)
			.timeout(if use_vulkan_timeout { u64::MAX } else { 0 })
			.semaphore(synchronizer.semaphore)
			.device_mask(1)
			.fence(synchronizer.fence);

		unsafe {
			if synchronizer.armed {
				self.device.device.wait_for_fences(&[synchronizer.fence], true, u64::MAX).expect(
					"Failed to wait for the Vulkan swapchain acquire fence. The most likely cause is that the device was lost.",
				);
			}
			self.device.device.reset_fences(&[synchronizer.fence]).expect(
				"Failed to reset the Vulkan swapchain acquire fence. The most likely cause is that the device was lost.",
			);
		}

		let result = loop {
			let result = unsafe { self.device.swapchain.acquire_next_image2(&acquire_info) };
			match result {
				Err(vk::Result::NOT_READY | vk::Result::TIMEOUT) if !use_vulkan_timeout => {
					std::thread::sleep(std::time::Duration::from_millis(1))
				}
				result => break result,
			}
		};

		// A failed acquire never signals the fence, so a later wait on it must be skipped.
		self.device.synchronizers[synchronizer_index].armed = result.is_ok();
		result
	}

	/// Keeps only keys whose images were acquired this frame, since presenting any other image is invalid.
	pub(crate) fn acquired_present_keys(
		&self,
		present_keys: &[graphics_hardware_interface::PresentKey],
	) -> smallvec::SmallVec<[graphics_hardware_interface::PresentKey; 4]> {
		present_keys
			.iter()
			.copied()
			.filter(|present_key| self.acquired_swapchains.contains(present_key))
			.collect()
	}

	fn get_synchronizer_mut(&mut self, syncronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> &mut Synchronizer {
		let index = self.device.get_syncronizer_handles(syncronizer_handle)[self.frame_key.sequence_index as usize].0 as usize;
		&mut self.device.synchronizers[index]
	}

	pub(crate) fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.device.swapchains[swapchain_handle.0 as usize]
	}

	pub(crate) fn get_presentable_swapchain_image_handle(
		&self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> ImageHandle {
		let swapchain = self.get_swapchain(present_key.swapchain);
		swapchain.native_images[present_key.image_index as usize]
	}
}

impl Context {
	/// Interns a factory-built image into this context, for loader threads that create objects outside a frame.
	pub fn intern_image(&mut self, image: crate::implementation::FactoryImage) -> graphics_hardware_interface::ImageHandle {
		let mut builder = crate::image::Builder::new(image.format, image.resource_uses)
			.extent(image.extent)
			.device_accesses(image.device_accesses)
			.use_case(image.use_case);
		builder.name = image.name.as_deref();
		builder.array_layers = image.array_layers;
		builder.cube_compatible = image.cube_compatible;
		builder.cube_array_compatible = image.cube_array_compatible;

		self.build_image(builder)
	}

	/// Interns a factory-built sampler into this context, for loader threads that create objects outside a frame.
	pub fn intern_sampler(
		&mut self,
		sampler: crate::implementation::FactorySampler,
	) -> graphics_hardware_interface::SamplerHandle {
		let mut builder = crate::sampler::Builder::new()
			.filtering_mode(sampler.filtering_mode)
			.reduction_mode(sampler.reduction_mode)
			.mip_map_mode(sampler.mip_map_mode)
			.addressing_mode(sampler.addressing_mode)
			.min_lod(sampler.min_lod)
			.max_lod(sampler.max_lod);
		if let Some(anisotropy) = sampler.anisotropy {
			builder = builder.anisotropy(anisotropy);
		}

		self.build_sampler(builder)
	}
}
