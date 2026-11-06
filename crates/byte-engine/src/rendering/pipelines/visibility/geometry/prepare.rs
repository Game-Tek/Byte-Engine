//! Worker-side conversion of mesh sources into the exact bytes the visibility geometry buffers store.
//!
//! Preparation loads or generates a mesh into one leased region of the upload arena and converts attributes into
//! the runtime formats (octahedral normals, half-float UVs, [`ShaderMeshletData`] and [`ShaderMeshLevel`] records). It
//! never assigns renderer slots or buffer offsets; that happens in [`GeometryBuffers::append_mesh`], which moves the
//! records to their place with [`PreparedMesh::place_records`].

use std::ops::Range;
use std::sync::Arc;

use resource_management::Reference;
use resource_management::resources::mesh::MAX_PRIMITIVE_LODS;
use resource_management::resources::mesh::Mesh;
use resource_management::resources::skeleton::SkinBinding;
use resource_management::stream::StreamMut;
use resource_management::types::{IndexStreamTypes, Stream, Streams, VertexSemantics};

use super::{
	GeometryCounts, MeshPrimitive, MeshRanges, PrimitiveLevels, SKINNING_JOINTS_STRIDE, SKINNING_NORMAL_STRIDE,
	SKINNING_POSITION_STRIDE, SKINNING_WEIGHTS_STRIDE,
};
use crate::rendering::mesh::generator::{MeshGenerator, validate_triangle_indices};
use crate::rendering::pipelines::visibility::layout::{
	RuntimeUnitVector, ShaderMeshLevel, ShaderMeshletData, TRIANGLE_COUNT, VERTEX_COUNT, VERTEX_LIGHTMAP_UV_BUFFER_STRIDE,
	VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE, level_sphere,
};
use crate::rendering::pipelines::visibility::lod::{MeshLod, MeshLods};
use crate::rendering::resource_loading::{StagingLease, UploadStagingArena};

/// Byte size of one baked meshlet record: two u8 counts, padding, and three packed vec4 bounds.
const RESOURCE_MESHLET_STRIDE: usize = 52;
const VERTEX_INDEX_STRIDE: usize = 2;
const F32_UV_STRIDE: usize = 8;
const F16_UV_STRIDE: usize = 4;
/// Upload arena alignment that satisfies every backend's buffer-copy requirement.
const STAGING_ALIGNMENT: usize = 256;
/// Material used by generated meshes that do not name their own.
pub(crate) const GENERATED_MESH_MATERIAL: &str = "white_solid.bema";

/// The `PreparedMesh` struct retains a mesh's converted geometry in its staging lease until the loader copied it.
pub(crate) struct PreparedMesh {
	pub(crate) staging: StagingLease,
	pub(super) streams: PreparedStreams,
	pub(crate) primitives: Vec<PreparedPrimitive>,
	pub(crate) counts: GeometryCounts,
	pub(crate) skeleton_node_count: u32,
}

/// The `PreparedStreams` struct locates each runtime stream inside the staging lease.
pub(super) struct PreparedStreams {
	pub(super) positions: Range<usize>,
	pub(super) normals: Range<usize>,
	pub(super) uvs: Range<usize>,
	/// Zero for every vertex of a primitive without lightmap UVs.
	pub(super) lightmap_uvs: Range<usize>,
	pub(super) vertex_indices: Range<usize>,
	pub(super) primitive_indices: Range<usize>,
	pub(super) meshlets: Range<usize>,
	/// One [`ShaderMeshLevel`] per held level of every primitive.
	pub(super) levels: Range<usize>,
	/// Present when the pipeline keeps flat triangle lists for ray tracing.
	pub(super) flat_triangles: Option<Range<usize>>,
}

/// The `PreparedPrimitive` struct is one primitive's record and where its elements lie in the prepared streams.
pub(crate) struct PreparedPrimitive {
	/// `material_index` and `skinning_source_vertex_offset` are finalized by [`GeometryBuffers::append_mesh`].
	pub(crate) primitive: MeshPrimitive,
	/// The primitive's elements of each prepared stream, so it can be placed apart from the rest of the mesh. Its
	/// level bases count from the start of the prepared streams, like these ranges.
	pub(crate) elements: MeshRanges,
	pub(super) skinning: Option<SkinningCopy>,
}

/// The `SkinningCopy` struct locates one skinned primitive's bind-pose streams inside the staging lease.
pub(super) struct SkinningCopy {
	pub(super) positions: Range<usize>,
	pub(super) normals: Range<usize>,
	pub(super) joints: Range<usize>,
	pub(super) weights: Range<usize>,
}

/// Reserves one 4-byte aligned range in a staging layout.
fn take_range(cursor: &mut usize, size: usize) -> Range<usize> {
	let start = cursor.next_multiple_of(4);
	*cursor = start + size;
	start..*cursor
}

