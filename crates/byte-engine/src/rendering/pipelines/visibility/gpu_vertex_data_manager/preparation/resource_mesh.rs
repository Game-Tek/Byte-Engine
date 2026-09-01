use super::*;

/// The `RequiredResourceMeshStreams` struct keeps the aggregate baked streams needed by Visibility preparation.
struct RequiredResourceMeshStreams {
	positions: ResourceStream,
	normals: ResourceStream,
	uvs: ResourceStream,
	vertex_indices: ResourceStream,
	meshlet_indices: ResourceStream,
	meshlets: ResourceStream,
}

/// The `ResourceMeshSkinning` struct keeps validated optional skin streams and per-primitive bounds metadata.
struct ResourceMeshSkinning {
	joints: Option<ResourceStream>,
	weights: Option<ResourceStream>,
	primitive_validations: Vec<LoadedPrimitiveValidation>,
	vertex_count: usize,
}

/// The `ResourceMeshStagingLayout` struct locates source and converted streams in one staging lease.
struct ResourceMeshStagingLayout {
	streams: PreparedGpuMeshStreams,
	source_normals: std::ops::Range<usize>,
	source_uvs: std::ops::Range<usize>,
	source_byte_count: usize,
	backing_size: usize,
	uv_source_format: UvSourceFormat,
	counts: PreparedGpuMeshCounts,
}

/// Resolves the aggregate streams required by the Visibility mesh format.
fn required_resource_mesh_streams(mesh: &Mesh) -> Option<RequiredResourceMeshStreams> {
	let require = |stream: Option<ResourceStream>, name: &str, cause: &str| {
		stream.or_else(|| {
			log::error!("Mesh resource does not contain a {name} stream. The most likely cause is that {cause}.");
			None
		})
	};
	let positions = require(
		mesh.position_stream(),
		"vertex position",
		"the mesh was baked without required visibility geometry",
	)?;
	let normals = require(
		mesh.normal_stream(),
		"vertex normal",
		"the mesh was baked without required visibility geometry",
	)?;
	let uvs = require(
		mesh.uv_stream(),
		"vertex UV",
		"the mesh was baked without required visibility geometry",
	)?;
	let vertex_indices = require(
		mesh.vertex_indices_stream(),
		"vertex index",
		"the mesh was baked without meshlet vertex indices",
	)?;
	require(
		mesh.triangle_indices_stream(),
		"triangle index",
		"the mesh was baked without triangle geometry",
	)?;
	let meshlet_indices = require(
		mesh.meshlet_indices_stream(),
		"meshlet index",
		"the mesh was baked without meshlet triangle indices",
	)?;
	let meshlets = require(
		mesh.meshlets_stream(),
		"meshlet",
		"the mesh was baked without meshlet metadata",
	)?;
	Some(RequiredResourceMeshStreams {
		positions,
		normals,
		uvs,
		vertex_indices,
		meshlet_indices,
		meshlets,
	})
}

