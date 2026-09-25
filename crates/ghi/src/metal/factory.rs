//! The `factory` module exposes detached Metal resource types for public API consumers.

/// The `Factory` struct provides detached Metal resource creation without owning render context state.
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

/// The `FactoryImage` type alias preserves the cross-platform detached image name.
pub type FactoryImage = image::Image;

/// The `FactorySampler` type alias preserves the cross-platform detached sampler name.
pub type FactorySampler = sampler::Sampler;

impl crate::device::Device for Factory {
	type Context = crate::metal::context::Context;
	type Allocator = std::alloc::Global;
	type RasterPipeline = Pipeline;
	type ComputePipeline = ComputePipeline;
	type Image = FactoryImage;
	type Sampler = FactorySampler;

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
		let shader = build_shader(
			&self.device,
			name,
			shader_source_type,
			stage,
			shader_resource_descriptors,
			self.settings.debug_labels,
		)?;
		self.shaders.push(shader);
		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
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

	/// Builds a Metal image that can be interned by a device later.
	fn build_image(&mut self, builder: crate::image::Builder) -> Self::Image {
		if builder.use_case == crate::UseCases::DYNAMIC {
			panic!(
				"Metal factory image creation does not support dynamic images. The most likely cause is that the image requires per-frame resource instances."
			);
		}

		if builder.device_accesses.intersects(crate::DeviceAccesses::HostOnly) {
			panic!(
				"Metal factory image creation does not support CPU-visible images. The most likely cause is that the image requires an associated staging buffer."
			);
		}

		build_image(
			&self.device,
			builder.name,
			image::ImageDescription::new(&builder),
			self.settings.debug_labels,
		)
	}

	/// Builds a Metal sampler that can be interned by a device later.
	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> Self::Sampler {
		build_sampler(&self.device, &builder, self.settings.debug_labels)
	}
}

use super::*;

/// These methods hand [`Factory`] products to a context so recordings can use them.
impl Context {
	/// Creates a [`Factory`] that builds shaders, pipelines, images, and samplers away from the render thread.
	pub fn create_factory(&self) -> Option<Factory> {
		Some(Factory::new(self.device.clone(), self.compiler.clone(), self.settings))
	}

	/// Interns a factory-built image into this device and returns its public image handle.
	pub fn intern_image(&mut self, image: image::Image) -> graphics_hardware_interface::ImageHandle {
		graphics_hardware_interface::ImageHandle(self.images.add(image).0)
	}

	/// Interns a factory-built sampler into this device and returns its public sampler handle.
	pub fn intern_sampler(&mut self, sampler: sampler::Sampler) -> graphics_hardware_interface::SamplerHandle {
		self.samplers.push(sampler);
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
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
