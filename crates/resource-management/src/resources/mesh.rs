use crate::{
	Reference, ReferenceModel, Solver, resource,
	resources::material::VariantModel,
	resources::skeleton::{Skeleton, SkeletonModel, SkinBinding, SkinJoint},
	solver::SolveError,
	types::{IndexStreamTypes, Stream, Streams, VertexComponent, VertexSemantics},
};

/// The `Primitive` struct supplies one renderable geometry range and its skeletal bindings to runtime rendering.
///
/// The same type is stored and loaded: a primitive names its material by index into [`Mesh::materials`], so
/// primitives that share a material share one reference and nothing is solved per primitive.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Primitive {
	/// Index of this primitive's material variant in [`Mesh::materials`].
	pub material: u32,
	pub transform_node: Option<u32>,
	pub skin: Option<u32>,
	pub streams: Vec<Stream>,
	pub bounding_box: math::AABB<crate::resources::ModelSpace>,
	/// A sphere around the full-detail triangles, as xyz center and w radius. It bounds every level, so a renderer
	/// culls the primitive with it whatever level it has loaded.
	pub bounding_sphere: [f32; 4],
	pub vertex_count: u32,
	/// The detail levels of this primitive, finest first. Level 0 is always present. It is full detail, with an error
	/// of zero, unless this primitive takes the place of others of its mesh, as a billboard does: then its error is
	/// the one those primitives are [replaced at](Self::replaced_at), and a renderer draws this primitive only where
	/// that error stays invisible.
	pub lods: Vec<PrimitiveLod>,
	/// The error from which other primitives of the mesh take this one's place, when any do. A renderer draws this
	/// primitive only where that error would show. The primitives on both sides share one [`Self::bounding_sphere`],
	/// so a renderer that measures distance with it switches them together.
	pub replaced_at: Option<f32>,
	/// The radius a renderer measures the primitive by when it decides whether a shadow caster is too small to draw,
	/// or `None` for the radius of [`Self::bounding_sphere`]. A primitive that repeats a mesh, as a level's
	/// [cluster mesh](/docs/reference/levels#merged-placements) does, holds the radius of its largest copy, since its
	/// sphere spans every copy.
	pub caster_radius: Option<f32>,
}

/// The most detail levels one primitive stores, counting full detail. Renderers size fixed level tables with it.
pub const MAX_PRIMITIVE_LODS: usize = 8;

/// The most vertices one primitive holds: its index streams store 16-bit indices.
pub const MAX_PRIMITIVE_VERTICES: usize = 1 << 16;

/// The most meshlets one detail level of a primitive holds. Renderers address a level's meshlets with 12 bits,
/// relative to its first meshlet, so baking fails for a level with more and loading rejects one.
pub const MAX_LEVEL_MESHLETS: usize = 1 << 12;

/// The `PrimitiveLod` struct lets a renderer load and draw one detail level of a primitive.
///
/// Each level's meshlets live in that level's streams, and they index a prefix of the primitive's vertices: the baker
/// orders vertices by the coarsest level that uses them, coarsest first. So a renderer that holds a level holds the
/// vertices of every coarser level too, and loading a finer level only appends vertices. Read the levels from
/// [`Primitive::lods`], and pick the coarsest one whose [`Self::error`] stays invisible at the distance the primitive
/// is drawn from.
#[derive(
	Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct PrimitiveLod {
	pub meshlet_count: u32,
	/// How many of the primitive's first vertices this level and the coarser ones use. Every index this level's
	/// meshlets name is below it.
	pub vertex_count: u32,
	/// Estimated largest distance, in model-space units, between this level's surface and full detail. Zero for
	/// level 0.
	pub error: f32,
}

impl Primitive {
	/// Returns this primitive's range of one stream at one detail level, relative to the mesh's stream of that type
	/// and level.
	pub fn stream(&self, stream_type: Streams, level: u8) -> Option<&Stream> {
		self.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type && stream.level == level)
	}
}

super::impl_resource_model!(Primitive, Primitive, "Primitive");

