//! The `acceleration_structures` module keeps a Metal context's acceleration structures from creation, through their
//! build and compaction, to their release once no frame can read them.
//!
//! The context creates and destroys structures in `context/resources/acceleration_structures.rs`, recordings build
//! them in `command_buffer/operations.rs`, and [`super::Context::advance_acceleration_structures`] compacts them as
//! their builds complete.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal as mtl;
use objc2_metal::{MTLAccelerationStructure as _, MTLBuffer as _, MTLDevice as _};
use smallvec::SmallVec;

use crate::slots::Slots;
use crate::{BottomLevelAccelerationStructureHandle, MAX_FRAMES_IN_FLIGHT, TopLevelAccelerationStructureHandle};

/// The byte size of one instance record Metal reads while building an instance acceleration structure.
pub(crate) const INSTANCE_DESCRIPTOR_SIZE: usize = size_of::<mtl::MTLIndirectAccelerationStructureInstanceDescriptor>();

/// The alignment of the scratch ranges builds receive.
const SCRATCH_ALIGNMENT: usize = 256;

/// The smallest scratch buffer an arena creates, so a frame of small builds does not grow it build by build.
const SMALLEST_SCRATCH_BUFFER: usize = 1 << 20;

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

/// The `BottomLevel` struct owns the storage of one bottom-level acceleration structure and the geometry its build
/// reads.
pub(crate) struct BottomLevel {
	pub(crate) structure: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	/// The value instance records name `structure` by, read once so writing instances sends no messages.
	pub(crate) resource_id: mtl::MTLResourceID,
	pub(crate) geometries: Box<[crate::rt::TriangleGeometry]>,
	pub(crate) build_scratch_size: usize,
	/// Recordings change it while they borrow the structures immutably.
	pub(crate) build_state: Cell<BuildState>,
}

/// The `TopLevel` struct owns the copies of one top-level acceleration structure, one for each frame in flight.
pub(crate) struct TopLevel {
	/// The copy a frame builds and reads, by the frame's sequence index.
	pub(crate) copies: SmallVec<[TopLevelCopy; MAX_FRAMES_IN_FLIGHT]>,
	pub(crate) max_instance_count: u32,
	pub(crate) build_scratch_size: usize,
}

/// The `TopLevelCopy` struct is the copy of a top-level structure that one frame sequence builds and traces.
pub(crate) struct TopLevelCopy {
	pub(crate) structure: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	/// The [`AccelerationStructures::epoch`] in which the copy's last executed build was recorded, or `None` before its
	/// first build. The copy names the bottom-level storage that compactions in later epochs replaced.
	pub(crate) built_epoch: Cell<Option<u64>>,
}

/// The `CompactionSource` struct is a bottom-level structure that compaction replaced, kept while top-level copies
/// built before the replacement may still name it.
struct CompactionSource {
	structure: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	/// The epoch the replacement started. Copies built in it or later name the replacement instead.
	epoch: u64,
}

/// The `AccelerationStructures` struct holds a context's acceleration structures by handle, together with the
/// structures compaction replaced that top-level copies may still name.
///
/// Destroyed structures need no list of their own: the queues' residency keeps an allocation until the recordings
/// that began before it was released complete.
#[derive(Default)]
pub(crate) struct AccelerationStructures {
	pub(crate) bottom_levels: Slots<BottomLevel>,
	pub(crate) top_levels: Slots<TopLevel>,
	compaction_sources: Vec<CompactionSource>,
	/// Counts the compactions that replaced a structure, which orders them against top-level builds.
	pub(crate) epoch: u64,
}

impl AccelerationStructures {
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

	/// Returns copy `copy` of the top-level structure `handle` names, or panics when it was destroyed.
	pub(crate) fn top_level_copy(
		&self,
		handle: TopLevelAccelerationStructureHandle,
		copy: u8,
	) -> &Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>> {
		&self.top_level(handle).copies[copy as usize].structure
	}

