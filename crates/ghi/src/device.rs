use crate::{
	image, pipelines, sampler,
	shader::{self, Sources},
	ShaderHandle, ShaderTypes,
};

/// The `Device` trait centralizes ownership of backend device state for the graphics hardware interface.
pub trait Device
where
	Self: Sized,
{
	type Context: crate::context::Context;
	type RasterPipeline;
	type ComputePipeline;
	type Image;
	type Sampler;

	/// Returns whether the backend API reported an error.
	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool;

	/// Creates a new rendering context that operates on this device.
	fn create_context(&self) -> Result<Self::Context, &'static str>;

	/// Creates a shader.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = shader::ShaderResourceDescriptor>,
	) -> Result<ShaderHandle, ()>;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> Self::RasterPipeline;

	/// Creates a compute pipeline from a builder.
	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> Self::ComputePipeline;

	/// Creates an image that can be interned into a rendering context later.
	fn build_image(&mut self, builder: image::Builder) -> Self::Image;

	/// Creates a sampler that can be interned into a rendering context later.
	fn build_sampler(&mut self, builder: sampler::Builder) -> Self::Sampler;
}

/// The `Features` struct selects optional GPU features during device creation.
///
/// Validation, API tracing, ray tracing, sparse resources, and geometry shaders
/// are disabled by default. Mesh shading is enabled by default. When `gpu` is
/// `None`, the backend selects an appropriate GPU.
#[derive(Debug, Clone, Copy)]
pub struct Features {
	pub(crate) validation: bool,
	pub(crate) gpu_validation: bool,
	pub(crate) api_dump: bool,
	pub(crate) ray_tracing: bool,
	pub(crate) debug_labels: bool,
	pub(crate) debug_log_function: Option<fn(&str)>,
	pub(crate) gpu: Option<&'static str>,
	pub(crate) sparse: bool,
	pub(crate) geometry_shader: bool,
	pub(crate) mesh_shading: bool,
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
			debug_log_function: None,
			gpu: None,
			sparse: false,
			geometry_shader: false,
			mesh_shading: true,
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
}
