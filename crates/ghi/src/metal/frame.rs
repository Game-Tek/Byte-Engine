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
/// Acquired drawables live on the device swapchains (`pending_drawable`) so acquisition can
/// happen before the frame is started; `_autorelease_pool` stays last so it drains after every other field.
pub struct Frame<'a> {
	frame_key: graphics_hardware_interface::FrameKey,
	queue_handle: graphics_hardware_interface::QueueHandle,
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
		// SAFETY: The pool is created and drained on the execution thread that owns this frame scope.
		let pool = unsafe { NSAutoreleasePool::new() };
		Self {
			frame_key,
			queue_handle,
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

	fn frame_buffer_parts(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> (*mut u8, usize) {
		let buffer = self.device.buffers.resource(self.get_current_buffer_handle(buffer_handle));
		let buffer = buffer
			.staging
			.map(|staging_handle| self.device.buffers.resource(staging_handle))
			.unwrap_or(buffer);

		(buffer.pointer, buffer.size)
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

	pub fn get_mut_buffer_slice<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	pub fn get_mut_dynamic_buffer_slice<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		let (pointer, byte_count) = self.frame_buffer_parts(buffer_handle.into());
		let pointer = crate::buffer::typed_buffer_pointer::<T>(pointer, byte_count).expect(
			"Failed to map a typed Metal frame buffer. The most likely cause is that the frame-local buffer has no sufficiently large, aligned CPU-visible storage.",
		);
		// SAFETY: The validated pointer addresses initialized POD storage and the frame owns exclusive access to its sequence resource.
		unsafe { &mut *pointer }
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		// SAFETY: `frame_texture_staging_parts` returns the live exclusive staging allocation and its exact size.
		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back((handle, None));
	}

	/// Schedules a rectangular upload from this frame's image staging storage.
	pub fn sync_texture_region(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		region: crate::image::Region,
	) {
		let handle = self.get_current_image_handle(image_handle);
		let image = self.device.images.resource(handle);
		region.validate(image.extent, image.format, image.array_layers);
		self.device.pending_image_syncs.push_back((handle, Some(region)));
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
		let mut drawables = Vec::new_in(self.allocator);
		drawables.extend(self.device.swapchains.iter().enumerate().filter_map(|(index, swapchain)| {
			swapchain
				.pending_drawable
				.as_ref()
				.map(|drawable| (SwapchainHandle(index as u64), drawable.clone()))
		}));
		let mut recording = self.device.create_command_buffer_recording_with_frame_key_in(
			command_buffer_handle,
			Some(self.frame_key),
			self.allocator,
		);
		recording.attach_drawables(drawables.into_iter());
		recording
	}

	/// Acquires a drawable from inside the started frame. The sequence synchronizer was already waited by `start_frame`.
	pub fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		self.device
			.acquire_swapchain_image_for_sequence(self.frame_key.sequence_index, swapchain_handle)
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

	/// Takes the pending drawables for this submission while preserving a missing drawable as an explicit skipped present.
	fn take_present_drawables(
		&mut self,
		present_keys: &[graphics_hardware_interface::PresentKey],
	) -> SmallVec<
		[(
			graphics_hardware_interface::PresentKey,
			Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
		); 4],
	> {
		present_keys
			.iter()
			.map(|&present_key| {
				let drawable = self.device.swapchains[present_key.swapchain.0 as usize]
					.pending_drawable
					.take();
				(present_key, drawable)
			})
			.collect()
	}

	/// Returns whether presentation needs a proxy-texture resolve before drawable submission.
	fn uses_proxy_swapchain(&self, present_keys: &[graphics_hardware_interface::PresentKey]) -> bool {
		present_keys
			.iter()
			.any(|key| self.device.swapchains[key.swapchain.0 as usize].uses_proxy)
	}

	/// Finishes and submits all frame command buffers through one Metal 4 queue commit.
	pub(crate) fn execute_finished_batch<'command>(
		&mut self,
		command_buffers: SmallVec<[super::FinishedCommandBuffer<'command>; 4]>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let present_drawables = self.take_present_drawables(present_keys);

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

		if self.uses_proxy_swapchain(present_keys) {
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

				// SAFETY: Source and drawable textures are retained and validated for the proxy resolve copy.
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

			for (present_key, drawable) in &present_drawables {
				if let Some(drawable) = drawable {
					let drawable: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
					stored_queue.queue.signalDrawable(drawable);
					let swapchain = &self.device.swapchains[present_key.swapchain.0 as usize];
					record_presented_time(drawable, swapchain.last_presented_time.clone());
					match swapchain.present_interval {
						// Metal schedules the drawable for the first refresh after the interval since the previous present.
						Some(interval) => drawable.presentAfterMinimumDuration(interval.as_secs_f64()),
						None => drawable.present(),
					}
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

/// Publishes the drawable's on-screen time into `slot` once the display shows it.
fn record_presented_time(drawable: &ProtocolObject<dyn mtl::MTLDrawable>, slot: std::sync::Arc<std::sync::atomic::AtomicU64>) {
	let handler = block2::StackBlock::new(move |drawable: std::ptr::NonNull<ProtocolObject<dyn mtl::MTLDrawable>>| {
		// Metal may invoke this block on any thread, so it only touches the shared atomic.
		// SAFETY: Metal keeps the drawable alive for the duration of the presented-handler invocation.
		let presented_time = unsafe { drawable.as_ref() }.presentedTime();
		if presented_time > 0.0 {
			slot.store(presented_time.to_bits(), std::sync::atomic::Ordering::Release);
		}
	});
	// SAFETY: Metal copies the block before this call returns, so the stack block may be dropped afterwards.
	unsafe { drawable.addPresentedHandler(std::ptr::NonNull::from(&*handler).as_ptr()) };
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= super::CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key
	}

	fn get_mut_buffer_slice<T: crate::Pod>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		// SAFETY: `frame_texture_staging_parts` returns the live exclusive staging allocation and its exact size.
		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back((handle, None));
	}

	fn sync_texture_region(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		region: crate::image::Region,
	) {
		Frame::sync_texture_region(self, image_handle, region);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: crate::Pod>(
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
	) -> Option<crate::frame::SwapchainAcquisition> {
		Frame::acquire_swapchain_image(self, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}
