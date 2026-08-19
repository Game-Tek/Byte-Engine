use std::collections::hash_map::Entry;

use ghi::{command_buffer::CommandBufferRecording as _, context::ContextCreate as _};
use resource_management::{
	resources::mesh::{Mesh, Primitive as ResourcePrimitive},
	types::{Stream as ResourceStream, Streams, VertexSemantics},
	Reference,
};
use utils::as_byte_slice;

pub(super) use super::upload_staging;
use crate::rendering::{
	mesh::generator::MeshGenerator,
	pipelines::visibility::{
		RuntimeUnitVector, RuntimeVertexNormal, RuntimeVertexUv, ShaderMeshletData, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES,
		MAX_TRIANGLES, MAX_VERTICES, VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE,
	},
	pipelines::visibility::{TRIANGLE_COUNT, VERTEX_COUNT},
};

mod data;
mod preparation;
mod residency;

pub use data::*;
pub(crate) use preparation::*;
#[cfg(test)]
pub(super) use preparation::{pack_f32_uvs, prepared_mesh_counts_match, validated_generated_indices, PreparedGpuMeshCounts};
#[cfg(test)]
pub(super) use residency::checked_visibility_capacity;

/// The `GPUVertexDataManager` is responsible for managing the vertex data buffers used in the visibility pipeline.
/// It tracks buffer offsets and counts for various resources, and provides handles to the vertex data buffers.
/// It performs uploads to the GPU of mesh resources.
#[derive(Clone)]
pub(crate) struct GPUVertexDataManager {
	/// Tracks buffer offsets and counts for various resources.
	visibility_info: VisibilityInfo,
	/// Tracks the compact immutable vertex ranges consumed by GPU skinning.
	skinning_source_vertex_count: u32,

	/// Vertex positions buffer for rendered meshes.
	pub vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex normals buffer for rendered meshes, octahedrally encoded as two UNORM16 components.
	pub vertex_normals_buffer: ghi::BufferHandle<[RuntimeVertexNormal; MAX_VERTICES]>,
	/// Vertex UVs buffer for rendered meshes, packed in the visibility runtime format.
	pub vertex_uvs_buffer: ghi::BufferHandle<[RuntimeVertexUv; MAX_VERTICES]>,
	/// Indices laid out as indices into the vertex buffers
	pub vertex_indices_buffer: ghi::BufferHandle<[u16; MAX_PRIMITIVE_TRIANGLES]>,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	pub primitive_indices_buffer: ghi::BufferHandle<[[u8; 3]; MAX_TRIANGLES]>,
	/// Buffer that stores the meshlet records.
	pub meshlets_data_buffer: ghi::BufferHandle<[ShaderMeshletData; MAX_MESHLETS]>,
	/// Bind-pose positions packed only for primitives that participate in GPU skinning.
	pub(super) skinning_rest_positions_buffer: ghi::BufferHandle<[[f32; 3]; MAX_VERTICES]>,
	/// Bind-pose normals packed only for primitives that participate in GPU skinning.
	pub(super) skinning_rest_normals_buffer: ghi::BufferHandle<[[f32; 3]; MAX_VERTICES]>,
	/// Four palette-local u16 joint indices packed into eight bytes per skinned vertex.
	pub(super) skinning_joints_buffer: ghi::BufferHandle<[[u16; 4]; MAX_VERTICES]>,
	/// Four normalized linear-blend weights packed beside each skinned vertex's joints.
	pub(super) skinning_weights_buffer: ghi::BufferHandle<[[f32; 4]; MAX_VERTICES]>,
}

#[cfg(test)]
mod tests {
	use super::{
		checked_visibility_capacity, pack_f32_uvs, prepared_mesh_counts_match, validated_generated_indices, PreparedGpuMesh,
		PreparedGpuMeshCounts,
	};
	use crate::rendering::mesh::generator::BoxMeshGenerator;

	#[test]
	fn generated_mesh_preparation_owns_complete_transfer_data() {
		let bytes = Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice());
		let staging = crate::rendering::pipelines::visibility::upload_staging::UploadStagingArena::new(bytes);
		let executor = resource_management::r#async::Executor::new().expect("mesh preparation test executor");
		let prepared = executor
			.block_on(PreparedGpuMesh::prepare_generated_mesh(&BoxMeshGenerator::new(), staging))
			.expect("The built-in box should produce valid visibility geometry.");

		assert_eq!(prepared.vertex_count, 24);
		assert_eq!(prepared.primitive_count, 24);
		assert_eq!(prepared.triangle_count, 12);
		assert_eq!(prepared.meshlet_count, 1);
		assert_eq!(prepared.primitives.len(), 1);
		assert_eq!(prepared.render_primitive_count(), 1);
		assert_eq!(prepared.primitives[0].mesh.meshlet_count, 1);
	}

	#[test]
	fn generated_indices_are_checked_before_u16_narrowing() {
		assert_eq!(validated_generated_indices(&[0, 2, 1], 3), Some(vec![0, 2, 1]));
		assert!(validated_generated_indices(&[3], 3).is_none());
		assert!(validated_generated_indices(&[u16::MAX as u32 + 1], u16::MAX as usize + 2).is_none());
	}

	#[test]
	fn visibility_capacity_rejects_only_the_overflowing_upload() {
		assert_eq!(checked_visibility_capacity(3, 2, 5, "test"), Some(5));
		assert_eq!(checked_visibility_capacity(3, 3, 5, "test"), None);
		assert_eq!(checked_visibility_capacity(1, usize::MAX, usize::MAX, "test"), None);
	}

	#[test]
	fn primitive_counts_must_consume_the_aggregate_streams() {
		let expected = PreparedGpuMeshCounts {
			vertices: 6,
			primitive_indices: 6,
			triangles: 2,
			meshlets: 1,
			skinning_vertices: 0,
		};

		assert!(prepared_mesh_counts_match(expected, expected));
		assert!(!prepared_mesh_counts_match(
			expected,
			PreparedGpuMeshCounts {
				primitive_indices: 5,
				..expected
			}
		));
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