/// The `Mesh` struct supplies packed geometry, material primitives, and optional skeletal bindings to runtime rendering.
///
/// Every stream belongs to one detail level, and each level's streams list the primitives in order. The index streams
/// use these meanings:
///
/// - `Vertices` entries index the primitive's vertices.
/// - `Meshlets` entries index their meshlet's entries in the `Vertices` stream.
/// - `Triangles`, stored at level 0 only, index the primitive's vertices.
///
/// Material variants stay unsolved: request each one by its ID when it is needed, which lets renderers load and
/// deduplicate materials on their own schedule instead of once per primitive.
#[derive(Debug, serde::Serialize)]
pub struct Mesh {
	pub skeleton: Option<Reference<Skeleton>>,
	pub skins: Vec<SkinBinding>,
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	/// The distinct material variants the primitives draw with.
	pub materials: Vec<ReferenceModel<VariantModel>>,
	pub primitives: Vec<Primitive>,
}

impl Mesh {
	pub fn primitives(&self) -> impl Iterator<Item = &Primitive> {
		self.primitives.iter()
	}

	/// Returns the material variant a primitive draws with.
	pub fn material(&self, primitive: &Primitive) -> &ReferenceModel<VariantModel> {
		&self.materials[primitive.material as usize]
	}

	/// Returns the mesh's stream of one type at one detail level. Its offset addresses the resource payload.
	pub fn stream(&self, stream_type: Streams, level: u8) -> Option<&Stream> {
		self.streams
			.iter()
			.find(|stream| stream.stream_type == stream_type && stream.level == level)
	}

	/// Returns the smallest box that holds every primitive's bounding box, in the coordinates the mesh was authored in,
	/// such as to frame the mesh in a preview; `None` for a mesh with no primitives. Node transforms aren't applied.
	pub fn bounding_box(&self) -> Option<math::AABB<crate::resources::ModelSpace>> {
		self.primitives
			.iter()
			.map(|primitive| primitive.bounding_box)
			.reduce(|bounds, next| bounds.union(&next))
	}

	/// Returns how many detail levels the mesh stores: the most of any primitive.
	pub fn level_count(&self) -> u8 {
		self.primitives
			.iter()
			.map(|primitive| primitive.lods.len() as u8)
			.max()
			.unwrap_or(0)
	}
}

/// The `MeshModel` struct preserves processed geometry and skeletal bindings for storage and later runtime solving.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct MeshModel {
	pub skeleton: Option<ReferenceModel<SkeletonModel>>,
	pub skins: Vec<SkinBinding>,
	pub vertex_components: Vec<VertexComponent>,
	pub streams: Vec<Stream>,
	/// The distinct material variants, each stored once however many primitives use it.
	pub materials: Vec<ReferenceModel<VariantModel>>,
	pub primitives: Vec<Primitive>,
}

super::impl_resource_model!(Mesh, MeshModel, "Mesh", references = mesh_references);

/// Lists the mesh's skeleton, then its material variants.
fn mesh_references(mesh: &MeshModel) -> Vec<&str> {
	mesh.skeleton
		.iter()
		.map(|skeleton| skeleton.id().as_str())
		.chain(mesh.materials.iter().map(|material| material.id().as_str()))
		.collect()
}

impl crate::StoredModel for MeshModel {
	type Resource = Mesh;

	/// Resolves the mesh's skeleton. The bake that stored the mesh guarantees its skin tables, material indices, and
	/// detail levels match its streams.
	fn solve_stored<'de>(
		gr: crate::SerializableResource,
		reader: crate::resource::resource_handler::MultiResourceReader,
		storage_backend: &'de dyn resource::DynReadStorageBackend,
	) -> crate::r#async::BoxedFuture<'de, Result<Reference<Mesh>, SolveError>> {
		crate::r#async::future(async move {
			let MeshModel {
				skeleton,
				skins,
				vertex_components,
				streams,
				materials,
				primitives,
			} = crate::from_slice(&gr.resource).map_err(|error| {
				SolveError::DeserializationFailed(format!(
					"Mesh resource could not be deserialized. The most likely cause is incompatible or corrupted mesh metadata: {error}."
				))
			})?;

			let skeleton = match skeleton {
				Some(skeleton) => Some(skeleton.solve(storage_backend).await?),
				None => None,
			};

			Ok(Reference::from_stored(
				gr,
				Mesh {
					skeleton,
					skins,
					vertex_components,
					streams,
					materials,
					primitives,
				},
				reader,
			))
		})
	}
}
