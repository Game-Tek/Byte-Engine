//! The `acceleration_structures` module keeps a Vulkan context's acceleration structures from creation, through their
//! build and compaction, to their release once no frame can read them.
//!
//! The context creates and destroys structures in `context/acceleration_structures.rs`, recordings build them in
//! `command_buffer/operations/acceleration_structures.rs`, and [`super::Context::advance_acceleration_structures`]
//! compacts them as their builds complete.
//!
//! Structures don't get an allocation each. They live in a few large blocks of [`StorageHeap`], so a scene of
//! thousands of meshes makes thousands of structures without making thousands of allocations, and a small structure
//! doesn't take a whole page.

use std::cell::Cell;

use ash::vk;
use smallvec::SmallVec;

use crate::slots::Slots;
use crate::{
	BottomLevelAccelerationStructureHandle, MAX_FRAMES_IN_FLIGHT, TopLevelAccelerationStructureHandle,
	graphics_hardware_interface,
};

/// The alignment Vulkan requires of an acceleration structure's offset in its buffer.
pub(crate) const STRUCTURE_ALIGNMENT: u64 = 256;

/// The smallest block [`StorageHeap`] adds, so the structures of a scene share a few allocations.
pub(crate) const STORAGE_BLOCK_SIZE: u64 = 32 << 20;

/// The smallest scratch buffer an arena creates, so a frame of small builds doesn't grow it build by build.
const SMALLEST_SCRATCH_BUFFER: u64 = 1 << 20;

/// The byte size of one instance record a top-level build reads.
pub(crate) const INSTANCE_SIZE: u64 = size_of::<vk::AccelerationStructureInstanceKHR>() as u64;

/// The `BuildState` enum follows a bottom-level structure through its one build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BuildState {
	/// No executed recording built the structure, so a recording may build it.
	Unbuilt,
	/// A recording that has not executed yet holds the structure's build.
	Recorded,
	/// The recording that built the structure executed.
	Built,
}

/// The `StorageRange` struct is where one acceleration structure lives: a range of one block of [`StorageHeap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StorageRange {
	pub(crate) block: u32,
	pub(crate) offset: u64,
	pub(crate) size: u64,
}

/// The `StorageBlock` struct is one buffer of acceleration structure storage, with the bytes no structure uses.
struct StorageBlock {
	buffer: vk::Buffer,
	free: utils::RangeAllocator,
}

/// The `StorageHeap` struct exists so acceleration structures share a few large device-local buffers instead of each
/// taking an allocation.
///
/// It only does the bookkeeping: [`Self::allocate`] finds a free range, and the context adds a block with
/// [`Self::add_block`] when none fits, because only the context can create buffers.
#[derive(Default)]
pub(crate) struct StorageHeap {
	blocks: Vec<StorageBlock>,
}

impl StorageHeap {
	/// Takes `size` bytes for one structure from the first block with room, or returns `None` when no block has it.
	pub(crate) fn allocate(&mut self, size: u64) -> Option<StorageRange> {
		self.blocks.iter_mut().enumerate().find_map(|(block, storage)| {
			let taken = storage.free.take(size as usize, STRUCTURE_ALIGNMENT as usize)?;
			Some(StorageRange {
				block: block as u32,
				offset: taken.start as u64,
				size,
			})
		})
	}

	/// Adds a block of `size` bytes backed by `buffer`, and returns its index. The context frees the block's memory
	/// with its other allocations when it drops.
	pub(crate) fn add_block(&mut self, buffer: vk::Buffer, size: u64) -> u32 {
		self.blocks.push(StorageBlock {
			buffer,
			free: utils::RangeAllocator::new(size as usize, 1),
		});
		(self.blocks.len() - 1) as u32
	}

	/// Returns the bytes of `range` to its block. Call it once no command reads the structure that lived there.
	pub(crate) fn free(&mut self, range: StorageRange) {
		let start = range.offset as usize;
		self.blocks[range.block as usize]
			.free
			.give_back(start..start + range.size as usize);
	}

	/// Returns the buffer that `range` lies in.
	pub(crate) fn buffer(&self, range: StorageRange) -> vk::Buffer {
		self.blocks[range.block as usize].buffer
	}

	/// Returns every block's buffer, for the context to destroy when it drops.
	pub(crate) fn buffers(&self) -> impl Iterator<Item = vk::Buffer> + '_ {
		self.blocks.iter().map(|block| block.buffer)
	}
}