impl PreparedMesh {
	/// Builds transfer-ready geometry from a generated mesh, with its flat triangle list when `flat_triangles` is set.
	pub(crate) async fn generated(
		generator: &dyn MeshGenerator,
		upload_staging: Arc<UploadStagingArena>,
		flat_triangles: bool,
	) -> Option<Self> {
		let positions = generator.positions();
		let normals = generator.normals();
		let uvs = generator.uvs();
		if positions.len() != normals.len() || positions.len() != uvs.len() {
			log::error!(
				"Generated mesh attributes are inconsistent. The most likely cause is that the mesh generator returned mismatched vertex attribute counts."
			);
			return None;
		}
		let indices = generator.indices();
		if let Err(error) = validate_triangle_indices(&indices, positions.len()) {
			log::error!("{error}");
			return None;
		}
		// Validation proved every index fits in 16 bits.
		let indices = indices.iter().map(|&index| index as u16).collect::<Vec<_>>();
		let (vertex_indices, primitive_indices, meshlets, level) = build_generated_meshlets(&indices, &positions);

		let mut cursor = 0;
		let streams = PreparedStreams {
			positions: take_range(&mut cursor, positions.len() * 12),
			normals: take_range(&mut cursor, normals.len() * VERTEX_NORMAL_BUFFER_STRIDE as usize),
			uvs: take_range(&mut cursor, uvs.len() * VERTEX_UV_BUFFER_STRIDE as usize),
			lightmap_uvs: take_range(&mut cursor, uvs.len() * VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize),
			vertex_indices: take_range(&mut cursor, vertex_indices.len() * VERTEX_INDEX_STRIDE),
			primitive_indices: take_range(&mut cursor, primitive_indices.len() * 3),
			meshlets: take_range(&mut cursor, meshlets.len() * std::mem::size_of::<ShaderMeshletData>()),
			levels: take_range(&mut cursor, std::mem::size_of::<ShaderMeshLevel>()),
			// A generated mesh is one primitive whose triangle list is already flat.
			flat_triangles: flat_triangles.then(|| take_range(&mut cursor, indices.len() * VERTEX_INDEX_STRIDE)),
		};
		let mut staging = allocate_staging(&upload_staging, cursor).await?;
		let backing = staging.bytes_mut();
		for (destination, &(x, y, z)) in backing[streams.positions.clone()]
			.as_chunks_mut::<12>()
			.0
			.iter_mut()
			.zip(positions.iter())
		{
			*destination = bytemuck::cast([x, y, z]);
		}
		for (destination, normal) in backing[streams.normals.clone()]
			.as_chunks_mut::<4>()
			.0
			.iter_mut()
			.zip(normals.iter())
		{
			write_unit_vector(destination, *normal);
		}
		for (destination, (u, v)) in backing[streams.uvs.clone()].as_chunks_mut::<4>().0.iter_mut().zip(uvs.iter()) {
			write_f16_pair(destination, *u, *v);
		}
		let lightmap_uvs = generator
			.lightmap_uvs()
			.filter(|lightmap_uvs| lightmap_uvs.len() == uvs.len());
		let lightmap_destination = backing[streams.lightmap_uvs.clone()].as_chunks_mut::<4>().0;
		match lightmap_uvs {
			Some(lightmap_uvs) => {
				for (destination, &(u, v)) in lightmap_destination.iter_mut().zip(lightmap_uvs.iter()) {
					write_unorm16_pair(destination, u, v);
				}
			}
			None => lightmap_destination.as_flattened_mut().fill(0),
		}
		backing[streams.vertex_indices.clone()].copy_from_slice(bytemuck::cast_slice(&vertex_indices));
		backing[streams.primitive_indices.clone()].copy_from_slice(bytemuck::cast_slice(&primitive_indices));
		backing[streams.meshlets.clone()].copy_from_slice(bytemuck::cast_slice(&meshlets));
		backing[streams.levels.clone()].copy_from_slice(bytemuck::bytes_of(&level));
		if let Some(range) = &streams.flat_triangles {
			backing[range.clone()].copy_from_slice(bytemuck::cast_slice(&indices));
		}
		let flat_triangle_count = streams.flat_triangles.as_ref().map_or(0, |_| (indices.len() / 3) as u32);

		Some(Self {
			staging,
			streams,
			primitives: vec![PreparedPrimitive {
				primitive: MeshPrimitive {
					lods: MeshLods::single(meshlets.len() as u32),
					bounding_sphere: level.bounding_sphere,
					vertex_count: positions.len() as u32,
					flat_triangle_count,
					..MeshPrimitive::default()
				},
				elements: MeshRanges {
					vertices: 0..positions.len() as u32,
					primitive_indices: 0..vertex_indices.len() as u32,
					triangles: 0..primitive_indices.len() as u32,
					meshlets: 0..meshlets.len() as u32,
					levels: 0..1,
					..MeshRanges::default()
				},
				skinning: None,
			}],
			counts: GeometryCounts {
				vertices: positions.len() as u32,
				primitive_indices: vertex_indices.len() as u32,
				triangles: primitive_indices.len() as u32,
				meshlets: meshlets.len() as u32,
				levels: 1,
				skinning_vertices: 0,
				flat_triangles: flat_triangle_count,
			},
			skeleton_node_count: 0,
		})
	}

	/// Loads the detail `levels` of a baked mesh resource and converts them into the runtime geometry formats, with
	/// flat triangle lists of each primitive's traced level when `flat_triangles` is set: its coarsest for
	/// [`PreparedLevels::Coarsest`] and its full detail for [`PreparedLevels::Finest`].
	///
	/// The prepared mesh holds each primitive's meshlets of the levels `levels` names, and every vertex they index: the
	/// primitive's vertices from its coarsest level down to the finest level held. Its primitives' level tables mark
	/// those levels as resident. Each needed level stream loads into the front of the staging lease, and every runtime
	/// stream is gathered behind them, primitive by primitive.
	pub(crate) async fn resource(
		mut resource: Reference<Mesh>,
		upload_staging: Arc<UploadStagingArena>,
		flat_triangles: bool,
		levels: PreparedLevels<'_>,
	) -> Option<Self> {
		let mesh = resource.resource();
		let plan = ResourcePlan::new(mesh, levels, flat_triangles)?;
		let skeleton_node_count = mesh
			.skeleton
			.as_ref()
			.map_or(0, |skeleton| skeleton.resource().nodes.len() as u32);
		// The bindings move out of the resource, which this function drops: nothing below reads its skins. Only a
		// preparation that holds a skinned primitive needs them.
		let skins = match plan.skinning {
			Some(_) => std::mem::take(&mut resource.resource_mut().skins)
				.into_iter()
				.map(Arc::new)
				.collect::<Vec<_>>(),
			None => Vec::new(),
		};
		let mut staging = allocate_staging(&upload_staging, plan.backing_size).await?;
		let backing = staging.bytes_mut();
		let (source_bytes, output) = backing.split_at_mut(plan.source_size);

		resource.load(plan.read_targets(source_bytes).into()).await.ok().or_else(|| {
			log::error!(
				"Mesh resource streams could not be loaded. The most likely cause is that the baked mesh payload is missing or unreadable."
			);
			None
		})?;
		let (primitives, flat_triangle_count) = plan.convert(resource.resource(), levels, source_bytes, output, &skins)?;
		let mut streams = plan.streams;
		let mut counts = plan.counts;
		if let Some(range) = streams.flat_triangles.as_mut() {
			range.end = range.start + flat_triangle_count as usize * 6;
			counts.flat_triangles = flat_triangle_count;
		}

		Some(Self {
			staging,
			streams,
			primitives,
			counts,
			skeleton_node_count,
		})
	}
}

/// The `PreparedSurface` struct is one prepared primitive's full-detail triangles, read back from the staging lease
/// before it uploads: what the lightmap bake rasterizes into texels.
///
/// [`PreparedMesh::surface`] returns it. Its streams are in the runtime formats; decode normals with
/// [`decode_octahedral_unit_vector`] and lightmap UVs by dividing by 65,535.
pub(crate) struct PreparedSurface<'a> {
	/// Object-space positions of the primitive's vertices.
	pub(crate) positions: &'a [[f32; 3]],
	/// Octahedrally encoded object-space normals.
	pub(crate) normals: &'a [[u16; 2]],
	/// UNORM16 lightmap UVs, all zero for a primitive without them.
	pub(crate) lightmap_uvs: &'a [[u16; 2]],
	/// The full-detail triangles, as indices of the vertices above.
	pub(crate) triangles: &'a [[u16; 3]],
}

