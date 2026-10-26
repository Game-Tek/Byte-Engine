//! The `factory` module exposes Vulkan resource types that move between threads and contexts.
//!
//! A [`Factory`] compiles shaders and pipelines away from the render thread. A [`DetachedImage`] carries a finished
//! image from one context to another, and a [`SharedBuffer`] lets a second context record copies into a buffer the
//! first context owns.

use ash::vk::{self, Handle as _};

use super::{Allocation, Buffer, BufferHandle, Context, Frame, Handles, Image, MappedMemoryPointer, TransitionState};
use crate::{MasterHandle as _, graphics_hardware_interface};

/// The `Factory` type alias lets compiler threads build shaders and pipelines without owning render context state.
///
/// Get one from [`Context::create_factory`], then hand its pipelines to [`Frame::intern_raster_pipeline`] or
/// [`Frame::intern_compute_pipeline`].
pub type Factory = crate::vulkan::Device;

pub use crate::vulkan::device::{ComputePipeline, RasterPipeline};

/// The `RayTracingPipeline` type names the ray tracing pipelines a Vulkan factory builds. Vulkan doesn't trace rays
/// yet, so the type has no values.
pub type RayTracingPipeline = std::convert::Infallible;

/// The `DetachedImage` struct is an image no context owns, on its way from one context to another.
///
/// A loader context fills an image, then [`Context::export_image`] moves it out and [`Context::intern_image`] or
/// [`Frame::intern_image`] gives it a handle in the context that renders with it. Dropping it without interning it
/// frees the image, so drop it before the device it came from.
pub struct DetachedImage {
	device: ash::Device,
	/// `None` once a context interned the image, so dropping the emptied value frees nothing.
	resources: Option<DetachedImageResources>,
}

/// The `DetachedImageResources` struct holds what a context hands over with an image: its native objects, the
/// memory behind them, and the layout its last submission left it in.
struct DetachedImageResources {
	name: Option<String>,
	/// The image without its allocation handles, which only mean something to the context that created them.
	image: Image,
	memory: vk::DeviceMemory,
	staging_memory: vk::DeviceMemory,
	layout: vk::ImageLayout,
}

impl Drop for DetachedImage {
	fn drop(&mut self) {
		let Some(resources) = self.resources.take() else {
			return;
		};
		let image = resources.image;
		// SAFETY: Exporting requires every submission that used the image to have completed, and no context holds
		// these objects anymore, so nothing else can use them.
		unsafe {
			for &view in image
				.image_views
				.iter()
				.chain(&image.layer_suffix_views)
				.chain([&image.full_image_view])
			{
				self.device.destroy_image_view(view, None);
			}
			self.device.destroy_image(image.image, None);
			if let Some(staging_buffer) = image.staging_buffer {
				self.device.destroy_buffer(staging_buffer, None);
			}
			// Freeing mapped memory unmaps it.
			self.device.free_memory(resources.memory, None);
			self.device.free_memory(resources.staging_memory, None);
		}
	}
}

/// The `SharedBuffer` struct lets a second context of the same device record copies into a buffer it does not own.
///
/// Get one from [`Context::share_buffer`] on the owning context, then pass it to [`Context::import_buffer`] on the
/// context that records the copies. The owning context must outlive every context that imported the buffer.
pub struct SharedBuffer<T: ?Sized> {
	buffer: vk::Buffer,
	size: usize,
	device_address: vk::DeviceAddress,
	uses: crate::Uses,
	access: crate::DeviceAccesses,
	contents: std::marker::PhantomData<T>,
}

// SAFETY: The shared buffer holds a Vulkan buffer handle, which has no thread affinity, and plain metadata. `T` only
// types the handle the importing context gets back; no `T` value is stored.
unsafe impl<T: ?Sized> Send for SharedBuffer<T> {}

/// These methods move resources between the contexts of one device and adopt [`Factory`] products.
impl Context {
	/// Creates a [`Factory`] that builds shaders and pipelines away from the render thread.
	pub fn create_factory(&self) -> Option<Factory> {
		Some(Factory::detached_with_resources(
			self.device.device.clone(),
			self.device.descriptor_heap_properties,
			self.device.pipeline_binaries.clone(),
		))
	}

