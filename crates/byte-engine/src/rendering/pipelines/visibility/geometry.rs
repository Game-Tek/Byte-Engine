//! GPU geometry storage for the visibility pipeline: parallel vertex streams, meshlet records, and skinning sources.
//!
//! [`PreparedMesh`] is built on a worker without touching GPU state, then [`GeometryBuffers::write_mesh`] appends
//! it to the fixed-capacity buffers on the render thread and returns the [`MeshData`] ranges the scene needs.

mod prepare;

use std::sync::Arc;

use ghi::{command_buffer::CommandBufferRecording as _, context::ContextCreate as _};
use resource_management::resources::skeleton::SkinBinding;

pub(crate) use self::prepare::{PreparedMesh, encode_octahedral_unit_vector};
use super::layout::{
	MAX_ADDRESSABLE_MESHLETS, MAX_INSTANCE_MESHLETS, RuntimeVertexNormal, RuntimeVertexUv, ShaderMeshletData,
	VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE,
};

/// Byte strides of the compact bind-pose streams consumed by GPU skinning.
pub(crate) const SKINNING_POSITION_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_NORMAL_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_JOINTS_STRIDE: usize = std::mem::size_of::<[u16; 4]>();
pub(crate) const SKINNING_WEIGHTS_STRIDE: usize = std::mem::size_of::<[f32; 4]>();

/// The `GeometryCapacity` struct sizes the scene-wide geometry buffers that every resident mesh is appended to.
///
/// All loaded meshes share these buffers, so size them for the most geometry an application keeps resident at once.
/// A mesh that does not fit is rejected when it is uploaded. Pass the capacity to
/// [`super::VisibilityPipelineSettings::with_geometry_capacity`], or set it with the startup parameters under
/// [`super::GEOMETRY_CAPACITY_PARAMETER_PREFIX`], such as `render.geometry.triangle-capacity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryCapacity {
	/// Vertices across every mesh. Each vertex stores a position, a normal, and a UV.
	pub vertices: u32,
	/// Meshlet-local vertex indices across every mesh. Each meshlet stores one per vertex it references.
	pub vertex_indices: u32,
	pub triangles: u32,
	pub meshlets: u32,
	/// Bind-pose vertices of meshes that use GPU skinning.
	pub skinning_vertices: u32,
}

impl Default for GeometryCapacity {
	fn default() -> Self {
		Self {
			vertices: 65536 * 4,
			vertex_indices: 65536 * 4,
			triangles: 65536 * 4,
			meshlets: 1024 * 4,
			skinning_vertices: 65536 * 4,
		}
	}
}

impl GeometryCapacity {
	/// Checks that every stream can be allocated and that the visibility buffer can address every meshlet.
	pub(crate) fn validate(&self) -> Result<(), String> {
		let streams = [
			("vertex", self.vertices),
			("vertex index", self.vertex_indices),
			("triangle", self.triangles),
			("meshlet", self.meshlets),
			("skinning vertex", self.skinning_vertices),
		];
		if let Some((stream, _)) = streams.iter().find(|(_, capacity)| *capacity == 0) {
			return Err(format!(
				"Geometry capacity was not set. The most likely cause is that the {stream} capacity is zero; every geometry buffer needs at least one element."
			));
		}
		if self.meshlets as usize > MAX_ADDRESSABLE_MESHLETS {
			return Err(format!(
				"Geometry capacity was not set. The most likely cause is that {} meshlets exceed the {MAX_ADDRESSABLE_MESHLETS} the visibility buffer can address.",
				self.meshlets
			));
		}
		Ok(())
	}
}

/// The `GeometryCounts` struct measures one mesh, or the whole resident set, in elements of each geometry stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GeometryCounts {
	pub(crate) vertices: u32,
	pub(crate) primitive_indices: u32,
	pub(crate) triangles: u32,
	pub(crate) meshlets: u32,
	pub(crate) skinning_vertices: u32,
}

