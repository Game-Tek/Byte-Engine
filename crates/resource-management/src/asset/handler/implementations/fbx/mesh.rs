use super::*;

/// The `FbxMeshImportContext` struct carries per-instance data shared by every material part and primitive batch.
pub(crate) struct FbxMeshImportContext<'a> {
	node: &'a ufbx::Node,
	mesh: &'a ufbx::Mesh,
	material_node: &'a ufbx::Node,
	normal_matrix: Option<ufbx::Matrix>,
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

		let normal_matrix = mesh
			.vertex_normal
			.exists
			.then(|| ufbx::matrix_for_normals(&node.geometry_to_world));

		Ok(Self {
			node,
			mesh,
			material_node: authored_material_node(node),
			normal_matrix,
			skin,
			transform_node,
			skin_index,
			fallback_joint,
			mirrored: determinant < 0.0,
		})
	}
}

/// The `FbxMeshProcessingError` type reports an FBX mesh import failure, either in the FBX data or in the shared mesh
/// processor.
pub(crate) type FbxMeshProcessingError = MeshPrimitiveProcessingError<FbxImportError>;

impl From<FbxImportError> for FbxMeshProcessingError {
	fn from(error: FbxImportError) -> Self {
		Self::Source(error)
	}
}

/// The `FbxPrimitiveSource` struct lends remapped FBX corners to the common processor without materializing attributes.
pub(crate) struct FbxPrimitiveSource<'context, 'scene, 'batch> {
	context: &'context FbxMeshImportContext<'scene>,
	/// The index of the primitive's material in the mesh's list of used materials.
	material_slot: usize,
	source_corners: &'batch [u32],
	indices: &'batch [u32],
}

impl FbxPrimitiveSource<'_, '_, '_> {
	fn corner(&self, source_corner: u32) -> Result<usize, FbxImportError> {
		let corner = source_corner as usize;
		if corner >= self.context.mesh.num_indices {
			return Err(FbxImportError::InvalidCornerIndex);
		}
		Ok(corner)
	}

	fn normal(&self, corner: usize) -> Result<Option<Vector<ModelSpace>>, FbxImportError> {
		self.context
			.normal_matrix
			.as_ref()
			.map(|matrix| normalized_direction(matrix, self.context.mesh.vertex_normal[corner]))
			.transpose()
	}

	/// Returns the corner's normal, tangent, and bitangent in that order, each `None` when the mesh does not store it.
	fn tangent_frame(&self, corner: usize) -> Result<[Option<Vector<ModelSpace>>; 3], FbxImportError> {
		let mesh = self.context.mesh;
		let geometry_to_world = &self.context.node.geometry_to_world;
		let normal = self.normal(corner)?;
		let transformed_bitangent = mesh
			.vertex_bitangent
			.exists
			.then(|| normalized_direction(geometry_to_world, mesh.vertex_bitangent[corner]))
			.transpose()?;
		let tangent = mesh
			.vertex_tangent
			.exists
			.then(|| normalized_direction(geometry_to_world, mesh.vertex_tangent[corner]))
			.transpose()?
			.map(|tangent| match normal {
				Some(normal) => orthogonalized_direction(tangent, normal),
				None => Ok(tangent),
			})
			.transpose()?;
		Ok([normal, tangent, transformed_bitangent])
	}
}

