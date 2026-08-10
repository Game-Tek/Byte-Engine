/// The `GPUVertexDataManager` is responsible for managing the vertex data buffers used in the visibility pipeline.
/// It tracks buffer offsets and counts for various resources, and provides handles to the vertex data buffers.
/// It performs uploads to the GPU of mesh resources.
#[derive(Clone)]
pub(super) struct GPUVertexDataManager {
	/// Tracks buffer offsets and counts for various resources.
	visibility_info: VisibilityInfo,
	/// Tracks the compact immutable vertex ranges consumed by GPU skinning.
	skinning_source_vertex_count: u32,

	/// Vertex positions buffer for rendered meshes.
	pub vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex normals buffer for rendered meshes, octahedrally encoded as two UNORM16 components.
	pub vertex_normals_buffer: ghi::BufferHandle<[RuntimeVertexNormal; MAX_VERTICES]>,
	/// Vertex UVs buffer for rendered meshes, packed in the visibility runtime format.
	pub vertex_uvs_buffer: ghi::BufferHandle<[RuntimeVertexUv; MAX_VERTICES]>,
	/// Indices laid out as indices into the vertex buffers
	pub vertex_indices_buffer: ghi::BufferHandle<[u16; MAX_PRIMITIVE_TRIANGLES]>,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	pub primitive_indices_buffer: ghi::BufferHandle<[[u8; 3]; MAX_TRIANGLES]>,
	/// Buffer that stores the meshlet records.
	pub meshlets_data_buffer: ghi::BufferHandle<[ShaderMeshletData; MAX_MESHLETS]>,
	/// Bind-pose positions packed only for primitives that participate in GPU skinning.
	pub(super) skinning_rest_positions_buffer: ghi::BufferHandle<[[f32; 3]; MAX_VERTICES]>,
	/// Bind-pose normals packed only for primitives that participate in GPU skinning.
	pub(super) skinning_rest_normals_buffer: ghi::BufferHandle<[[f32; 3]; MAX_VERTICES]>,
	/// Four palette-local u16 joint indices packed into eight bytes per skinned vertex.
	pub(super) skinning_joints_buffer: ghi::BufferHandle<[[u16; 4]; MAX_VERTICES]>,
	/// Four normalized linear-blend weights packed beside each skinned vertex's joints.
	pub(super) skinning_weights_buffer: ghi::BufferHandle<[[f32; 4]; MAX_VERTICES]>,
}

/// The `PreparedGpuMesh` struct retains validated mesh ranges in their leased GPU upload-buffer region.
///
/// Pass it to [`GPUVertexDataManager::write_prepared_gpu_mesh_data_and_return_mesh_object`] when its lease is ready.
pub(super) struct PreparedGpuMesh {
	staging: super::upload_staging::StagingLease,
	streams: PreparedGpuMeshStreams,
	primitives: Vec<PreparedGpuMeshPrimitive>,
	vertex_count: usize,
	primitive_count: usize,
	triangle_count: usize,
	meshlet_count: usize,
	skinning_vertex_count: usize,
}

/// The `PreparedGpuMeshStreams` struct locates transfer-ready streams in one owned byte backing.
struct PreparedGpuMeshStreams {
	positions: std::ops::Range<usize>,
	normals: std::ops::Range<usize>,
	uvs: std::ops::Range<usize>,
	vertex_indices: std::ops::Range<usize>,
	primitive_indices: std::ops::Range<usize>,
	meshlets: std::ops::Range<usize>,
	skinning_normals: Option<std::ops::Range<usize>>,
	skinning_joints: Option<std::ops::Range<usize>>,
	skinning_weights: Option<std::ops::Range<usize>>,
}

/// The `PreparedGpuMeshPrimitive` struct retains one primitive's relative GPU ranges and optional skinning copies.
struct PreparedGpuMeshPrimitive {
	mesh: MeshPrimitive,
	skinning: Option<PreparedGpuSkinningCopy>,
}

/// The `PreparedGpuSkinningCopy` struct locates one primitive in the prepared aggregate skinning streams.
struct PreparedGpuSkinningCopy {
	positions: std::ops::Range<usize>,
	normals: std::ops::Range<usize>,
	joints: std::ops::Range<usize>,
	weights: std::ops::Range<usize>,
}

/// The `PreparedGpuMeshCounts` struct defines the aggregate geometry contract that primitive metadata must satisfy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreparedGpuMeshCounts {
	vertices: usize,
	primitive_indices: usize,
	triangles: usize,
	meshlets: usize,
	skinning_vertices: usize,
}