impl GeometryCounts {
	/// Returns the counts after appending `mesh`, or `None` when any stream would exceed its GPU buffer.
	fn grown_by(self, mesh: GeometryCounts, capacity: &GeometryCapacity) -> Option<Self> {
		Some(Self {
			vertices: checked_capacity(self.vertices, mesh.vertices, capacity.vertices, "vertex")?,
			primitive_indices: checked_capacity(
				self.primitive_indices,
				mesh.primitive_indices,
				capacity.vertex_indices,
				"vertex index",
			)?,
			triangles: checked_capacity(self.triangles, mesh.triangles, capacity.triangles, "triangle")?,
			meshlets: checked_capacity(self.meshlets, mesh.meshlets, capacity.meshlets, "meshlet")?,
			skinning_vertices: checked_capacity(
				self.skinning_vertices,
				mesh.skinning_vertices,
				capacity.skinning_vertices,
				"skinning vertex",
			)?,
		})
	}
}

/// Adds one mesh count, rejecting only the upload that would overflow the buffer's configured capacity.
fn checked_capacity(current: u32, additional: u32, capacity: u32, element: &str) -> Option<u32> {
	let total = current as u64 + additional as u64;
	if total > capacity as u64 {
		log::error!(
			"Visibility {element} buffer capacity exceeded. The most likely cause is that the resident scene needs {total} {element} elements but the geometry capacity is {capacity}; raise it with VisibilityPipelineSettings::with_geometry_capacity or the render.geometry startup parameters."
		);
		return None;
	}
	Some(total as u32)
}

/// The `MeshData` struct locates one resident mesh in the visibility geometry buffers.
#[derive(Debug, Clone)]
pub(crate) struct MeshData {
	pub(crate) primitives: Vec<MeshPrimitive>,
	/// Number of global pose matrices expected from a renderable using this mesh.
	pub(crate) skeleton_node_count: u32,
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	/// Base triangle in the primitive-index buffer, stored as index / 3.
	pub(crate) triangle_offset: u32,
	pub(crate) meshlet_offset: u32,
}

/// The `MeshPrimitive` struct locates one primitive's geometry, material slot, and optional skin inside its mesh.
#[derive(Debug, Clone)]
pub(crate) struct MeshPrimitive {
	pub(crate) material_index: u32,
	pub(crate) meshlet_count: u32,
	/// Offsets relative to the owning mesh.
	pub(crate) meshlet_offset: u32,
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	pub(crate) triangle_offset: u32,
	/// First vertex in the compact bind-pose skinning buffers, once resident.
	pub(crate) skinning_source_vertex_offset: Option<u32>,
	pub(crate) skinning_vertex_count: u32,
	pub(crate) skin: Option<Arc<SkinBinding>>,
}

/// The `GeometryHandles` struct names the GPU streams every visibility stage reads.
///
/// Handles are stable for the life of the pipeline, so the render thread copies this value at construction
/// while the loader retains the allocation cursors that decide where the next mesh lands.
#[derive(Clone, Copy)]
pub(crate) struct GeometryHandles {
	/// The element count of every stream below.
	pub(crate) capacity: GeometryCapacity,
	pub(crate) vertex_positions: ghi::BufferHandle<[[f32; 3]]>,
	/// Octahedrally encoded normals, two UNORM16 components each.
	pub(crate) vertex_normals: ghi::BufferHandle<[RuntimeVertexNormal]>,
	pub(crate) vertex_uvs: ghi::BufferHandle<[RuntimeVertexUv]>,
	/// Meshlet-local indices into the vertex buffers.
	pub(crate) vertex_indices: ghi::BufferHandle<[u16]>,
	/// Triangles as indices into `vertex_indices`.
	pub(crate) primitive_indices: ghi::BufferHandle<[[u8; 3]]>,
	pub(crate) meshlets: ghi::BufferHandle<[ShaderMeshletData]>,
	/// Bind-pose attributes packed only for primitives that participate in GPU skinning.
	pub(crate) skinning_rest_positions: ghi::BufferHandle<[[f32; 3]]>,
	pub(crate) skinning_rest_normals: ghi::BufferHandle<[[f32; 3]]>,
	pub(crate) skinning_joints: ghi::BufferHandle<[[u16; 4]]>,
	pub(crate) skinning_weights: ghi::BufferHandle<[[f32; 4]]>,
}

