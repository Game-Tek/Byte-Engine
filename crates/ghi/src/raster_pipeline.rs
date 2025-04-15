use std::borrow::Cow;

use crate::{PipelineAttachmentInformation, PipelineLayoutHandle, ShaderParameter, VertexElement};

pub struct Builder<'a> {
	pub(super) layout: PipelineLayoutHandle,
	pub(super) vertex_elements: Cow<'a, [VertexElement]>,
	pub(super) render_targets: Cow<'a, [PipelineAttachmentInformation]>,
	pub(super) shaders: Cow<'a, [ShaderParameter<'a>]>,
}

impl <'a> Builder<'a> {
	pub fn new(layout: PipelineLayoutHandle, vertex_elements: &'a [VertexElement], shaders: &'a [ShaderParameter], render_targets: &'a [PipelineAttachmentInformation]) -> Self {
		Self { layout, vertex_elements: Cow::Borrowed(vertex_elements), shaders: Cow::Borrowed(shaders), render_targets: Cow::Borrowed(render_targets) }
	}
}