impl PreparedGpuMesh {
	/// Loads and validates a resource mesh into owned memory without borrowing transfer recording state.
	///
	/// The returned mesh keeps its staging lease until the transfer frame completes.
	pub(super) async fn prepare_resource_mesh(
		mut resource: Reference<Mesh>,
		upload_staging: std::sync::Arc<super::upload_staging::UploadStagingArena>,
	) -> Option<Self> {
		let mesh = resource.resource();

		let Some(positions_stream) = mesh.position_stream() else {
			log::error!(
				"Mesh resource does not contain a vertex position stream. The most likely cause is that the mesh was baked without required visibility geometry."
			);
			return None;
		};
		let Some(normals_stream) = mesh.normal_stream() else {
			log::error!(
				"Mesh resource does not contain a vertex normal stream. The most likely cause is that the mesh was baked without required visibility geometry."
			);
			return None;
		};
		let Some(uvs_stream) = mesh.uv_stream() else {
			log::error!(
				"Mesh resource does not contain a vertex UV stream. The most likely cause is that the mesh was baked without required visibility geometry."
			);
			return None;
		};
		let Some(vertex_indices_stream) = mesh.vertex_indices_stream() else {
			log::error!(
				"Mesh resource does not contain a vertex index stream. The most likely cause is that the mesh was baked without meshlet vertex indices."
			);
			return None;
		};
		let Some(_triangle_indices_stream) = mesh.triangle_indices_stream() else {
			log::error!(
				"Mesh resource does not contain a triangle index stream. The most likely cause is that the mesh was baked without triangle geometry."
			);
			return None;
		};
		let Some(meshlet_indices_stream) = mesh.meshlet_indices_stream() else {
			log::error!(
				"Mesh resource does not contain a meshlet index stream. The most likely cause is that the mesh was baked without meshlet triangle indices."
			);
			return None;
		};
		let Some(meshlets_stream) = mesh.meshlets_stream() else {
			log::error!(
				"Mesh resource does not contain a meshlet stream. The most likely cause is that the mesh was baked without meshlet metadata."
			);
			return None;
		};

		if mesh
			.primitives
			.iter()
			.filter_map(|primitive| primitive.skin)
			.any(|skin_index| skin_index as usize >= mesh.skins.len())
		{
			log::error!(
				"Skinned primitive references a missing skin binding. The most likely cause is corrupted primitive metadata."
			);
			return None;
		}

		let mut primitive_validations = Vec::new();
		if primitive_validations.try_reserve_exact(mesh.primitives.len()).is_err() {
			log::error!(
				"Mesh primitive validation metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large primitive list."
			);
			return None;
		}
		for primitive in &mesh.primitives {
			primitive_validations.push(LoadedPrimitiveValidation {
				vertex_count: primitive.vertex_count,
				vertex_indices: primitive
					.stream(Streams::Indices(resource_management::types::IndexStreamTypes::Vertices))
					.cloned(),
				triangle_indices: primitive
					.stream(Streams::Indices(resource_management::types::IndexStreamTypes::Meshlets))
					.cloned(),
				meshlets: primitive.meshlet_stream().cloned(),
				joints: primitive.stream(Streams::Vertices(VertexSemantics::Joints)).cloned(),
				weights: primitive.stream(Streams::Vertices(VertexSemantics::Weights)).cloned(),
				palette_len: primitive.skin.map(|skin_index| mesh.skins[skin_index as usize].len()),
			});
		}
		let has_skinned_primitives = mesh.primitives.iter().any(|primitive| primitive.skin.is_some());
		let joints_stream = if has_skinned_primitives {
			let Some(stream) = mesh.vertex_stream(VertexSemantics::Joints).cloned() else {
				log::error!(
					"Skinned mesh is missing the joint-index stream. The most likely cause is that the mesh was baked without complete skinning vertex attributes."
				);
				return None;
			};
			Some(stream)
		} else {
			None
		};
		let weights_stream = if has_skinned_primitives {
			let Some(stream) = mesh.vertex_stream(VertexSemantics::Weights).cloned() else {
				log::error!(
					"Skinned mesh is missing the vertex-weight stream. The most likely cause is that the mesh was baked without complete skinning vertex attributes."
				);
				return None;
			};
			Some(stream)
		} else {
			None
		};

		let mut skinning_vertex_count = 0usize;
		for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
			if primitive.skin.is_none() {
				continue;
			}
			let Some(joints_stream) = joints_stream.as_ref() else {
				log::error!(
					"Skinned mesh is missing its aggregate joint-index stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(weights_stream) = weights_stream.as_ref() else {
				log::error!(
					"Skinned mesh is missing its aggregate vertex-weight stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};

			if validate_skinning_primitive_stream(
				primitive,
				primitive_index,
				&positions_stream,
				VertexSemantics::Position,
				SKINNING_POSITION_STRIDE,
			)
			.is_err() || validate_skinning_primitive_stream(
				primitive,
				primitive_index,
				&normals_stream,
				VertexSemantics::Normal,
				SKINNING_NORMAL_STRIDE,
			)
			.is_err() || validate_skinning_primitive_stream(
				primitive,
				primitive_index,
				joints_stream,
				VertexSemantics::Joints,
				SKINNING_JOINTS_STRIDE,
			)
			.is_err() || validate_skinning_primitive_stream(
				primitive,
				primitive_index,
				weights_stream,
				VertexSemantics::Weights,
				SKINNING_WEIGHTS_STRIDE,
			)
			.is_err()
			{
				return None;
			}

			let Some(updated_count) = skinning_vertex_count.checked_add(primitive.vertex_count as usize) else {
				log::error!(
					"Skinned mesh vertex count is too large. The most likely cause is corrupted primitive metadata containing an overflowing vertex count."
				);
				return None;
			};
			skinning_vertex_count = updated_count;
		}

		let uv_source_format = match mesh
			.vertex_components
			.iter()
			.find(|component| component.semantic == VertexSemantics::UV && component.channel == 0)
			.map(|component| component.format.as_str())
		{
			Some("vec2f16") => UvSourceFormat::F16,
			Some("vec2f") => UvSourceFormat::F32,
			format => {
				log::error!(
					"Unsupported mesh UV format {format:?}. The most likely cause is that the asset uses a vertex format other than vec2f16 or vec2f."
				);
				return None;
			}
		};
		let vertex_count = validated_stream_count(&positions_stream, "position", SKINNING_POSITION_STRIDE)?;
		let normal_count = validated_stream_count(&normals_stream, "normal", NORMAL_F32_SOURCE_STRIDE)?;
		if normal_count != vertex_count {
			log::error!(
				"Mesh normals are not vec3f or do not match the position count. The most likely cause is malformed or unsupported vertex stream metadata."
			);
			return None;
		}
		let uv_stride = match uv_source_format {
			UvSourceFormat::F16 => UV_F16_SOURCE_STRIDE,
			UvSourceFormat::F32 => UV_F32_SOURCE_STRIDE,
		};
		let uv_count = validated_stream_count(&uvs_stream, "UV", uv_stride)?;
		if uv_count != vertex_count {
			log::error!(
				"Mesh UV count does not match its position count. The most likely cause is malformed vertex stream metadata."
			);
			return None;
		}

		let primitive_count = validated_stream_count(&vertex_indices_stream, "meshlet vertex-index", 2)?;
		let meshlet_index_count = validated_stream_count(&meshlet_indices_stream, "meshlet triangle-index", 1)?;
		if !meshlet_index_count.is_multiple_of(3) {
			log::error!(
				"Meshlet triangle-index stream does not contain complete triangles. The most likely cause is truncated baked meshlet index data."
			);
			return None;
		}
		let triangle_count = meshlet_index_count / 3;
		let meshlet_count = validated_stream_count(&meshlets_stream, "meshlet", RESOURCE_MESHLET_STRIDE)?;
		let runtime_normal_size = checked_mesh_byte_size(vertex_count, VERTEX_NORMAL_BUFFER_STRIDE as usize, "normal")?;
		let runtime_uv_size = checked_mesh_byte_size(vertex_count, VERTEX_UV_BUFFER_STRIDE as usize, "UV")?;
		let runtime_meshlet_size = checked_mesh_byte_size(meshlet_count, std::mem::size_of::<ShaderMeshletData>(), "meshlet")?;
		let source_sizes = [
			positions_stream.size,
			normals_stream.size,
			uvs_stream.size,
			vertex_indices_stream.size,
			meshlet_indices_stream.size,
			meshlets_stream.size,
			joints_stream.as_ref().map_or(0, |stream| stream.size),
			weights_stream.as_ref().map_or(0, |stream| stream.size),
		];
		let Some(_) = source_sizes
			.into_iter()
			.try_fold(0usize, |total, size| total.checked_add(size))
		else {
			log::error!(
				"Mesh stream byte count overflowed. The most likely cause is corrupted stream metadata with invalid sizes."
			);
			return None;
		};
		let mut cursor = 0usize;
		let positions = take_range_aligned(&mut cursor, positions_stream.size, 4);
		let source_normals = take_range_aligned(&mut cursor, normals_stream.size, 4);
		let source_uvs = take_range_aligned(&mut cursor, uvs_stream.size, 4);
		let vertex_indices = take_range_aligned(&mut cursor, vertex_indices_stream.size, 4);
		let primitive_indices = take_range_aligned(&mut cursor, meshlet_indices_stream.size, 4);
		let source_meshlets = take_range(&mut cursor, meshlets_stream.size);
		let skinning_joints = joints_stream
			.as_ref()
			.map(|stream| take_range_aligned(&mut cursor, stream.size, 4));
		let skinning_weights = weights_stream
			.as_ref()
			.map(|stream| take_range_aligned(&mut cursor, stream.size, 4));
		let source_byte_count = cursor;
		let normals = take_range_aligned(&mut cursor, runtime_normal_size, 4);
		let uvs = match uv_source_format {
			UvSourceFormat::F16 => source_uvs.clone(),
			UvSourceFormat::F32 => take_range_aligned(&mut cursor, runtime_uv_size, 4),
		};
		let meshlets = take_range_aligned(&mut cursor, runtime_meshlet_size, 4);
		let backing_size = cursor;

		let mut staging = upload_staging.allocate(backing_size, 256).await.or_else(|| {
			log::error!(
				"Prepared mesh exceeds the GPU upload arena. The most likely cause is that the resource is larger than the configured upload capacity."
			);
			None
		})?;
		let backing = staging.bytes_mut();
		let (prepared_primitives, runtime_meshlets) = {
			let mut source_allocator = utils::BufferAllocator::new(&mut backing[..source_byte_count]);
			let mut streams = Vec::with_capacity(if has_skinned_primitives { 8 } else { 6 });
			streams.push(resource_management::stream::StreamMut::new(
				"Vertex.Position",
				source_allocator.take_with_offset_aligned(positions_stream.size, 4).1,
			));
			streams.push(resource_management::stream::StreamMut::new(
				"Vertex.Normal",
				source_allocator.take_with_offset_aligned(normals_stream.size, 4).1,
			));
			streams.push(resource_management::stream::StreamMut::new(
				"Vertex.UV",
				source_allocator.take_with_offset_aligned(uvs_stream.size, 4).1,
			));
			streams.push(resource_management::stream::StreamMut::new(
				"VertexIndices",
				source_allocator.take_with_offset_aligned(vertex_indices_stream.size, 4).1,
			));
			streams.push(resource_management::stream::StreamMut::new(
				"MeshletIndices",
				source_allocator.take_with_offset_aligned(meshlet_indices_stream.size, 4).1,
			));
			streams.push(resource_management::stream::StreamMut::new(
				"Meshlets",
				source_allocator.take(meshlets_stream.size),
			));
			if let (Some(joints), Some(weights)) = (&joints_stream, &weights_stream) {
				streams.push(resource_management::stream::StreamMut::new(
					"Vertex.Joints",
					source_allocator.take_with_offset_aligned(joints.size, 4).1,
				));
				streams.push(resource_management::stream::StreamMut::new(
					"Vertex.Weights",
					source_allocator.take_with_offset_aligned(weights.size, 4).1,
				));
			}

			let loaded = resource
				.load(streams.into())
				.await
				.map_err(|_| {
					log::error!(
					"Mesh resource streams could not be loaded. The most likely cause is that the baked mesh payload is missing or unreadable."
				);
				})
				.ok()?;
			if validate_loaded_mesh_indices(&primitive_validations, &loaded).is_err()
				|| validate_loaded_skin_joints(&primitive_validations, &loaded).is_err()
			{
				return None;
			}

			let Some(loaded_meshlets) = loaded.stream("Meshlets") else {
				log::error!(
					"Loaded mesh data is missing its meshlet stream. The most likely cause is that the resource loader returned an incomplete read target."
				);
				return None;
			};
			build_prepared_resource_primitives(
				resource.resource(),
				loaded_meshlets.buffer(),
				PreparedGpuMeshCounts {
					vertices: vertex_count,
					primitive_indices: primitive_count,
					triangles: triangle_count,
					meshlets: meshlet_count,
					skinning_vertices: skinning_vertex_count,
				},
			)?
		};

		let (source, output) = backing.split_at_mut(source_byte_count);
		pack_f32_normals(
			&source[source_normals.clone()],
			&mut output[normals.start - source_byte_count..normals.end - source_byte_count],
			vertex_count,
		);
		if uv_source_format == UvSourceFormat::F32 {
			pack_f32_uvs(
				&source[source_uvs],
				&mut output[uvs.start - source_byte_count..uvs.end - source_byte_count],
				vertex_count,
			);
		}
		output[meshlets.start - source_byte_count..meshlets.end - source_byte_count]
			.copy_from_slice(as_byte_slice(runtime_meshlets.as_slice()));

		Some(Self {
			staging,
			streams: PreparedGpuMeshStreams {
				positions,
				normals,
				uvs,
				vertex_indices,
				primitive_indices,
				meshlets,
				skinning_normals: has_skinned_primitives.then_some(source_normals),
				skinning_joints,
				skinning_weights,
			},
			primitives: prepared_primitives,
			vertex_count,
			primitive_count,
			triangle_count,
			meshlet_count,
			skinning_vertex_count,
		})
	}

	/// Builds transfer-ready owned geometry from a generated mesh without borrowing GPU recording state.
	///
	/// The returned mesh keeps its staging lease until the transfer frame completes.
	pub(super) async fn prepare_generated_mesh(
		generator: &dyn MeshGenerator,
		upload_staging: std::sync::Arc<super::upload_staging::UploadStagingArena>,
	) -> Option<Self> {
		let positions = generator.positions();
		let normals = generator.normals();
		let uvs = generator.uvs();

		if positions.len() != normals.len() || positions.len() != uvs.len() {
			log::error!(
				"Generated mesh attributes are inconsistent. The most likely cause is that the mesh generator returned mismatched vertex attribute counts."
			);
			return None;
		}
		let indices = validated_generated_indices(generator.indices().as_ref(), positions.len())?;

		let (vertex_indices, primitive_indices, meshlets) =
			GPUVertexDataManager::build_generated_meshlets(&indices, positions.as_ref()).ok()?;
		let Some(meshlet_count) = u32::try_from(meshlets.len()).ok() else {
			log::error!(
				"Generated mesh has too many meshlets. The most likely cause is that the generator exceeded the visibility meshlet metadata limit."
			);
			return None;
		};
		let sizes = [
			std::mem::size_of_val(positions.as_ref()),
			normals.len().checked_mul(VERTEX_NORMAL_BUFFER_STRIDE as usize)?,
			uvs.len().checked_mul(VERTEX_UV_BUFFER_STRIDE as usize)?,
			std::mem::size_of_val(vertex_indices.as_slice()),
			std::mem::size_of_val(primitive_indices.as_slice()),
			std::mem::size_of_val(meshlets.as_slice()),
		];
		let mut cursor = 0usize;
		let position_range = take_range_aligned(&mut cursor, sizes[0], 4);
		let normal_range = take_range_aligned(&mut cursor, sizes[1], 4);
		let uv_range = take_range_aligned(&mut cursor, sizes[2], 4);
		let vertex_index_range = take_range_aligned(&mut cursor, sizes[3], 4);
		let primitive_index_range = take_range_aligned(&mut cursor, sizes[4], 4);
		let meshlet_range = take_range_aligned(&mut cursor, sizes[5], 4);
		let backing_size = cursor;

		let mut staging = upload_staging.allocate(backing_size, 256).await.or_else(|| {
			log::error!(
				"Generated mesh exceeds the GPU upload arena. The most likely cause is that its geometry is larger than the configured upload capacity."
			);
			None
		})?;
		let backing = staging.bytes_mut();
		backing[position_range.clone()].copy_from_slice(as_byte_slice(positions.as_ref()));
		for (destination, &normal) in backing[normal_range.clone()]
			.chunks_exact_mut(VERTEX_NORMAL_BUFFER_STRIDE as usize)
			.zip(normals.iter())
		{
			let encoded = encode_octahedral_normal(normal);
			destination[..2].copy_from_slice(&encoded[0].to_ne_bytes());
			destination[2..].copy_from_slice(&encoded[1].to_ne_bytes());
		}
		for (destination, &(u, v)) in backing[uv_range.clone()]
			.chunks_exact_mut(VERTEX_UV_BUFFER_STRIDE as usize)
			.zip(uvs.iter())
		{
			destination[..2].copy_from_slice(&half::f16::from_f32(u).to_bits().to_ne_bytes());
			destination[2..].copy_from_slice(&half::f16::from_f32(v).to_bits().to_ne_bytes());
		}
		backing[vertex_index_range.clone()].copy_from_slice(as_byte_slice(vertex_indices.as_slice()));
		backing[primitive_index_range.clone()].copy_from_slice(as_byte_slice(primitive_indices.as_slice()));
		backing[meshlet_range.clone()].copy_from_slice(as_byte_slice(meshlets.as_slice()));

		Some(Self {
			staging,
			streams: PreparedGpuMeshStreams {
				positions: position_range,
				normals: normal_range,
				uvs: uv_range,
				vertex_indices: vertex_index_range,
				primitive_indices: primitive_index_range,
				meshlets: meshlet_range,
				skinning_normals: None,
				skinning_joints: None,
				skinning_weights: None,
			},
			primitives: vec![PreparedGpuMeshPrimitive {
				mesh: MeshPrimitive {
					meshlet_count,
					meshlet_offset: 0,
					vertex_offset: 0,
					primitive_offset: 0,
					triangle_offset: 0,
					skinning_source_vertex_offset: None,
					skinning_vertex_count: 0,
				},
				skinning: None,
			}],
			vertex_count: positions.len(),
			primitive_count: vertex_indices.len(),
			triangle_count: primitive_indices.len(),
			meshlet_count: meshlets.len(),
			skinning_vertex_count: 0,
		})
	}

	/// Returns the number of render-facing primitives produced by this prepared mesh.
	///
	/// Use this before GPU recording to validate separately retained material and skin metadata.
	pub(super) fn render_primitive_count(&self) -> usize {
		self.primitives.len()
	}

	/// Returns the upload-buffer lease after its GPU copies have been recorded.
	pub(super) fn into_staging(self) -> super::upload_staging::StagingLease {
		self.staging
	}
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
		let skinning_rest_positions_buffer = context.build_buffer::<[[f32; 3]; MAX_VERTICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Rest Positions")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let skinning_rest_normals_buffer = context.build_buffer::<[[f32; 3]; MAX_VERTICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Rest Normals")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let skinning_joints_buffer = context.build_buffer::<[[u16; 4]; MAX_VERTICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Joints")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let skinning_weights_buffer = context.build_buffer::<[[f32; 4]; MAX_VERTICES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Skinning Weights")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		Self {
			visibility_info: VisibilityInfo::default(),
			skinning_source_vertex_count: 0,
			vertex_positions_buffer: vertex_positions_buffer_handle,
			vertex_normals_buffer: vertex_normals_buffer_handle,
			vertex_uvs_buffer: vertex_uv_buffer_handle,
			vertex_indices_buffer: vertex_indices_buffer_handle,
			primitive_indices_buffer: primitive_indices_buffer_handle,
			meshlets_data_buffer,
			skinning_rest_positions_buffer,
			skinning_rest_normals_buffer,
			skinning_joints_buffer,
			skinning_weights_buffer,
		}
	}

	/// Records a prepared mesh into visibility GPU storage without performing resource I/O.
	pub(super) fn write_prepared_gpu_mesh_data_and_return_mesh_object(
		&mut self,
		c: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		prepared: &PreparedGpuMesh,
	) -> Option<MeshData> {
		let next_visibility_info = self.ensure_geometry_capacity(
			prepared.vertex_count,
			prepared.primitive_count,
			prepared.triangle_count,
			prepared.meshlet_count,
		)?;
		let next_skinning_source_vertex_count = self.ensure_skinning_source_capacity(prepared.skinning_vertex_count)?;

		let staging_base = prepared.staging.offset();
		let positions_staging_offset = staging_base + prepared.streams.positions.start;
		let normals_staging_offset = staging_base + prepared.streams.normals.start;
		let uvs_staging_offset = staging_base + prepared.streams.uvs.start;
		let vertex_indices_staging_offset = staging_base + prepared.streams.vertex_indices.start;
		let primitive_indices_staging_offset = staging_base + prepared.streams.primitive_indices.start;
		let meshlets_staging_offset = staging_base + prepared.streams.meshlets.start;
		let skinning_normals_staging_offset = prepared
			.streams
			.skinning_normals
			.as_ref()
			.map(|range| staging_base + range.start);
		let skinning_joints_staging_offset = prepared
			.streams
			.skinning_joints
			.as_ref()
			.map(|range| staging_base + range.start);
		let skinning_weights_staging_offset = prepared
			.streams
			.skinning_weights
			.as_ref()
			.map(|range| staging_base + range.start);

		let vertex_offset = self.visibility_info.vertex_count as usize;
		let primitive_offset = self.visibility_info.primitives_count as usize;
		let triangle_offset = self.visibility_info.triangle_count as usize;
		let meshlet_offset = self.visibility_info.meshlet_count as usize;
		c.copy_buffers(&[
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				positions_staging_offset,
				self.vertex_positions_buffer.into(),
				vertex_offset * std::mem::size_of::<(f32, f32, f32)>(),
				prepared.streams.positions.len(),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				normals_staging_offset,
				self.vertex_normals_buffer.into(),
				vertex_offset * VERTEX_NORMAL_BUFFER_STRIDE as usize,
				prepared.streams.normals.len(),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				uvs_staging_offset,
				self.vertex_uvs_buffer.into(),
				vertex_offset * VERTEX_UV_BUFFER_STRIDE as usize,
				prepared.streams.uvs.len(),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				vertex_indices_staging_offset,
				self.vertex_indices_buffer.into(),
				primitive_offset * std::mem::size_of::<u16>(),
				prepared.streams.vertex_indices.len(),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				primitive_indices_staging_offset,
				self.primitive_indices_buffer.into(),
				triangle_offset * std::mem::size_of::<[u8; 3]>(),
				prepared.streams.primitive_indices.len(),
			),
			ghi::BufferCopyDescriptor::new(
				staging_data_buffer,
				meshlets_staging_offset,
				self.meshlets_data_buffer.into(),
				meshlet_offset * std::mem::size_of::<ShaderMeshletData>(),
				prepared.streams.meshlets.len(),
			),
		]);

		let skinning_source_start = self.skinning_source_vertex_count;
		let primitives = prepared
			.primitives
			.iter()
			.map(|primitive| {
				if let Some(skinning) = &primitive.skinning {
					let normals_base = skinning_normals_staging_offset
						.expect("Prepared skinned geometry retains an aggregate normal stream for transfer.");
					let joints_base = skinning_joints_staging_offset
						.expect("Prepared skinned geometry retains an aggregate joint stream for transfer.");
					let weights_base = skinning_weights_staging_offset
						.expect("Prepared skinned geometry retains an aggregate weight stream for transfer.");
					let source_vertex_offset = primitive
						.mesh
						.skinning_source_vertex_offset
						.expect("A prepared skinning copy has a relative compact source offset.");
					let destination_vertex_offset = (skinning_source_start + source_vertex_offset) as usize;
					c.copy_buffers(&[
						ghi::BufferCopyDescriptor::new(
							staging_data_buffer,
							positions_staging_offset + skinning.positions.start,
							self.skinning_rest_positions_buffer.into(),
							destination_vertex_offset * SKINNING_POSITION_STRIDE,
							skinning.positions.len(),
						),
						ghi::BufferCopyDescriptor::new(
							staging_data_buffer,
							normals_base + skinning.normals.start,
							self.skinning_rest_normals_buffer.into(),
							destination_vertex_offset * SKINNING_NORMAL_STRIDE,
							skinning.normals.len(),
						),
						ghi::BufferCopyDescriptor::new(
							staging_data_buffer,
							joints_base + skinning.joints.start,
							self.skinning_joints_buffer.into(),
							destination_vertex_offset * SKINNING_JOINTS_STRIDE,
							skinning.joints.len(),
						),
						ghi::BufferCopyDescriptor::new(
							staging_data_buffer,
							weights_base + skinning.weights.start,
							self.skinning_weights_buffer.into(),
							destination_vertex_offset * SKINNING_WEIGHTS_STRIDE,
							skinning.weights.len(),
						),
					]);
				}

				let mut mesh = primitive.mesh.clone();
				if let Some(relative_offset) = mesh.skinning_source_vertex_offset.as_mut() {
					*relative_offset = relative_offset.checked_add(skinning_source_start).expect(
						"Visibility skinning source offset overflowed. The most likely cause is corrupted prepared mesh metadata.",
					);
				}
				mesh
			})
			.collect::<Vec<_>>();

		let mesh = MeshData {
			vertex_offset: self.visibility_info.vertex_count,
			primitive_offset: self.visibility_info.primitives_count,
			triangle_offset: self.visibility_info.triangle_count,
			meshlet_offset: self.visibility_info.meshlet_count,
			acceleration_structure: None,
			primitives,
		};
		self.skinning_source_vertex_count = next_skinning_source_vertex_count;
		self.visibility_info = next_visibility_info;

		Some(mesh)
	}