/// The `GeometryBuffers` struct appends meshes to the geometry streams in the order they become resident.
pub(crate) struct GeometryBuffers {
	counts: GeometryCounts,
	handles: GeometryHandles,
}

impl GeometryHandles {
	/// Creates every geometry stream with the element count `capacity` gives it.
	///
	/// `capacity` comes from [`super::VisibilityPipelineSettings::geometry_capacity`], which only holds validated values.
	/// Next, hand the handles to [`GeometryBuffers::new`] so meshes can be appended.
	pub(crate) fn new(context: &mut ghi::implementation::Context, capacity: GeometryCapacity) -> Self {
		debug_assert!(capacity.validate().is_ok());
		let geometry = ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage;
		let indices = ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage;
		let storage = ghi::Uses::Storage;
		let build = |name, uses, length: u32| {
			ghi::buffer::Builder::new(uses)
				.name(name)
				.length(length as usize)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let GeometryCapacity {
			vertices,
			vertex_indices,
			triangles,
			meshlets,
			skinning_vertices: skinning,
		} = capacity;
		Self {
			capacity,
			vertex_positions: context.build_buffer(build("Visibility Vertex Positions Buffer", geometry, vertices)),
			vertex_normals: context.build_buffer(build("Visibility Vertex Normals Buffer", geometry, vertices)),
			vertex_uvs: context.build_buffer(build("Visibility Vertex UV Buffer", geometry, vertices)),
			vertex_indices: context.build_buffer(build("Visibility Index Buffer", indices, vertex_indices)),
			primitive_indices: context.build_buffer(build("Visibility Primitive Indices Buffer", indices, triangles)),
			meshlets: context.build_buffer(build("Visibility Meshlets Data", storage, meshlets)),
			skinning_rest_positions: context.build_buffer(build("Visibility Skinning Rest Positions", storage, skinning)),
			skinning_rest_normals: context.build_buffer(build("Visibility Skinning Rest Normals", storage, skinning)),
			skinning_joints: context.build_buffer(build("Visibility Skinning Joints", storage, skinning)),
			skinning_weights: context.build_buffer(build("Visibility Skinning Weights", storage, skinning)),
		}
	}
}

impl GeometryBuffers {
	/// Creates the append cursor over already-created geometry streams.
	pub(crate) fn new(handles: GeometryHandles) -> Self {
		Self {
			counts: GeometryCounts::default(),
			handles,
		}
	}

