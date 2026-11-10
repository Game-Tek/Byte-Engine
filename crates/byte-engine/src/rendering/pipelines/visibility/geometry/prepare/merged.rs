//! Merged meshes: copies of one baked mesh, built in memory as a single mesh.
//!
//! A level that places one mesh hundreds of times draws cheaper when nearby copies share a primitive, which the
//! renderer culls and picks a detail level for as a whole. Storing such geometry would repeat the mesh once per copy
//! on disk, so it is built here instead, from two things: the baked mesh, read once into a [`MergedSource`] that stays
//! in memory, and the copies' placements.
//!
//! A merged mesh holds no geometry of its own. Each primitive holds exactly one detail level at a time, the one its
//! instances want, built by [`PreparedMesh::merged`] by repeating that level of the source once for each copy: its
//! vertices transformed, and its meshlets copied with their bounds and cones transformed. Nothing is clustered again,
//! so building a level is a straight copy.
//!
//! A small mesh bakes into meshlets far below the size a meshlet may reach, and repeating them would draw as many
//! meshlets as copies. So consecutive meshlets, of one copy and of the copies after it, are packed into one while
//! they fit; see [`SourceLevel::pack`]. Copies follow one another in the order their placements are given, so pass
//! neighbors next to each other to keep a packed meshlet compact.

use resource_management::resources::mesh::{MAX_LEVEL_MESHLETS, MAX_PRIMITIVE_VERTICES, Primitive};

use super::*;
use crate::gameplay::Transform;

/// The `MergedSource` struct keeps one baked mesh in memory in the form merged levels are built from, so no level
/// of a merged mesh reads the resource again.
///
/// Read it once with [`Self::read`]. Describe a merged mesh with [`Self::primitives`], then build the level each
/// primitive wants with [`PreparedMesh::merged`].
pub(crate) struct MergedSource {
	primitives: Vec<SourcePrimitive>,
}

/// The `SourcePrimitive` struct is one primitive of a [`MergedSource`]: every vertex, and every level's meshlets.
struct SourcePrimitive {
	material: String,
	/// Object-space sphere around full detail, as xyz center and w radius.
	bounding_sphere: [f32; 4],
	/// The primitive's vertices, ordered by the coarsest level that uses them, coarsest first, as baked.
	positions: Vec<[f32; 3]>,
	normals: Vec<[f32; 3]>,
	/// UVs, already as the half-float pairs the vertex buffers store.
	uvs: Vec<[u8; 4]>,
	/// The detail levels, finest first.
	levels: Vec<SourceLevel>,
}

/// The `SourceLevel` struct is one detail level of a [`SourcePrimitive`].
struct SourceLevel {
	/// How many of the primitive's first vertices the level uses.
	vertex_count: u32,
	error: f32,
	meshlets: Vec<SourceMeshlet>,
	/// Each meshlet's vertices, one meshlet after another, as indices of the primitive's vertices.
	vertex_indices: Vec<u16>,
	/// Each meshlet's triangles, one meshlet after another, as indices of the meshlet's vertices.
	triangles: Vec<[u8; 3]>,
}

impl SourceLevel {
	/// Packs this level's meshlets, repeated for `copies` copies, into as few meshlets as hold them in order, and
	/// calls `packed` with each one's meshlets: a range of the sequence that lists every meshlet of the first copy,
	/// then of the second, and so on.
	///
	/// A packed meshlet takes the next meshlet while both stay within the vertices and triangles one meshlet draws.
	/// Only neighbors in the sequence join, so the result depends on nothing but the counts and is the same each time.
	fn pack(&self, copies: usize, mut packed: impl FnMut(Range<usize>)) {
		let (mut first, mut vertices, mut triangles) = (0, 0, 0);
		let total = copies * self.meshlets.len();
		for index in 0..total {
			let [meshlet_vertices, meshlet_triangles] = self.meshlets[index % self.meshlets.len()].counts.map(u32::from);
			if index > first && (vertices + meshlet_vertices > VERTEX_COUNT || triangles + meshlet_triangles > TRIANGLE_COUNT) {
				packed(first..index);
				(first, vertices, triangles) = (index, 0, 0);
			}
			vertices += meshlet_vertices;
			triangles += meshlet_triangles;
		}
		if first < total {
			packed(first..total);
		}
	}
}

/// The `SourceMeshlet` struct is one baked meshlet's counts and culling bounds.
struct SourceMeshlet {
	/// Vertex and triangle counts.
	counts: [u8; 2],
	sphere: [f32; 4],
	cone_axis: [f32; 3],
	cone_cutoff: f32,
}

