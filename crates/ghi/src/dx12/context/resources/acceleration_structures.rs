use super::super::*;

impl Device {
	pub(crate) fn acceleration_structure_resource_count(&self) -> usize {
		self.acceleration_structure_resource_count
	}

	pub(crate) fn native_acceleration_structure_resource_count(&self) -> usize {
		self.native_acceleration_structure_resource_count
	}

	pub(crate) fn acceleration_structure_instance_write_count(&self) -> usize {
		self.acceleration_structure_instance_write_count
	}

	pub(crate) fn shader_binding_table_write_count(&self) -> usize {
		self.shader_binding_table_write_count
	}

	pub(crate) fn top_level_acceleration_structure_build_record_count(&self) -> usize {
		self.top_level_acceleration_structure_build_record_count
	}

	pub(crate) fn bottom_level_acceleration_structure_build_record_count(&self) -> usize {
		self.bottom_level_acceleration_structure_build_record_count
	}

	pub(crate) fn native_top_level_acceleration_structure_build_encode_count(&self) -> usize {
		self.native_top_level_acceleration_structure_build_encode_count
	}

	pub(crate) fn native_bottom_level_acceleration_structure_build_encode_count(&self) -> usize {
		self.native_bottom_level_acceleration_structure_build_encode_count
	}

	pub(crate) fn acceleration_structure_size(&self, handle: TopLevelAccelerationStructureHandle) -> Option<usize> {
		self.top_level_acceleration_structures
			.get(handle.0 as usize)
			.map(|acceleration_structure| acceleration_structure.size)
	}

	pub(crate) fn acceleration_structure_gpu_address(&self, handle: TopLevelAccelerationStructureHandle) -> Option<u64> {
		self.top_level_acceleration_structures
			.get(handle.0 as usize)
			.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
	}

	pub(crate) fn bottom_level_acceleration_structure_size(
		&self,
		handle: BottomLevelAccelerationStructureHandle,
	) -> Option<usize> {
		self.bottom_level_acceleration_structures
			.get(handle.0 as usize)
			.map(|acceleration_structure| acceleration_structure.size)
	}

	pub(crate) fn bottom_level_acceleration_structure_gpu_address(
		&self,
		handle: BottomLevelAccelerationStructureHandle,
	) -> Option<u64> {
		self.bottom_level_acceleration_structures
			.get(handle.0 as usize)
			.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
	}

	pub fn create_acceleration_structure_instance_buffer(
		&mut self,
		_name: Option<&str>,
		max_instance_count: u32,
	) -> BaseBufferHandle {
		let size = (max_instance_count as usize)
			.checked_mul(std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>())
			.expect(
				"DX12 instance buffer size overflowed. The most likely cause is an instance count that exceeds the host address range.",
			);
		let handle = self.create_buffer_with_layout(
			Layout::from_size_align(size, 16).unwrap(),
			Uses::Storage,
			DeviceAccesses::HostToDevice,
			BufferStorage::Static,
		);
		BaseBufferHandle(handle)
	}

	pub fn create_top_level_acceleration_structure(
		&mut self,
		_name: Option<&str>,
		max_instance_count: u32,
	) -> TopLevelAccelerationStructureHandle {
		let size = self.top_level_acceleration_structure_size(max_instance_count);
		let (resource, native_resource) = self.create_acceleration_structure_resource(size);
		if resource.is_some() {
			self.acceleration_structure_resource_count += 1;
		}
		if native_resource {
			self.native_acceleration_structure_resource_count += 1;
		}
		self.top_level_acceleration_structures.push(AccelerationStructure {
			resource,
			size,
			native_resource,
		});
		TopLevelAccelerationStructureHandle((self.top_level_acceleration_structures.len() - 1) as u64)
	}

