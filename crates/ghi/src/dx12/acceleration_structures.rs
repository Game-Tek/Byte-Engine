//! Holds a DX12 context's acceleration structures, and the scratch memory their builds use.
//!
//! The context creates, destroys, builds, and compacts structures in `context/acceleration_structures.rs`. The design
//! follows the Vulkan backend's, which the ray tracing design note (`docs/develop/rendering/ray-tracing.mdx`)
//! describes: bottom-level structures are built once and compacted once their build completes, and each top-level
//! structure has one copy for each frame in flight, which the frame's sequence rebuilds from instances the CPU writes.

use std::cell::Cell;

use smallvec::SmallVec;
use windows::Win32::Graphics::Direct3D12 as d3d12;
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

use crate::slots::Slots;
use crate::{BottomLevelAccelerationStructureHandle, MAX_FRAMES_IN_FLIGHT, TopLevelAccelerationStructureHandle};

/// The alignment of acceleration structure storage and of build scratch memory.
pub(crate) const STRUCTURE_ALIGNMENT: u64 = d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT as u64;

/// The smallest scratch buffer a frame sequence keeps, so the first builds of a scene don't grow it build by build.
const SMALLEST_SCRATCH_BUFFER: u64 = 1 << 20;

/// The size of one instance record that a top-level build reads.
pub(crate) const INSTANCE_SIZE: u64 = size_of::<d3d12::D3D12_RAYTRACING_INSTANCE_DESC>() as u64;

/// Whether a bottom-level structure holds a build, which an instance needs before a top-level build places it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BuildState {
	Unbuilt,
	/// A recording built it. The recording may not have executed yet, but every later recording runs after it.
	Recorded,
}

/// A bottom-level structure: one committed buffer the size its build needs, and the geometry it is built from.
pub(crate) struct BottomLevel {
	pub(crate) resource: ID3D12Resource,
	pub(crate) address: u64,
	pub(crate) size: u64,
	pub(crate) geometries: Box<[crate::rt::TriangleGeometry]>,
	pub(crate) build_scratch_size: u64,
	pub(crate) build_state: Cell<BuildState>,
}

/// A top-level structure, with one copy for each frame in flight.
pub(crate) struct TopLevel {
	pub(crate) copies: SmallVec<[TopLevelCopy; MAX_FRAMES_IN_FLIGHT]>,
	pub(crate) max_instance_count: u32,
	pub(crate) build_scratch_size: u64,
}

/// One frame sequence's copy of a top-level structure, and the instance records its builds read.
pub(crate) struct TopLevelCopy {
	pub(crate) resource: ID3D12Resource,
	pub(crate) address: u64,
	/// An upload buffer of the structure's maximum instance count, mapped for its whole life.
	pub(crate) instances: ID3D12Resource,
	pub(crate) instance_records: *mut d3d12::D3D12_RAYTRACING_INSTANCE_DESC,
	/// The index of the frame that last recorded a build of this copy, so a frame can't build it twice.
	pub(crate) recorded_frame: Cell<Option<u64>>,
	/// The [`AccelerationStructures::epoch`] of the copy's last executed build, which bottom-level storage the build
	/// names by address must outlive.
	pub(crate) built_epoch: Cell<Option<u64>>,
}

// SAFETY: `instance_records` points into the mapping of `instances`, which this value owns and keeps mapped. No other
// value writes through it, so moving the copy to another thread with its context is sound.
unsafe impl Send for TopLevelCopy {}

/// Where a submission's fence reaches once the submission completes: a synchronizer and the value it signals.
pub(crate) type Completion = (crate::synchronizer::SynchronizerHandle, u64);

/// The `SizeQuery` struct is where the builds of one call write the size each structure compacts to, which the
/// context reads once they complete.
pub(crate) struct SizeQuery {
	/// CPU-readable memory that shaders may write, with one 64-bit size per structure. It's held only to keep `sizes`
	/// mapped.
	#[expect(dead_code, reason = "Owning the resource keeps the memory `sizes` points into alive.")]
	pub(crate) resource: ID3D12Resource,
	pub(crate) sizes: *const u64,
	/// The handle bits of each structure, in the order of their sizes.
	pub(crate) structures: SmallVec<[u64; 8]>,
}

