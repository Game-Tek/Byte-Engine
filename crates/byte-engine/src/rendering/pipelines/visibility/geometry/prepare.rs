//! Worker-side conversion of mesh sources into the exact bytes the visibility geometry buffers store.
//!
//! Preparation loads or generates a mesh into one leased region of the upload arena and converts attributes into
//! the runtime formats (octahedral normals, half-float UVs, [`ShaderMeshletData`] records). It never assigns
//! renderer slots or buffer offsets; that happens in [`GeometryBuffers::write_mesh`].

use std::ops::Range;
use std::sync::Arc;

use resource_management::Reference;
use resource_management::resources::mesh::Mesh;
use resource_management::resources::skeleton::SkinBinding;
use resource_management::types::{Stream, Streams, VertexSemantics};
use utils::as_byte_slice;

use super::{
	GeometryCounts, MeshPrimitive, SKINNING_JOINTS_STRIDE, SKINNING_NORMAL_STRIDE, SKINNING_POSITION_STRIDE,
	SKINNING_WEIGHTS_STRIDE,
};
use crate::rendering::mesh::generator::MeshGenerator;
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
const GENERATED_MESH_MATERIAL: &str = "white_solid.bema";

/// The `PreparedMesh` struct retains a mesh's converted geometry in its staging lease until the transfer frame completes.
pub(crate) struct PreparedMesh {
	pub(super) staging: StagingLease,
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
	/// `material_index` and `skinning_source_vertex_offset` are finalized by [`GeometryBuffers::write_mesh`].
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
		let indices = validated_generated_indices(&generator.indices(), positions.len())?;
		let (vertex_indices, primitive_indices, meshlets) = build_generated_meshlets(&indices, &positions)?;

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
		backing[streams.positions.clone()].copy_from_slice(as_byte_slice(&positions));
		for (destination, normal) in backing[streams.normals.clone()].chunks_exact_mut(4).zip(normals.iter()) {
			write_unit_vector(destination, *normal);
		}
		for (destination, (u, v)) in backing[streams.uvs.clone()].chunks_exact_mut(4).zip(uvs.iter()) {
			write_f16_pair(destination, *u, *v);
		}
		backing[streams.vertex_indices.clone()].copy_from_slice(as_byte_slice(&vertex_indices));
		backing[streams.primitive_indices.clone()].copy_from_slice(as_byte_slice(&primitive_indices));
		backing[streams.meshlets.clone()].copy_from_slice(as_byte_slice(&meshlets));

