use super::super::*;
use crate::metal::acceleration_structures::{
	BottomLevel, CompactionCopy, PendingCompaction, TopLevel, TopLevelCopy, instance_descriptor, primitive_descriptor,
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
		if let Some(name) = name.filter(|_| self.settings.debug_labels) {
			structure.setLabel(Some(&NSString::from_str(name)));
		}
		(structure, sizes.buildScratchBufferSize)
	}

	/// Panics unless the context supports ray tracing. `operation` names what needed it, such as "create a shader
	/// binding table".
	pub(in crate::metal) fn assert_ray_tracing(&self, operation: &str) {
		if !crate::context::Context::supports_ray_tracing(self) {
			rt::unsupported("Metal", operation);
		}
	}

	/// Returns the GPU range of `size` bytes from `buffer`, after checking that geometry may be built from it.
	fn geometry_range(&self, buffer: &crate::BufferDescriptor, size: usize, alignment: usize) -> mtl::MTL4BufferRange {
		let resource = self.buffers.get_single(buffer.buffer);
		rt::check_geometry_range(
			"Metal",
			buffer,
			size,
			alignment,
			resource.map(|resource| (resource.uses, resource.size)),
		);
		// The check panics for a missing buffer.
		let resource = resource.unwrap_or_else(|| unreachable!());
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
		rt::check_bottom_level_geometries("Metal", geometries);
		for geometry in geometries {
			// Builds resolve the same ranges later, so only creation checks their alignment.
			self.geometry_range(&geometry.positions.buffer_offset, geometry.positions.size, 4);
			self.geometry_range(
				&geometry.indices,
				rt::index_bytes(geometry),
				rt::index_size(geometry.index_format),
			);
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
			build_state: rt::BuildState::Unbuilt,
		});
		crate::BottomLevelAccelerationStructureHandle::from_bits(handle)
	}

	/// Creates a top-level structure with one copy for each frame in flight, each sized for `max_instance_count`
	/// instances.
	pub(crate) fn create_top_level(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.assert_ray_tracing("create a top-level acceleration structure");
		rt::check_max_instance_count("Metal", max_instance_count);
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
				built_epoch: None,
			});
		}
		let handle = self.acceleration_structures.top_levels.insert(TopLevel {
			copies,
			max_instance_count,
			build_scratch_size,
		});
		crate::TopLevelAccelerationStructureHandle::from_bits(handle)
	}

	/// Destroys the bottom-level structure `handle` names.
	///
	/// The queues keep its storage until the recordings that began before this call complete.
	pub(crate) fn destroy_bottom_level(&mut self, handle: crate::BottomLevelAccelerationStructureHandle) {
		let bottom_level = self.acceleration_structures.bottom_levels.remove(handle.bits()).unwrap_or_else(|| {
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
		self.acceleration_structures.top_levels.remove(handle.bits()).unwrap_or_else(|| {
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
				acceleration_structures
					.bottom_levels
					.get(pending.build.handle.bits())
					.is_none() || synchronizers.resource_mut(pending.synchronizer).poll(queues)
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
		let Some(bottom_level) = self.acceleration_structures.bottom_levels.get(handle.bits()) else {
			return;
		};
		let size = pending.compacted_size();
		// A size of zero would mean the build wrote nothing, and a size that is not smaller saves nothing.
		if size == 0 || size >= bottom_level.structure.size() {
			return;
		}
		let compacted = self.allocator.acceleration_structure(size);
		if let Some(label) = bottom_level.structure.label() {
			compacted.setLabel(Some(&*label));
		}
		pin_everywhere(&mut self.queues, &compacted);
		let source = self.acceleration_structures.replace_bottom_level(handle, compacted.clone());
		self.compaction_copies.push(CompactionCopy {
			slot: crate::slots::Slots::<BottomLevel>::index(handle.bits()),
			source,
			destination: compacted,
		});
	}
}
