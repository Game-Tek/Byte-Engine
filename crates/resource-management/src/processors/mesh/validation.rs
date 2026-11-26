#[derive(Debug, PartialEq, Eq)]
pub enum MeshProcessingError {
	TooManyVertices {
		primitive: usize,
		vertices: usize,
	},
	TooManyLightmapVertices {
		primitive: usize,
		vertices: usize,
	},
	TooManyLevelMeshlets {
		primitive: usize,
		level: usize,
		meshlets: usize,
	},
	AttributeLengthMismatch(VertexSemantics),
	InvalidTriangleIndexCount,
	InvalidPositionData,
	FailedToBuildMeshlets,
}

impl std::fmt::Display for MeshProcessingError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::TooManyVertices { primitive, vertices } => write!(
				f,
				"Primitive {primitive} has {vertices} vertices, more than the 65,536 a primitive holds. The most likely cause is a glTF primitive that large: the importer splits large FBX primitives, but not glTF ones. Split the primitive in the source model."
			),
			Self::TooManyLightmapVertices { primitive, vertices } => write!(
				f,
				"Primitive {primitive} needs {vertices} vertices after its lightmap UVs split them, more than the 65,536 a primitive holds. The most likely cause is a large primitive whose surface turns often, so its charts share many vertices. Split the primitive in the source model, or set `lightmap_uvs.enabled` to `false` in its BEAD file."
			),
			Self::TooManyLevelMeshlets {
				primitive,
				level,
				meshlets,
			} => write!(
				f,
				"Detail level {level} of primitive {primitive} needs {meshlets} meshlets, more than the {} one level holds. The most likely cause is a primitive whose triangles share few vertices, so each meshlet fills its 64 vertices long before its 124 triangles. Weld the primitive's duplicate vertices or split it in the source model. See {}.",
				crate::resources::mesh::MAX_LEVEL_MESHLETS,
				crate::online_docs_url("reference/assets#detail-levels")
			),
			Self::AttributeLengthMismatch(semantic) => write!(
				f,
				"Mesh attribute length does not match the position stream. The most likely cause is that {semantic:?} does not contain one value per vertex."
			),
			Self::InvalidTriangleIndexCount => write!(
				f,
				"Triangle index count is invalid. The most likely cause is that the index stream length is not divisible by three."
			),
			Self::InvalidPositionData => write!(
				f,
				"Mesh position data is invalid. The most likely cause is an empty position stream or a non-finite component."
			),
			Self::FailedToBuildMeshlets => write!(
				f,
				"Meshlet generation failed. The most likely cause is that the packed position stream could not be adapted for meshopt."
			),
		}
	}
}

impl std::error::Error for MeshProcessingError {}

/// Returns whether each semantic of `vertex_layout` appears once, in its canonical format, as model handlers declare
/// it. The packer writes every stream in its canonical format, so any other declaration would mislabel the bytes.
pub(super) fn is_canonical_layout(vertex_layout: &[VertexComponent]) -> bool {
	vertex_layout.iter().enumerate().all(|(index, component)| {
		component.format == VertexComponent::canonical(component.semantic).format
			&& vertex_layout[..index]
				.iter()
				.all(|earlier| earlier.semantic != component.semantic)
	})
}

/// Returns whether `vertex_skin` targets joints of `skin` with normalized weights, as model handlers import them.
pub(super) fn is_normalized_vertex_skin(vertex_skin: &VertexSkin, skin: &SkinBinding) -> bool {
	let total = vertex_skin.weights.iter().sum::<f32>();
	vertex_skin.joints.iter().all(|joint| (*joint as usize) < skin.entries.len())
		&& vertex_skin.weights.iter().all(|weight| weight.is_finite() && *weight >= 0.0)
		&& (total - 1.0).abs() <= 1.0e-4
}

use super::source::VertexSkin;
use crate::{
	resources::skeleton::SkinBinding,
	types::{VertexComponent, VertexSemantics},
};