	fn build_generated_meshlets(
		indices: &[u16],
		positions: &[(f32, f32, f32)],
	) -> Result<(Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>), ()> {
		if !indices.len().is_multiple_of(3) {
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

		let primitive_offset = u32::try_from(vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh primitive-index offset exceeds its GPU representation. The most likely cause is that the generator returned an impractically large mesh."
			);
		})?;
		let triangle_offset = u32::try_from(primitive_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh triangle-index offset exceeds its GPU representation. The most likely cause is that the generator returned an impractically large mesh."
			);
		})?;
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
			cone_axis: encode_octahedral_normal((0.0, 0.0, 1.0)),
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

	/// Returns the geometry counters after this upload, or rejects only the mesh that exceeded fixed GPU storage.
	fn ensure_geometry_capacity(
		&self,
		additional_vertices: usize,
		additional_primitives: usize,
		additional_triangles: usize,
		additional_meshlets: usize,
	) -> Option<VisibilityInfo> {
		Some(VisibilityInfo {
			instance_count: self.visibility_info.instance_count,
			vertex_count: checked_visibility_capacity(
				self.visibility_info.vertex_count,
				additional_vertices,
				MAX_VERTICES,
				"vertex",
			)?,
			primitives_count: checked_visibility_capacity(
				self.visibility_info.primitives_count,
				additional_primitives,
				MAX_PRIMITIVE_TRIANGLES,
				"primitive index",
			)?,
			triangle_count: checked_visibility_capacity(
				self.visibility_info.triangle_count,
				additional_triangles,
				MAX_TRIANGLES,
				"triangle index",
			)?,
			meshlet_count: checked_visibility_capacity(
				self.visibility_info.meshlet_count,
				additional_meshlets,
				MAX_MESHLETS,
				"meshlet",
			)?,
		})
	}

	/// Rejects uploads that cannot fit in the compact immutable skinning source buffers.
	fn ensure_skinning_source_capacity(&self, additional_vertices: usize) -> Option<u32> {
		checked_visibility_capacity(
			self.skinning_source_vertex_count,
			additional_vertices,
			MAX_VERTICES,
			"skinning source vertex",
		)
	}
}

