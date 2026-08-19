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

	fn material(&self) -> &crate::ReferenceModel<crate::resources::material::VariantModel>;

	fn transform_node(&self) -> Option<u32> {
		None
	}

	fn skin(&self) -> Option<u32> {
		None
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error>;

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_, Self::Error>;

	fn normals(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 3], Self::Error>>>)
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 4], Self::Error>>>)
	}

	fn bitangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
		Ok(None::<std::iter::Empty<Result<[f32; 3], Self::Error>>>)
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
