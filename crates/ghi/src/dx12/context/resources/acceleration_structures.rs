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
		let size = max_instance_count as usize * std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>();
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
		let mut info = D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO::default();
		unsafe {
			self.device.GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut info);
		}
		(info.ResultDataMaxSizeInBytes > 0).then(|| Self::align_up(info.ResultDataMaxSizeInBytes as usize, 256).max(256))
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
		let Some(buffer) = self.buffer(instances_buffer_handle) else {
			return;
		};
		let descriptor_size = std::mem::size_of::<D3D12_RAYTRACING_INSTANCE_DESC>();
		let offset = instance_index.saturating_mul(descriptor_size);
		if offset + descriptor_size > buffer.size {
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
		let Some(pipeline) = self.pipelines.get(pipeline_handle.0 as usize) else {
			return;
		};
		if !matches!(pipeline.kind, PipelineKind::RayTracing) || !pipeline.shaders.contains(&shader_handle) {
			return;
		}
		let Some(buffer) = self.buffer(sbt_buffer_handle) else {
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
		let end = sbt_record_offset.saturating_add(identifier.len());
		if end > buffer.size {
			return;
		}
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
		let scratch =
			self.buffer_address_for_sequence(build.scratch_buffer.buffer, sequence_index) + build.scratch_buffer.offset as u64;
		if destination == 0 || scratch == 0 {
			return;
		}
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
		let instances = unsafe { instances_resource.GetGPUVirtualAddress() };
		if instances == 0 {
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
		let scratch =
			self.buffer_address_for_sequence(build.scratch_buffer.buffer, sequence_index) + build.scratch_buffer.offset as u64;
		let Some(geometry) =
			self.bottom_level_geometry_desc(command_buffer_handle, &command_list, &build.description, sequence_index)
		else {
			return;
		};
		if destination == 0 || scratch == 0 {
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
				let index_format = match index_format {
					DataTypes::U16 => DXGI_FORMAT_R16_UINT,
					DataTypes::U32 => DXGI_FORMAT_R32_UINT,
					_ => return None,
				};
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
				let vertex_address =
					unsafe { vertex_resource.GetGPUVirtualAddress() } + vertex_buffer.buffer_offset.offset as u64;
				let index_address = unsafe { index_resource.GetGPUVirtualAddress() } + index_buffer.buffer_offset.offset as u64;
				if vertex_address == 0 || index_address == 0 {
					return None;
				}
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
							Transform3x4: 0,
							IndexFormat: index_format,
							VertexFormat: vertex_format,
							IndexCount: triangle_count.saturating_mul(3),
							VertexCount: *vertex_count,
							IndexBuffer: index_address,
							VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: vertex_address,
								StrideInBytes: vertex_buffer.stride as u64,
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
				Some(D3D12_RAYTRACING_GEOMETRY_DESC {
					Type: D3D12_RAYTRACING_GEOMETRY_TYPE_PROCEDURAL_PRIMITIVE_AABBS,
					Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
					Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
						AABBs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
							AABBCount: *transform_count as u64,
							AABBs: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
								StartAddress: address,
								StrideInBytes: std::mem::size_of::<windows::Win32::Graphics::Direct3D12::D3D12_RAYTRACING_AABB>(
								) as u64,
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
		self.sync_buffer_for_sequence(buffer_handle, sequence_index);
		let source = self.buffer_resource_for_sequence(buffer_handle, sequence_index)?;
		let heap_kind = self.buffer_heap_kind_for_sequence(buffer_handle, sequence_index)?;
		if heap_kind == BufferHeapKind::Default {
			self.transition_tracked_buffer(
				command_list,
				buffer_handle,
				&source,
				BufferBarrierState::ACCELERATION_STRUCTURE_INPUT,
			);
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