	/// Moves a finished image out of this context so another context of the same device can intern it.
	///
	/// Wait for every submission that uses the image before exporting it. The handle is invalid in this context
	/// afterwards. Next, pass the image to [`Self::intern_image`] or [`Frame::intern_image`] on the receiving context.
	///
	/// # Panics
	///
	/// Panics when the handle names no image of this context, an image with one representation per frame, a
	/// swapchain image, or an image-group member, whose memory belongs to this context's group heap.
	pub fn export_image(&mut self, image: graphics_hardware_interface::ImageHandle) -> DetachedImage {
		let handle = crate::vulkan::ImageHandle(image.0.index());
		let exportable = self
			.images
			.get(handle.0 as usize)
			.is_some_and(|stored| stored.owns_image && stored.next.is_none() && !stored.image.is_null());
		assert!(
			exportable,
			"Image export failed. The most likely cause is that the handle is stale, belongs to another context, or names a dynamic or swapchain image."
		);
		assert!(
			self.image_groups.member(image.0).is_none(),
			"Image export failed. The most likely cause is that the image is an image-group member, whose memory stays with its group."
		);

		// The slot stays, emptied, so the handle can never name a different image later.
		let stored = &mut self.images[handle.0 as usize];
		let emptied = Image {
			owns_image: false,
			..super::context::unbacked_image(stored.format_, stored.uses, stored.extent, stored.access)
		};
		let mut stored = std::mem::replace(stored, emptied);
		let memory = self.release_allocation(stored.allocation.take());
		let staging_memory = self.release_allocation(stored.staging_allocation.take());
		let layout = self
			.states
			.remove(&Handles::Image(handle))
			.map_or(vk::ImageLayout::UNDEFINED, |state| state.layout);

		DetachedImage {
			device: self.device.device.clone(),
			resources: Some(DetachedImageResources {
				name: self.get_object_debug_name(image.into()),
				image: stored,
				memory,
				staging_memory,
				layout,
			}),
		}
	}

