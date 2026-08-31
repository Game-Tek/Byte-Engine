use super::*;

/// The `VertexAttributeMask` struct keeps fixed semantic availability reusable across FBX mesh-import loops.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VertexAttributeMask(u8);

impl VertexAttributeMask {
	/// Captures authored FBX attribute availability once for all primitive batches of a mesh instance.
	pub(crate) fn from_mesh(mesh: &ufbx::Mesh) -> Self {
		let mut attributes = Self::default();

		if mesh.vertex_normal.exists {
			attributes.insert(VertexSemantics::Normal);
		}

		if mesh.vertex_tangent.exists {
			attributes.insert(VertexSemantics::Tangent);
		}

		if mesh.vertex_bitangent.exists {
			attributes.insert(VertexSemantics::BiTangent);
		}

		if mesh.vertex_uv.exists {
			attributes.insert(VertexSemantics::UV);
		}

		if mesh.vertex_color.exists {
			attributes.insert(VertexSemantics::Color);
		}

		attributes
	}

	pub(crate) fn contains(self, semantic: VertexSemantics) -> bool {
		self.0 & vertex_semantic_bit(semantic) != 0
	}

	pub(crate) fn insert(&mut self, semantic: VertexSemantics) -> bool {
		let bit = vertex_semantic_bit(semantic);

		let inserted = self.0 & bit == 0;

		self.0 |= bit;

		inserted
	}
}

/// Maps the engine's fixed vertex semantics to compact importer state.
pub(crate) const fn vertex_semantic_bit(semantic: VertexSemantics) -> u8 {
	match semantic {
		VertexSemantics::Position => 1 << 0,
		VertexSemantics::Normal => 1 << 1,
		VertexSemantics::Tangent => 1 << 2,
		VertexSemantics::BiTangent => 1 << 3,
		VertexSemantics::UV => 1 << 4,
		VertexSemantics::Color => 1 << 5,
		VertexSemantics::Joints => 1 << 6,
		VertexSemantics::Weights => 1 << 7,
	}
}

/// The `FbxMeshImportContext` struct carries per-instance data shared by every material part and primitive batch.
pub(crate) struct FbxMeshImportContext<'a> {
	node: &'a ufbx::Node,
	mesh: &'a ufbx::Mesh,
	material_node: &'a ufbx::Node,
	normal_matrix: Option<ufbx::Matrix>,
	source_attributes: VertexAttributeMask,
	skin: Option<&'a ufbx::SkinDeformer>,
	transform_node: Option<u32>,
	skin_index: Option<u32>,
	fallback_joint: Option<u16>,
	mirrored: bool,
}

impl<'a> FbxMeshImportContext<'a> {
	/// Builds reusable instance state and validates invariants before primitive batches are extracted.
	pub(crate) fn new(
		node: &'a ufbx::Node,
		mesh: &'a ufbx::Mesh,
		skin: Option<&'a ufbx::SkinDeformer>,
		transform_node: Option<u32>,
		skin_index: Option<u32>,
		fallback_joint: Option<u16>,
	) -> Result<Self, FbxImportError> {
		let determinant = ufbx::matrix_determinant(&node.geometry_to_world);

		if !determinant.is_finite() {
			return Err(FbxImportError::NonFinite("mesh instance transform determinant"));
		}

		if transform_node.is_some() && determinant.abs() <= f64::EPSILON {
			return Err(FbxImportError::NonInvertibleAnimatedMeshTransform);
		}

		let source_attributes = VertexAttributeMask::from_mesh(mesh);

		let normal_matrix = source_attributes
			.contains(VertexSemantics::Normal)
			.then(|| ufbx::matrix_for_normals(&node.geometry_to_world));

		Ok(Self {
			node,
			mesh,
			material_node: authored_material_node(node),
			normal_matrix,
			source_attributes,
			skin,
			transform_node,
			skin_index,
			fallback_joint,
			mirrored: determinant < 0.0,
		})
	}
}

/// The `FbxMeshProcessingError` enum preserves importer diagnostics and common mesh-processing failures.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FbxMeshProcessingError {
	Import(FbxImportError),
	Processing(MeshProcessingError),
}

impl std::fmt::Display for FbxMeshProcessingError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Import(error) => error.fmt(formatter),
			Self::Processing(error) => error.fmt(formatter),
		}
	}
}

impl std::error::Error for FbxMeshProcessingError {}

impl From<FbxImportError> for FbxMeshProcessingError {
	fn from(error: FbxImportError) -> Self {
		Self::Import(error)
	}
}

