use math::{Point, Vector};

use crate::resources::ModelSpace;

/// The `VertexSkin` struct keeps one vertex's fixed-width joint and weight values together while they are imported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertexSkin {
	pub joints: [u16; 4],
	pub weights: [f32; 4],
}

/// The `MeshPrimitiveSource` trait provides borrowed mesh input to [`MeshProcessorSession`](super::MeshProcessorSession) without requiring owned attribute staging.
///
/// Implement each method with a concrete iterator over source-format data. The returned iterators are statically dispatched and
/// may normalize values while they are read. After creating a source, pass a shared reference to
/// [`MeshProcessorSession::push_primitive`](super::MeshProcessorSession::push_primitive).
pub trait MeshPrimitiveSource {
	type Error;

	/// Returns the index of this primitive's material in the list the importer passes when it finishes the mesh.
	///
	/// Geometry doesn't depend on material contents, so an importer can process primitives while their materials
	/// still bake and resolve the slots only at [`MeshProcessorSession::finish_into`](super::MeshProcessorSession::finish_into).
	/// Finishing the mesh panics when a slot is outside that list.
	fn material_slot(&self) -> usize;

	fn transform_node(&self) -> Option<u32> {
		None
	}

	fn skin(&self) -> Option<u32> {
		None
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error>;

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error>;

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<Vector<ModelSpace>, Self::Error>>>)
	}

	/// Returns each vertex's tangent direction in `[x, y, z]` and the handedness of its bitangent in `w`.
	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 4], Self::Error>>>)
	}

	fn bitangents(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<Vector<ModelSpace>, Self::Error>>>)
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 2], Self::Error>>>)
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 4], Self::Error>>>)
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<VertexSkin, Self::Error>>>)
	}
}
