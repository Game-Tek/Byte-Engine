use objc2_foundation::NSAutoreleasePool;
use objc2_foundation::NSString;
use objc2_metal::{MTL4CommandEncoder, MTL4CommandQueue, MTL4ComputeCommandEncoder, MTLDrawable};

use super::*;
use crate::SwapchainHandle;
use crate::image::ImageHandle;

/// The `Frame` struct scopes Metal rendering state to one frame.
///
/// Its `NSAutoreleasePool` releases temporary Metal objects at the end of the frame.
/// Without this pool, objects accumulate on threads that do not have a run-loop pool.
///
/// Field order matters: Rust drops fields in declaration order. The drawables must be
/// released before the autorelease pool drains, so `_autorelease_pool` is declared last.
pub struct Frame<'a> {
	frame_key: graphics_hardware_interface::FrameKey,
	queue_handle: graphics_hardware_interface::QueueHandle,
	drawables: Vec<(SwapchainHandle, Retained<ProtocolObject<dyn CAMetalDrawable>>), &'a dyn std::alloc::Allocator>,
	device: &'a mut context::Context,
	allocator: &'a dyn std::alloc::Allocator,
	_autorelease_pool: Retained<NSAutoreleasePool>,
}

impl<'a> Frame<'a> {
	pub fn new(
		device: &'a mut context::Context,
		frame_key: graphics_hardware_interface::FrameKey,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Self {
		assert!(
			!device.queues.is_empty(),
			"Metal frame creation failed. The most likely cause is that the context has no command queues.",
		);
		Self::new_for_queue(device, frame_key, graphics_hardware_interface::QueueHandle(0), allocator)
	}

	/// Creates a frame that batches command buffers through the selected queue.
	pub(crate) fn new_for_queue(
		device: &'a mut context::Context,
		frame_key: graphics_hardware_interface::FrameKey,
		queue_handle: graphics_hardware_interface::QueueHandle,
		allocator: &'a dyn std::alloc::Allocator,
	) -> Self {
		let pool = unsafe { NSAutoreleasePool::new() };
		Self {
			frame_key,
			queue_handle,
			drawables: Vec::new_in(allocator),
			device,
			allocator,
			_autorelease_pool: pool,
		}
	}

	fn get_current_image_handle(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device
			.images
			.nth_handle(image_handle, self.frame_key.sequence_index as _)
			.unwrap()
	}

	fn get_current_buffer_handle(
		&self,
		buffer_handle: graphics_hardware_interface::BaseBufferHandle,
	) -> crate::buffer::BufferHandle {
		self.device
			.buffers
			.nth_handle(buffer_handle, self.frame_key.sequence_index as _)
			.expect(
				"Missing Metal frame-local buffer. The most likely cause is that the dynamic buffer chain was not created for this frame.",
			)
	}

	fn frame_buffer_pointer(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> *mut u8 {
		let buffer = self.device.buffers.resource(self.get_current_buffer_handle(buffer_handle));
		let buffer = buffer
			.staging
			.map(|staging_handle| self.device.buffers.resource(staging_handle))
			.unwrap_or(buffer);

		buffer.pointer
	}

	fn frame_texture_staging_parts(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> (*mut u8, usize) {
		let image = self.device.images.resource(self.get_current_image_handle(image_handle));
		let staging = image.staging.as_ref().expect(
			"Missing Metal texture staging data. The most likely cause is that CPU texture access was requested for a device-only image.",
		);

		(staging.as_ptr() as *mut u8, staging.len())
	}
}

impl Frame<'_> {
	pub fn intern_raster_pipeline(
		&mut self,
		pipeline: crate::metal::device::Pipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_raster_pipeline(pipeline)
	}

	pub fn intern_compute_pipeline(
		&mut self,
		pipeline: crate::metal::device::ComputePipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_compute_pipeline(pipeline)
	}

	/// Interns a factory-built image through this frame's device.
	pub fn intern_image(&mut self, image: crate::metal::device::Image) -> graphics_hardware_interface::ImageHandle {
		self.device.intern_image(image)
	}

	/// Interns a factory-built sampler through this frame's device.
	pub fn intern_sampler(&mut self, sampler: crate::metal::device::Sampler) -> graphics_hardware_interface::SamplerHandle {
		self.device.intern_sampler(sampler)
	}

	pub fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	pub fn get_mut_dynamic_buffer_slice<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		unsafe { &mut *(self.frame_buffer_pointer(buffer_handle.into()) as *mut T) }
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back(handle);
	}

	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	/// Resizes the current image and schedules the other frame-local images for safe replacement.
	pub fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		let handle = self.get_current_image_handle(image_handle);
		if self.device.resize_image_internal(handle, extent) {
			// Other frame-local images may still be in flight, so replace each one when its frame is reused.
			self.device
				.resize_image_on_other_frames(image_handle, extent, self.frame_key.sequence_index);
		}
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		let mut drawables = Vec::with_capacity_in(self.drawables.len(), self.allocator);
		drawables.extend(
			self.drawables
				.iter()
				.map(|(swapchain, drawable)| (*swapchain, drawable.clone())),
		);
		let mut recording = self.device.create_command_buffer_recording_with_frame_key_in(
			command_buffer_handle,
			Some(self.frame_key),
			self.allocator,
		);
		recording.attach_drawables(drawables.into_iter());
		recording
	}

