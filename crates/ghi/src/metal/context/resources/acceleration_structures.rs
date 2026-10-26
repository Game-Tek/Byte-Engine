use std::cell::Cell;

use super::super::*;
use crate::metal::acceleration_structures::{
	BottomLevel, BuildState, CompactionCopy, PendingCompaction, TopLevel, TopLevelCopy, index_bytes, index_size,
	instance_descriptor, primitive_descriptor,
};
use crate::rt;

/// Names an acceleration structure as the allocation queue residency tracks.
fn allocation(structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure>) -> &ProtocolObject<dyn mtl::MTLAllocation> {
	ProtocolObject::from_ref(structure)
}

/// Makes `structure` resident on every queue until [`unpin_everywhere`] releases it.
///
/// Rays reach bottom-level structures only through the instances of a top-level structure, so no command names them
/// for residency the way commands name the buffers and images they use.
fn pin_everywhere(queues: &mut [queue::StoredQueue], structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure>) {
	for queue in queues {
		queue.residency.pin(allocation(structure));
	}
}

/// Releases a pin [`pin_everywhere`] took. Each queue keeps the structure until its recordings that began before this
/// call complete.
fn unpin_everywhere(queues: &mut [queue::StoredQueue], structure: &ProtocolObject<dyn mtl::MTLAccelerationStructure>) {
	let address = std::ptr::from_ref(allocation(structure)).cast::<()>() as usize;
	for queue in queues {
		queue.residency.unpin(address);
	}
}

impl Context {
	/// Allocates storage for the acceleration structure `descriptor` describes, and returns it with the scratch bytes
	/// its build needs.
	fn allocate_acceleration_structure(
		&mut self,
		name: Option<&str>,
		descriptor: &mtl::MTLAccelerationStructureDescriptor,
	) -> (Retained<ProtocolObject<dyn mtl::MTLAccelerationStructure>>, usize) {
		let sizes = self.device.accelerationStructureSizesWithDescriptor(descriptor);
		let structure = self.allocator.acceleration_structure(sizes.accelerationStructureSize);
		#[cfg(debug_assertions)]
		if let Some(name) = name.filter(|_| self.settings.debug_labels) {
			structure.setLabel(Some(&NSString::from_str(name)));
		}
		#[cfg(not(debug_assertions))]
		let _ = name;
		(structure, sizes.buildScratchBufferSize)
	}

	/// Panics unless the context supports ray tracing. `operation` names what needed it, such as "create a shader
	/// binding table".
	pub(in crate::metal) fn assert_ray_tracing(&self, operation: &str) {
		assert!(
			crate::context::Context::supports_ray_tracing(self),
			"Metal could not {operation}. The most likely cause is that the device was created without the ray tracing feature, or that its GPU does not support ray tracing. Check Context::supports_ray_tracing before you {operation}, and request Features::ray_tracing when you create the device.",
		);
	}

	/// Returns the GPU range of `size` bytes from `buffer`, after checking that geometry may be built from it.
	fn geometry_range(&self, buffer: &crate::BufferDescriptor, size: usize, alignment: usize) -> mtl::MTL4BufferRange {
		let resource = self.buffers.get_single(buffer.buffer).expect(
			"Metal triangle geometry names a missing buffer. The most likely cause is that the buffer handle came from another context. Build geometry from buffers this context created.",
		);
		assert!(
			resource.uses.contains(crate::Uses::AccelerationStructureBuild),
			"Metal triangle geometry reads a buffer without the acceleration structure build use. The most likely cause is that the buffer was created for drawing only. Add Uses::AccelerationStructureBuild to the buffer's builder.",
		);
		assert!(
			buffer.offset.checked_add(size).is_some_and(|end| end <= resource.size),
			"Metal triangle geometry passes the end of its buffer. The most likely cause is that the triangle count, stride, or offset describes more data than the buffer holds. range_end={}, buffer_size={}. Reduce the count or fix the offset.",
			buffer.offset.saturating_add(size),
			resource.size,
		);
		assert!(
			buffer.offset.is_multiple_of(alignment),
			"Metal triangle geometry starts at a misaligned offset. The most likely cause is that the offset was not rounded to the element size. offset={}, alignment={alignment}. Align the offset to {alignment} bytes.",
			buffer.offset,
		);
		mtl::MTL4BufferRange {
			bufferAddress: resource.gpu_address + buffer.offset as u64,
			length: size as u64,
		}
	}

