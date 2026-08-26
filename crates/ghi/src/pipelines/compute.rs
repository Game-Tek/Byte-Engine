/// The `Builder` struct collects portable compute state before a backend creates its native pipeline.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) shader: crate::pipelines::ShaderParameter<'a>,
}

impl<'a> Builder<'a> {
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		shader: crate::pipelines::ShaderParameter<'a>,
	) -> Self {
		Self {
			name: None,
			push_constant_ranges,
			shader,
		}
	}

	/// Names this pipeline for graphics debuggers.
	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}
}
