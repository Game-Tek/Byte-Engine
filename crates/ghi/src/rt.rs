//! The `rt` module describes the geometry and instances of the acceleration structures rays traverse.
//!
//! Check [`crate::context::Context::supports_ray_tracing`] first. Then create bottom-level structures from
//! [`TriangleGeometry`] with [`crate::context::ContextCreate::create_bottom_level_acceleration_structure`], build each
//! once with [`crate::command_buffer::CommandBufferRecording::build_bottom_level_acceleration_structures`], and place
//! them every frame with [`Instance`] through
//! [`crate::command_buffer::CommandBufferRecording::build_top_level_acceleration_structure`]. Shade the hits with a
//! [`crate::pipelines::ray_tracing::Builder`] pipeline, whose hit groups a shader binding table maps the hit records
//! to.

use crate::{BottomLevelAccelerationStructureHandle, BufferDescriptor, BufferStridedRange, DataTypes, Uses};

/// One more than the largest custom index or hit group offset an [`Instance`] can carry.
///
/// Vulkan stores both in 24 bits, so every backend keeps them below this value.
pub const INSTANCE_INDEX_LIMIT: u32 = 1 << 24;

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

/// The `BuildState` enum follows a bottom-level structure through its one build, for backends whose recordings can run
/// in another order than they were recorded in.
// DX12 keeps its own two-state enum, because a later DX12 recording always runs after the one that built it.
#[cfg(not(target_os = "windows"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BuildState {
	/// No executed recording built the structure, so a recording may build it.
	Unbuilt,
	/// A recording that has not executed yet holds the structure's build.
	Recorded,
	/// The recording that built the structure executed.
	Built,
}

/// Returns the bytes one index of `format` occupies: 2 for [`DataTypes::U16`], otherwise 4.
pub(crate) fn index_size(format: DataTypes) -> usize {
	match format {
		DataTypes::U16 => 2,
		_ => 4,
	}
}

/// Returns the bytes the indices of `geometry` span.
pub(crate) fn index_bytes(geometry: &TriangleGeometry) -> usize {
	geometry.triangle_count as usize * 3 * index_size(geometry.index_format)
}

/// Panics for a handle whose structure was destroyed. `level` is "bottom-level" or "top-level".
pub(crate) fn stale_handle(level: &str) -> ! {
	panic!(
		"The {level} acceleration structure handle is stale. The most likely cause is that the structure was destroyed before this call. Stop using a handle once you destroy its structure, and create a new structure instead."
	)
}

/// Panics because `backend`'s context can't trace rays. `operation` names what needed ray tracing, such as "create a
/// shader binding table".
// DX12 names the DXR tier it needs in its own message.
#[cfg(not(target_os = "windows"))]
pub(crate) fn unsupported(backend: &str, operation: &str) -> ! {
	panic!(
		"{backend} could not {operation}. The most likely cause is that the device was created without the ray tracing feature, or that its GPU does not support ray tracing. Check Context::supports_ray_tracing before you {operation}, and request Features::ray_tracing when you create the device."
	)
}

/// Panics unless `geometries` can make a bottom-level structure: at least one geometry, each with 16- or 32-bit
/// indices and a position stride of at least 12 bytes that is a multiple of 4.
///
/// Check each geometry's buffer ranges next with [`check_geometry_range`].
pub(crate) fn check_bottom_level_geometries(backend: &str, geometries: &[TriangleGeometry]) {
	assert!(
		!geometries.is_empty(),
		"{backend} bottom-level acceleration structure creation failed. The most likely cause is that no triangle geometry was given. Pass at least one geometry.",
	);
	for geometry in geometries {
		assert!(
			matches!(geometry.index_format, DataTypes::U16 | DataTypes::U32),
			"{backend} triangle geometry has an unsupported index format. The most likely cause is that the index format is not a 16-bit or 32-bit integer. Use DataTypes::U16 or DataTypes::U32.",
		);
		let stride = geometry.positions.stride;
		assert!(
			stride >= 12 && stride.is_multiple_of(4),
			"{backend} triangle geometry has an unsupported position stride. The most likely cause is that the stride is smaller than three floats or not a multiple of 4 bytes. stride={stride}. Use a stride of at least 12 bytes that is a multiple of 4.",
		);
	}
}