	/// Creates a bottom-level structure over `geometries`, pinned on every queue, after validating each geometry.
	pub(crate) fn create_bottom_level(
		&mut self,
		name: Option<&str>,
		geometries: &[rt::TriangleGeometry],
	) -> crate::BottomLevelAccelerationStructureHandle {
		self.assert_ray_tracing("create a bottom-level acceleration structure");
		assert!(
			!geometries.is_empty(),
			"Metal bottom-level acceleration structure creation failed. The most likely cause is that no triangle geometry was given. Pass at least one geometry.",
		);
		for geometry in geometries {
			assert!(
				matches!(geometry.index_format, crate::DataTypes::U16 | crate::DataTypes::U32),
				"Metal triangle geometry has an unsupported index format. The most likely cause is that the index format is not a 16-bit or 32-bit integer. Use DataTypes::U16 or DataTypes::U32.",
			);
			let stride = geometry.positions.stride;
			assert!(
				stride >= 12 && stride.is_multiple_of(4),
				"Metal triangle geometry has an unsupported position stride. The most likely cause is that the stride is smaller than three floats or not a multiple of 4 bytes. stride={stride}. Use a stride of at least 12 bytes that is a multiple of 4.",
			);
			// Builds resolve the same ranges later, so only creation checks their alignment.
			self.geometry_range(&geometry.positions.buffer_offset, geometry.positions.size, 4);
			self.geometry_range(&geometry.indices, index_bytes(geometry), index_size(geometry.index_format));
		}
		// Sizing reads only counts and formats, but the descriptor is the one a build would use.
		let descriptor = primitive_descriptor(geometries, |buffer, size| self.geometry_range(buffer, size, 1));
		let (structure, build_scratch_size) = self.allocate_acceleration_structure(name, &descriptor);
		pin_everywhere(&mut self.queues, &structure);
		let resource_id = structure.gpuResourceID();
		let handle = self.acceleration_structures.bottom_levels.insert(BottomLevel {
			structure,
			resource_id,
			geometries: geometries.into(),
			build_scratch_size,
			build_state: Cell::new(BuildState::Unbuilt),
		});
		crate::BottomLevelAccelerationStructureHandle(handle)
	}

	/// Creates a top-level structure with one copy for each frame in flight, each sized for `max_instance_count`
	/// instances.
	pub(crate) fn create_top_level(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.assert_ray_tracing("create a top-level acceleration structure");
		assert!(
			max_instance_count > 0,
			"Metal top-level acceleration structure creation failed. The most likely cause is that the maximum instance count is zero. Pass the largest number of instances a frame places.",
		);
		// Sizing reads the instance count; builds name their own instance records.
		let descriptor = instance_descriptor(
			mtl::MTL4BufferRange {
				bufferAddress: 0,
				length: 0,
			},
			max_instance_count as usize,
		);
		let mut copies = SmallVec::new();
		let mut build_scratch_size = 0;
		for _ in 0..self.frames {
			let (structure, scratch_size) = self.allocate_acceleration_structure(name, &descriptor);
			build_scratch_size = scratch_size;
			copies.push(TopLevelCopy {
				structure,
				built_epoch: Cell::new(None),
			});
		}
		let handle = self.acceleration_structures.top_levels.insert(TopLevel {
			copies,
			max_instance_count,
			build_scratch_size,
		});
		crate::TopLevelAccelerationStructureHandle(handle)
	}

	/// Destroys the bottom-level structure `handle` names.
	///
	/// The queues keep its storage until the recordings that began before this call complete.
	pub(crate) fn destroy_bottom_level(&mut self, handle: crate::BottomLevelAccelerationStructureHandle) {
		let bottom_level = self.acceleration_structures.bottom_levels.remove(handle.0).unwrap_or_else(|| {
			panic!(
				"The bottom-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		unpin_everywhere(&mut self.queues, &bottom_level.structure);
	}

	/// Destroys the top-level structure `handle` names.
	///
	/// Recordings retained the copies they built, and descriptor sets pin the copies they bind, so the queues keep
	/// each copy as long as a frame recorded before this call may read it.
	pub(crate) fn destroy_top_level(&mut self, handle: crate::TopLevelAccelerationStructureHandle) {
		self.acceleration_structures.top_levels.remove(handle.0).unwrap_or_else(|| {
			panic!(
				"The top-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
	}

	/// Compacts the bottom-level structures whose builds completed, and releases the storage compaction replaced once
	/// no top-level copy names it.
	///
	/// Polls without waiting. A frame calls it before it rewinds its upload pages, which hold the compacted sizes its
	/// builds wrote.
	pub(crate) fn advance_acceleration_structures(&mut self) {
		let Self {
			pending_compactions,
			synchronizers,
			queues,
			acceleration_structures,
			..
		} = self;
		let completed = pending_compactions
			.extract_if(.., |pending| {
				// A destroyed structure has nothing left to compact.
				acceleration_structures.bottom_levels.get(pending.build.handle.0).is_none()
					|| synchronizers.resource_mut(pending.synchronizer).poll(queues)
			})
			.collect::<SmallVec<[_; 4]>>();
		for pending in completed {
			self.compact(pending);
		}
		let queues = &mut self.queues;
		self.acceleration_structures
			.release_compaction_sources(|structure| unpin_everywhere(queues, structure));
	}

	/// Replaces a built bottom-level structure with a compacted copy when Metal reports a smaller size for it.
	///
	/// The copy runs in the next internal submission, which every later recording follows.
	fn compact(&mut self, pending: PendingCompaction) {
		let handle = pending.build.handle;
		let Some(bottom_level) = self.acceleration_structures.bottom_levels.get(handle.0) else {
			return;
		};
		let size = pending.compacted_size();
		// A size of zero would mean the build wrote nothing, and a size that is not smaller saves nothing.
		if size == 0 || size >= bottom_level.structure.size() {
			return;
		}
		let compacted = self.allocator.acceleration_structure(size);
		#[cfg(debug_assertions)]
		if let Some(label) = bottom_level.structure.label() {
			compacted.setLabel(Some(&*label));
		}
		pin_everywhere(&mut self.queues, &compacted);
		let source = self.acceleration_structures.replace_bottom_level(handle, compacted.clone());
		self.compaction_copies.push(CompactionCopy {
			slot: crate::slots::Slots::<BottomLevel>::index(handle.0),
			source,
			destination: compacted,
		});
	}
}