impl MergedSource {
	/// Reads `mesh` from its whole loaded `payload`.
	///
	/// Returns the most likely cause and its fix for a mesh that can't merge: one that is skinned or animated by a
	/// node, since merged geometry never moves, or one whose payload doesn't hold what the mesh describes.
	pub(crate) fn read(mesh: &Mesh, payload: &[u8]) -> Result<Self, &'static str> {
		if mesh
			.primitives
			.iter()
			.any(|primitive| primitive.skin.is_some() || primitive.transform_node.is_some())
		{
			return Err(
				"The most likely cause is that the mesh is skinned or animated, and merged geometry never moves. Merge a static mesh instead.",
			);
		}
		mesh.primitives
			.iter()
			.map(|primitive| SourcePrimitive::read(mesh, primitive, payload))
			.collect::<Option<Vec<_>>>()
			.map(|primitives| Self { primitives })
			.ok_or(
				"The most likely cause is a mesh baked by another engine version, or one without normals or UVs. Bake the mesh again.",
			)
	}

	/// Returns the material ID of each primitive, in primitive order.
	pub(crate) fn materials(&self) -> impl Iterator<Item = &str> {
		self.primitives.iter().map(|primitive| primitive.material.as_str())
	}

	/// Returns the primitives of the mesh that repeats this source at each of `placements`, none holding any geometry
	/// yet: each one's bounds, and its detail levels with none resident.
	///
	/// Returns the most likely cause when the copies don't fit one primitive.
	pub(crate) fn primitives(&self, placements: &[Transform]) -> Result<Vec<MeshPrimitive>, String> {
		let copies: Vec<Copy> = placements.iter().map(Copy::new).collect();
		self.primitives
			.iter()
			.map(|primitive| {
				let full = &primitive.levels[0];
				if copies.len() * full.vertex_count as usize > MAX_PRIMITIVE_VERTICES
					|| copies.len() * full.meshlets.len() > MAX_LEVEL_MESHLETS
				{
					return Err(format!(
						"Merged mesh was rejected. The most likely cause is that {} copies of a primitive with {} vertices and {} meshlets don't fit the {MAX_PRIMITIVE_VERTICES} vertices and {MAX_LEVEL_MESHLETS} meshlets one primitive holds. Merge fewer copies into each mesh.",
						copies.len(),
						full.vertex_count,
						full.meshlets.len()
					));
				}
				Ok(MeshPrimitive {
					lods: primitive.lods(&copies, None),
					bounding_sphere: sphere_around(copies.iter().map(|copy| copy.sphere(primitive.bounding_sphere))),
					..MeshPrimitive::default()
				})
			})
			.collect()
	}
}

impl SourcePrimitive {
	fn read(mesh: &Mesh, primitive: &Primitive, payload: &[u8]) -> Option<Self> {
		// A primitive's range of a stream is relative to the mesh's stream of that type and level.
		let bytes = |stream_type: Streams, level: usize| {
			let block = mesh.stream(stream_type, level as u8)?;
			let range = primitive.stream(stream_type, level as u8)?;
			let start = block.offset.checked_add(range.offset)?;
			Some((payload.get(start..start.checked_add(range.size)?)?, range.stride))
		};
		let vertex_count = primitive.vertex_count as usize;
		let (mut positions, mut normals, mut uvs) = (
			Vec::with_capacity(vertex_count),
			Vec::with_capacity(vertex_count),
			Vec::with_capacity(vertex_count),
		);
		// Each level's streams hold the vertices it adds to the coarser levels, so they are read coarsest first.
		for level in (0..primitive.lods.len()).rev() {
			let Some((level_positions, SKINNING_POSITION_STRIDE)) = bytes(Streams::Vertices(VertexSemantics::Position), level)
			else {
				// A level that adds no vertices has no streams.
				continue;
			};
			let (level_normals, SKINNING_NORMAL_STRIDE) = bytes(Streams::Vertices(VertexSemantics::Normal), level)? else {
				return None;
			};
			let (level_uvs, uv_stride) = bytes(Streams::Vertices(VertexSemantics::UV), level)?;
			let vector = |bytes: &[u8; 12]| std::array::from_fn(|component| read_f32(bytes, component * 4));
			positions.extend(level_positions.as_chunks::<12>().0.iter().map(vector));
			normals.extend(level_normals.as_chunks::<12>().0.iter().map(vector));
			match uv_stride {
				F32_UV_STRIDE => uvs.extend(level_uvs.as_chunks::<F32_UV_STRIDE>().0.iter().map(|uv| {
					let mut packed = [0; 4];
					write_f16_pair(&mut packed, read_f32(uv, 0), read_f32(uv, 4));
					packed
				})),
				F16_UV_STRIDE => uvs.extend_from_slice(level_uvs.as_chunks::<F16_UV_STRIDE>().0),
				_ => return None,
			}
		}
		if positions.len() != vertex_count || normals.len() != vertex_count || uvs.len() != vertex_count {
			return None;
		}

		let levels = primitive
			.lods
			.iter()
			.enumerate()
			.map(|(level, lod)| {
				let (records, RESOURCE_MESHLET_STRIDE) = bytes(MESHLET_STREAMS[2], level)? else {
					return None;
				};
				let (vertex_indices, _) = bytes(MESHLET_STREAMS[0], level)?;
				let (triangles, _) = bytes(MESHLET_STREAMS[1], level)?;
				let meshlets: Vec<SourceMeshlet> = records
					.as_chunks::<RESOURCE_MESHLET_STRIDE>()
					.0
					.iter()
					.map(|record| {
						let [x, y, z, _] = read_vec4(record, RESOURCE_CONE_AXIS_OFFSET);
						SourceMeshlet {
							counts: [record[0], record[1]],
							sphere: read_vec4(record, RESOURCE_SPHERE_OFFSET),
							cone_axis: [x, y, z],
							cone_cutoff: read_vec4(record, RESOURCE_CONE_CUTOFF_OFFSET)[3],
						}
					})
					.collect();
				let vertex_indices: Vec<u16> = vertex_indices
					.as_chunks::<VERTEX_INDEX_STRIDE>()
					.0
					.iter()
					.map(|index| u16::from_le_bytes(*index))
					.collect();
				let triangles = triangles.as_chunks::<3>().0.to_vec();
				// The meshlets must cover both index streams and stay inside the level's vertices.
				let covered = meshlets.iter().fold((0, 0), |(vertices, triangles), meshlet| {
					(vertices + meshlet.counts[0] as usize, triangles + meshlet.counts[1] as usize)
				});
				(meshlets.len() == lod.meshlet_count as usize
					&& covered == (vertex_indices.len(), triangles.len())
					&& vertex_indices.iter().all(|&index| u32::from(index) < lod.vertex_count)
					&& lod.vertex_count as usize <= vertex_count)
					.then_some(SourceLevel {
						vertex_count: lod.vertex_count,
						error: lod.error,
						meshlets,
						vertex_indices,
						triangles,
					})
			})
			.collect::<Option<Vec<_>>>()?;
		if levels.is_empty() {
			return None;
		}

		Some(Self {
			material: mesh.material(primitive).id().to_string(),
			bounding_sphere: primitive.bounding_sphere,
			positions,
			normals,
			uvs,
			levels,
		})
	}