impl MeshPrimitiveSource for FbxPrimitiveSource<'_, '_, '_> {
	type Error = FbxImportError;

	fn material_slot(&self) -> usize {
		self.material_slot
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

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
		Ok(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let position = ufbx::transform_position(
				&self.context.node.geometry_to_world,
				self.context.mesh.vertex_position[corner],
			);
			vec3_to_f32(position, "mesh position").map(Point::from_array)
		}))
	}

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		if self.context.normal_matrix.is_none() {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			self.normal(corner)?.ok_or(FbxImportError::ZeroDirection)
		})))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.context.mesh.vertex_tangent.exists {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			let [normal, tangent, bitangent] = self.tangent_frame(corner)?;
			let tangent = tangent.ok_or(FbxImportError::ZeroDirection)?;
			let handedness = match (normal, bitangent) {
				(Some(normal), Some(bitangent)) => tangent_handedness(normal, tangent, bitangent),
				_ => 1.0,
			};
			Ok([tangent.x(), tangent.y(), tangent.z(), handedness])
		})))
	}

	fn bitangents(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		if !self.context.mesh.vertex_bitangent.exists {
			return Ok(None);
		}
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			match self.tangent_frame(corner)? {
				[Some(normal), Some(tangent), Some(bitangent)] => {
					let handedness = tangent_handedness(normal, tangent, bitangent);
					Ok(normal.cross(tangent) * handedness)
				}
				[Some(normal), None, Some(bitangent)] => orthogonalized_direction(bitangent, normal),
				[_, _, Some(bitangent)] => Ok(bitangent),
				[_, _, None] => Err(FbxImportError::ZeroDirection),
			}
		})))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		Ok(Some(self.source_corners.iter().map(|&source_corner| {
			let corner = self.corner(source_corner)?;
			if self.context.mesh.vertex_uv.exists {
				let uv = self.context.mesh.vertex_uv[corner];
				Ok([finite_f32(uv.x, "mesh UV")?, 1.0 - finite_f32(uv.y, "mesh UV")?])
			} else {
				Ok([0.0, 0.0])
			}
		})))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.context.mesh.vertex_color.exists {
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
		let (.., total) = strongest_skin_influences(skin, vertex)?;
		if total == 0.0 {
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
			adjusted_inverse_bind_matrix: matrix_to_affine(&adjusted)?,
		});
	}

	let fallback_joint = if needs_fallback {
		let index = u16::try_from(entries.len()).map_err(|_| FbxImportError::TooManyJoints)?;

		// ufbx evaluates an unweighted control point with the mesh instance transform. Binding the
		// fallback entry to that node preserves the behavior when the mesh or an ancestor animates.
		entries.push(SkinPaletteEntry {
			joint: SkinJoint::Node(remap_skeleton_node(source_to_skeleton, node.element.typed_id)?),
			adjusted_inverse_bind_matrix: matrix_to_affine(&geometry_world_inverse)?,
		});

		Some(index)
	} else {
		None
	};

	Ok((SkinBinding { entries }, fallback_joint))
}