impl PreparedMesh {
	/// Returns the full-detail triangles of primitive `index`, or `None` when the mesh holds no flat triangle list for
	/// it: it was prepared without one, or the primitive is skinned.
	///
	/// Only a mesh prepared with [`PreparedLevels::Finest`] or generated holds every triangle of its primitives here.
	pub(crate) fn surface(&self, index: usize) -> Option<PreparedSurface<'_>> {
		let flat = self.streams.flat_triangles.as_ref()?;
		let prepared = self.primitives.get(index)?;
		let primitive = &prepared.primitive;
		if primitive.flat_triangle_count == 0 {
			return None;
		}
		let bytes = self.staging.bytes();
		let vertices = prepared.elements.vertices.start as usize..prepared.elements.vertices.end as usize;
		let stream = |range: &Range<usize>, stride: usize| &bytes[range.start + vertices.start * stride..][..vertices.len() * stride];
		let triangles = primitive.flat_triangle_offset as usize..(primitive.flat_triangle_offset + primitive.flat_triangle_count) as usize;
		Some(PreparedSurface {
			positions: bytemuck::cast_slice(stream(&self.streams.positions, 12)),
			normals: bytemuck::cast_slice(stream(&self.streams.normals, VERTEX_NORMAL_BUFFER_STRIDE as usize)),
			lightmap_uvs: bytemuck::cast_slice(stream(
				&self.streams.lightmap_uvs,
				VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize,
			)),
			triangles: bytemuck::cast_slice(&bytes[flat.start + triangles.start * 6..][..triangles.len() * 6]),
		})
	}
}

impl PreparedMesh {
	/// Rewrites the meshlet and level records of the prepared `elements` for the buffer elements `placed`, where the
	/// geometry buffers put those elements, so the records name buffer elements instead of prepared ones.
	///
	/// [`super::GeometryBuffers`] calls it once per placement, before it copies the records, for the whole mesh or for
	/// one primitive's block. Each record moves exactly once, so never pass a record's elements twice.
	pub(crate) fn place_records(&mut self, elements: &MeshRanges, placed: &MeshRanges) {
		let shift = |from: &Range<u32>, to: &Range<u32>| to.start.wrapping_sub(from.start);
		let (vertex_indices, triangles) = (
			shift(&elements.primitive_indices, &placed.primitive_indices),
			shift(&elements.triangles, &placed.triangles),
		);
		let (meshlets, vertices) = (shift(&elements.meshlets, &placed.meshlets), shift(&elements.vertices, &placed.vertices));
		let bytes = self.staging.bytes_mut();

		const MESHLET_STRIDE: usize = std::mem::size_of::<ShaderMeshletData>();
		let records = &mut bytes[self.streams.meshlets.clone()][elements.meshlets.start as usize * MESHLET_STRIDE..]
			[..elements.meshlets.len() * MESHLET_STRIDE];
		for record in records.as_chunks_mut::<MESHLET_STRIDE>().0 {
			// The lease keeps no alignment for the records, so each one is read and written as bytes.
			let mut meshlet: ShaderMeshletData = bytemuck::pod_read_unaligned(record);
			meshlet.primitive_offset = meshlet.primitive_offset.wrapping_add(vertex_indices);
			meshlet.triangle_offset = meshlet.triangle_offset.wrapping_add(triangles);
			record.copy_from_slice(bytemuck::bytes_of(&meshlet));
		}

		const LEVEL_STRIDE: usize = std::mem::size_of::<ShaderMeshLevel>();
		let records = &mut bytes[self.streams.levels.clone()][elements.levels.start as usize * LEVEL_STRIDE..]
			[..elements.levels.len() * LEVEL_STRIDE];
		for record in records.as_chunks_mut::<LEVEL_STRIDE>().0 {
			let mut level: ShaderMeshLevel = bytemuck::pod_read_unaligned(record);
			level.first_meshlet = level.first_meshlet.wrapping_add(meshlets);
			level.vertex_base = level.vertex_base.wrapping_add(vertices);
			record.copy_from_slice(bytemuck::bytes_of(&level));
		}
	}
}

async fn allocate_staging(upload_staging: &Arc<UploadStagingArena>, byte_count: usize) -> Option<StagingLease> {
	upload_staging.allocate(byte_count, STAGING_ALIGNMENT).await.or_else(|| {
		log::error!(
			"Prepared mesh exceeds the GPU upload arena. The most likely cause is that the mesh is larger than the configured upload capacity."
		);
		None
	})
}

/* Resource meshes */

/// The `PreparedLevels` enum chooses which detail levels of each primitive a prepared mesh holds.
///
/// Primitives have different level counts, so each one's coarsest level is its own last level, not a level number the
/// mesh shares. That keeps a primitive with few levels from pulling the other primitives' finer levels in with it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PreparedLevels<'a> {
	/// Each primitive's coarsest level, which every instance of the mesh can draw.
	Coarsest,
	/// Each primitive's full-detail level, with every vertex. The lightmap bake traces and rasterizes it.
	Finest,
	/// For each listed primitive, in ascending primitive order, its levels from [`PrimitiveLevels::finest`] down to,
	/// but not including, its coarsest. Nothing of an unlisted primitive.
	Finer(&'a [PrimitiveLevels]),
}

impl PreparedLevels<'_> {
	/// Returns the levels of primitive `primitive`, which has `level_count` levels, that this preparation holds.
	fn of(self, primitive: usize, level_count: usize) -> Range<usize> {
		let coarsest = level_count - 1;
		match self {
			Self::Coarsest => coarsest..level_count,
			Self::Finest => 0..1,
			Self::Finer(primitives) => match primitives.binary_search_by_key(&(primitive as u32), |levels| levels.primitive) {
				Ok(index) => (primitives[index].finest as usize).min(coarsest)..coarsest,
				Err(_) => coarsest..coarsest,
			},
		}
	}

	/// Returns whether level `level` of a primitive with `level_count` levels is the one whose triangles this
	/// preparation flattens for rays: the coarsest for [`Self::Coarsest`], full detail for [`Self::Finest`], and none
	/// for finer levels.
	fn traced(self, level: usize, level_count: usize) -> bool {
		match self {
			Self::Coarsest => level + 1 == level_count,
			Self::Finest => level == 0,
			Self::Finer(_) => false,
		}
	}
}

/// Vertex attribute streams the visibility format reads, in the order each level loads them.
///
/// Lightmap UVs are optional: a primitive without them reads zeros, and no lightmap gives it a rectangle.
const VERTEX_SEMANTICS: [VertexSemantics; 4] = [
	VertexSemantics::Position,
	VertexSemantics::Normal,
	VertexSemantics::UV,
	VertexSemantics::LightmapUV,
];
/// Meshlet streams the visibility format reads, in the order each level loads them.
const MESHLET_STREAMS: [Streams; 3] = [
	Streams::Indices(IndexStreamTypes::Vertices),
	Streams::Indices(IndexStreamTypes::Meshlets),
	Streams::Meshlets,
];