impl From<MeshProcessingError> for FbxMeshProcessingError {
	fn from(error: MeshProcessingError) -> Self {
		Self::Processing(error)
	}
}

/// The `FbxPrimitiveSource` struct lends remapped FBX corners to the common processor without materializing attributes.
pub(crate) struct FbxPrimitiveSource<'context, 'scene, 'batch> {
	context: &'context FbxMeshImportContext<'scene>,
	material: &'batch ReferenceModel<VariantModel>,
	source_corners: &'batch [u32],
	indices: &'batch [u32],
}

impl<'context, 'scene, 'batch> FbxPrimitiveSource<'context, 'scene, 'batch> {
	pub(crate) fn new(
		context: &'context FbxMeshImportContext<'scene>,
		material: &'batch ReferenceModel<VariantModel>,
		batch: &'batch RemappedCorners<'_>,
	) -> Result<Self, FbxImportError> {
		if batch.source_corners.is_empty() {
			return Err(FbxImportError::EmptyPrimitive);
		}
		Ok(Self {
			context,
			material,
			source_corners: &batch.source_corners,
			indices: &batch.indices,
		})
	}

	fn corner(&self, source_corner: u32) -> Result<usize, FbxImportError> {
		let corner = source_corner as usize;
		if corner >= self.context.mesh.num_indices {
			return Err(FbxImportError::InvalidCornerIndex);
		}
		Ok(corner)
	}

	fn normal(&self, corner: usize) -> Result<Option<[f32; 3]>, FbxImportError> {
		self.context
			.normal_matrix
			.as_ref()
			.map(|matrix| normalized_direction(matrix, self.context.mesh.vertex_normal[corner]))
			.transpose()
	}

	fn tangent_frame(&self, corner: usize) -> Result<(Option<[f32; 3]>, Option<[f32; 3]>, Option<[f32; 3]>), FbxImportError> {
		let normal = self.normal(corner)?;
		let transformed_bitangent = self
			.context
			.source_attributes
			.contains(VertexSemantics::BiTangent)
			.then(|| {
				normalized_direction(
					&self.context.node.geometry_to_world,
					self.context.mesh.vertex_bitangent[corner],
				)
			})
			.transpose()?;
		let tangent = self
			.context
			.source_attributes
			.contains(VertexSemantics::Tangent)
			.then(|| normalized_direction(&self.context.node.geometry_to_world, self.context.mesh.vertex_tangent[corner]))
			.transpose()?
			.map(|tangent| match normal {
				Some(normal) => orthogonalized_direction(tangent, normal),
				None => Ok(tangent),
			})
			.transpose()?;
		Ok((normal, tangent, transformed_bitangent))
	}
}

impl MeshPrimitiveSource for FbxPrimitiveSource<'_, '_, '_> {
	type Error = FbxImportError;

	fn material(&self) -> &ReferenceModel<VariantModel> {
		self.material
	}

	fn transform_node(&self) -> Option<u32> {
		self.context.transform_node
	}

	fn skin(&self) -> Option<u32> {
		self.context.skin_index
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
		Ok(self.indices.iter().enumerate().map(|(index, _)| {
			let source_index = if self.context.mirrored {
				match index % 3 {
					1 => index + 1,
					2 => index - 1,
					_ => index,
				}
			} else {
				index
			};
			self.indices
				.get(source_index)
				.copied()
				.ok_or(FbxImportError::InvalidTriangleCount)
		}))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_, Self::Error> {
		Ok(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let position = ufbx::transform_position(
				&self.context.node.geometry_to_world,
				self.context.mesh.vertex_position[corner],
			);
			vec3_to_f32(position, "mesh position")
		}))
	}

	fn normals(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
		if self.context.normal_matrix.is_none() {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			self.normal(corner)?.ok_or(FbxImportError::ZeroDirection)
		})))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.context.source_attributes.contains(VertexSemantics::Tangent) {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let (normal, tangent, bitangent) = self.tangent_frame(corner)?;
			let tangent = tangent.ok_or(FbxImportError::ZeroDirection)?;
			let handedness = match (normal, bitangent) {
				(Some(normal), Some(bitangent)) => tangent_handedness(normal, tangent, bitangent),
				_ => 1.0,
			};
			Ok([tangent[0], tangent[1], tangent[2], handedness])
		})))
	}

	fn bitangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 3], Self::Error>> + '_>, Self::Error> {
		if !self.context.source_attributes.contains(VertexSemantics::BiTangent) {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let (normal, tangent, transformed_bitangent) = self.tangent_frame(corner)?;
			match (normal, tangent, transformed_bitangent) {
				(Some(normal), Some(tangent), Some(bitangent)) => {
					let handedness = tangent_handedness(normal, tangent, bitangent);
					Ok(scale_vec3(cross_vec3(normal, tangent), handedness))
				}
				(Some(normal), None, Some(bitangent)) => orthogonalized_direction(bitangent, normal),
				(_, _, Some(bitangent)) => Ok(bitangent),
				(_, _, None) => Err(FbxImportError::ZeroDirection),
			}
		})))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			if self.context.source_attributes.contains(VertexSemantics::UV) {
				let uv = self.context.mesh.vertex_uv[corner];
				Ok([finite_f32(uv.x, "mesh UV")?, 1.0 - finite_f32(uv.y, "mesh UV")?])
			} else {
				Ok([0.0, 0.0])
			}
		})))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.context.source_attributes.contains(VertexSemantics::Color) {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let color = self.context.mesh.vertex_color[corner];
			Ok([
				finite_f32(color.x, "mesh color")?,
				finite_f32(color.y, "mesh color")?,
				finite_f32(color.z, "mesh color")?,
				finite_f32(color.w, "mesh color")?,
			])
		})))
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		let Some(skin) = self.context.skin else {
			return Ok(None);
		};
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let logical_vertex = *self
				.context
				.mesh
				.vertex_indices
				.get(corner)
				.ok_or(FbxImportError::InvalidCornerIndex)? as usize;
			let (joints, weights) = skin_weights(skin, logical_vertex, self.context.fallback_joint)?;
			Ok(VertexSkin { joints, weights })
		})))
	}
}