/// The `BottomLevel` struct owns one bottom-level acceleration structure and the geometry its build reads.
pub(crate) struct BottomLevel {
	pub(crate) structure: vk::AccelerationStructureKHR,
	/// The value instance records name `structure` by, read once so writing instances calls no function.
	pub(crate) address: vk::DeviceAddress,
	pub(crate) storage: StorageRange,
	pub(crate) geometries: Box<[crate::rt::TriangleGeometry]>,
	pub(crate) build_scratch_size: u64,
	/// Recordings change it while they borrow the structures immutably.
	pub(crate) build_state: Cell<BuildState>,
}

/// The `TopLevel` struct owns the copies of one top-level acceleration structure, one for each frame in flight.
pub(crate) struct TopLevel {
	/// The copy a frame builds and reads, by the frame's sequence index.
	pub(crate) copies: SmallVec<[TopLevelCopy; MAX_FRAMES_IN_FLIGHT]>,
	pub(crate) max_instance_count: u32,
	pub(crate) build_scratch_size: u64,
}

/// The `TopLevelCopy` struct is the copy of a top-level structure that one frame sequence builds and traces.
pub(crate) struct TopLevelCopy {
	pub(crate) structure: vk::AccelerationStructureKHR,
	/// The value descriptors name `structure` by.
	pub(crate) address: vk::DeviceAddress,
	pub(crate) storage: StorageRange,
	/// The instance records the copy's builds read. The CPU writes them while it records a build, which is safe because
	/// the frame that built the copy before was this sequence's previous frame, which completed before this one began.
	pub(crate) instances: InstanceBuffer,
	/// The [`AccelerationStructures::epoch`] in which the copy's last executed build was recorded, or `None` before its
	/// first build. The copy names the bottom-level storage that compactions in later epochs replaced.
	pub(crate) built_epoch: Cell<Option<u64>>,
	/// The frame that recorded the copy's last build. A second build in the same frame would overwrite the instance
	/// records the first one reads before the GPU runs it.
	pub(crate) recorded_frame: Cell<Option<u64>>,
}

/// The `InstanceBuffer` struct is the CPU-written memory one top-level copy's builds read their instance records from.
pub(crate) struct InstanceBuffer {
	pub(crate) buffer: vk::Buffer,
	pub(crate) allocation: graphics_hardware_interface::AllocationHandle,
	pub(crate) address: vk::DeviceAddress,
	pub(crate) pointer: super::MappedMemoryPointer,
}

/// The `CompactionSource` struct is the storage of a bottom-level structure that compaction replaced, kept while
/// top-level copies built before the replacement may still name it.
pub(crate) struct CompactionSource {
	pub(crate) structure: vk::AccelerationStructureKHR,
	pub(crate) storage: StorageRange,
	/// The epoch the replacement started. Copies built in it or later name the replacement instead.
	epoch: u64,
}

/// The `Completion` enum says which GPU work has to complete before a recording's results are readable.
#[derive(Clone, Copy)]
pub(crate) enum Completion {
	/// A frame recording, which completed once the context saw its frame complete.
	Frame(u64),
	/// A recording outside frames, which completed once its synchronizer's fence signaled.
	Synchronizer(crate::synchronizer::SynchronizerHandle),
}

/// The `SizeQuery` struct is the bottom-level builds of one call, whose compacted sizes the GPU writes into a query
/// pool after the builds.
pub(crate) struct SizeQuery {
	pub(crate) pool: vk::QueryPool,
	/// The structure each query reports, by query index.
	pub(crate) structures: SmallVec<[BottomLevelAccelerationStructureHandle; 8]>,
}

/// The `PendingCompaction` struct is the executed builds of one [`SizeQuery`], whose sizes become readable once their
/// recording completes.
pub(crate) struct PendingCompaction {
	pub(crate) query: SizeQuery,
	pub(crate) completion: Completion,
}

/// The `CompactionCopy` struct is one compaction to record: the copy of a built structure into smaller storage.
///
/// The structure keeps its storage until the recording that holds the copy executes, so a recording that is dropped
/// leaves the structure as it was and hands the copy back for the next one.
pub(crate) struct CompactionCopy {
	pub(crate) handle: BottomLevelAccelerationStructureHandle,
	pub(crate) structure: vk::AccelerationStructureKHR,
	pub(crate) address: vk::DeviceAddress,
	pub(crate) storage: StorageRange,
}

/// The `ScratchBuffer` struct is the device memory builds use while they run.
#[derive(Clone, Copy)]
pub(crate) struct ScratchBuffer {
	pub(crate) buffer: vk::Buffer,
	pub(crate) allocation: graphics_hardware_interface::AllocationHandle,
	pub(crate) address: vk::DeviceAddress,
	pub(crate) size: u64,
}

