/// The `GPUVertexDataManager` is responsible for managing the vertex data buffers used in the visibility pipeline.
/// It tracks buffer offsets and counts for various resources, and provides handles to the vertex data buffers.
/// It performs uploads to the GPU of mesh resources.
#[derive(Clone)]
pub(super) struct GPUVertexDataManager {
	/// Tracks buffer offsets and counts for various resources.
	visibility_info: VisibilityInfo,

	/// Vertex positions buffer for rendered meshes.
	pub vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex normals buffer for rendered meshes.
	pub vertex_normals_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex UVs buffer for rendered meshes.
	pub vertex_uvs_buffer: ghi::BufferHandle<[(f32, f32); MAX_VERTICES]>,
	/// Indices laid out as indices into the vertex buffers
	pub vertex_indices_buffer: ghi::BufferHandle<[u16; MAX_PRIMITIVE_TRIANGLES]>,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	pub primitive_indices_buffer: ghi::BufferHandle<[[u8; 3]; MAX_TRIANGLES]>,
	/// Handle to the buffer where each meshlet's data is stored.
	pub meshlets_data_buffer: ghi::BufferHandle<[ShaderMeshletData; MAX_MESHLETS]>,
}

impl GPUVertexDataManager {
	pub fn new(context: &mut ghi::implementation::Context) -> Self {
		let vertex_positions_buffer_handle = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Positions Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_normals_buffer_handle = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Normals Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_uv_buffer_handle = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex UV Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_indices_buffer_handle = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Index Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let primitive_indices_buffer_handle = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Primitive Indices Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let meshlets_data_buffer = context.build_buffer::<[ShaderMeshletData; MAX_MESHLETS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshlets Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		Self {
			visibility_info: VisibilityInfo::default(),
			vertex_positions_buffer: vertex_positions_buffer_handle,
			vertex_normals_buffer: vertex_normals_buffer_handle,
			vertex_uvs_buffer: vertex_uv_buffer_handle,
			vertex_indices_buffer: vertex_indices_buffer_handle,
			primitive_indices_buffer: primitive_indices_buffer_handle,
			meshlets_data_buffer,
		}
	}

	/// Writes GPU mesh data for a mesh resource and returns the mesh object.
	/// Does not check if the resource is already loaded.
	/// Meshes may not be available yet for rendering, this just writes the mesh data to the GPU.
	pub fn write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		resource_request: &mut Reference<Mesh>,
	) -> Option<MeshData> {
		let mesh_resource = resource_request.resource();

		let Some(positions_stream) = mesh_resource.position_stream() else {
			log::error!("Mesh resource does not contain vertex position stream");
			return None;
		};

		let Some(normals_stream) = mesh_resource.normal_stream() else {
			log::error!("Mesh resource does not contain vertex normal stream");
			return None;
		};

		let Some(uvs_stream) = mesh_resource.uv_stream() else {
			log::error!("Mesh resource does not contain vertex uv stream");
			return None;
		};

		let Some(vertex_indices_stream) = mesh_resource.vertex_indices_stream() else {
			log::error!("Mesh resource does not contain vertex index stream");
			return None;
		};

		let Some(triangle_indices_stream) = mesh_resource.triangle_indices_stream() else {
			log::error!("Mesh resource does not contain triangle index stream");
			return None;
		};

		let Some(meshlet_indices_stream) = mesh_resource.meshlet_indices_stream() else {
			log::error!("Mesh resource does not contain meshlet index stream");
			return None;
		};

		let Some(meshlets_stream) = mesh_resource.meshlets_stream() else {
			log::error!("Mesh resource does not contain meshlet stream");
			return None;
		};

		assert_eq!(meshlet_indices_stream.stride, 1, "Meshlet index stream is not u8");
		assert_eq!(vertex_indices_stream.stride, 2, "Vertex index stream is not u16");
		assert_eq!(
			meshlets_stream.stride, RESOURCE_MESHLET_STRIDE,
			"Meshlet stream stride does not match the packed meshlet bounds record"
		);
		assert_eq!(
			meshlet_indices_stream.count() % 3,
			0,
			"Meshlet index stream does not contain complete triangles"
		);

		let vertex_count = positions_stream.count();
		let primitive_count = vertex_indices_stream.count();
		let triangle_count = meshlet_indices_stream.count() / 3;
		let total_meshlet_count = meshlets_stream.count();
		let vertex_offset = self.visibility_info.vertex_count as usize;
		let primitive_offset = self.visibility_info.primitives_count as usize;
		let triangle_offset = self.visibility_info.triangle_count as usize;

		self.ensure_geometry_capacity(vertex_count, primitive_count, triangle_count, total_meshlet_count);

		let mut meshlet_stream_buffer = vec![0u8; meshlets_stream.size];

		let (vertex_positions_staging_offset, vertex_positions_buffer) = slice.take_with_offset(positions_stream.size);
		let (vertex_normals_staging_offset, vertex_normals_buffer) = slice.take_with_offset(normals_stream.size);
		let (vertex_uv_staging_offset, vertex_uv_buffer) = slice.take_with_offset(uvs_stream.size);
		let (vertex_indices_staging_offset, vertex_indices_buffer) = slice.take_with_offset(vertex_indices_stream.size);
		let (primitive_indices_staging_offset, primitive_indices_buffer) = slice.take_with_offset(meshlet_indices_stream.size);

		let mut buffer_allocator = utils::BufferAllocator::new(&mut meshlet_stream_buffer);

		let streams = vec![
			resource_management::stream::StreamMut::new("Vertex.Position", vertex_positions_buffer),
			resource_management::stream::StreamMut::new("Vertex.Normal", vertex_normals_buffer),
			resource_management::stream::StreamMut::new("Vertex.UV", vertex_uv_buffer),
			resource_management::stream::StreamMut::new("VertexIndices", vertex_indices_buffer),
			resource_management::stream::StreamMut::new("MeshletIndices", primitive_indices_buffer),
			resource_management::stream::StreamMut::new("Meshlets", buffer_allocator.take(meshlets_stream.size)),
		];

		let Ok(load_target) = resource_request.load(streams.into()) else {
			log::warn!("Failed to load mesh data");
			return None;
		};

		c.copy_buffers(&[
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_positions_staging_offset,
				self.vertex_positions_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32, f32)>(),
				positions_stream.size,
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_normals_staging_offset,
				self.vertex_normals_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32, f32)>(),
				normals_stream.size,
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_uv_staging_offset,
				self.vertex_uvs_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32)>(),
				uvs_stream.size,
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_indices_staging_offset,
				self.vertex_indices_buffer.into(),
				primitive_offset * std::mem::size_of::<u16>(),
				vertex_indices_stream.size,
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				primitive_indices_staging_offset,
				self.primitive_indices_buffer.into(),
				triangle_offset * std::mem::size_of::<[u8; 3]>(),
				meshlet_indices_stream.size,
			),
		]);

		let Reference {
			resource: Mesh {
				vertex_components,
				streams,
				primitives,
			},
			..
		} = resource_request;

		let vcps = primitives
			.iter()
			.scan(0, |state, p| {
				let offset = *state;
				*state += p.vertex_count;
				offset.into()
			})
			.collect::<Vec<_>>();

		let meshlets_per_primitive = primitives
			.into_iter()
			.zip(vcps.iter())
			.scan(
				(0, 0, 0),
				|(mesh_primitive_counter, mesh_triangle_counter, mesh_meshlet_counter), (primitive, vcps)| {
					let vertex_offset = *vcps;
					let primitive_offset = *mesh_primitive_counter;
					let triangle_offset = *mesh_triangle_counter;
					let meshlet_offset = *mesh_meshlet_counter;

					let meshlets = if let Some(stream) = primitive.meshlet_stream() {
						let m = load_target.stream("Meshlets").unwrap();
						let meshlet_stream = &m.buffer()[stream.offset..stream.offset + stream.size];

						meshlet_stream
							.chunks_exact(RESOURCE_MESHLET_STRIDE)
							.map(read_resource_meshlet)
							.scan(
								(0, 0),
								|(primitive_primitive_counter, primitive_triangle_counter), meshlet| {
									let meshlet_primitive_count = meshlet.primitive_count;
									let meshlet_triangle_count = meshlet.triangle_count;

									let primitive_offset = *primitive_primitive_counter;
									let triangle_offset = *primitive_triangle_counter;

									// Update vertex and triangle offsets per meshlet, relative to the primitive
									*primitive_primitive_counter += meshlet_primitive_count as u32;
									*primitive_triangle_counter += meshlet_triangle_count as u32;

									// Update vertex, triangle and meshlet offsets per meshlet, relative to the mesh
									*mesh_primitive_counter += meshlet_primitive_count as u32;
									*mesh_triangle_counter += meshlet_triangle_count as u32;
									*mesh_meshlet_counter += 1;

									ShaderMeshletData {
										primitive_offset,
										triangle_offset,
										primitive_count: meshlet_primitive_count,
										triangle_count: meshlet_triangle_count,
										center_radius: meshlet.center_radius,
										cone_apex_cutoff: meshlet.cone_apex_cutoff,
										cone_axis: meshlet.cone_axis,
									}
									.into()
								},
							)
							.collect::<Vec<_>>()
					} else {
						panic!();
					};

					(
						MeshPrimitive {
							meshlet_count: meshlets.len() as u32,
							meshlet_offset,
							vertex_offset,
							primitive_offset,
							triangle_offset,
						},
						meshlets,
						primitive,
					)
						.into()
				},
			)
			.collect::<Vec<_>>();

		let meshlets_per_primitive = meshlets_per_primitive
			.into_iter()
			.map(|(mp, meshlets, primitive)| (mp, meshlets))
			.collect::<Vec<_>>();

		let meshlets_data = meshlets_per_primitive
			.iter()
			.flat_map(|(_, meshlets)| meshlets.iter().copied())
			.collect::<Vec<_>>();
		debug_assert_eq!(meshlets_data.len(), total_meshlet_count);

		let meshlets_data_size = std::mem::size_of_val(meshlets_data.as_slice());
		let (meshlets_data_staging_offset, meshlets_data_bytes) = slice.take_with_offset(meshlets_data_size);
		meshlets_data_bytes.copy_from_slice(as_byte_slice(meshlets_data.as_slice()));
		c.copy_buffers(&[ghi::BufferCopyDescriptor::new(
			staging_data_buffer,
			meshlets_data_staging_offset,
			self.meshlets_data_buffer.into(),
			self.visibility_info.meshlet_count as usize * std::mem::size_of::<ShaderMeshletData>(),
			meshlets_data_size,
		)]);

		let primitives = meshlets_per_primitive.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();

		let meshlet_offset = self.visibility_info.meshlet_count;

		let acceleration_structure = if let Some(triangle_indices_stream) = None as Option<resource_management::types::Stream> {
			let index_format = match triangle_indices_stream.stride {
				2 => ghi::DataTypes::U16,
				4 => ghi::DataTypes::U32,
				_ => panic!("Unsupported index format"),
			};

			// let bottom_level_acceleration_structure =
			// 	c.create_bottom_level_acceleration_structure(&ghi::BottomLevelAccelerationStructure {
			// 		description: ghi::BottomLevelAccelerationStructureDescriptions::Mesh {
			// 			vertex_count: positions_stream.count() as u32,
			// 			vertex_position_encoding: ghi::Encodings::FloatingPoint,
			// 			triangle_count: triangle_indices_stream.count() as u32 / 3,
			// 			index_format,
			// 		},
			// 	});

			// ray_tracing.pending_meshes.push(MeshState::Build { mesh_handle: mesh.resource_id.to_string() });

			None
		} else {
			None
		};

		let mesh = MeshData {
			vertex_offset: self.visibility_info.vertex_count,
			primitive_offset: self.visibility_info.primitives_count,
			triangle_offset: self.visibility_info.triangle_count,
			meshlet_offset,
			acceleration_structure,
			primitives,
		};

		self.update_visibility_info_stats(vertex_count, primitive_count, triangle_count, total_meshlet_count);

		Some(mesh)
	}

	/// Writes the mesh data to the GPU and returns the mesh object.
	///
	/// # Returns
	///
	/// A tuple containing the updated buffer allocator and the mesh data.
	pub fn write_gpu_mesh_data_and_return_mesh_object_for_mesh_generator<'slf, 'buffer>(
		&'slf mut self,
		generator: &dyn MeshGenerator,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> Option<MeshData> {
		let positions = generator.positions();
		let normals = generator.normals();
		let uvs = generator.uvs();
		let indices = generator.indices().iter().map(|&index| index as u16).collect::<Vec<_>>();

		if positions.len() != normals.len() || positions.len() != uvs.len() {
			log::error!(
				"Generated mesh attributes are inconsistent. The most likely cause is that the mesh generator returned mismatched vertex attribute counts."
			);
			return None;
		}

		let (vertex_indices, primitive_indices, meshlets) =
			Self::build_generated_meshlets(&indices, positions.as_ref()).ok()?;

		self.ensure_geometry_capacity(positions.len(), vertex_indices.len(), primitive_indices.len(), meshlets.len());

		let vertex_offset = self.visibility_info.vertex_count as usize;
		let primitive_offset = self.visibility_info.primitives_count as usize;
		let triangle_offset = self.visibility_info.triangle_count as usize;
		let meshlet_offset = self.visibility_info.meshlet_count as usize;

		let (vertex_positions_staging_offset, vertex_positions_buffer) =
			slice.take_with_offset(std::mem::size_of_val(positions.as_ref()));
		vertex_positions_buffer.copy_from_slice(as_byte_slice(positions.as_ref()));

		let (vertex_normals_staging_offset, vertex_normals_buffer) =
			slice.take_with_offset(std::mem::size_of_val(normals.as_ref()));
		vertex_normals_buffer.copy_from_slice(as_byte_slice(normals.as_ref()));

		let (vertex_uv_staging_offset, vertex_uv_buffer) = slice.take_with_offset(std::mem::size_of_val(uvs.as_ref()));
		vertex_uv_buffer.copy_from_slice(as_byte_slice(uvs.as_ref()));

		let (vertex_indices_staging_offset, indices_buffer) =
			slice.take_with_offset(std::mem::size_of_val(vertex_indices.as_slice()));
		indices_buffer.copy_from_slice(as_byte_slice(vertex_indices.as_slice()));

		let (primitive_indices_staging_offset, primitive_indices_buffer) =
			slice.take_with_offset(std::mem::size_of_val(primitive_indices.as_slice()));
		primitive_indices_buffer.copy_from_slice(as_byte_slice(primitive_indices.as_slice()));

		let (meshlets_data_staging_offset, meshlets_data_buffer) =
			slice.take_with_offset(std::mem::size_of_val(meshlets.as_slice()));
		meshlets_data_buffer.copy_from_slice(as_byte_slice(meshlets.as_slice()));

		c.copy_buffers(&[
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_positions_staging_offset,
				self.vertex_positions_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32, f32)>(),
				std::mem::size_of_val(positions.as_ref()),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_normals_staging_offset,
				self.vertex_normals_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32, f32)>(),
				std::mem::size_of_val(normals.as_ref()),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_uv_staging_offset,
				self.vertex_uvs_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32)>(),
				std::mem::size_of_val(uvs.as_ref()),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_indices_staging_offset,
				self.vertex_indices_buffer.into(),
				primitive_offset * std::mem::size_of::<u16>(),
				std::mem::size_of_val(vertex_indices.as_slice()),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				primitive_indices_staging_offset,
				self.primitive_indices_buffer.into(),
				triangle_offset * std::mem::size_of::<[u8; 3]>(),
				std::mem::size_of_val(primitive_indices.as_slice()),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				meshlets_data_staging_offset,
				self.meshlets_data_buffer.into(),
				meshlet_offset * std::mem::size_of::<ShaderMeshletData>(),
				std::mem::size_of_val(meshlets.as_slice()),
			),
		]);

		let mesh = MeshData {
			vertex_offset: self.visibility_info.vertex_count,
			primitive_offset: self.visibility_info.primitives_count,
			triangle_offset: self.visibility_info.triangle_count,
			meshlet_offset: self.visibility_info.meshlet_count,
			acceleration_structure: None,
			primitives: vec![MeshPrimitive {
				meshlet_count: meshlets.len() as u32,
				meshlet_offset: 0,
				vertex_offset: 0,
				primitive_offset: 0,
				triangle_offset: 0,
			}],
		};

		self.update_visibility_info_stats(positions.len(), vertex_indices.len(), primitive_indices.len(), meshlets.len());

		Some(mesh)
	}

	fn build_generated_meshlets(
		indices: &[u16],
		positions: &[(f32, f32, f32)],
	) -> Result<(Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>), ()> {
		if indices.len() % 3 != 0 {
			log::error!(
				"Generated mesh indices are invalid. The most likely cause is that the mesh generator returned a triangle list whose index count is not divisible by three."
			);
			return Err(());
		}

		let mut vertex_indices = Vec::new();
		let mut primitive_indices = Vec::new();
		let mut meshlets = Vec::new();

		let mut meshlet_vertex_indices = Vec::<u16>::new();
		let mut meshlet_triangles = Vec::<[u8; 3]>::new();

		for triangle in indices.chunks_exact(3) {
			let unique_vertices = triangle
				.iter()
				.filter(|index| !meshlet_vertex_indices.contains(index))
				.count();

			if !meshlet_triangles.is_empty()
				&& (meshlet_vertex_indices.len() + unique_vertices > VERTEX_COUNT as usize
					|| meshlet_triangles.len() >= TRIANGLE_COUNT as usize)
			{
				Self::push_generated_meshlet(
					&mut vertex_indices,
					&mut primitive_indices,
					&mut meshlets,
					&mut meshlet_vertex_indices,
					&mut meshlet_triangles,
					positions,
				)?;
			}

			let mut local_triangle = [0u8; 3];

			for (slot, index) in triangle.iter().enumerate() {
				let local_index = if let Some(existing) = meshlet_vertex_indices.iter().position(|value| value == index) {
					existing
				} else {
					meshlet_vertex_indices.push(*index);
					meshlet_vertex_indices.len() - 1
				};

				local_triangle[slot] = local_index as u8;
			}

			meshlet_triangles.push(local_triangle);
		}

		Self::push_generated_meshlet(
			&mut vertex_indices,
			&mut primitive_indices,
			&mut meshlets,
			&mut meshlet_vertex_indices,
			&mut meshlet_triangles,
			positions,
		)?;

		Ok((vertex_indices, primitive_indices, meshlets))
	}

	fn push_generated_meshlet(
		vertex_indices: &mut Vec<u16>,
		primitive_indices: &mut Vec<[u8; 3]>,
		meshlets: &mut Vec<ShaderMeshletData>,
		meshlet_vertex_indices: &mut Vec<u16>,
		meshlet_triangles: &mut Vec<[u8; 3]>,
		positions: &[(f32, f32, f32)],
	) -> Result<(), ()> {
		if meshlet_triangles.is_empty() {
			return Ok(());
		}

		let primitive_offset = vertex_indices.len() as u32;
		let triangle_offset = primitive_indices.len() as u32;
		let primitive_count = u32::try_from(meshlet_vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds vertex limits. The most likely cause is that too many unique vertices were packed into a single meshlet."
			);
		})?;
		let triangle_count = u32::try_from(meshlet_triangles.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds triangle limits. The most likely cause is that too many triangles were packed into a single meshlet."
			);
		})?;
		let center_radius = Self::generated_meshlet_center_radius(meshlet_vertex_indices, positions);

		vertex_indices.extend(meshlet_vertex_indices.iter().copied());
		primitive_indices.extend(meshlet_triangles.iter().copied());
		meshlets.push(ShaderMeshletData {
			primitive_offset,
			triangle_offset,
			primitive_count,
			triangle_count,
			center_radius,
			cone_apex_cutoff: [0.0, 0.0, 0.0, 2.0],
			cone_axis: [0.0, 0.0, 1.0, 0.0],
		});

		meshlet_vertex_indices.clear();
		meshlet_triangles.clear();

		Ok(())
	}

	/// Computes a conservative object-space bounding sphere for a generated meshlet.
	fn generated_meshlet_center_radius(meshlet_vertex_indices: &[u16], positions: &[(f32, f32, f32)]) -> [f32; 4] {
		let mut min = [f32::INFINITY; 3];
		let mut max = [f32::NEG_INFINITY; 3];

		for &index in meshlet_vertex_indices {
			let position = positions[index as usize];
			let values = [position.0, position.1, position.2];
			for axis in 0..3 {
				min[axis] = min[axis].min(values[axis]);
				max[axis] = max[axis].max(values[axis]);
			}
		}

		let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
		let mut radius_squared = 0.0f32;

		for &index in meshlet_vertex_indices {
			let position = positions[index as usize];
			let delta = [position.0 - center[0], position.1 - center[1], position.2 - center[2]];
			radius_squared = radius_squared.max(delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]);
		}

		[center[0], center[1], center[2], radius_squared.sqrt()]
	}

	fn update_visibility_info_stats(
		&mut self,
		vertex_count: usize,
		primitive_count: usize,
		triangle_count: usize,
		total_meshlet_count: usize,
	) {
		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;
	}

	fn ensure_geometry_capacity(
		&self,
		additional_vertices: usize,
		additional_primitives: usize,
		additional_triangles: usize,
		additional_meshlets: usize,
	) {
		let total_vertices = self.visibility_info.vertex_count as usize + additional_vertices;
		if total_vertices > MAX_VERTICES {
			panic!(
				"Visibility vertex buffer limit exceeded. The most likely cause is that the scene contains more vertex data than the visibility pipeline supports."
			);
		}

		let total_primitives = self.visibility_info.primitives_count as usize + additional_primitives;
		if total_primitives > MAX_PRIMITIVE_TRIANGLES {
			panic!(
				"Visibility primitive index limit exceeded. The most likely cause is that the scene contains more primitive index data than the visibility pipeline supports."
			);
		}

		let total_triangles = self.visibility_info.triangle_count as usize + additional_triangles;
		if total_triangles > MAX_TRIANGLES {
			panic!(
				"Visibility triangle index limit exceeded. The most likely cause is that the scene contains more triangle index data than the visibility pipeline supports."
			);
		}

		let total_meshlets = self.visibility_info.meshlet_count as usize + additional_meshlets;
		if total_meshlets > MAX_MESHLETS {
			panic!(
				"Visibility meshlet limit exceeded. The most likely cause is that the scene contains more meshlets than the visibility pipeline supports."
			);
		}
	}
}

