use super::*;

impl PreparedGpuMesh {
	/// Loads and validates a resource mesh into owned memory without borrowing transfer recording state.
	///
	/// The returned mesh keeps its staging lease until the transfer frame completes.
	pub(crate) async fn prepare_resource_mesh(
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
}