/// Reads a logical vertex's four strongest influences as joints, clamped weights, and their unnormalized total.
pub(crate) fn strongest_skin_influences(
	skin: &ufbx::SkinDeformer,
	logical_vertex: usize,
) -> Result<([u16; 4], [f32; 4], f64), FbxImportError> {
	let mut joints = [0u16; 4];

	let mut weights = [0.0f32; 4];

	let mut total = 0.0f64;

	// `clean_skin_weights` makes each ufbx influence range strongest-first, so truncation does not
	// need a transient sorting buffer and remains deterministic for the fixed-width GPU stream.
	for (index, influence) in skin_influences(skin, logical_vertex)?.iter().take(4).enumerate() {
		if influence.cluster_index as usize >= skin.clusters.len() {
			return Err(FbxImportError::InvalidSkinCluster);
		}

		joints[index] = influence.cluster_index as u16;

		weights[index] = finite_f32(influence.weight, "skin weight")?.max(0.0);

		total += weights[index] as f64;
	}

	Ok((joints, weights, total))
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

/// Converts ufbx's affine column vectors into an engine affine matrix.
pub(crate) fn matrix_to_affine(matrix: &ufbx::Matrix) -> Result<AffineMatrix, FbxImportError> {
	let column = |x, y, z| vec3_to_f32(ufbx::Vec3 { x, y, z }, "skin matrix");
	Ok(AffineMatrix::from_columns([
		column(matrix.m00, matrix.m10, matrix.m20)?,
		column(matrix.m01, matrix.m11, matrix.m21)?,
		column(matrix.m02, matrix.m12, matrix.m22)?,
		column(matrix.m03, matrix.m13, matrix.m23)?,
	]))
}

/// Yields each mesh instance that can contribute triangles, with its node, in scene node order.
///
/// Material keys, the vertex layout, scratch estimates, and geometry import all walk these instances.
pub(crate) fn fbx_mesh_instances(scene: &ufbx::Scene) -> impl Iterator<Item = (&ufbx::Node, &ufbx::Mesh)> {
	(&scene.nodes).into_iter().filter_map(|node| {
		let mesh = node.mesh.as_deref()?;
		(mesh.num_indices != 0 && mesh.num_faces != 0 && mesh.num_triangles != 0).then_some((node, mesh))
	})
}

/// Estimates worst-case sizes for the reusable triangulation scratch, corner, and corner-remap buffers, in that order,
/// from ufbx metadata.
pub(crate) fn fbx_mesh_allocation_estimates(scene: &ufbx::Scene) -> (usize, usize, usize) {
	let (mut scratch, mut corners, mut remap) = (3, 0, 0);

	for (_, mesh) in fbx_mesh_instances(scene) {
		scratch = scratch.max(mesh.max_face_triangles.saturating_mul(3));

		remap = remap.max(mesh.num_indices);

		let mesh_corners = if mesh.material_parts.is_empty() {
			mesh.num_triangles.saturating_mul(3)
		} else {
			mesh.material_parts
				.iter()
				.map(|part| part.num_triangles.saturating_mul(3))
				.max()
				.unwrap_or(0)
		};
		corners = corners.max(mesh_corners);
	}

	(scratch, corners, remap)
}

/// Builds the final engine vertex layout once from the FBX meshes that can contribute primitives.
///
/// Any contributing mesh adds positions and UVs, with zero UVs for meshes that store none. Every other stream is
/// present when any mesh stores it.
pub(crate) fn fbx_vertex_layout(scene: &ufbx::Scene) -> Vec<VertexComponent> {
	let mut any_mesh = false;
	let mut normal = false;
	let mut tangent = false;
	let mut bitangent = false;
	let mut color = false;
	let mut skinned = false;
	for (_, mesh) in fbx_mesh_instances(scene) {
		any_mesh = true;
		normal |= mesh.vertex_normal.exists;
		tangent |= mesh.vertex_tangent.exists;
		bitangent |= mesh.vertex_bitangent.exists;
		color |= mesh.vertex_color.exists;
		skinned |= !mesh.skin_deformers.is_empty();
	}

	[
		(VertexSemantics::Position, any_mesh),
		(VertexSemantics::Normal, normal),
		(VertexSemantics::Tangent, tangent),
		(VertexSemantics::BiTangent, bitangent),
		(VertexSemantics::UV, any_mesh),
		(VertexSemantics::Color, color),
		(VertexSemantics::Joints, skinned),
		(VertexSemantics::Weights, skinned),
	]
	.into_iter()
	.filter_map(|(semantic, present)| present.then(|| VertexComponent::canonical(semantic)))
	.collect()
}

/// Streams FBX primitives into a session that can write its final blocks directly to resource storage.
///
/// Each primitive names its material by its key's position in `material_keys`, the list [`used_material_keys`]
/// returns, so the geometry can be processed while those materials still bake.
pub(crate) fn import_fbx_mesh_session<'a>(
	scene: &ufbx::Scene,
	material_keys: &[MaterialKey],
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
) -> Result<MeshProcessorSession, FbxMeshProcessingError> {
	let (scratch_capacity, corner_capacity, remap_capacity) = fbx_mesh_allocation_estimates(scene);
	let mut processor = MeshProcessor::new().begin(fbx_vertex_layout(scene), skeleton, Vec::new())?;
	let mut primitive_count = 0usize;

	// Reuse triangulation and corner-remap storage across mesh instances and material parts to bound import allocations.
	let mut scratch = Vec::with_capacity_in(scratch_capacity, allocator);

	let mut corners = Vec::with_capacity_in(corner_capacity, allocator);

	let mut remap = Vec::with_capacity_in(remap_capacity, allocator);

	for (node, mesh) in fbx_mesh_instances(scene) {
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

		scratch.resize(mesh.max_face_triangles.saturating_mul(3).max(3), 0u32);

		remap.clear();

		remap.resize(mesh.num_indices, u32::MAX);

		// Triangulates the visible faces of one material part and processes them before the next part reuses the
		// corner storage.
		let mut import_part = |part: usize, faces: &mut dyn Iterator<Item = usize>, triangles: usize| {
			corners.clear();

			corners.reserve(triangles.saturating_mul(3));

			for face_index in faces {
				let face = mesh.faces.get(face_index).copied().ok_or(FbxImportError::InvalidFaceIndex)?;

				if is_visible_polygon_face(mesh, face_index)
					&& append_triangulated_face(mesh, face, &mut scratch, &mut corners)?
				{
					culled_polygons.record(face.num_indices);
				}
			}

			primitive_count +=
				import_fbx_material_corners(&context, part, &corners, &mut remap, material_keys, &mut processor, allocator)?;

			Ok::<_, FbxMeshProcessingError>(())
		};

		if mesh.material_parts.is_empty() {
			import_part(0, &mut (0..mesh.faces.len()), mesh.num_triangles)?;
		} else {
			for part in &mesh.material_parts {
				let faces = &mut part.face_indices.iter().map(|&face_index| face_index as usize);
				import_part(part.index as usize, faces, part.num_triangles)?;
			}
		}
	}

	if primitive_count == 0 {
		return Err(FbxImportError::NoMesh.into());
	}
	Ok(processor)
}