/// Selects one deformer whose joint weights can use the engine's automatic skinning path.
pub(crate) fn select_fbx_skin(mesh: &ufbx::Mesh) -> Result<Option<&ufbx::SkinDeformer>, FbxImportError> {
	if mesh.skin_deformers.len() > 1 {
		return Err(FbxImportError::MultipleSkinDeformers);
	}

	let Some(skin) = mesh.skin_deformers.as_ref().first().map(AsRef::as_ref) else {
		return Ok(None);
	};

	if skin.skinning_method == ufbx::SkinningMethod::BlendedDqLinear
		|| (skin.skinning_method != ufbx::SkinningMethod::DualQuaternion
			&& skin.vertices.iter().any(|vertex| vertex.dq_weight > 0.0))
	{
		return Err(FbxImportError::UnsupportedBlendedDualQuaternionSkinning);
	}

	Ok(Some(skin))
}

/// Builds one mesh-instance palette and adjusts inverse binds for the importer's flattened vertex space.
pub(crate) fn import_fbx_skin_binding(
	node: &ufbx::Node,
	skin: &ufbx::SkinDeformer,
	source_to_skeleton: &[u32],
) -> Result<(SkinBinding, Option<u16>), FbxImportError> {
	let determinant = ufbx::matrix_determinant(&node.geometry_to_world);

	if !determinant.is_finite() {
		return Err(FbxImportError::NonFinite("skinned mesh transform determinant"));
	}

	if determinant.abs() <= f64::EPSILON {
		return Err(FbxImportError::NonInvertibleSkinTransform);
	}

	let mut needs_fallback = false;

	for vertex in 0..skin.vertices.len() {
		if strongest_skin_weight_total(skin, vertex)? == 0.0 {
			needs_fallback = true;

			break;
		}
	}

	let palette_len = skin.clusters.len().saturating_add(usize::from(needs_fallback));

	if palette_len > MAX_PRIMITIVE_VERTICES {
		return Err(FbxImportError::TooManyJoints);
	}

	let geometry_world_inverse = ufbx::matrix_invert(&node.geometry_to_world);

	let mut entries = Vec::with_capacity(palette_len);

	for cluster in &skin.clusters {
		let bone = cluster.bone_node.as_ref().ok_or(FbxImportError::MissingSkinBone)?;

		// Vertices already contain `geometry_to_world`, so remove that flattened bind transform after
		// ufbx's geometry-to-bone matrix. A runtime global bone matrix can then produce the final palette.
		let adjusted = ufbx::matrix_mul(&cluster.geometry_to_bone, &geometry_world_inverse);

		entries.push(SkinPaletteEntry {
			joint: SkinJoint::Node(remap_skeleton_node(source_to_skeleton, bone.element.typed_id)?),
			adjusted_inverse_bind_matrix: matrix_to_columns(&adjusted)?,
		});
	}

	let fallback_joint = if needs_fallback {
		let index = u16::try_from(entries.len()).map_err(|_| FbxImportError::TooManyJoints)?;

		// ufbx evaluates an unweighted control point with the mesh instance transform. Binding the
		// fallback entry to that node preserves the behavior when the mesh or an ancestor animates.
		entries.push(SkinPaletteEntry {
			joint: SkinJoint::Node(remap_skeleton_node(source_to_skeleton, node.element.typed_id)?),
			adjusted_inverse_bind_matrix: matrix_to_columns(&geometry_world_inverse)?,
		});

		Some(index)
	} else {
		None
	};

	Ok((SkinBinding { entries }, fallback_joint))
}

