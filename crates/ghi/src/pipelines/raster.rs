use std::borrow::Cow;

use crate::{
	pipelines::{ShaderParameter, VertexElement},
	DescriptorSetTemplateHandle, Formats,
};

pub struct Builder<'a> {
	pub(crate) descriptor_set_templates: Cow<'a, [DescriptorSetTemplateHandle]>,
	pub(crate) push_constant_ranges: Cow<'a, [crate::pipelines::PushConstantRange]>,
	pub(crate) vertex_elements: Cow<'a, [VertexElement<'a>]>,
	pub(crate) render_targets: Cow<'a, [AttachmentDescriptor]>,
	pub(crate) shaders: Cow<'a, [ShaderParameter<'a>]>,
	pub(crate) face_winding: FaceWinding,
	pub(crate) cull_mode: CullMode,
}

impl<'a> Builder<'a> {
	pub fn new(
		descriptor_set_templates: &'a [DescriptorSetTemplateHandle],
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		vertex_elements: &'a [VertexElement],
		shaders: &'a [ShaderParameter],
		render_targets: &'a [AttachmentDescriptor],
	) -> Self {
		Self {
			descriptor_set_templates: Cow::Borrowed(descriptor_set_templates),
			push_constant_ranges: Cow::Borrowed(push_constant_ranges),
			vertex_elements: Cow::Borrowed(vertex_elements),
			shaders: Cow::Borrowed(shaders),
			render_targets: Cow::Borrowed(render_targets),
			face_winding: FaceWinding::Clockwise,
			cull_mode: CullMode::Back,
		}
	}

	pub fn face_winding(mut self, face_winding: FaceWinding) -> Self {
		self.face_winding = face_winding;
		self
	}

	pub fn cull_mode(mut self, cull_mode: CullMode) -> Self {
		self.cull_mode = cull_mode;
		self
	}
}

#[derive(Clone, Copy, Default)]
pub enum FaceWinding {
	#[default]
	Clockwise,
	CounterClockwise,
}

#[derive(Clone, Copy, Default)]
pub enum CullMode {
	None,
	Front,
	#[default]
	Back,
}

#[derive(Clone, Copy, Default)]
pub enum BlendMode {
	#[default]
	None,
	Alpha,
}

#[derive(Clone, Copy)]
/// The `AttachmentDescriptor` struct captures the render-target state a raster pipeline needs for a single attachment.
pub struct AttachmentDescriptor {
	/// The format of the attachment.
	pub(crate) format: Formats,
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
	/// The blend behavior to use when writing the attachment.
	pub(crate) blend: BlendMode,
}

impl AttachmentDescriptor {
	pub fn new(format: Formats) -> Self {
		Self {
			format,
			layer: None,
			blend: BlendMode::None,
		}
	}

	pub fn layer(mut self, layer: u32) -> Self {
		self.layer = Some(layer);
		self
	}

	pub fn blend(mut self, blend: BlendMode) -> Self {
		self.blend = blend;
		self
	}
}

#[cfg(test)]
mod tests {
	use super::{AttachmentDescriptor, BlendMode, Builder, CullMode, FaceWinding};

	#[test]
	fn attachment_descriptor_defaults_to_no_blending() {
		let descriptor = AttachmentDescriptor::new(crate::Formats::RGBA8UNORM);

		assert!(matches!(descriptor.blend, BlendMode::None));
	}

	#[test]
	fn attachment_descriptor_can_enable_alpha_blending() {
		let descriptor = AttachmentDescriptor::new(crate::Formats::RGBA8UNORM).blend(BlendMode::Alpha);

		assert!(matches!(descriptor.blend, BlendMode::Alpha));
	}

	#[test]
	fn builder_defaults_to_clockwise_backface_culling() {
		let builder = Builder::new(&[], &[], &[], &[], &[]);

		assert!(matches!(builder.face_winding, FaceWinding::Clockwise));
		assert!(matches!(builder.cull_mode, CullMode::Back));
	}
}
