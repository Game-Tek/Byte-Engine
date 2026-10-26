/// The `Builder` struct describes a ray tracing pipeline: the ray generation shader a trace launches, and the hit
/// groups that shader runs for the hits it finds.
///
/// Each hit group is one closest-hit shader, numbered by its position in `hit_groups`. Create the pipeline with
/// [`crate::context::ContextCreate::create_ray_tracing_pipeline`], or on another thread with a factory's
/// [`crate::device::Device::create_ray_tracing_pipeline`]. Next, map hit records to its groups with
/// [`crate::context::ContextCreate::write_shader_binding_table`], then launch it with
/// [`crate::command_buffer::BoundRayTracingPipelineMode::trace_rays`].
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) ray_generation: crate::pipelines::ShaderParameter<'a>,
	pub(crate) hit_groups: &'a [crate::pipelines::ShaderParameter<'a>],
}

impl<'a> Builder<'a> {
	/// Describes a pipeline that launches `ray_generation` and runs the closest-hit shaders of `hit_groups`.
	///
	/// The ray generation shader binds every descriptor set it declares, and each closest-hit shader declares the first
	/// of those sets, in the same order, or fewer. Pass at least one hit group.
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		ray_generation: crate::pipelines::ShaderParameter<'a>,
		hit_groups: &'a [crate::pipelines::ShaderParameter<'a>],
	) -> Self {
		Self {
			name: None,
			push_constant_ranges,
			ray_generation,
			hit_groups,
		}
	}

	/// Names this pipeline for graphics debuggers.
	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}
}
