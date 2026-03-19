use std::borrow::Cow;

use crate::{
	pipelines::{ShaderParameter, VertexElement},
	Formats, PipelineLayoutHandle,
};

pub struct Builder<'a> {
	pub(crate) layout: PipelineLayoutHandle,
	pub(crate) vertex_elements: Cow<'a, [VertexElement<'a>]>,
	pub(crate) render_targets: Cow<'a, [AttachmentDescriptor]>,
	pub(crate) shaders: Cow<'a, [ShaderParameter<'a>]>,
}

impl<'a> Builder<'a> {
	pub fn new(
		layout: PipelineLayoutHandle,
		vertex_elements: &'a [VertexElement],
		shaders: &'a [ShaderParameter],
		render_targets: &'a [AttachmentDescriptor],
	) -> Self {
		Self {
			layout,
			vertex_elements: Cow::Borrowed(vertex_elements),
			shaders: Cow::Borrowed(shaders),
			render_targets: Cow::Borrowed(render_targets),
		}
	}
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
	use super::{AttachmentDescriptor, BlendMode};

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
}