	/// Puts `compacted` in place of the storage of the bottom-level structure `handle` names, and keeps the replaced
	/// storage until every top-level copy built before now was built again.
	///
	/// Returns the replaced storage, which the copy into `compacted` reads.
	pub(crate) fn replace_bottom_level(
		&mut self,
		handle: BottomLevelAccelerationStructureHandle,
		compacted: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	) -> Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>> {
		let bottom_level = self
			.bottom_levels
			.get_mut(handle.0)
			.unwrap_or_else(|| stale_handle("bottom-level"));
		bottom_level.resource_id = compacted.gpuResourceID();
		let source = std::mem::replace(&mut bottom_level.structure, compacted);
		self.epoch += 1;
		self.compaction_sources.push(CompactionSource {
			structure: source.clone(),
			epoch: self.epoch,
		});
		source
	}

	/// Lets go of every replaced structure no top-level copy names any more, handing each to `unpin` first.
	///
	/// A copy names a replaced structure until a build that executed after the replacement writes it again. Frames
	/// recorded before that build may still read the old contents, which the residency `unpin` hands the structure to
	/// keeps for them.
	pub(crate) fn release_compaction_sources(
		&mut self,
		mut unpin: impl FnMut(&ProtocolObject<dyn mtl::MTLAccelerationStructure>),
	) {
		if self.compaction_sources.is_empty() {
			return;
		}
		let oldest_build = self
			.top_levels
			.values()
			.flat_map(|top_level| &top_level.copies)
			.filter_map(|copy| copy.built_epoch.get())
			.min()
			.unwrap_or(u64::MAX);
		self.compaction_sources.retain(|source| {
			if source.epoch > oldest_build {
				return true;
			}
			unpin(&source.structure);
			false
		});
	}
}

/// Panics for a handle whose structure was destroyed.
fn stale_handle(level: &str) -> ! {
	panic!(
		"The {level} acceleration structure handle is stale. The most likely cause is that the structure was destroyed before this call. Stop using a handle once you destroy its structure, and create a new structure instead."
	)
}

/// The `RecordedBuild` struct is one bottom-level build a recording holds, with the shared memory Metal writes the
/// structure's compacted size to.
pub(crate) struct RecordedBuild {
	pub(crate) handle: BottomLevelAccelerationStructureHandle,
	pub(crate) size_buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
	pub(crate) size_offset: usize,
}

/// The `RecordedBuilds` struct is the acceleration-structure builds one recording holds until it executes or is
/// dropped.
///
/// A recording fills it as it records builds, and whoever submits or drops the recording settles it with
/// [`Self::submit`] or [`Self::abandon`].
#[derive(Default)]
pub(crate) struct RecordedBuilds {
	pub(crate) bottom_levels: SmallVec<[RecordedBuild; 4]>,
	/// The top-level copies the recording builds, by handle and copy, with the epoch whose bottom-level storage their
	/// instance records name.
	pub(crate) top_levels: SmallVec<[(TopLevelAccelerationStructureHandle, u8, u64); 2]>,
}

impl RecordedBuilds {
	/// Marks the builds executed: bottom-level structures become built and wait in `pending` for compaction, and
	/// top-level copies record the epoch their instance records were written in.
	pub(crate) fn submit(
		self,
		structures: &AccelerationStructures,
		synchronizer: crate::synchronizer::SynchronizerHandle,
		pending: &mut Vec<PendingCompaction>,
	) {
		for build in self.bottom_levels {
			// A structure destroyed after its build was recorded still builds; its compaction finds the handle stale.
			if let Some(bottom_level) = structures.bottom_levels.get(build.handle.0) {
				debug_assert_eq!(bottom_level.build_state.get(), BuildState::Recorded);
				bottom_level.build_state.set(BuildState::Built);
			}
			pending.push(PendingCompaction { build, synchronizer });
		}
		for (handle, copy, epoch) in self.top_levels {
			if let Some(top_level) = structures.top_levels.get(handle.0) {
				top_level.copies[copy as usize].built_epoch.set(Some(epoch));
			}
		}
	}

	/// Returns the bottom-level structures to the unbuilt state after their recording was dropped without executing.
	///
	/// Top-level copies keep what their last executed build wrote, so they need nothing.
	pub(crate) fn abandon(self, structures: &AccelerationStructures) {
		for build in self.bottom_levels {
			if let Some(bottom_level) = structures.bottom_levels.get(build.handle.0) {
				bottom_level.build_state.set(BuildState::Unbuilt);
			}
		}
	}
}