/// Sums the retained fixed-width influences without allocating temporary weight storage.
pub(crate) fn strongest_skin_weight_total(skin: &ufbx::SkinDeformer, logical_vertex: usize) -> Result<f64, FbxImportError> {
	let influences = skin_influences(skin, logical_vertex)?;

	let mut total = 0.0;

	for influence in influences.iter().take(4) {
		if influence.cluster_index as usize >= skin.clusters.len() {
			return Err(FbxImportError::InvalidSkinCluster);
		}

		total += finite_f32(influence.weight, "skin weight")?.max(0.0) as f64;
	}

	Ok(total)
}

/// Borrows one logical vertex's sorted ufbx influence range after validating its bounds.
pub(crate) fn skin_influences(skin: &ufbx::SkinDeformer, logical_vertex: usize) -> Result<&[ufbx::SkinWeight], FbxImportError> {
	let vertex = skin.vertices.get(logical_vertex).ok_or(FbxImportError::InvalidSkinVertex)?;

	let begin = vertex.weight_begin as usize;

	let end = begin
		.checked_add(vertex.num_weights as usize)
		.ok_or(FbxImportError::InvalidSkinVertex)?;

	skin.weights.get(begin..end).ok_or(FbxImportError::InvalidSkinVertex)
}

/// Converts ufbx's affine column vectors into the serialized four-column matrix representation.
pub(crate) fn matrix_to_columns(matrix: &ufbx::Matrix) -> Result<AffineMatrix4x3Columns, FbxImportError> {
	Ok([
		[
			finite_f32(matrix.m00, "skin matrix")?,
			finite_f32(matrix.m10, "skin matrix")?,
			finite_f32(matrix.m20, "skin matrix")?,
		],
		[
			finite_f32(matrix.m01, "skin matrix")?,
			finite_f32(matrix.m11, "skin matrix")?,
			finite_f32(matrix.m21, "skin matrix")?,
		],
		[
			finite_f32(matrix.m02, "skin matrix")?,
			finite_f32(matrix.m12, "skin matrix")?,
			finite_f32(matrix.m22, "skin matrix")?,
		],
		[
			finite_f32(matrix.m03, "skin matrix")?,
			finite_f32(matrix.m13, "skin matrix")?,
			finite_f32(matrix.m23, "skin matrix")?,
		],
	])
}

/// The `FbxMeshAllocationEstimates` struct carries scene-derived capacities for reusable importer buffers.
pub(crate) struct FbxMeshAllocationEstimates {
	scratch: usize,
	corners: usize,
	remap: usize,
}

/// Estimates common-case primitive count and worst-case reusable scratch sizes from ufbx metadata.
pub(crate) fn fbx_mesh_allocation_estimates(scene: &ufbx::Scene) -> FbxMeshAllocationEstimates {
	let mut estimates = FbxMeshAllocationEstimates {
		scratch: 3,
		corners: 0,
		remap: 0,
	};

	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};

		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}

		estimates.scratch = estimates.scratch.max(mesh.max_face_triangles.saturating_mul(3));

		estimates.remap = estimates.remap.max(mesh.num_indices);

		let mesh_corners = if mesh.material_parts.is_empty() {
			mesh.num_triangles.saturating_mul(3)
		} else {
			mesh.material_parts
				.iter()
				.map(|part| part.num_triangles.saturating_mul(3))
				.max()
				.unwrap_or(0)
		};
		estimates.corners = estimates.corners.max(mesh_corners);
	}

	estimates
}