/// Spells out each level's stream names at compile time, so reads name their streams without formatting any.
macro_rules! level_stream_names {
	($($level:literal),*) => {
		[$([
			concat!("level[", $level, "].Vertex.Position"),
			concat!("level[", $level, "].Vertex.Normal"),
			concat!("level[", $level, "].Vertex.UV"),
			concat!("level[", $level, "].Vertex.LightmapUV"),
			concat!("level[", $level, "].VertexIndices"),
			concat!("level[", $level, "].MeshletIndices"),
			concat!("level[", $level, "].Meshlets"),
		]),*]
	};
}
/// Each level's position, normal, UV, lightmap UV, meshlet vertex-index, triangle-index, and record stream names, in the
/// order a [`ResourcePlan`] lays them out.
const LEVEL_STREAM_NAMES: [[&str; 7]; MAX_PRIMITIVE_LODS] = level_stream_names!(0, 1, 2, 3, 4, 5, 6, 7);

/// The `ResourcePlan` struct lays out one preparation of a baked mesh: which level streams load where in the front of
/// the staging lease, and where each runtime stream lands behind them.
///
/// It reads only the mesh's metadata, so every size is known before the payload loads.
struct ResourcePlan {
	/// Where each level's position, normal, UV, and lightmap UV streams load, indexed by level; empty where none loads.
	vertex_sources: [[Range<usize>; 4]; MAX_PRIMITIVE_LODS],
	/// Where each level's meshlet vertex-index, triangle-index, and record streams load, indexed by level.
	meshlet_sources: [[Range<usize>; 3]; MAX_PRIMITIVE_LODS],
	/// How many of the source ranges above are not empty, each one read target.
	source_count: usize,
	/// Where the level 0 joint and weight streams load, when a skinned primitive's full detail is held.
	skinning: Option<(Range<usize>, Range<usize>)>,
	uvs_are_f32: bool,
	source_size: usize,
	backing_size: usize,
	/// The runtime streams, as ranges of the whole staging lease.
	streams: PreparedStreams,
	counts: GeometryCounts,
}

impl ResourcePlan {
	/// Validates the streams of `mesh` that `levels` needs and lays out the preparation, with room for each primitive's
	/// coarsest triangles when `flat_triangles` is set.
	fn new(mesh: &Mesh, levels: PreparedLevels<'_>, flat_triangles: bool) -> Option<Self> {
		let uvs_are_f32 = match mesh
			.vertex_components
			.iter()
			.find(|component| component.semantic == VertexSemantics::UV && component.channel == 0)
			.map(|component| component.format.as_str())
		{
			Some("vec2f16") => false,
			Some("vec2f") => true,
			format => {
				log::error!(
					"Unsupported mesh UV format {format:?}. The most likely cause is that the asset uses a vertex format other than vec2f16 or vec2f."
				);
				return None;
			}
		};
		let uv_stride = if uvs_are_f32 { F32_UV_STRIDE } else { F16_UV_STRIDE };
		// Solving the mesh checked that no primitive has more than `MAX_PRIMITIVE_LODS` levels.
		let level_count = mesh.level_count() as usize;
		// Which levels' streams some primitive reads.
		let mut vertex_levels = [false; MAX_PRIMITIVE_LODS];
		let mut meshlet_levels = [false; MAX_PRIMITIVE_LODS];
		let mut counts = GeometryCounts::default();
		let mut skinned = false;
		let mut flat_triangle_bound = 0;
		for (index, primitive) in mesh.primitives.iter().enumerate() {
			let held = levels.of(index, primitive.lods.len());
			if held.is_empty() {
				continue;
			}
			// A primitive's vertices for a level are its vertices of that level and every coarser one.
			for level in held.start..primitive.lods.len() {
				vertex_levels[level] = true;
			}
			counts.vertices += primitive.lods[held.start].vertex_count;
			counts.levels += held.len() as u32;
			for level in held.clone() {
				meshlet_levels[level] = true;
				let count = |stream_type, name, stride| {
					primitive
						.stream(stream_type, level as u8)
						.map_or(Some(0), |stream| stream_count(stream, name, stride))
				};
				counts.primitive_indices += count(MESHLET_STREAMS[0], "meshlet vertex-index", VERTEX_INDEX_STRIDE)? as u32;
				let triangle_indices = count(MESHLET_STREAMS[1], "meshlet triangle-index", 1)?;
				if !triangle_indices.is_multiple_of(3) {
					log::error!(
						"Mesh primitive {index} detail level {level} does not contain complete triangles. The most likely cause is truncated baked meshlet index data."
					);
					return None;
				}
				counts.triangles += (triangle_indices / 3) as u32;
				counts.meshlets += count(MESHLET_STREAMS[2], "meshlet", RESOURCE_MESHLET_STRIDE)? as u32;
				if levels.traced(level, primitive.lods.len()) && primitive.skin.is_none() {
					flat_triangle_bound += triangle_indices / 3;
				}
			}
			if primitive.skin.is_some() {
				skinned = true;
				counts.skinning_vertices += primitive.vertex_count;
			}
		}

		// Every needed stream loads into its own 4-byte aligned range at the front of the lease.
		let mut cursor = 0;
		let mut source_count = 0;
		let mut source = |stream: Option<&Stream>, name: &str, stride: usize| -> Option<Range<usize>> {
			let Some(stream) = stream else {
				return Some(cursor..cursor);
			};
			stream_count(stream, name, stride)?;
			source_count += 1;
			Some(take_range(&mut cursor, stream.size))
		};
		let mut vertex_sources: [[Range<usize>; 4]; MAX_PRIMITIVE_LODS] = Default::default();
		let mut meshlet_sources: [[Range<usize>; 3]; MAX_PRIMITIVE_LODS] = Default::default();
		for level in 0..level_count {
			let vertex_stream = |index: usize| {
				vertex_levels[level]
					.then(|| mesh.stream(Streams::Vertices(VERTEX_SEMANTICS[index]), level as u8))
					.flatten()
			};
			vertex_sources[level] = [
				source(vertex_stream(0), "position", SKINNING_POSITION_STRIDE)?,
				source(vertex_stream(1), "normal", SKINNING_NORMAL_STRIDE)?,
				source(vertex_stream(2), "UV", uv_stride)?,
				source(vertex_stream(3), "lightmap UV", F32_UV_STRIDE)?,
			];
			let meshlet_stream = |index: usize| {
				meshlet_levels[level]
					.then(|| mesh.stream(MESHLET_STREAMS[index], level as u8))
					.flatten()
			};
			meshlet_sources[level] = [
				source(meshlet_stream(0), "meshlet vertex-index", VERTEX_INDEX_STRIDE)?,
				source(meshlet_stream(1), "meshlet triangle-index", 1)?,
				source(meshlet_stream(2), "meshlet", RESOURCE_MESHLET_STRIDE)?,
			];
		}
		let skinning = if skinned {
			let require = |semantic, name| {
				mesh.stream(Streams::Vertices(semantic), 0).or_else(|| {
					log::error!(
						"Mesh resource does not contain a full-detail {name} stream. The most likely cause is that the mesh was baked without the skin data its skinned primitives need."
					);
					None
				})
			};
			let joints = require(VertexSemantics::Joints, "joint-index")?;
			let weights = require(VertexSemantics::Weights, "vertex-weight")?;
			Some((
				source(Some(joints), "joint-index", SKINNING_JOINTS_STRIDE)?,
				source(Some(weights), "vertex-weight", SKINNING_WEIGHTS_STRIDE)?,
			))
		} else {
			None
		};
		let source_size = cursor;

		let vertices = counts.vertices as usize;
		let streams = PreparedStreams {
			positions: take_range(&mut cursor, vertices * SKINNING_POSITION_STRIDE),
			normals: take_range(&mut cursor, vertices * VERTEX_NORMAL_BUFFER_STRIDE as usize),
			uvs: take_range(&mut cursor, vertices * VERTEX_UV_BUFFER_STRIDE as usize),
			lightmap_uvs: take_range(&mut cursor, vertices * VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize),
			vertex_indices: take_range(&mut cursor, counts.primitive_indices as usize * VERTEX_INDEX_STRIDE),
			primitive_indices: take_range(&mut cursor, counts.triangles as usize * 3),
			meshlets: take_range(
				&mut cursor,
				counts.meshlets as usize * std::mem::size_of::<ShaderMeshletData>(),
			),
			levels: take_range(
				&mut cursor,
				counts.levels as usize * std::mem::size_of::<ShaderMeshLevel>(),
			),
			flat_triangles: (flat_triangles && !matches!(levels, PreparedLevels::Finer(_)))
				.then(|| take_range(&mut cursor, flat_triangle_bound * 6)),
		};
		Some(Self {
			vertex_sources,
			meshlet_sources,
			source_count,
			skinning,
			uvs_are_f32,
			source_size,
			backing_size: cursor,
			streams,
			counts,
		})
	}