/// The `ScratchArena` struct hands acceleration structure builds the scratch memory they need, from one buffer that
/// grows to the most its frame sequence needed.
///
/// A context keeps one arena per frame sequence and rewinds it once that sequence's previous frame completed. Builds
/// in one frame take separate ranges, so they need no barrier between them.
#[derive(Default)]
pub(crate) struct ScratchArena {
	pub(crate) buffer: Option<ScratchBuffer>,
	cursor: u64,
}

impl ScratchArena {
	/// Rewinds the arena; the caller guarantees no command still uses its memory.
	pub(crate) fn reset(&mut self) {
		self.cursor = 0;
	}

	/// Returns how many bytes the buffer has left after the ranges handed out since the last reset.
	pub(crate) fn remaining(&self) -> u64 {
		self.buffer.map_or(0, |buffer| buffer.size - self.cursor)
	}

	/// Returns the address of `size` bytes at `alignment` that one build may use, apart from every other range handed
	/// out since the last reset, or `None` when the buffer has no room for them.
	pub(crate) fn allocate(&mut self, size: u64, alignment: u64) -> Option<vk::DeviceAddress> {
		let buffer = self.buffer?;
		// Scratch addresses must be aligned, not offsets, so the alignment applies to the address.
		let address = (buffer.address + self.cursor).next_multiple_of(alignment);
		let end = address - buffer.address + size.max(1);
		if end > buffer.size {
			return None;
		}
		self.cursor = end;
		Some(address)
	}

	/// Returns the size a replacement buffer needs so that `size` bytes at `alignment` fit, doubling the current one.
	pub(crate) fn grown_size(&self, size: u64, alignment: u64) -> u64 {
		let current = self.buffer.map_or(0, |buffer| buffer.size);
		(current * 2).max(size + alignment).max(SMALLEST_SCRATCH_BUFFER)
	}

	/// Puts `buffer` in place of the arena's buffer, rewound, and returns the buffer it replaced.
	pub(crate) fn replace(&mut self, buffer: ScratchBuffer) -> Option<ScratchBuffer> {
		self.cursor = 0;
		self.buffer.replace(buffer)
	}
}

/// The `AccelerationStructures` struct holds a context's acceleration structures by handle, together with the storage
/// compaction replaced that top-level copies may still name.
#[derive(Default)]
pub(crate) struct AccelerationStructures {
	pub(crate) bottom_levels: Slots<BottomLevel>,
	pub(crate) top_levels: Slots<TopLevel>,
	pub(crate) storage: StorageHeap,
	compaction_sources: Vec<CompactionSource>,
	/// Counts the compactions that replaced a structure, which orders them against top-level builds.
	pub(crate) epoch: u64,
	/// Executed builds whose compacted sizes aren't readable yet.
	pub(crate) pending_compactions: Vec<PendingCompaction>,
	/// Compactions the next recording records.
	pub(crate) ready_compactions: Vec<CompactionCopy>,
	/// The scratch memory of each frame sequence's builds.
	pub(crate) scratch: [ScratchArena; MAX_FRAMES_IN_FLIGHT],
	/// Scratch buffers of recordings outside frames, kept until their synchronizer signals.
	pub(crate) transient_scratch: Vec<(Completion, ScratchBuffer)>,
	/// The emptied box of the last settled [`RecordedBuilds`], which the next recording that builds takes, so a frame
	/// that builds allocates none.
	pub(crate) spare_builds: Option<Box<RecordedBuilds>>,
}

impl AccelerationStructures {
	/// Takes the work out of `builds` and keeps its box for the next recording that builds.
	pub(crate) fn reuse_builds(&mut self, builds: &mut Box<RecordedBuilds>) -> RecordedBuilds {
		let work = std::mem::take(&mut **builds);
		self.spare_builds = Some(std::mem::take(builds));
		work
	}

	/// Returns the bottom-level structure `handle` names, or panics when it was destroyed.
	pub(crate) fn bottom_level(&self, handle: BottomLevelAccelerationStructureHandle) -> &BottomLevel {
		self.bottom_levels
			.get(handle.0)
			.unwrap_or_else(|| stale_handle("bottom-level"))
	}

	/// Returns the top-level structure `handle` names, or panics when it was destroyed.
	pub(crate) fn top_level(&self, handle: TopLevelAccelerationStructureHandle) -> &TopLevel {
		self.top_levels.get(handle.0).unwrap_or_else(|| stale_handle("top-level"))
	}