// SAFETY: `sizes` points into the mapping of `resource`, which this value owns and keeps mapped, and only this value
// reads through it, so moving the query to another thread with its context is sound.
unsafe impl Send for SizeQuery {}

/// The `PendingCompaction` struct is a size query whose builds were submitted, waiting for them to complete.
pub(crate) struct PendingCompaction {
	pub(crate) query: SizeQuery,
	pub(crate) completion: Completion,
}

/// The `CompactionCopy` struct is the smaller storage a bottom-level structure is copied into, until the recording that
/// copies it executes.
pub(crate) struct CompactionCopy {
	/// The bits of the structure's handle.
	pub(crate) handle: u64,
	pub(crate) resource: ID3D12Resource,
	pub(crate) address: u64,
	pub(crate) size: u64,
}

/// The `CompactionSource` struct is storage that compaction replaced, which lives until every top-level copy built
/// before the replacement was built again.
pub(crate) struct CompactionSource {
	pub(crate) resource: ID3D12Resource,
	pub(crate) epoch: u64,
}

/// The `RecordedBuilds` struct is what one recording did to acceleration structures, which the context settles when
/// the recording executes or undoes when it is dropped.
#[derive(Default)]
pub(crate) struct RecordedBuilds {
	/// The bits of each bottom-level structure the recording builds.
	pub(crate) bottom_levels: SmallVec<[u64; 8]>,
	pub(crate) size_queries: SmallVec<[SizeQuery; 1]>,
	/// The bits, copy, and epoch at record time of each top-level copy the recording builds.
	pub(crate) top_levels: SmallVec<[(u64, u8, u64); 2]>,
	pub(crate) compactions: SmallVec<[CompactionCopy; 4]>,
}

/// The `ScratchArena` struct hands out the scratch ranges of one frame sequence's builds from one buffer, so the builds
/// of a recording can run at the same time.
#[derive(Default)]
pub(crate) struct ScratchArena {
	pub(crate) resource: Option<ID3D12Resource>,
	/// The address of the buffer's first byte.
	base: u64,
	size: u64,
	cursor: u64,
}

impl ScratchArena {
	/// Makes the whole buffer available again. Call it once the sequence's previous frame completed.
	pub(crate) fn reset(&mut self) {
		self.cursor = 0;
	}

	/// Returns the bytes left after the ranges taken since the last reset.
	pub(crate) fn remaining(&self) -> u64 {
		self.size - self.cursor
	}

	/// Takes `size` bytes at a multiple of `alignment` and returns their address, or `None` when they don't fit.
	pub(crate) fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
		if self.size == 0 {
			return None;
		}
		let address = (self.base + self.cursor).next_multiple_of(alignment);
		let end = address - self.base + size.max(1);
		if end > self.size {
			return None;
		}
		self.cursor = end;
		Some(address)
	}

	/// Returns the size a replacement buffer gets so that `size` bytes fit: at least twice the current size.
	pub(crate) fn grown_size(&self, size: u64) -> u64 {
		(self.size * 2).max(size).max(SMALLEST_SCRATCH_BUFFER)
	}

	/// Replaces the buffer with `resource`, of `size` bytes at `base`, and makes all of it available. Recordings that
	/// used the old buffer keep it alive.
	pub(crate) fn replace(&mut self, resource: Option<ID3D12Resource>, base: u64, size: u64) {
		self.resource = resource;
		self.base = base;
		self.size = size;
		self.cursor = 0;
	}
}

/// The `AccelerationStructures` struct holds every acceleration structure of a context and the scratch memory of each
/// frame sequence.
pub(crate) struct AccelerationStructures {
	pub(crate) bottom_levels: Slots<BottomLevel>,
	pub(crate) top_levels: Slots<TopLevel>,
	pub(crate) scratch: [ScratchArena; MAX_FRAMES_IN_FLIGHT],
	/// Counts the compactions that took effect. A top-level copy built at an epoch names the bottom-level storage of
	/// that epoch.
	pub(crate) epoch: u64,
	pub(crate) pending_compactions: Vec<PendingCompaction>,
	/// Copies into compacted storage that the next recording records.
	pub(crate) ready_compactions: Vec<CompactionCopy>,
	compaction_sources: Vec<CompactionSource>,
}