	/// Returns the level table of this primitive repeated at `copies`, with `resident`, a level and the record and
	/// first vertex it was built at, as its only resident level.
	///
	/// Every copy of a level shows the source's error enlarged by its own scale, and one error has to hold for all of
	/// them, so each level takes the error of the most enlarged copy.
	fn lods(&self, copies: &[Copy], resident: Option<(usize, MeshLod)>) -> MeshLods {
		let largest_scale = largest_scale(copies);
		let mut lods = [MeshLod::default(); MAX_PRIMITIVE_LODS];
		for (index, (lod, level)) in lods.iter_mut().zip(&self.levels).enumerate() {
			*lod = match resident {
				Some((resident, built)) if resident == index => built,
				_ => MeshLod::default(),
			};
			let mut meshlets = 0;
			level.pack(copies.len(), |_| meshlets += 1);
			lod.meshlet_count = meshlets;
			lod.error = level.error * largest_scale;
		}
		MeshLods::new(&lods[..self.levels.len()])
	}
}

/// The `Copy` struct is one placement prepared for transforming many vertices: its rotation as matrix columns.
struct Copy {
	position: [f32; 3],
	scale: [f32; 3],
	axes: [[f32; 3]; 3],
	/// Whether the placement flips handedness, which turns triangles the other way.
	mirrored: bool,
	largest_scale: f32,
	/// Whether every axis scales alike, which keeps the angles a normal cone's cutoff measures.
	uniform: bool,
}

impl Copy {
	fn new(placement: &Transform) -> Self {
		let (position, scale) = (placement.get_position(), placement.scale().to_array());
		let magnitudes = scale.map(f32::abs);
		let largest_scale = magnitudes.into_iter().fold(0.0, f32::max);
		let smallest_scale = magnitudes.into_iter().fold(f32::INFINITY, f32::min);
		Self {
			position: [position.x(), position.y(), position.z()],
			scale,
			axes: placement
				.get_orientation()
				.axes::<()>()
				.map(|axis| axis.into_vector().to_array()),
			mirrored: scale.iter().product::<f32>() < 0.0,
			largest_scale,
			uniform: largest_scale - smallest_scale <= largest_scale * 1e-3,
		}
	}

	/// Turns `vector` by the placement's rotation.
	fn rotate(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
		std::array::from_fn(|row| self.axes[0][row] * x + self.axes[1][row] * y + self.axes[2][row] * z)
	}

	/// Returns where the placement puts `point`: scaled, then turned, then moved, as a level's `transform` applies.
	fn point(&self, point: [f32; 3]) -> [f32; 3] {
		let rotated = self.rotate(std::array::from_fn(|axis| point[axis] * self.scale[axis]));
		std::array::from_fn(|axis| rotated[axis] + self.position[axis])
	}

	/// Transforms a surface normal, which a non-uniform scale tilts the opposite way from the surface. The result
	/// need not be unit length: the octahedral encoding and the cone encoding both normalize.
	fn normal(&self, normal: [f32; 3]) -> (f32, f32, f32) {
		// A flattened axis has no inverse; its normal component is dropped instead of becoming infinite.
		let [x, y, z] = self.rotate(std::array::from_fn(|axis| {
			if self.scale[axis] == 0.0 {
				0.0
			} else {
				normal[axis] / self.scale[axis]
			}
		}));
		(x, y, z)
	}

	/// Returns a sphere around where the placement puts `sphere`, as xyz center and w radius.
	fn sphere(&self, [x, y, z, radius]: [f32; 4]) -> [f32; 4] {
		let [x, y, z] = self.point([x, y, z]);
		[x, y, z, radius * self.largest_scale]
	}
}

/// Returns the scale of the most enlarged of `copies`.
fn largest_scale(copies: &[Copy]) -> f32 {
	copies.iter().map(|copy| copy.largest_scale).fold(0.0, f32::max)
}