const RESOURCE_MESHLET_STRIDE: usize = 52;

/// The `ResourceMeshletData` struct carries meshlet metadata decoded from the packed resource stream.
#[derive(Clone, Copy)]
struct ResourceMeshletData {
	primitive_count: u32,
	triangle_count: u32,
	center_radius: [f32; 4],
	cone_apex_cutoff: [f32; 4],
	cone_axis: [f32; 4],
}

/// Decodes one packed meshlet record without assuming the resource stream is naturally aligned.
fn read_resource_meshlet(bytes: &[u8]) -> ResourceMeshletData {
	debug_assert_eq!(bytes.len(), RESOURCE_MESHLET_STRIDE);

	ResourceMeshletData {
		primitive_count: bytes[0] as u32,
		triangle_count: bytes[1] as u32,
		center_radius: read_f32x4(bytes, 4),
		cone_apex_cutoff: read_f32x4(bytes, 20),
		cone_axis: read_f32x4(bytes, 36),
	}
}

fn read_f32x4(bytes: &[u8], offset: usize) -> [f32; 4] {
	[
		read_f32(bytes, offset),
		read_f32(bytes, offset + 4),
		read_f32(bytes, offset + 8),
		read_f32(bytes, offset + 12),
	]
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
	f32::from_le_bytes(
		bytes[offset..offset + 4].try_into().expect(
			"Packed meshlet record is truncated. The most likely cause is that the meshlet stream stride is incorrect.",
		),
	)
}