	/// Splits `source`, the front of the staging lease, into one named read target per stream the plan loads.
	///
	/// The plan lays its sources out level by level, then the skinning streams, so walking them in that order meets
	/// their ranges in ascending order, which splitting needs.
	fn read_targets<'b>(&self, source: &'b mut [u8]) -> Vec<StreamMut<'b>> {
		let levels = self.vertex_sources.iter().zip(&self.meshlet_sources).zip(LEVEL_STREAM_NAMES);
		let level_sources =
			levels.flat_map(|((vertices, meshlets), names)| names.into_iter().zip(vertices.iter().chain(meshlets)));
		let skinning_sources = self
			.skinning
			.iter()
			.flat_map(|(joints, weights)| [("level[0].Vertex.Joints", joints), ("level[0].Vertex.Weights", weights)]);
		let mut targets = Vec::with_capacity(self.source_count);
		let mut allocator = utils::BufferAllocator::new(source);
		let mut cursor = 0;
		for (name, range) in level_sources.chain(skinning_sources).filter(|(_, range)| !range.is_empty()) {
			allocator.take(range.start - cursor);
			cursor = range.end;
			targets.push(StreamMut::new(name, allocator.take(range.len())));
		}
		targets
	}

	/// Gathers and converts every held level of every primitive from the loaded `source` into `output`, the lease
	/// after the source region, and returns the prepared primitives with the number of flat triangles written.
	///
	/// Each primitive's vertices follow one another, coarsest level first, the order its meshlets index them in.
	/// Meshlet and level records count from the start of the prepared streams, and each held level names its record,
	/// so [`PreparedMesh::place_records`] can move any primitive on its own.
	fn convert(
		&self,
		mesh: &Mesh,
		levels: PreparedLevels<'_>,
		source: &[u8],
		output: &mut [u8],
		skins: &[Arc<SkinBinding>],
	) -> Option<(Vec<PreparedPrimitive>, u32)> {
		let rebase = |range: &Range<usize>| range.start - self.source_size..range.end - self.source_size;
		let streams = &self.streams;
		let mut vertex = 0usize;
		let mut vertex_index = 0usize;
		let mut triangle = 0usize;
		let mut meshlet = 0usize;
		let mut level_record = 0usize;
		let mut flat = 0usize;
		let mut skinning_vertex = 0;
		let mut primitives = Vec::with_capacity(mesh.primitives.len());
		for (index, primitive) in mesh.primitives.iter().enumerate() {
			let held = levels.of(index, primitive.lods.len());
			let vertex_base = vertex as u32;
			let flat_start = flat as u32;
			let (primitive_vertex_index, primitive_triangle, primitive_meshlet, primitive_level) =
				(vertex_index, triangle, meshlet, level_record);
			let mut lods = [MeshLod::default(); MAX_PRIMITIVE_LODS];
			for (level, lod) in primitive.lods.iter().enumerate() {
				lods[level] = MeshLod {
					meshlet_count: lod.meshlet_count,
					error: lod.error,
					..MeshLod::default()
				};
			}
			let slice = |stream_type, level: usize, base: &Range<usize>| -> Option<Range<usize>> {
				let stream = primitive.stream(stream_type, level as u8)?;
				let start = base.start + stream.offset;
				let range = start..start + stream.size;
				(range.end <= base.end).then_some(range).or_else(|| {
					log::error!(
						"Mesh primitive {index} {stream_type:?} range at detail level {level} is out of bounds. The most likely cause is that its baked range does not refer to the level's stream."
					);
					None
				})
			};

			// Vertices, coarsest level first.
			if !held.is_empty() {
				for level in (held.start..primitive.lods.len()).rev() {
					let sources = &self.vertex_sources[level];
					let (Some(positions), Some(normals), Some(uvs)) = (
						slice(Streams::Vertices(VertexSemantics::Position), level, &sources[0]),
						slice(Streams::Vertices(VertexSemantics::Normal), level, &sources[1]),
						slice(Streams::Vertices(VertexSemantics::UV), level, &sources[2]),
					) else {
						// A level that adds no vertices has no slices.
						continue;
					};
					let count = positions.len() / SKINNING_POSITION_STRIDE;
					output[rebase(&streams.positions)][vertex * SKINNING_POSITION_STRIDE..][..positions.len()]
						.copy_from_slice(&source[positions]);
					for (destination, normal) in output[rebase(&streams.normals)].as_chunks_mut::<4>().0[vertex..][..count]
						.iter_mut()
						.zip(source[normals].as_chunks::<12>().0)
					{
						write_unit_vector(destination, (read_f32(normal, 0), read_f32(normal, 4), read_f32(normal, 8)));
					}
					let destination = &mut output[rebase(&streams.uvs)].as_chunks_mut::<F16_UV_STRIDE>().0[vertex..][..count];
					// Half floats keep sampler coordinates outside [0, 1] instead of clamping them.
					if self.uvs_are_f32 {
						for (destination, uv) in destination.iter_mut().zip(source[uvs].as_chunks::<F32_UV_STRIDE>().0) {
							write_f16_pair(destination, read_f32(uv, 0), read_f32(uv, 4));
						}
					} else {
						destination.as_flattened_mut().copy_from_slice(&source[uvs]);
					}
					let destination = &mut output[rebase(&streams.lightmap_uvs)]
						.as_chunks_mut::<{ VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize }>()
						.0[vertex..][..count];
					match slice(Streams::Vertices(VertexSemantics::LightmapUV), level, &sources[3]) {
						Some(lightmap_uvs) => {
							for (destination, uv) in destination
								.iter_mut()
								.zip(source[lightmap_uvs].as_chunks::<F32_UV_STRIDE>().0)
							{
								write_unorm16_pair(destination, read_f32(uv, 0), read_f32(uv, 4));
							}
						}
						// The lease is reused, so a primitive without lightmap UVs clears its share.
						None => destination.as_flattened_mut().fill(0),
					}
					vertex += count;
				}
				let expected = primitive.lods[held.start].vertex_count as usize;
				if vertex - vertex_base as usize != expected {
					log::error!(
						"Mesh primitive {index} holds {} vertices for detail level {}, not the {expected} its metadata names. The most likely cause is inconsistent baked vertex streams.",
						vertex - vertex_base as usize,
						held.start
					);
					return None;
				}
			}

			// Meshlets of every held level.
			for level in held.clone() {
				let sources = &self.meshlet_sources[level];
				let empty = |base: &Range<usize>| base.start..base.start;
				let vertex_indices = slice(MESHLET_STREAMS[0], level, &sources[0]).unwrap_or_else(|| empty(&sources[0]));
				let triangles = slice(MESHLET_STREAMS[1], level, &sources[1]).unwrap_or_else(|| empty(&sources[1]));
				let records = slice(MESHLET_STREAMS[2], level, &sources[2]).unwrap_or_else(|| empty(&sources[2]));
				let first_meshlet = meshlet;
				let vertex_index_start = vertex_index;
				let triangle_start = triangle;
				const RUNTIME_MESHLET_STRIDE: usize = std::mem::size_of::<ShaderMeshletData>();
				const LEVEL_STRIDE: usize = std::mem::size_of::<ShaderMeshLevel>();
				let resource_records = source[records.clone()].as_chunks::<RESOURCE_MESHLET_STRIDE>().0;
				// The level's sphere comes first, since each meshlet stores its sphere within it.
				let bounding_sphere = level_sphere(sphere_around(
					resource_records.iter().map(|bytes| read_vec4(bytes, RESOURCE_SPHERE_OFFSET)),
				));
				for bytes in resource_records {
					let record = read_resource_meshlet(bytes, vertex_index as u32, triangle as u32, bounding_sphere);
					vertex_index += record.vertex_count() as usize;
					triangle += record.triangle_count() as usize;
					output[rebase(&streams.meshlets)][meshlet * RUNTIME_MESHLET_STRIDE..][..RUNTIME_MESHLET_STRIDE]
						.copy_from_slice(bytemuck::bytes_of(&record));
					meshlet += 1;
				}
				if (vertex_index - vertex_index_start) * VERTEX_INDEX_STRIDE != vertex_indices.len()
					|| (triangle - triangle_start) * 3 != triangles.len()
					|| meshlet - first_meshlet != primitive.lods[level].meshlet_count as usize
				{
					log::error!(
						"Mesh primitive {index} detail level {level} meshlets do not cover its index streams. The most likely cause is inconsistent baked meshlet data."
					);
					return None;
				}
				output[rebase(&streams.vertex_indices)][vertex_index_start * VERTEX_INDEX_STRIDE..][..vertex_indices.len()]
					.copy_from_slice(&source[vertex_indices.clone()]);
				output[rebase(&streams.primitive_indices)][triangle_start * 3..][..triangles.len()]
					.copy_from_slice(&source[triangles.clone()]);
				let record = ShaderMeshLevel {
					bounding_sphere,
					first_meshlet: first_meshlet as u32,
					meshlet_count: (meshlet - first_meshlet) as u32,
					vertex_base,
					_padding: 0,
				};
				output[rebase(&streams.levels)][level_record * LEVEL_STRIDE..][..LEVEL_STRIDE]
					.copy_from_slice(bytemuck::bytes_of(&record));
				lods[level] = MeshLod {
					level: level_record as u32,
					vertex_base,
					resident: true,
					..lods[level]
				};
				level_record += 1;
				// Rays hit each unskinned primitive's coarsest level, and the lightmap bake's rays its full detail.
				if let Some(range) = &streams.flat_triangles
					&& levels.traced(level, primitive.lods.len())
					&& primitive.skin.is_none()
				{
					let written = flatten_meshlet_triangles(
						source[records].as_chunks::<RESOURCE_MESHLET_STRIDE>().0,
						source[vertex_indices].as_chunks::<VERTEX_INDEX_STRIDE>().0,
						source[triangles].as_chunks::<3>().0,
						&mut output[rebase(range)].as_chunks_mut::<6>().0[flat..],
					);
					flat += written;
				}
			}

			let skinning = match (primitive.skin, &self.skinning) {
				(Some(_), Some((joints, weights))) if !held.is_empty() => {
					let sources = &self.vertex_sources[0];
					Some(SkinningCopy {
						positions: slice(Streams::Vertices(VertexSemantics::Position), 0, &sources[0])?,
						normals: slice(Streams::Vertices(VertexSemantics::Normal), 0, &sources[1])?,
						joints: slice(Streams::Vertices(VertexSemantics::Joints), 0, joints)?,
						weights: slice(Streams::Vertices(VertexSemantics::Weights), 0, weights)?,
					})
				}
				_ => None,
			};
			let skinned = skinning.is_some();
			primitives.push(PreparedPrimitive {
				primitive: MeshPrimitive {
					material_index: 0,
					lods: MeshLods::new(&lods[..primitive.lods.len()]),
					bounding_sphere: primitive.bounding_sphere,
					vertex_offset: vertex_base,
					vertex_count: vertex as u32 - vertex_base,
					flat_triangle_offset: flat_start,
					flat_triangle_count: flat as u32 - flat_start,
					skinning_source_vertex_offset: skinning.as_ref().map(|_| skinning_vertex),
					skinning_vertex_count: skinning.as_ref().map_or(0, |_| primitive.vertex_count),
					skin: primitive.skin.map(|skin_index| skins[skin_index as usize].clone()),
				},
				elements: MeshRanges {
					vertices: vertex_base..vertex as u32,
					primitive_indices: primitive_vertex_index as u32..vertex_index as u32,
					triangles: primitive_triangle as u32..triangle as u32,
					meshlets: primitive_meshlet as u32..meshlet as u32,
					levels: primitive_level as u32..level_record as u32,
					..MeshRanges::default()
				},
				skinning,
			});
			if skinned {
				skinning_vertex += primitive.vertex_count;
			}
		}
		Some((primitives, flat as u32))
	}
}