impl PreparedMesh {
	/// Builds, for each primitive `levels` lists, the one detail level it names of the mesh that repeats `source` at
	/// each of `placements`, in the runtime geometry formats.
	///
	/// `levels` ascends by primitive. Each listed primitive's level table marks only that level resident, and an
	/// unlisted primitive holds nothing. Meshlets too small to fill one are packed with their neighbors, in the order
	/// of `placements`. The copies must fit one primitive, which [`MergedSource::primitives`] checks.
	/// Merged levels keep no lightmap UVs, since copies would share them, and no flat triangles for rays. Next, place
	/// each primitive with [`super::GeometryBuffers::append_primitives`].
	pub(crate) async fn merged(
		source: &MergedSource,
		placements: &[Transform],
		upload_staging: Arc<UploadStagingArena>,
		levels: &[PrimitiveLevels],
	) -> Option<Self> {
		let copies: Vec<Copy> = placements.iter().map(Copy::new).collect();
		// The level each primitive builds, or `None` for one the load doesn't list.
		let built = |primitive: usize| {
			let level = levels
				.binary_search_by_key(&(primitive as u32), |levels| levels.primitive)
				.ok()?;
			let source = &source.primitives[primitive];
			Some((levels[level].finest as usize).min(source.levels.len() - 1))
		};
		let mut counts = GeometryCounts::default();
		for (index, primitive) in source.primitives.iter().enumerate() {
			let Some(level) = built(index) else {
				continue;
			};
			let level = &primitive.levels[level];
			let copies = copies.len() as u32;
			counts.vertices += copies * level.vertex_count;
			counts.primitive_indices += copies * level.vertex_indices.len() as u32;
			counts.triangles += copies * level.triangles.len() as u32;
			level.pack(copies as usize, |_| counts.meshlets += 1);
			counts.levels += 1;
		}

		const MESHLET_STRIDE: usize = std::mem::size_of::<ShaderMeshletData>();
		const LEVEL_STRIDE: usize = std::mem::size_of::<ShaderMeshLevel>();
		let mut cursor = 0;
		let streams = PreparedStreams {
			positions: take_range(&mut cursor, counts.vertices as usize * 12),
			normals: take_range(&mut cursor, counts.vertices as usize * VERTEX_NORMAL_BUFFER_STRIDE as usize),
			uvs: take_range(&mut cursor, counts.vertices as usize * VERTEX_UV_BUFFER_STRIDE as usize),
			lightmap_uvs: take_range(
				&mut cursor,
				counts.vertices as usize * VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize,
			),
			vertex_indices: take_range(&mut cursor, counts.primitive_indices as usize * VERTEX_INDEX_STRIDE),
			primitive_indices: take_range(&mut cursor, counts.triangles as usize * 3),
			meshlets: take_range(&mut cursor, counts.meshlets as usize * MESHLET_STRIDE),
			levels: take_range(&mut cursor, counts.levels as usize * LEVEL_STRIDE),
			flat_triangles: None,
		};
		let mut staging = allocate_staging(&upload_staging, cursor).await?;
		let backing = staging.bytes_mut();
		backing[streams.lightmap_uvs.clone()].fill(0);

		// Where the next element of each stream goes, counted in elements from the start of the prepared streams.
		let (mut vertex, mut vertex_index, mut triangle, mut meshlet, mut level_record) =
			(0usize, 0usize, 0usize, 0usize, 0usize);
		// Scratch the primitives share: each source meshlet's sphere at each copy, and each packed meshlet's source
		// meshlets with the sphere around them.
		let mut spheres = Vec::new();
		let mut packed = Vec::<(Range<usize>, [f32; 4])>::new();
		let mut primitives = Vec::with_capacity(source.primitives.len());
		for (index, primitive) in source.primitives.iter().enumerate() {
			let Some(level_index) = built(index) else {
				primitives.push(PreparedPrimitive {
					primitive: MeshPrimitive {
						lods: primitive.lods(&copies, None),
						..MeshPrimitive::default()
					},
					elements: MeshRanges::default(),
					skinning: None,
				});
				continue;
			};
			let level = &primitive.levels[level_index];
			let level_vertices = level.vertex_count as usize;
			let (first_vertex, first_vertex_index, first_triangle, first_meshlet) = (vertex, vertex_index, triangle, meshlet);

			// Each copy's vertices, one copy after another.
			for copy in &copies {
				let positions = &mut backing[streams.positions.clone()].as_chunks_mut::<12>().0[vertex..][..level_vertices];
				for (destination, &position) in positions.iter_mut().zip(&primitive.positions) {
					*destination = bytemuck::cast(copy.point(position));
				}
				let normals = &mut backing[streams.normals.clone()].as_chunks_mut::<4>().0[vertex..][..level_vertices];
				for (destination, &normal) in normals.iter_mut().zip(&primitive.normals) {
					write_unit_vector(destination, copy.normal(normal));
				}
				backing[streams.uvs.clone()].as_chunks_mut::<4>().0[vertex..][..level_vertices]
					.copy_from_slice(&primitive.uvs[..level_vertices]);
				vertex += level_vertices;
			}

			// Where each source meshlet lands, per copy, then the packed meshlets over them. The level's sphere comes
			// before any record, since each meshlet stores its sphere within it.
			let source_meshlets = level.meshlets.len();
			spheres.clear();
			spheres.extend(
				copies
					.iter()
					.flat_map(|copy| level.meshlets.iter().map(|meshlet| copy.sphere(meshlet.sphere))),
			);
			packed.clear();
			level.pack(copies.len(), |meshlets| {
				let sphere = sphere_around(spheres[meshlets.clone()].iter().copied());
				packed.push((meshlets, sphere));
			});
			let bounding_sphere = level_sphere(sphere_around(packed.iter().map(|(_, sphere)| *sphere)));

			// The source meshlet's start in the level's index streams, which restarts with each copy.
			let (mut source_vertex_index, mut source_triangle) = (0, 0);
			for (meshlets, sphere) in &packed {
				let (first_meshlet_vertex_index, first_meshlet_triangle) = (vertex_index, triangle);
				let (mut meshlet_vertices, mut meshlet_triangles) = (0u8, 0u8);
				for index in meshlets.clone() {
					let (copy_index, source_index) = (index / source_meshlets, index % source_meshlets);
					if source_index == 0 {
						(source_vertex_index, source_triangle) = (0, 0);
					}
					let (copy, source_meshlet) = (&copies[copy_index], &level.meshlets[source_index]);
					let [vertex_count, triangle_count] = source_meshlet.counts.map(usize::from);

					// The copy's vertices follow the earlier copies', so its meshlets name them that much further on.
					let first_copy_vertex = (copy_index * level_vertices) as u16;
					let vertex_indices = &mut backing[streams.vertex_indices.clone()]
						.as_chunks_mut::<VERTEX_INDEX_STRIDE>()
						.0[vertex_index..][..vertex_count];
					for (destination, &index) in vertex_indices
						.iter_mut()
						.zip(&level.vertex_indices[source_vertex_index..][..vertex_count])
					{
						*destination = (first_copy_vertex + index).to_le_bytes();
					}
					let triangles =
						&mut backing[streams.primitive_indices.clone()].as_chunks_mut::<3>().0[triangle..][..triangle_count];
					for (destination, &corners) in triangles
						.iter_mut()
						.zip(&level.triangles[source_triangle..][..triangle_count])
					{
						// A corner names a vertex of its meshlet, which now follow the vertices packed before them.
						let [a, b, c] = corners.map(|corner| corner + meshlet_vertices);
						// Mirroring turns a triangle the other way, so a mirrored copy's corners swap to keep it facing out.
						*destination = if copy.mirrored { [a, c, b] } else { [a, b, c] };
					}
					meshlet_vertices += vertex_count as u8;
					meshlet_triangles += triangle_count as u8;
					vertex_index += vertex_count;
					triangle += triangle_count;
					source_vertex_index += vertex_count;
					source_triangle += triangle_count;
				}

				// One meshlet keeps its normal cone, turned with its copy. Several have no cone in common, and a
				// stretched cone no longer bounds its normals; a cutoff past one keeps such a meshlet.
				let (cone_axis, cone_cutoff) = match meshlets.len() {
					1 if copies[meshlets.start / source_meshlets].uniform => {
						let source_meshlet = &level.meshlets[meshlets.start % source_meshlets];
						let (x, y, z) = copies[meshlets.start / source_meshlets].normal(source_meshlet.cone_axis);
						([x, y, z], source_meshlet.cone_cutoff)
					}
					_ => ([0.0, 0.0, 1.0], 2.0),
				};
				let record = ShaderMeshletData::new(
					first_meshlet_vertex_index as u32,
					first_meshlet_triangle as u32,
					[meshlet_vertices, meshlet_triangles],
					*sphere,
					bounding_sphere,
					cone_axis,
					cone_cutoff,
				);
				backing[streams.meshlets.clone()][meshlet * MESHLET_STRIDE..][..MESHLET_STRIDE]
					.copy_from_slice(bytemuck::bytes_of(&record));
				meshlet += 1;
			}

			let record = ShaderMeshLevel {
				bounding_sphere,
				first_meshlet: first_meshlet as u32,
				meshlet_count: (meshlet - first_meshlet) as u32,
				vertex_base: first_vertex as u32,
				// Each copy casts a shadow of its own, so the largest copy stands for the level, not the level's sphere.
				caster_radius: primitive.bounding_sphere[3] * largest_scale(&copies),
			};
			backing[streams.levels.clone()][level_record * LEVEL_STRIDE..][..LEVEL_STRIDE]
				.copy_from_slice(bytemuck::bytes_of(&record));
			let resident = MeshLod {
				level: level_record as u32,
				vertex_base: first_vertex as u32,
				resident: true,
				..MeshLod::default()
			};
			primitives.push(PreparedPrimitive {
				primitive: MeshPrimitive {
					lods: primitive.lods(&copies, Some((level_index, resident))),
					vertex_offset: first_vertex as u32,
					vertex_count: (vertex - first_vertex) as u32,
					..MeshPrimitive::default()
				},
				elements: MeshRanges {
					vertices: first_vertex as u32..vertex as u32,
					primitive_indices: first_vertex_index as u32..vertex_index as u32,
					triangles: first_triangle as u32..triangle as u32,
					meshlets: first_meshlet as u32..meshlet as u32,
					levels: level_record as u32..level_record as u32 + 1,
					..MeshRanges::default()
				},
				skinning: None,
			});
			level_record += 1;
		}

		Some(Self {
			staging,
			streams,
			primitives,
			counts,
			skeleton_node_count: 0,
		})
	}
}

