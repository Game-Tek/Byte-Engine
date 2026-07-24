pub struct Builder<'a> {
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) shader: crate::pipelines::ShaderParameter<'a>,
}

impl<'a> Builder<'a> {
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		shader: crate::pipelines::ShaderParameter<'a>,
	) -> Self {
		Self {
			push_constant_ranges,
			shader,
		}
	}
}
