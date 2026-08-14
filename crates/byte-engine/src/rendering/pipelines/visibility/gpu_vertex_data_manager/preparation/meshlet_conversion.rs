use super::*;

/// Converts validated resource metadata and loaded meshlet bytes into transfer-ready primitive records.
pub(crate) fn build_prepared_resource_primitives(
	mesh: &Mesh,
	meshlet_bytes: &[u8],
	expected_counts: PreparedGpuMeshCounts,
) -> Option<(Vec<PreparedGpuMeshPrimitive>, Vec<ShaderMeshletData>)> {
	let mut primitives = Vec::new();
	if primitives.try_reserve_exact(mesh.primitives.len()).is_err() {
		log::error!(
			"Prepared primitive metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large primitive list."
		);
		return None;
	}
	let mut runtime_meshlets = Vec::new();
	if runtime_meshlets.try_reserve_exact(expected_counts.meshlets).is_err() {
		log::error!(
			"Prepared meshlet metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large meshlet list."
		);
		return None;
	}
	let mut vertex_offset = 0usize;
	let mut primitive_offset = 0usize;
	let mut triangle_offset = 0usize;
	let mut skinning_vertex_offset = 0usize;

	for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
		let (Ok(meshlet_offset), Ok(render_vertex_offset), Ok(primitive_meshlet_offset), Ok(primitive_triangle_offset)) = (
			u32::try_from(runtime_meshlets.len()),
			u32::try_from(vertex_offset),
			u32::try_from(primitive_offset),
			u32::try_from(triangle_offset),
		) else {
			log::error!(
				"Mesh primitive {primitive_index} offsets exceed their GPU representation. The most likely cause is overflowing baked primitive metadata."
			);
			return None;
		};
		let Some(meshlet_stream) = primitive.meshlet_stream() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet stream. The most likely cause is incomplete baked primitive metadata."
			);
			return None;
		};
		validated_stream_count(meshlet_stream, "primitive meshlet", RESOURCE_MESHLET_STRIDE)?;
		let Some(meshlet_end) = meshlet_stream.offset.checked_add(meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range overflows. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		let Some(source) = meshlet_bytes.get(meshlet_stream.offset..meshlet_end) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate meshlet stream."
			);
			return None;
		};
		let mut local_primitive_offset = 0u32;
		let mut local_triangle_offset = 0u32;

		for bytes in source.chunks_exact(RESOURCE_MESHLET_STRIDE) {
			let meshlet = read_resource_meshlet(bytes);
			runtime_meshlets.push(ShaderMeshletData {
				primitive_offset: local_primitive_offset,
				triangle_offset: local_triangle_offset,
				primitive_count: meshlet.primitive_count,
				triangle_count: meshlet.triangle_count,
				center_radius: meshlet.center_radius,
				cone_apex_cutoff: meshlet.cone_apex_cutoff,
				cone_axis: encode_octahedral_normal((meshlet.cone_axis[0], meshlet.cone_axis[1], meshlet.cone_axis[2])),
			});
			let (Some(next_local_primitive_offset), Some(next_local_triangle_offset)) = (
				local_primitive_offset.checked_add(meshlet.primitive_count),
				local_triangle_offset.checked_add(meshlet.triangle_count),
			) else {
				log::error!(
					"Mesh primitive {primitive_index} meshlet counts overflow. The most likely cause is corrupted baked meshlet metadata."
				);
				return None;
			};
			let (Some(next_primitive_offset), Some(next_triangle_offset)) = (
				primitive_offset.checked_add(meshlet.primitive_count as usize),
				triangle_offset.checked_add(meshlet.triangle_count as usize),
			) else {
				log::error!("Mesh aggregate index counts overflow. The most likely cause is corrupted baked meshlet metadata.");
				return None;
			};
			local_primitive_offset = next_local_primitive_offset;
			local_triangle_offset = next_local_triangle_offset;
			primitive_offset = next_primitive_offset;
			triangle_offset = next_triangle_offset;
		}

		let (relative_skinning_offset, skinning) = if primitive.skin.is_some() {
			let Some(positions) = primitive.stream(Streams::Vertices(VertexSemantics::Position)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its position stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(normals) = primitive.stream(Streams::Vertices(VertexSemantics::Normal)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its normal stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(joints) = primitive.stream(Streams::Vertices(VertexSemantics::Joints)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its joint-index stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(weights) = primitive.stream(Streams::Vertices(VertexSemantics::Weights)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its vertex-weight stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Ok(relative_offset) = u32::try_from(skinning_vertex_offset) else {
				log::error!(
					"Skinned primitive {primitive_index} source offset exceeds its GPU representation. The most likely cause is overflowing baked primitive metadata."
				);
				return None;
			};
			let Some(next_skinning_vertex_offset) = skinning_vertex_offset.checked_add(primitive.vertex_count as usize) else {
				log::error!(
					"Skinned primitive {primitive_index} vertex count overflows. The most likely cause is corrupted baked primitive metadata."
				);
				return None;
			};
			skinning_vertex_offset = next_skinning_vertex_offset;
			let checked_range = |stream: &ResourceStream, semantic: &str| {
				let Some(end) = stream.offset.checked_add(stream.size) else {
					log::error!(
						"Skinned primitive {primitive_index} {semantic} range overflows. The most likely cause is corrupted baked primitive metadata."
					);
					return None;
				};
				Some(stream.offset..end)
			};

			(
				Some(relative_offset),
				Some(PreparedGpuSkinningCopy {
					positions: checked_range(positions, "position")?,
					normals: checked_range(normals, "normal")?,
					joints: checked_range(joints, "joint-index")?,
					weights: checked_range(weights, "vertex-weight")?,
				}),
			)
		} else {
			(None, None)
		};

		let Ok(meshlet_count) = u32::try_from(source.len() / RESOURCE_MESHLET_STRIDE) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet count exceeds its GPU representation. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		let Some(next_vertex_offset) = vertex_offset.checked_add(primitive.vertex_count as usize) else {
			log::error!(
				"Mesh primitive {primitive_index} vertex count overflows. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		primitives.push(PreparedGpuMeshPrimitive {
			mesh: MeshPrimitive {
				meshlet_count,
				meshlet_offset,
				vertex_offset: render_vertex_offset,
				primitive_offset: primitive_meshlet_offset,
				triangle_offset: primitive_triangle_offset,
				skinning_source_vertex_offset: relative_skinning_offset,
				skinning_vertex_count: relative_skinning_offset.map_or(0, |_| primitive.vertex_count),
			},
			skinning,
		});
		vertex_offset = next_vertex_offset;
	}

	let actual_counts = PreparedGpuMeshCounts {
		vertices: vertex_offset,
		primitive_indices: primitive_offset,
		triangles: triangle_offset,
		meshlets: runtime_meshlets.len(),
		skinning_vertices: skinning_vertex_offset,
	};
	if !prepared_mesh_counts_match(expected_counts, actual_counts) {
		return None;
	}

	Some((primitives, runtime_meshlets))
}
/// The `ResourceMeshletData` struct carries meshlet metadata decoded from the packed resource stream.
#[derive(Clone, Copy)]
pub(crate) struct ResourceMeshletData {
	pub(crate) primitive_count: u32,
	pub(crate) triangle_count: u32,
	pub(crate) center_radius: [f32; 4],
	pub(crate) cone_apex_cutoff: [f32; 4],
	pub(crate) cone_axis: [f32; 4],
}

/// Decodes one packed meshlet record without assuming the resource stream is naturally aligned.
pub(crate) fn read_resource_meshlet(bytes: &[u8]) -> ResourceMeshletData {
	debug_assert_eq!(bytes.len(), RESOURCE_MESHLET_STRIDE);

	ResourceMeshletData {
		primitive_count: bytes[0] as u32,
		triangle_count: bytes[1] as u32,
		center_radius: read_f32x4(bytes, 4),
		cone_apex_cutoff: read_f32x4(bytes, 20),
		cone_axis: read_f32x4(bytes, 36),
	}
}

fn read_f32x4(bytes: &[u8], offset: usize) -> [f32; 4] {
	[
		read_f32(bytes, offset),
		read_f32(bytes, offset + 4),
		read_f32(bytes, offset + 8),
		read_f32(bytes, offset + 12),
	]
}
fn read_f32(bytes: &[u8], offset: usize) -> f32 {
	f32::from_le_bytes(
		bytes[offset..offset + 4].try_into().expect(
			"Packed meshlet record is truncated. The most likely cause is that the meshlet stream stride is incorrect.",
		),
	)
}
