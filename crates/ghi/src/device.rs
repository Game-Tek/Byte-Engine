use std::alloc::Allocator;

use crate::{
	ShaderHandle, ShaderTypes, pipelines,
	shader::{self, Sources},
};

/// The `Device` trait centralizes ownership of backend device state for the graphics hardware interface.
pub trait Device
where
	Self: Sized,
{
	type Context: crate::context::Context;
	type Allocator: Allocator;
	type RasterPipeline;
	type ComputePipeline;
	/// A backend without ray tracing names an uninhabited type, so none of its pipelines can exist.
	type RayTracingPipeline;

	/// Returns the allocator used for device-owned host memory.
	fn allocator(&self) -> &Self::Allocator;

	/// Returns whether the backend API reported an error.
	fn has_errors(&self) -> bool;

	/// Creates a new rendering context that operates on this device.
	fn create_context(&self) -> Result<Self::Context, &'static str>;

	/// Creates a shader. `descriptor_sets` declares its descriptor set layouts, in the order its native source
	/// declares them; see [`shader::ShaderDescriptorSet`].
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = shader::ShaderDescriptorSet>,
	) -> Result<ShaderHandle, ()>;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> Self::RasterPipeline;

	/// Creates a compute pipeline from a builder.
	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> Self::ComputePipeline;

	/// Creates a tile pipeline from a builder.
	///
	/// Check [`crate::context::Context::supports_tile_functions`] first; backends without tile functions panic. Next,
	/// intern the pipeline in the frame that renders with it, as a raster pipeline.
	fn create_tile_pipeline(&mut self, _builder: pipelines::tile::Builder) -> Self::RasterPipeline {
		pipelines::tile::unsupported(std::any::type_name::<Self>())
	}

	/// Creates a ray tracing pipeline from a builder.
	///
	/// Check [`crate::context::Context::supports_ray_tracing`] first. A factory compiles each closest-hit shader once
	/// for each set of specialization constants, so creating the pipeline again with more hit groups compiles only
	/// the new ones. Next, intern the pipeline in the frame that renders with it.
	fn create_ray_tracing_pipeline(&mut self, builder: pipelines::ray_tracing::Builder) -> Self::RayTracingPipeline;
}

/// The `Features` struct selects optional GPU features during device creation.
///
/// Validation, API tracing, ray tracing, sparse resources, geometry shaders, GPU timing
/// counters, and the application-owned pipeline cache are disabled by default. Mesh shading is
/// enabled by default. When `gpu` is `None`, the backend selects an appropriate GPU.
#[derive(Debug, Clone, Copy)]
pub struct Features {
	pub(crate) validation: bool,
	pub(crate) gpu_validation: bool,
	pub(crate) api_dump: bool,
	pub(crate) ray_tracing: bool,
	pub(crate) debug_labels: bool,
	/// Whether GPU timing counters write timestamps; see [`Self::counters`]. Off by default.
	pub(crate) counters: bool,
	pub(crate) debug_log_function: Option<fn(&str)>,
	pub(crate) gpu: Option<&'static str>,
	pub(crate) sparse: bool,
	pub(crate) geometry_shader: bool,
	pub(crate) mesh_shading: bool,
	pub(crate) pipeline_cache: bool,
	/// Whether windows that present on display refresh are paced by a display link; see [`Self::display_link`].
	pub(crate) display_link: bool,
}

impl Default for Features {
	fn default() -> Self {
		Self::new()
	}
}

impl Features {
	pub fn new() -> Self {
		Self {
			validation: false,
			gpu_validation: false,
			api_dump: false,
			ray_tracing: false,
			debug_labels: false,
			counters: false,
			debug_log_function: None,
			gpu: None,
			sparse: false,
			geometry_shader: false,
			mesh_shading: true,
			pipeline_cache: false,
			display_link: false,
		}
	}

	pub fn validation(mut self, validation: bool) -> Self {
		self.validation = validation;
		self
	}

	pub fn gpu_validation(mut self, gpu_validation: bool) -> Self {
		self.gpu_validation = gpu_validation;
		self
	}

	pub fn api_dump(mut self, api_dump: bool) -> Self {
		self.api_dump = api_dump;
		self
	}

	pub fn ray_tracing(mut self, ray_tracing: bool) -> Self {
		self.ray_tracing = ray_tracing;
		self
	}

	pub fn debug_labels(mut self, debug_labels: bool) -> Self {
		self.debug_labels = debug_labels;
		self
	}

	/// Makes GPU timing counters write timestamps. Off by default.
	///
	/// Every context still creates and accepts counters while this is off, but no timestamp is written, no native
	/// timestamp storage is created, and [`crate::context::Context::counter_duration`] always returns `None`, so the
	/// GPU does no timing work. Turn it on to profile frames; read the durations as
	/// [`crate::context::ContextCreate::create_counter`] describes. An instance and a device that both receive
	/// features enable counters when either one asks for them.
	pub fn counters(mut self, counters: bool) -> Self {
		self.counters = counters;
		self
	}

	pub fn debug_log_function(mut self, debug_log_function: fn(&str)) -> Self {
		self.debug_log_function = Some(debug_log_function);
		self
	}

	pub fn gpu(mut self, gpu: &'static str) -> Self {
		self.gpu = Some(gpu);
		self
	}

	pub fn sparse(mut self, sparse: bool) -> Self {
		self.sparse = sparse;
		self
	}

	pub fn geometry_shader(mut self, geometry_shader: bool) -> Self {
		self.geometry_shader = geometry_shader;
		self
	}

	pub fn mesh_shading(mut self, mesh_shading: bool) -> Self {
		self.mesh_shading = mesh_shading;
		self
	}

	/// Makes the application the owner of the compiled pipeline cache instead of the GPU driver.
	///
	/// Enable this only when the application persists the cache: load it with
	/// [`crate::context::Context::load_pipeline_cache`] before creating pipelines and save
	/// [`crate::context::Context::pipeline_cache_data`] before exiting. The driver stops keeping its own cache where it can, so an
	/// application that enables this without saving compiles every pipeline again in each run.
	pub fn pipeline_cache(mut self, pipeline_cache: bool) -> Self {
		self.pipeline_cache = pipeline_cache;
		self
	}

	/// Paces windows by the display instead of by a free swapchain image. Off by default; Metal and Vulkan read it.
	///
	/// Metal waits for a display link. Vulkan already waits for the display when the driver supports present waits,
	/// and with this on it also projects when the frame will be shown from the swapchain's presentation timings, when
	/// the surface reports them.
	///
	/// With it on, [`crate::context::Context::acquire_swapchain_image`] waits until the display schedules the frame
	/// and reports when that frame will be shown, through
	/// [`crate::frame::SwapchainAcquisition::target_present_time`]. Swapchains bound with
	/// [`crate::PresentationModes::Inmediate`] ignore it, since nothing waits on the display.
	pub fn display_link(mut self, display_link: bool) -> Self {
		self.display_link = display_link;
		self
	}
}
