//! Creates, destroys, builds, and compacts a DX12 context's acceleration structures.
//!
//! The structures live in `dx12/acceleration_structures.rs`. A destroyed structure lives on until the frames recorded
//! before its destruction complete, and every recording that builds or reads one keeps it alive until the recording
//! completes, so a destruction never pulls a structure from under the GPU.
//!
//! Compaction follows the Vulkan backend's: each bottom-level build writes the size its structure compacts to, the
//! context reads the sizes once the build completes and makes smaller storage for each structure that shrinks, the
//! next recording copies the structures into it, and they take it when that recording executes.

use windows::Win32::Graphics::Direct3D12 as d3d12;

use super::*;
use crate::dx12::acceleration_structures::{
	BottomLevel, BuildState, CompactionCopy, Completion, INSTANCE_SIZE, PendingCompaction, STRUCTURE_ALIGNMENT,
	ScratchArena, SizeQuery, TopLevel, TopLevelCopy, build_inputs, index_bytes, index_size, instance_record,
	triangle_geometry,
};
use crate::rt;

/// The syncs that read a bottom-level structure after its build: top-level builds, and the copies and size queries of
/// compaction.
const BOTTOM_LEVEL_READERS: d3d12::D3D12_BARRIER_SYNC = d3d12::D3D12_BARRIER_SYNC(
	d3d12::D3D12_BARRIER_SYNC_BUILD_RAYTRACING_ACCELERATION_STRUCTURE.0
		| d3d12::D3D12_BARRIER_SYNC_COPY_RAYTRACING_ACCELERATION_STRUCTURE.0
		| d3d12::D3D12_BARRIER_SYNC_EMIT_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO.0,
);

/// The syncs that read a top-level structure after its build: every shader that runs a ray query or a trace.
const TOP_LEVEL_READERS: d3d12::D3D12_BARRIER_SYNC =
	d3d12::D3D12_BARRIER_SYNC(d3d12::D3D12_BARRIER_SYNC_ALL_SHADING.0 | d3d12::D3D12_BARRIER_SYNC_RAYTRACING.0);

impl Device {
	/// Panics unless the context traces rays. `operation` names what needed ray tracing, such as "create a shader
	/// binding table".
	pub(crate) fn require_ray_tracing(&self, operation: &str) {
		assert!(
			self.ray_tracing_supported,
			"DX12 could not {operation}. The most likely cause is that the device was created without the ray tracing feature, or that its GPU doesn't support DXR tier 1.1. Check Context::supports_ray_tracing, and use a technique without rays when it is false."
		);
	}

	/// Reports whether the device was created with the ray tracing feature and its GPU supports DXR tier 1.1, which
	/// inline ray queries need.
	pub(crate) fn device_supports_ray_tracing(device: &ID3D12Device10, settings: Features) -> bool {
		settings.ray_tracing
			&& Self::feature_support(
				device,
				d3d12::D3D12_FEATURE_D3D12_OPTIONS5,
				d3d12::D3D12_FEATURE_DATA_D3D12_OPTIONS5::default(),
			)
			.is_some_and(|data| data.RaytracingTier.0 >= d3d12::D3D12_RAYTRACING_TIER_1_1.0)
	}

