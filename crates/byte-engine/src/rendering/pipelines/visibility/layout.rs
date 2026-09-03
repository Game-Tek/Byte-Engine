//! Fixed limits, buffer strides, and descriptor bindings shared by every visibility shader and the CPU code that feeds them.
//!
//! Slots below 1024 belong to the base descriptor set that every visibility stage binds. Slots from 1033
//! belong to the per-sink visibility set (material dispatch bookkeeping and the visibility buffer images)
//! and the material-evaluation set (lighting, materials, shadow maps, and environment). Keep the numbers in
//! sync with [`super::shader_generator::VisibilityShaderScope`] and the checked-in BESL assets.

use ghi::{AccessPolicies, ResourceKind, ResourceSlot, ShaderResourceDescriptor, TextureViewTypes};

use super::shader_data::{LightingData, MaterialData};

/* Limits */

pub(crate) const MAX_MESHLETS: usize = 1024 * 4;
pub(crate) const MAX_INSTANCES: usize = 1024;
pub(crate) const MAX_MATERIALS: usize = 1024;
/// One bit per material slot; a set bit means the material has at least one active instance this frame.
pub(crate) type ActiveMaterialMask = [u64; MAX_MATERIALS / u64::BITS as usize];
/// Materials use a small indirection table so generated shaders can use stable per-material slots into the
/// larger scene-wide bindless texture pool.
pub(crate) const MAX_MATERIAL_TEXTURES: usize = 16;
pub(crate) const MAX_BINDLESS_TEXTURES: usize = 1024;
pub(crate) const MAX_LIGHTS: usize = 16;
pub(crate) const MAX_TRIANGLES: usize = 65536 * 4;
pub(crate) const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
pub(crate) const MAX_VERTICES: usize = 65536 * 4;
pub(crate) const MAX_PIXEL_MAPPING_ENTRIES: usize = 3840 * 2160;

/// Vertices and triangles one meshlet can hold.
pub(crate) const VERTEX_COUNT: u32 = 64;
pub(crate) const TRIANGLE_COUNT: u32 = 126;
/// Meshlets culled by one task workgroup.
pub(crate) const MESHLET_CULLING_TASK_GROUP_SIZE: u32 = 32;

/* Shadow views */

pub(crate) const SHADOW_CASCADE_COUNT: usize = 4;
pub(crate) const SHADOW_MAP_RESOLUTION: u32 = 2048;
/// The largest local-light shadow pools that fit the visibility light table.
pub(crate) const MAX_CONE_SHADOW_POOL_CAPACITY: usize = MAX_LIGHTS;
pub(crate) const MAX_POINT_SHADOW_POOL_CAPACITY: usize = MAX_LIGHTS;
/// Pool capacities used when an application does not configure them.
pub(crate) const DEFAULT_CONE_SHADOW_POOL_CAPACITY: usize = 4;
pub(crate) const DEFAULT_POINT_SHADOW_POOL_CAPACITY: usize = 4;
pub(crate) const CONE_SHADOW_MAP_RESOLUTION: u32 = 1024;
pub(crate) const POINT_SHADOW_MAP_RESOLUTION: u32 = 1024;
/// Local-light maps use 16-bit depth to halve their memory; cascades and the camera keep 32-bit depth.
pub(crate) const CONE_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
pub(crate) const POINT_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth16;
pub(crate) const DIRECTIONAL_SHADOW_MAP_FORMAT: ghi::Formats = ghi::Formats::Depth32;
/// Layout of the `views` buffer: camera, then cascades, then cone layers, then point cube faces.
pub(crate) const CONE_SHADOW_VIEW_OFFSET: usize = 1 + SHADOW_CASCADE_COUNT;
pub(crate) const POINT_SHADOW_FACE_COUNT: usize = 6;
pub(crate) const POINT_SHADOW_VIEW_OFFSET: usize = CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_POOL_CAPACITY;
pub(crate) const SHADOW_VIEW_COUNT: usize = POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT;

/* Runtime vertex formats */

