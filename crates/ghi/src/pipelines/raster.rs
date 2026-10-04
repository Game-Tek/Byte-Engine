use crate::{
	Formats,
	pipelines::{ShaderParameter, VertexElement},
};

/// The `Builder` struct collects portable raster state before a backend creates its native pipeline.
pub struct Builder<'a> {
	pub(crate) name: Option<&'a str>,
	pub(crate) push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
	pub(crate) vertex_elements: &'a [VertexElement<'a>],
	pub(crate) render_targets: &'a [AttachmentDescriptor],
	pub(crate) shaders: &'a [ShaderParameter<'a>],
	pub(crate) face_winding: FaceWinding,
	pub(crate) cull_mode: CullMode,
	pub(crate) fill_mode: FillMode,
	pub(crate) depth_write: bool,
}

impl<'a> Builder<'a> {
	pub fn new(
		push_constant_ranges: &'a [crate::pipelines::PushConstantRange],
		vertex_elements: &'a [VertexElement],
		shaders: &'a [ShaderParameter],
		render_targets: &'a [AttachmentDescriptor],
	) -> Self {
		Self {
			name: None,
			push_constant_ranges,
			vertex_elements,
			shaders,
			render_targets,
			face_winding: FaceWinding::Clockwise,
			cull_mode: CullMode::Back,
			fill_mode: FillMode::Solid,
			depth_write: true,
		}
	}

	/// Names this pipeline for graphics debuggers.
	pub fn name(mut self, name: &'a str) -> Self {
		self.name = Some(name);
		self
	}

	pub fn face_winding(mut self, face_winding: FaceWinding) -> Self {
		self.face_winding = face_winding;
		self
	}

	pub fn cull_mode(mut self, cull_mode: CullMode) -> Self {
		self.cull_mode = cull_mode;
		self
	}

	/// Selects whether triangles are filled or rasterized as their edges.
	pub fn fill_mode(mut self, fill_mode: FillMode) -> Self {
		self.fill_mode = fill_mode;
		self
	}

	/// Selects whether passing depth fragments update the bound depth attachment.
	pub fn depth_write(mut self, depth_write: bool) -> Self {
		self.depth_write = depth_write;
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

/// The `FillMode` enum selects how a raster pipeline converts triangles into fragments.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum FillMode {
	/// Fills triangle interiors.
	#[default]
	Solid,
	/// Rasterizes triangle edges without filling their interiors.
	Wireframe,
}

#[derive(Clone, Copy, Default)]
pub enum BlendMode {
	#[default]
	None,
	/// Applies straight-alpha source-over blending to both color and alpha.
	Alpha,
	/// Applies source-over blending to color that the shader already multiplied by its alpha.
	///
	/// An alpha of `0` adds the color, which suits emissive effects such as sparks, and an alpha of `1` covers what is
	/// behind it. Both kinds can share one draw without sorting against each other only when they are additive.
	Premultiplied,
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
