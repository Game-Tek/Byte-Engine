//! Records acceleration structure builds and compactions. The context creates the structures and settles what a
//! recording did to them in `context/acceleration_structures.rs`.

use super::*;
use crate::vulkan::Tasks;
use crate::vulkan::acceleration_structures::{
	BOTTOM_LEVEL_FLAGS, BuildState, CompactionCopy, SizeQuery, TOP_LEVEL_FLAGS, index_bytes, instance_geometry,
	instance_record, triangle_geometry,
};

/// Describes an access of an acceleration structure build or copy, which all run in the build stage.
fn build_access(handle: Handles, access: vk::AccessFlags2) -> VulkanConsumption {
	vulkan_consumption(handle, vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR, access)
}

/// Adds `read` to `reads`, widening the read of the same buffer to the span that covers both. The geometries of a
/// batch share a few buffers, so the batch then waits once per buffer instead of once per geometry. The span also
/// covers bytes between geometries, which only orders the builds more than they need.
fn add_build_read(reads: &mut SmallVec<[VulkanConsumption; 4]>, read: VulkanConsumption) {
	let Some(span) = reads.iter_mut().find(|span| span.handle == read.handle) else {
		reads.push(read);
		return;
	};
	if let (Some(span), Some(range)) = (&mut span.range, read.range) {
		*span = BufferRange::from_bounds(span.offset.min(range.offset), span.end().max(range.end()));
	}
}

