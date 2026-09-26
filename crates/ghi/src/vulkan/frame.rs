use ash::vk;
use utils::Extent;

use super::{command_buffer::CommandBufferRecording, context::Context};
use crate::{
	FrameKey, HandleLike as _, MasterHandle as _,
	context::ContextCreate as _,
	graphics_hardware_interface,
	vulkan::{ImageHandle, Swapchain, Synchronizer, Tasks},
};

pub struct Frame<'a> {
	frame_key: FrameKey,
	device: &'a mut Context,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut Context, frame_key: FrameKey) -> Self {
		Self { frame_key, device }
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
		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.command_buffer)];

		let wait_semaphores = present_keys
			.iter()
			.map(|present_key| {
				let swapchain = self.get_swapchain(present_key.swapchain);
				let semaphore = swapchain.acquire_synchronizers[present_key.sequence_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;
				// Waiting only at the image's first-use stage lets earlier work in the submission run before acquisition;
				// that first barrier's source scope is the same stage, so its layout transition still follows the wait.
				let first_use_stage = swapchain.acquire_wait_stages[present_key.sequence_index as usize];
				let stage_mask = if first_use_stage.is_empty() {
					vk::PipelineStageFlags2::ALL_COMMANDS
				} else {
					first_use_stage
				};
				vk::SemaphoreSubmitInfo::default().semaphore(semaphore).stage_mask(stage_mask)
			})
			.collect::<Vec<_>>();
		// ALL_COMMANDS orders the signal after the pre-present layout transition whatever stage wrote last,
		// as the Khronos swapchain synchronization example allows.
		let signal_semaphores = present_keys
			.iter()
			.map(|present_key| {
				let semaphore = self.get_swapchain(present_key.swapchain).submit_synchronizers
					[present_key.image_index as usize]
					.access(&self.device.synchronizers)
					.semaphore;
				vk::SemaphoreSubmitInfo::default()
					.semaphore(semaphore)
					.stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
			})
			.collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores);
		let execution_completion_fence =
			synchronizer.map_or(vk::Fence::null(), |synchronizer| self.get_synchronizer(synchronizer).fence);
		let vk_queue = self.device.vk_queues[command_buffer.vk_queue_index]
			.lock()
			.expect("Failed to lock Vulkan queue for frame submission. The most likely cause is that another thread panicked while holding the queue lock.");
		unsafe {
			self.device
				.device
				.queue_submit2(*vk_queue, &[submit_info], execution_completion_fence)
				.expect("Failed to submit command buffer.");
		}
		for handle in texture_readbacks {
			self.device.texture_readbacks.mark_submitted(handle);
		}

		// Binary semaphores are consumed by one wait, so each present waits only on its own image's render semaphore.
		for (presentation, signal) in present_keys.iter().zip(&signal_semaphores) {
			let wait_semaphores = [signal.semaphore];
			let swapchains = [self.get_swapchain(presentation.swapchain).swapchain];
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
		// The queue lock borrows the context, so release it before marking the fence as pending.
		drop(vk_queue);
		if let Some(synchronizer) = synchronizer {
			self.get_synchronizer_mut(synchronizer).armed = true;
		}

		self.device.states.extend(states);
		self.device.buffer_states.extend(buffer_states);
	}

	pub(crate) fn complete_without_submissions(
		&mut self,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) {
		let fence = self.get_synchronizer(synchronizer_handle).fence;
		{
			let queue = self.device.vk_queues[self.device.queues[0].vk_queue_index]
				.lock()
				.expect("Failed to lock Vulkan queue for empty frame submission. The most likely cause is that another thread panicked while holding the queue lock.");
			unsafe {
				self.device
					.device
					.queue_submit2(*queue, &[vk::SubmitInfo2::default()], fence)
					.expect(
						"Failed to submit empty Vulkan frame. The most likely cause is that the completion fence is invalid.",
					);
			}
		}
		self.get_synchronizer_mut(synchronizer_handle).armed = true;
	}

	fn get_current_image_handle(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		let handles = ImageHandle(image_handle.index()).get_all(&self.device.images);
		handles[(self.frame_key.sequence_index as usize).rem_euclid(handles.len())]
	}

	/// Returns the public handle of the image selected for this frame.
	fn get_current_image(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> crate::ImageHandle {
		crate::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(
			self.get_current_image_handle(image_handle).0,
		))
	}

	fn create_command_buffer_recording_internal(
		&mut self,
		command_buffer_handle: crate::CommandBufferHandle,
		include_implicit_sync: bool,
	) -> CommandBufferRecording<'_> {
		let sequence_index = self.frame_key.sequence_index;
		// Update descriptors before creating command buffer.
		self.device.process_tasks(sequence_index);

		// Explicit transfer command buffers must not consume frame-global pending uploads. Those uploads belong to the
		// normal render recording path, and stealing them here makes helper transfer submissions write render-frame
		// resources such as dynamic view buffers.
		let (buffer_copies, images) = if include_implicit_sync {
			let device = &mut *self.device;
			// Copy each persistent source into this frame's staging buffer and enqueue the staging to GPU copy, so every
			// frame gets the latest data even if the CPU didn't write this frame.
			if super::buffer::PERSISTENT_WRITE {
				for master_handle in &device.persistent_write_dynamic_buffers {
					let frame_buffer_handle = device.buffers.nth_handle(*master_handle, sequence_index as _).unwrap();
					let frame_buffer = device.buffers.resource(frame_buffer_handle);
					let source_buffer = device.buffers.resource(
						frame_buffer
							.source
							.expect("Persistent write dynamic buffer must have a source"),
					);
					let staging_buffer = device.buffers.resource(
						frame_buffer
							.staging
							.expect("Persistent write dynamic buffer must have per-frame staging"),
					);
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

					device.pending_buffer_syncs.insert(frame_buffer_handle);
				}
			}

			device.take_pending_syncs()
		} else {
			(Vec::new(), Vec::new())
		};

		let mut recording = CommandBufferRecording::new(self.device, command_buffer_handle, self.frame_key.into());
		recording.sync_buffers(buffer_copies.into_iter());
		recording.sync_textures(images.into_iter());
		recording
	}

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

	fn get_synchronizer_mut(
		&mut self,
		syncronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> &mut Synchronizer {
		let index = self.device.get_syncronizer_handles(syncronizer_handle)[self.frame_key.sequence_index as usize].0 as usize;
		&mut self.device.synchronizers[index]
	}

	pub(crate) fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.device.swapchains[swapchain_handle.0 as usize]
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

	fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		self.device.get_texture_slice_mut(self.get_current_image(texture_handle))
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		self.device.sync_texture(self.get_current_image(image_handle));
	}

	fn sync_texture_region(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		region: crate::image::Region,
	) {
		let handle = self.get_current_image_handle(image_handle);
		let image = &self.device.images[handle.0 as usize];
		region.validate(image.extent, image.format_, image.layers.map_or(1, |layers| layers.get()));
		assert!(
			image.staging_buffer.is_some(),
			"Texture staging is missing. The most likely cause is an image without host upload access."
		);
		self.device.pending_image_syncs.insert((handle, Some(region)));
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	/// Acquires a swapchain image from inside the started frame. The sequence fence was already waited by `start_frame`.
	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: crate::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		self.device
			.acquire_swapchain_image_for_sequence(self.frame_key.sequence_index, swapchain_handle)
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		self.device.image_groups.assert_resizable(image_handle);
		let current_frame = self.frame_key.sequence_index;
		let image_handles = ImageHandle(image_handle.index()).get_all(&self.device.images);
		// Every earlier resize queued its extent for the other copies after resizing this one, so matching copies
		// mean no resize toward a different extent is still pending.
		if image_handles
			.iter()
			.all(|handle| self.device.images[handle.0 as usize].extent == extent)
		{
			return;
		}
		let handle = image_handles[(current_frame as usize).rem_euclid(image_handles.len())];

		// Replaced storage is destroyed only once in-flight frames finish, so even a shared static image resizes now.
		self.device.resize_image_internal(handle, extent, current_frame);

		// Other sequences' copies may still be in flight, so they are rebuilt when their own frame starts.
		if image_handles.len() > 1 {
			self.device
				.add_task_to_all_other_frames(Tasks::ResizeImage { handle, extent }, current_frame);
		}
	}

	fn place_image_group(&mut self, group: graphics_hardware_interface::ImageGroupHandle, members: &[crate::ImageGroupMember]) {
		self.device.place_image_group(group, members);
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
		let handle = buffers
			.nth_handle(buffer_handle.into(), self.frame_key.sequence_index as _)
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
		let pointer = <T as crate::buffer::BufferContents>::from_raw_parts(pointer, byte_count).expect(
			"Failed to map a typed Vulkan frame buffer. The most likely cause is that the frame-local buffer has no sufficiently large, aligned CPU-visible storage.",
		);
		// SAFETY: The validated pointer addresses initialized POD storage and the frame owns exclusive access to its sequence resource.
		unsafe { &mut *pointer }
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
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