/// Returns the element count of a stream after validating its stride.
fn stream_count(stream: &Stream, name: &str, expected_stride: usize) -> Option<usize> {
	if stream.stride != expected_stride || !stream.size.is_multiple_of(expected_stride) {
		log::error!(
			"Mesh {name} stream has an invalid layout. The most likely cause is incompatible baked stream metadata; expected stride {expected_stride}, found stride {} and size {}.",
			stream.stride,
			stream.size
		);
		return None;
	}
	Some(stream.size / expected_stride)
}

/// Writes the triangles of one primitive level's baked meshlet `records` to `flat`, each as three indices relative to
/// the primitive's first vertex, and returns how many it wrote.
///
/// Acceleration structures and hit shaders read this flat form. A meshlet triangle names its corners among its
/// meshlet's vertex indices, which name the primitive's vertices, so each corner takes both steps. `vertex_indices` and
/// `triangles` are the level's slices of the primitive's streams, which the records walk in order.
fn flatten_meshlet_triangles(
	records: &[[u8; RESOURCE_MESHLET_STRIDE]],
	vertex_indices: &[[u8; VERTEX_INDEX_STRIDE]],
	triangles: &[[u8; 3]],
	flat: &mut [[u8; 6]],
) -> usize {
	let (mut vertex_index, mut triangle, mut written) = (0, 0, 0);
	for record in records {
		let (vertex_count, triangle_count) = (record[0] as usize, record[1] as usize);
		let meshlet_vertices = &vertex_indices[vertex_index..][..vertex_count];
		for corners in &triangles[triangle..][..triangle_count] {
			let corners = corners.map(|corner| u16::from_le_bytes(meshlet_vertices[corner as usize]));
			flat[written] = bytemuck::cast(corners.map(u16::to_le));
			written += 1;
		}
		vertex_index += vertex_count;
		triangle += triangle_count;
	}
	written
}