/// Builds the final engine vertex layout once from the FBX meshes that can contribute primitives.
pub(crate) fn fbx_vertex_layout(scene: &ufbx::Scene) -> Vec<VertexComponent> {
	let mut semantics = VertexAttributeMask::default();
	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};
		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}
		semantics.insert(VertexSemantics::Position);
		semantics.insert(VertexSemantics::UV);
		for semantic in [
			VertexSemantics::Normal,
			VertexSemantics::Tangent,
			VertexSemantics::BiTangent,
			VertexSemantics::Color,
		] {
			if VertexAttributeMask::from_mesh(mesh).contains(semantic) {
				semantics.insert(semantic);
			}
		}
		if !mesh.skin_deformers.is_empty() {
			semantics.insert(VertexSemantics::Joints);
			semantics.insert(VertexSemantics::Weights);
		}
	}

	[
		(VertexSemantics::Position, "vec3f"),
		(VertexSemantics::Normal, "vec3f"),
		(VertexSemantics::Tangent, "vec4f"),
		(VertexSemantics::BiTangent, "vec3f"),
		(VertexSemantics::UV, "vec2f"),
		(VertexSemantics::Color, "vec4f"),
		(VertexSemantics::Joints, "vec4u16"),
		(VertexSemantics::Weights, "vec4f"),
	]
	.into_iter()
	.filter(|(semantic, _)| semantics.contains(*semantic))
	.map(|(semantic, format)| VertexComponent {
		semantic,
		format: format.to_string(),
		channel: 0,
	})
	.collect()
}

/// Streams every mesh instance and material part through the common processor while reusing FBX topology scratch.
#[allow(dead_code)] // Buffer-owning callers still use this utility outside the direct storage path.
pub(crate) fn import_fbx_meshes<'a>(
	scene: &ufbx::Scene,
	materials: &ResolvedFbxMaterials,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	mesh_processor: MeshProcessor,
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
) -> Result<ProcessedMesh, FbxMeshProcessingError> {
	Ok(import_fbx_mesh_session(
		scene,
		materials,
		skeleton,
		source_to_skeleton,
		mesh_processor,
		allocator,
		culled_polygons,
	)?
	.finish())
}

/// Streams FBX primitives into a session that can write its final blocks directly to resource storage.
pub(crate) fn import_fbx_mesh_session<'a>(
	scene: &ufbx::Scene,
	materials: &ResolvedFbxMaterials,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	mesh_processor: MeshProcessor,
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
) -> Result<MeshProcessorSession, FbxMeshProcessingError> {
	let estimates = fbx_mesh_allocation_estimates(scene);
	let vertex_layout = fbx_vertex_layout(scene);
	let mut processor = mesh_processor.begin(vertex_layout, skeleton, Vec::new())?;
	let mut primitive_count = 0usize;

	let mut scratch = Vec::with_capacity_in(estimates.scratch, allocator);

	let mut corners = Vec::with_capacity_in(estimates.corners, allocator);

	let mut remap = Vec::with_capacity_in(estimates.remap, allocator);

	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};

		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}

		let skin = select_fbx_skin(mesh)?;

		let (skin_index, fallback_joint) = if let Some(skin) = skin {
			let (binding, fallback_joint) = import_fbx_skin_binding(node, skin, source_to_skeleton)?;
			let skin_index = processor.add_skin(binding)?;

			(Some(skin_index), fallback_joint)
		} else {
			(None, None)
		};

		let transform_node = if source_to_skeleton.is_empty() {
			None
		} else {
			Some(remap_skeleton_node(source_to_skeleton, node.element.typed_id)?)
		};

		let context = FbxMeshImportContext::new(node, mesh, skin, transform_node, skin_index, fallback_joint)?;

		// Reuse triangulation and corner-remap storage across mesh instances and material parts to bound import allocations.
		let scratch_len = mesh.max_face_triangles.saturating_mul(3).max(3);

		scratch.resize(scratch_len, 0u32);

		corners.clear();

		remap.clear();

		remap.resize(mesh.num_indices, u32::MAX);

		if mesh.material_parts.is_empty() {
			corners.reserve(mesh.num_triangles.saturating_mul(3));

			for (face_index, &face) in mesh.faces.iter().enumerate() {
				if !is_visible_polygon_face(mesh, face_index) {
					continue;
				}

				if append_triangulated_face(mesh, face, &mut scratch, &mut corners)?
					== TriangulatedFaceAppendResult::CulledDegenerate
				{
					culled_polygons.record(face.num_indices);
				}
			}

			import_fbx_material_corners(&context, 0, &corners, &mut remap, materials, &mut processor, allocator)
				.map(|count| primitive_count += count)?;
		} else {
			for part in &mesh.material_parts {
				corners.clear();

				let required_capacity = part.num_triangles.saturating_mul(3);

				if corners.capacity() < required_capacity {
					corners.reserve(required_capacity.saturating_sub(corners.len()));
				}

				for &face_index in &part.face_indices {
					let face = mesh
						.faces
						.get(face_index as usize)
						.copied()
						.ok_or(FbxImportError::InvalidFaceIndex)?;

					if face.num_indices < 3 || mesh.face_hole.get(face_index as usize).copied().unwrap_or(false) {
						continue;
					}

					if append_triangulated_face(mesh, face, &mut scratch, &mut corners)?
						== TriangulatedFaceAppendResult::CulledDegenerate
					{
						culled_polygons.record(face.num_indices);
					}
				}

				import_fbx_material_corners(
					&context,
					part.index as usize,
					&corners,
					&mut remap,
					materials,
					&mut processor,
					allocator,
				)
				.map(|count| primitive_count += count)?;
			}
		}
	}

	if primitive_count == 0 {
		return Err(FbxImportError::NoMesh.into());
	}
	Ok(processor)
}