	/// Creates a committed buffer of `size` bytes in `heap` memory with `flags`, named `name` for graphics debuggers.
	///
	/// A `D3D12_HEAP_TYPE_CUSTOM` heap is CPU-cached system memory, which the CPU reads and shaders and builds may
	/// write.
	pub(crate) fn create_ray_tracing_buffer(
		&self,
		name: Option<&str>,
		size: u64,
		heap: d3d12::D3D12_HEAP_TYPE,
		flags: D3D12_RESOURCE_FLAGS,
	) -> ID3D12Resource {
		let heap_properties = if heap == d3d12::D3D12_HEAP_TYPE_CUSTOM {
			D3D12_HEAP_PROPERTIES {
				Type: heap,
				CPUPageProperty: d3d12::D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
				MemoryPoolPreference: d3d12::D3D12_MEMORY_POOL_L0,
				CreationNodeMask: 1,
				VisibleNodeMask: 1,
			}
		} else {
			D3D12_HEAP_PROPERTIES {
				Type: heap,
				CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
				MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
				CreationNodeMask: 1,
				VisibleNodeMask: 1,
			}
		};
		let desc = D3D12_RESOURCE_DESC1 {
			Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
			Alignment: 0,
			Width: size.max(1),
			Height: 1,
			DepthOrArraySize: 1,
			MipLevels: 1,
			Format: DXGI_FORMAT_UNKNOWN,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
			Flags: flags,
			SamplerFeedbackMipRegion: Default::default(),
		};
		let mut resource: Option<ID3D12Resource> = None;
		// SAFETY: The descriptions are complete, and enhanced-barrier buffers start UNDEFINED whatever their heap.
		let result = unsafe {
			self.device.CreateCommittedResource3(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&desc,
				D3D12_BARRIER_LAYOUT_UNDEFINED,
				None,
				None::<&ID3D12ProtectedResourceSession>,
				None,
				&mut resource,
			)
		};
		let resource = result.ok().and(resource).unwrap_or_else(|| {
			self.drain_debug_messages();
			panic!(
				"DX12 ray tracing buffer creation failed. The most likely cause is that the device is out of memory. Build fewer or smaller acceleration structures. size={size}, name={name:?}"
			)
		});
		self.name_object(&resource, name);
		resource
	}

	/// Creates `size` bytes of acceleration structure storage and returns it with its address.
	fn create_structure_storage(&self, name: Option<&str>, size: u64) -> (ID3D12Resource, u64) {
		let resource = self.create_ray_tracing_buffer(
			name,
			size.next_multiple_of(STRUCTURE_ALIGNMENT),
			D3D12_HEAP_TYPE_DEFAULT,
			D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS | d3d12::D3D12_RESOURCE_FLAG_RAYTRACING_ACCELERATION_STRUCTURE,
		);
		// SAFETY: The resource is a live buffer.
		let address = unsafe { resource.GetGPUVirtualAddress() };
		(resource, address)
	}

	/// Returns the storage and scratch sizes the build `inputs` describes needs.
	fn prebuild_info(
		&self,
		inputs: &d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
	) -> d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO {
		let mut info = d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
		// SAFETY: Sizing reads only the counts, formats, and flags of the inputs, which describe the builds the
		// structure gets.
		unsafe { self.device.GetRaytracingAccelerationStructurePrebuildInfo(inputs, &mut info) };
		info
	}

	/// Checks that `size` bytes from `buffer` lie in a buffer that geometry may be built from, at a multiple of
	/// `alignment`.
	fn check_geometry_range(&self, buffer: &BufferDescriptor, size: usize, alignment: usize) {
		let resource = self.buffer(buffer.buffer).expect(
			"DX12 triangle geometry names a missing buffer. The most likely cause is that the buffer handle came from another context. Build geometry from buffers this context created.",
		);
		assert!(
			resource.uses.contains(Uses::AccelerationStructureBuild),
			"DX12 triangle geometry reads a buffer without the acceleration structure build use. The most likely cause is that the buffer was created for drawing only. Add Uses::AccelerationStructureBuild to the buffer."
		);
		assert!(
			buffer.offset.checked_add(size).is_some_and(|end| end <= resource.size),
			"DX12 triangle geometry passes the end of its buffer. The most likely cause is that the triangle count, stride, or offset describes more data than the buffer holds. range_end={}, buffer_size={}. Fix the geometry's counts.",
			buffer.offset.saturating_add(size),
			resource.size,
		);
		assert!(
			buffer.offset.is_multiple_of(alignment),
			"DX12 triangle geometry starts at a misaligned offset. The most likely cause is that the offset was not rounded to the element size. offset={}, alignment={alignment}. Align the offset to {alignment} bytes.",
			buffer.offset,
		);
	}