/// Builds the per-primitive metadata used to validate loaded index and skin streams.
fn primitive_validations(mesh: &Mesh) -> Option<Vec<LoadedPrimitiveValidation>> {
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

	let mut validations = Vec::new();
	validations.try_reserve_exact(mesh.primitives.len()).ok().or_else(|| {
		log::error!(
			"Mesh primitive validation metadata could not be allocated. The most likely cause is a malformed resource describing an impractically large primitive list."
		);
		None
	})?;
	for primitive in &mesh.primitives {
		validations.push(LoadedPrimitiveValidation {
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
	Some(validations)
}

/// Validates every skinned primitive against the aggregate skin stream ranges.
fn validate_resource_mesh_skinning(
	mesh: &Mesh,
	positions: &ResourceStream,
	normals: &ResourceStream,
	joints: &ResourceStream,
	weights: &ResourceStream,
) -> Option<usize> {
	let mut vertex_count = 0usize;
	for (primitive_index, primitive) in mesh
		.primitives
		.iter()
		.enumerate()
		.filter(|(_, primitive)| primitive.skin.is_some())
	{
		let streams = [
			(positions, VertexSemantics::Position, SKINNING_POSITION_STRIDE),
			(normals, VertexSemantics::Normal, SKINNING_NORMAL_STRIDE),
			(joints, VertexSemantics::Joints, SKINNING_JOINTS_STRIDE),
			(weights, VertexSemantics::Weights, SKINNING_WEIGHTS_STRIDE),
		];
		if streams.into_iter().any(|(stream, semantic, stride)| {
			validate_skinning_primitive_stream(primitive, primitive_index, stream, semantic, stride).is_err()
		}) {
			return None;
		}
		vertex_count = vertex_count.checked_add(primitive.vertex_count as usize).or_else(|| {
			log::error!(
				"Skinned mesh vertex count is too large. The most likely cause is corrupted primitive metadata containing an overflowing vertex count."
			);
			None
		})?;
	}
	Some(vertex_count)
}

/// Resolves and validates optional aggregate skinning streams.
fn resource_mesh_skinning(mesh: &Mesh, required: &RequiredResourceMeshStreams) -> Option<ResourceMeshSkinning> {
	let primitive_validations = primitive_validations(mesh)?;
	let has_skinned_primitives = mesh.primitives.iter().any(|primitive| primitive.skin.is_some());
	if !has_skinned_primitives {
		return Some(ResourceMeshSkinning {
			joints: None,
			weights: None,
			primitive_validations,
			vertex_count: 0,
		});
	}

	let joints = mesh.vertex_stream(VertexSemantics::Joints).cloned().or_else(|| {
		log::error!(
			"Skinned mesh is missing the joint-index stream. The most likely cause is that the mesh was baked without complete skinning vertex attributes."
		);
		None
	})?;
	let weights = mesh.vertex_stream(VertexSemantics::Weights).cloned().or_else(|| {
		log::error!(
			"Skinned mesh is missing the vertex-weight stream. The most likely cause is that the mesh was baked without complete skinning vertex attributes."
		);
		None
	})?;
	let vertex_count = validate_resource_mesh_skinning(mesh, &required.positions, &required.normals, &joints, &weights)?;
	Some(ResourceMeshSkinning {
		joints: Some(joints),
		weights: Some(weights),
		primitive_validations,
		vertex_count,
	})
}

/// Computes validated counts and aligned source/output ranges for one staging lease.
fn resource_mesh_staging_layout(
	mesh: &Mesh,
	required: &RequiredResourceMeshStreams,
	skinning: &ResourceMeshSkinning,
) -> Option<ResourceMeshStagingLayout> {
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
	let vertex_count = validated_stream_count(&required.positions, "position", SKINNING_POSITION_STRIDE)?;
	let normal_count = validated_stream_count(&required.normals, "normal", NORMAL_F32_SOURCE_STRIDE)?;
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
	if validated_stream_count(&required.uvs, "UV", uv_stride)? != vertex_count {
		log::error!(
			"Mesh UV count does not match its position count. The most likely cause is malformed vertex stream metadata."
		);
		return None;
	}

	let primitive_count = validated_stream_count(&required.vertex_indices, "meshlet vertex-index", 2)?;
	let meshlet_index_count = validated_stream_count(&required.meshlet_indices, "meshlet triangle-index", 1)?;
	if !meshlet_index_count.is_multiple_of(3) {
		log::error!(
			"Meshlet triangle-index stream does not contain complete triangles. The most likely cause is truncated baked meshlet index data."
		);
		return None;
	}
	let meshlet_count = validated_stream_count(&required.meshlets, "meshlet", RESOURCE_MESHLET_STRIDE)?;
	let runtime_normal_size = checked_mesh_byte_size(vertex_count, VERTEX_NORMAL_BUFFER_STRIDE as usize, "normal")?;
	let runtime_uv_size = checked_mesh_byte_size(vertex_count, VERTEX_UV_BUFFER_STRIDE as usize, "UV")?;
	let runtime_meshlet_size = checked_mesh_byte_size(meshlet_count, std::mem::size_of::<ShaderMeshletData>(), "meshlet")?;
	let source_sizes = [
		required.positions.size,
		required.normals.size,
		required.uvs.size,
		required.vertex_indices.size,
		required.meshlet_indices.size,
		required.meshlets.size,
		skinning.joints.as_ref().map_or(0, |stream| stream.size),
		skinning.weights.as_ref().map_or(0, |stream| stream.size),
	];
	source_sizes.into_iter().try_fold(0usize, usize::checked_add).or_else(|| {
		log::error!(
			"Mesh stream byte count overflowed. The most likely cause is corrupted stream metadata with invalid sizes."
		);
		None
	})?;

	let mut cursor = 0usize;
	let positions = take_range_aligned(&mut cursor, required.positions.size, 4);
	let source_normals = take_range_aligned(&mut cursor, required.normals.size, 4);
	let source_uvs = take_range_aligned(&mut cursor, required.uvs.size, 4);
	let vertex_indices = take_range_aligned(&mut cursor, required.vertex_indices.size, 4);
	let primitive_indices = take_range_aligned(&mut cursor, required.meshlet_indices.size, 4);
	let source_meshlets = take_range(&mut cursor, required.meshlets.size);
	let skinning_joints = skinning
		.joints
		.as_ref()
		.map(|stream| take_range_aligned(&mut cursor, stream.size, 4));
	let skinning_weights = skinning
		.weights
		.as_ref()
		.map(|stream| take_range_aligned(&mut cursor, stream.size, 4));
	let source_byte_count = cursor;
	let normals = take_range_aligned(&mut cursor, runtime_normal_size, 4);
	let uvs = match uv_source_format {
		UvSourceFormat::F16 => source_uvs.clone(),
		UvSourceFormat::F32 => take_range_aligned(&mut cursor, runtime_uv_size, 4),
	};
	let meshlets = take_range_aligned(&mut cursor, runtime_meshlet_size, 4);

	Some(ResourceMeshStagingLayout {
		streams: PreparedGpuMeshStreams {
			positions,
			normals,
			uvs,
			vertex_indices,
			primitive_indices,
			meshlets,
			skinning_normals: skinning.joints.is_some().then_some(source_normals.clone()),
			skinning_joints,
			skinning_weights,
		},
		source_normals,
		source_uvs,
		source_byte_count,
		backing_size: cursor,
		uv_source_format,
		counts: PreparedGpuMeshCounts {
			vertices: vertex_count,
			primitive_indices: primitive_count,
			triangles: meshlet_index_count / 3,
			meshlets: meshlet_count,
			skinning_vertices: skinning.vertex_count,
		},
	})
}

/// Loads every selected aggregate stream into the source section of a staging lease.
async fn load_resource_mesh_streams<'a>(
	resource: &mut Reference<Mesh>,
	required: &RequiredResourceMeshStreams,
	skinning: &ResourceMeshSkinning,
	backing: &'a mut [u8],
	source_byte_count: usize,
) -> Option<resource_management::resource::ReadTargets<'a>> {
	let mut source_allocator = utils::BufferAllocator::new(&mut backing[..source_byte_count]);
	let mut streams = Vec::with_capacity(if skinning.joints.is_some() { 8 } else { 6 });
	for (name, size) in [
		("Vertex.Position", required.positions.size),
		("Vertex.Normal", required.normals.size),
		("Vertex.UV", required.uvs.size),
		("VertexIndices", required.vertex_indices.size),
		("MeshletIndices", required.meshlet_indices.size),
	] {
		streams.push(resource_management::stream::StreamMut::new(
			name,
			source_allocator.take_with_offset_aligned(size, 4).1,
		));
	}
	streams.push(resource_management::stream::StreamMut::new(
		"Meshlets",
		source_allocator.take(required.meshlets.size),
	));
	if let (Some(joints), Some(weights)) = (&skinning.joints, &skinning.weights) {
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
	if validate_loaded_mesh_indices(&skinning.primitive_validations, &loaded).is_err()
		|| validate_loaded_skin_joints(&skinning.primitive_validations, &loaded).is_err()
	{
		return None;
	}
	Some(loaded)
}

impl PreparedGpuMesh {
	/// Loads and validates a resource mesh into owned memory without borrowing transfer recording state.
	///
	/// The returned mesh keeps its staging lease until the transfer frame completes.
	pub(crate) async fn prepare_resource_mesh(
		mut resource: Reference<Mesh>,
		upload_staging: std::sync::Arc<super::upload_staging::UploadStagingArena>,
	) -> Option<Self> {
		let required = required_resource_mesh_streams(resource.resource())?;
		let skinning = resource_mesh_skinning(resource.resource(), &required)?;
		let layout = resource_mesh_staging_layout(resource.resource(), &required, &skinning)?;
		let mut staging = upload_staging.allocate(layout.backing_size, 256).await.or_else(|| {
			log::error!(
				"Prepared mesh exceeds the GPU upload arena. The most likely cause is that the resource is larger than the configured upload capacity."
			);
			None
		})?;
		let backing = staging.bytes_mut();
		let (prepared_primitives, runtime_meshlets) = {
			let loaded =
				load_resource_mesh_streams(&mut resource, &required, &skinning, backing, layout.source_byte_count).await?;
			let loaded_meshlets = loaded.stream("Meshlets").or_else(|| {
				log::error!(
					"Loaded mesh data is missing its meshlet stream. The most likely cause is that the resource loader returned an incomplete read target."
				);
				None
			})?;
			build_prepared_resource_primitives(resource.resource(), loaded_meshlets.buffer(), layout.counts)?
		};

		let (source, output) = backing.split_at_mut(layout.source_byte_count);
		pack_f32_normals(
			&source[layout.source_normals.clone()],
			&mut output[layout.streams.normals.start - layout.source_byte_count
				..layout.streams.normals.end - layout.source_byte_count],
			layout.counts.vertices,
		);
		if layout.uv_source_format == UvSourceFormat::F32 {
			pack_f32_uvs(
				&source[layout.source_uvs.clone()],
				&mut output
					[layout.streams.uvs.start - layout.source_byte_count..layout.streams.uvs.end - layout.source_byte_count],
				layout.counts.vertices,
			);
		}
		output
			[layout.streams.meshlets.start - layout.source_byte_count..layout.streams.meshlets.end - layout.source_byte_count]
			.copy_from_slice(as_byte_slice(runtime_meshlets.as_slice()));

		Some(Self {
			staging,
			streams: layout.streams,
			primitives: prepared_primitives,
			vertex_count: layout.counts.vertices,
			primitive_count: layout.counts.primitive_indices,
			triangle_count: layout.counts.triangles,
			meshlet_count: layout.counts.meshlets,
			skinning_vertex_count: layout.counts.skinning_vertices,
		})
	}
}