/// Processes one triangulated material part immediately so source-corner storage can be reused by the next part.
pub(crate) fn import_fbx_material_corners<'a>(
	context: &FbxMeshImportContext<'_>,
	material_slot: usize,
	corners: &[u32],
	remap: &mut [u32],
	materials: &ResolvedFbxMaterials,
	processor: &mut MeshProcessorSession,
	allocator: &'a dyn Allocator,
) -> Result<usize, FbxMeshProcessingError> {
	if corners.is_empty() {
		return Ok(0);
	}

	let material = materials.get(material_key_for_slot(context.material_node, context.mesh, material_slot))?;
	let mut processed = 0;
	for batch in remap_triangle_corners(context.mesh.num_indices, corners, remap, allocator)? {
		let source = FbxPrimitiveSource::new(context, material, &batch)?;
		processor.push_primitive(&source).map_err(|error| match error {
			MeshPrimitiveProcessingError::Source(error) => FbxMeshProcessingError::Import(error),
			MeshPrimitiveProcessingError::Processing(error) => FbxMeshProcessingError::Processing(error),
		})?;
		processed += 1;
	}
	Ok(processed)
}

/// The `TriangulatedFaceAppendResult` enum records whether a source face produced triangles or was malformed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TriangulatedFaceAppendResult {
	Appended,
	CulledDegenerate,
}

/// The `FbxCulledPolygonCounts` struct accumulates concise import diagnostics without logging once per malformed face.
#[derive(Default)]
pub(crate) struct FbxCulledPolygonCounts {
	triangles: usize,
	quads: usize,
	polygons: usize,
}

impl FbxCulledPolygonCounts {
	/// Records one source polygon by its authored corner count for the final import summary.
	pub(crate) fn record(&mut self, corner_count: u32) {
		match corner_count {
			3 => self.triangles += 1,
			4 => self.quads += 1,
			_ => self.polygons += 1,
		}
	}

	/// Adds the malformed geometry summary to the requested resource's trace.
	pub(crate) fn trace(&self, context: BakeContext<'_>) {
		if self.triangles + self.quads + self.polygons == 0 {
			return;
		}

		context.info(format_args!(
			"Culled degenerate FBX geometry: {} triangle(s), {} quad(s), and {} other polygon(s). The most likely cause is repeated or collinear vertex positions, which produce zero-area triangles and undefined normal data.",
			self.triangles,
			self.quads,
			self.polygons,
		));
	}
}

/// Appends a triangulated face into caller-owned scratch and corner storage.
pub(crate) fn append_triangulated_face<A: Allocator>(
	mesh: &ufbx::Mesh,
	face: ufbx::Face,
	scratch: &mut [u32],
	corners: &mut Vec<u32, A>,
) -> Result<TriangulatedFaceAppendResult, FbxImportError> {
	let triangle_count = mesh.triangulate_face(scratch, face) as usize;

	let index_count = triangle_count.saturating_mul(3);

	if index_count > scratch.len() {
		return Err(FbxImportError::TriangulationOverflow);
	}

	let triangles = &scratch[..index_count];

	// Retained triangles may share malformed corner normals with a degenerate sibling, so discard the source polygon as a unit.
	for triangle in triangles.chunks_exact(3) {
		if is_degenerate_fbx_triangle(mesh, triangle)? {
			return Ok(TriangulatedFaceAppendResult::CulledDegenerate);
		}
	}

	corners.extend_from_slice(triangles);

	Ok(TriangulatedFaceAppendResult::Appended)
}

