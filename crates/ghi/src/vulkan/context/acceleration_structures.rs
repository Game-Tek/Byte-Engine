//! Creates, destroys, and compacts a Vulkan context's acceleration structures. Recordings build them in
//! `command_buffer/operations/acceleration_structures.rs`.

use super::*;
use crate::rt;
use crate::vulkan::acceleration_structures::{
	BOTTOM_LEVEL_FLAGS, BottomLevel, BuildState, CompactionCopy, Completion, INSTANCE_SIZE, InstanceBuffer, RecordedBuilds,
	STORAGE_BLOCK_SIZE, ScratchBuffer, StorageRange, TOP_LEVEL_FLAGS, TopLevel, TopLevelCopy, index_bytes, index_size,
	instance_geometry, triangle_geometry,
};
use crate::vulkan::device::RayTracing;

impl Context {
	/// Returns the device's ray tracing functions, or panics unless the context supports ray tracing. `operation`
	/// names what needed them, such as "create a shader binding table".
	pub(crate) fn ray_tracing(&self, operation: &str) -> &std::sync::Arc<RayTracing> {
		self.device.ray_tracing.as_ref().unwrap_or_else(|| {
			panic!(
				"Vulkan could not {operation}. The most likely cause is that the device was created without the ray tracing feature, or that its GPU does not support ray tracing. Check Context::supports_ray_tracing before you {operation}, and request Features::ray_tracing when you create the device."
			)
		})
	}

	/// Takes `size` bytes of acceleration structure storage, adding a block to the heap when no block has room.
	fn allocate_structure_storage(&mut self, size: u64) -> StorageRange {
		if let Some(range) = self.acceleration_structures.storage.allocate(size) {
			return range;
		}
		let block_size = size.max(STORAGE_BLOCK_SIZE);
		let (buffer, ..) = self.create_dedicated_buffer(
			Some("Acceleration Structures"),
			block_size as usize,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::GpuWrite,
		);
		self.acceleration_structures.storage.add_block(buffer.resource, block_size);
		self.acceleration_structures.storage.allocate(size).expect(
			"Vulkan acceleration structure storage has no room in a block made for it. The most likely cause is a fault in the storage heap.",
		)
	}

	/// Creates an acceleration structure of `ty` that occupies `size` bytes of storage, and returns it with the address
	/// instances and descriptors name it by and the storage it lives in.
	fn create_structure(
		&mut self,
		name: Option<&str>,
		ty: vk::AccelerationStructureTypeKHR,
		size: u64,
	) -> (vk::AccelerationStructureKHR, vk::DeviceAddress, StorageRange) {
		let storage = self.allocate_structure_storage(size);
		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(self.acceleration_structures.storage.buffer(storage))
			.offset(storage.offset)
			.size(size)
			.ty(ty);
		let functions = &self.ray_tracing("create an acceleration structure").acceleration_structure;
		// SAFETY: The storage range lies in a buffer with acceleration structure storage usage, at an aligned offset.
		let structure = unsafe { functions.create_acceleration_structure(&create_info, None) }.expect(
			"Vulkan acceleration structure creation failed. The most likely cause is that the device is out of memory. Build fewer or smaller structures.",
		);
		// SAFETY: The structure was just created.
		let address = unsafe {
			functions.get_acceleration_structure_device_address(
				&vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(structure),
			)
		};
		self.set_name(structure, name);
		(structure, address, storage)
	}

	/// Returns the storage and scratch sizes a build of `geometries` needs, where `primitive_counts` holds the most
	/// primitives each geometry has.
	fn build_sizes(
		&self,
		ty: vk::AccelerationStructureTypeKHR,
		flags: vk::BuildAccelerationStructureFlagsKHR,
		geometries: &[vk::AccelerationStructureGeometryKHR<'_>],
		primitive_counts: &[u32],
	) -> vk::AccelerationStructureBuildSizesInfoKHR<'static> {
		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(ty)
			.flags(flags)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.geometries(geometries);
		let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
		// SAFETY: Sizing reads only the counts, formats, and flags, which describe the builds this structure gets.
		unsafe {
			self.ray_tracing("size an acceleration structure")
				.acceleration_structure
				.get_acceleration_structure_build_sizes(
					vk::AccelerationStructureBuildTypeKHR::DEVICE,
					&build_info,
					Some(primitive_counts),
					&mut sizes,
				);
		}
		sizes
	}