/// The `PendingCompaction` struct is an executed bottom-level build whose compacted size becomes readable once the
/// synchronizer of its submission signals.
pub(crate) struct PendingCompaction {
	pub(crate) build: RecordedBuild,
	pub(crate) synchronizer: crate::synchronizer::SynchronizerHandle,
}

impl PendingCompaction {
	/// Returns the size Metal wrote for the structure to compact into.
	///
	/// Call it only after the build's submission completed.
	pub(crate) fn compacted_size(&self) -> usize {
		let pointer = self.build.size_buffer.contents().as_ptr().cast::<u8>();
		// SAFETY: The offset was allocated for eight bytes in this shared buffer, and the command that wrote them
		// completed, so the CPU reads a final value.
		let size = unsafe { pointer.add(self.build.size_offset).cast::<u64>().read_unaligned() };
		size as usize
	}
}

/// The `CompactionCopy` struct is one compaction to encode in the next internal submission.
pub(crate) struct CompactionCopy {
	/// The slot of the compacted bottom-level structure, which orders the copy against the structure's other uses.
	pub(crate) slot: u32,
	pub(crate) source: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
	pub(crate) destination: Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>,
}

/// The `ScratchArena` struct hands acceleration-structure builds the scratch memory they need, from one private buffer
/// that grows to the most its frame sequence has needed.
///
/// A frame owns one arena per sequence index and resets it once that sequence's previous frame completed. Recordings
/// outside frames share a transient arena that starts empty for each of them.
pub(crate) struct ScratchArena {
	buffer: Option<Retained<ProtocolObject<dyn mtl::MTLBuffer>>>,
	cursor: usize,
	debug_labels: bool,
}

impl ScratchArena {
	pub(crate) fn new(debug_labels: bool) -> Self {
		Self {
			buffer: None,
			cursor: 0,
			debug_labels,
		}
	}

	/// Rewinds the arena; the caller guarantees no command still uses its memory.
	pub(crate) fn reset(&mut self) {
		self.cursor = 0;
	}

	/// Drops the buffer so the next build starts a fresh one; commands that use the old buffer keep it alive.
	pub(crate) fn discard(&mut self) {
		self.buffer = None;
		self.cursor = 0;
	}

