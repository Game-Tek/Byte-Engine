#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]
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
		self.device.intern_pipeline(pipeline)
	}

	pub fn intern_compute_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_pipeline(pipeline)
	}

	/// Interns a ray tracing pipeline a [`Factory`] linked, so this frame's recordings can trace with it.
	///
	/// Next, write a shader binding table for the new handle with
	/// [`crate::context::ContextCreate::write_shader_binding_table`]: a table keeps working only with the pipeline its
	/// last write named.
	pub fn intern_ray_tracing_pipeline(&mut self, pipeline: RayTracingPipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_pipeline(pipeline)
	}

	/// Interns an image another context exported, so this frame's recordings can use it.
	pub fn intern_image(&mut self, image: DetachedImage) -> graphics_hardware_interface::ImageHandle {
		self.device.intern_image(image)
	}

	/// Deletes an image once no frame can use it anymore.
	///
	/// The handle is invalid when this returns, but frames recorded so far, this one included, may still read the
	/// image, so the context frees it once they complete. Write another resource to every descriptor set binding that
	/// names the image first. Work submitted outside frames is not waited for.
	///
	/// # Panics
	///
	/// Panics when the handle names no image of this context, an image with one representation per frame, or an
	/// image-group member, or when a descriptor set still binds the image.
	pub fn delete_image(&mut self, image: graphics_hardware_interface::ImageHandle) {
		self.device.delete_image(image, self.frame_key.sequence_index);
	}

	pub fn device(&mut self) -> &mut context::Context {
		self.device
	}

	/// Submits the frame's command buffers, then copies each presented swapchain's image into a drawable and presents it.
	///
	/// Frames render into swapchain images rather than drawables, so their commands are committed before any drawable
	/// is taken. A display-synced swapchain took its drawable from its display link when it was acquired, so it
	/// presents without waiting. An immediate swapchain takes one with `nextDrawable` here, after the commit, so the GPU
	/// renders this frame while the CPU waits, and only the small resolve command waits on the display, through
	/// `waitForDrawable`.
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
		let mut submitted_builds = SmallVec::<[acceleration_structures::RecordedBuilds; 4]>::new();
		for command_buffer in command_buffers {
			let super::FinishedCommandBuffer {
				queue_handle,
				command_buffer,
				texture_readbacks,
				builds,
			} = command_buffer;

			assert_eq!(
				queue_handle, self.queue_handle,
				"Metal 4 frame batch submission failed. The most likely cause is that a command buffer from another GHI queue was recorded into this execution.",
			);
			native_commands.push(command_buffer);
			submitted_readbacks.extend(texture_readbacks);
			submitted_builds.push(builds);
		}

		let rendered = !native_commands.is_empty();
		if rendered {
			let submitted =
				self.device.queues[self.queue_handle.index() as usize].submit_batch(self.queue_handle, native_commands);
			for handle in &submitted_readbacks {
				self.device.texture_readbacks.mark_submitted(*handle, Some(synchronizer));
			}
			for builds in submitted_builds {
				builds.submit(
					&mut self.device.acceleration_structures,
					synchronizer,
					&mut self.device.pending_compactions,
				);
			}
			self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
		}

		let submitted = match self.present(present_keys) {
			Some(submitted) => submitted,
			None if rendered => return,
			// An empty command still gives a frame that submitted nothing a completion point.
			None => {
				let stored_queue = &mut self.device.queues[self.queue_handle.index() as usize];
				let command = stored_queue.acquire_native_command(Some("Empty Frame"), self.device.settings.debug_labels);
				stored_queue.submit_batch(self.queue_handle, [command].into_iter().collect())
			}
		};
		self.device.synchronizers.resource_mut(synchronizer).signal(submitted);
	}

	/// Takes a drawable for each presented swapchain, copies the frame's swapchain image into it, and presents it.
	///
	/// The drawable is the one the display link handed out at acquisition, or for an immediate swapchain a new one from
	/// `nextDrawable`. Returns the submitted resolve, or `None` when no swapchain had a drawable to present.
	fn present(&mut self, present_keys: &[graphics_hardware_interface::PresentKey]) -> Option<queue::SubmittedBatch> {
		let sequence_index = self.frame_key.sequence_index as usize;
		let present_drawables = present_keys
			.iter()
			.filter_map(|&present_key| {
				let swapchain = &mut self.device.swapchains[present_key.swapchain.index() as usize];
				// A swapchain acquired at a zero extent has no image, so nothing was rendered for it.
				swapchain.images[sequence_index]?;
				// A display-linked swapchain took its drawable when the sequence acquired it.
				if swapchain.display_link.is_some() {
					return swapchain.drawables[sequence_index]
						.take()
						.map(|drawable| (present_key, drawable));
				}
				// An immediate swapchain doesn't wait for a refresh, so `nextDrawable` blocks only while the display holds
				// every drawable. Unattended runs bind these for windows that are never shown, which hold none. It returns
				// `None` when no drawable became available within Core Animation's one-second timeout.
				// SAFETY: The pool is created and drained on this thread, so the drawable's autoreleased reference does
				// not outlive the closure and only the returned reference keeps it from the layer's pool.
				let _pool = unsafe { NSAutoreleasePool::new() };
				swapchain.layer.nextDrawable().map(|drawable| (present_key, drawable))
			})
			.collect::<SmallVec<[_; 4]>>();
		if present_drawables.is_empty() {
			return None;
		}

		let mut recording = super::CommandBufferRecording::new(
			self.device,
			self.queue_handle,
			Some("Present Resolve"),
			Some(self.frame_key),
			&std::alloc::Global,
		);
		recording.resolve_swapchain_images(&present_drawables);
		let mut command = recording.into_finished().command_buffer;

		let stored_queue = &mut self.device.queues[self.queue_handle.index() as usize];
		for (_, drawable) in &present_drawables {
			command.retain_drawable(drawable);
			stored_queue.queue.waitForDrawable(drawable.as_ref());
		}
		let submitted = stored_queue.submit_batch(self.queue_handle, [command].into_iter().collect());
		for (present_key, drawable) in &present_drawables {
			stored_queue.resource_tracker.forget_drawable(drawable.texture().as_ref());
			let drawable: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			stored_queue.queue.signalDrawable(drawable);
			let swapchain = &self.device.swapchains[present_key.swapchain.index() as usize];
			record_presented_time(drawable, swapchain.last_presented_time.clone());
			match swapchain.present_interval {
				// Metal schedules the drawable for the first refresh after the interval since the previous present.
				Some(interval) => drawable.presentAfterMinimumDuration(interval.as_secs_f64()),
				None => drawable.present(),
			}
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

	fn map<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: crate::BufferHandle<T>) -> &mut T {
		let buffer = self
			.device
			.buffers
			.get_single(buffer_handle.into())
			.expect("Missing Metal buffer. The most likely cause is that the buffer handle came from another context.");
		crate::buffer::assert_frame_mappable(buffer.host_access, buffer.copies);
		let pointer = self
			.device
			.typed_buffer_pointer::<T>(buffer_handle, self.frame_key.sequence_index);
		// SAFETY: The pointer addresses initialized POD storage, and starting this frame waited for the last frame that
		// used its sequence's copy, so the CPU has it to itself.
		unsafe { &mut *pointer }
	}

	fn read<T: ?Sized + crate::buffer::BufferContents>(&self, buffer_handle: crate::BufferHandle<T>) -> Option<&T> {
		let buffer = self
			.device
			.buffers
			.get_single(buffer_handle.into())
			.expect("Missing Metal buffer. The most likely cause is that the buffer handle came from another context.");
		crate::buffer::assert_frame_readable(buffer.host_access, buffer.copies);
		// The frame that completed when this one started is the one before it in this frame's sequence.
		if self.frame_key.frame_index < u64::from(self.device.frames) {
			return None;
		}
		let pointer = self
			.device
			.typed_buffer_pointer::<T>(buffer_handle, self.frame_key.sequence_index);
		// SAFETY: That frame completed, and this frame's commands, the next to use its copy, haven't run.
		Some(unsafe { &*pointer })
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
		length: usize,
	) {
		crate::context::Context::resize_buffer(&mut *self.device, buffer_handle, length);
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		self.resize_image_storage(image_handle, extent, None);
	}

	fn resize_image_layers(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		array_layers: std::num::NonZeroU32,
	) {
		crate::image::assert_resizable_layers(
			self.device
				.images
				.resource(self.get_current_image_handle(image_handle))
				.description
				.cube_array_compatible,
			array_layers,
		);
		self.resize_image_storage(image_handle, extent, Some(array_layers.get()));
	}

	fn place_image_group(&mut self, group: graphics_hardware_interface::ImageGroupHandle, members: &[crate::ImageGroupMember]) {
		self.device.place_image_group(group, members);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CBR<'record> {
		let command_buffer = &self.device.command_buffers[command_buffer_handle.index() as usize];
		let queue_handle = command_buffer.queue_handle;
		let name = self
			.device
			.settings
			.debug_labels
			.then(|| command_buffer.name.clone())
			.flatten();
		self.device
			.start_recording(queue_handle, name.as_deref(), Some(self.frame_key), self.allocator)
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

impl Frame<'_> {
	/// Resizes the current image and schedules the other frame-local images for safe replacement.
	///
	/// `array_layers` replaces the layer count, or keeps it when `None`.
	fn resize_image_storage(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		array_layers: Option<u32>,
	) {
		self.device.image_groups.assert_resizable(image_handle);
		let handle = self.get_current_image_handle(image_handle);
		if self.device.resize_image_internal(handle, extent, array_layers) {
			// Other frame-local images may still be in flight, so replace each one when its frame is reused.
			let frames = self.device.frames;
			for offset in 1..frames {
				self.device.tasks.push(Task {
					kind: TaskKind::ResizeImage {
						handle: image_handle,
						extent,
						array_layers,
					},
					frame: (self.frame_key.sequence_index + offset) % frames,
				});
			}
		}
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}