	/// Puts the compacted storage of `copy` in place of the storage of the structure it compacts, and keeps the
	/// replaced storage until every top-level copy built before now was built again.
	///
	/// Returns `false`, and changes nothing, when the structure was destroyed since the copy was queued.
	pub(crate) fn replace_bottom_level(&mut self, copy: &CompactionCopy) -> bool {
		let Some(bottom_level) = self.bottom_levels.get_mut(copy.handle.0) else {
			return false;
		};
		let structure = std::mem::replace(&mut bottom_level.structure, copy.structure);
		let storage = std::mem::replace(&mut bottom_level.storage, copy.storage);
		bottom_level.address = copy.address;
		self.epoch += 1;
		self.compaction_sources.push(CompactionSource {
			structure,
			storage,
			epoch: self.epoch,
		});
		true
	}

	/// Takes every replaced structure no top-level copy names any more, so the caller can destroy each once the frames
	/// recorded before now complete.
	///
	/// A copy names a replaced structure until a build that executed after the replacement writes it again.
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

	/// Takes every replaced structure, released or not, for the context to destroy when it drops.
	pub(crate) fn take_all_compaction_sources(&mut self) -> std::vec::Drain<'_, CompactionSource> {
		self.compaction_sources.drain(..)
	}
}

/// Panics for a handle whose structure was destroyed.
fn stale_handle(level: &str) -> ! {
	panic!(
		"The {level} acceleration structure handle is stale. The most likely cause is that the structure was destroyed before this call. Stop using a handle once you destroy its structure, and create a new structure instead."
	)
}

/// The `RecordedBuilds` struct is the acceleration structure work one recording holds until it executes or is dropped.
///
/// A recording fills it as it records builds and compactions, and whoever submits or drops the recording settles it
/// with [`Context::submit_recorded_builds`](super::Context::submit_recorded_builds) or
/// [`Context::abandon_recorded_builds`](super::Context::abandon_recorded_builds).
#[derive(Default)]
pub(crate) struct RecordedBuilds {
	pub(crate) size_queries: SmallVec<[SizeQuery; 1]>,
	/// The top-level copies the recording builds, by handle and copy, with the epoch whose bottom-level storage their
	/// instance records name.
	pub(crate) top_levels: SmallVec<[(TopLevelAccelerationStructureHandle, u8, u64); 2]>,
	pub(crate) compactions: SmallVec<[CompactionCopy; 4]>,
	/// The scratch buffer of a recording outside frames, which only it uses.
	pub(crate) transient_scratch: ScratchArena,
	/// Scratch buffers of a recording outside frames that a larger one replaced, which its earlier builds still use.
	pub(crate) retired_scratch: SmallVec<[ScratchBuffer; 1]>,
}

/// Returns the bytes one index of `format` occupies: 2 for [`crate::DataTypes::U16`], otherwise 4.
pub(crate) fn index_size(format: crate::DataTypes) -> usize {
	match format {
		crate::DataTypes::U16 => 2,
		_ => 4,
	}
}

/// Returns the bytes the indices of `geometry` span.
pub(crate) fn index_bytes(geometry: &crate::rt::TriangleGeometry) -> usize {
	geometry.triangle_count as usize * 3 * index_size(geometry.index_format)
}

/// Describes one triangle geometry the way Vulkan sizes and builds it. `vertices` and `indices` are the device
/// addresses its ranges start at, which sizing ignores.
pub(crate) fn triangle_geometry(
	geometry: &crate::rt::TriangleGeometry,
	vertices: vk::DeviceAddress,
	indices: vk::DeviceAddress,
) -> vk::AccelerationStructureGeometryKHR<'static> {
	let vertex_count = geometry.positions.size / geometry.positions.stride;
	let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
		.vertex_format(vk::Format::R32G32B32_SFLOAT)
		.vertex_data(vk::DeviceOrHostAddressConstKHR {
			device_address: vertices,
		})
		.vertex_stride(geometry.positions.stride as vk::DeviceSize)
		.max_vertex(vertex_count.saturating_sub(1) as u32)
		.index_type(match geometry.index_format {
			crate::DataTypes::U16 => vk::IndexType::UINT16,
			_ => vk::IndexType::UINT32,
		})
		.index_data(vk::DeviceOrHostAddressConstKHR { device_address: indices });
	vk::AccelerationStructureGeometryKHR::default()
		.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
		.geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
		// Every triangle is opaque, so traversal accepts the nearest hit without running an any-hit shader.
		.flags(vk::GeometryFlagsKHR::OPAQUE)
}