#[cfg(test)]
mod tests {
	use std::convert::Infallible;

	use math::{Orientation, Point, Scale, Vector};
	use resource_management::ReferenceModel;
	use resource_management::processors::mesh::{MeshPrimitiveSource, MeshProcessorSession, MeshSettings};
	use resource_management::resources::material::VariantModel;
	use resource_management::types::VertexComponent;

	use super::*;

	/// A rippled square of quads, one unit wide, facing +Z, with this many quads to a side. The ripples leave coarser
	/// levels an error.
	struct Ripples(u32);

	impl Ripples {
		fn vertices(&self) -> impl ExactSizeIterator<Item = [f32; 2]> {
			let side = self.0;
			(0..(side + 1) * (side + 1))
				.map(move |vertex| [vertex % (side + 1), vertex / (side + 1)].map(|v| v as f32 / side as f32))
		}
	}

	impl MeshPrimitiveSource for Ripples {
		type Error = Infallible;

		fn material_slot(&self) -> usize {
			0
		}

		fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Infallible>> + '_, Infallible> {
			let side = self.0;
			let quads = (0..side * side).flat_map(move |quad| {
				let corner = quad / side * (side + 1) + quad % side;
				let (right, above) = (corner + 1, corner + side + 1);
				[corner, right, above, right, above + 1, above]
			});
			Ok(quads.collect::<Vec<_>>().into_iter().map(Ok))
		}