	/// Interns an image another context exported and returns the handle recordings use it with.
	///
	/// The image keeps the layout its last submission left it in, so its first use here records the barrier that
	/// moves it to the layout that use needs.
	pub fn intern_image(&mut self, mut image: DetachedImage) -> graphics_hardware_interface::ImageHandle {
		let resources = image
			.resources
			.take()
			.expect("Detached image is empty. The most likely cause is a backend bug, since only interning empties one.");
		let mut stored = resources.image;
		let staging_pointer = stored.pointer.map_or(std::ptr::null_mut(), |pointer| pointer.0);
		stored.allocation = self.adopt_allocation(resources.memory, std::ptr::null_mut());
		stored.staging_allocation = self.adopt_allocation(resources.staging_memory, staging_pointer);

		let handle = crate::vulkan::ImageHandle(self.images.len() as u64);
		self.images.push(stored);
		// The exporting context's stages mean nothing here. A transfer write is the last thing a loader does to an
		// image and a stage every queue has, so the first use here orders itself after it.
		if resources.layout != vk::ImageLayout::UNDEFINED {
			self.states.insert(
				Handles::Image(handle),
				TransitionState {
					access: vk::AccessFlags2::NONE,
					..TransitionState::new(
						vk::PipelineStageFlags2::TRANSFER,
						vk::AccessFlags2::TRANSFER_WRITE,
						resources.layout,
					)
				},
			);
		}

		let handle = graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(handle.0));
		self.set_object_debug_name(resources.name.as_deref(), handle.into());
		handle
	}

	/// Takes the device memory out of an allocation, leaving its entry empty so the context never frees it.
	fn release_allocation(&mut self, allocation: Option<graphics_hardware_interface::AllocationHandle>) -> vk::DeviceMemory {
		let Some(allocation) = allocation else {
			return vk::DeviceMemory::null();
		};
		let allocation = &mut self.allocations[allocation.0 as usize];
		allocation.pointer = MappedMemoryPointer(std::ptr::null_mut());
		std::mem::replace(&mut allocation.memory, vk::DeviceMemory::null())
	}

	/// Registers device memory another context allocated, so this context frees it with the resource it backs.
	fn adopt_allocation(
		&mut self,
		memory: vk::DeviceMemory,
		pointer: *mut u8,
	) -> Option<graphics_hardware_interface::AllocationHandle> {
		if memory.is_null() {
			return None;
		}
		self.allocations.push(Allocation {
			memory,
			pointer: MappedMemoryPointer(pointer),
		});
		Some(graphics_hardware_interface::AllocationHandle(
			self.allocations.len() as u64 - 1,
		))
	}

	/// Shares a buffer this context owns so another context of the same device can record copies into it.
	///
	/// Keep this context alive for as long as any context uses the imported buffer. From its next frame on, this
	/// context orders its first use of the buffer in each frame after the other context's copies. Next, pass the
	/// value to [`Self::import_buffer`] on that context.
	///
	/// # Panics
	///
	/// Panics when the handle names no buffer of this context or a buffer with one representation per frame.
	pub fn share_buffer<T: ?Sized>(&self, buffer: graphics_hardware_interface::BufferHandle<T>) -> SharedBuffer<T> {
		let master: graphics_hardware_interface::BaseBufferHandle = buffer.into();
		let shared = self.buffers.get_unique(master).expect(
			"Buffer sharing failed. The most likely cause is that the handle is stale, belongs to another context, or names a dynamic buffer.",
		);
		let handle = BufferHandle(master.index());
		let mut shared_buffers = self.shared_buffers.borrow_mut();
		if !shared_buffers.contains(&handle) {
			shared_buffers.push(handle);
		}
		SharedBuffer {
			buffer: shared.buffer,
			size: shared.size,
			device_address: shared.device_address,
			uses: shared.uses,
			access: shared.access,
			contents: std::marker::PhantomData,
		}
	}

	/// Imports a buffer another context shared and returns the handle this context records copies with.
	///
	/// This context only records GPU copies with the handle. CPU access stays with the owning context, which also
	/// keeps the buffer alive.
	pub fn import_buffer<T: ?Sized>(&mut self, buffer: SharedBuffer<T>) -> graphics_hardware_interface::BufferHandle<T> {
		let (handle, _) = self.buffers.add(Buffer {
			staging: None,
			buffer: buffer.buffer,
			size: buffer.size,
			device_address: buffer.device_address,
			pointer: MappedMemoryPointer(std::ptr::null_mut()),
			allocation: None,
			uses: buffer.uses,
			access: buffer.access,
			imported: true,
		});
		graphics_hardware_interface::BufferHandle(handle, std::marker::PhantomData)
	}

	/// Records that another context may have written every shared buffer since this context last used it.
	///
	/// The tracker cannot see another context's copies. Treating each shared buffer as freshly written by a transfer
	/// makes the first use in the frame record a barrier, which is what makes those copies visible to it. The state
	/// also keeps everything this context still had pending on the buffer.
	pub(crate) fn expect_shared_buffer_writes(&mut self) {
		for &handle in self.shared_buffers.get_mut().iter() {
			let key = Handles::Buffer(handle);
			let ranges = self.buffer_states.entry(key).or_default();
			let tracked = self
				.states
				.get(&key)
				.into_iter()
				.chain(ranges.iter().map(|range| &range.state));
			let (stage, access) = tracked.fold(
				(vk::PipelineStageFlags2::COPY, vk::AccessFlags2::TRANSFER_WRITE),
				|(stage, access), state| {
					(
						stage | state.stage | state.last_write_stage,
						access | state.access | state.last_write_access,
					)
				},
			);
			let state = TransitionState::new(stage, access, vk::ImageLayout::UNDEFINED);
			// Ranged consumptions only order themselves against ranged states, so the write covers the whole buffer.
			ranges.clear();
			ranges.push(super::BufferTransitionState {
				range: super::BufferRange::new(0, vk::WHOLE_SIZE),
				state,
			});
			self.states.insert(key, state);
		}
	}
}

/// These methods adopt [`Factory`] products and exported images from inside a started frame.
impl Frame<'_> {
	/// Interns a raster pipeline that a factory thread already compiled into this frame's device.
	pub fn intern_raster_pipeline(&mut self, pipeline: RasterPipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device_mut().add_pipeline(pipeline.pipeline, pipeline.layout)
	}

	/// Interns a factory-built compute pipeline into this frame's device.
	pub fn intern_compute_pipeline(&mut self, pipeline: ComputePipeline) -> graphics_hardware_interface::PipelineHandle {
		self.device_mut().add_pipeline(pipeline.pipeline, pipeline.layout)
	}

	/// Interns a factory-built ray tracing pipeline, of which Vulkan has none.
	pub fn intern_ray_tracing_pipeline(&mut self, pipeline: RayTracingPipeline) -> graphics_hardware_interface::PipelineHandle {
		match pipeline {}
	}

	/// Interns an image another context exported. See [`Context::intern_image`].
	pub fn intern_image(&mut self, image: DetachedImage) -> graphics_hardware_interface::ImageHandle {
		self.device_mut().intern_image(image)
	}
}
