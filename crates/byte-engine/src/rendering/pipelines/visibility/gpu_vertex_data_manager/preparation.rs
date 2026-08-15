use super::*;

mod generated_mesh;
mod meshlet_conversion;
mod resource_mesh;
mod skinning_validation;
mod staging_layout;
mod stream_index_validation;

pub(super) use meshlet_conversion::{build_prepared_resource_primitives, read_resource_meshlet};
pub(super) use skinning_validation::{
	validate_loaded_skin_joints, validate_skinning_primitive_stream, LoadedPrimitiveValidation, SKINNING_JOINTS_STRIDE,
	SKINNING_NORMAL_STRIDE, SKINNING_POSITION_STRIDE, SKINNING_WEIGHTS_STRIDE,
};
pub(super) use staging_layout::pack_f32_normals;
pub(crate) use staging_layout::{encode_octahedral_unit_vector, pack_f32_uvs};
pub(super) use staging_layout::{
	take_range, take_range_aligned, UvSourceFormat, NORMAL_F32_SOURCE_STRIDE, RESOURCE_MESHLET_STRIDE, UV_F16_SOURCE_STRIDE,
	UV_F32_SOURCE_STRIDE,
};
pub(super) use stream_index_validation::{checked_mesh_byte_size, validate_loaded_mesh_indices, validated_stream_count};
pub(crate) use stream_index_validation::{prepared_mesh_counts_match, validated_generated_indices};

/// The `PreparedGpuMeshCounts` struct defines the aggregate geometry contract that primitive metadata must satisfy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedGpuMeshCounts {
	pub(super) vertices: usize,
	pub(super) primitive_indices: usize,
	pub(super) triangles: usize,
	pub(super) meshlets: usize,
	pub(super) skinning_vertices: usize,
}

/// The `PreparedGpuMesh` struct retains validated mesh ranges in their leased GPU upload-buffer region.
///
/// Pass it to [`GPUVertexDataManager::write_prepared_gpu_mesh_data_and_return_mesh_object`] when its lease is ready.
pub(crate) struct PreparedGpuMesh {
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) streams: PreparedGpuMeshStreams,
	pub(super) primitives: Vec<PreparedGpuMeshPrimitive>,
	pub(super) vertex_count: usize,
	pub(super) primitive_count: usize,
	pub(super) triangle_count: usize,
	pub(super) meshlet_count: usize,
	pub(super) skinning_vertex_count: usize,
}

/// The `PreparedGpuMeshStreams` struct locates transfer-ready streams in one owned byte backing.
pub(super) struct PreparedGpuMeshStreams {
	pub(super) positions: std::ops::Range<usize>,
	pub(super) normals: std::ops::Range<usize>,
	pub(super) uvs: std::ops::Range<usize>,
	pub(super) vertex_indices: std::ops::Range<usize>,
	pub(super) primitive_indices: std::ops::Range<usize>,
	pub(super) meshlets: std::ops::Range<usize>,
	pub(super) skinning_normals: Option<std::ops::Range<usize>>,
	pub(super) skinning_joints: Option<std::ops::Range<usize>>,
	pub(super) skinning_weights: Option<std::ops::Range<usize>>,
}

/// The `PreparedGpuMeshPrimitive` struct retains one primitive's relative GPU ranges and optional skinning copies.
pub(super) struct PreparedGpuMeshPrimitive {
	pub(super) mesh: MeshPrimitive,
	pub(super) skinning: Option<PreparedGpuSkinningCopy>,
}

/// The `PreparedGpuSkinningCopy` struct locates one primitive in the prepared aggregate skinning streams.
pub(super) struct PreparedGpuSkinningCopy {
	pub(super) positions: std::ops::Range<usize>,
	pub(super) normals: std::ops::Range<usize>,
	pub(super) joints: std::ops::Range<usize>,
	pub(super) weights: std::ops::Range<usize>,
}

impl PreparedGpuMesh {
	/// Returns the number of render-facing primitives produced by this prepared mesh.
	///
	/// Use this before GPU recording to validate separately retained material and skin metadata.
	pub(crate) fn render_primitive_count(&self) -> usize {
		self.primitives.len()
	}

	/// Returns the upload-buffer lease after its GPU copies have been recorded.
	pub(crate) fn into_staging(self) -> super::upload_staging::StagingLease {
		self.staging
	}
}