/// Rejects zero-area triangles before their undefined shading directions reach vertex attribute import.
pub(crate) fn is_degenerate_fbx_triangle(mesh: &ufbx::Mesh, triangle: &[u32]) -> Result<bool, FbxImportError> {
	let mut positions = [ufbx::Vec3::default(); 3];

	for (position, &corner) in positions.iter_mut().zip(triangle) {
		let position_index = mesh
			.vertex_position
			.indices
			.get(corner as usize)
			.ok_or(FbxImportError::InvalidCornerIndex)?;

		*position = *mesh
			.vertex_position
			.values
			.get(*position_index as usize)
			.ok_or(FbxImportError::InvalidCornerIndex)?;
	}

	// Authored zero-area faces are already degenerate in mesh-local space, so avoid repeated per-instance transforms here.
	let first_edge = [
		positions[1].x - positions[0].x,
		positions[1].y - positions[0].y,
		positions[1].z - positions[0].z,
	];

	let second_edge = [
		positions[2].x - positions[0].x,
		positions[2].y - positions[0].y,
		positions[2].z - positions[0].z,
	];

	let area = [
		first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
		first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
		first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
	];

	Ok(area == [0.0; 3])
}

/// The `RemappedCorners` struct carries one u16-compatible primitive's source-corner lookup and local indices.
pub(crate) struct RemappedCorners<'a> {
	pub(crate) source_corners: Vec<u32, &'a dyn Allocator>,
	pub(crate) indices: Vec<u32, &'a dyn Allocator>,
}

/// Splits and remaps corner-indexed triangles so every processed primitive remains representable by the engine's u16 index streams.
pub(crate) fn remap_triangle_corners<'a>(
	source_corner_count: usize,
	corners: &[u32],
	remap: &mut [u32],
	allocator: &'a dyn Allocator,
) -> Result<Vec<RemappedCorners<'a>, &'a dyn Allocator>, FbxImportError> {
	if !corners.len().is_multiple_of(3) {
		return Err(FbxImportError::InvalidTriangleCount);
	}

	if remap.len() != source_corner_count {
		return Err(FbxImportError::InvalidCornerIndex);
	}

	let unique_corner_capacity = source_corner_count.min(corners.len()).min(MAX_PRIMITIVE_VERTICES);

	let index_capacity = if source_corner_count <= MAX_PRIMITIVE_VERTICES {
		corners.len()
	} else {
		corners.len().min(MAX_PRIMITIVE_VERTICES.saturating_mul(3))
	};

	let batch_capacity = source_corner_count
		.min(corners.len())
		.div_ceil(MAX_PRIMITIVE_VERTICES.saturating_sub(2))
		.max(1);

	let mut source_corners = Vec::with_capacity_in(unique_corner_capacity, allocator);

	let mut indices = Vec::with_capacity_in(index_capacity, allocator);

	let mut batches = Vec::with_capacity_in(batch_capacity, allocator);

	for triangle in corners.chunks_exact(3) {
		let mut new_corners = 0usize;

		for &corner in triangle {
			let corner = corner as usize;

			if corner >= source_corner_count {
				return Err(FbxImportError::InvalidCornerIndex);
			}

			if remap[corner] == u32::MAX {
				new_corners += 1;
			}
		}

		if !indices.is_empty() && source_corners.len() + new_corners > MAX_PRIMITIVE_VERTICES {
			for &corner in &source_corners {
				remap[corner as usize] = u32::MAX;
			}

			batches.push(RemappedCorners {
				source_corners: std::mem::replace(
					&mut source_corners,
					Vec::with_capacity_in(unique_corner_capacity, allocator),
				),
				indices: std::mem::replace(&mut indices, Vec::with_capacity_in(index_capacity, allocator)),
			});
		}

		for &corner in triangle {
			let slot = &mut remap[corner as usize];

			if *slot == u32::MAX {
				*slot = source_corners.len() as u32;

				source_corners.push(corner);
			}

			indices.push(*slot);
		}
	}

	if !indices.is_empty() {
		for &corner in &source_corners {
			remap[corner as usize] = u32::MAX;
		}

		batches.push(RemappedCorners { source_corners, indices });
	}

	Ok(batches)
}

