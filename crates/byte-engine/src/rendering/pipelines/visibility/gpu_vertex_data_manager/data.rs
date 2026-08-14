#[derive(Clone, Copy, Default)]
pub struct VisibilityInfo {
	pub instance_count: u32,
	pub triangle_count: u32,
	pub meshlet_count: u32,
	pub vertex_count: u32,
	pub primitives_count: u32,
}

/// The `MeshData` struct stores the geometry ranges needed after a mesh resource
/// enters visibility GPU storage.
#[derive(Debug, Clone)]
pub struct MeshData {
	pub primitives: Vec<MeshPrimitive>,
	/// Base position in the vertex buffer.
	pub vertex_offset: u32,
	pub primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub triangle_offset: u32,
	/// Base position in the meshlet buffer, relative to the mesh.
	pub meshlet_offset: u32,
	pub acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

/// The `MeshPrimitive` struct locates one primitive's geometry and optional skinning inputs in visibility buffers.
#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The meshlet count.
	pub meshlet_count: u32,
	/// Base position in the meshlet buffer, relative to the primitive.
	pub meshlet_offset: u32,
	/// Base position in the vertex buffer.
	pub vertex_offset: u32,
	/// Base position in the primitive-index buffer.
	pub primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub triangle_offset: u32,
	/// The first vertex in the compact immutable skinning source buffers, when this primitive is skinned.
	pub skinning_source_vertex_offset: Option<u32>,
	/// The number of vertices the skinning compute pass writes for this primitive.
	pub skinning_vertex_count: u32,
}