/// Byte offsets of the bounding sphere, the cone apex and cutoff, and the cone axis in a baked meshlet record.
const RESOURCE_SPHERE_OFFSET: usize = 4;
const RESOURCE_CONE_CUTOFF_OFFSET: usize = 20;
const RESOURCE_CONE_AXIS_OFFSET: usize = 36;

/// Decodes one baked meshlet record into the runtime record whose indices start at `primitive_offset` and
/// `triangle_offset`, with its sphere stored within `level_sphere`, without assuming the resource stream is aligned.
fn read_resource_meshlet(bytes: &[u8], primitive_offset: u32, triangle_offset: u32, level_sphere: [f32; 4]) -> ShaderMeshletData {
	let [axis_x, axis_y, axis_z, _] = read_vec4(bytes, RESOURCE_CONE_AXIS_OFFSET);
	let [.., cutoff] = read_vec4(bytes, RESOURCE_CONE_CUTOFF_OFFSET);
	ShaderMeshletData::new(
		primitive_offset,
		triangle_offset,
		[bytes[0], bytes[1]],
		read_vec4(bytes, RESOURCE_SPHERE_OFFSET),
		level_sphere,
		[axis_x, axis_y, axis_z],
		cutoff,
	)
}

fn read_vec4(bytes: &[u8], offset: usize) -> [f32; 4] {
	std::array::from_fn(|component| read_f32(bytes, offset + 4 * component))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
	f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four-byte float"))
}

/* Generated meshes */

/// Greedily packs a generated triangle list into meshlets that respect the shader's vertex and triangle limits, and
/// returns them with the record of the one level they make.
fn build_generated_meshlets(
	indices: &[u16],
	positions: &[(f32, f32, f32)],
) -> (Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>, ShaderMeshLevel) {
	debug_assert!(
		indices.len().is_multiple_of(3),
		"Generated mesh indices are not a triangle list. The most likely cause is that validate_triangle_indices was skipped."
	);
	let mut vertex_indices = Vec::new();
	let mut primitive_indices = Vec::new();
	// Each meshlet's offsets, counts, and object-space sphere, encoded once the level's sphere is known.
	let mut meshlets = Vec::<([u32; 2], [u8; 2], [f32; 4])>::new();
	let mut meshlet_vertices = Vec::<u16>::new();
	let mut meshlet_triangles = Vec::<[u8; 3]>::new();
	let mut flush = |meshlet_vertices: &mut Vec<u16>, meshlet_triangles: &mut Vec<[u8; 3]>| {
		if meshlet_triangles.is_empty() {
			return;
		}
		meshlets.push((
			[vertex_indices.len() as u32, primitive_indices.len() as u32],
			[meshlet_vertices.len() as u8, meshlet_triangles.len() as u8],
			// A conservative object-space bounding sphere around the meshlet's vertices.
			sphere_around(meshlet_vertices.iter().map(|&index| {
				let (x, y, z) = positions[index as usize];
				[x, y, z, 0.0]
			})),
		));
		vertex_indices.append(meshlet_vertices);
		primitive_indices.append(meshlet_triangles);
	};

	for triangle in indices.as_chunks::<3>().0 {
		let new_vertices = triangle.iter().filter(|index| !meshlet_vertices.contains(index)).count();
		if meshlet_vertices.len() + new_vertices > VERTEX_COUNT as usize || meshlet_triangles.len() >= TRIANGLE_COUNT as usize {
			flush(&mut meshlet_vertices, &mut meshlet_triangles);
		}
		let mut local_triangle = [0u8; 3];
		for (slot, index) in triangle.iter().enumerate() {
			let local_index = meshlet_vertices.iter().position(|value| value == index).unwrap_or_else(|| {
				meshlet_vertices.push(*index);
				meshlet_vertices.len() - 1
			});
			local_triangle[slot] = local_index as u8;
		}
		meshlet_triangles.push(local_triangle);
	}
	flush(&mut meshlet_vertices, &mut meshlet_triangles);

	let bounding_sphere = level_sphere(sphere_around(meshlets.iter().map(|(_, _, sphere)| *sphere)));
	let level = ShaderMeshLevel {
		bounding_sphere,
		meshlet_count: meshlets.len() as u32,
		..ShaderMeshLevel::default()
	};
	// Generated meshes compute no normal cones, so a cutoff past one keeps every meshlet.
	let meshlets = meshlets
		.into_iter()
		.map(|([primitive_offset, triangle_offset], counts, sphere)| {
			ShaderMeshletData::new(primitive_offset, triangle_offset, counts, sphere, bounding_sphere, [0.0, 0.0, 1.0], 2.0)
		})
		.collect();
	(vertex_indices, primitive_indices, meshlets, level)
}