/// Adds one mesh count without letting a single oversized upload stop the shared transfer worker.
fn checked_visibility_capacity(current: u32, additional: usize, limit: usize, element: &str) -> Option<u32> {
	let Some(total) = (current as usize).checked_add(additional) else {
		log::error!(
			"Visibility {element} count overflowed. The most likely cause is corrupted prepared mesh metadata containing an invalid count."
		);
		return None;
	};
	if total > limit {
		log::error!(
			"Visibility {element} buffer limit exceeded. The most likely cause is that the scene contains more {element} data than the visibility pipeline supports."
		);
		return None;
	}
	let Ok(total) = u32::try_from(total) else {
		log::error!(
			"Visibility {element} count exceeds its GPU representation. The most likely cause is corrupted prepared mesh metadata containing an invalid count."
		);
		return None;
	};
	Some(total)
}

/// Returns a typed resource stream count after validating its complete byte layout.
fn validated_stream_count(stream: &ResourceStream, name: &str, expected_stride: usize) -> Option<usize> {
	if stream.stride != expected_stride || expected_stride == 0 || !stream.size.is_multiple_of(expected_stride) {
		log::error!(
			"Mesh {name} stream has an invalid layout. The most likely cause is malformed or incompatible baked stream metadata; expected stride {expected_stride}, found stride {} and size {}.",
			stream.stride,
			stream.size
		);
		return None;
	}
	Some(stream.size / expected_stride)
}