		Some(Self {
			staging,
			streams,
			primitives: vec![PreparedPrimitive {
				material_id: GENERATED_MESH_MATERIAL.to_string(),
				primitive: MeshPrimitive {
					material_index: 0,
					meshlet_count: meshlets.len() as u32,
					meshlet_offset: 0,
					vertex_offset: 0,
					primitive_offset: 0,
					triangle_offset: 0,
					skinning_source_vertex_offset: None,
					skinning_vertex_count: 0,
					skin: None,
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
		let skins = mesh.skins.iter().cloned().map(Arc::new).collect::<Vec<_>>();
		let skeleton_node_count = mesh
			.skeleton
			.as_ref()
			.map_or(0, |skeleton| skeleton.resource().nodes.len() as u32);
		let mut staging = allocate_staging(&upload_staging, layout.backing_size).await?;
		let backing = staging.bytes_mut();
		let (source_bytes, output) = backing.split_at_mut(layout.source_byte_count);

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
		let (primitives, meshlets) = build_resource_primitives(resource.resource(), meshlet_bytes, &skins, layout.counts)?;

		// Converted streams live after every loaded source stream, so their ranges are rebased into `output`.
		let rebase = |range: &Range<usize>| range.start - layout.source_byte_count..range.end - layout.source_byte_count;
		let vertex_count = layout.counts.vertices as usize;
		for (destination, source) in output[rebase(&layout.streams.normals)]
			.chunks_exact_mut(4)
			.zip(source_bytes[layout.source_normals.clone()].chunks_exact(12))
		{
			write_unit_vector(destination, (read_f32(source, 0), read_f32(source, 4), read_f32(source, 8)));
		}
		if layout.uvs_are_f32 {
			pack_f32_uvs(
				&source_bytes[layout.source_uvs.clone()],
				&mut output[rebase(&layout.streams.uvs)],
				vertex_count,
			);
		}
		output[rebase(&layout.streams.meshlets)].copy_from_slice(as_byte_slice(&meshlets));

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
	source_normals: Range<usize>,
	source_uvs: Range<usize>,
	uvs_are_f32: bool,
	source_byte_count: usize,
	backing_size: usize,
	counts: GeometryCounts,
}

impl ResourceStreams {
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

		let mut cursor = 0;
		let positions = take_range(&mut cursor, self.positions.size);
		let source_normals = take_range(&mut cursor, self.normals.size);
		let source_uvs = take_range(&mut cursor, self.uvs.size);
		let vertex_indices = take_range(&mut cursor, self.vertex_indices.size);
		let primitive_indices = take_range(&mut cursor, self.meshlet_indices.size);
		// Source meshlets are only read on the CPU; they need no copy alignment.
		cursor += self.meshlets.size;
		if let Some((joints, weights)) = &self.skinning {
			take_range(&mut cursor, joints.size);
			take_range(&mut cursor, weights.size);
		}
		let source_byte_count = cursor;
		let normals = take_range(&mut cursor, vertex_count * VERTEX_NORMAL_BUFFER_STRIDE as usize);
		let uvs = if uvs_are_f32 {
			take_range(&mut cursor, vertex_count * VERTEX_UV_BUFFER_STRIDE as usize)
		} else {
			source_uvs.clone()
		};
		let meshlets = take_range(&mut cursor, meshlet_count * std::mem::size_of::<ShaderMeshletData>());

		Some(ResourceLayout {
			streams: PreparedStreams {
				positions,
				normals,
				uvs,
				vertex_indices,
				primitive_indices,
				meshlets,
			},
			source_normals,
			source_uvs,
			uvs_are_f32,
			source_byte_count,
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
	fn read_targets<'b>(&self, backing: &'b mut [u8]) -> Vec<resource_management::stream::StreamMut<'b>> {
		let mut allocator = utils::BufferAllocator::new(backing);
		let mut streams = Vec::with_capacity(8);
		for (name, size) in [
			("Vertex.Position", self.positions.size),
			("Vertex.Normal", self.normals.size),
			("Vertex.UV", self.uvs.size),
			("VertexIndices", self.vertex_indices.size),
			("MeshletIndices", self.meshlet_indices.size),
		] {
			streams.push(resource_management::stream::StreamMut::new(
				name,
				allocator.take_with_offset_aligned(size, 4).1,
			));
		}
		streams.push(resource_management::stream::StreamMut::new(
			"Meshlets",
			allocator.take(self.meshlets.size),
		));
		if let Some((joints, weights)) = &self.skinning {
			for (name, size) in [("Vertex.Joints", joints.size), ("Vertex.Weights", weights.size)] {
				streams.push(resource_management::stream::StreamMut::new(
					name,
					allocator.take_with_offset_aligned(size, 4).1,
				));
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
		let Some(source) = meshlet_bytes.get(meshlet_stream.offset..meshlet_stream.offset + meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {index} meshlet range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate meshlet stream."
			);
			return None;
		};
		let meshlet_offset = meshlets.len() as u32;
		let mut local_primitive_offset = 0;
		let mut local_triangle_offset = 0;
		for bytes in source.chunks_exact(RESOURCE_MESHLET_STRIDE) {
			let meshlet = read_resource_meshlet(bytes);
			meshlets.push(ShaderMeshletData {
				primitive_offset: local_primitive_offset,
				triangle_offset: local_triangle_offset,
				primitive_count: meshlet.primitive_count,
				triangle_count: meshlet.triangle_count,
				center_radius: meshlet.center_radius,
				cone_apex_cutoff: meshlet.cone_apex_cutoff,
				cone_axis: encode_octahedral_unit_vector((meshlet.cone_axis[0], meshlet.cone_axis[1], meshlet.cone_axis[2])),
			});
			local_primitive_offset += meshlet.primitive_count;
			local_triangle_offset += meshlet.triangle_count;
		}

		let skinning = match primitive.skin {
			Some(_) => {
				let range = |semantic| {
					// Stream presence and bounds were validated by `ResourceStreams::validate_skinning`.
					let stream = primitive
						.stream(Streams::Vertices(semantic))
						.expect("validated skinning stream");
					stream.offset..stream.offset + stream.size
				};
				Some(SkinningCopy {
					positions: range(VertexSemantics::Position),
					normals: range(VertexSemantics::Normal),
					joints: range(VertexSemantics::Joints),
					weights: range(VertexSemantics::Weights),
				})
			}
			None => None,
		};
		primitives.push(PreparedPrimitive {
			material_id: primitive.material.id().to_string(),
			primitive: MeshPrimitive {
				material_index: 0,
				meshlet_count: (source.len() / RESOURCE_MESHLET_STRIDE) as u32,
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

/// One baked meshlet record decoded from the packed resource stream.
struct ResourceMeshlet {
	primitive_count: u32,
	triangle_count: u32,
	center_radius: [f32; 4],
	cone_apex_cutoff: [f32; 4],
	cone_axis: [f32; 4],
}

/// Decodes one packed meshlet record without assuming the resource stream is aligned.
fn read_resource_meshlet(bytes: &[u8]) -> ResourceMeshlet {
	let read_vec4 = |offset| {
		[
			read_f32(bytes, offset),
			read_f32(bytes, offset + 4),
			read_f32(bytes, offset + 8),
			read_f32(bytes, offset + 12),
		]
	};
	ResourceMeshlet {
		primitive_count: bytes[0] as u32,
		triangle_count: bytes[1] as u32,
		center_radius: read_vec4(4),
		cone_apex_cutoff: read_vec4(20),
		cone_axis: read_vec4(36),
	}
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
	f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four-byte float"))
}

/* Generated meshes */

/// Narrows generated indices only after proving that every value addresses an available vertex.
fn validated_generated_indices(indices: &[u32], vertex_count: usize) -> Option<Vec<u16>> {
	indices
		.iter()
		.map(|&index| {
			if index as usize >= vertex_count {
				log::error!(
					"Generated mesh index {index} references a missing vertex. The most likely cause is that the generator returned an index outside its {vertex_count} positions."
				);
				return None;
			}
			u16::try_from(index).ok().or_else(|| {
				log::error!(
					"Generated mesh index {index} exceeds the u16 vertex-index limit. The most likely cause is that one generated primitive contains more than 65536 vertices."
				);
				None
			})
		})
		.collect()
}

/// Greedily packs a generated triangle list into meshlets that respect the shader's vertex and triangle limits.
fn build_generated_meshlets(
	indices: &[u16],
	positions: &[(f32, f32, f32)],
) -> Option<(Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>)> {
	if !indices.len().is_multiple_of(3) {
		log::error!(
			"Generated mesh indices are invalid. The most likely cause is that the mesh generator returned a triangle list whose index count is not divisible by three."
		);
		return None;
	}
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
			center_radius: bounding_sphere(meshlet_vertices, positions),
			cone_apex_cutoff: [0.0, 0.0, 0.0, 2.0],
			cone_axis: encode_octahedral_unit_vector((0.0, 0.0, 1.0)),
		});
		vertex_indices.append(meshlet_vertices);
		primitive_indices.append(meshlet_triangles);
	};

	for triangle in indices.chunks_exact(3) {
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
	Some((vertex_indices, primitive_indices, meshlets))
}

/// Computes a conservative object-space bounding sphere for one generated meshlet.
fn bounding_sphere(meshlet_vertices: &[u16], positions: &[(f32, f32, f32)]) -> [f32; 4] {
	let mut min = [f32::INFINITY; 3];
	let mut max = [f32::NEG_INFINITY; 3];
	for &index in meshlet_vertices {
		let (x, y, z) = positions[index as usize];
		for (axis, value) in [x, y, z].into_iter().enumerate() {
			min[axis] = min[axis].min(value);
			max[axis] = max[axis].max(value);
		}
	}
	let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
	let radius_squared = meshlet_vertices
		.iter()
		.map(|&index| {
			let (x, y, z) = positions[index as usize];
			let delta = [x - center[0], y - center[1], z - center[2]];
			delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]
		})
		.fold(0.0f32, f32::max);
	[center[0], center[1], center[2], radius_squared.sqrt()]
}

/* Attribute packing */

/// Octahedrally encodes one unit vector into two UNORM16 components.
pub(crate) fn encode_octahedral_unit_vector(vector: (f32, f32, f32)) -> RuntimeUnitVector {
	let length = vector.0.abs() + vector.1.abs() + vector.2.abs();
	if !length.is_finite() || length == 0.0 {
		return [32768, 32768];
	}
	let mut x = vector.0 / length;
	let mut y = vector.1 / length;
	let z = vector.2 / length;
	if z < 0.0 {
		let sign = |value: f32| if value < 0.0 { -1.0 } else { 1.0 };
		let (previous_x, previous_y) = (x, y);
		x = (1.0 - previous_y.abs()) * sign(previous_x);
		y = (1.0 - previous_x.abs()) * sign(previous_y);
	}
	let unorm16 = |value: f32| ((value * 0.5 + 0.5).clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
	[unorm16(x), unorm16(y)]
}

fn write_unit_vector(destination: &mut [u8], vector: (f32, f32, f32)) {
	let encoded = encode_octahedral_unit_vector(vector);
	destination[..2].copy_from_slice(&encoded[0].to_ne_bytes());
	destination[2..4].copy_from_slice(&encoded[1].to_ne_bytes());
}

fn write_f16_pair(destination: &mut [u8], u: f32, v: f32) {
	destination[..2].copy_from_slice(&half::f16::from_f32(u).to_bits().to_ne_bytes());
	destination[2..4].copy_from_slice(&half::f16::from_f32(v).to_bits().to_ne_bytes());
}

/// Converts an f32 UV stream to half-float storage without clamping sampler coordinates.
fn pack_f32_uvs(source: &[u8], destination: &mut [u8], vertex_count: usize) {
	for (source, destination) in source
		.chunks_exact(F32_UV_STRIDE)
		.zip(destination.chunks_exact_mut(F16_UV_STRIDE))
		.take(vertex_count)
	{
		write_f16_pair(destination, read_f32(source, 0), read_f32(source, 4));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::mesh::generator::BoxMeshGenerator;

	#[test]
	fn generated_mesh_preparation_owns_complete_transfer_data() {
		let bytes = Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice());
		let executor = resource_management::r#async::Executor::new().expect("mesh preparation test executor");
		let prepared = executor
			.block_on(async {
				let (staging, worker) = UploadStagingArena::new_for_test(bytes);
				resource_management::r#async::spawn(worker.run()).detach();
				PreparedMesh::generated(&BoxMeshGenerator::new(), staging).await
			})
			.expect("The built-in box should produce valid visibility geometry.");

		assert_eq!(
			prepared.counts,
			GeometryCounts {
				vertices: 24,
				primitive_indices: 24,
				triangles: 12,
				meshlets: 1,
				skinning_vertices: 0,
			}
		);
		assert_eq!(prepared.primitives.len(), 1);
		assert_eq!(prepared.primitives[0].primitive.meshlet_count, 1);
		assert_eq!(prepared.primitives[0].material_id, GENERATED_MESH_MATERIAL);
	}

	#[test]
	fn generated_indices_are_checked_before_u16_narrowing() {
		assert_eq!(validated_generated_indices(&[0, 2, 1], 3), Some(vec![0, 2, 1]));
		assert!(validated_generated_indices(&[3], 3).is_none());
		assert!(validated_generated_indices(&[u16::MAX as u32 + 1], u16::MAX as usize + 2).is_none());
	}

	#[test]
	fn octahedral_encoding_preserves_axes_and_folds_the_lower_hemisphere() {
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 1.0)), [32768, 32768]);
		assert_eq!(encode_octahedral_unit_vector((1.0, 0.0, 0.0)), [65535, 32768]);
		assert_eq!(encode_octahedral_unit_vector((0.0, -1.0, 0.0)), [32768, 0]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, -1.0)), [65535, 65535]);
		assert_eq!(encode_octahedral_unit_vector((0.0, 0.0, 0.0)), [32768, 32768]);
	}

	#[test]
	fn packed_uvs_preserve_wrapping_coordinates() {
		let values = [[-0.5f32, 2.0f32], [1.25f32, -3.0f32]];
		let source = values
			.iter()
			.flat_map(|uv| uv.iter().flat_map(|component| component.to_ne_bytes()))
			.collect::<Vec<_>>();
		let mut packed = [0u8; 8];

		pack_f32_uvs(&source, &mut packed, values.len());

		let decoded = packed
			.chunks_exact(2)
			.map(|bytes| half::f16::from_bits(u16::from_ne_bytes(bytes.try_into().unwrap())).to_f32())
			.collect::<Vec<_>>();
		assert_eq!(decoded, vec![-0.5, 2.0, 1.25, -3.0]);
	}
}
