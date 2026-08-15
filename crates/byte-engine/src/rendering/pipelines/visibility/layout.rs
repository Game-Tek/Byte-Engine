/* BASE */
/// Shader binding used to access scene views.
// Every backend stores affine matrices as twelve floats; MSL reconstructs native float4x3 values when reading them.
pub(crate) const VIEW_DATA_BUFFER_STRIDE: u32 = 176;
pub(crate) const VIEWS_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VIEW_DATA_BUFFER_STRIDE);
// ShaderMesh retains an explicit 16-byte record alignment while its affine matrix occupies 48 bytes.
pub(crate) const MESH_DATA_BUFFER_STRIDE: u32 = 80;
pub(crate) const MESH_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(MESH_DATA_BUFFER_STRIDE);
pub(crate) const VERTEX_POSITIONS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(2),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(12);
/// The octahedrally encoded runtime unit-vector element.
pub(crate) type RuntimeUnitVector = [u16; 2];
/// The octahedrally encoded runtime normal element.
pub(crate) type RuntimeVertexNormal = RuntimeUnitVector;
pub(crate) const VERTEX_NORMAL_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexNormal>() as u32;
pub(crate) const VERTEX_NORMAL_SHADER_TYPE: &str = "vec2u16";
pub(crate) const VERTEX_NORMALS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(3),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_NORMAL_BUFFER_STRIDE);
pub(crate) const SKINNED_VERTICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(4),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(32);
/// The packed half-float runtime UV element preserves sampler wrapping coordinates outside `[0, 1]`.
pub(crate) type RuntimeVertexUv = [u16; 2];
pub(crate) const VERTEX_UV_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexUv>() as u32;
pub(crate) const VERTEX_UV_SHADER_TYPE: &str = "vec2f16";
pub(crate) const VERTEX_UV_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(5),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_UV_BUFFER_STRIDE);
// HLSL reads packed narrow indices through 32-bit structured words. Metal and
// Vulkan expose their native scalar element widths directly.
pub(crate) const VERTEX_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 2 };
pub(crate) const VERTEX_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(6),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(VERTEX_INDEX_BUFFER_STRIDE);
pub(crate) const PRIMITIVE_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 1 };
pub(crate) const PRIMITIVE_INDICES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(7),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(PRIMITIVE_INDEX_BUFFER_STRIDE);
pub(crate) const MESHLET_DATA_BUFFER_STRIDE: u32 = std::mem::size_of::<ShaderMeshletData>() as u32;
pub(crate) const MESHLET_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(8),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(MESHLET_DATA_BUFFER_STRIDE);
pub(crate) const MESH_DISPATCH_WORK_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1063),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(4);
pub(crate) const TEXTURES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::new(
	ghi::ResourceSlot::new(9),
	ghi::ResourceKind::CombinedImageSampler,
	MAX_BINDLESS_TEXTURES as u32,
	ghi::AccessPolicies::READ,
);

/* Visibility */
pub(crate) const MATERIAL_COUNT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_OFFSET_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(4);
pub(crate) const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ.union(ghi::AccessPolicies::WRITE),
)
.buffer_stride(12);
pub(crate) const MATERIAL_XY_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1037),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
)
.buffer_stride(4);
pub(crate) const TRIANGLE_INDEX_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1039),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ,
);
pub(crate) const INSTANCE_ID_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1040),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ,
);

/* Material Evaluation */
pub(crate) const VERTEX_COUNT: u32 = 64;
pub(crate) const TRIANGLE_COUNT: u32 = 126;
pub(crate) const MESHLET_CULLING_TASK_GROUP_SIZE: u32 = 32;

pub(crate) const MAX_MESHLETS: usize = 1024 * 4;
pub(crate) const MAX_INSTANCES: usize = 1024;
pub(crate) const MAX_MATERIALS: usize = 1024;
pub(super) type ActiveMaterialMask = [u64; MAX_MATERIALS / u64::BITS as usize];
// Materials keep a small indirection table so generated shaders can use stable per-material slots,
// while the descriptor array itself is a larger scene-wide bindless texture pool.
pub(crate) const MAX_MATERIAL_TEXTURES: usize = 16;
pub(crate) const MAX_BINDLESS_TEXTURES: usize = 1024;
pub(crate) const MAX_LIGHTS: usize = 16;
pub(crate) const MAX_TRIANGLES: usize = 65536 * 4;
pub(crate) const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
pub(crate) const MAX_VERTICES: usize = 65536 * 4;
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;
pub(crate) const SHADOW_CASCADE_COUNT: usize = 4;
pub(crate) const SHADOW_MAP_RESOLUTION: u32 = 2048;
/// The largest cone shadow pool that fits the visibility light table.
pub(crate) const MAX_CONE_SHADOW_POOL_CAPACITY: usize = MAX_LIGHTS;
/// The cone shadow pool capacity used when an application does not configure one.
pub(crate) const DEFAULT_CONE_SHADOW_POOL_CAPACITY: usize = 4;
pub(crate) const CONE_SHADOW_MAP_RESOLUTION: u32 = 1024;
/// The depth format that halves the memory used by cone-light shadow maps.
pub(crate) const CONE_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
/// The largest point-shadow pool that fits the visibility light table.
pub(crate) const MAX_POINT_SHADOW_POOL_CAPACITY: usize = MAX_LIGHTS;
/// The point-shadow pool capacity used when an application does not configure one.
pub(crate) const DEFAULT_POINT_SHADOW_POOL_CAPACITY: usize = 4;
pub(crate) const POINT_SHADOW_MAP_RESOLUTION: u32 = 1024;
/// The depth format that halves the memory used by point-light cube shadow maps.
pub(crate) const POINT_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
/// The depth format retained for directional cascades and the camera depth target.
pub(crate) const DIRECTIONAL_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth32;
pub(crate) const CONE_SHADOW_VIEW_OFFSET: usize = 1 + SHADOW_CASCADE_COUNT;
pub(crate) const POINT_SHADOW_FACE_COUNT: usize = 6;
pub(crate) const POINT_SHADOW_VIEW_OFFSET: usize = CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_POOL_CAPACITY;
pub(crate) const SHADOW_VIEW_COUNT: usize = POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT;

/// The `ShaderMeshletData` struct stores meshlet offsets and object-space culling bounds for GPU visibility passes.
#[derive(Copy, Clone)]
#[repr(C)]
pub(crate) struct ShaderMeshletData {
	/// Base index into the vertex-index buffer.
	/// ```glsl
	/// vertex_index = mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + gl_LocalInvocationID.x];
	/// ```
	pub(crate) primitive_offset: u32,
	/// Base triangle index into the primitive-index buffer.
	///
	/// The stored value divides the raw index by 3 because each triangle has three indices.
	/// ```glsl
	/// triangle_index = primitive_indices.primitive_indices[(meshlet.triangle_offset + gl_LocalInvocationID.x) * 3 + 0..2]
	/// ```
	pub(crate) triangle_offset: u32,
	/// Number of meshlet-local primitives.
	pub(crate) primitive_count: u32,
	// The number of triangles in the meshlet
	pub(crate) triangle_count: u32,
	/// Object-space bounding sphere encoded as xyz center and w radius.
	pub(crate) center_radius: [f32; 4],
	/// Object-space normal-cone apex encoded as xyz apex and w cutoff.
	pub(crate) cone_apex_cutoff: [f32; 4],
	/// Octahedrally encoded object-space normal-cone axis.
	pub(crate) cone_axis: RuntimeVertexNormal,
}
