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

#[derive(Clone, Copy)]
/// Stores the information of an attachment.
pub struct AttachmentDescriptor {
	/// The format of the attachment.
	pub(crate) format: Formats,
	/// The image layer index for the attachment.
	pub(crate) layer: Option<u32>,
}

impl AttachmentDescriptor {
	pub fn new(format: Formats) -> Self {
		Self { format, layer: None }
	}

	pub fn layer(mut self, layer: u32) -> Self {
		self.layer = Some(layer);
		self
	}
}
