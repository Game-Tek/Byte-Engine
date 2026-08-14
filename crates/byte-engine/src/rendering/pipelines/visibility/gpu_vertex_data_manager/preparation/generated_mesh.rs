use super::*;

impl PreparedGpuMesh {
	/// Builds transfer-ready owned geometry from a generated mesh without borrowing GPU recording state.
	///
	/// The returned mesh keeps its staging lease until the transfer frame completes.
	pub(crate) async fn prepare_generated_mesh(
		generator: &dyn MeshGenerator,
		upload_staging: std::sync::Arc<super::upload_staging::UploadStagingArena>,
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
		let indices = validated_generated_indices(generator.indices().as_ref(), positions.len())?;

		let (vertex_indices, primitive_indices, meshlets) =
			GPUVertexDataManager::build_generated_meshlets(&indices, positions.as_ref()).ok()?;
		let Some(meshlet_count) = u32::try_from(meshlets.len()).ok() else {
			log::error!(
				"Generated mesh has too many meshlets. The most likely cause is that the generator exceeded the visibility meshlet metadata limit."
			);
			return None;
		};
		let sizes = [
			std::mem::size_of_val(positions.as_ref()),
			normals.len().checked_mul(VERTEX_NORMAL_BUFFER_STRIDE as usize)?,
			uvs.len().checked_mul(VERTEX_UV_BUFFER_STRIDE as usize)?,
			std::mem::size_of_val(vertex_indices.as_slice()),
			std::mem::size_of_val(primitive_indices.as_slice()),
			std::mem::size_of_val(meshlets.as_slice()),
		];
		let mut cursor = 0usize;
		let position_range = take_range_aligned(&mut cursor, sizes[0], 4);
		let normal_range = take_range_aligned(&mut cursor, sizes[1], 4);
		let uv_range = take_range_aligned(&mut cursor, sizes[2], 4);
		let vertex_index_range = take_range_aligned(&mut cursor, sizes[3], 4);
		let primitive_index_range = take_range_aligned(&mut cursor, sizes[4], 4);
		let meshlet_range = take_range_aligned(&mut cursor, sizes[5], 4);
		let backing_size = cursor;

		let mut staging = upload_staging.allocate(backing_size, 256).await.or_else(|| {
			log::error!(
				"Generated mesh exceeds the GPU upload arena. The most likely cause is that its geometry is larger than the configured upload capacity."
			);
			None
		})?;
		let backing = staging.bytes_mut();
		backing[position_range.clone()].copy_from_slice(as_byte_slice(positions.as_ref()));
		for (destination, &normal) in backing[normal_range.clone()]
			.chunks_exact_mut(VERTEX_NORMAL_BUFFER_STRIDE as usize)
			.zip(normals.iter())
		{
			let encoded = encode_octahedral_normal(normal);
			destination[..2].copy_from_slice(&encoded[0].to_ne_bytes());
			destination[2..].copy_from_slice(&encoded[1].to_ne_bytes());
		}
		for (destination, &(u, v)) in backing[uv_range.clone()]
			.chunks_exact_mut(VERTEX_UV_BUFFER_STRIDE as usize)
			.zip(uvs.iter())
		{
			destination[..2].copy_from_slice(&half::f16::from_f32(u).to_bits().to_ne_bytes());
			destination[2..].copy_from_slice(&half::f16::from_f32(v).to_bits().to_ne_bytes());
		}
		backing[vertex_index_range.clone()].copy_from_slice(as_byte_slice(vertex_indices.as_slice()));
		backing[primitive_index_range.clone()].copy_from_slice(as_byte_slice(primitive_indices.as_slice()));
		backing[meshlet_range.clone()].copy_from_slice(as_byte_slice(meshlets.as_slice()));

		Some(Self {
			staging,
			streams: PreparedGpuMeshStreams {
				positions: position_range,
				normals: normal_range,
				uvs: uv_range,
				vertex_indices: vertex_index_range,
				primitive_indices: primitive_index_range,
				meshlets: meshlet_range,
				skinning_normals: None,
				skinning_joints: None,
				skinning_weights: None,
			},
			primitives: vec![PreparedGpuMeshPrimitive {
				mesh: MeshPrimitive {
					meshlet_count,
					meshlet_offset: 0,
					vertex_offset: 0,
					primitive_offset: 0,
					triangle_offset: 0,
					skinning_source_vertex_offset: None,
					skinning_vertex_count: 0,
				},
				skinning: None,
			}],
			vertex_count: positions.len(),
			primitive_count: vertex_indices.len(),
			triangle_count: primitive_indices.len(),
			meshlet_count: meshlets.len(),
			skinning_vertex_count: 0,
		})
	}
}
