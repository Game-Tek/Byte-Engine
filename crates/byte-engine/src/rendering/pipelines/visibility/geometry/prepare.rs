//! Worker-side conversion of mesh sources into the exact bytes the visibility geometry buffers store.
//!
//! Preparation loads or generates a mesh into one leased region of the upload arena and converts attributes into
//! the runtime formats (octahedral normals, half-float UVs, [`ShaderMeshletData`] records). It never assigns
//! renderer slots or buffer offsets; that happens in [`GeometryBuffers::append_mesh`].

use std::ops::Range;
use std::sync::Arc;

use resource_management::Reference;
use resource_management::resources::mesh::Mesh;
use resource_management::resources::skeleton::SkinBinding;
use resource_management::stream::StreamMut;
use resource_management::types::{Stream, Streams, VertexSemantics};

use super::{
	GeometryCounts, MeshPrimitive, SKINNING_JOINTS_STRIDE, SKINNING_NORMAL_STRIDE, SKINNING_POSITION_STRIDE,
	SKINNING_WEIGHTS_STRIDE,
};
use crate::rendering::mesh::generator::{MeshGenerator, validate_triangle_indices};
use crate::rendering::pipelines::visibility::layout::{
	RuntimeUnitVector, ShaderMeshletData, TRIANGLE_COUNT, VERTEX_COUNT, VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE,
};
use crate::rendering::resource_loading::{StagingLease, UploadStagingArena};

/// Byte size of one baked meshlet record: two u8 counts, padding, and three packed vec4 bounds.
const RESOURCE_MESHLET_STRIDE: usize = 52;
const VERTEX_INDEX_STRIDE: usize = 2;
const F32_UV_STRIDE: usize = 8;
const F16_UV_STRIDE: usize = 4;
/// Upload arena alignment that satisfies every backend's buffer-copy requirement.
const STAGING_ALIGNMENT: usize = 256;
/// Material used by generated meshes.
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
	pub(super) vertex_indices: Range<usize>,
	pub(super) primitive_indices: Range<usize>,
	pub(super) meshlets: Range<usize>,
}

