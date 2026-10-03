use objc2_foundation::NSAutoreleasePool;
use objc2_metal::{MTL4CommandQueue, MTLDrawable};

use super::*;
use crate::image::ImageHandle;

/// The `Frame` struct scopes Metal rendering state to one frame.
///
/// Its `NSAutoreleasePool` releases temporary Metal objects at the end of the frame.
/// Without this pool, objects accumulate on threads that do not have a run-loop pool.
///
/// `_autorelease_pool` stays last so it drains after every other field.
pub struct Frame<'a> {
	frame_key: graphics_hardware_interface::FrameKey,
	queue_handle: graphics_hardware_interface::QueueHandle,
	device: &'a mut context::Context,
	allocator: &'a dyn std::alloc::Allocator,
	_autorelease_pool: Retained<NSAutoreleasePool>,
}

impl<'a> Frame<'a> {
	/// Creates a frame that batches command buffers through the selected queue.
	pub(crate) fn new(
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

	pub fn intern_raster_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_raster_pipeline(pipeline)
	}

	pub fn intern_compute_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_compute_pipeline(pipeline)
	}

	/// Interns an image another context exported, so this frame's recordings can use it.
	pub fn intern_image(&mut self, image: DetachedImage) -> graphics_hardware_interface::ImageHandle {
		self.device.intern_image(image)
	}

	pub fn device(&mut self) -> &mut context::Context {
		self.device
	}

	/// Submits the frame's command buffers, then copies each presented swapchain's image into a drawable and presents it.
	///
	/// Frames render into swapchain images rather than drawables, so their commands are committed before any drawable
	/// is taken. `nextDrawable` blocks the CPU until the display releases a drawable, which with two drawables is the
	/// refresh that shows the previous frame. Taking drawables only after the commit lets the GPU render this frame
	/// during that wait, and only the small resolve command waits on the display, through `waitForDrawable`.
	pub(crate) fn execute_finished_batch(
		&mut self,
		command_buffers: SmallVec<[super::FinishedCommandBuffer; 4]>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let synchronizer =
			context::synchronizer_for_sequence(&self.device.synchronizers, synchronizer, self.frame_key.sequence_index);

		let mut native_commands = SmallVec::<[queue::NativeCommand; 4]>::new();
		let mut submitted_readbacks = SmallVec::<[graphics_hardware_interface::TextureCopyHandle; 8]>::new();
		for command_buffer in command_buffers {
			let super::FinishedCommandBuffer {
				queue_handle,
				command_buffer,
				texture_readbacks,
			} = command_buffer;

			assert_eq!(
				queue_handle, self.queue_handle,
				"Metal 4 frame batch submission failed. The most likely cause is that a command buffer from another GHI queue was recorded into this execution.",
			);
			native_commands.push(command_buffer);
			submitted_readbacks.extend(texture_readbacks);
		}

		let mut submitted_any = false;
		if !native_commands.is_empty() {
			let submitted = self.device.queues[self.queue_handle.0 as usize].submit_batch(self.queue_handle, native_commands);
			for handle in &submitted_readbacks {
				self.device.texture_readbacks.mark_submitted(*handle, Some(synchronizer));
			}
			self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
			submitted_any = true;
		}

		if let Some(submitted) = self.present(present_keys) {
			self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
			submitted_any = true;
		}

		// An empty command still gives a frame that submitted nothing a completion point.
		if !submitted_any {
			let stored_queue = &mut self.device.queues[self.queue_handle.0 as usize];
			let command = stored_queue.acquire_native_command(Some("Empty Frame"), self.device.settings.debug_labels);
			let submitted = stored_queue.submit_batch(self.queue_handle, [command].into_iter().collect());
			self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
		}
	}

	/// Takes a drawable for each presented swapchain, copies the frame's swapchain image into it, and presents it.
	///
	/// Returns the submitted resolve, or `None` when no swapchain had a drawable to present, for example while its
	/// window is occluded and Core Animation times out.
	fn present(&mut self, present_keys: &[graphics_hardware_interface::PresentKey]) -> Option<queue::SubmittedBatch> {
		let sequence_index = self.frame_key.sequence_index as usize;
		let present_drawables = present_keys
			.iter()
			// A swapchain acquired at a zero extent has no image, so nothing was rendered for it.
			.filter(|present_key| self.device.swapchains[present_key.swapchain.0 as usize].images[sequence_index].is_some())
			.filter_map(|&present_key| {
				self.device
					.next_drawable(present_key.swapchain)
					.map(|drawable| (present_key, drawable))
			})
			.collect::<SmallVec<[_; 4]>>();
		if present_drawables.is_empty() {
			return None;
		}

		let mut recording = self.device.begin_recording(
			self.queue_handle,
			Some("Present Resolve"),
			Some(self.frame_key),
			&std::alloc::Global,
		);
		recording.resolve_swapchain_images(&present_drawables);
		let mut command = recording.into_finished().command_buffer;
		for (_, drawable) in &present_drawables {
			command.retain_drawable(drawable);
		}

		let stored_queue = &mut self.device.queues[self.queue_handle.0 as usize];
		for (_, drawable) in &present_drawables {
			let drawable: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			stored_queue.queue.waitForDrawable(drawable);
		}
		let submitted = stored_queue.submit_batch(self.queue_handle, [command].into_iter().collect());
		for (present_key, drawable) in &present_drawables {
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

		for (_, drawable) in &present_drawables {
			stored_queue.resource_tracker.forget_drawable(drawable.texture().as_ref());
		}

		Some(submitted)
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

	fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: crate::BufferHandle<T>,
	) -> &mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	/// Queues the upload of this frame's copy of the buffer, so a dynamic buffer uploads the copy this frame wrote.
	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device
			.sync_buffer_copy(buffer_handle.into(), self.frame_key.sequence_index);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &mut [u8] {
		let handle = self.get_current_image_handle(texture_handle);
		self.device.images.resource_mut(handle).staging.as_deref_mut().expect(
			"Missing Metal texture staging data. The most likely cause is that CPU texture access was requested for a device-only image.",
		)
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back((handle, None));
	}

	/// Schedules a rectangular upload from this frame's image staging storage.
	fn sync_texture_region(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		region: crate::image::Region,
	) {
		let handle = self.get_current_image_handle(image_handle);
		let image = self.device.images.resource(handle);
		region.validate(
			image.description.extent,
			image.description.format,
			image.description.array_layers,
		);
		self.device.pending_image_syncs.push_back((handle, Some(region)));
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		let pointer = self
			.device
			.typed_buffer_pointer::<T>(buffer_handle, self.frame_key.sequence_index);
		// SAFETY: The validated pointer addresses initialized POD storage and the frame owns exclusive access to its sequence resource.
		unsafe { &mut *pointer }
	}

	/// Resizes the current image and schedules the other frame-local images for safe replacement.
	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		self.device.image_groups.assert_resizable(image_handle);
		let handle = self.get_current_image_handle(image_handle);
		if self.device.resize_image_internal(handle, extent) {
			// Other frame-local images may still be in flight, so replace each one when its frame is reused.
			self.device
				.resize_image_on_other_frames(image_handle, extent, self.frame_key.sequence_index);
		}
	}

	fn place_image_group(&mut self, group: graphics_hardware_interface::ImageGroupHandle, members: &[crate::ImageGroupMember]) {
		self.device.place_image_group(group, members);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.device.create_command_buffer_recording_with_frame_key_in(
			command_buffer_handle,
			Some(self.frame_key),
			self.allocator,
		)
	}

	/// Prepares the swapchain image from inside the started frame. The sequence synchronizer was already waited by `start_frame`.
	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		self.device
			.acquire_swapchain_image_for_sequence(self.frame_key.sequence_index, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}