	/// Checks that `size` bytes from `buffer` lie in a buffer that geometry may be built from, at a multiple of
	/// `alignment`.
	fn check_geometry_range(&self, buffer: &crate::BufferDescriptor, size: usize, alignment: usize) {
		let resource = self.buffers.get_single(buffer.buffer).expect(
			"Vulkan triangle geometry names a missing buffer. The most likely cause is that the buffer handle came from another context. Build geometry from buffers this context created.",
		);
		assert!(
			resource.uses.contains(crate::Uses::AccelerationStructureBuild),
			"Vulkan triangle geometry reads a buffer without the acceleration structure build use. The most likely cause is that the buffer was created for drawing only. Add Uses::AccelerationStructureBuild to the buffer's builder.",
		);
		assert!(
			buffer.offset.checked_add(size).is_some_and(|end| end <= resource.size),
			"Vulkan triangle geometry passes the end of its buffer. The most likely cause is that the triangle count, stride, or offset describes more data than the buffer holds. range_end={}, buffer_size={}. Reduce the count or fix the offset.",
			buffer.offset.saturating_add(size),
			resource.size,
		);
		assert!(
			buffer.offset.is_multiple_of(alignment),
			"Vulkan triangle geometry starts at a misaligned offset. The most likely cause is that the offset was not rounded to the element size. offset={}, alignment={alignment}. Align the offset to {alignment} bytes.",
			buffer.offset,
		);
	}

	/// Creates a bottom-level structure over `geometries`, sized for their build, after validating each geometry.
	pub(crate) fn create_bottom_level(
		&mut self,
		name: Option<&str>,
		geometries: &[rt::TriangleGeometry],
	) -> crate::BottomLevelAccelerationStructureHandle {
		self.ray_tracing("create a bottom-level acceleration structure");
		assert!(
			!geometries.is_empty(),
			"Vulkan bottom-level acceleration structure creation failed. The most likely cause is that no triangle geometry was given. Pass at least one geometry.",
		);
		for geometry in geometries {
			assert!(
				matches!(geometry.index_format, crate::DataTypes::U16 | crate::DataTypes::U32),
				"Vulkan triangle geometry has an unsupported index format. The most likely cause is that the index format is not a 16-bit or 32-bit integer. Use DataTypes::U16 or DataTypes::U32.",
			);
			let stride = geometry.positions.stride;
			assert!(
				stride >= 12 && stride.is_multiple_of(4),
				"Vulkan triangle geometry has an unsupported position stride. The most likely cause is that the stride is smaller than three floats or not a multiple of 4 bytes. stride={stride}. Use a stride of at least 12 bytes that is a multiple of 4.",
			);
			// Builds resolve the same ranges later, so only creation checks them.
			self.check_geometry_range(&geometry.positions.buffer_offset, geometry.positions.size, 4);
			self.check_geometry_range(&geometry.indices, index_bytes(geometry), index_size(geometry.index_format));
		}

		// Sizing ignores addresses, so the descriptions name none.
		let descriptions = geometries
			.iter()
			.map(|geometry| triangle_geometry(geometry, 0, 0))
			.collect::<SmallVec<[_; 8]>>();
		let triangle_counts = geometries
			.iter()
			.map(|geometry| geometry.triangle_count)
			.collect::<SmallVec<[_; 8]>>();
		let sizes = self.build_sizes(
			vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
			BOTTOM_LEVEL_FLAGS,
			&descriptions,
			&triangle_counts,
		);
		let (structure, address, storage) = self.create_structure(
			name,
			vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
			sizes.acceleration_structure_size,
		);
		let handle = self.acceleration_structures.bottom_levels.insert(BottomLevel {
			structure,
			address,
			storage,
			geometries: geometries.into(),
			build_scratch_size: sizes.build_scratch_size,
			build_state: std::cell::Cell::new(BuildState::Unbuilt),
		});
		crate::BottomLevelAccelerationStructureHandle(handle)
	}

