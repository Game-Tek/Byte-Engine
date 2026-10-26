//! The `rt` module describes the geometry and instances of the acceleration structures rays traverse.
//!
//! Check [`crate::context::Context::supports_ray_tracing`] first. Then create bottom-level structures from
//! [`TriangleGeometry`] with [`crate::context::ContextCreate::create_bottom_level_acceleration_structure`], build each
//! once with [`crate::command_buffer::CommandBufferRecording::build_bottom_level_acceleration_structures`], and place
//! them every frame with [`Instance`] through
//! [`crate::command_buffer::CommandBufferRecording::build_top_level_acceleration_structure`]. Shade the hits with a
//! [`crate::pipelines::ray_tracing::Builder`] pipeline, whose hit groups a shader binding table maps the hit records
//! to.

use crate::{BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferStridedRange, DataTypes};

/// One more than the largest custom index or hit group offset an [`Instance`] can carry.
///
/// Vulkan stores both in 24 bits, so every backend keeps them below this value.
pub const INSTANCE_INDEX_LIMIT: u32 = 1 << 24;

/// Stops a backend without ray tracing at its first acceleration structure, ray tracing pipeline, or shader binding
/// table call.
///
/// [`crate::context::Context::supports_ray_tracing`] is always `false` on such a backend, so reaching this means the
/// application skipped that check.
#[cfg(not(target_os = "macos"))]
pub(crate) fn unsupported(backend: &str) -> ! {
	panic!(
		"{backend} cannot build acceleration structures or trace rays. The most likely cause is that the application did not check Context::supports_ray_tracing, which is always false on {backend}. Check Context::supports_ray_tracing, and use a technique without rays when it is false."
	)
}

/// The `TriangleGeometry` struct describes one run of indexed triangles that a bottom-level acceleration structure
/// holds, read from buffers the application already filled.
///
/// Pass one for each opaque primitive of a mesh to
/// [`crate::context::ContextCreate::create_bottom_level_acceleration_structure`]. The order you pass them in numbers
/// them, and a hit reports that number as its geometry index. Both buffers need [`crate::Uses::AccelerationStructureBuild`].
/// Positions start at a multiple of 4 bytes, at a stride of at least 12 bytes that is a multiple of 4, and indices
/// start at a multiple of their size.
#[derive(Clone, Copy)]
pub struct TriangleGeometry {
	/// The vertex positions: three `f32` each, a stride apart. The range's size divided by its stride is the vertex
	/// count.
	pub positions: BufferStridedRange,
	/// Where the first index of the first triangle is. Indices count from the start of `positions`.
	pub indices: BufferDescriptor,
	/// [`DataTypes::U16`] or [`DataTypes::U32`].
	pub index_format: DataTypes,
	pub triangle_count: u32,
}

/// The `Instance` struct places one bottom-level acceleration structure in the world a top-level structure covers.
///
/// Pass a frame's instances to [`crate::command_buffer::CommandBufferRecording::build_top_level_acceleration_structure`].
/// A hit on the instance reports `custom_index`, and its record is `hit_group_offset` plus the hit geometry's index.
#[derive(Clone, Copy)]
pub struct Instance {
	/// Three rows of four floats that map object space to world space, applied to the object position extended with a
	/// fourth component of 1.
	pub transform: [[f32; 4]; 3],
	/// A value below [`INSTANCE_INDEX_LIMIT`] that hits on this instance report, such as the index of its first
	/// geometry in an application table.
	pub custom_index: u32,
	/// A ray hits this instance only when the ray's mask and this mask share a bit.
	pub mask: u8,
	/// A value below [`INSTANCE_INDEX_LIMIT`] that a hit adds to its geometry index to give its record.
	pub hit_group_offset: u32,
	/// The structure this instance places. Record its build before the top-level build that reads this instance.
	pub bottom_level: BottomLevelAccelerationStructureHandle,
}