/// Describes a top-level build over the instance records at `instances`, the way Vulkan sizes and builds it.
pub(crate) fn instance_geometry(instances: vk::DeviceAddress) -> vk::AccelerationStructureGeometryKHR<'static> {
	vk::AccelerationStructureGeometryKHR::default()
		.geometry_type(vk::GeometryTypeKHR::INSTANCES)
		.geometry(vk::AccelerationStructureGeometryDataKHR {
			instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
				.array_of_pointers(false)
				.data(vk::DeviceOrHostAddressConstKHR {
					device_address: instances,
				}),
		})
}

/// The build flags of every bottom-level structure: each is built once and traced every frame after that, and
/// compacts itself once its build completes.
pub(crate) const BOTTOM_LEVEL_FLAGS: vk::BuildAccelerationStructureFlagsKHR = vk::BuildAccelerationStructureFlagsKHR::from_raw(
	vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE.as_raw()
		| vk::BuildAccelerationStructureFlagsKHR::ALLOW_COMPACTION.as_raw(),
);

/// The build flags of every top-level structure. Rays trace a frame's copy many times for each build, so the build
/// favors trace speed, as for bottom levels.
pub(crate) const TOP_LEVEL_FLAGS: vk::BuildAccelerationStructureFlagsKHR =
	vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE;

/// Converts `instance` into the record Vulkan reads, which names its bottom-level structure by `address`.
pub(crate) fn instance_record(
	instance: &crate::rt::Instance,
	address: vk::DeviceAddress,
) -> vk::AccelerationStructureInstanceKHR {
	// GHI transforms are three rows of four, which is the layout Vulkan reads too.
	let matrix = std::array::from_fn(|index| instance.transform[index / 4][index % 4]);
	vk::AccelerationStructureInstanceKHR {
		transform: vk::TransformMatrixKHR { matrix },
		instance_custom_index_and_mask: vk::Packed24_8::new(instance.custom_index, instance.mask),
		instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(instance.hit_group_offset, 0),
		acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: address },
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(size: u64) -> ScratchBuffer {
		ScratchBuffer {
			buffer: vk::Buffer::null(),
			allocation: graphics_hardware_interface::AllocationHandle(0),
			address: 0x1000,
			size,
		}
	}

	#[test]
	fn scratch_ranges_are_aligned_and_apart_until_the_arena_rewinds() {
		let mut arena = ScratchArena::default();
		assert_eq!(arena.allocate(16, 128), None);
		arena.replace(scratch(512));

		assert_eq!(arena.allocate(100, 128), Some(0x1000));
		assert_eq!(arena.allocate(100, 128), Some(0x1080));
		assert_eq!(arena.allocate(300, 128), None);
		arena.reset();
		assert_eq!(arena.allocate(300, 128), Some(0x1000));
	}

	#[test]
	fn storage_reuses_the_bytes_of_released_structures() {
		let mut heap = StorageHeap::default();
		assert_eq!(heap.allocate(1024), None);
		heap.add_block(vk::Buffer::null(), 4096);

		let first = heap.allocate(1000).unwrap();
		let second = heap.allocate(1000).unwrap();
		assert_eq!((first.offset, second.offset), (0, 1024));
		heap.free(first);
		assert_eq!(heap.allocate(1024).map(|range| range.offset), Some(0));
	}

	#[test]
	fn instance_records_keep_every_field() {
		let instance = crate::rt::Instance {
			transform: [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0]],
			custom_index: crate::rt::INSTANCE_INDEX_LIMIT - 1,
			mask: 0x81,
			hit_group_offset: 70_000,
			bottom_level: BottomLevelAccelerationStructureHandle(0),
		};
		let record = instance_record(&instance, 0xABC0);

		assert_eq!(record.transform.matrix[3], 4.0);
		assert_eq!(record.transform.matrix[11], 12.0);
		assert_eq!(
			record.instance_custom_index_and_mask.low_24(),
			crate::rt::INSTANCE_INDEX_LIMIT - 1
		);
		assert_eq!(record.instance_custom_index_and_mask.high_8(), 0x81);
		assert_eq!(record.instance_shader_binding_table_record_offset_and_flags.low_24(), 70_000);
		// SAFETY: The record was built with a device address.
		assert_eq!(unsafe { record.acceleration_structure_reference.device_handle }, 0xABC0);
	}
}