/// The octahedrally encoded runtime unit-vector element.
pub(crate) type RuntimeUnitVector = [u16; 2];
pub(crate) type RuntimeVertexNormal = RuntimeUnitVector;
pub(crate) const VERTEX_NORMAL_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexNormal>() as u32;
/// Half-float UVs preserve sampler wrapping coordinates outside `[0, 1]`.
pub(crate) type RuntimeVertexUv = [u16; 2];
pub(crate) const VERTEX_UV_BUFFER_STRIDE: u32 = std::mem::size_of::<RuntimeVertexUv>() as u32;
// Every backend stores affine matrices as twelve floats; MSL reconstructs native float4x3 values when reading them.
pub(crate) const VIEW_DATA_BUFFER_STRIDE: u32 = 176;
// ShaderMesh retains an explicit 16-byte record alignment while its affine matrix occupies 48 bytes.
pub(crate) const MESH_DATA_BUFFER_STRIDE: u32 = 80;
// HLSL reads packed narrow indices through 32-bit structured words. Metal and Vulkan expose native widths.
pub(crate) const VERTEX_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 2 };
pub(crate) const PRIMITIVE_INDEX_BUFFER_STRIDE: u32 = if cfg!(target_os = "windows") { 4 } else { 1 };
pub(crate) const MESHLET_DATA_BUFFER_STRIDE: u32 = std::mem::size_of::<ShaderMeshletData>() as u32;

/// The `ShaderMeshletData` struct stores meshlet offsets and object-space culling bounds for GPU visibility passes.
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct ShaderMeshletData {
	/// Base index into the vertex-index buffer, relative to the mesh.
	pub(crate) primitive_offset: u32,
	/// Base triangle into the primitive-index buffer, relative to the mesh. Multiply by three for the raw index.
	pub(crate) triangle_offset: u32,
	/// Number of meshlet-local vertices.
	pub(crate) primitive_count: u32,
	/// Number of meshlet-local triangles.
	pub(crate) triangle_count: u32,
	/// Object-space bounding sphere encoded as xyz center and w radius.
	pub(crate) center_radius: [f32; 4],
	/// Object-space normal-cone apex encoded as xyz apex and w cutoff. A cutoff above one disables cone rejection.
	pub(crate) cone_apex_cutoff: [f32; 4],
	/// Octahedrally encoded object-space normal-cone axis.
	pub(crate) cone_axis: RuntimeVertexNormal,
}

/* Binding helpers */

const fn buffer(slot: u32, access: AccessPolicies, stride: u32) -> ShaderResourceDescriptor {
	ShaderResourceDescriptor::single(ResourceSlot::new(slot), ResourceKind::StorageBuffer, access).buffer_stride(stride)
}

const fn storage_image(slot: u32, access: AccessPolicies) -> ShaderResourceDescriptor {
	ShaderResourceDescriptor::single(ResourceSlot::new(slot), ResourceKind::StorageImage, access)
}

const fn sampled_image(slot: u32) -> ShaderResourceDescriptor {
	ShaderResourceDescriptor::single(
		ResourceSlot::new(slot),
		ResourceKind::CombinedImageSampler,
		AccessPolicies::READ,
	)
}

/* Base descriptor set */

pub(crate) const VIEWS_DATA_BINDING: ShaderResourceDescriptor = buffer(0, AccessPolicies::READ, VIEW_DATA_BUFFER_STRIDE);
pub(crate) const MESH_DATA_BINDING: ShaderResourceDescriptor = buffer(1, AccessPolicies::READ, MESH_DATA_BUFFER_STRIDE);
pub(crate) const VERTEX_POSITIONS_BINDING: ShaderResourceDescriptor = buffer(2, AccessPolicies::READ, 12);
pub(crate) const VERTEX_NORMALS_BINDING: ShaderResourceDescriptor =
	buffer(3, AccessPolicies::READ, VERTEX_NORMAL_BUFFER_STRIDE);
pub(crate) const SKINNED_VERTICES_BINDING: ShaderResourceDescriptor = buffer(4, AccessPolicies::READ, 32);
pub(crate) const VERTEX_UV_BINDING: ShaderResourceDescriptor = buffer(5, AccessPolicies::READ, VERTEX_UV_BUFFER_STRIDE);
pub(crate) const VERTEX_INDICES_BINDING: ShaderResourceDescriptor = buffer(6, AccessPolicies::READ, VERTEX_INDEX_BUFFER_STRIDE);
pub(crate) const PRIMITIVE_INDICES_BINDING: ShaderResourceDescriptor =
	buffer(7, AccessPolicies::READ, PRIMITIVE_INDEX_BUFFER_STRIDE);