/// Selects and normalizes the four strongest influences, routing unweighted vertices to the animated mesh-node fallback.
pub(crate) fn skin_weights(
	skin: &ufbx::SkinDeformer,
	logical_vertex: usize,
	fallback_joint: Option<u16>,
) -> Result<([u16; 4], [f32; 4]), FbxImportError> {
	let influences = skin_influences(skin, logical_vertex)?;

	let mut joints = [0u16; 4];

	let mut weights = [0.0f32; 4];

	let mut total = 0.0f64;

	// `clean_skin_weights` makes each ufbx influence range strongest-first, so truncation does not
	// need a transient sorting buffer and remains deterministic for the fixed-width GPU stream.
	for (index, influence) in influences.iter().take(4).enumerate() {
		if influence.cluster_index as usize >= skin.clusters.len() {
			return Err(FbxImportError::InvalidSkinCluster);
		}

		joints[index] = influence.cluster_index as u16;

		weights[index] = finite_f32(influence.weight, "skin weight")?.max(0.0);

		total += weights[index] as f64;
	}

	if total > 0.0 {
		for weight in &mut weights {
			*weight = (*weight as f64 / total) as f32;
		}
	} else {
		joints[0] = fallback_joint.ok_or(FbxImportError::MissingFallbackJoint)?;

		weights[0] = 1.0;
	}

	Ok((joints, weights))
}

/// Transforms and normalizes a direction while rejecting degenerate authored values.
pub(crate) fn normalized_direction(matrix: &ufbx::Matrix, direction: ufbx::Vec3) -> Result<[f32; 3], FbxImportError> {
	let direction = ufbx::transform_direction(matrix, direction);

	normalize_vec3(vec3_to_f32(direction, "mesh direction")?)
}

/// Removes the normal component from a transformed tangent-space direction and normalizes the result.
pub(crate) fn orthogonalized_direction(direction: [f32; 3], normal: [f32; 3]) -> Result<[f32; 3], FbxImportError> {
	let alignment = dot_vec3(direction, normal);

	normalize_vec3([
		direction[0] - normal[0] * alignment,
		direction[1] - normal[1] * alignment,
		direction[2] - normal[2] * alignment,
	])
}

/// Normalizes an imported vector without allowing zero-length or non-finite shading data.
pub(crate) fn normalize_vec3(mut direction: [f32; 3]) -> Result<[f32; 3], FbxImportError> {
	let length_squared = direction.iter().map(|component| component * component).sum::<f32>();

	if !length_squared.is_finite() || length_squared <= f32::MIN_POSITIVE {
		return Err(FbxImportError::ZeroDirection);
	}

	let inverse_length = length_squared.sqrt().recip();

	for component in &mut direction {
		*component *= inverse_length;
	}

	Ok(direction)
}

/// Computes tangent-space orientation after the node's geometry transform has been applied.
pub(crate) fn tangent_handedness(normal: [f32; 3], tangent: [f32; 3], bitangent: [f32; 3]) -> f32 {
	let alignment = dot_vec3(cross_vec3(normal, tangent), bitangent);

	if alignment < 0.0 { -1.0 } else { 1.0 }
}

/// Computes the dot product used by tangent-frame orthonormalization.
pub(crate) fn dot_vec3(left: [f32; 3], right: [f32; 3]) -> f32 {
	left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

/// Computes the cross product used to reconstruct an orthonormal bitangent.
pub(crate) fn cross_vec3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

/// Applies tangent-space handedness without allocating an intermediate vector.
pub(crate) fn scale_vec3(value: [f32; 3], scale: f32) -> [f32; 3] {
	[value[0] * scale, value[1] * scale, value[2] * scale]
}

/// Converts ufbx's double-precision vectors to the engine's finite single-precision representation.
pub(crate) fn vec3_to_f32(value: ufbx::Vec3, context: &'static str) -> Result<[f32; 3], FbxImportError> {
	Ok([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
	])
}

/// Converts ufbx's x/y/z/w quaternion layout to finite single-precision components.
pub(crate) fn quat_to_f32(value: ufbx::Quat, context: &'static str) -> Result<[f32; 4], FbxImportError> {
	Ok([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
		finite_f32(value.w, context)?,
	])
}

/// Converts imported numeric data to f32 while retaining an error context for malformed files.
pub(crate) fn finite_f32(value: f64, context: &'static str) -> Result<f32, FbxImportError> {
	if value.is_finite() && value >= f32::MIN as f64 && value <= f32::MAX as f64 {
		Ok(value as f32)
	} else {
		Err(FbxImportError::NonFinite(context))
	}
}

/// Copies authored names only when they contain a useful resource label.
pub(crate) fn non_empty_name(name: &ufbx::String) -> Option<String> {
	(!name.is_empty()).then(|| name.as_ref().to_string())
}