/// Panics unless `size` bytes from `range` lie in a buffer that geometry may be built from, at a multiple of
/// `alignment`.
///
/// `buffer` is the uses and size of the buffer `range` names, as the backend looked them up, or `None` when the
/// backend has no such buffer. Positions use an alignment of 4, and indices [`index_size`].
pub(crate) fn check_geometry_range(
	backend: &str,
	range: &BufferDescriptor,
	size: usize,
	alignment: usize,
	buffer: Option<(Uses, usize)>,
) {
	let Some((uses, buffer_size)) = buffer else {
		panic!(
			"{backend} triangle geometry names a missing buffer. The most likely cause is that the buffer handle came from another context. Build geometry from buffers this context created."
		);
	};
	assert!(
		uses.contains(Uses::AccelerationStructureBuild),
		"{backend} triangle geometry reads a buffer without the acceleration structure build use. The most likely cause is that the buffer was created for drawing only. Add Uses::AccelerationStructureBuild to the buffer's builder.",
	);
	assert!(
		range.offset.checked_add(size).is_some_and(|end| end <= buffer_size),
		"{backend} triangle geometry passes the end of its buffer. The most likely cause is that the triangle count, stride, or offset describes more data than the buffer holds. range_end={}, buffer_size={buffer_size}. Reduce the count or fix the offset.",
		range.offset.saturating_add(size),
	);
	assert!(
		range.offset.is_multiple_of(alignment),
		"{backend} triangle geometry starts at a misaligned offset. The most likely cause is that the offset was not rounded to the element size. offset={}, alignment={alignment}. Align the offset to {alignment} bytes.",
		range.offset,
	);
}

/// Panics unless a top-level structure may be created for `max_instance_count` instances.
pub(crate) fn check_max_instance_count(backend: &str, max_instance_count: u32) {
	assert!(
		max_instance_count > 0,
		"{backend} top-level acceleration structure creation failed. The most likely cause is that the maximum instance count is zero. Pass the largest number of instances a frame places.",
	);
}

/// Panics unless a recording may claim the build of a bottom-level structure, which it may only while `unbuilt`.
pub(crate) fn check_unbuilt(backend: &str, unbuilt: bool) {
	assert!(
		unbuilt,
		"{backend} bottom-level acceleration structure was already built. The most likely cause is that its build was recorded twice, or that a recording which has not executed yet holds its build. Build each bottom-level structure once, and create a new structure for different geometry.",
	);
}

/// Panics because a top-level build was recorded without a frame, which picks the copy it builds.
pub(crate) fn outside_frame(backend: &str) -> ! {
	panic!(
		"{backend} top-level acceleration structure build was recorded outside a frame. The most likely cause is that the recording was created from a command buffer instead of a frame. Record top-level builds in a frame's recording, because each frame builds its own copy."
	)
}

/// Panics because a top-level structure has no copy for the recording's frame.
pub(crate) fn missing_frame_copy(backend: &str) -> ! {
	panic!(
		"{backend} top-level acceleration structure has no copy for this frame. The most likely cause is that the frames in flight grew after the structure was created. Create top-level structures after you set the frames in flight."
	)
}

/// Panics unless a top-level structure created for `max_instance_count` instances holds `instance_count`.
pub(crate) fn check_instance_count(backend: &str, instance_count: usize, max_instance_count: u32) {
	assert!(
		instance_count <= max_instance_count as usize,
		"{backend} top-level acceleration structure build has more instances than the structure holds. The most likely cause is that the scene grew past the maximum instance count given at creation. instances={instance_count}, max_instance_count={max_instance_count}. Create the structure with a larger maximum instance count.",
	);
}

/// Panics unless this is the frame's first build of a top-level copy. `first` is false when the frame built it
/// already.
// Only Vulkan and DX12 check this.
#[cfg(not(target_os = "macos"))]
pub(crate) fn check_first_build_this_frame(backend: &str, first: bool) {
	assert!(
		first,
		"{backend} top-level acceleration structure was built twice in one frame. The most likely cause is that two recordings of the frame build the same structure. Build each top-level structure once a frame, after every change to its instances.",
	);
}

/// Panics unless `instance` may be written to an instance record. `bottom_level_built` is false when no recording
/// claimed the build of the structure it places.
pub(crate) fn check_instance(backend: &str, instance: &Instance, bottom_level_built: bool) {
	assert!(
		instance.custom_index < INSTANCE_INDEX_LIMIT && instance.hit_group_offset < INSTANCE_INDEX_LIMIT,
		"{backend} acceleration structure instance has a custom index or hit group offset of 2^24 or more. The most likely cause is that an index table outgrew 24 bits. custom_index={}, hit_group_offset={}. Keep both values below rt::INSTANCE_INDEX_LIMIT.",
		instance.custom_index,
		instance.hit_group_offset,
	);
	assert!(
		bottom_level_built,
		"{backend} acceleration structure instance places a bottom-level structure that was never built. The most likely cause is that its build was not recorded before this top-level build. Record its build with build_bottom_level_acceleration_structures in this recording or an earlier one.",
	);
}
