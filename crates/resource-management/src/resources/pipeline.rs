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

super::impl_direct_resource!(Pipeline, "Pipeline");

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn raster_attachments_deserialize_integer_and_depth16_formats() {
		let pipeline: Pipeline = serde_json::from_str(
			r#"{"name":"Visibility","kind":{"type":"raster","shaders":[],"attachments":[{"format":"u32"},{"format":"depth16"}]}}"#,
		)
		.expect("Pipeline attachment formats must deserialize from their persisted names.");

		let PipelineKind::Raster { attachments, .. } = pipeline.kind else {
			panic!("Pipeline fixture must deserialize as a raster pipeline.");
		};

		assert!(matches!(attachments[0].format, Format::U32));
		assert!(matches!(attachments[1].format, Format::Depth16));
	}

	#[test]
	fn raster_fill_mode_defaults_to_solid_and_deserializes_wireframe() {
		let default_pipeline: Pipeline =
			serde_json::from_str(r#"{"name":"Solid","kind":{"type":"raster","shaders":[],"attachments":[]}}"#)
				.expect("Raster fill mode must remain optional for existing pipeline assets.");
		let PipelineKind::Raster { fill_mode, .. } = default_pipeline.kind else {
			panic!("Pipeline fixture must deserialize as a raster pipeline.");
		};
		assert!(matches!(fill_mode, FillMode::Solid));

		let wireframe_pipeline: Pipeline = serde_json::from_str(
			r#"{"name":"Wireframe","kind":{"type":"raster","shaders":[],"attachments":[],"fill_mode":"wireframe"}}"#,
		)
		.expect("Wireframe must deserialize as a portable raster fill mode.");
		let PipelineKind::Raster { fill_mode, .. } = wireframe_pipeline.kind else {
			panic!("Pipeline fixture must deserialize as a raster pipeline.");
		};
		assert!(matches!(fill_mode, FillMode::Wireframe));
	}
}