#[derive(Clone, Copy, Default)]
pub struct VisibilityInfo {
	pub instance_count: u32,
	pub triangle_count: u32,
	pub meshlet_count: u32,
	pub vertex_count: u32,
	pub primitives_count: u32,
}

/// This structure hosts data analogous to the mesh resource's data.
/// Like the scene manager `MeshData` but only contains data relevant to the geometric properties.
#[derive(Debug, Clone)]
pub struct MeshData {
	pub primitives: Vec<MeshPrimitive>,
	/// The base position into the vertex buffer
	pub vertex_offset: u32,
	pub primitive_offset: u32,
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	pub triangle_offset: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the mesh
	pub meshlet_offset: u32,
	pub acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The meshlet count.
	pub meshlet_count: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the primitive in the mesh
	pub meshlet_offset: u32,
	/// The vertex offset.
	/// The base position into the vertex buffer
	pub vertex_offset: u32,
	/// The primitive indices offset.
	/// The base position into the primitive indices buffer
	pub primitive_offset: u32,
	/// The triangle offset.
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	pub triangle_offset: u32,
}

use std::collections::hash_map::Entry;

use ghi::command_buffer::CommandBufferRecording as _;
use resource_management::{resources::mesh::Mesh, Reference};
use utils::as_byte_slice;

use crate::rendering::{
	mesh::generator::MeshGenerator,
	pipelines::visibility::{ShaderMeshletData, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES},
	pipelines::visibility::{TRIANGLE_COUNT, VERTEX_COUNT},
};