impl Default for AccelerationStructures {
	fn default() -> Self {
		Self {
			bottom_levels: Slots::new(),
			top_levels: Slots::new(),
			scratch: Default::default(),
			epoch: 0,
			pending_compactions: Vec::new(),
			ready_compactions: Vec::new(),
			compaction_sources: Vec::new(),
		}
	}
}

impl AccelerationStructures {
	pub(crate) fn bottom_level(&self, handle: BottomLevelAccelerationStructureHandle) -> &BottomLevel {
		self.bottom_levels
			.get(handle.bits())
			.unwrap_or_else(|| stale_handle("bottom-level"))
	}

	pub(crate) fn top_level(&self, handle: TopLevelAccelerationStructureHandle) -> &TopLevel {
		self.top_levels
			.get(handle.bits())
			.unwrap_or_else(|| stale_handle("top-level"))
	}

	/// Moves the structure `copy` compacts into its compacted storage, keeping the storage it replaces until every
	/// top-level copy built before now was built again. Returns `copy` when its structure was destroyed meanwhile.
	pub(crate) fn replace_bottom_level(&mut self, copy: CompactionCopy) -> Option<CompactionCopy> {
		let Some(bottom_level) = self.bottom_levels.get_mut(copy.handle) else {
			return Some(copy);
		};
		let resource = std::mem::replace(&mut bottom_level.resource, copy.resource);
		bottom_level.address = copy.address;
		bottom_level.size = copy.size;
		self.epoch += 1;
		self.compaction_sources.push(CompactionSource {
			resource,
			epoch: self.epoch,
		});
		None
	}

	/// Takes the replaced storage that no top-level copy names anymore: every copy that was ever built was built
	/// again since the replacement.
	pub(crate) fn take_released_compaction_sources(&mut self) -> impl Iterator<Item = CompactionSource> + '_ {
		let oldest_build = self
			.top_levels
			.values()
			.flat_map(|top_level| &top_level.copies)
			.filter_map(|copy| copy.built_epoch.get())
			.min()
			.unwrap_or(u64::MAX);
		self.compaction_sources
			.extract_if(.., move |source| source.epoch <= oldest_build)
	}
}

fn stale_handle(level: &str) -> ! {
	panic!(
		"The {level} acceleration structure handle is stale. The most likely cause is that the structure was destroyed before this call. Stop using a handle once you destroy its structure, and create a new structure instead."
	)
}

/// Returns the size of one index of `format`.
pub(crate) fn index_size(format: crate::DataTypes) -> usize {
	match format {
		crate::DataTypes::U16 => 2,
		_ => 4,
	}
}

/// Returns the bytes the indices of `geometry` take.
pub(crate) fn index_bytes(geometry: &crate::rt::TriangleGeometry) -> usize {
	geometry.triangle_count as usize * 3 * index_size(geometry.index_format)
}

/// Describes `geometry` for a build that reads its positions at `vertices` and its indices at `indices`.
///
/// Every geometry is opaque, so no build or trace runs an any-hit shader.
pub(crate) fn triangle_geometry(
	geometry: &crate::rt::TriangleGeometry,
	vertices: u64,
	indices: u64,
) -> d3d12::D3D12_RAYTRACING_GEOMETRY_DESC {
	let vertex_count = geometry.positions.size / geometry.positions.stride;
	d3d12::D3D12_RAYTRACING_GEOMETRY_DESC {
		Type: d3d12::D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
		Flags: d3d12::D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
		Anonymous: d3d12::D3D12_RAYTRACING_GEOMETRY_DESC_0 {
			Triangles: d3d12::D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
				Transform3x4: 0,
				IndexFormat: match geometry.index_format {
					crate::DataTypes::U16 => windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT,
					_ => windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R32_UINT,
				},
				VertexFormat: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R32G32B32_FLOAT,
				IndexCount: geometry.triangle_count * 3,
				VertexCount: vertex_count as u32,
				IndexBuffer: indices,
				VertexBuffer: d3d12::D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
					StartAddress: vertices,
					StrideInBytes: geometry.positions.stride as u64,
				},
			},
		},
	}
}