impl CommandBufferRecording<'_> {
	/// Returns the acceleration structure work the recording holds, making room for it on the first build in the box
	/// the context kept from the last recording that built.
	fn builds(&mut self) -> &mut crate::vulkan::acceleration_structures::RecordedBuilds {
		self.builds
			.get_or_insert_with(|| self.device.acceleration_structures.spare_builds.take().unwrap_or_default())
	}

	/// Returns the device address of `size` bytes from `buffer` for this recording's frame, with the read a build makes
	/// of them.
	fn build_input(&self, buffer: &crate::BufferDescriptor, size: usize) -> (vk::DeviceAddress, VulkanConsumption) {
		let handle = self.get_internal_buffer_handle(buffer.buffer);
		let address = self.get_buffer(handle).device_address + buffer.offset as vk::DeviceAddress;
		let read = VulkanConsumption {
			range: Some(BufferRange::new(buffer.offset as vk::DeviceSize, size as vk::DeviceSize)),
			..vulkan_consumption(
				Handles::Buffer(handle),
				vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
				vk::AccessFlags2::SHADER_READ,
			)
		};
		(address, read)
	}

	/// Returns the scratch arena of the frame sequence `sequence`, or this recording's own outside frames.
	fn scratch_arena(&mut self, sequence: Option<usize>) -> &mut crate::vulkan::acceleration_structures::ScratchArena {
		match sequence {
			Some(sequence) => &mut self.device.acceleration_structures.scratch[sequence],
			None => &mut self.builds().transient_scratch,
		}
	}

	/// Returns the scratch addresses of builds that need `sizes` bytes each, all in one buffer so they can run at once.
	///
	/// A frame takes them from its sequence's arena, which grows when they don't fit; the buffer it replaces lives on
	/// until the frame completes. A recording outside frames gets a buffer of its own, which the context keeps until
	/// the recording's synchronizer signals.
	fn scratch_addresses(&mut self, sizes: &[u64]) -> SmallVec<[vk::DeviceAddress; 8]> {
		let alignment = self.device.ray_tracing("build an acceleration structure").scratch_alignment;
		// Each range may skip up to an alignment of bytes, so this many bytes always hold all of them.
		let needed = sizes.iter().map(|size| size.max(&1) + alignment).sum::<u64>();
		let sequence = self.frame_key.map(|frame_key| frame_key.sequence_index as usize);
		let arena = self.scratch_arena(sequence);
		if arena.remaining() < needed {
			let size = arena.grown_size(needed, alignment);
			let buffer = self.device.create_scratch_buffer(size);
			// Commands this recording or this frame recorded may still use the replaced buffer.
			if let Some(replaced) = self.scratch_arena(sequence).replace(buffer) {
				match sequence {
					Some(_) => self.device.retire_dedicated_buffer(replaced.buffer, replaced.allocation),
					None => self.builds().retired_scratch.push(replaced),
				}
			}
		}
		let arena = self.scratch_arena(sequence);
		sizes
			.iter()
			.map(|&size| {
				arena.allocate(size, alignment).expect(
					"Vulkan acceleration structure scratch has no room for builds it was sized for. The most likely cause is a fault in the scratch arena.",
				)
			})
			.collect()
	}

	/// Records the builds of `handles` as one command, so they run at the same time, and the query of the size each
	/// structure compacts to.
	pub(super) fn record_bottom_level_builds(&mut self, handles: &[crate::BottomLevelAccelerationStructureHandle]) {
		if handles.is_empty() {
			return;
		}
		let structures = &self.device.acceleration_structures;
		let mut reads = SmallVec::<[VulkanConsumption; 4]>::new();
		let mut geometries = SmallVec::<[vk::AccelerationStructureGeometryKHR<'static>; 16]>::new();
		let mut ranges = SmallVec::<[vk::AccelerationStructureBuildRangeInfoKHR; 16]>::new();
		let mut scratch_sizes = SmallVec::<[u64; 8]>::new();
		for &handle in handles {
			let bottom_level = structures.bottom_level(handle);
			assert!(
				bottom_level.build_state.get() == BuildState::Unbuilt,
				"The bottom-level acceleration structure was already built. The most likely cause is that its build was recorded twice, or that a recording which has not executed yet holds its build. Build each bottom-level structure once, and create a new structure for different geometry.",
			);
			bottom_level.build_state.set(BuildState::Recorded);
			for geometry in &bottom_level.geometries {
				let (vertices, vertex_read) = self.build_input(&geometry.positions.buffer_offset, geometry.positions.size);
				let (indices, index_read) = self.build_input(&geometry.indices, index_bytes(geometry));
				add_build_read(&mut reads, vertex_read);
				add_build_read(&mut reads, index_read);
				geometries.push(triangle_geometry(geometry, vertices, indices));
				ranges.push(vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(geometry.triangle_count));
			}
			scratch_sizes.push(bottom_level.build_scratch_size);
		}
		let scratch = self.scratch_addresses(&scratch_sizes);

		// Each build writes only its own structure, so all of them run after one barrier and without any between them.
		self.vulkan_consume_resources(reads.into_iter().chain([build_access(
			Handles::AccelerationStructure(AccelerationStructureMemory::BOTTOM_LEVELS),
			vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
		)]))
		.apply(self);

		let structures = &self.device.acceleration_structures;
		let mut first_geometry = 0;
		let mut infos = SmallVec::<[vk::AccelerationStructureBuildGeometryInfoKHR<'_>; 8]>::new();
		let mut build_ranges = SmallVec::<[Option<&[vk::AccelerationStructureBuildRangeInfoKHR]>; 8]>::new();
		let mut built = SmallVec::<[vk::AccelerationStructureKHR; 8]>::new();
		for (&handle, &scratch) in handles.iter().zip(&scratch) {
			let bottom_level = structures.bottom_level(handle);
			let geometry_range = first_geometry..first_geometry + bottom_level.geometries.len();
			first_geometry = geometry_range.end;
			infos.push(
				vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.flags(BOTTOM_LEVEL_FLAGS)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.dst_acceleration_structure(bottom_level.structure)
					.geometries(&geometries[geometry_range.clone()])
					.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch }),
			);
			build_ranges.push(Some(&ranges[geometry_range]));
			built.push(bottom_level.structure);
		}
		let command_buffer = self.get_command_buffer().command_buffer;
		let functions = &self
			.device
			.ray_tracing("build an acceleration structure")
			.acceleration_structure;
		// SAFETY: Each structure was sized for its geometry at creation, each scratch range covers the size Vulkan
		// reported for its build, the ranges don't overlap, and the barrier above made the geometry visible.
		unsafe { functions.cmd_build_acceleration_structures(command_buffer, &infos, &build_ranges) };

		// The compacted sizes read what the builds wrote, so they follow all of them after one barrier.
		self.vulkan_consume_resources([build_access(
			Handles::AccelerationStructure(AccelerationStructureMemory::BOTTOM_LEVELS),
			vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR,
		)])
		.apply(self);
		let pool_info = vk::QueryPoolCreateInfo::default()
			.query_type(vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR)
			.query_count(built.len() as u32);
		// SAFETY: The create info is complete, and the context destroys the pool once it read the sizes.
		let pool = unsafe { self.device.create_query_pool(&pool_info, None) }.expect(
			"Vulkan query pool creation failed. The most likely cause is that the device is out of memory. Build fewer acceleration structures at once.",
		);
		let functions = &self
			.device
			.ray_tracing("build an acceleration structure")
			.acceleration_structure;
		// SAFETY: The pool is new, so the host reset brings every query to the state the command needs, and the barrier
		// above ordered the command after the builds.
		unsafe {
			self.device.reset_query_pool(pool, 0, built.len() as u32);
			functions.cmd_write_acceleration_structures_properties(
				command_buffer,
				&built,
				vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR,
				pool,
				0,
			);
		}
		self.builds().size_queries.push(SizeQuery {
			pool,
			structures: handles.iter().copied().collect(),
		});
	}

	/// Records the build of this frame's copy of `structure` from `instances`, writing their records into the copy's
	/// instance memory first.
	pub(super) fn record_top_level_build(
		&mut self,
		structure: crate::TopLevelAccelerationStructureHandle,
		instances: &[crate::rt::Instance],
	) {
		let frame_key = self.frame_key.expect(
			"The top-level acceleration structure build was recorded outside a frame. The most likely cause is that the recording was created from a command buffer instead of a frame. Record top-level builds in a frame's recording, because each frame builds its own copy.",
		);
		let structures = &self.device.acceleration_structures;
		let top_level = structures.top_level(structure);
		assert!(
			instances.len() <= top_level.max_instance_count as usize,
			"The top-level acceleration structure build has more instances than the structure holds. The most likely cause is that the scene grew past the maximum instance count given at creation. instances={}, max_instance_count={}. Create the structure with a larger maximum instance count.",
			instances.len(),
			top_level.max_instance_count,
		);
		let copy_index = frame_key.sequence_index;
		let copy = top_level.copies.get(copy_index as usize).expect(
			"The top-level acceleration structure has no copy for this frame. The most likely cause is that the frames in flight grew after the structure was created. Create top-level structures after you set the frames in flight.",
		);
		assert!(
			copy.recorded_frame.replace(Some(frame_key.frame_index)) != Some(frame_key.frame_index),
			"The top-level acceleration structure was built twice in one frame. The most likely cause is that two recordings of the frame build the same structure. Build each top-level structure once a frame, after every change to its instances.",
		);

		// SAFETY: The copy's memory holds its maximum instance count, which bounds `instances`, it is mapped coherent
		// memory, and the build that read it last belonged to this sequence's previous frame, which completed.
		let records = unsafe {
			std::slice::from_raw_parts_mut(
				copy.instances.pointer.0.cast::<vk::AccelerationStructureInstanceKHR>(),
				instances.len(),
			)
		};
		for (record, instance) in records.iter_mut().zip(instances) {
			assert!(
				instance.custom_index < crate::rt::INSTANCE_INDEX_LIMIT
					&& instance.hit_group_offset < crate::rt::INSTANCE_INDEX_LIMIT,
				"An acceleration structure instance has a custom index or hit group offset of 2^24 or more. The most likely cause is that an index table outgrew 24 bits. custom_index={}, hit_group_offset={}. Keep both values below rt::INSTANCE_INDEX_LIMIT.",
				instance.custom_index,
				instance.hit_group_offset,
			);
			let bottom_level = structures.bottom_level(instance.bottom_level);
			assert!(
				bottom_level.build_state.get() != BuildState::Unbuilt,
				"An acceleration structure instance places a bottom-level structure that was never built. The most likely cause is that its build was not recorded before this top-level build. Record its build with build_bottom_level_acceleration_structures in this recording or an earlier one.",
			);
			*record = instance_record(instance, bottom_level.address);
		}
		let (destination, instance_address, build_scratch_size) =
			(copy.structure, copy.instances.address, top_level.build_scratch_size);
		let epoch = structures.epoch;
		let scratch = self.scratch_addresses(&[build_scratch_size])[0];

		// Reading every bottom-level structure orders the build after the bottom-level builds and compactions before it.
		self.vulkan_consume_resources([
			build_access(
				Handles::AccelerationStructure(AccelerationStructureMemory::BOTTOM_LEVELS),
				vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR,
			),
			build_access(
				Handles::AccelerationStructure(AccelerationStructureMemory::top_level(structure, copy_index)),
				vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
			),
		])
		.apply(self);

		let geometries = [instance_geometry(instance_address)];
		let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.flags(TOP_LEVEL_FLAGS)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.dst_acceleration_structure(destination)
			.geometries(&geometries)
			.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch });
		let range = [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(instances.len() as u32)];
		let command_buffer = self.get_command_buffer().command_buffer;
		let functions = &self
			.device
			.ray_tracing("build an acceleration structure")
			.acceleration_structure;
		// SAFETY: The copy was sized for the structure's maximum instance count, which bounds this build; the scratch
		// range covers the size Vulkan reported for it; and the records name bottom-level structures that stay alive
		// until a later build replaces them.
		unsafe { functions.cmd_build_acceleration_structures(command_buffer, &[info], &[Some(&range)]) };
		self.builds().top_levels.push((structure, copy_index, epoch));
	}

	/// Records the copies that compact bottom-level structures into the smaller storage the context made for them.
	///
	/// Each structure keeps its storage until this recording executes, and the copies return to the context when it is
	/// dropped instead.
	pub(crate) fn record_compactions(&mut self, copies: Vec<CompactionCopy>) {
		if copies.is_empty() {
			return;
		}
		self.vulkan_consume_resources([build_access(
			Handles::AccelerationStructure(AccelerationStructureMemory::BOTTOM_LEVELS),
			vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
		)])
		.apply(self);
		let command_buffer = self.get_command_buffer().command_buffer;
		for copy in copies {
			let Some(source) = self.device.acceleration_structures.bottom_levels.get(copy.handle.0) else {
				// A structure destroyed since its compaction was queued needs none; its compacted storage goes with it.
				self.device.defer_destruction(Tasks::DeleteAccelerationStructure {
					handle: copy.structure,
					storage: copy.storage,
				});
				continue;
			};
			let info = vk::CopyAccelerationStructureInfoKHR::default()
				.src(source.structure)
				.dst(copy.structure)
				.mode(vk::CopyAccelerationStructureModeKHR::COMPACT);
			let functions = &self
				.device
				.ray_tracing("compact an acceleration structure")
				.acceleration_structure;
			// SAFETY: The source's build completed before the context queued the copy, and the destination was sized for
			// the compacted size Vulkan reported for it.
			unsafe { functions.cmd_copy_acceleration_structure(command_buffer, &info) };
			self.builds().compactions.push(copy);
		}
	}
}