		fn positions(
			&self,
		) -> Result<
			impl ExactSizeIterator<Item = Result<Point<resource_management::resources::ModelSpace>, Infallible>> + '_,
			Infallible,
		> {
			Ok(self
				.vertices()
				.map(|[x, y]| Ok(Point::new(x, y, 0.02 * (x * 20.0).sin() * (y * 20.0).cos()))))
		}

		fn normals(
			&self,
		) -> Result<
			Option<impl ExactSizeIterator<Item = Result<Vector<resource_management::resources::ModelSpace>, Infallible>> + '_>,
			Infallible,
		> {
			Ok(Some(self.vertices().map(|_| Ok(Vector::new(0.0, 0.0, 1.0)))))
		}

		fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Infallible>> + '_>, Infallible> {
			Ok(Some(self.vertices().map(Ok)))
		}
	}

	/// Bakes the rippled square of 16 by 16 quads and reads it back as a merged source.
	fn source() -> MergedSource {
		source_of(16)
	}

	/// Bakes a rippled square with `side` quads to a side and reads it back as a merged source.
	fn source_of(side: u32) -> MergedSource {
		let layout = [VertexSemantics::Position, VertexSemantics::Normal, VertexSemantics::UV]
			.map(VertexComponent::canonical)
			.to_vec();
		let mut session = MeshProcessorSession::new(layout, None, Vec::new(), MeshSettings::default()).unwrap();
		session.push_primitive(&Ripples(side)).unwrap();
		// Only the variant's ID is read here, so its model stays empty.
		let material = ReferenceModel::<VariantModel>::new_serialized("grass.variant", 0, 0, Vec::new(), None);
		let baked = session.finish(&[material]);
		let mesh = Mesh {
			skeleton: None,
			skins: baked.mesh.skins,
			vertex_components: baked.mesh.vertex_components,
			streams: baked.mesh.streams,
			materials: baked.mesh.materials,
			primitives: baked.mesh.primitives,
		};
		MergedSource::read(&mesh, &baked.buffer).expect("a static mesh with normals and UVs should read")
	}

	/// One copy in place, one enlarged, stood up on the x axis and moved, and one mirrored and moved.
	fn placements() -> [Transform; 3] {
		let half = std::f32::consts::FRAC_1_SQRT_2;
		[
			Transform::identity(),
			Transform::new(
				Point::new(10.0, 0.0, 0.0),
				Scale::new(2.0, 2.0, 2.0),
				// A quarter turn around x takes +y to +z and +z to -y.
				Orientation::try_from_array([half, 0.0, 0.0, half]).unwrap(),
			),
			Transform::new(
				Point::new(-10.0, 0.0, 0.0),
				Scale::new(-1.0, 1.0, 1.0),
				Orientation::identity(),
			),
		]
	}

	/// Builds level `level` of the rippled square merged at `placements`, on a staging arena of its own.
	fn build(source: &MergedSource, placements: &[Transform], level: u8, check: impl FnOnce(&PreparedMesh)) {
		let mut bytes = vec![0u8; 1 << 20];
		let executor = resource_management::r#async::Executor::new().expect("merged preparation test executor");
		executor.block_on(async {
			let (staging, worker) = UploadStagingArena::new_for_test(&mut bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let levels = [PrimitiveLevels {
				primitive: 0,
				finest: level,
				block: None,
				full: None,
			}];
			let prepared = PreparedMesh::merged(source, placements, staging, &levels)
				.await
				.expect("a merged level should build");
			check(&prepared);
		});
	}

	#[test]
	fn a_merged_mesh_starts_without_geometry_and_scales_each_level_by_its_copies() {
		let source = source();
		let levels = &source.primitives[0].levels;
		assert!(
			levels.len() > 1,
			"The rippled square should bake into more than one detail level."
		);

		let primitives = source.primitives(&placements()).expect("three copies fit one primitive");

		let [primitive] = primitives.as_slice() else {
			panic!("The merged mesh should have the source's one primitive.");
		};
		assert_eq!(primitive.lods.levels().len(), levels.len());
		assert_eq!(primitive.lods.drawn(0), None);
		for (lod, level) in primitive.lods.levels().iter().zip(levels) {
			// Packing joins meshlets, so the copies never take more than three times the source's.
			assert!((1..=3 * level.meshlets.len()).contains(&(lod.meshlet_count as usize)));
			// The enlarged copy shows each level's error twice as large.
			assert_eq!(lod.error, 2.0 * level.error);
		}
		// The sphere reaches the copies ten units to either side.
		let [x, _, _, radius] = primitive.bounding_sphere;
		assert!(x - radius <= -11.0 && x + radius >= 12.0);
		assert_eq!(source.materials().collect::<Vec<_>>(), ["grass.variant"]);
	}

	#[test]
	fn copies_that_do_not_fit_one_primitive_are_rejected() {
		let source = source();
		// The square has 289 vertices, so 227 copies pass the 65,536 one primitive holds.
		let placements = vec![Transform::identity(); 227];

		assert!(source.primitives(&placements).is_err());
	}

	#[test]
	fn a_built_level_repeats_the_source_level_at_each_placement() {
		let source = source();
		let placements = placements();
		let level = &source.primitives[0].levels[1];
		let level_vertices = level.vertex_count as usize;

		build(&source, &placements, 1, |prepared| {
			let bytes = prepared.staging.bytes();
			let primitive = &prepared.primitives[0];
			// Only the built level is resident, whatever level an instance wants.
			let drawn = primitive.primitive.lods.drawn(0).expect("the built level is resident");
			assert_eq!(drawn.meshlet_count, prepared.counts.meshlets);
			assert_eq!(primitive.primitive.lods.levels().iter().filter(|lod| lod.resident).count(), 1);
			assert_eq!(prepared.counts.vertices as usize, 3 * level_vertices);
			assert_eq!(prepared.counts.levels, 1);

			let positions: &[[f32; 3]] = bytemuck::cast_slice(&bytes[prepared.streams.positions.clone()]);
			let normals: Vec<[f32; 3]> = bytes[prepared.streams.normals.clone()]
				.as_chunks::<4>()
				.0
				.iter()
				.map(|normal| decode_octahedral_unit_vector(bytemuck::cast(*normal)))
				.collect();
			let close = |a: [f32; 3], b: [f32; 3], tolerance: f32| (0..3).all(|axis| (a[axis] - b[axis]).abs() < tolerance);
			for vertex in 0..level_vertices {
				let [x, y, z] = positions[vertex];
				assert_eq!(positions[vertex], source.primitives[0].positions[vertex]);
				// Doubled, stood up on the x axis, and moved ten units along it.
				assert!(close(
					positions[level_vertices + vertex],
					[10.0 + 2.0 * x, -2.0 * z, 2.0 * y],
					1e-5
				));
				assert!(close(normals[level_vertices + vertex], [0.0, -1.0, 0.0], 1e-3));
				// Mirrored across x and moved ten units the other way, still facing +z.
				assert!(close(positions[2 * level_vertices + vertex], [-10.0 - x, y, z], 1e-5));
				assert!(close(normals[2 * level_vertices + vertex], [0.0, 0.0, 1.0], 1e-3));
			}

			let record: ShaderMeshLevel = bytemuck::pod_read_unaligned(&bytes[prepared.streams.levels.clone()]);
			assert_eq!(record.meshlet_count, prepared.counts.meshlets);
			// The level casts shadows as its largest copy does, the doubled one, not as its sphere over all three.
			let source_radius = source.primitives[0].bounding_sphere[3];
			assert_eq!(record.caster_radius, 2.0 * source_radius);
			assert!(record.caster_radius < record.bounding_sphere[3]);
			assert!(record.meshlet_count as usize <= 3 * level.meshlets.len());
			let vertex_indices: Vec<u16> = bytes[prepared.streams.vertex_indices.clone()]
				.as_chunks::<2>()
				.0
				.iter()
				.map(|index| u16::from_le_bytes(*index))
				.collect();
			let triangles = bytes[prepared.streams.primitive_indices.clone()].as_chunks::<3>().0;
			let mut facing = Vec::new();
			for meshlet in bytes[prepared.streams.meshlets.clone()]
				.as_chunks::<{ std::mem::size_of::<ShaderMeshletData>() }>()
				.0
			{
				let meshlet: ShaderMeshletData = bytemuck::pod_read_unaligned(meshlet);
				assert!(meshlet.vertex_count() <= VERTEX_COUNT && meshlet.triangle_count() <= TRIANGLE_COUNT);
				let meshlet_vertices = &vertex_indices[meshlet.primitive_offset as usize..][..meshlet.vertex_count() as usize];
				let [x, y, z, radius] = meshlet.sphere(record.bounding_sphere);
				for &vertex in meshlet_vertices {
					let [vx, vy, vz] = positions[vertex as usize];
					let distance = ((vx - x).powi(2) + (vy - y).powi(2) + (vz - z).powi(2)).sqrt();
					assert!(
						distance <= radius * 1.001 + 1e-4,
						"A meshlet's sphere leaves out one of its vertices. The most likely cause is a sphere that didn't follow its copy's placement."
					);
				}
				for corners in &triangles[meshlet.triangle_offset as usize..][..meshlet.triangle_count() as usize] {
					let [a, b, c] = corners.map(|corner| positions[meshlet_vertices[corner as usize] as usize]);
					let (ab, ac): ([f32; 3], [f32; 3]) =
						(std::array::from_fn(|i| b[i] - a[i]), std::array::from_fn(|i| c[i] - a[i]));
					let face = [
						ab[1] * ac[2] - ab[2] * ac[1],
						ab[2] * ac[0] - ab[0] * ac[2],
						ab[0] * ac[1] - ab[1] * ac[0],
					];
					let normal = normals[meshlet_vertices[corners[0] as usize] as usize];
					// How squarely the triangle faces its normal, over its unscaled area so copies compare.
					facing.push((0..3).map(|i| face[i] * normal[i]).sum::<f32>());
				}
			}
			assert_eq!(facing.len(), 3 * level.triangles.len());
			// Each copy lists the source's triangles in order, so a triangle compares with the first copy's.
			let (first, others) = facing.split_at(level.triangles.len());
			for (index, &copy_facing) in others.iter().enumerate() {
				let source_facing = first[index % first.len()];
				// The simplifier leaves a few triangles edge-on to their normal, which face neither way.
				if source_facing.abs() > 1e-4 {
					assert_eq!(
						copy_facing > 0.0,
						source_facing > 0.0,
						"A copy's triangle winds the other way from the first copy's. The most likely cause is a mirrored copy whose corners were not swapped."
					);
				}
			}
		});
	}

	#[test]
	fn small_meshlets_of_neighboring_copies_pack_into_full_ones() {
		// One quad bakes into one meshlet of four vertices and two triangles, so sixteen copies fill a meshlet's 64
		// vertices.
		let source = source_of(1);
		let level = &source.primitives[0].levels[0];
		assert_eq!((level.meshlets.len(), level.vertex_count), (1, 4));
		let placements: Vec<Transform> = (0..40)
			.map(|copy| Transform::from_position(Point::new(copy as f32 * 2.0, 0.0, 0.0)))
			.collect();

		assert_eq!(source.primitives(&placements).unwrap()[0].lods.levels()[0].meshlet_count, 3);
		build(&source, &placements, 0, |prepared| {
			let bytes = prepared.staging.bytes();
			let meshlets: Vec<ShaderMeshletData> = bytes[prepared.streams.meshlets.clone()]
				.as_chunks::<{ std::mem::size_of::<ShaderMeshletData>() }>()
				.0
				.iter()
				.map(|meshlet| bytemuck::pod_read_unaligned(meshlet))
				.collect();
			let counts: Vec<(u32, u32)> = meshlets
				.iter()
				.map(|meshlet| (meshlet.vertex_count(), meshlet.triangle_count()))
				.collect();
			assert_eq!(counts, [(64, 32), (64, 32), (32, 16)]);

			// Every triangle still draws its own copy: its corners are the quad's, two units further along per copy.
			let positions: &[[f32; 3]] = bytemuck::cast_slice(&bytes[prepared.streams.positions.clone()]);
			let vertex_indices: Vec<u16> = bytes[prepared.streams.vertex_indices.clone()]
				.as_chunks::<2>()
				.0
				.iter()
				.map(|index| u16::from_le_bytes(*index))
				.collect();
			let triangles = bytes[prepared.streams.primitive_indices.clone()].as_chunks::<3>().0;
			let record: ShaderMeshLevel = bytemuck::pod_read_unaligned(&bytes[prepared.streams.levels.clone()]);
			let mut copy = 0;
			for meshlet in &meshlets {
				let meshlet_vertices = &vertex_indices[meshlet.primitive_offset as usize..][..meshlet.vertex_count() as usize];
				let [x, _, _, radius] = meshlet.sphere(record.bounding_sphere);
				for (triangle, corners) in triangles[meshlet.triangle_offset as usize..][..meshlet.triangle_count() as usize]
					.iter()
					.enumerate()
				{
					for &corner in corners {
						let [vertex_x, ..] = positions[meshlet_vertices[corner as usize] as usize];
						let copy_x = (copy + triangle / 2) as f32 * 2.0;
						assert!(
							(copy_x..=copy_x + 1.0).contains(&vertex_x),
							"A packed triangle names a vertex of another copy. The most likely cause is corners that kept their source meshlet's numbering."
						);
						assert!((vertex_x - x).abs() <= radius * 1.001 + 1e-4);
					}
				}
				copy += meshlet.triangle_count() as usize / 2;
			}
			assert_eq!(copy, 40);
		});
	}
}
