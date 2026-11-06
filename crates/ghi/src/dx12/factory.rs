//! The `factory` module holds the DX12 types that move between threads and contexts.
//!
//! A [`Factory`] compiles shaders and pipelines away from the render thread. A [`DetachedImage`] carries a finished
//! image from one context to another, and a [`SharedBuffer`] lets a second context record copies into a buffer the
//! first context owns.

/// The `Factory` struct lets compiler threads build shaders and pipelines without owning render context state.
///
/// It wraps a context of the rendering device that never records or presents, so compiling HLSL and creating pipeline
/// states, the slow part of making a pipeline, runs on the thread that owns the factory. Get one from
/// [`Context::create_factory`](crate::dx12::Context::create_factory), then hand its pipelines to
/// [`Frame::intern_raster_pipeline`](crate::dx12::Frame::intern_raster_pipeline) or the other `intern_` methods.
pub struct Factory {
	pub(crate) context: crate::dx12::Device,
}

/// The `RasterPipeline` struct is a raster pipeline a factory compiled, on its way to the context that binds it.
pub struct RasterPipeline(pub(crate) crate::dx12::context::CompiledPipeline);

/// The `ComputePipeline` struct is a compute pipeline a factory compiled, on its way to the context that binds it.
pub struct ComputePipeline(pub(crate) crate::dx12::context::CompiledPipeline);

/// The `RayTracingPipeline` struct is a ray tracing pipeline a factory linked, on its way to the context that traces
/// with it.
pub struct RayTracingPipeline(pub(crate) crate::dx12::context::CompiledPipeline);

/// The `DetachedImage` struct is an image no context owns, on its way from one context to another.
///
/// A loader context fills an image, then [`Context::export_image`](crate::dx12::Context::export_image) moves it out and
/// [`Context::intern_image`](crate::dx12::Context::intern_image) or [`Frame::intern_image`](crate::dx12::Frame::intern_image)
/// gives it a handle in the context that renders with it. Its native resource is reference counted, so dropping it
/// without interning it frees the image.
pub struct DetachedImage {
	pub(crate) image: crate::dx12::context::Image,
	/// The barrier state the exporting context's last submission left the resource in, or `None` when it never
	/// recorded one.
	pub(crate) state: Option<crate::dx12::context::TextureBarrierState>,
}

/// The `SharedBuffer` struct lets a second context of the same device record copies into a buffer it does not own.
///
/// Get one from [`Context::share_buffer`](crate::dx12::Context::share_buffer) on the owning context, then pass it to
/// [`Context::import_buffer`](crate::dx12::Context::import_buffer) on the context that records the copies. The owning
/// context must outlive every context that imported the buffer.
pub struct SharedBuffer<T: ?Sized> {
	pub(crate) resource: ID3D12Resource,
	pub(crate) size: usize,
	pub(crate) uses: Uses,
	pub(crate) access: DeviceAccesses,
	pub(crate) heap_kind: crate::dx12::context::BufferHeapKind,
	pub(crate) contents: std::marker::PhantomData<T>,
}

// SAFETY: The shared buffer holds a reference-counted D3D12 resource, which is free-threaded, and plain metadata. `T`
// only types the handle the importing context gets back; no `T` value is stored.
unsafe impl<T: ?Sized> Send for SharedBuffer<T> {}

impl crate::device::Device for Factory {
	type Context = crate::dx12::Device;
	type Allocator = std::alloc::Global;
	type RasterPipeline = RasterPipeline;
	type ComputePipeline = ComputePipeline;
	type RayTracingPipeline = RayTracingPipeline;

	fn allocator(&self) -> &Self::Allocator {
		&std::alloc::Global
	}

	fn has_errors(&self) -> bool {
		self.context.has_errors()
	}

	fn create_context(&self) -> Result<Self::Context, &'static str> {
		Err(
			"Detached DX12 device cannot create a rendering context. The most likely cause is that asynchronous resource construction attempted to become the primary graphics device.",
		)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<ShaderHandle, ()> {
		self.context.create_shader(name, shader_source_type, stage, descriptor_sets)
	}

	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> Self::RasterPipeline {
		RasterPipeline(self.context.compile_raster_pipeline(builder))
	}

	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> Self::ComputePipeline {
		ComputePipeline(self.context.compile_compute_pipeline(builder))
	}

	fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> Self::RayTracingPipeline {
		RayTracingPipeline(self.context.compile_ray_tracing_pipeline(builder))
	}
}

use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

use crate::{DeviceAccesses, ShaderHandle, ShaderTypes, Uses, pipelines, shader::Sources};
