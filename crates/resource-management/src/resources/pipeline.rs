//! Persisted graphics pipeline descriptions.

/// The `Pipeline` struct exists to let render dependants request complete GPU pipelines by resource ID.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Pipeline {
	pub name: String,
	pub kind: PipelineKind,
}

/// The `PipelineKind` enum identifies the portable state needed by each pipeline class.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineKind {
	Compute {
		shader: String,
		#[serde(default)]
		push_constants: Vec<PushConstantRange>,
	},
	Raster {
		shaders: Vec<String>,
		#[serde(default)]
		push_constants: Vec<PushConstantRange>,
		#[serde(default)]
		vertex_elements: Vec<VertexElement>,
		attachments: Vec<Attachment>,
		#[serde(default)]
		face_winding: FaceWinding,
		#[serde(default)]
		cull_mode: CullMode,
		#[serde(default)]
		fill_mode: FillMode,
		#[serde(default = "default_depth_write")]
		depth_write: bool,
	},
	/// A screen pass whose shader is a `Tile` stage shader. The pass that runs it supplies its outputs' formats, which
	/// the pipeline is compiled for on platforms that run the shader as a tile function.
	Tile {
		shader: String,
		#[serde(default)]
		push_constants: Vec<PushConstantRange>,
	},
	/// A tile pipeline that a `.fusion` asset bakes from pixel step pipelines, so a renderer can record a run of those
	/// steps as one pass. Its shader runs step `i` while bit `i` of the 4-byte step mask in `push_constants` is set.
	///
	/// Renderers find these with a query for `Pipeline` resources whose `kind` is `fused`, and compile them like a
	/// [`Self::Tile`] pipeline.
	Fused {
		shader: String,
		/// The step pipelines' IDs, in the order the shader runs them.
		steps: Vec<String>,
		#[serde(default)]
		push_constants: Vec<PushConstantRange>,
	},
}

impl PipelineKind {
	/// Returns the name of the kind, as the `type` field of a `.pipeline` file and the `kind` query property spell it.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Compute { .. } => "compute",
			Self::Raster { .. } => "raster",
			Self::Tile { .. } => "tile",
			Self::Fused { .. } => "fused",
		}
	}
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PushConstantRange {
	pub offset: u32,
	pub size: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct VertexElement {
	pub name: String,
	pub format: Format,
	pub binding: u32,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Attachment {
	pub format: Format,
	#[serde(default)]
	pub layer: Option<u32>,
	#[serde(default)]
	pub blend: BlendMode,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
	Float,
	Float2,
	Float3,
	Float4,
	U16,
	U32,
	Rgba8Unorm,
	Rgba16Unorm,
	Rgba16Float,
	Rg11b10Float,
	Depth16,
	Depth32,
}

#[derive(
	Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
	#[default]
	None,
	Alpha,
	/// Source-over blending for color the shader already multiplied by its alpha.
	Premultiplied,
}

#[derive(
	Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FaceWinding {
	#[default]
	Clockwise,
	CounterClockwise,
}

#[derive(
	Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CullMode {
	None,
	Front,
	#[default]
	Back,
}

/// The `FillMode` enum selects whether a persisted raster pipeline fills triangles or draws their edges.
#[derive(
	Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
	/// Fills triangle interiors.
	#[default]
	Solid,
	/// Rasterizes triangle edges without filling their interiors.
	Wireframe,
}

const fn default_depth_write() -> bool {
	true
}

super::impl_direct_resource!(
	Pipeline,
	"Pipeline",
	references = pipeline_references,
	queryable = pipeline_queryable_properties
);

/// Lists the shaders the pipeline runs, by ID, then a fused pipeline's step pipelines.
fn pipeline_references(pipeline: &Pipeline) -> Vec<&str> {
	match &pipeline.kind {
		PipelineKind::Compute { shader, .. } | PipelineKind::Tile { shader, .. } => vec![shader.as_str()],
		PipelineKind::Raster { shaders, .. } => shaders.iter().map(String::as_str).collect(),
		PipelineKind::Fused { shader, steps, .. } => std::iter::once(shader).chain(steps).map(String::as_str).collect(),
	}
}

/// Indexes a pipeline by name and by kind, so renderers can find every fused pipeline in the store.
fn pipeline_queryable_properties(pipeline: &Pipeline, id: &str) -> Vec<crate::QueryableProperty> {
	vec![
		crate::QueryableProperty {
			name: "name".to_string(),
			value: id.into(),
		},
		crate::QueryableProperty {
			name: "kind".to_string(),
			value: pipeline.kind.name().into(),
		},
	]
}