/// Processes one triangulated material part immediately so source-corner storage can be reused by the next part.
///
/// `part` is the part's material index within its FBX mesh, and `material_keys` lists the materials the whole mesh
/// uses, in the order the finished mesh receives them.
pub(crate) fn import_fbx_material_corners<'a>(
	context: &FbxMeshImportContext<'_>,
	part: usize,
	corners: &[u32],
	remap: &mut [u32],
	material_keys: &[MaterialKey],
	processor: &mut MeshProcessorSession,
	allocator: &'a dyn Allocator,
) -> Result<usize, FbxMeshProcessingError> {
	if corners.is_empty() {
		return Ok(0);
	}

	let key = material_key_for_slot(context.material_node, context.mesh, part);
	let material_slot = material_keys
		.iter()
		.position(|used| *used == key)
		.ok_or(FbxImportError::MissingMaterial)?;
	let mut processed = 0;
	for batch in remap_triangle_corners(context.mesh.num_indices, corners, remap, allocator)? {
		if batch.source_corners.is_empty() {
			return Err(FbxImportError::EmptyPrimitive.into());
		}
		let source = FbxPrimitiveSource {
			context,
			material_slot,
			source_corners: &batch.source_corners,
			indices: &batch.indices,
		};
		processor.push_primitive(&source)?;
		processed += 1;
	}
	Ok(processed)
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
///
/// Returns `true` when the face was culled as degenerate instead of appended.
pub(crate) fn append_triangulated_face<A: Allocator>(
	mesh: &ufbx::Mesh,
	face: ufbx::Face,
	scratch: &mut [u32],
	corners: &mut Vec<u32, A>,
) -> Result<bool, FbxImportError> {
	let triangle_count = mesh.triangulate_face(scratch, face) as usize;

	let index_count = triangle_count.saturating_mul(3);

	if index_count > scratch.len() {
		return Err(FbxImportError::TriangulationOverflow);
	}

	let triangles = &scratch[..index_count];

	// Retained triangles may share malformed corner normals with a degenerate sibling, so discard the source polygon as a unit.
	for triangle in triangles.as_chunks::<3>().0 {
		if is_degenerate_fbx_triangle(mesh, triangle)? {
			return Ok(true);
		}
	}

	corners.extend_from_slice(triangles);

	Ok(false)
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
	let [a, b, c] = positions.map(|position| [position.x, position.y, position.z]);

	let first_edge: [f64; 3] = std::array::from_fn(|axis| b[axis] - a[axis]);

	let second_edge: [f64; 3] = std::array::from_fn(|axis| c[axis] - a[axis]);

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

	for triangle in corners.as_chunks::<3>().0 {
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
	let (mut joints, mut weights, total) = strongest_skin_influences(skin, logical_vertex)?;

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
pub(crate) fn normalized_direction(matrix: &ufbx::Matrix, direction: ufbx::Vec3) -> Result<Vector<ModelSpace>, FbxImportError> {
	let direction = ufbx::transform_direction(matrix, direction);

	normalize_direction(Vector::from_array(vec3_to_f32(direction, "mesh direction")?))
}

/// Removes the normal component from a transformed tangent-space direction and normalizes the result.
pub(crate) fn orthogonalized_direction(
	direction: Vector<ModelSpace>,
	normal: Vector<ModelSpace>,
) -> Result<Vector<ModelSpace>, FbxImportError> {
	normalize_direction(direction - normal * direction.dot(normal))
}

/// Normalizes an imported vector without allowing zero-length or non-finite shading data.
fn normalize_direction(direction: Vector<ModelSpace>) -> Result<Vector<ModelSpace>, FbxImportError> {
	direction
		.normalized()
		.map(UnitVector::into_vector)
		.map_err(|_| FbxImportError::ZeroDirection)
}

/// Computes tangent-space orientation after the node's geometry transform has been applied.
pub(crate) fn tangent_handedness(
	normal: Vector<ModelSpace>,
	tangent: Vector<ModelSpace>,
	bitangent: Vector<ModelSpace>,
) -> f32 {
	let alignment = normal.cross(tangent).dot(bitangent);
	if alignment < 0.0 { -1.0 } else { 1.0 }
}

/// Converts ufbx's double-precision vectors to the engine's finite single-precision representation.
pub(crate) fn vec3_to_f32(value: ufbx::Vec3, context: &'static str) -> Result<[f32; 3], FbxImportError> {
	Ok([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
	])
}

/// Converts ufbx's x/y/z/w quaternion layout to a normalized orientation.
pub(crate) fn quat_to_orientation(value: ufbx::Quat, context: &'static str) -> Result<Orientation, FbxImportError> {
	Orientation::try_from_array([
		finite_f32(value.x, context)?,
		finite_f32(value.y, context)?,
		finite_f32(value.z, context)?,
		finite_f32(value.w, context)?,
	])
	.map_err(|_| FbxImportError::ZeroRotation(context))
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