/// Computes one prepared stream size without accepting count multiplication overflow.
fn checked_mesh_byte_size(element_count: usize, stride: usize, name: &str) -> Option<usize> {
	let Some(byte_count) = element_count.checked_mul(stride) else {
		log::error!(
			"Prepared mesh {name} byte count overflowed. The most likely cause is malformed metadata containing an invalid element count."
		);
		return None;
	};
	Some(byte_count)
}

/// Narrows generated indices only after proving that every value addresses an available vertex.
fn validated_generated_indices(indices: &[u32], vertex_count: usize) -> Option<Vec<u16>> {
	let mut validated = Vec::new();
	if validated.try_reserve_exact(indices.len()).is_err() {
		log::error!(
			"Generated mesh indices could not be allocated. The most likely cause is that the generator returned an impractically large index list."
		);
		return None;
	}
	for (index_position, &index) in indices.iter().enumerate() {
		if index as usize >= vertex_count {
			log::error!(
				"Generated mesh index {index_position} references missing vertex {index}. The most likely cause is that the generator returned an index outside its {vertex_count} positions."
			);
			return None;
		}
		let Ok(index) = u16::try_from(index) else {
			log::error!(
				"Generated mesh index {index_position} exceeds the u16 vertex-index limit. The most likely cause is that one generated primitive contains more than 65536 addressable vertices."
			);
			return None;
		};
		validated.push(index);
	}
	Some(validated)
}

/// Confirms that primitive-local metadata consumes exactly the aggregate streams prepared for upload.
fn prepared_mesh_counts_match(expected: PreparedGpuMeshCounts, actual: PreparedGpuMeshCounts) -> bool {
	if actual == expected {
		return true;
	}
	log::error!(
		"Prepared primitive counts do not match the aggregate mesh streams: expected {expected:?}, found {actual:?}. The most likely cause is inconsistent or overlapping baked primitive ranges."
	);
	false
}

/// Reserves one contiguous range in an already size-checked owned backing.
fn take_range(cursor: &mut usize, size: usize) -> std::ops::Range<usize> {
	let start = *cursor;
	*cursor += size;
	start..*cursor
}

/// Reserves one GPU-copy-compatible aligned range in a size-checked staging lease.
fn take_range_aligned(cursor: &mut usize, size: usize, alignment: usize) -> std::ops::Range<usize> {
	*cursor = cursor.next_multiple_of(alignment);
	take_range(cursor, size)
}