	/// Records the copies that append a prepared mesh and returns where it now lives.
	///
	/// `material_indices` supplies the render-thread material slot for each prepared primitive. Nothing is
	/// appended when the mesh does not fit, so partial residency is never published.
	pub(crate) fn write_mesh(
		&mut self,
		c: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_buffer: ghi::BaseBufferHandle,
		prepared: &PreparedMesh,
		material_indices: &[u32],
	) -> Option<MeshData> {
		// Task shaders address an instance's meshlets relative to its first one, with a bounded index width.
		if let Some(primitive) = prepared
			.primitives
			.iter()
			.find(|prepared| prepared.primitive.meshlet_count as usize > MAX_INSTANCE_MESHLETS)
		{
			log::error!(
				"Visibility mesh upload was rejected. The most likely cause is that a primitive has {} meshlets, more than the {MAX_INSTANCE_MESHLETS} one instance can address; split the primitive.",
				primitive.primitive.meshlet_count
			);
			return None;
		}
		let next_counts = self.counts.grown_by(prepared.counts, &self.handles.capacity)?;
		let base = &self.counts;
		let staging = prepared.staging.offset();
		let streams = &prepared.streams;
		let copy = |source: &std::ops::Range<usize>, destination: ghi::BaseBufferHandle, destination_offset: usize| {
			ghi::BufferCopyDescriptor::new(
				staging_buffer,
				staging + source.start,
				destination,
				destination_offset,
				source.len(),
			)
		};
		c.copy_buffers(&[
			copy(
				&streams.positions,
				self.handles.vertex_positions.into(),
				base.vertices as usize * 12,
			),
			copy(
				&streams.normals,
				self.handles.vertex_normals.into(),
				base.vertices as usize * VERTEX_NORMAL_BUFFER_STRIDE as usize,
			),
			copy(
				&streams.uvs,
				self.handles.vertex_uvs.into(),
				base.vertices as usize * VERTEX_UV_BUFFER_STRIDE as usize,
			),
			copy(
				&streams.vertex_indices,
				self.handles.vertex_indices.into(),
				base.primitive_indices as usize * 2,
			),
			copy(
				&streams.primitive_indices,
				self.handles.primitive_indices.into(),
				base.triangles as usize * 3,
			),
			copy(
				&streams.meshlets,
				self.handles.meshlets.into(),
				base.meshlets as usize * std::mem::size_of::<ShaderMeshletData>(),
			),
		]);

		let primitives = prepared
			.primitives
			.iter()
			.zip(material_indices)
			.map(|(prepared, material_index)| {
				let mut primitive = prepared.primitive.clone();
				primitive.material_index = *material_index;
				if let (Some(skinning), Some(relative_offset)) =
					(&prepared.skinning, primitive.skinning_source_vertex_offset.as_mut())
				{
					let destination_vertex = (base.skinning_vertices + *relative_offset) as usize;
					*relative_offset += base.skinning_vertices;
					c.copy_buffers(&[
						copy(
							&skinning.positions,
							self.handles.skinning_rest_positions.into(),
							destination_vertex * SKINNING_POSITION_STRIDE,
						),
						copy(
							&skinning.normals,
							self.handles.skinning_rest_normals.into(),
							destination_vertex * SKINNING_NORMAL_STRIDE,
						),
						copy(
							&skinning.joints,
							self.handles.skinning_joints.into(),
							destination_vertex * SKINNING_JOINTS_STRIDE,
						),
						copy(
							&skinning.weights,
							self.handles.skinning_weights.into(),
							destination_vertex * SKINNING_WEIGHTS_STRIDE,
						),
					]);
				}
				primitive
			})
			.collect();

		let mesh = MeshData {
			primitives,
			skeleton_node_count: prepared.skeleton_node_count,
			vertex_offset: base.vertices,
			primitive_offset: base.primitive_indices,
			triangle_offset: base.triangles,
			meshlet_offset: base.meshlets,
		};
		self.counts = next_counts;
		Some(mesh)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn visibility_capacity_rejects_only_the_overflowing_upload() {
		assert_eq!(checked_capacity(3, 2, 5, "test"), Some(5));
		assert_eq!(checked_capacity(3, 3, 5, "test"), None);
		assert_eq!(checked_capacity(u32::MAX, 1, u32::MAX, "test"), None);
	}

	#[test]
	fn geometry_capacity_rejects_empty_or_unaddressable_streams() {
		assert_eq!(GeometryCapacity::default().validate(), Ok(()));
		let empty = GeometryCapacity {
			triangles: 0,
			..GeometryCapacity::default()
		};
		assert!(empty.validate().is_err_and(|reason| reason.contains("triangle capacity is zero")));
		let unaddressable = GeometryCapacity {
			meshlets: MAX_ADDRESSABLE_MESHLETS as u32 + 1,
			..GeometryCapacity::default()
		};
		assert!(unaddressable.validate().is_err());
	}
}