	/// Returns a buffer and the range of `size` bytes in it that one build may use, apart from every other range
	/// handed out since the last reset.
	///
	/// When the buffer is full, a larger one replaces it; the commands that used the old one retained it.
	pub(crate) fn allocate(
		&mut self,
		device: &ProtocolObject<dyn mtl::MTLDevice>,
		size: usize,
	) -> (&Retained<ProtocolObject<dyn mtl::MTLBuffer>>, mtl::MTL4BufferRange) {
		let size = size.max(1);
		let offset = self.cursor.next_multiple_of(SCRATCH_ALIGNMENT);
		let capacity = self.buffer.as_ref().map_or(0, |buffer| buffer.length());
		let offset = if offset + size <= capacity {
			offset
		} else {
			let capacity = (capacity * 2)
				.max(size.next_multiple_of(SCRATCH_ALIGNMENT))
				.max(SMALLEST_SCRATCH_BUFFER);
			let buffer = device
				.newBufferWithLength_options(capacity, mtl::MTLResourceOptions::StorageModePrivate)
				.expect(
					"Metal acceleration structure scratch allocation failed. The most likely cause is that the device is out of memory. Build fewer acceleration structures per frame.",
				);
			#[cfg(debug_assertions)]
			if self.debug_labels {
				use objc2_metal::MTLResource as _;
				buffer.setLabel(Some(&objc2_foundation::NSString::from_str("Acceleration Structure Scratch")));
			}
			self.buffer = Some(buffer);
			0
		};
		self.cursor = offset + size;
		let buffer = self
			.buffer
			.as_ref()
			.expect("The scratch arena has a buffer after it made room for the build.");
		let range = mtl::MTL4BufferRange {
			bufferAddress: buffer.gpuAddress() + offset as u64,
			length: size as u64,
		};
		(buffer, range)
	}
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

/// Describes a bottom-level structure over `geometries` the way Metal sizes and builds it.
///
/// `resolve` returns where a geometry's buffer range of the given size starts. Sizing and building resolve the same
/// ranges, so the storage a creation sizes fits the build.
pub(crate) fn primitive_descriptor(
	geometries: &[crate::rt::TriangleGeometry],
	mut resolve: impl FnMut(&crate::BufferDescriptor, usize) -> mtl::MTL4BufferRange,
) -> Retained<mtl::MTL4PrimitiveAccelerationStructureDescriptor> {
	let geometry_descriptors = geometries
		.iter()
		.enumerate()
		.map(|(index, geometry)| {
			let descriptor = mtl::MTL4AccelerationStructureTriangleGeometryDescriptor::new();
			descriptor.setVertexFormat(mtl::MTLAttributeFormat::Float3);
			descriptor.setIndexType(super::utils::to_index_type(geometry.index_format));
			// Every triangle is opaque, so traversal accepts the nearest hit without running an intersection function.
			descriptor.setOpaque(true);
			let vertices = resolve(&geometry.positions.buffer_offset, geometry.positions.size);
			let indices = resolve(&geometry.indices, index_bytes(geometry));
			// SAFETY: `resolve` checked both ranges against the buffers behind them, and the counts and stride describe
			// the geometry those ranges hold.
			unsafe {
				descriptor.setVertexBuffer(vertices);
				descriptor.setVertexStride(geometry.positions.stride);
				descriptor.setIndexBuffer(indices);
				descriptor.setTriangleCount(geometry.triangle_count as usize);
				// Metal reports a hit's function ID as its instance's offset plus its geometry's. Giving each geometry
				// its index makes that the record every backend computes: hit group offset plus geometry index.
				descriptor.setIntersectionFunctionTableOffset(index);
			}
			Retained::into_super(descriptor)
		})
		.collect::<SmallVec<[_; 8]>>();
	let descriptor = mtl::MTL4PrimitiveAccelerationStructureDescriptor::new();
	descriptor.setGeometryDescriptors(Some(&objc2_foundation::NSArray::from_retained_slice(&geometry_descriptors)));
	// A bottom-level structure is built once and traced every frame after that.
	descriptor.setUsage(mtl::MTLAccelerationStructureUsage::PreferFastIntersection);
	descriptor
}

/// Describes a top-level build over `instance_count` records starting at `instances`, the way Metal sizes and builds
/// it.
pub(crate) fn instance_descriptor(
	instances: mtl::MTL4BufferRange,
	instance_count: usize,
) -> Retained<mtl::MTL4InstanceAccelerationStructureDescriptor> {
	let descriptor = mtl::MTL4InstanceAccelerationStructureDescriptor::new();
	// SAFETY: The caller sized `instances` for `instance_count` records, or passes an empty range only to size.
	unsafe {
		descriptor.setInstanceDescriptorBuffer(instances);
		descriptor.setInstanceDescriptorStride(INSTANCE_DESCRIPTOR_SIZE);
		descriptor.setInstanceCount(instance_count);
	}
	descriptor.setInstanceDescriptorType(mtl::MTLAccelerationStructureInstanceDescriptorType::Indirect);
	descriptor
}

/// Converts `instance` into the record Metal reads, which names its bottom-level structure by `resource_id`.
pub(crate) fn instance_record(
	instance: &crate::rt::Instance,
	resource_id: mtl::MTLResourceID,
) -> mtl::MTLIndirectAccelerationStructureInstanceDescriptor {
	// GHI transforms are three rows of four; Metal reads four packed columns of three.
	let columns = std::array::from_fn(|column| mtl::MTLPackedFloat3 {
		x: instance.transform[0][column],
		y: instance.transform[1][column],
		z: instance.transform[2][column],
	});
	mtl::MTLIndirectAccelerationStructureInstanceDescriptor {
		transformationMatrix: mtl::MTLPackedFloat4x3 { columns },
		options: mtl::MTLAccelerationStructureInstanceOptions::empty(),
		mask: u32::from(instance.mask),
		intersectionFunctionTableOffset: instance.hit_group_offset,
		userID: instance.custom_index,
		accelerationStructureID: resource_id,
	}
}