/// Returns a sphere, as xyz center and w radius, that contains every sphere in `spheres`. Points are spheres of radius
/// zero.
///
/// It centers on the box around the spheres, which keeps it close to the smallest one for the elongated shapes meshes
/// usually have. No spheres give a zero sphere.
fn sphere_around(spheres: impl Iterator<Item = [f32; 4]> + Clone) -> [f32; 4] {
	use maths_rs::{Vec3f, dist, max, min};

	let (mut low, mut high) = (Vec3f::from(f32::INFINITY), Vec3f::from(f32::NEG_INFINITY));
	for [x, y, z, radius] in spheres.clone() {
		low = min(low, Vec3f::new(x, y, z) - radius);
		high = max(high, Vec3f::new(x, y, z) + radius);
	}
	if low.x > high.x {
		return [0.0; 4];
	}
	let center = (low + high) * 0.5;
	let radius = spheres
		.map(|[x, y, z, radius]| dist(Vec3f::new(x, y, z), center) + radius)
		.fold(0.0f32, f32::max);
	[center.x, center.y, center.z, radius]
}

/* Attribute packing */

/// Octahedrally encodes one unit vector into two UNORM16 components.
pub(crate) fn encode_octahedral_unit_vector(vector: (f32, f32, f32)) -> RuntimeUnitVector {
	let length = vector.0.abs() + vector.1.abs() + vector.2.abs();
	if !length.is_finite() || length == 0.0 {
		return [32768, 32768];
	}
	let (x, y, z) = (vector.0 / length, vector.1 / length, vector.2 / length);
	let sign = |value: f32| if value < 0.0 { -1.0 } else { 1.0 };
	// The lower hemisphere folds into the square's outer triangles.
	let (x, y) = if z < 0.0 {
		((1.0 - y.abs()) * sign(x), (1.0 - x.abs()) * sign(y))
	} else {
		(x, y)
	};
	let unorm16 = |value: f32| ((value * 0.5 + 0.5).clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
	[unorm16(x), unorm16(y)]
}

/// Decodes a unit vector [`encode_octahedral_unit_vector`] encoded, as the shaders decode it.
pub(crate) fn decode_octahedral_unit_vector(encoded: RuntimeUnitVector) -> [f32; 3] {
	let snorm = |value: u16| value as f32 / u16::MAX as f32 * 2.0 - 1.0;
	let (x, y) = (snorm(encoded[0]), snorm(encoded[1]));
	let z = 1.0 - x.abs() - y.abs();
	// The square's outer triangles hold the lower hemisphere, folded.
	let fold = (-z).max(0.0);
	let x = x + if x >= 0.0 { -fold } else { fold };
	let y = y + if y >= 0.0 { -fold } else { fold };
	let length = (x * x + y * y + z * z).sqrt();
	[x / length, y / length, z / length]
}

fn write_unit_vector(destination: &mut [u8; 4], vector: (f32, f32, f32)) {
	*destination = bytemuck::cast(encode_octahedral_unit_vector(vector));
}

fn write_f16_pair(destination: &mut [u8; 4], u: f32, v: f32) {
	*destination = bytemuck::cast([half::f16::from_f32(u).to_bits(), half::f16::from_f32(v).to_bits()]);
}

/// Writes a lightmap UV pair as UNORM16, which shaders decode by dividing by 65,535.
fn write_unorm16_pair(destination: &mut [u8; 4], u: f32, v: f32) {
	let unorm16 = |value: f32| (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
	*destination = bytemuck::cast([unorm16(u), unorm16(v)]);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn flat_triangles_name_primitive_vertices_through_each_meshlet() {
		// Two baked meshlets: three vertices and one triangle, then three vertices and two triangles.
		let record = |vertex_count, triangle_count| {
			let mut record = [0u8; RESOURCE_MESHLET_STRIDE];
			record[0] = vertex_count;
			record[1] = triangle_count;
			record
		};
		let records = [record(3, 1), record(3, 2)];
		let vertex_indices = [5u16, 6, 7, 1, 2, 3].map(u16::to_le_bytes);
		let triangles = [[2, 0, 1], [0, 2, 1], [1, 1, 0]];
		let mut flat = [[0u8; 6]; 3];

		let written = flatten_meshlet_triangles(&records, &vertex_indices, &triangles, &mut flat);

		assert_eq!(written, 3);
		let flat = flat.map(|bytes| {
			std::array::from_fn::<u16, 3, _>(|corner| u16::from_le_bytes([bytes[corner * 2], bytes[corner * 2 + 1]]))
		});
		assert_eq!(flat, [[7, 5, 6], [1, 3, 2], [2, 2, 1]]);
	}

	#[test]
	fn octahedral_encoding_preserves_axes_and_folds_the_lower_hemisphere() {
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 1.0)), [32768, 32768]);
		assert_eq!(encode_octahedral_unit_vector((1.0, 0.0, 0.0)), [65535, 32768]);
		assert_eq!(encode_octahedral_unit_vector((0.0, -1.0, 0.0)), [32768, 0]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, -1.0)), [65535, 65535]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 0.0)), [32768, 32768]);
	}

	/// Verifies a level's sphere contains every meshlet's sphere, so culling the instance never hides a meshlet.
	#[test]
	fn sphere_around_contains_every_sphere() {
		let spheres = [[0.0, 0.0, 0.0, 1.0], [10.0, 0.0, 0.0, 0.5], [3.0, -4.0, 2.0, 2.0]];

		let [x, y, z, radius] = sphere_around(spheres.iter().copied());

		for [mx, my, mz, meshlet_radius] in spheres {
			let distance = ((mx - x).powi(2) + (my - y).powi(2) + (mz - z).powi(2)).sqrt();
			assert!(
				distance + meshlet_radius <= radius + 1e-5,
				"Sphere {:?} leaves the enclosing sphere {:?}.",
				[mx, my, mz, meshlet_radius],
				[x, y, z, radius]
			);
		}
		assert_eq!(sphere_around(std::iter::empty()), [0.0; 4]);
	}
}