/// Converts validated resource metadata and loaded meshlet bytes into transfer-ready primitive records.
fn build_prepared_resource_primitives(
	mesh: &Mesh,
	meshlet_bytes: &[u8],
	expected_counts: PreparedGpuMeshCounts,
) -> Option<(Vec<PreparedGpuMeshPrimitive>, Vec<ShaderMeshletData>)> {
	let mut primitives = Vec::new();
	if primitives.try_reserve_exact(mesh.primitives.len()).is_err() {
		log::error!(
			"Prepared primitive metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large primitive list."
		);
		return None;
	}
	let mut runtime_meshlets = Vec::new();
	if runtime_meshlets.try_reserve_exact(expected_counts.meshlets).is_err() {
		log::error!(
			"Prepared meshlet metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large meshlet list."
		);
		return None;
	}
	let mut vertex_offset = 0usize;
	let mut primitive_offset = 0usize;
	let mut triangle_offset = 0usize;
	let mut skinning_vertex_offset = 0usize;

	for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
		let (Ok(meshlet_offset), Ok(render_vertex_offset), Ok(primitive_meshlet_offset), Ok(primitive_triangle_offset)) = (
			u32::try_from(runtime_meshlets.len()),
			u32::try_from(vertex_offset),
			u32::try_from(primitive_offset),
			u32::try_from(triangle_offset),
		) else {
			log::error!(
				"Mesh primitive {primitive_index} offsets exceed their GPU representation. The most likely cause is overflowing baked primitive metadata."
			);
			return None;
		};
		let Some(meshlet_stream) = primitive.meshlet_stream() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet stream. The most likely cause is incomplete baked primitive metadata."
			);
			return None;
		};
		if validated_stream_count(meshlet_stream, "primitive meshlet", RESOURCE_MESHLET_STRIDE).is_none() {
			return None;
		}
		let Some(meshlet_end) = meshlet_stream.offset.checked_add(meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range overflows. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		let Some(source) = meshlet_bytes.get(meshlet_stream.offset..meshlet_end) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate meshlet stream."
			);
			return None;
		};
		let mut local_primitive_offset = 0u32;
		let mut local_triangle_offset = 0u32;

		for bytes in source.chunks_exact(RESOURCE_MESHLET_STRIDE) {
			let meshlet = read_resource_meshlet(bytes);
			runtime_meshlets.push(ShaderMeshletData {
				primitive_offset: local_primitive_offset,
				triangle_offset: local_triangle_offset,
				primitive_count: meshlet.primitive_count,
				triangle_count: meshlet.triangle_count,
				center_radius: meshlet.center_radius,
				cone_apex_cutoff: meshlet.cone_apex_cutoff,
				cone_axis: encode_octahedral_normal((meshlet.cone_axis[0], meshlet.cone_axis[1], meshlet.cone_axis[2])),
			});
			let (Some(next_local_primitive_offset), Some(next_local_triangle_offset)) = (
				local_primitive_offset.checked_add(meshlet.primitive_count),
				local_triangle_offset.checked_add(meshlet.triangle_count),
			) else {
				log::error!(
					"Mesh primitive {primitive_index} meshlet counts overflow. The most likely cause is corrupted baked meshlet metadata."
				);
				return None;
			};
			let (Some(next_primitive_offset), Some(next_triangle_offset)) = (
				primitive_offset.checked_add(meshlet.primitive_count as usize),
				triangle_offset.checked_add(meshlet.triangle_count as usize),
			) else {
				log::error!("Mesh aggregate index counts overflow. The most likely cause is corrupted baked meshlet metadata.");
				return None;
			};
			local_primitive_offset = next_local_primitive_offset;
			local_triangle_offset = next_local_triangle_offset;
			primitive_offset = next_primitive_offset;
			triangle_offset = next_triangle_offset;
		}

		let (relative_skinning_offset, skinning) = if primitive.skin.is_some() {
			let Some(positions) = primitive.stream(Streams::Vertices(VertexSemantics::Position)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its position stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(normals) = primitive.stream(Streams::Vertices(VertexSemantics::Normal)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its normal stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(joints) = primitive.stream(Streams::Vertices(VertexSemantics::Joints)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its joint-index stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Some(weights) = primitive.stream(Streams::Vertices(VertexSemantics::Weights)) else {
				log::error!(
					"Skinned primitive {primitive_index} is missing its vertex-weight stream. The most likely cause is incomplete baked skinning metadata."
				);
				return None;
			};
			let Ok(relative_offset) = u32::try_from(skinning_vertex_offset) else {
				log::error!(
					"Skinned primitive {primitive_index} source offset exceeds its GPU representation. The most likely cause is overflowing baked primitive metadata."
				);
				return None;
			};
			let Some(next_skinning_vertex_offset) = skinning_vertex_offset.checked_add(primitive.vertex_count as usize) else {
				log::error!(
					"Skinned primitive {primitive_index} vertex count overflows. The most likely cause is corrupted baked primitive metadata."
				);
				return None;
			};
			skinning_vertex_offset = next_skinning_vertex_offset;
			let checked_range = |stream: &ResourceStream, semantic: &str| {
				let Some(end) = stream.offset.checked_add(stream.size) else {
					log::error!(
						"Skinned primitive {primitive_index} {semantic} range overflows. The most likely cause is corrupted baked primitive metadata."
					);
					return None;
				};
				Some(stream.offset..end)
			};

			(
				Some(relative_offset),
				Some(PreparedGpuSkinningCopy {
					positions: checked_range(positions, "position")?,
					normals: checked_range(normals, "normal")?,
					joints: checked_range(joints, "joint-index")?,
					weights: checked_range(weights, "vertex-weight")?,
				}),
			)
		} else {
			(None, None)
		};

		let Ok(meshlet_count) = u32::try_from(source.len() / RESOURCE_MESHLET_STRIDE) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet count exceeds its GPU representation. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		let Some(next_vertex_offset) = vertex_offset.checked_add(primitive.vertex_count as usize) else {
			log::error!(
				"Mesh primitive {primitive_index} vertex count overflows. The most likely cause is corrupted baked primitive metadata."
			);
			return None;
		};
		primitives.push(PreparedGpuMeshPrimitive {
			mesh: MeshPrimitive {
				meshlet_count,
				meshlet_offset,
				vertex_offset: render_vertex_offset,
				primitive_offset: primitive_meshlet_offset,
				triangle_offset: primitive_triangle_offset,
				skinning_source_vertex_offset: relative_skinning_offset,
				skinning_vertex_count: relative_skinning_offset.map_or(0, |_| primitive.vertex_count),
			},
			skinning,
		});
		vertex_offset = next_vertex_offset;
	}

	let actual_counts = PreparedGpuMeshCounts {
		vertices: vertex_offset,
		primitive_indices: primitive_offset,
		triangles: triangle_offset,
		meshlets: runtime_meshlets.len(),
		skinning_vertices: skinning_vertex_offset,
	};
	if !prepared_mesh_counts_match(expected_counts, actual_counts) {
		return None;
	}

	Some((primitives, runtime_meshlets))
}

const RESOURCE_MESHLET_STRIDE: usize = 52;
const NORMAL_F32_SOURCE_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
const UV_F16_SOURCE_STRIDE: usize = std::mem::size_of::<RuntimeVertexUv>();
const UV_F32_SOURCE_STRIDE: usize = std::mem::size_of::<[f32; 2]>();

#[derive(Clone, Copy, PartialEq, Eq)]
enum UvSourceFormat {
	F16,
	F32,
}

/// Octahedrally encodes one unit normal into two UNORM16 components.
fn encode_octahedral_normal(normal: (f32, f32, f32)) -> RuntimeVertexNormal {
	let length = normal.0.abs() + normal.1.abs() + normal.2.abs();
	if !length.is_finite() || length == 0.0 {
		return [32768, 32768];
	}

	let mut x = normal.0 / length;
	let mut y = normal.1 / length;
	let z = normal.2 / length;
	if z < 0.0 {
		let previous_x = x;
		let sign_x = if previous_x < 0.0 { -1.0 } else { 1.0 };
		let sign_y = if y < 0.0 { -1.0 } else { 1.0 };
		x = (1.0 - y.abs()) * sign_x;
		y = (1.0 - previous_x.abs()) * sign_y;
	}
	[encode_unorm16(x * 0.5 + 0.5), encode_unorm16(y * 0.5 + 0.5)]
}

/// Converts vec3f normals to octahedral runtime storage in transfer staging memory.
fn pack_f32_normals(source: &[u8], destination: &mut [u8], vertex_count: usize) {
	for index in 0..vertex_count {
		let offset = index * NORMAL_F32_SOURCE_STRIDE;
		let component = |component_offset| {
			f32::from_ne_bytes(
				source[offset + component_offset..offset + component_offset + 4]
					.try_into()
					.expect("A validated f32 normal component is four bytes."),
			)
		};
		let encoded = encode_octahedral_normal((component(0), component(4), component(8)));
		let destination_offset = index * VERTEX_NORMAL_BUFFER_STRIDE as usize;
		destination[destination_offset..destination_offset + 2].copy_from_slice(&encoded[0].to_ne_bytes());
		destination[destination_offset + 2..destination_offset + 4].copy_from_slice(&encoded[1].to_ne_bytes());
	}
}

/// Encodes one UV component using the visibility pipeline's UNORM conversion policy.
fn encode_unorm16(value: f32) -> u16 {
	(value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

/// Converts an f32 UV stream to half-float transfer storage without clamping sampler coordinates.
fn pack_f32_uvs(source: &[u8], destination: &mut [u8], vertex_count: usize) {
	for index in 0..vertex_count {
		let source_offset = index * UV_F32_SOURCE_STRIDE;
		let destination_offset = index * UV_F16_SOURCE_STRIDE;
		let u = f32::from_ne_bytes(
			source[source_offset..source_offset + 4]
				.try_into()
				.expect("A validated f32 UV is four bytes."),
		);
		let v = f32::from_ne_bytes(
			source[source_offset + 4..source_offset + 8]
				.try_into()
				.expect("A validated f32 UV is four bytes."),
		);
		destination[destination_offset..destination_offset + 2]
			.copy_from_slice(&half::f16::from_f32(u).to_bits().to_ne_bytes());
		destination[destination_offset + 2..destination_offset + 4]
			.copy_from_slice(&half::f16::from_f32(v).to_bits().to_ne_bytes());
	}
}

#[cfg(test)]
mod tests {
	use super::{
		checked_visibility_capacity, pack_f32_uvs, prepared_mesh_counts_match, validated_generated_indices, PreparedGpuMesh,
		PreparedGpuMeshCounts,
	};
	use crate::rendering::mesh::generator::BoxMeshGenerator;

	#[test]
	fn generated_mesh_preparation_owns_complete_transfer_data() {
		let bytes = Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice());
		let staging = crate::rendering::pipelines::visibility::upload_staging::UploadStagingArena::new(bytes);
		let executor = resource_management::r#async::Executor::new().expect("mesh preparation test executor");
		let prepared = executor
			.block_on(PreparedGpuMesh::prepare_generated_mesh(&BoxMeshGenerator::new(), staging))
			.expect("The built-in box should produce valid visibility geometry.");

		assert_eq!(prepared.vertex_count, 24);
		assert_eq!(prepared.primitive_count, 24);
		assert_eq!(prepared.triangle_count, 12);
		assert_eq!(prepared.meshlet_count, 1);
		assert_eq!(prepared.primitives.len(), 1);
		assert_eq!(prepared.render_primitive_count(), 1);
		assert_eq!(prepared.primitives[0].mesh.meshlet_count, 1);
	}

	#[test]
	fn generated_indices_are_checked_before_u16_narrowing() {
		assert_eq!(validated_generated_indices(&[0, 2, 1], 3), Some(vec![0, 2, 1]));
		assert!(validated_generated_indices(&[3], 3).is_none());
		assert!(validated_generated_indices(&[u16::MAX as u32 + 1], u16::MAX as usize + 2).is_none());
	}

	#[test]
	fn visibility_capacity_rejects_only_the_overflowing_upload() {
		assert_eq!(checked_visibility_capacity(3, 2, 5, "test"), Some(5));
		assert_eq!(checked_visibility_capacity(3, 3, 5, "test"), None);
		assert_eq!(checked_visibility_capacity(1, usize::MAX, usize::MAX, "test"), None);
	}

	#[test]
	fn primitive_counts_must_consume_the_aggregate_streams() {
		let expected = PreparedGpuMeshCounts {
			vertices: 6,
			primitive_indices: 6,
			triangles: 2,
			meshlets: 1,
			skinning_vertices: 0,
		};
		assert!(prepared_mesh_counts_match(expected, expected));
		assert!(!prepared_mesh_counts_match(
			expected,
			PreparedGpuMeshCounts {
				primitive_indices: 5,
				..expected
			}
		));
	}

	#[test]
	fn packed_uvs_preserve_wrapping_coordinates() {
		let values = [[-0.5f32, 2.0f32], [1.25f32, -3.0f32]];
		let source = values
			.iter()
			.flat_map(|uv| uv.iter().flat_map(|component| component.to_ne_bytes()))
			.collect::<Vec<_>>();
		let mut packed = [0u8; 8];

		pack_f32_uvs(&source, &mut packed, values.len());

		let decoded = packed
			.chunks_exact(2)
			.map(|bytes| half::f16::from_bits(u16::from_ne_bytes(bytes.try_into().unwrap())).to_f32())
			.collect::<Vec<_>>();
		assert_eq!(decoded, vec![-0.5, 2.0, 1.25, -3.0]);
	}
}
const SKINNING_POSITION_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
const SKINNING_NORMAL_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
const SKINNING_JOINTS_STRIDE: usize = std::mem::size_of::<[u16; 4]>();
const SKINNING_WEIGHTS_STRIDE: usize = std::mem::size_of::<[f32; 4]>();

const _: () = {
	assert!(SKINNING_POSITION_STRIDE == 12);
	assert!(SKINNING_NORMAL_STRIDE == 12);
	assert!(SKINNING_JOINTS_STRIDE == 8);
	assert!(SKINNING_WEIGHTS_STRIDE == 16);
};

/// Validates a primitive-local skinning stream against its loaded aggregate stream.
fn validate_skinning_primitive_stream(
	primitive: &ResourcePrimitive,
	primitive_index: usize,
	aggregate_stream: &ResourceStream,
	semantic: VertexSemantics,
	expected_stride: usize,
) -> Result<(), ()> {
	if aggregate_stream.stride != expected_stride || !aggregate_stream.size.is_multiple_of(expected_stride) {
		log::error!(
			"Skinned mesh {semantic:?} stream has invalid aggregate metadata. The most likely cause is that the mesh was baked with an incompatible vertex format; expected stride {expected_stride}, found stride {} and size {}.",
			aggregate_stream.stride,
			aggregate_stream.size
		);
		return Err(());
	}

	let Some(primitive_stream) = primitive.stream(Streams::Vertices(semantic)) else {
		log::error!(
			"Skinned primitive {primitive_index} is missing its {semantic:?} stream. The most likely cause is that the mesh was baked without complete per-primitive skinning metadata."
		);
		return Err(());
	};

	let Some(expected_size) = (primitive.vertex_count as usize).checked_mul(expected_stride) else {
		log::error!(
			"Skinned primitive {primitive_index} has invalid {semantic:?} stream metadata. The most likely cause is an overflowing vertex count in the baked mesh."
		);
		return Err(());
	};
	if primitive_stream.stride != expected_stride
		|| primitive_stream.size != expected_size
		|| !primitive_stream.offset.is_multiple_of(expected_stride)
	{
		log::error!(
			"Skinned primitive {primitive_index} has invalid {semantic:?} stream metadata. The most likely cause is that its offset, stride, or byte size does not match its {} vertices; expected vertex-aligned offset, stride {expected_stride}, and size {expected_size}, found offset {}, stride {}, and size {}.",
			primitive.vertex_count,
			primitive_stream.offset,
			primitive_stream.stride,
			primitive_stream.size
		);
		return Err(());
	}

	let Some(stream_end) = primitive_stream.offset.checked_add(primitive_stream.size) else {
		log::error!(
			"Skinned primitive {primitive_index} has an invalid {semantic:?} stream range. The most likely cause is an overflowing byte offset in the baked mesh."
		);
		return Err(());
	};
	if stream_end > aggregate_stream.size {
		log::error!(
			"Skinned primitive {primitive_index} has an out-of-bounds {semantic:?} stream range. The most likely cause is that its primitive-local range does not refer to the baked aggregate stream."
		);
		return Err(());
	}

	Ok(())
}

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

/// The `LoadedPrimitiveValidation` struct retains the ranges needed to validate loaded bytes without borrowing the resource.
struct LoadedPrimitiveValidation {
	vertex_count: u32,
	vertex_indices: Option<ResourceStream>,
	triangle_indices: Option<ResourceStream>,
	meshlets: Option<ResourceStream>,
	joints: Option<ResourceStream>,
	weights: Option<ResourceStream>,
	palette_len: Option<usize>,
}

/// Rejects meshlet references that would address vertices or triangle lanes outside their primitive-local ranges.
fn validate_loaded_mesh_indices(
	primitives: &[LoadedPrimitiveValidation],
	loaded: &resource_management::resource::ReadTargets<'_>,
) -> Result<(), ()> {
	let Some(vertex_indices) = loaded.stream("VertexIndices") else {
		log::error!(
			"Loaded mesh data is missing its vertex-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(triangle_indices) = loaded.stream("MeshletIndices") else {
		log::error!(
			"Loaded mesh data is missing its meshlet triangle-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(meshlets) = loaded.stream("Meshlets") else {
		log::error!(
			"Loaded mesh data is missing its meshlet stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let vertex_indices = vertex_indices.buffer();
	let triangle_indices = triangle_indices.buffer();
	let meshlets = meshlets.buffer();

	for (primitive_index, primitive) in primitives.iter().enumerate() {
		let Some(vertex_stream) = primitive.vertex_indices.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its vertex-index stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		let Some(triangle_stream) = primitive.triangle_indices.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet triangle-index stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		let Some(meshlet_stream) = primitive.meshlets.as_ref() else {
			log::error!(
				"Mesh primitive {primitive_index} is missing its meshlet stream. The most likely cause is incomplete baked primitive metadata."
			);
			return Err(());
		};
		if validated_stream_count(vertex_stream, "primitive meshlet vertex-index", 2).is_none()
			|| validated_stream_count(triangle_stream, "primitive meshlet triangle-index", 1).is_none()
			|| validated_stream_count(meshlet_stream, "primitive meshlet", RESOURCE_MESHLET_STRIDE).is_none()
		{
			return Err(());
		}
		let mut vertex_cursor = vertex_stream.offset;
		let mut triangle_cursor = triangle_stream.offset;
		let Some(vertex_end) = vertex_stream.offset.checked_add(vertex_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} vertex-index range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(triangle_stream_end) = triangle_stream.offset.checked_add(triangle_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} triangle-index range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(meshlet_end) = meshlet_stream.offset.checked_add(meshlet_stream.size) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		if vertex_end > vertex_indices.len() || triangle_stream_end > triangle_indices.len() {
			log::error!(
				"Mesh primitive {primitive_index} index range is out of bounds. The most likely cause is that its baked range does not refer to the aggregate index streams."
			);
			return Err(());
		}
		let Some(meshlet_bytes) = meshlets.get(meshlet_stream.offset..meshlet_end) else {
			log::error!(
				"Mesh primitive {primitive_index} meshlet range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};

		for meshlet_bytes in meshlet_bytes.chunks_exact(RESOURCE_MESHLET_STRIDE) {
			let meshlet = read_resource_meshlet(meshlet_bytes);
			let Some(vertex_byte_count) = (meshlet.primitive_count as usize).checked_mul(2) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index count overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(next_vertex_cursor) = vertex_cursor.checked_add(vertex_byte_count) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index range overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(meshlet_vertices) = vertex_indices.get(vertex_cursor..next_vertex_cursor) else {
				log::error!(
					"Mesh primitive {primitive_index} vertex-index range is out of bounds. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			for bytes in meshlet_vertices.chunks_exact(2) {
				let index = u16::from_ne_bytes([bytes[0], bytes[1]]);
				if index as u32 >= primitive.vertex_count {
					log::error!(
						"Mesh primitive {primitive_index} references vertex {index} outside its {} vertices. The most likely cause is corrupted meshlet vertex indices.",
						primitive.vertex_count
					);
					return Err(());
				}
			}
			let Some(triangle_byte_count) = (meshlet.triangle_count as usize).checked_mul(3) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index count overflows. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			};
			let Some(triangle_end) = triangle_cursor.checked_add(triangle_byte_count) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index range overflows. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			let Some(meshlet_triangles) = triangle_indices.get(triangle_cursor..triangle_end) else {
				log::error!(
					"Mesh primitive {primitive_index} triangle-index range is out of bounds. The most likely cause is corrupted meshlet counts."
				);
				return Err(());
			};
			for &index in meshlet_triangles {
				if index as u32 >= meshlet.primitive_count {
					log::error!(
						"Mesh primitive {primitive_index} has a triangle index outside its meshlet vertex range. The most likely cause is corrupted meshlet triangle data."
					);
					return Err(());
				}
			}
			vertex_cursor = next_vertex_cursor;
			triangle_cursor = triangle_end;
			if vertex_cursor > vertex_end || triangle_cursor > triangle_stream_end {
				log::error!(
					"Mesh primitive {primitive_index} meshlet counts exceed its declared index ranges. The most likely cause is corrupted meshlet metadata."
				);
				return Err(());
			}
		}
		if vertex_cursor != vertex_end || triangle_cursor != triangle_stream_end {
			log::error!(
				"Mesh primitive {primitive_index} meshlet counts do not consume its declared index ranges. The most likely cause is inconsistent meshlet metadata."
			);
			return Err(());
		}
	}
	Ok(())
}

/// Rejects palette-local joints that could read outside the selected skin binding.
fn validate_loaded_skin_joints(
	primitives: &[LoadedPrimitiveValidation],
	loaded: &resource_management::resource::ReadTargets<'_>,
) -> Result<(), ()> {
	if !primitives.iter().any(|primitive| primitive.palette_len.is_some()) {
		return Ok(());
	}
	let Some(joints) = loaded.stream("Vertex.Joints") else {
		log::error!(
			"Loaded skinned mesh data is missing its joint-index stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	let Some(weights) = loaded.stream("Vertex.Weights") else {
		log::error!(
			"Loaded skinned mesh data is missing its vertex-weight stream. The most likely cause is that the resource loader returned an incomplete read target."
		);
		return Err(());
	};
	for (primitive_index, primitive) in primitives.iter().enumerate() {
		let Some(palette_len) = primitive.palette_len else { continue };
		let Some(stream) = primitive.joints.as_ref() else {
			log::error!(
				"Skinned primitive {primitive_index} is missing its joint-index stream. The most likely cause is incomplete baked skinning metadata."
			);
			return Err(());
		};
		let Some(weight_stream) = primitive.weights.as_ref() else {
			log::error!(
				"Skinned primitive {primitive_index} is missing its vertex-weight stream. The most likely cause is incomplete baked skinning metadata."
			);
			return Err(());
		};
		let Some(stream_end) = stream.offset.checked_add(stream.size) else {
			log::error!(
				"Skinned primitive {primitive_index} joint range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(joint_bytes) = joints.buffer().get(stream.offset..stream_end) else {
			log::error!(
				"Skinned primitive {primitive_index} joint range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(weight_end) = weight_stream.offset.checked_add(weight_stream.size) else {
			log::error!(
				"Skinned primitive {primitive_index} weight range overflows. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		let Some(weight_bytes) = weights.buffer().get(weight_stream.offset..weight_end) else {
			log::error!(
				"Skinned primitive {primitive_index} weight range is out of bounds. The most likely cause is corrupted stream metadata."
			);
			return Err(());
		};
		for (vertex, vertex_weights) in joint_bytes
			.chunks_exact(SKINNING_JOINTS_STRIDE)
			.zip(weight_bytes.chunks_exact(SKINNING_WEIGHTS_STRIDE))
		{
			for (lane, weight) in vertex.chunks_exact(2).zip(vertex_weights.chunks_exact(4)) {
				let joint = u16::from_ne_bytes([lane[0], lane[1]]);
				let weight = f32::from_ne_bytes([weight[0], weight[1], weight[2], weight[3]]);
				if weight > 0.0 && joint as usize >= palette_len {
					log::error!(
						"Skinned primitive {primitive_index} references joint {joint} outside its {palette_len}-matrix palette. The most likely cause is corrupted or legacy skinning data."
					);
					return Err(());
				}
			}
		}
	}
	Ok(())
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

/// The `MeshData` struct stores the geometry ranges needed after a mesh resource
/// enters visibility GPU storage.
#[derive(Debug, Clone)]
pub struct MeshData {
	pub primitives: Vec<MeshPrimitive>,
	/// Base position in the vertex buffer.
	pub vertex_offset: u32,
	pub primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub triangle_offset: u32,
	/// Base position in the meshlet buffer, relative to the mesh.
	pub meshlet_offset: u32,
	pub acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

/// The `MeshPrimitive` struct locates one primitive's geometry and optional skinning inputs in visibility buffers.
#[derive(Debug, Clone)]
pub struct MeshPrimitive {
	/// The meshlet count.
	pub meshlet_count: u32,
	/// Base position in the meshlet buffer, relative to the primitive.
	pub meshlet_offset: u32,
	/// Base position in the vertex buffer.
	pub vertex_offset: u32,
	/// Base position in the primitive-index buffer.
	pub primitive_offset: u32,
	/// Base triangle position in the primitive-index buffer, stored as index / 3.
	pub triangle_offset: u32,
	/// The first vertex in the compact immutable skinning source buffers, when this primitive is skinned.
	pub skinning_source_vertex_offset: Option<u32>,
	/// The number of vertices the skinning compute pass writes for this primitive.
	pub skinning_vertex_count: u32,
}

use std::collections::hash_map::Entry;

use ghi::{command_buffer::CommandBufferRecording as _, context::ContextCreate as _};
use resource_management::{
	resources::mesh::{Mesh, Primitive as ResourcePrimitive},
	types::{Stream as ResourceStream, Streams, VertexSemantics},
	Reference,
};
use utils::as_byte_slice;

use crate::rendering::{
	mesh::generator::MeshGenerator,
	pipelines::visibility::{
		RuntimeVertexNormal, RuntimeVertexUv, ShaderMeshletData, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES,
		MAX_VERTICES, VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE,
	},
	pipelines::visibility::{TRIANGLE_COUNT, VERTEX_COUNT},
};