	/// Creates a top-level structure with one copy for each frame in flight, each sized for `max_instance_count`
	/// instances and with instance records of its own.
	pub(crate) fn create_top_level(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.ray_tracing("create a top-level acceleration structure");
		assert!(
			max_instance_count > 0,
			"Vulkan top-level acceleration structure creation failed. The most likely cause is that the maximum instance count is zero. Pass the largest number of instances a frame places.",
		);
		// Sizing ignores the address of the instance records; builds name their own.
		let sizes = self.build_sizes(
			vk::AccelerationStructureTypeKHR::TOP_LEVEL,
			TOP_LEVEL_FLAGS,
			&[instance_geometry(0)],
			&[max_instance_count],
		);
		let mut copies = SmallVec::new();
		for _ in 0..self.frames {
			let (structure, address, storage) = self.create_structure(
				name,
				vk::AccelerationStructureTypeKHR::TOP_LEVEL,
				sizes.acceleration_structure_size,
			);
			let (buffer, allocation, instances_address, pointer) = self.create_dedicated_buffer(
				name,
				(u64::from(max_instance_count) * INSTANCE_SIZE) as usize,
				vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
					| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
				crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
			);
			copies.push(TopLevelCopy {
				structure,
				address,
				storage,
				instances: InstanceBuffer {
					buffer: buffer.resource,
					allocation,
					address: instances_address,
					pointer: crate::vulkan::MappedMemoryPointer(pointer),
				},
				built_epoch: std::cell::Cell::new(None),
				recorded_frame: std::cell::Cell::new(None),
			});
		}
		let handle = self.acceleration_structures.top_levels.insert(TopLevel {
			copies,
			max_instance_count,
			build_scratch_size: sizes.build_scratch_size,
		});
		crate::TopLevelAccelerationStructureHandle(handle)
	}