	pub fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &BottomLevelAccelerationStructure,
	) -> BottomLevelAccelerationStructureHandle {
		let size = self.bottom_level_acceleration_structure_allocation_size(description);
		let (resource, native_resource) = self.create_acceleration_structure_resource(size);
		if resource.is_some() {
			self.acceleration_structure_resource_count += 1;
		}
		if native_resource {
			self.native_acceleration_structure_resource_count += 1;
		}
		self.bottom_level_acceleration_structures.push(AccelerationStructure {
			resource,
			size,
			native_resource,
		});
		BottomLevelAccelerationStructureHandle((self.bottom_level_acceleration_structures.len() - 1) as u64)
	}

	pub(crate) fn create_acceleration_structure_resource(&mut self, size: usize) -> (Option<ID3D12Resource>, bool) {
		if size == 0 {
			return (None, false);
		}

		let heap_properties = D3D12_HEAP_PROPERTIES {
			Type: D3D12_HEAP_TYPE_DEFAULT,
			CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
			MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
			CreationNodeMask: 1,
			VisibleNodeMask: 1,
		};
		let resource_desc = D3D12_RESOURCE_DESC1 {
			Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
			Alignment: 0,
			Width: size.max(1) as u64,
			Height: 1,
			DepthOrArraySize: 1,
			MipLevels: 1,
			Format: DXGI_FORMAT_UNKNOWN,
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
			// Enhanced barriers identify AS storage with a resource flag while DXR still requires UAV-capable memory.
			Flags: D3D12_RESOURCE_FLAG_RAYTRACING_ACCELERATION_STRUCTURE | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
			SamplerFeedbackMipRegion: Default::default(),
		};

		let mut resource: Option<ID3D12Resource> = None;
		let result = unsafe {
			self.device.CreateCommittedResource3(
				&heap_properties,
				D3D12_HEAP_FLAG_NONE,
				&resource_desc,
				D3D12_BARRIER_LAYOUT_UNDEFINED,
				None,
				None::<&ID3D12ProtectedResourceSession>,
				None,
				&mut resource,
			)
		};
		if result.is_ok() {
			return (resource, true);
		}

		let (resource, ..) = self.create_buffer_resource(size, DeviceAccesses::DeviceOnly);
		(resource, false)
	}

	pub(crate) fn top_level_acceleration_structure_size(&self, max_instance_count: u32) -> usize {
		let fallback = Self::align_up(max_instance_count as usize * 128, 256).max(256);
		self.ray_tracing_prebuild_result_size(D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: max_instance_count,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			// Prebuild checks whether GPUVA fields are null, so use a dummy non-null address before real buffers exist.
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 { InstanceDescs: 1 },
		})
		.unwrap_or(fallback)
	}

	pub(crate) fn bottom_level_acceleration_structure_allocation_size(
		&self,
		description: &BottomLevelAccelerationStructure,
	) -> usize {
		let fallback = Self::bottom_level_acceleration_structure_estimated_size(description);
		let Some(geometry) = Self::bottom_level_geometry_desc_for_prebuild(description) else {
			return fallback;
		};
		self.ray_tracing_prebuild_result_size(D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: 1,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
				pGeometryDescs: &geometry,
			},
		})
		.unwrap_or(fallback)
	}

	pub(crate) fn ray_tracing_prebuild_result_size(
		&self,
		inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
	) -> Option<usize> {
		let info = self.ray_tracing_prebuild_info(&inputs);
		(info.ResultDataMaxSizeInBytes > 0).then(|| Self::align_up(info.ResultDataMaxSizeInBytes as usize, 256).max(256))
	}

	/// Queries the native storage requirements for one acceleration-structure build shape.
	fn ray_tracing_prebuild_info(
		&self,
		inputs: &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
	) -> D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO {
		let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
		unsafe {
			self.device.GetRaytracingAccelerationStructurePrebuildInfo(inputs, &mut info);
		}
		info
	}

	pub(crate) fn bottom_level_geometry_desc_for_prebuild(
		description: &BottomLevelAccelerationStructure,
	) -> Option<D3D12_RAYTRACING_GEOMETRY_DESC> {
		match description.description {
			crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				vertex_position_encoding,
				triangle_count,
				index_format,
			} => {
				let vertex_format = match vertex_position_encoding {
					crate::Encodings::FloatingPoint => DXGI_FORMAT_R32G32B32_FLOAT,
					_ => return None,
				};
				let index_format = match index_format {
					DataTypes::U16 => DXGI_FORMAT_R16_UINT,
					DataTypes::U32 => DXGI_FORMAT_R32_UINT,
					_ => return None,
				};
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
							Transform3x4: 0,
							IndexFormat: index_format,
							VertexFormat: vertex_format,
							IndexCount: triangle_count.saturating_mul(3),
							VertexCount: vertex_count,
							// Prebuild does not read GPU memory but may check whether addresses are null.
							IndexBuffer: 1,
							VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: 1,
								StrideInBytes: std::mem::size_of::<[f32; 3]>() as u64,
							},
						},
					},
				})
			}
			crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
							AABBCount: transform_count as u64,
							AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								// Prebuild does not read GPU memory but may check whether addresses are null.
								StartAddress: 1,
								StrideInBytes: std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB>(
								) as u64,
							},
						},
					},
				})
			}
		}
	}

	pub(crate) fn bottom_level_acceleration_structure_estimated_size(description: &BottomLevelAccelerationStructure) -> usize {
		let size = match description.description {
			crate::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				triangle_count,
				..
			} => vertex_count as usize * 32 + triangle_count as usize * 64,
			crate::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => transform_count as usize * 128,
		};
		Self::align_up(size, 256).max(256)
	}

	pub fn write_instance(
		&mut self,
		instances_buffer_handle: BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: BottomLevelAccelerationStructureHandle,
	) {
		assert!(
			sbt_record_offset <= 0x00ff_ffff,
			"DX12 instance shader table offset exceeds 24 bits. The most likely cause is that the ray tracing pipeline generated too many hit-group records.",
		);
		let Some(buffer_size) = self.buffer(instances_buffer_handle).map(|buffer| buffer.size) else {
			return;
		};
		let descriptor_size = std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>();
		let Some(offset) = instance_index.checked_mul(descriptor_size) else {
			return;
		};
		let Some(end) = offset.checked_add(descriptor_size) else {
			return;
		};
		if end > buffer_size {
			return;
		}
		let Some(bottom_level) = self
			.bottom_level_acceleration_structures
			.get(acceleration_structure.0 as usize)
		else {
			return;
		};
		let address = bottom_level
			.resource
			.as_ref()
			.map(|resource| unsafe { resource.GetGPUVirtualAddress() })
			.unwrap_or(0);
		let instance = D3D12_RAYTRACING_INSTANCE_DESC {
			Transform: [
				transform[0][0],
				transform[0][1],
				transform[0][2],
				transform[0][3],
				transform[1][0],
				transform[1][1],
				transform[1][2],
				transform[1][3],
				transform[2][0],
				transform[2][1],
				transform[2][2],
				transform[2][3],
			],
			_bitfield1: ((mask as u32) << 24) | (custom_index as u32 & 0x00ff_ffff),
			_bitfield2: ((D3D12_RAYTRACING_INSTANCE_FLAG_FORCE_OPAQUE.0 as u32) << 24)
				| (sbt_record_offset as u32 & 0x00ff_ffff),
			AccelerationStructure: address,
		};
		let Some(buffer) = self.buffer_mut(instances_buffer_handle) else {
			return;
		};
		Self::mark_buffer_host_write(buffer);
		// SAFETY: The checked descriptor range lies in the allocated shadow buffer and the source is one initialized descriptor.
		unsafe {
			std::ptr::copy_nonoverlapping(
				(&instance as *const D3D12_RAYTRACING_INSTANCE_DESC).cast::<u8>(),
				buffer.data.add(offset),
				descriptor_size,
			);
		}
		Self::sync_buffer_storage(buffer);
		self.acceleration_structure_instance_write_count += 1;
	}

	pub fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: PipelineHandle,
		shader_handle: ShaderHandle,
	) {
		assert!(
			sbt_record_offset
				.is_multiple_of(windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as usize,),
			"DX12 shader binding table record is misaligned. The most likely cause is that its byte offset is not a multiple of 32.",
		);
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::RayTracing) || !pipeline.shaders.contains(&shader_handle) {
			return;
		}
		let Some(buffer_size) = self.buffer(sbt_buffer_handle).map(|buffer| buffer.size) else {
			return;
		};
		let identifier = if let Some(identifier) = pipeline.ray_tracing_shader_identifiers.get(&shader_handle) {
			*identifier
		} else {
			if pipeline.ray_tracing_state_object.is_some() {
				self.log_dx12_error(format!(
					"Missing DX12 ray tracing shader identifier. The most likely cause is that the shader handle {} was not exported by the ray tracing state object.",
					shader_handle.0
				));
			}
			Self::placeholder_shader_identifier(pipeline_handle, shader_handle)
		};
		let end = sbt_record_offset
			.checked_add(identifier.len())
			.expect("DX12 shader binding table record range overflowed. The most likely cause is an invalid record offset.");
		assert!(
			end <= buffer_size,
			"DX12 shader binding table record exceeds the buffer. The most likely cause is that its offset was built from stale pipeline metadata. record_end={end}, buffer_size={buffer_size}",
		);
		let Some(buffer) = self.buffer_mut(sbt_buffer_handle) else {
			return;
		};
		Self::mark_buffer_host_write(buffer);
		// SAFETY: The checked record range lies in the allocated shadow buffer and `identifier` owns all copied bytes.
		unsafe {
			std::ptr::copy_nonoverlapping(identifier.as_ptr(), buffer.data.add(sbt_record_offset), identifier.len());
		}
		Self::sync_buffer_storage(buffer);
		self.shader_binding_table_write_count += 1;
	}

	pub(crate) fn placeholder_shader_identifier(pipeline_handle: PipelineHandle, shader_handle: ShaderHandle) -> [u8; 32] {
		let mut identifier = [0u8; 32];
		identifier[0..8].copy_from_slice(b"DX12SBT\0");
		identifier[8..16].copy_from_slice(&pipeline_handle.0.to_le_bytes());
		identifier[16..24].copy_from_slice(&shader_handle.0.to_le_bytes());
		identifier
	}

	pub(crate) fn record_top_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::TopLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(acceleration_structure) = self
			.top_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if acceleration_structure.resource.is_none() {
			return;
		}
		let Some(scratch_resource) = self.buffer_resource_for_sequence(build.scratch_buffer.buffer, sequence_index) else {
			return;
		};

		self.transition_tracked_buffer(
			&command_list,
			build.scratch_buffer.buffer,
			&scratch_resource,
			BufferBarrierState::ACCELERATION_STRUCTURE_SCRATCH,
		);
		self.mark_command_buffer_work(command_buffer_handle);
		match build.description {
			crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, .. } => {
				if let Some(instance_resource) = self.buffer_resource_for_sequence(instances_buffer, sequence_index) {
					self.transition_tracked_buffer(
						&command_list,
						instances_buffer,
						&instance_resource,
						BufferBarrierState::ACCELERATION_STRUCTURE_INPUT,
					);
					self.mark_command_buffer_work(command_buffer_handle);
				}
			}
		}
		self.encode_top_level_acceleration_structure_build(command_buffer_handle, &command_list, build, sequence_index);
		self.top_level_acceleration_structure_build_record_count += 1;
	}

	pub(crate) fn record_bottom_level_acceleration_structure_builds(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
		sequence_index: u8,
	) {
		for build in builds {
			if self
				.bottom_level_acceleration_structures
				.get(build.acceleration_structure.0 as usize)
				.and_then(|acceleration_structure| acceleration_structure.resource.as_ref())
				.is_none()
			{
				continue;
			}
			if !self.prepare_bottom_level_build_inputs(command_buffer_handle, build, sequence_index) {
				continue;
			}
			self.encode_bottom_level_acceleration_structure_build(command_buffer_handle, build, sequence_index);
			self.bottom_level_acceleration_structure_build_record_count += 1;
		}
	}

	/// Resolves one aligned scratch-buffer address after validating the complete native scratch range.
	fn acceleration_structure_scratch_address(
		&mut self,
		scratch_buffer: &BufferDescriptor,
		sequence_index: u8,
		required_size: usize,
	) -> Option<u64> {
		let buffer_size = self.buffer(scratch_buffer.buffer)?.size;
		let scratch_end = scratch_buffer.offset.checked_add(required_size).expect(
			"DX12 acceleration structure scratch range overflowed. The most likely cause is an invalid scratch offset or prebuild size.",
		);
		assert!(
			scratch_buffer.offset < buffer_size && scratch_end <= buffer_size,
			"DX12 acceleration structure scratch range exceeds the buffer. The most likely cause is that the build description uses a stale or undersized scratch allocation. offset={}, required_size={required_size}, buffer_size={buffer_size}",
			scratch_buffer.offset,
		);
		let base = self.buffer_address_for_sequence(scratch_buffer.buffer, sequence_index);
		if base == 0 {
			return None;
		}
		let address = base.checked_add(scratch_buffer.offset as u64).expect(
			"DX12 acceleration structure scratch address overflowed. The most likely cause is an invalid native resource address or offset.",
		);
		assert!(
			address.is_multiple_of(
				windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT as u64,
			),
			"DX12 acceleration structure scratch address is misaligned. The most likely cause is that its byte offset is not a multiple of 256.",
		);
		Some(address)
	}

	/// Queries the scratch-buffer bytes required by one fully resolved native build description.
	fn ray_tracing_prebuild_scratch_size(
		&self,
		inputs: &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
	) -> Option<usize> {
		let info = self.ray_tracing_prebuild_info(inputs);
		if info.ScratchDataSizeInBytes == 0 {
			return None;
		}
		Some(usize::try_from(info.ScratchDataSizeInBytes).expect(
			"DX12 acceleration structure scratch size exceeds the host address range. The most likely cause is invalid prebuild information from the device.",
		))
	}

	pub(crate) fn encode_top_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		build: &crate::rt::TopLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list4) = command_list.cast::<ID3D12GraphicsCommandList4>().ok() else {
			return;
		};
		let Some(acceleration_structure) = self
			.top_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if !acceleration_structure.native_resource {
			return;
		}
		let Some(destination_resource) = acceleration_structure.resource.clone() else {
			return;
		};
		let destination = unsafe { destination_resource.GetGPUVirtualAddress() };
		if destination == 0 {
			return;
		}
		assert!(
			destination.is_multiple_of(
				windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT as u64,
			),
			"DX12 top-level acceleration structure address is misaligned. The most likely cause is an invalid native resource allocation.",
		);
		let crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
			instances_buffer,
			instance_count,
		} = build.description;
		let Some(instances_resource) = self.acceleration_structure_build_input_resource(
			command_buffer_handle,
			command_list,
			instances_buffer,
			sequence_index,
		) else {
			return;
		};
		let instance_bytes = (instance_count as usize)
			.checked_mul(std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>())
			.expect(
				"DX12 instance range overflowed. The most likely cause is an instance count that exceeds the host address range.",
			);
		let instance_buffer_size = self.buffer(instances_buffer).map(|buffer| buffer.size).unwrap_or(0);
		assert!(
			instance_bytes <= instance_buffer_size,
			"DX12 instance range exceeds the buffer. The most likely cause is that instance_count exceeds the capacity used to create the instance buffer. required={instance_bytes}, buffer_size={instance_buffer_size}",
		);
		let instances = unsafe { instances_resource.GetGPUVirtualAddress() };
		if instances == 0 {
			return;
		}
		assert!(
			instances
				.is_multiple_of(windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_INSTANCE_DESCS_BYTE_ALIGNMENT as u64,),
			"DX12 instance descriptor address is misaligned. The most likely cause is an invalid native buffer allocation.",
		);
		let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: instance_count,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
				InstanceDescs: instances,
			},
		};
		let Some(required_scratch_size) = self.ray_tracing_prebuild_scratch_size(&inputs) else {
			return;
		};
		let Some(scratch) =
			self.acceleration_structure_scratch_address(&build.scratch_buffer, sequence_index, required_scratch_size)
		else {
			return;
		};
		if scratch == 0 {
			return;
		}
		let desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
			DestAccelerationStructureData: destination,
			Inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
				Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
				Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
				NumDescs: instance_count,
				DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
				Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
					InstanceDescs: instances,
				},
			},
			SourceAccelerationStructureData: 0,
			ScratchAccelerationStructureData: scratch,
		};
		self.transition_acceleration_structure_for_build(command_list, &destination_resource);
		unsafe {
			command_list4.BuildRaytracingAccelerationStructure(&desc, None);
		}
		self.complete_acceleration_structure_build(command_list, &destination_resource);
		self.mark_command_buffer_work(command_buffer_handle);
		self.uav_barrier_count += 1;
		self.native_top_level_acceleration_structure_build_encode_count += 1;
	}

	pub(crate) fn encode_bottom_level_acceleration_structure_build(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(command_list4) = command_list.cast::<ID3D12GraphicsCommandList4>().ok() else {
			return;
		};
		let Some(acceleration_structure) = self
			.bottom_level_acceleration_structures
			.get(build.acceleration_structure.0 as usize)
		else {
			return;
		};
		if !acceleration_structure.native_resource {
			return;
		}
		let Some(destination_resource) = acceleration_structure.resource.clone() else {
			return;
		};
		let destination = unsafe { destination_resource.GetGPUVirtualAddress() };
		let Some(geometry) =
			self.bottom_level_geometry_desc(command_buffer_handle, &command_list, &build.description, sequence_index)
		else {
			return;
		};
		if destination == 0 {
			return;
		}
		assert!(
			destination.is_multiple_of(
				windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BYTE_ALIGNMENT as u64,
			),
			"DX12 bottom-level acceleration structure address is misaligned. The most likely cause is an invalid native resource allocation.",
		);
		let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
			Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
			Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
			NumDescs: 1,
			DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
			Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
				pGeometryDescs: &geometry,
			},
		};
		let Some(required_scratch_size) = self.ray_tracing_prebuild_scratch_size(&inputs) else {
			return;
		};
		let Some(scratch) =
			self.acceleration_structure_scratch_address(&build.scratch_buffer, sequence_index, required_scratch_size)
		else {
			return;
		};
		if scratch == 0 {
			return;
		}
		let desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
			DestAccelerationStructureData: destination,
			Inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
				Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
				Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
				NumDescs: 1,
				DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
				Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
					pGeometryDescs: &geometry,
				},
			},
			SourceAccelerationStructureData: 0,
			ScratchAccelerationStructureData: scratch,
		};
		self.transition_acceleration_structure_for_build(&command_list, &destination_resource);
		unsafe {
			command_list4.BuildRaytracingAccelerationStructure(&desc, None);
		}
		self.complete_acceleration_structure_build(&command_list, &destination_resource);
		self.mark_command_buffer_work(command_buffer_handle);
		self.uav_barrier_count += 1;
		self.native_bottom_level_acceleration_structure_build_encode_count += 1;
	}

	pub(crate) fn bottom_level_geometry_desc(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		description: &crate::rt::BottomLevelAccelerationStructureBuildDescriptions,
		sequence_index: u8,
	) -> Option<D3D12_RAYTRACING_GEOMETRY_DESC> {
		match description {
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
				vertex_buffer,
				vertex_count,
				vertex_position_encoding,
				index_buffer,
				triangle_count,
				index_format,
			} => {
				let vertex_format = match vertex_position_encoding {
					crate::Encodings::FloatingPoint => DXGI_FORMAT_R32G32B32_FLOAT,
					_ => return None,
				};
				let (index_format, index_element_size) = match index_format {
					DataTypes::U16 => (DXGI_FORMAT_R16_UINT, std::mem::size_of::<u16>()),
					DataTypes::U32 => (DXGI_FORMAT_R32_UINT, std::mem::size_of::<u32>()),
					_ => return None,
				};
				let vertex_component_size = std::mem::size_of::<f32>();
				let vertex_data_size = std::mem::size_of::<[f32; 3]>();
				assert!(
					vertex_buffer.stride >= vertex_data_size,
					"DX12 ray tracing vertex stride is too small. The most likely cause is that the mesh stride does not contain one three-component position.",
				);
				assert!(
					vertex_buffer.stride.is_multiple_of(vertex_component_size),
					"DX12 ray tracing vertex stride is misaligned. The most likely cause is that the mesh stride is not a multiple of its 32-bit vertex component size.",
				);
				let native_vertex_stride = u32::try_from(vertex_buffer.stride).expect(
					"DX12 ray tracing vertex stride exceeds 32 bits. The most likely cause is invalid mesh layout metadata.",
				);
				let vertex_bytes = if *vertex_count == 0 {
					0
				} else {
					(*vertex_count as usize - 1)
						.checked_mul(vertex_buffer.stride)
						.and_then(|offset| offset.checked_add(vertex_data_size))
						.expect(
							"DX12 ray tracing vertex range overflowed. The most likely cause is invalid vertex count or stride metadata.",
						)
				};
				assert!(
					vertex_bytes <= vertex_buffer.size,
					"DX12 ray tracing vertices exceed their declared range. The most likely cause is that vertex_count or stride does not match the mesh buffer. required={vertex_bytes}, range_size={}",
					vertex_buffer.size,
				);
				let vertex_range_end = vertex_buffer.buffer_offset.offset.checked_add(vertex_buffer.size).expect(
					"DX12 ray tracing vertex buffer range overflowed. The most likely cause is invalid mesh buffer offset or size metadata.",
				);
				let vertex_buffer_size = self.buffer(vertex_buffer.buffer_offset.buffer)?.size;
				assert!(
					vertex_range_end <= vertex_buffer_size,
					"DX12 ray tracing vertex range exceeds the buffer. The most likely cause is stale mesh buffer metadata. range_end={vertex_range_end}, buffer_size={vertex_buffer_size}",
				);

				let index_count = triangle_count.checked_mul(3).expect(
					"DX12 ray tracing index count overflowed. The most likely cause is a triangle count that exceeds the 32-bit native index-count field.",
				);
				let index_bytes = (index_count as usize).checked_mul(index_element_size).expect(
					"DX12 ray tracing index range overflowed. The most likely cause is invalid triangle count metadata.",
				);
				assert!(
					index_bytes <= index_buffer.size,
					"DX12 ray tracing indices exceed their declared range. The most likely cause is that triangle_count or index_format does not match the mesh buffer. required={index_bytes}, range_size={}",
					index_buffer.size,
				);
				let index_range_end = index_buffer.buffer_offset.offset.checked_add(index_buffer.size).expect(
					"DX12 ray tracing index buffer range overflowed. The most likely cause is invalid mesh buffer offset or size metadata.",
				);
				let index_buffer_size = self.buffer(index_buffer.buffer_offset.buffer)?.size;
				assert!(
					index_range_end <= index_buffer_size,
					"DX12 ray tracing index range exceeds the buffer. The most likely cause is stale mesh buffer metadata. range_end={index_range_end}, buffer_size={index_buffer_size}",
				);
				assert!(
					index_buffer.buffer_offset.offset.is_multiple_of(index_element_size),
					"DX12 ray tracing index offset is misaligned. The most likely cause is that the mesh offset is not a multiple of its index format size.",
				);
				let vertex_resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					vertex_buffer.buffer_offset.buffer,
					sequence_index,
				)?;
				let index_resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					index_buffer.buffer_offset.buffer,
					sequence_index,
				)?;
				let vertex_address = unsafe { vertex_resource.GetGPUVirtualAddress() }
					.checked_add(vertex_buffer.buffer_offset.offset as u64)
					.expect(
						"DX12 ray tracing vertex address overflowed. The most likely cause is an invalid native resource address or offset.",
					);
				let index_address = unsafe { index_resource.GetGPUVirtualAddress() }
					.checked_add(index_buffer.buffer_offset.offset as u64)
					.expect(
						"DX12 ray tracing index address overflowed. The most likely cause is an invalid native resource address or offset.",
					);
				if vertex_address == 0 || index_address == 0 {
					return None;
				}
				assert!(
					vertex_address.is_multiple_of(vertex_component_size as u64),
					"DX12 ray tracing vertex address is misaligned. The most likely cause is that the mesh offset is not a multiple of its 32-bit vertex component size.",
				);
				assert!(
					index_address.is_multiple_of(index_element_size as u64),
					"DX12 ray tracing index address is misaligned. The most likely cause is that the mesh offset is not a multiple of its index format size.",
				);
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
							Transform3x4: 0,
							IndexFormat: index_format,
							VertexFormat: vertex_format,
							IndexCount: index_count,
							VertexCount: *vertex_count,
							IndexBuffer: index_address,
							VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: vertex_address,
								StrideInBytes: native_vertex_stride as u64,
							},
						},
					},
				})
			}
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB {
				aabb_buffer,
				transform_count,
				..
			} => {
				let aabb_stride = std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB>();
				let required_bytes = (*transform_count as usize).checked_mul(aabb_stride).expect(
					"DX12 ray tracing AABB range overflowed. The most likely cause is a transform count that exceeds the host address range.",
				);
				let buffer_size = self.buffer(*aabb_buffer)?.size;
				assert!(
					required_bytes <= buffer_size,
					"DX12 ray tracing AABBs exceed the buffer. The most likely cause is that transform_count exceeds the buffer capacity. required={required_bytes}, buffer_size={buffer_size}",
				);
				let resource = self.acceleration_structure_build_input_resource(
					command_buffer_handle,
					command_list,
					*aabb_buffer,
					sequence_index,
				)?;
				let address = unsafe { resource.GetGPUVirtualAddress() };
				if address == 0 {
					return None;
				}
				assert!(
					address.is_multiple_of(windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB_BYTE_ALIGNMENT as u64,)
						&& aabb_stride.is_multiple_of(
							windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB_BYTE_ALIGNMENT as usize,
						),
					"DX12 ray tracing AABB address or stride is misaligned. The most likely cause is an invalid native buffer allocation or host structure layout.",
				);
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
							AABBCount: *transform_count as u64,
							AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: address,
								StrideInBytes: aabb_stride as u64,
							},
						},
					},
				})
			}
		}
	}

	pub(crate) fn acceleration_structure_build_input_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		buffer_handle: BaseBufferHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let source = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let heap_kind = self.buffer_heap_kind_for_sequence(buffer_handle, sequence_index)?;
		let source_access = if heap_kind == BufferHeapKind::Default {
			BufferBarrierState::ACCELERATION_STRUCTURE_INPUT
		} else {
			BufferBarrierState::COPY_SOURCE
		};
		// Host-visible build inputs are staged, but the source heap must still permit the staging copy.
		self.transition_tracked_buffer(command_list, buffer_handle, &source, source_access);
		if heap_kind == BufferHeapKind::Default {
			return Some(source);
		}

		let size = self.buffer(buffer_handle)?.size;
		let (Some(staged), ..) = self.create_buffer_resource(size, DeviceAccesses::DeviceOnly) else {
			return Some(source);
		};
		self.transition_tracked_buffer(command_list, buffer_handle, &staged, BufferBarrierState::COPY_DESTINATION);
		unsafe {
			command_list.CopyBufferRegion(&staged, 0, &source, 0, size as u64);
		}
		self.transition_tracked_buffer(
			command_list,
			buffer_handle,
			&staged,
			BufferBarrierState::ACCELERATION_STRUCTURE_INPUT,
		);
		self.mark_command_buffer_work(command_buffer_handle);
		self.buffer_copy_count += 1;
		self.retain_command_buffer_upload_resource(command_buffer_handle, staged.clone());
		Some(staged)
	}

	pub(crate) fn prepare_bottom_level_build_inputs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		build: &crate::rt::BottomLevelAccelerationStructureBuild,
		sequence_index: u8,
	) -> bool {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return false;
		};
		let Some(scratch_resource) = self.buffer_resource_for_sequence(build.scratch_buffer.buffer, sequence_index) else {
			return false;
		};
		self.transition_tracked_buffer(
			&command_list,
			build.scratch_buffer.buffer,
			&scratch_resource,
			BufferBarrierState::ACCELERATION_STRUCTURE_SCRATCH,
		);
		self.mark_command_buffer_work(command_buffer_handle);

		let mut transition_input = |buffer_handle: BaseBufferHandle| {
			let Some(resource) = self.buffer_resource_for_sequence(buffer_handle, sequence_index) else {
				return false;
			};
			self.transition_tracked_buffer(
				&command_list,
				buffer_handle,
				&resource,
				BufferBarrierState::ACCELERATION_STRUCTURE_INPUT,
			);
			true
		};

		let inputs_ready = match &build.description {
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
				vertex_buffer,
				index_buffer,
				..
			} => transition_input(vertex_buffer.buffer_offset.buffer) && transition_input(index_buffer.buffer_offset.buffer),
			crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB {
				aabb_buffer,
				transform_buffer,
				..
			} => transition_input(*aabb_buffer) && transition_input(*transform_buffer),
		};
		if inputs_ready {
			self.mark_command_buffer_work(command_buffer_handle);
		}
		inputs_ready
	}
}
