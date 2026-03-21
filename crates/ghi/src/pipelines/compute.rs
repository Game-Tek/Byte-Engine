pub struct Builder<'a> {
	pub(crate) descriptor_set_templates: &'a [crate::DescriptorSetTemplateHandle],
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) shader: crate::pipelines::ShaderParameter<'a>,
}

impl<'a> Builder<'a> {
	pub fn new(
		descriptor_set_templates: &'a [crate::DescriptorSetTemplateHandle],
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		shader: crate::pipelines::ShaderParameter<'a>,
	) -> Self {
		Self {
			descriptor_set_templates,
			push_constant_ranges,
			shader,
		}
	}
}