	/// Destroys the bottom-level structure `handle` names once the frames recorded before this call complete.
	pub(crate) fn destroy_bottom_level(&mut self, handle: crate::BottomLevelAccelerationStructureHandle) {
		let bottom_level = self.acceleration_structures.bottom_levels.remove(handle.0).unwrap_or_else(|| {
			panic!(
				"The bottom-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		self.defer_destruction(Tasks::DeleteAccelerationStructure {
			handle: bottom_level.structure,
			storage: bottom_level.storage,
		});
	}

	/// Destroys the top-level structure `handle` names, and every copy, once the frames recorded before this call
	/// complete.
	pub(crate) fn destroy_top_level(&mut self, handle: crate::TopLevelAccelerationStructureHandle) {
		let top_level = self.acceleration_structures.top_levels.remove(handle.0).unwrap_or_else(|| {
			panic!(
				"The top-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		for copy in top_level.copies {
			self.defer_destruction(Tasks::DeleteAccelerationStructure {
				handle: copy.structure,
				storage: copy.storage,
			});
			self.retire_dedicated_buffer(copy.instances.buffer, copy.instances.allocation);
		}
	}

	/// Creates a scratch buffer of `size` bytes that builds read and write on the device.
	pub(crate) fn create_scratch_buffer(&mut self, size: u64) -> ScratchBuffer {
		let (buffer, allocation, address, _) = self.create_dedicated_buffer(
			Some("Acceleration Structure Scratch"),
			size as usize,
			vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::GpuWrite,
		);
		ScratchBuffer {
			buffer: buffer.resource,
			allocation,
			address,
			size,
		}
	}

	/// Reports whether the GPU finished the work `completion` waits for.
	fn completion_reached(&self, completion: Completion) -> bool {
		match completion {
			Completion::Frame(index) => self.completed_frame.is_some_and(|completed| completed >= index),
			Completion::Synchronizer(handle) => self.private_synchronizer_signaled(handle),
		}
	}

	/// Settles a recording's acceleration structure work once the recording executes: its bottom-level structures
	/// become built and wait for compaction, its top-level copies record the epoch they were built in, and its
	/// compactions replace the storage of the structures they compact.
	pub(crate) fn submit_recorded_builds(&mut self, mut builds: Box<RecordedBuilds>, completion: Completion) {
		let builds = self.acceleration_structures.reuse_builds(&mut builds);
		let structures = &mut self.acceleration_structures;
		for query in builds.size_queries {
			for handle in &query.structures {
				// A structure destroyed after its build was recorded still builds; its compaction finds the handle stale.
				if let Some(bottom_level) = structures.bottom_levels.get(handle.0) {
					debug_assert_eq!(bottom_level.build_state.get(), BuildState::Recorded);
					bottom_level.build_state.set(BuildState::Built);
				}
			}
			structures
				.pending_compactions
				.push(crate::vulkan::acceleration_structures::PendingCompaction { query, completion });
		}
		for (handle, copy, epoch) in builds.top_levels {
			if let Some(top_level) = structures.top_levels.get(handle.0) {
				top_level.copies[copy as usize].built_epoch.set(Some(epoch));
			}
		}
		for scratch in builds.transient_scratch.buffer.into_iter().chain(builds.retired_scratch) {
			structures.transient_scratch.push((completion, scratch));
		}
		for copy in builds.compactions {
			if !self.acceleration_structures.replace_bottom_level(&copy) {
				// The structure was destroyed since the copy was queued, so its compacted storage serves nobody.
				self.defer_destruction(Tasks::DeleteAccelerationStructure {
					handle: copy.structure,
					storage: copy.storage,
				});
			}
		}
	}

	/// Undoes what a recording that never executed did to acceleration structures: its bottom-level structures become
	/// unbuilt, so a later recording can build them, and its compactions wait for the next recording.
	///
	/// Top-level copies keep what their last executed build wrote, so they need nothing.
	pub(crate) fn abandon_recorded_builds(&mut self, mut builds: Box<RecordedBuilds>) {
		let builds = self.acceleration_structures.reuse_builds(&mut builds);
		for query in builds.size_queries {
			for handle in &query.structures {
				if let Some(bottom_level) = self.acceleration_structures.bottom_levels.get(handle.0) {
					bottom_level.build_state.set(BuildState::Unbuilt);
				}
			}
			// SAFETY: The recording that would have written the pool never executes, so nothing uses it.
			unsafe { self.device.destroy_query_pool(query.pool, None) };
		}
		for (handle, copy, _) in builds.top_levels {
			if let Some(top_level) = self.acceleration_structures.top_levels.get(handle.0) {
				top_level.copies[copy as usize].recorded_frame.set(None);
			}
		}
		self.acceleration_structures.ready_compactions.extend(builds.compactions);
		for scratch in builds.transient_scratch.buffer.into_iter().chain(builds.retired_scratch) {
			// The recording that used the buffer never executes, so nothing waits for it.
			self.run_destruction_task(&Tasks::DeleteVulkanBuffer { handle: scratch.buffer });
			self.run_destruction_task(&Tasks::FreeAllocation {
				handle: scratch.allocation,
			});
		}
	}

	/// Compacts the bottom-level structures whose builds completed, releases the scratch of completed recordings
	/// outside frames, and destroys the storage compaction replaced once no top-level copy names it.
	///
	/// Polls without waiting. A frame calls it as it starts.
	pub(crate) fn advance_acceleration_structures(&mut self) {
		if self.device.ray_tracing.is_none() {
			return;
		}
		let mut pending = std::mem::take(&mut self.acceleration_structures.pending_compactions);
		let completed = pending
			.extract_if(.., |pending| self.completion_reached(pending.completion))
			.collect::<SmallVec<[_; 4]>>();
		self.acceleration_structures.pending_compactions = pending;
		for pending in completed {
			self.queue_compactions(pending.query);
		}

		let mut transient = std::mem::take(&mut self.acceleration_structures.transient_scratch);
		let released = transient
			.extract_if(.., |(completion, _)| self.completion_reached(*completion))
			.collect::<SmallVec<[_; 2]>>();
		self.acceleration_structures.transient_scratch = transient;
		for (_, scratch) in released {
			self.retire_dedicated_buffer(scratch.buffer, scratch.allocation);
		}

		let released = self
			.acceleration_structures
			.take_released_compaction_sources()
			.collect::<SmallVec<[_; 4]>>();
		for source in released {
			self.defer_destruction(Tasks::DeleteAccelerationStructure {
				handle: source.structure,
				storage: source.storage,
			});
		}
	}

	/// Reads the compacted sizes a completed size query holds, and queues a copy into smaller storage for each
	/// structure that would shrink. The next recording records the copies.
	fn queue_compactions(&mut self, query: crate::vulkan::acceleration_structures::SizeQuery) {
		let mut sizes = SmallVec::<[u64; 8]>::from_elem(0, query.structures.len());
		// SAFETY: The recording that wrote the queries completed, so every result is available.
		let result = unsafe {
			self.device
				.get_query_pool_results(query.pool, 0, &mut sizes, vk::QueryResultFlags::TYPE_64)
		};
		// SAFETY: Nothing uses the pool once its results are read.
		unsafe { self.device.destroy_query_pool(query.pool, None) };
		// A query that reports no size leaves its structure at its built size, which still traces.
		if result.is_err() {
			return;
		}

		for (&handle, &size) in query.structures.iter().zip(&sizes) {
			let Some(bottom_level) = self.acceleration_structures.bottom_levels.get(handle.0) else {
				continue;
			};
			// A size of zero would mean the build wrote nothing, and a size that is not smaller saves nothing.
			if size == 0 || size >= bottom_level.storage.size {
				continue;
			}
			let (structure, address, storage) =
				self.create_structure(None, vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL, size);
			self.acceleration_structures.ready_compactions.push(CompactionCopy {
				handle,
				structure,
				address,
				storage,
			});
		}
	}

	/// Destroys every acceleration structure, its storage, and every scratch buffer. Call it only while the context
	/// drops, after the device is idle.
	pub(crate) fn destroy_acceleration_structures(&mut self) {
		let Some(ray_tracing) = self.device.ray_tracing.clone() else {
			return;
		};
		let functions = &ray_tracing.acceleration_structure;
		let structures = &mut self.acceleration_structures;
		// SAFETY: The device is idle, so no command uses any of these objects.
		unsafe {
			for bottom_level in structures.bottom_levels.values() {
				functions.destroy_acceleration_structure(bottom_level.structure, None);
			}
			for copy in structures.top_levels.values().flat_map(|top_level| &top_level.copies) {
				functions.destroy_acceleration_structure(copy.structure, None);
				self.device.device.destroy_buffer(copy.instances.buffer, None);
			}
			for source in structures.take_all_compaction_sources() {
				functions.destroy_acceleration_structure(source.structure, None);
			}
			for copy in structures.ready_compactions.drain(..) {
				functions.destroy_acceleration_structure(copy.structure, None);
			}
			for pending in structures.pending_compactions.drain(..) {
				self.device.device.destroy_query_pool(pending.query.pool, None);
			}
			let scratch = structures
				.scratch
				.iter()
				.filter_map(|arena| arena.buffer)
				.chain(structures.transient_scratch.drain(..).map(|(_, scratch)| scratch));
			for scratch in scratch {
				self.device.device.destroy_buffer(scratch.buffer, None);
			}
			for buffer in structures.storage.buffers() {
				self.device.device.destroy_buffer(buffer, None);
			}
		}
		// The allocations behind every buffer are freed with the context's other allocations.
	}
}