/// The `PreparedPrimitive` struct is one primitive's record plus the material it needs resolved on the render thread.
pub(crate) struct PreparedPrimitive {
	pub(crate) material_id: String,
	/// `material_index` and `skinning_source_vertex_offset` are finalized by [`GeometryBuffers::append_mesh`].
	pub(crate) primitive: MeshPrimitive,
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
	/// Builds transfer-ready geometry from a generated mesh.
	pub(crate) async fn generated(generator: &dyn MeshGenerator, upload_staging: Arc<UploadStagingArena>) -> Option<Self> {
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
		let (vertex_indices, primitive_indices, meshlets) = build_generated_meshlets(&indices, &positions);

		let mut cursor = 0;
		let streams = PreparedStreams {
			positions: take_range(&mut cursor, positions.len() * 12),
			normals: take_range(&mut cursor, normals.len() * VERTEX_NORMAL_BUFFER_STRIDE as usize),
			uvs: take_range(&mut cursor, uvs.len() * VERTEX_UV_BUFFER_STRIDE as usize),
			vertex_indices: take_range(&mut cursor, vertex_indices.len() * VERTEX_INDEX_STRIDE),
			primitive_indices: take_range(&mut cursor, primitive_indices.len() * 3),
			meshlets: take_range(&mut cursor, meshlets.len() * std::mem::size_of::<ShaderMeshletData>()),
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
		backing[streams.vertex_indices.clone()].copy_from_slice(bytemuck::cast_slice(&vertex_indices));
		backing[streams.primitive_indices.clone()].copy_from_slice(bytemuck::cast_slice(&primitive_indices));
		backing[streams.meshlets.clone()].copy_from_slice(bytemuck::cast_slice(&meshlets));

		Some(Self {
			staging,
			streams,
			primitives: vec![PreparedPrimitive {
				material_id: GENERATED_MESH_MATERIAL.to_string(),
				primitive: MeshPrimitive {
					meshlet_count: meshlets.len() as u32,
					bounding_sphere: enclosing_sphere(&meshlets),
					..MeshPrimitive::default()
				},
				skinning: None,
			}],
			counts: GeometryCounts {
				vertices: positions.len() as u32,
				primitive_indices: vertex_indices.len() as u32,
				triangles: primitive_indices.len() as u32,
				meshlets: meshlets.len() as u32,
				skinning_vertices: 0,
			},
			skeleton_node_count: 0,
		})
	}

	/// Loads a baked mesh resource and converts it into the runtime geometry formats.
	pub(crate) async fn resource(mut resource: Reference<Mesh>, upload_staging: Arc<UploadStagingArena>) -> Option<Self> {
		let mesh = resource.resource();
		let source = ResourceStreams::new(mesh)?;
		let layout = source.layout(mesh)?;
		let skeleton_node_count = mesh
			.skeleton
			.as_ref()
			.map_or(0, |skeleton| skeleton.resource().nodes.len() as u32);
		// The bindings move out of the resource, which this function drops: nothing below reads its skins.
		let skins = std::mem::take(&mut resource.resource_mut().skins)
			.into_iter()
			.map(Arc::new)
			.collect::<Vec<_>>();
		let mut staging = allocate_staging(&upload_staging, layout.backing_size).await?;
		let backing = staging.bytes_mut();
		let (source_bytes, output) = backing.split_at_mut(layout.source.byte_count);

		let loaded = resource
			.load(source.read_targets(source_bytes).into())
			.await
			.ok()
			.or_else(|| {
				log::error!(
					"Mesh resource streams could not be loaded. The most likely cause is that the baked mesh payload is missing or unreadable."
				);
				None
			})?;
		let meshlet_bytes = loaded.stream("Meshlets").expect("requested meshlet stream").buffer();
		let (primitives, meshlets) =
			build_resource_primitives(resource.resource(), meshlet_bytes, &skins, layout.counts, &layout.source)?;

		// Converted streams live after every loaded source stream, so their ranges are rebased into `output`.
		let rebase = |range: &Range<usize>| range.start - layout.source.byte_count..range.end - layout.source.byte_count;
		for (destination, source) in output[rebase(&layout.streams.normals)]
			.as_chunks_mut::<4>()
			.0
			.iter_mut()
			.zip(source_bytes[layout.source.normals.clone()].as_chunks::<12>().0.iter())
		{
			write_unit_vector(destination, (read_f32(source, 0), read_f32(source, 4), read_f32(source, 8)));
		}
		// Half floats keep sampler coordinates outside [0, 1] instead of clamping them.
		if layout.uvs_are_f32 {
			for (destination, source) in output[rebase(&layout.streams.uvs)]
				.as_chunks_mut::<F16_UV_STRIDE>()
				.0
				.iter_mut()
				.zip(source_bytes[layout.source.uvs.clone()].as_chunks::<F32_UV_STRIDE>().0)
			{
				write_f16_pair(destination, read_f32(source, 0), read_f32(source, 4));
			}
		}
		output[rebase(&layout.streams.meshlets)].copy_from_slice(bytemuck::cast_slice(&meshlets));

		Some(Self {
			staging,
			streams: layout.streams,
			primitives,
			counts: layout.counts,
			skeleton_node_count,
		})
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

/// The `ResourceStreams` struct is the set of aggregate baked streams the visibility format needs.
struct ResourceStreams {
	positions: Stream,
	normals: Stream,
	uvs: Stream,
	vertex_indices: Stream,
	meshlet_indices: Stream,
	meshlets: Stream,
	/// Present when at least one primitive is skinned.
	skinning: Option<(Stream, Stream)>,
}

/// The `ResourceLayout` struct places loaded source streams and converted runtime streams in one staging lease.
struct ResourceLayout {
	streams: PreparedStreams,
	/// Where every source stream was loaded; conversions and skinned copies read from these ranges.
	source: SourceStagingLayout,
	uvs_are_f32: bool,
	backing_size: usize,
	counts: GeometryCounts,
}

/// The `SourceStagingLayout` struct places every loaded source stream in the front of a staging lease.
///
/// [`ResourceStreams::read_targets`] loads the streams in this order, so a copy that reads a source stream
/// addresses it through these ranges rather than through the baked resource's own offsets. A baked primitive records
/// its stream offsets relative to its semantic's aggregate stream, while the copies in
/// [`GeometryBuffers::append_mesh`] address the staging lease, so these ranges convert between the two.
struct SourceStagingLayout {
	positions: Range<usize>,
	normals: Range<usize>,
	uvs: Range<usize>,
	vertex_indices: Range<usize>,
	primitive_indices: Range<usize>,
	/// Joint indices and weights, present when at least one primitive is skinned.
	skinning: Option<(Range<usize>, Range<usize>)>,
	byte_count: usize,
}

impl ResourceStreams {
	/// Reserves the source region in the order [`Self::read_targets`] loads it.
	fn source_staging_layout(&self) -> SourceStagingLayout {
		let mut cursor = 0;
		let positions = take_range(&mut cursor, self.positions.size);
		let normals = take_range(&mut cursor, self.normals.size);
		let uvs = take_range(&mut cursor, self.uvs.size);
		let vertex_indices = take_range(&mut cursor, self.vertex_indices.size);
		let primitive_indices = take_range(&mut cursor, self.meshlet_indices.size);
		// Source meshlets are only read on the CPU; they need no copy alignment.
		cursor += self.meshlets.size;
		let skinning = self.skinning.as_ref().map(|(joints, weights)| {
			let joints = take_range(&mut cursor, joints.size);
			let weights = take_range(&mut cursor, weights.size);
			(joints, weights)
		});

		SourceStagingLayout {
			positions,
			normals,
			uvs,
			vertex_indices,
			primitive_indices,
			skinning,
			byte_count: cursor,
		}
	}

	fn new(mesh: &Mesh) -> Option<Self> {
		let require = |stream: Option<Stream>, name: &str| {
			stream.or_else(|| {
				log::error!(
					"Mesh resource does not contain a {name} stream. The most likely cause is that the mesh was baked without the geometry the visibility pipeline needs."
				);
				None
			})
		};
		let skinned = mesh.primitives.iter().any(|primitive| primitive.skin.is_some());
		let skinning = if skinned {
			Some((
				require(mesh.vertex_stream(VertexSemantics::Joints).cloned(), "joint-index")?,
				require(mesh.vertex_stream(VertexSemantics::Weights).cloned(), "vertex-weight")?,
			))
		} else {
			None
		};
		Some(Self {
			positions: require(mesh.position_stream(), "vertex position")?,
			normals: require(mesh.normal_stream(), "vertex normal")?,
			uvs: require(mesh.uv_stream(), "vertex UV")?,
			vertex_indices: require(mesh.vertex_indices_stream(), "vertex index")?,
			meshlet_indices: require(mesh.meshlet_indices_stream(), "meshlet index")?,
			meshlets: require(mesh.meshlets_stream(), "meshlet")?,
			skinning,
		})
	}

	/// Validates stream strides and computes where every stream lands in the staging lease.
	fn layout(&self, mesh: &Mesh) -> Option<ResourceLayout> {
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
		let vertex_count = stream_count(&self.positions, "position", SKINNING_POSITION_STRIDE)?;
		if stream_count(&self.normals, "normal", SKINNING_NORMAL_STRIDE)? != vertex_count
			|| stream_count(&self.uvs, "UV", if uvs_are_f32 { F32_UV_STRIDE } else { F16_UV_STRIDE })? != vertex_count
		{
			log::error!(
				"Mesh attribute counts do not match the position count. The most likely cause is malformed vertex stream metadata."
			);
			return None;
		}
		let primitive_index_count = stream_count(&self.vertex_indices, "meshlet vertex-index", VERTEX_INDEX_STRIDE)?;
		let meshlet_index_count = stream_count(&self.meshlet_indices, "meshlet triangle-index", 1)?;
		if !meshlet_index_count.is_multiple_of(3) {
			log::error!(
				"Meshlet triangle-index stream does not contain complete triangles. The most likely cause is truncated baked meshlet index data."
			);
			return None;
		}
		let meshlet_count = stream_count(&self.meshlets, "meshlet", RESOURCE_MESHLET_STRIDE)?;
		let skinning_vertices = self.validate_skinning(mesh)?;

		let source = self.source_staging_layout();
		let mut cursor = source.byte_count;
		let normals = take_range(&mut cursor, vertex_count * VERTEX_NORMAL_BUFFER_STRIDE as usize);
		let uvs = if uvs_are_f32 {
			take_range(&mut cursor, vertex_count * VERTEX_UV_BUFFER_STRIDE as usize)
		} else {
			source.uvs.clone()
		};
		let meshlets = take_range(&mut cursor, meshlet_count * std::mem::size_of::<ShaderMeshletData>());

		Some(ResourceLayout {
			streams: PreparedStreams {
				positions: source.positions.clone(),
				normals,
				uvs,
				vertex_indices: source.vertex_indices.clone(),
				primitive_indices: source.primitive_indices.clone(),
				meshlets,
			},
			source,
			uvs_are_f32,
			backing_size: cursor,
			counts: GeometryCounts {
				vertices: vertex_count as u32,
				primitive_indices: primitive_index_count as u32,
				triangles: (meshlet_index_count / 3) as u32,
				meshlets: meshlet_count as u32,
				skinning_vertices: skinning_vertices as u32,
			},
		})
	}

	/// Checks every skinned primitive's streams against the aggregate skin streams and returns the skinned vertex total.
	fn validate_skinning(&self, mesh: &Mesh) -> Option<usize> {
		let Some((joints, weights)) = &self.skinning else {
			return Some(0);
		};
		let aggregates = [
			(&self.positions, VertexSemantics::Position, SKINNING_POSITION_STRIDE),
			(&self.normals, VertexSemantics::Normal, SKINNING_NORMAL_STRIDE),
			(joints, VertexSemantics::Joints, SKINNING_JOINTS_STRIDE),
			(weights, VertexSemantics::Weights, SKINNING_WEIGHTS_STRIDE),
		];
		let mut vertex_count = 0;
		for (index, primitive) in mesh.primitives.iter().enumerate() {
			let Some(skin_index) = primitive.skin else {
				continue;
			};
			if skin_index as usize >= mesh.skins.len() {
				log::error!(
					"Skinned primitive {index} references a missing skin binding. The most likely cause is corrupted primitive metadata."
				);
				return None;
			}
			for (aggregate, semantic, stride) in aggregates {
				let Some(stream) = primitive.stream(Streams::Vertices(semantic)) else {
					log::error!(
						"Skinned primitive {index} is missing its {semantic:?} stream. The most likely cause is that the mesh was baked without complete per-primitive skinning metadata."
					);
					return None;
				};
				let expected_size = primitive.vertex_count as usize * stride;
				if aggregate.stride != stride
					|| stream.stride != stride
					|| stream.size != expected_size
					|| !stream.offset.is_multiple_of(stride)
					|| stream.offset + stream.size > aggregate.size
				{
					log::error!(
						"Skinned primitive {index} has an invalid {semantic:?} stream. The most likely cause is that its offset, stride, or size does not match its {} vertices inside the baked aggregate stream.",
						primitive.vertex_count
					);
					return None;
				}
			}
			vertex_count += primitive.vertex_count as usize;
		}
		Some(vertex_count)
	}

	/// Builds the named read targets that load every source stream into the front of the staging lease.
	fn read_targets<'b>(&self, backing: &'b mut [u8]) -> Vec<StreamMut<'b>> {
		let mut allocator = utils::BufferAllocator::new(backing);
		let mut streams = Vec::with_capacity(8);
		for (name, size) in [
			("Vertex.Position", self.positions.size),
			("Vertex.Normal", self.normals.size),
			("Vertex.UV", self.uvs.size),
			("VertexIndices", self.vertex_indices.size),
			("MeshletIndices", self.meshlet_indices.size),
		] {
			streams.push(StreamMut::new(name, allocator.take_with_offset_aligned(size, 4).1));
		}
		streams.push(StreamMut::new("Meshlets", allocator.take(self.meshlets.size)));
		if let Some((joints, weights)) = &self.skinning {
			for (name, size) in [("Vertex.Joints", joints.size), ("Vertex.Weights", weights.size)] {
				streams.push(StreamMut::new(name, allocator.take_with_offset_aligned(size, 4).1));
			}
		}
		streams
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

/// Converts baked primitive metadata and meshlet records into per-primitive ranges and runtime meshlets.
fn build_resource_primitives(
	mesh: &Mesh,
	meshlet_bytes: &[u8],
	skins: &[Arc<SkinBinding>],
	expected: GeometryCounts,
	source: &SourceStagingLayout,
) -> Option<(Vec<PreparedPrimitive>, Vec<ShaderMeshletData>)> {
	let mut primitives = Vec::with_capacity(mesh.primitives.len());
	let mut meshlets = Vec::with_capacity(expected.meshlets as usize);
	let mut counts = GeometryCounts::default();

	for (index, primitive) in mesh.primitives.iter().enumerate() {
		let Some(meshlet_stream) = primitive.meshlet_stream() else {
			log::error!(
				"Mesh primitive {index} is missing its meshlet stream. The most likely cause is incomplete baked primitive metadata."
			);
			return None;
		};
		stream_count(meshlet_stream, "primitive meshlet", RESOURCE_MESHLET_STRIDE)?;
		let Some(meshlet_source) = meshlet_bytes.get(meshlet_stream.offset..meshlet_stream.offset + meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {index} meshlet range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate meshlet stream."
			);
			return None;
		};
		let meshlet_offset = meshlets.len() as u32;
		let mut local_primitive_offset = 0;
		let mut local_triangle_offset = 0;
		for bytes in meshlet_source.as_chunks::<RESOURCE_MESHLET_STRIDE>().0 {
			let meshlet = read_resource_meshlet(bytes, local_primitive_offset, local_triangle_offset);
			local_primitive_offset += meshlet.primitive_count;
			local_triangle_offset += meshlet.triangle_count;
			meshlets.push(meshlet);
		}

		let skinning = primitive.skin.map(|_| {
			let (joints, weights) = source
				.skinning
				.as_ref()
				.expect("a skinned primitive requires skinning staging ranges");
			// A baked stream offset is relative to its semantic's aggregate stream, so rebase it onto
			// where that stream was loaded in the staging lease. Without the rebase every primitive
			// would copy from the front of the lease, which holds the position stream.
			let range = |semantic, base: &Range<usize>| {
				// Stream presence and bounds were validated by `ResourceStreams::validate_skinning`.
				let stream = primitive
					.stream(Streams::Vertices(semantic))
					.expect("validated skinning stream");
				let range = base.start + stream.offset..base.start + stream.offset + stream.size;
				debug_assert!(
					range.start >= base.start && range.end <= base.end,
					"Skinned primitive {semantic:?} copy leaves its staging stream. The most likely cause is a baked offset used without rebasing it onto the staging lease."
				);
				range
			};
			SkinningCopy {
				positions: range(VertexSemantics::Position, &source.positions),
				normals: range(VertexSemantics::Normal, &source.normals),
				joints: range(VertexSemantics::Joints, joints),
				weights: range(VertexSemantics::Weights, weights),
			}
		});
		primitives.push(PreparedPrimitive {
			material_id: mesh.material(primitive).id().as_ref().to_string(),
			primitive: MeshPrimitive {
				material_index: 0,
				meshlet_count: (meshlet_source.len() / RESOURCE_MESHLET_STRIDE) as u32,
				bounding_sphere: enclosing_sphere(&meshlets[meshlet_offset as usize..]),
				meshlet_offset,
				vertex_offset: counts.vertices,
				primitive_offset: counts.primitive_indices,
				triangle_offset: counts.triangles,
				skinning_source_vertex_offset: primitive.skin.map(|_| counts.skinning_vertices),
				skinning_vertex_count: primitive.skin.map_or(0, |_| primitive.vertex_count),
				skin: primitive.skin.map(|skin_index| skins[skin_index as usize].clone()),
			},
			skinning,
		});
		counts.vertices += primitive.vertex_count;
		counts.primitive_indices += local_primitive_offset;
		counts.triangles += local_triangle_offset;
		if primitive.skin.is_some() {
			counts.skinning_vertices += primitive.vertex_count;
		}
	}
	counts.meshlets = meshlets.len() as u32;

	if counts != expected {
		log::error!(
			"Prepared primitive counts do not match the aggregate mesh streams: expected {expected:?}, found {counts:?}. The most likely cause is inconsistent or overlapping baked primitive ranges."
		);
		return None;
	}
	Some((primitives, meshlets))
}

/// Decodes one packed meshlet record into the runtime record at the given mesh-relative offsets, without assuming the
/// resource stream is aligned.
fn read_resource_meshlet(bytes: &[u8], primitive_offset: u32, triangle_offset: u32) -> ShaderMeshletData {
	let read_vec4 = |offset: usize| -> [f32; 4] { std::array::from_fn(|component| read_f32(bytes, offset + 4 * component)) };
	let [axis_x, axis_y, axis_z, _] = read_vec4(36);
	ShaderMeshletData {
		primitive_offset,
		triangle_offset,
		primitive_count: bytes[0] as u32,
		triangle_count: bytes[1] as u32,
		center_radius: read_vec4(4),
		cone_apex_cutoff: read_vec4(20),
		cone_axis: encode_octahedral_unit_vector((axis_x, axis_y, axis_z)),
	}
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
	f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four-byte float"))
}

/* Generated meshes */

/// Greedily packs a generated triangle list into meshlets that respect the shader's vertex and triangle limits.
fn build_generated_meshlets(
	indices: &[u16],
	positions: &[(f32, f32, f32)],
) -> (Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>) {
	debug_assert!(
		indices.len().is_multiple_of(3),
		"Generated mesh indices are not a triangle list. The most likely cause is that validate_triangle_indices was skipped."
	);
	let mut vertex_indices = Vec::new();
	let mut primitive_indices = Vec::new();
	let mut meshlets = Vec::new();
	let mut meshlet_vertices = Vec::<u16>::new();
	let mut meshlet_triangles = Vec::<[u8; 3]>::new();
	let mut flush = |meshlet_vertices: &mut Vec<u16>, meshlet_triangles: &mut Vec<[u8; 3]>| {
		if meshlet_triangles.is_empty() {
			return;
		}
		meshlets.push(ShaderMeshletData {
			primitive_offset: vertex_indices.len() as u32,
			triangle_offset: primitive_indices.len() as u32,
			primitive_count: meshlet_vertices.len() as u32,
			triangle_count: meshlet_triangles.len() as u32,
			// A conservative object-space bounding sphere around the meshlet's vertices.
			center_radius: sphere_around(meshlet_vertices.iter().map(|&index| {
				let (x, y, z) = positions[index as usize];
				[x, y, z, 0.0]
			})),
			cone_apex_cutoff: [0.0, 0.0, 0.0, 2.0],
			cone_axis: encode_octahedral_unit_vector((0.0, 0.0, 1.0)),
		});
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
	(vertex_indices, primitive_indices, meshlets)
}

/// Returns a sphere that contains every meshlet's bounding sphere, as xyz center and w radius.
pub(crate) fn enclosing_sphere(meshlets: &[ShaderMeshletData]) -> [f32; 4] {
	sphere_around(meshlets.iter().map(|meshlet| meshlet.center_radius))
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

fn write_unit_vector(destination: &mut [u8; 4], vector: (f32, f32, f32)) {
	*destination = bytemuck::cast(encode_octahedral_unit_vector(vector));
}

fn write_f16_pair(destination: &mut [u8; 4], u: f32, v: f32) {
	*destination = bytemuck::cast([half::f16::from_f32(u).to_bits(), half::f16::from_f32(v).to_bits()]);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Verifies each skinned source stream is copied from its own staging range.
	///
	/// A baked primitive records stream offsets relative to its semantic's aggregate stream, while the copies in
	/// `append_mesh` address the staging lease. Using a baked offset directly aliased every skinned stream onto the
	/// front of the lease, so joints and weights read position bytes and the mesh rendered in its bind pose.
	#[test]
	fn skinned_source_streams_occupy_distinct_staging_ranges() {
		let stream = |stream_type, size, stride| Stream {
			stream_type,
			offset: 0,
			size,
			stride,
		};
		let vertices = 8;
		let streams = ResourceStreams {
			positions: stream(Streams::Vertices(VertexSemantics::Position), vertices * 12, 12),
			normals: stream(Streams::Vertices(VertexSemantics::Normal), vertices * 12, 12),
			uvs: stream(Streams::Vertices(VertexSemantics::UV), vertices * 8, 8),
			vertex_indices: stream(
				Streams::Indices(resource_management::types::IndexStreamTypes::Vertices),
				vertices * 2,
				2,
			),
			meshlet_indices: stream(
				Streams::Indices(resource_management::types::IndexStreamTypes::Triangles),
				3,
				1,
			),
			meshlets: stream(Streams::Meshlets, RESOURCE_MESHLET_STRIDE, RESOURCE_MESHLET_STRIDE),
			skinning: Some((
				stream(Streams::Vertices(VertexSemantics::Joints), vertices * 8, 8),
				stream(Streams::Vertices(VertexSemantics::Weights), vertices * 16, 16),
			)),
		};

		let layout = streams.source_staging_layout();
		let (joints, weights) = layout.skinning.expect("a skinned mesh reserves joint and weight staging");

		// Every skinned source must have its own bytes; aliasing is what produced the bind-pose regression.
		let ranges = [&layout.positions, &layout.normals, &joints, &weights];
		for (index, first) in ranges.iter().enumerate() {
			assert!(!first.is_empty(), "skinned source stream {index} reserved no staging bytes");
			for second in &ranges[index + 1..] {
				assert!(
					first.end <= second.start || second.end <= first.start,
					"skinned source streams overlap in the staging lease: {first:?} and {second:?}"
				);
			}
		}
		assert_eq!(joints.len(), vertices * 8);
		assert_eq!(weights.len(), vertices * 16);
	}

	#[test]
	fn octahedral_encoding_preserves_axes_and_folds_the_lower_hemisphere() {
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 1.0)), [32768, 32768]);
		assert_eq!(encode_octahedral_unit_vector((1.0, 0.0, 0.0)), [65535, 32768]);
		assert_eq!(encode_octahedral_unit_vector((0.0, -1.0, 0.0)), [32768, 0]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, -1.0)), [65535, 65535]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 0.0)), [32768, 32768]);
	}

	/// Verifies a primitive's sphere contains every meshlet's sphere, so culling the instance never hides a meshlet.
	#[test]
	fn enclosing_sphere_contains_every_meshlet_sphere() {
		let meshlet = |center_radius| ShaderMeshletData {
			center_radius,
			..bytemuck::Zeroable::zeroed()
		};
		let meshlets = [
			meshlet([0.0, 0.0, 0.0, 1.0]),
			meshlet([10.0, 0.0, 0.0, 0.5]),
			meshlet([3.0, -4.0, 2.0, 2.0]),
		];

		let [x, y, z, radius] = enclosing_sphere(&meshlets);

		for meshlet in &meshlets {
			let [mx, my, mz, meshlet_radius] = meshlet.center_radius;
			let distance = ((mx - x).powi(2) + (my - y).powi(2) + (mz - z).powi(2)).sqrt();
			assert!(
				distance + meshlet_radius <= radius + 1e-5,
				"Meshlet sphere {:?} leaves the primitive sphere {:?}.",
				meshlet.center_radius,
				[x, y, z, radius]
			);
		}
		assert_eq!(enclosing_sphere(&[]), [0.0; 4]);
	}
}