	/// Creates a bottom-level structure over `geometries`, sized for their build, after validating each geometry.
	pub(crate) fn create_bottom_level(
		&mut self,
		name: Option<&str>,
		geometries: &[rt::TriangleGeometry],
	) -> BottomLevelAccelerationStructureHandle {
		self.require_ray_tracing("create a bottom-level acceleration structure");
		assert!(
			!geometries.is_empty(),
			"DX12 bottom-level acceleration structure creation failed. The most likely cause is that no triangle geometry was given. Pass at least one geometry."
		);
		for geometry in geometries {
			assert!(
				matches!(geometry.index_format, DataTypes::U16 | DataTypes::U32),
				"DX12 triangle geometry has an unsupported index format. The most likely cause is that the index format is not a 16-bit or 32-bit integer. Use DataTypes::U16 or DataTypes::U32."
			);
			let stride = geometry.positions.stride;
			assert!(
				stride >= 12 && stride.is_multiple_of(4),
				"DX12 triangle geometry has an unsupported position stride. The most likely cause is that the stride is smaller than three floats or not a multiple of 4 bytes. stride={stride}. Use a stride of at least 12 bytes that is a multiple of 4."
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
		let info = self.prebuild_info(&build_inputs(&descriptions, 0, 0));
		let (resource, address) = self.create_structure_storage(name, info.ResultDataMaxSizeInBytes);
		let handle = self.acceleration_structures.bottom_levels.insert(BottomLevel {
			resource,
			address,
			size: info.ResultDataMaxSizeInBytes,
			geometries: geometries.into(),
			build_scratch_size: info.ScratchDataSizeInBytes,
			build_state: std::cell::Cell::new(BuildState::Unbuilt),
		});
		BottomLevelAccelerationStructureHandle::from_bits(handle)
	}

	/// Creates a top-level structure with one copy for each frame in flight, each sized for `max_instance_count`
	/// instances and with instance records of its own.
	pub(crate) fn create_top_level(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		self.require_ray_tracing("create a top-level acceleration structure");
		assert!(
			max_instance_count > 0,
			"DX12 top-level acceleration structure creation failed. The most likely cause is that the maximum instance count is zero. Pass the largest number of instances a frame places."
		);
		// Sizing ignores the address of the instance records; builds name their own.
		let info = self.prebuild_info(&build_inputs(&[], 0, max_instance_count));
		let mut copies = SmallVec::new();
		for _ in 0..self.frames {
			let (resource, address) = self.create_structure_storage(name, info.ResultDataMaxSizeInBytes);
			let instances = self.create_ray_tracing_buffer(
				name,
				u64::from(max_instance_count) * INSTANCE_SIZE,
				D3D12_HEAP_TYPE_UPLOAD,
				D3D12_RESOURCE_FLAG_NONE,
			);
			let mut mapped = std::ptr::null_mut();
			// SAFETY: Upload buffers map for writing, and the empty read range says the CPU reads nothing.
			unsafe { instances.Map(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }), Some(&mut mapped)) }.expect(
				"DX12 instance buffer mapping failed. The most likely cause is that the device was removed. Check the device removed reason.",
			);
			copies.push(TopLevelCopy {
				resource,
				address,
				instances,
				instance_records: mapped.cast(),
				recorded_frame: std::cell::Cell::new(None),
				built_epoch: std::cell::Cell::new(None),
			});
		}
		let handle = self.acceleration_structures.top_levels.insert(TopLevel {
			copies,
			max_instance_count,
			build_scratch_size: info.ScratchDataSizeInBytes,
		});
		TopLevelAccelerationStructureHandle::from_bits(handle)
	}

	/// Releases `resource` once every frame sequence starts again, when the frames recorded before this call have
	/// completed.
	fn retire_after_recorded_frames(&mut self, resource: ID3D12Resource) {
		for sequence_index in 0..self.frames {
			self.defer_task(sequence_index, DeferredTask::RetireResource(resource.clone()));
		}
	}

