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

/// Panics unless a ray tracing pipeline may launch a shader of `stage` as its ray generation shader. `name` is the
/// shader's name, when the backend keeps one.
// DX12 keeps shader types, not stages,, so only Metal and Vulkan call it.
#[cfg(not(target_os = "windows"))]
pub(crate) fn check_ray_generation_stage(backend: &str, name: Option<&str>, stage: crate::Stages) {
	assert!(
		stage == crate::Stages::RAYGEN,
		"{backend} ray tracing pipeline creation failed because its ray generation shader {} is a {stage:?} shader. The most likely cause is that the shaders were passed to ray_tracing::Builder::new in the wrong order. Pass the ray generation shader first and closest-hit shaders as the hit groups.",
		name.unwrap_or("(unnamed)"),
	);
}

/// Panics unless a ray tracing pipeline has hit groups: `hit_group_count` is how many its builder passes.
pub(crate) fn check_hit_group_count(backend: &str, hit_group_count: usize) {
	assert!(
		hit_group_count > 0,
		"{backend} ray tracing pipeline creation failed because the pipeline has no hit group. The most likely cause is that it was built before any closest-hit shader was ready. Pass at least one closest-hit shader, such as a default one for surfaces whose own shader isn't ready.",
	);
}

/// Panics unless a shader of `stage` may run as a hit group. `name` is the shader's name, when the backend keeps one.
// DX12 keeps shader types, not stages,, so only Metal and Vulkan call it.
#[cfg(not(target_os = "windows"))]
pub(crate) fn check_hit_group_stage(backend: &str, name: Option<&str>, stage: crate::Stages) {
	assert!(
		stage == crate::Stages::CLOSEST_HIT,
		"{backend} ray tracing pipeline creation failed because hit group shader {} is a {stage:?} shader. The most likely cause is that a shader of another stage was passed as a hit group. Pass only closest-hit shaders as hit groups.",
		name.unwrap_or("(unnamed)"),
	);
}

/// Panics because `operation` received a pipeline that isn't a ray tracing pipeline.
pub(crate) fn not_ray_tracing_pipeline(backend: &str, operation: &str) -> ! {
	panic!(
		"{backend} {operation} received a pipeline that isn't a ray tracing pipeline. The most likely cause is that a raster or compute pipeline handle was passed. Pass the handle of a ray tracing pipeline."
	)
}

/// Panics because a shader binding table write named a table the context doesn't have.
pub(crate) fn unknown_table_write(backend: &str) -> ! {
	panic!(
		"{backend} shader binding table write failed because the handle names no table of this context. The most likely cause is that the handle came from another context. Write tables through the context that created them."
	)
}

/// Panics unless every record of `hit_groups` names one of the `hit_group_count` hit groups of the pipeline a shader
/// binding table is written for.
pub(crate) fn check_records(backend: &str, hit_group_count: usize, hit_groups: &[u32]) {
	for (record, &group) in hit_groups.iter().enumerate() {
		assert!(
			(group as usize) < hit_group_count,
			"{backend} shader binding table write failed because record {record} names hit group {group}, and the pipeline has {hit_group_count} hit groups. The most likely cause is that the records were made for another version of the pipeline, with more hit groups. Write the table for the pipeline whose hit groups the records name.",
		);
	}
}

/// Returns the pipeline a trace launches, and panics when the recording bound none.
pub(crate) fn bound_pipeline(backend: &str, bound: Option<crate::PipelineHandle>) -> crate::PipelineHandle {
	bound.unwrap_or_else(|| {
		panic!(
			"{backend} trace_rays has no pipeline bound. The most likely cause is that trace_rays was called before bind_ray_tracing_pipeline. Bind the ray tracing pipeline first."
		)
	})
}

/// Panics because a trace named a shader binding table the context doesn't have.
pub(crate) fn unknown_table_trace(backend: &str) -> ! {
	panic!(
		"{backend} trace_rays failed because the shader binding table handle names no table of this context. The most likely cause is that the handle came from another context. Trace with tables the recording's context created."
	)
}

/// Panics unless the shader binding table `name` may be traced with `bound`. `written_for` is the pipeline the table
/// was last written for, or `None` when it was never written.
pub(crate) fn check_table_pipeline(
	backend: &str,
	name: Option<&str>,
	written_for: Option<crate::PipelineHandle>,
	bound: crate::PipelineHandle,
) {
	let Some(written_for) = written_for else {
		panic!(
			"{backend} trace_rays failed because shader binding table {name:?} has never been written. The most likely cause is that the table was traced before write_shader_binding_table gave it records. Write the table for the bound pipeline before tracing with it."
		);
	};
	assert!(
		written_for == bound,
		"{backend} trace_rays failed because shader binding table {name:?} was last written for pipeline {written_for:?}, and pipeline {bound:?} is bound. The most likely cause is that the pipeline was created again, with more hit groups, and the table wasn't written for the new one. Write the table for the bound pipeline before tracing with it.",
	);
}

/// Returns the width, height, and depth a trace of `extent` launches, or `None` when it launches nothing.
///
/// A height or depth of 0 counts as 1, as one- and two-dimensional extents leave them, and a width of 0 traces
/// nothing. Check the grid against the device's limit with [`check_launch`].
pub(crate) fn launch_grid(extent: ::utils::Extent) -> Option<[u32; 3]> {
	(extent.width() != 0).then(|| [extent.width(), extent.height().max(1), extent.depth().max(1)])
}

/// Panics unless one trace may launch `grid`, which holds at most `limit` invocations.
// Metal checks no launch limit, so only Vulkan and DX12 call it.
#[cfg(not(target_os = "macos"))]
pub(crate) fn check_launch(backend: &str, grid: [u32; 3], limit: u64) {
	let invocations = grid.iter().map(|&size| u64::from(size)).product::<u64>();
	assert!(
		invocations <= limit,
		"{backend} trace_rays launches more invocations than the device allows. The most likely cause is a launch sized for more rays than one trace can run. invocations={invocations}, limit={limit}. Split the launch into several traces.",
	);
}