/// Describes the bottom-level build of `geometries`, or, with `geometries` empty, the top-level build of
/// `instance_count` instance records at `instances`.
pub(crate) fn build_inputs(
	geometries: &[d3d12::D3D12_RAYTRACING_GEOMETRY_DESC],
	instances: u64,
	instance_count: u32,
) -> d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
	let fast_trace = d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE;
	if geometries.is_empty() {
		return d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
			Flags: fast_trace,
			NumDescs: instance_count,
			DescsLayout: d3d12::D3D12_ELEMENTS_LAYOUT_ARRAY,
			Anonymous: d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
				InstanceDescs: instances,
			},
		};
	}
	d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
		Type: d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
		Flags: fast_trace | d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_ALLOW_COMPACTION,
		NumDescs: geometries.len() as u32,
		DescsLayout: d3d12::D3D12_ELEMENTS_LAYOUT_ARRAY,
		Anonymous: d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
			pGeometryDescs: geometries.as_ptr(),
		},
	}
}

/// Returns the record a top-level build reads for `instance`, which places the bottom-level structure at `address`.
pub(crate) fn instance_record(instance: &crate::rt::Instance, address: u64) -> d3d12::D3D12_RAYTRACING_INSTANCE_DESC {
	// Both D3D12 and the instance store the transform as three rows of four floats.
	let transform = std::array::from_fn(|index| instance.transform[index / 4][index % 4]);
	d3d12::D3D12_RAYTRACING_INSTANCE_DESC {
		Transform: transform,
		// InstanceID in the low 24 bits, InstanceMask in the high 8.
		_bitfield1: instance.custom_index | (u32::from(instance.mask) << 24),
		// InstanceContributionToHitGroupIndex in the low 24 bits, no flags in the high 8.
		_bitfield2: instance.hit_group_offset,
		AccelerationStructure: address,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn instance_records_keep_every_field() {
		let instance = crate::rt::Instance {
			transform: [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0]],
			custom_index: crate::rt::INSTANCE_INDEX_LIMIT - 1,
			mask: 0x81,
			hit_group_offset: 70_000,
			bottom_level: BottomLevelAccelerationStructureHandle::from_bits(1),
		};
		let record = instance_record(&instance, 0xABC0);
		assert_eq!(record.Transform[3], 4.0);
		assert_eq!(record.Transform[11], 12.0);
		assert_eq!(record._bitfield1 & 0xFF_FFFF, crate::rt::INSTANCE_INDEX_LIMIT - 1);
		assert_eq!(record._bitfield1 >> 24, 0x81);
		assert_eq!(record._bitfield2 & 0xFF_FFFF, 70_000);
		assert_eq!(record._bitfield2 >> 24, 0);
		assert_eq!(record.AccelerationStructure, 0xABC0);
		assert_eq!(INSTANCE_SIZE, 64);
	}

	#[test]
	fn scratch_ranges_are_aligned_and_apart_until_the_arena_rewinds() {
		let mut arena = ScratchArena::default();
		assert_eq!(arena.allocate(16, 256), None);
		assert_eq!(arena.grown_size(16), SMALLEST_SCRATCH_BUFFER);
		arena.replace(None, 0x1000, 1024);
		assert_eq!(arena.allocate(100, 256), Some(0x1000));
		assert_eq!(arena.allocate(100, 256), Some(0x1100));
		assert_eq!(arena.allocate(600, 256), None);
		assert_eq!(arena.remaining(), 1024 - 0x164);
		arena.reset();
		assert_eq!(arena.allocate(600, 256), Some(0x1000));
		assert_eq!(arena.grown_size(4 * SMALLEST_SCRATCH_BUFFER), 4 * SMALLEST_SCRATCH_BUFFER);
	}
}
