use super::*;

/// Returns a typed resource stream count after validating its complete byte layout.
pub(crate) fn validated_stream_count(stream: &ResourceStream, name: &str, expected_stride: usize) -> Option<usize> {
	if stream.stride != expected_stride || expected_stride == 0 || !stream.size.is_multiple_of(expected_stride) {
		log::error!(
			"Mesh {name} stream has an invalid layout. The most likely cause is malformed or incompatible baked stream metadata; expected stride {expected_stride}, found stride {} and size {}.",
			stream.stride,
			stream.size
		);
		return None;
	}
	Some(stream.size / expected_stride)
}

/// Computes one prepared stream size without accepting count multiplication overflow.
pub(crate) fn checked_mesh_byte_size(element_count: usize, stride: usize, name: &str) -> Option<usize> {
	let Some(byte_count) = element_count.checked_mul(stride) else {
		log::error!(
			"Prepared mesh {name} byte count overflowed. The most likely cause is malformed metadata containing an invalid element count."
		);
		return None;
	};
	Some(byte_count)
}

/// Narrows generated indices only after proving that every value addresses an available vertex.
pub(crate) fn validated_generated_indices(indices: &[u32], vertex_count: usize) -> Option<Vec<u16>> {
	let mut validated = Vec::new();
	if validated.try_reserve_exact(indices.len()).is_err() {
		log::error!(
			"Generated mesh indices could not be allocated. The most likely cause is that the generator returned an impractically large index list."
		);
		return None;
	}
	for (index_position, &index) in indices.iter().enumerate() {
		if index as usize >= vertex_count {
			log::error!(
				"Generated mesh index {index_position} references missing vertex {index}. The most likely cause is that the generator returned an index outside its {vertex_count} positions."
			);
			return None;
		}
		let Ok(index) = u16::try_from(index) else {
			log::error!(
				"Generated mesh index {index_position} exceeds the u16 vertex-index limit. The most likely cause is that one generated primitive contains more than 65536 addressable vertices."
			);
			return None;
		};
		validated.push(index);
	}
	Some(validated)
}

/// Confirms that primitive-local metadata consumes exactly the aggregate streams prepared for upload.
pub(crate) fn prepared_mesh_counts_match(expected: PreparedGpuMeshCounts, actual: PreparedGpuMeshCounts) -> bool {
	if actual == expected {
		return true;
	}
	log::error!(
		"Prepared primitive counts do not match the aggregate mesh streams: expected {expected:?}, found {actual:?}. The most likely cause is inconsistent or overlapping baked primitive ranges."
	);
	false
}
/// Rejects meshlet references that would address vertices or triangle lanes outside their primitive-local ranges.
// Keep all cross-stream bounds checks together so each primitive is validated against one coherent snapshot.
#[allow(clippy::too_many_lines)]
pub(crate) fn validate_loaded_mesh_indices(
	primitives: &[LoadedPrimitiveValidation],
	loaded: &resource_management::resource::ReadTargets<'_>,
) -> Result<(), ()> {
	let Some(vertex_indices) = loaded.stream("VertexIndices") else {
		log::error!(
			"Loaded mesh data is missing its vertex-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(triangle_indices) = loaded.stream("MeshletIndices") else {
		log::error!(
			"Loaded mesh data is missing its meshlet triangle-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(meshlets) = loaded.stream("Meshlets") else {
		log::error!(
			"Loaded mesh data is missing its meshlet stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let vertex_indices = vertex_indices.buffer();
	let triangle_indices = triangle_indices.buffer();
	let meshlets = meshlets.buffer();

	for (primitive_index, primitive) in primitives.iter().enumerate() {
		let Some(vertex_stream) = primitive.vertex_indices.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its vertex-index stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		let Some(triangle_stream) = primitive.triangle_indices.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet triangle-index stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		let Some(meshlet_stream) = primitive.meshlets.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		if validated_stream_count(vertex_stream, "primitive meshlet vertex-index", 2).is_none()
			|| validated_stream_count(triangle_stream, "primitive meshlet triangle-index", 1).is_none()
			|| validated_stream_count(meshlet_stream, "primitive meshlet", RESOURCE_MESHLET_STRIDE).is_none()
		{
			return Err(());
		}
		let mut vertex_cursor = vertex_stream.offset;
		let mut triangle_cursor = triangle_stream.offset;
		let Some(vertex_end) = vertex_stream.offset.checked_add(vertex_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} vertex-index range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(triangle_stream_end) = triangle_stream.offset.checked_add(triangle_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} triangle-index range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(meshlet_end) = meshlet_stream.offset.checked_add(meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		if vertex_end > vertex_indices.len() || triangle_stream_end > triangle_indices.len() {
			log::error!(
				"Mesh primitive {primitive_index} index range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate index streams."
			);
			return Err(());
		}
		let Some(meshlet_bytes) = meshlets.get(meshlet_stream.offset..meshlet_end) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};

		for meshlet_bytes in meshlet_bytes.chunks_exact(RESOURCE_MESHLET_STRIDE) {
			let meshlet = read_resource_meshlet(meshlet_bytes);
			let Some(vertex_byte_count) = (meshlet.primitive_count as usize).checked_mul(2) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index count overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(next_vertex_cursor) = vertex_cursor.checked_add(vertex_byte_count) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index range overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(meshlet_vertices) = vertex_indices.get(vertex_cursor..next_vertex_cursor) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index range is out of bounds. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			for bytes in meshlet_vertices.chunks_exact(2) {
				let index = u16::from_ne_bytes([bytes[0], bytes[1]]);
				if index as u32 >= primitive.vertex_count {
					log::error!(
						"Mesh primitive {primitive_index} references vertex {index} outside its {} vertices. The most likely cause is corrupted meshlet vertex indices.",
						primitive.vertex_count
					);
					return Err(());
				}
			}
			let Some(triangle_byte_count) = (meshlet.triangle_count as usize).checked_mul(3) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index count overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(triangle_end) = triangle_cursor.checked_add(triangle_byte_count) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index range overflows. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			let Some(meshlet_triangles) = triangle_indices.get(triangle_cursor..triangle_end) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index range is out of bounds. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			for &index in meshlet_triangles {
				if index as u32 >= meshlet.primitive_count {
					log::error!(
						"Mesh primitive {primitive_index} has a triangle index outside its meshlet vertex range. The most likely cause is corrupted meshlet triangle data."
					);
					return Err(());
				}
			}
			vertex_cursor = next_vertex_cursor;
			triangle_cursor = triangle_end;
			if vertex_cursor > vertex_end || triangle_cursor > triangle_stream_end {
				log::error!(
					"Mesh primitive {primitive_index} meshlet counts exceed its declared index ranges. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			}
		}
		if vertex_cursor != vertex_end || triangle_cursor != triangle_stream_end {
			log::error!(
				"Mesh primitive {primitive_index} meshlet counts do not consume its declared index ranges. The most likely cause is inconsistent meshlet metadata."
			);
			return Err(());
		}
	}
	Ok(())
}
