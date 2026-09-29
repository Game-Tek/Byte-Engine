//! The `factory` module exposes Metal resource types that move between threads and contexts.
//!
//! A [`Factory`] compiles shaders and pipelines away from the render thread. A [`DetachedImage`] carries a finished
//! image from one context to another, and a [`SharedBuffer`] lets a second context record copies into a buffer the
//! first context owns.

/// The `Factory` struct lets compiler threads build shaders and pipelines without owning render context state.
///
/// Get one from [`Context::create_factory`], then hand its pipelines to [`Frame::intern_raster_pipeline`] or
/// [`Frame::intern_compute_pipeline`].
pub struct Factory {
	pub(crate) device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
	pub(crate) compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
	pub settings: crate::device::Features,
	pub(crate) shaders: Vec<Shader>,
}

impl Factory {
	/// Creates a detached Metal factory from a backend device snapshot.
	pub(crate) fn new(
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		compiler: Retained<ProtocolObject<dyn mtl::MTL4Compiler>>,
		settings: crate::device::Features,
	) -> Self {
		Self {
			device,
			compiler,
			settings,
			shaders: Vec::new(),
		}
	}
}

/// The `RasterPipeline` type alias preserves the cross-platform raster pipeline name.
pub type RasterPipeline = Pipeline;

/// The `ComputePipeline` type alias preserves the cross-platform compute pipeline name.
pub type ComputePipeline = Pipeline;

/// The `DetachedImage` type is an image no context owns, on its way from one context to another.
///
/// A loader context fills an image, then [`Context::export_image`] moves it out and [`Context::intern_image`] or
/// [`Frame::intern_image`] gives it a handle in the context that renders with it.
pub type DetachedImage = image::Image;

/// The `SharedBuffer` struct lets a second context of the same device record copies into a buffer it does not own.
///
/// Get one from [`Context::share_buffer`] on the owning context, then pass it to [`Context::import_buffer`] on the
/// context that records the copies. The owning context must outlive every context that imported the buffer.
pub struct SharedBuffer<T: ?Sized> {
	name: Option<String>,
	buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	size: usize,
	uses: crate::Uses,
	access: crate::DeviceAccesses,
	contents: std::marker::PhantomData<T>,
}

// SAFETY: The shared buffer owns a retained Metal buffer, which Metal documents as usable from any thread, and plain
// metadata. Moving it between threads adds no shared state.
unsafe impl<T: ?Sized> Send for SharedBuffer<T> {}

impl crate::device::Device for Factory {
	type Context = crate::metal::context::Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = Pipeline;
	type ComputePipeline = ComputePipeline;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		false
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Err(
			"Detached Metal factory cannot create a rendering context. The most likely cause is that asynchronous resource construction attempted to become the primary graphics device.",
		)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		add_shader(
			&mut self.shaders,
			&self.device,
			name,
			shader_source_type,
			stage,
			shader_resource_descriptors,
			self.settings.debug_labels,
		)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> Self::RasterPipeline {
		build_raster_pipeline(
			&self.device,
			&self.compiler,
			&self.shaders,
			self.settings.debug_labels,
			builder,
		)
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> Self::ComputePipeline {
		build_compute_pipeline(
			&self.device,
			&self.compiler,
			&self.shaders,
			self.settings.debug_labels,
			builder,
		)
	}
}

use super::*;

/// These methods move resources between the contexts of one device and adopt [`Factory`] products.
impl Context {
	/// Creates a [`Factory`] that builds shaders and pipelines away from the render thread.
	pub fn create_factory(&self) -> Option<Factory> {
		Some(Factory::new(self.device.clone(), self.compiler.clone(), self.settings))
	}

	/// Moves a finished image out of this context so another context of the same device can intern it.
	///
	/// Wait for every submission that uses the image before exporting it. The handle is invalid in this context
	/// afterwards. Next, pass the image to [`Self::intern_image`] or [`Frame::intern_image`] on the receiving context.
	///
	/// # Panics
	///
	/// Panics when the handle names no image of this context, an image with one representation per frame, or an
	/// image-group member, whose memory belongs to this context's group heap.
	pub fn export_image(&mut self, image: graphics_hardware_interface::ImageHandle) -> DetachedImage {
		let image = self.images.take(image.0).expect(
			"Image export failed. The most likely cause is that the handle is stale, belongs to another context, or names a dynamic image.",
		);
		assert!(
			image.slot.is_none(),
			"Image export failed. The most likely cause is that the image is an image-group member, whose memory stays with its group."
		);
		image
	}

	/// Interns an image another context exported and returns the handle recordings use it with.
	///
	/// Metal tracks hazards from each image's first use in a context, so the image needs no state from its old one.
	pub fn intern_image(&mut self, image: DetachedImage) -> graphics_hardware_interface::ImageHandle {
		graphics_hardware_interface::ImageHandle(self.images.add(image).0)
	}

	/// Shares a buffer this context owns so another context of the same device can record copies into it.
	///
	/// Keep this context alive for as long as any context uses the imported buffer. Next, pass the value to
	/// [`Self::import_buffer`] on that context.
	///
	/// # Panics
	///
	/// Panics when the handle names no buffer of this context or a buffer with one representation per frame.
	pub fn share_buffer<T: ?Sized>(&self, buffer: graphics_hardware_interface::BufferHandle<T>) -> SharedBuffer<T> {
		let shared = self.buffers.get_unique(buffer.into()).expect(
			"Buffer sharing failed. The most likely cause is that the handle is stale, belongs to another context, or names a dynamic buffer.",
		);
		SharedBuffer {
			name: shared.name.clone(),
			buffer: shared.buffer.clone(),
			size: shared.size,
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
		use objc2_metal::MTLBuffer as _;

		let gpu_address = buffer.buffer.gpuAddress();
		let (handle, _) = self.buffers.add(buffer::Buffer {
			name: buffer.name,
			staging: None,
			buffer: buffer.buffer,
			size: buffer.size,
			gpu_address,
			pointer: std::ptr::null_mut(),
			uses: buffer.uses,
			access: buffer.access,
		});
		graphics_hardware_interface::BufferHandle(handle, std::marker::PhantomData)
	}

	/// Adopts a pipeline built by a [`Factory`] and returns the handle recordings bind it with.
	pub fn intern_raster_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.pipelines.push(pipeline);
		graphics_hardware_interface::PipelineHandle((self.pipelines.len() - 1) as u64)
	}

	/// Adopts a compute pipeline built by a [`Factory`]; Metal stores it like any other pipeline.
	pub fn intern_compute_pipeline(&mut self, pipeline: Pipeline) -> graphics_hardware_interface::PipelineHandle {
		self.intern_raster_pipeline(pipeline)
	}
}