	/// Destroys the bottom-level structure `handle` names once the frames recorded before this call complete.
	pub(crate) fn destroy_bottom_level(&mut self, handle: BottomLevelAccelerationStructureHandle) {
		let bottom_level = self.acceleration_structures.bottom_levels.remove(handle.bits()).unwrap_or_else(|| {
			panic!(
				"The bottom-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		self.retire_after_recorded_frames(bottom_level.resource);
	}

	/// Destroys the top-level structure `handle` names, and every copy, once the frames recorded before this call
	/// complete.
	pub(crate) fn destroy_top_level(&mut self, handle: TopLevelAccelerationStructureHandle) {
		let top_level = self.acceleration_structures.top_levels.remove(handle.bits()).unwrap_or_else(|| {
			panic!(
				"The top-level acceleration structure handle is stale. The most likely cause is that the structure was destroyed twice. Destroy each structure once."
			)
		});
		for copy in top_level.copies {
			self.retire_after_recorded_frames(copy.resource);
			self.retire_after_recorded_frames(copy.instances);
		}
	}

	/// Makes the scratch of `sequence_index` available again. Call it as the sequence's frame starts, once its previous
	/// frame completed.
	pub(crate) fn reset_acceleration_structure_scratch(&mut self, sequence_index: u8) {
		self.acceleration_structures.scratch[sequence_index as usize].reset();
	}

	/// Returns the scratch addresses of builds that need `sizes` bytes each, all in one buffer so they can run at once.
	///
	/// A frame takes them from its sequence's arena, which grows when they don't fit. A recording outside frames gets
	/// a buffer of its own. Either way the recording keeps the buffer alive until it completes, so a buffer the arena
	/// replaces lives on for the recordings that used it.
	fn scratch_addresses(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		frame_sequence: Option<u8>,
		sizes: &[u64],
	) -> SmallVec<[u64; 8]> {
		// Each range may skip up to an alignment of bytes, so this many bytes always hold all of them.
		let needed = sizes.iter().map(|size| size.max(&1) + STRUCTURE_ALIGNMENT).sum::<u64>();
		let mut own_arena = ScratchArena::default();
		let grown_size = match frame_sequence {
			Some(sequence) => {
				let arena = &self.acceleration_structures.scratch[sequence as usize];
				(arena.remaining() < needed).then(|| arena.grown_size(needed))
			}
			None => Some(needed),
		};
		if let Some(size) = grown_size {
			let resource = self.create_ray_tracing_buffer(
				Some("Acceleration Structure Scratch"),
				size,
				D3D12_HEAP_TYPE_DEFAULT,
				D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
			);
			// SAFETY: The resource is a live buffer.
			let address = unsafe { resource.GetGPUVirtualAddress() };
			let arena = match frame_sequence {
				Some(sequence) => &mut self.acceleration_structures.scratch[sequence as usize],
				None => &mut own_arena,
			};
			arena.replace(Some(resource), address, size);
		}
		let arena = match frame_sequence {
			Some(sequence) => &mut self.acceleration_structures.scratch[sequence as usize],
			None => &mut own_arena,
		};
		let addresses = sizes
			.iter()
			.map(|&size| {
				arena.allocate(size, STRUCTURE_ALIGNMENT).expect(
					"DX12 acceleration structure scratch has no room for builds it was sized for. The most likely cause is a fault in the scratch arena.",
				)
			})
			.collect();
		if let Some(resource) = arena.resource.clone() {
			self.retain_command_buffer_resource(command_buffer_handle, &resource);
		}
		addresses
	}

	/// Returns the address of `buffer` in sequence `sequence_index`, after moving the buffer to the state builds read
	/// geometry in.
	fn build_input_address(
		&mut self,
		command_list: &ID3D12GraphicsCommandList7,
		buffer: &BufferDescriptor,
		sequence_index: u8,
	) -> u64 {
		let resource = self.buffer_resource_for_sequence(buffer.buffer, sequence_index).expect(
			"DX12 triangle geometry reads a buffer without a native resource. The most likely cause is that the buffer is empty or its allocation failed. Build geometry from filled buffers.",
		);
		self.transition_tracked_buffer(
			command_list,
			buffer.buffer,
			&resource,
			BufferBarrierState::ACCELERATION_STRUCTURE_INPUT,
		);
		// SAFETY: The resource is a live buffer.
		unsafe { resource.GetGPUVirtualAddress() + buffer.offset as u64 }
	}

	/// Records the builds of `handles`, which run at the same time, then the barrier that makes them readable.
	///
	/// `frame_sequence` is the sequence of the recording's frame, or `None` outside frames, and `sequence_index` the
	/// sequence whose buffer copies the geometry is read from.
	pub(crate) fn record_bottom_level_builds(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		frame_sequence: Option<u8>,
		sequence_index: u8,
		handles: &[BottomLevelAccelerationStructureHandle],
	) {
		if handles.is_empty() {
			return;
		}
		self.require_ray_tracing("build an acceleration structure");
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let mut geometries = SmallVec::<[rt::TriangleGeometry; 16]>::new();
		let mut scratch_sizes = SmallVec::<[u64; 8]>::new();
		for &handle in handles {
			let bottom_level = self.acceleration_structures.bottom_level(handle);
			assert!(
				bottom_level.build_state.get() == BuildState::Unbuilt,
				"The bottom-level acceleration structure was already built. The most likely cause is that its build was recorded twice, or that a recording which has not executed yet holds its build. Build each bottom-level structure once."
			);
			bottom_level.build_state.set(BuildState::Recorded);
			geometries.extend(bottom_level.geometries.iter().copied());
			scratch_sizes.push(bottom_level.build_scratch_size);
		}
		let descriptions = geometries
			.iter()
			.map(|geometry| {
				let vertices = self.build_input_address(&command_list, &geometry.positions.buffer_offset, sequence_index);
				let indices = self.build_input_address(&command_list, &geometry.indices, sequence_index);
				triangle_geometry(geometry, vertices, indices)
			})
			.collect::<SmallVec<[_; 16]>>();
		let scratch = self.scratch_addresses(command_buffer_handle, frame_sequence, &scratch_sizes);
		// Each build writes the size its structure compacts to, which the context reads once the build completes.
		let query = self.create_ray_tracing_buffer(
			Some("Acceleration Structure Sizes"),
			(handles.len() * size_of::<u64>()) as u64,
			d3d12::D3D12_HEAP_TYPE_CUSTOM,
			D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
		);
		let mut sizes = std::ptr::null_mut();
		// SAFETY: Custom write-back heaps map, and the CPU reads the whole buffer once the builds complete.
		unsafe { query.Map(0, None, Some(&mut sizes)) }.expect(
			"DX12 acceleration structure size query mapping failed. The most likely cause is that the device was removed. Check the device removed reason.",
		);
		// SAFETY: The resource is a live buffer.
		let query_address = unsafe { query.GetGPUVirtualAddress() };
		self.retain_command_buffer_resource(command_buffer_handle, &query);

		let mut first_geometry = 0;
		for (index, (&handle, &scratch)) in handles.iter().zip(&scratch).enumerate() {
			let bottom_level = self.acceleration_structures.bottom_level(handle);
			let geometry_range = first_geometry..first_geometry + bottom_level.geometries.len();
			first_geometry = geometry_range.end;
			let desc = d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
				DestAccelerationStructureData: bottom_level.address,
				Inputs: build_inputs(&descriptions[geometry_range], 0, 0),
				SourceAccelerationStructureData: 0,
				ScratchAccelerationStructureData: scratch,
			};
			let resource = bottom_level.resource.clone();
			let size_query = d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO_DESC {
				DestBuffer: query_address + (index * size_of::<u64>()) as u64,
				InfoType: d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_POSTBUILD_INFO_COMPACTED_SIZE,
			};
			// SAFETY: The structure was sized for its geometry at creation, the scratch range covers the size DX12
			// reported for its build, the ranges don't overlap, the geometry is in the build input state, and the size
			// lands in a range of the query of its own.
			unsafe { command_list.BuildRaytracingAccelerationStructure(&desc, Some(&[size_query])) };
			self.retain_command_buffer_resource(command_buffer_handle, &resource);
		}
		// Each build writes only its own structure, so all of them run without a barrier between them, and one barrier
		// makes all of them readable.
		Self::global_barrier(
			&command_list,
			d3d12::D3D12_BARRIER_SYNC_BUILD_RAYTRACING_ACCELERATION_STRUCTURE,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_WRITE,
			BOTTOM_LEVEL_READERS,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_READ,
		);
		let builds = &mut self.command_buffers[command_buffer_handle.index() as usize].recorded_builds;
		builds.bottom_levels.extend(handles.iter().map(|handle| handle.bits()));
		builds.size_queries.push(SizeQuery {
			resource: query,
			sizes: sizes.cast::<u64>().cast_const(),
			structures: handles.iter().map(|handle| handle.bits()).collect(),
		});
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Records the build of `frame_key`'s copy of `structure` from `instances`, writing their records into the copy's
	/// instance memory first, then the barrier that makes the copy readable to shaders.
	pub(crate) fn record_top_level_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		frame_key: crate::FrameKey,
		structure: TopLevelAccelerationStructureHandle,
		instances: &[rt::Instance],
	) {
		self.require_ray_tracing("build an acceleration structure");
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let structures = &self.acceleration_structures;
		let top_level = structures.top_level(structure);
		assert!(
			instances.len() <= top_level.max_instance_count as usize,
			"The top-level acceleration structure build has more instances than the structure holds. The most likely cause is that the scene grew past the maximum instance count given at creation. instances={}, max_instance_count={}. Create the structure with a larger maximum.",
			instances.len(),
			top_level.max_instance_count,
		);
		let copy_index = frame_key.sequence_index;
		let copy = top_level.copies.get(copy_index as usize).expect(
			"The top-level acceleration structure has no copy for this frame. The most likely cause is that the frames in flight grew after the structure was created. Create top-level structures after you set the frames in flight.",
		);
		assert!(
			copy.recorded_frame.replace(Some(frame_key.frame_index)) != Some(frame_key.frame_index),
			"The top-level acceleration structure was built twice in one frame. The most likely cause is that two recordings of the frame build the same structure. Build each top-level structure once a frame."
		);
		// SAFETY: The copy's records hold its maximum instance count, which bounds `instances`, they are mapped for
		// the copy's life, and the build that read them last belonged to this sequence's previous frame, which
		// completed.
		let records = unsafe { std::slice::from_raw_parts_mut(copy.instance_records, instances.len()) };
		for (record, instance) in records.iter_mut().zip(instances) {
			assert!(
				instance.custom_index < rt::INSTANCE_INDEX_LIMIT && instance.hit_group_offset < rt::INSTANCE_INDEX_LIMIT,
				"An acceleration structure instance has a custom index or hit group offset of 2^24 or more. The most likely cause is that an index table outgrew 24 bits. custom_index={}, hit_group_offset={}. Keep both below 2^24.",
				instance.custom_index,
				instance.hit_group_offset,
			);
			let bottom_level = structures.bottom_level(instance.bottom_level);
			assert!(
				bottom_level.build_state.get() != BuildState::Unbuilt,
				"An acceleration structure instance places a bottom-level structure that was never built. The most likely cause is that its build was not recorded before this top-level build. Record its build with build_bottom_level_acceleration_structures first."
			);
			*record = instance_record(instance, bottom_level.address);
		}
		// SAFETY: The instance buffer is a live buffer.
		let instance_address = unsafe { copy.instances.GetGPUVirtualAddress() };
		let (destination, resources, build_scratch_size, epoch) = (
			copy.address,
			[copy.resource.clone(), copy.instances.clone()],
			top_level.build_scratch_size,
			structures.epoch,
		);
		let scratch = self.scratch_addresses(command_buffer_handle, Some(frame_key.sequence_index), &[build_scratch_size])[0];
		let desc = d3d12::D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
			DestAccelerationStructureData: destination,
			Inputs: build_inputs(&[], instance_address, instances.len() as u32),
			SourceAccelerationStructureData: 0,
			ScratchAccelerationStructureData: scratch,
		};
		// SAFETY: The copy was sized for the structure's maximum instance count, which bounds this build; the scratch
		// range covers the size DX12 reported for it; the records name built bottom-level structures, whose builds the
		// barrier after them ordered before this one; and the copy's last readers belong to frames that completed.
		unsafe { command_list.BuildRaytracingAccelerationStructure(&desc, None) };
		for resource in &resources {
			self.retain_command_buffer_resource(command_buffer_handle, resource);
		}
		Self::global_barrier(
			&command_list,
			d3d12::D3D12_BARRIER_SYNC_BUILD_RAYTRACING_ACCELERATION_STRUCTURE,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_WRITE,
			TOP_LEVEL_READERS,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_READ,
		);
		self.command_buffers[command_buffer_handle.index() as usize]
			.recorded_builds
			.top_levels
			.push((structure.bits(), copy_index, epoch));
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Records one global barrier, which acceleration structures need because they never change layout.
	fn global_barrier(
		command_list: &ID3D12GraphicsCommandList7,
		sync_before: d3d12::D3D12_BARRIER_SYNC,
		access_before: d3d12::D3D12_BARRIER_ACCESS,
		sync_after: d3d12::D3D12_BARRIER_SYNC,
		access_after: d3d12::D3D12_BARRIER_ACCESS,
	) {
		let mut barriers = EnhancedBarrierBatch::default();
		barriers.global.push(D3D12_GLOBAL_BARRIER {
			SyncBefore: sync_before,
			SyncAfter: sync_after,
			AccessBefore: access_before,
			AccessAfter: access_after,
		});
		barriers.submit(command_list);
	}

	/// Records the copies that compact bottom-level structures into the smaller storage the context made for them, at
	/// the start of a recording.
	///
	/// Each structure keeps its storage until this recording executes, and the copies return to the context if it is
	/// dropped instead.
	pub(crate) fn record_ready_compactions(&mut self, command_buffer_handle: CommandBufferHandle) {
		if self.acceleration_structures.ready_compactions.is_empty() {
			return;
		}
		let Some(command_list) = self.command_list(command_buffer_handle).cloned() else {
			return;
		};
		let copies = std::mem::take(&mut self.acceleration_structures.ready_compactions);
		for copy in copies {
			let Some(source) = self.acceleration_structures.bottom_levels.get(copy.handle) else {
				// A structure destroyed since its compaction was queued needs none; its compacted storage goes with it.
				continue;
			};
			let source = (source.address, source.resource.clone());
			// SAFETY: The source's build completed before the context queued the copy, and the destination was sized for
			// the compacted size DX12 reported for it.
			unsafe {
				command_list.CopyRaytracingAccelerationStructure(
					copy.address,
					source.0,
					d3d12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_COPY_MODE_COMPACT,
				)
			};
			self.retain_command_buffer_resource(command_buffer_handle, &source.1);
			self.retain_command_buffer_resource(command_buffer_handle, &copy.resource);
			self.command_buffers[command_buffer_handle.index() as usize]
				.recorded_builds
				.compactions
				.push(copy);
		}
		// Top-level builds later in the recording, and size queries of compacted structures, read the copies.
		Self::global_barrier(
			&command_list,
			d3d12::D3D12_BARRIER_SYNC_COPY_RAYTRACING_ACCELERATION_STRUCTURE,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_WRITE,
			BOTTOM_LEVEL_READERS,
			d3d12::D3D12_BARRIER_ACCESS_RAYTRACING_ACCELERATION_STRUCTURE_READ,
		);
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Settles what a recording did to acceleration structures once it was submitted, to complete at `completion`:
	/// its size queries wait for the builds, its top-level copies record the epoch they were built at, and its
	/// compactions replace the storage of the structures they compact.
	pub(crate) fn submit_recorded_builds(&mut self, command_buffer_handle: CommandBufferHandle, completion: Completion) {
		let builds = std::mem::take(&mut self.command_buffers[command_buffer_handle.index() as usize].recorded_builds);
		let structures = &mut self.acceleration_structures;
		structures
			.pending_compactions
			.extend(builds.size_queries.into_iter().map(|query| PendingCompaction { query, completion }));
		for (bits, copy, epoch) in builds.top_levels {
			if let Some(top_level) = structures.top_levels.get(bits) {
				top_level.copies[copy as usize].built_epoch.set(Some(epoch));
			}
		}
		// A structure destroyed since the copy was recorded drops its compacted storage, which the recording keeps
		// alive until it completes.
		for copy in builds.compactions {
			let _ = structures.replace_bottom_level(copy);
		}
	}

	/// Undoes what a recording that never executes did to acceleration structures: its bottom-level structures become
	/// unbuilt, so a later recording can build them, its top-level copies can be built again in the frame, and its
	/// compactions wait for the next recording.
	///
	/// Top-level copies keep what their last executed build wrote, so they need nothing else.
	pub(crate) fn abandon_recorded_builds(&mut self, command_buffer_handle: CommandBufferHandle) {
		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.index() as usize) else {
			return;
		};
		let builds = std::mem::take(&mut command_buffer.recorded_builds);
		let structures = &mut self.acceleration_structures;
		for bits in builds.bottom_levels {
			// A structure destroyed since its build was recorded needs nothing.
			if let Some(bottom_level) = structures.bottom_levels.get(bits) {
				bottom_level.build_state.set(BuildState::Unbuilt);
			}
		}
		for (bits, copy, _) in builds.top_levels {
			if let Some(top_level) = structures.top_levels.get(bits) {
				top_level.copies[copy as usize].recorded_frame.set(None);
			}
		}
		structures.ready_compactions.extend(builds.compactions);
	}

	/// Makes compacted storage for the structures whose builds completed, and releases the storage compaction replaced
	/// once no top-level copy names it.
	///
	/// Polls without waiting. A frame calls it as it starts.
	pub(crate) fn advance_acceleration_structures(&mut self) {
		let mut pending = std::mem::take(&mut self.acceleration_structures.pending_compactions);
		let completed = pending
			.extract_if(.., |pending| self.completion_reached(pending.completion))
			.collect::<SmallVec<[_; 4]>>();
		self.acceleration_structures.pending_compactions = pending;
		for pending in completed {
			self.queue_compactions(pending.query);
		}
		let released = self
			.acceleration_structures
			.take_released_compaction_sources()
			.collect::<SmallVec<[_; 4]>>();
		for source in released {
			self.retire_after_recorded_frames(source.resource);
		}
	}

	/// Reports whether the GPU finished the submission that signals `completion`.
	fn completion_reached(&self, (synchronizer, value): Completion) -> bool {
		self.synchronizers
			.get(synchronizer.index() as usize)
			// SAFETY: The synchronizer registry owns the live fence queried here.
			.is_some_and(|synchronizer| unsafe { synchronizer.fence.GetCompletedValue() } >= value)
	}

	/// Reads the compacted sizes a completed size query holds, and makes smaller storage for each structure that would
	/// shrink. The next recording copies the structures into it.
	fn queue_compactions(&mut self, query: SizeQuery) {
		for (index, &bits) in query.structures.iter().enumerate() {
			// SAFETY: The builds that wrote the sizes completed, and the mapping holds one size per structure.
			let size = unsafe { query.sizes.add(index).read() };
			let Some(bottom_level) = self.acceleration_structures.bottom_levels.get(bits) else {
				continue;
			};
			// A size of zero would mean the build wrote nothing, and a size that is not smaller saves nothing.
			if size == 0 || size >= bottom_level.size {
				continue;
			}
			let (resource, address) = self.create_structure_storage(None, size);
			self.acceleration_structures.ready_compactions.push(CompactionCopy {
				handle: bits,
				resource,
				address,
				size,
			});
		}
	}

	/// Returns the bytes the bottom-level structure `handle` occupies.
	pub(crate) fn bottom_level_size(&self, handle: BottomLevelAccelerationStructureHandle) -> usize {
		self.acceleration_structures.bottom_level(handle).size as usize
	}

	/// Writes the view of the copy of `handle` that sequence `sequence_index` builds into resource heap slot `slot`.
	pub(crate) fn write_native_acceleration_structure_descriptor(
		&self,
		handle: TopLevelAccelerationStructureHandle,
		sequence_index: u8,
		slot: u32,
	) {
		let top_level = self.acceleration_structures.top_level(handle);
		let copy = &top_level.copies[sequence_index as usize % top_level.copies.len()];
		let desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
			Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
				RaytracingAccelerationStructure: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_SRV { Location: copy.address },
			},
			..Self::null_acceleration_structure_srv_desc()
		};
		let cpu_handle = self.shader_visible_cpu_handle(false, slot);
		// SAFETY: The view names the copy by address, which lives as long as the structure, and the handle is a slot of
		// the shader-visible heap.
		unsafe { self.device.CreateShaderResourceView(None::<&ID3D12Resource>, Some(&desc), cpu_handle) };
	}

	/// Returns the storage of the copy of `handle` that sequence `sequence_index` builds, which recordings that read
	/// it keep alive.
	pub(crate) fn top_level_copy_resource(
		&self,
		handle: TopLevelAccelerationStructureHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let top_level = self.acceleration_structures.top_levels.get(handle.bits())?;
		Some(top_level.copies[sequence_index as usize % top_level.copies.len()].resource.clone())
	}
}
