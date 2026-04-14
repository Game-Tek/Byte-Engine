use crate::{
	pipelines,
	shader::{self, Sources},
	ShaderHandle, ShaderTypes,
};

pub trait Factory {
	type RasterPipeline;

	/// Creates a shader.
	/// # Arguments
	/// * `name` - The name of the shader.
	/// * `shader_source_type` - The type of the shader source.
	/// * `stage` - The stage of the shader.
	/// * `shader_binding_descriptors` - The binding descriptors of the shader.
	/// # Returns
	/// The handle of the shader.
	/// # Errors
	/// Returns an error if the shader source was GLSL source code and could not be compiled.
	/// Returns an error if the shader source was SPIR-V binary and could not aligned to 4 bytes.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: Sources,
		stage: ShaderTypes,
		shader_binding_descriptors: impl IntoIterator<Item = shader::BindingDescriptor>,
	) -> Result<ShaderHandle, ()>;

	/// Creates a graphics/rasterization pipeline from a builder.
	fn create_raster_pipeline(&mut self, builder: pipelines::raster::Builder) -> Self::RasterPipeline;

	type ComputePipeline;

	/// Creates a compute pipeline from a builder.
	fn create_compute_pipeline(&mut self, builder: pipelines::compute::Builder) -> Self::ComputePipeline;
}
