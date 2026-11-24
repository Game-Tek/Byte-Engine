use math::{Point, Vector};

use crate::resources::ModelSpace;

/// The `VertexSkin` struct keeps one vertex's fixed-width joint and weight values together while they are imported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertexSkin {
	pub joints: [u16; 4],
	pub weights: [f32; 4],
}

/// The `AuthoredLevel` struct places one coarser detail level modeled by hand among a primitive's levels.
///
/// Return them from [`MeshPrimitiveSource::authored_levels`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthoredLevel {
	/// Where the level's triangles start in [`MeshPrimitiveSource::indices`], in indices.
	pub start: usize,
	/// The detail level it is, 1 or more, increasing from one authored level to the next. A number between two
	/// authored levels' is a gap the bake fills with simplified levels.
	pub level: u8,
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

	/// Returns the coarser detail levels modeled by hand, finest first.
	///
	/// The indices before the first level's start are full detail, and each level runs to the next start or the end.
	/// Every level indexes this primitive's vertices, so an importer appends a coarser model's vertices after full
	/// detail's. Mesh processing keeps each one as authored at its [`AuthoredLevel::level`], and simplifies levels only
	/// into the numbers no authored level takes. Empty, the default, means full detail alone, which mesh processing
	/// simplifies into every coarser level.
	fn authored_levels(&self) -> &[AuthoredLevel] {
		&[]
	}

	/// Returns how many detail levels the primitive holds at most, full detail included, or `None` to hold the
	/// [`MeshLodSettings::max_levels`](super::MeshLodSettings::max_levels) of the bake.
	///
	/// An importer limits it when other primitives take over from this one at a later level, as
	/// [`MeshProcessorSession::replace_primitives`](super::MeshProcessorSession::replace_primitives) records, so the
	/// bake generates no levels past the takeover.
	fn level_count(&self) -> Option<u8> {
		None
	}

	/// Returns the radius a renderer measures the primitive by when it culls small shadow casters, or `None`, the
	/// default, for the radius of the primitive's bounding sphere. A source that repeats geometry returns the radius
	/// of its largest copy.
	fn caster_radius(&self) -> Option<f32> {
		None
	}

	/// Returns the primitive's triangles, three vertex indices each.
	///
	/// Wind each triangle so that `(b − a) × (c − a)` points behind its front face, into the surface. Mesh processing
	/// swaps every triangle's last two indices, which turns that winding into the engine's
	/// [front faces](/docs/reference/conventions#camera-and-clip-space).
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
