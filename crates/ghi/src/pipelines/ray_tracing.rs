use std::borrow::Cow;

pub struct Builder<'a> {
	pub(crate) push_constant_ranges: Cow<'a, [crate::pipelines::PushConstantRange]>,
	pub(crate) shaders: Cow<'a, [crate::pipelines::ShaderParameter<'a>]>,
}

impl<'a> Builder<'a> {
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		shaders: &'a [crate::pipelines::ShaderParameter<'a>],
	) -> Self {
		Self {
			push_constant_ranges: Cow::Borrowed(push_constant_ranges),
			shaders: Cow::Borrowed(shaders),
		}
	}
}