	pub fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		let sequence_index = self.frame_key.sequence_index;

		// Update layer extent before acquiring the drawable so that if a resize occurred,
		// the drawable is allocated at the correct size. update_layer_extent only calls
		// setDrawableSize when the size actually changed, avoiding unnecessary drawable
		// pool invalidation.
		let extent = {
			let swapchain = &self.device.swapchains[swapchain_handle.0 as usize];
			update_layer_extent(&swapchain.layer, &swapchain.view)
		};
		self.device.swapchains[swapchain_handle.0 as usize].extent = extent;

		// Proxy swapchains must keep their intermediate texture aligned with the drawable.
		if self.device.swapchains[swapchain_handle.0 as usize].uses_proxy {
			self.device.resize_swapchain_images(swapchain_handle, extent);
		}

		let drawable = self.device.swapchains[swapchain_handle.0 as usize]
			.layer
			.nextDrawable()
			.expect("Failed to acquire Metal drawable. The most likely cause is that the layer has no available drawables.");

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: 0,
			sequence_index,
			swapchain: swapchain_handle,
		};

		self.drawables.push((swapchain_handle, drawable));
		if !self.device.swapchains[swapchain_handle.0 as usize].uses_proxy {
			// A CAMetalLayer supplies a different drawable texture on each acquisition.
			self.device
				.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle(
					swapchain_handle.0,
				)));
		}

		(present_key, extent)
	}

	pub fn device(&mut self) -> &mut context::Context {
		self.device
	}

	pub fn execute_finished(
		&mut self,
		command_buffer: super::FinishedCommandBuffer<'_>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let mut command_buffers = SmallVec::new();
		command_buffers.push(command_buffer);
		self.execute_finished_batch(command_buffers, present_keys, synchronizer);
	}

	/// Finishes and submits all frame command buffers through one Metal 4 queue commit.
	pub(crate) fn execute_finished_batch<'command>(
		&mut self,
		command_buffers: SmallVec<[super::FinishedCommandBuffer<'command>; 4]>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let mut present_drawables = SmallVec::<
			[(
				graphics_hardware_interface::PresentKey,
				Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
			); 4],
		>::new();
		for &present_key in present_keys {
			let drawable = self
				.drawables
				.iter()
				.position(|(swapchain, _)| *swapchain == present_key.swapchain)
				.map(|index| self.drawables.swap_remove(index).1);
			present_drawables.push((present_key, drawable));
		}

		let mut native_commands = SmallVec::<[queue::NativeCommand; 4]>::new();
		let mut submitted_readbacks = SmallVec::<[graphics_hardware_interface::TextureCopyHandle; 8]>::new();
		for command_buffer in command_buffers {
			let super::FinishedCommandBuffer {
				command_buffer_handle,
				command_buffer,
				texture_readbacks,
				_marker,
			} = command_buffer;
			let command_queue = self.device.command_buffers[command_buffer_handle.0 as usize].queue_handle;

			assert_eq!(
				command_queue, self.queue_handle,
				"Metal 4 frame batch submission failed. The most likely cause is that a command buffer from another GHI queue was recorded into this execution.",
			);
			native_commands.push(command_buffer);
			submitted_readbacks.extend(texture_readbacks);
		}

		let uses_proxy = present_keys
			.iter()
			.any(|key| self.device.swapchains[key.swapchain.0 as usize].uses_proxy);
		if uses_proxy {
			// Proxy copies use a separate command so frame render commands can end before presentation work is appended.
			let mut resolve_command = self.device.queues[self.queue_handle.0 as usize]
				.acquire_native_command(Some("Present Resolve"), self.device.settings.debug_labels);
			let copy_encoder = resolve_command.compute_command_encoder().expect(
				"Metal 4 present resolve encoder creation failed. The most likely cause is that the resolve command was not recording.",
			);
			let queue_index = self.queue_handle.0 as usize;
			let mut resource_tracker = std::mem::take(&mut self.device.queues[queue_index].resource_tracker);
			resource_tracker.begin_recording();
			let resolve_scope = synchronization::MetalEncoderScope::Encoder(0);
			#[cfg(debug_assertions)]
			if self.device.settings.debug_labels {
				copy_encoder.setLabel(Some(&NSString::from_str("Present Resolve")));
			}

			for (present_key, drawable) in &present_drawables {
				if !self.device.swapchains[present_key.swapchain.0 as usize].uses_proxy {
					continue;
				}
				let Some(drawable) = drawable else {
					continue;
				};
				let swapchain = &self.device.swapchains[present_key.swapchain.0 as usize];
				let Some(proxy_image) = swapchain.images[present_key.sequence_index as usize] else {
					continue;
				};
				let source_texture = self.device.images.resource(proxy_image).texture.clone();
				let destination_texture = drawable.texture();
				resolve_command.retain_texture(source_texture.clone());
				resolve_command.retain_texture(destination_texture.clone());
				let barrier = resource_tracker.consume(
					resolve_scope,
					[
						synchronization::MetalResourceUse::image(
							proxy_image,
							None,
							None,
							mtl::MTLStages::Blit,
							crate::AccessPolicies::READ,
						),
						synchronization::MetalResourceUse::drawable(
							destination_texture.as_ref(),
							mtl::MTLStages::Blit,
							crate::AccessPolicies::WRITE,
						),
					],
				);
				barrier.encode_compute(copy_encoder.as_ref());

				unsafe {
					copy_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
				}
			}
			copy_encoder.endEncoding();
			resource_tracker.finish_recording();
			self.device.queues[queue_index].resource_tracker = resource_tracker;
			native_commands.push(resolve_command);
		}

		// An empty command still advances the frame synchronizer and provides a valid commit point for presentation.
		if native_commands.is_empty() {
			native_commands.push(
				self.device.queues[self.queue_handle.0 as usize]
					.acquire_native_command(Some("Empty Frame"), self.device.settings.debug_labels),
			);
		}
		for command in &mut native_commands {
			for (_, drawable) in &present_drawables {
				if let Some(drawable) = drawable {
					command.retain_drawable(drawable.clone());
				}
			}
		}

		let submitted = {
			let stored_queue = &mut self.device.queues[self.queue_handle.0 as usize];
			for (_, drawable) in &present_drawables {
				if let Some(drawable) = drawable {
					let drawable: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
					stored_queue.queue.waitForDrawable(drawable);
				}
			}

			let submitted = stored_queue.submit_batch(self.queue_handle, native_commands);
			for handle in &submitted_readbacks {
				self.device.texture_readbacks.mark_submitted(*handle);
			}

			for (_, drawable) in &present_drawables {
				if let Some(drawable) = drawable {
					let drawable: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
					stored_queue.queue.signalDrawable(drawable);
					drawable.present();
				}
			}
			submitted
		};

		let resource_tracker = &mut self.device.queues[self.queue_handle.0 as usize].resource_tracker;
		for (_, drawable) in &present_drawables {
			if let Some(drawable) = drawable {
				let texture = drawable.texture();
				resource_tracker.forget_drawable(texture.as_ref());
			}
		}

		let synchronizer = self
			.device
			.synchronizer_for_sequence(synchronizer, self.frame_key.sequence_index);
		self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= super::CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key
	}

	fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back(handle);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		Frame::get_mut_dynamic_buffer_slice(self, buffer_handle)
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		Frame::resize_image(self, image_handle, extent);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CBR<'record> {
		Frame::create_command_buffer_recording(self, command_buffer_handle)
	}

	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		Frame::acquire_swapchain_image(self, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> crate::AllocationHandle {
		self.device.create_allocation(size, resource_uses, resource_device_accesses)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> crate::MeshHandle {
		self.device
			.add_mesh_from_vertices_and_indices(vertex_count, index_count, vertices, indices, vertex_layout)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<crate::ShaderHandle, ()> {
		self.device
			.create_shader(name, shader_source_type, stage, shader_resource_descriptors)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>) -> crate::DescriptorSetHandle {
		self.device.create_descriptor_set(name)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> crate::PipelineHandle {
		self.device.create_raster_pipeline(builder)
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> crate::PipelineHandle {
		self.device.create_compute_pipeline(builder)
	}

	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> crate::PipelineHandle {
		self.device.create_ray_tracing_pipeline(builder)
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::BufferHandle<T> {
		self.device.build_buffer(builder)
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		self.device.build_dynamic_buffer(builder)
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		self.device.build_dynamic_image(builder)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> crate::ImageHandle {
		self.device.build_image(builder)
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> crate::SamplerHandle {
		self.device.build_sampler(builder)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::BaseBufferHandle {
		self.device
			.create_acceleration_structure_instance_buffer(name, max_instance_count)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.device.create_top_level_acceleration_structure(name, max_instance_count)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &crate::BottomLevelAccelerationStructure,
	) -> crate::BottomLevelAccelerationStructureHandle {
		self.device.create_bottom_level_acceleration_structure(description)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> crate::SynchronizerHandle {
		self.device.create_synchronizer(name, signaled)
	}
}