pub(crate) const MESHLET_DATA_BINDING: ShaderResourceDescriptor = buffer(8, AccessPolicies::READ, MESHLET_DATA_BUFFER_STRIDE);
pub(crate) const TEXTURES_BINDING: ShaderResourceDescriptor = ShaderResourceDescriptor::new(
	ResourceSlot::new(9),
	ResourceKind::CombinedImageSampler,
	MAX_BINDLESS_TEXTURES as u32,
	AccessPolicies::READ,
);
pub(crate) const MATERIALS_DATA_BINDING: ShaderResourceDescriptor =
	buffer(1046, AccessPolicies::READ, std::mem::size_of::<MaterialData>() as u32);
pub(crate) const MESH_DISPATCH_WORK_BINDING: ShaderResourceDescriptor = buffer(1063, AccessPolicies::READ, 4);

/* Visibility descriptor set */

pub(crate) const MATERIAL_COUNT_BINDING: ShaderResourceDescriptor = buffer(1033, AccessPolicies::READ_WRITE, 4);
pub(crate) const MATERIAL_OFFSET_BINDING: ShaderResourceDescriptor = buffer(1034, AccessPolicies::READ_WRITE, 4);
pub(crate) const MATERIAL_OFFSET_SCRATCH_BINDING: ShaderResourceDescriptor = buffer(1035, AccessPolicies::READ_WRITE, 4);
pub(crate) const MATERIAL_EVALUATION_DISPATCHES_BINDING: ShaderResourceDescriptor =
	buffer(1036, AccessPolicies::READ_WRITE, 12);
pub(crate) const MATERIAL_XY_BINDING: ShaderResourceDescriptor = buffer(1037, AccessPolicies::WRITE, 4);
pub(crate) const TRIANGLE_INDEX_BINDING: ShaderResourceDescriptor = storage_image(1039, AccessPolicies::READ);
pub(crate) const INSTANCE_ID_BINDING: ShaderResourceDescriptor = storage_image(1040, AccessPolicies::READ);

/* Material evaluation descriptor set */

pub(crate) const LIT_BINDING: ShaderResourceDescriptor = storage_image(1041, AccessPolicies::READ_WRITE);
pub(crate) const LIGHTING_DATA_BINDING: ShaderResourceDescriptor =
	buffer(1045, AccessPolicies::READ, std::mem::size_of::<LightingData>() as u32);
pub(crate) const AO_MAP_BINDING: ShaderResourceDescriptor = sampled_image(1051);
pub(crate) const SHADOW_MAP_BINDING: ShaderResourceDescriptor =
	sampled_image(1052).texture_view_type(TextureViewTypes::Texture2DArray);
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING: ShaderResourceDescriptor = sampled_image(1053);
pub(crate) const ENVIRONMENT_BINDING: ShaderResourceDescriptor =
	sampled_image(1054).texture_view_type(TextureViewTypes::TextureCube);
pub(crate) const SPECULAR_ENVIRONMENT_BINDING: ShaderResourceDescriptor =
	sampled_image(1055).texture_view_type(TextureViewTypes::TextureCube);
pub(crate) const CONE_SHADOW_MAP_BINDING: ShaderResourceDescriptor =
	sampled_image(1064).texture_view_type(TextureViewTypes::Texture2DArray);
pub(crate) const POINT_SHADOW_MAP_BINDING: ShaderResourceDescriptor =
	sampled_image(1065).texture_view_type(TextureViewTypes::TextureCubeArray);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shader_meshlet_data_matches_packed_buffer_layout() {
		assert_eq!(std::mem::align_of::<ShaderMeshletData>(), 4);
		assert_eq!(std::mem::size_of::<ShaderMeshletData>(), 52);
		assert_eq!(std::mem::offset_of!(ShaderMeshletData, center_radius), 16);
		assert_eq!(std::mem::offset_of!(ShaderMeshletData, cone_apex_cutoff), 32);
		assert_eq!(std::mem::offset_of!(ShaderMeshletData, cone_axis), 48);
	}
}
