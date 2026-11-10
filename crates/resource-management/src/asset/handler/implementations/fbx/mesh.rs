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

/// The `FbxSegment` struct lends one FBX mesh's remapped corners to a primitive: its full detail, or one authored
/// coarser level read from another file.
pub(crate) struct FbxSegment<'context, 'scene, 'batch> {
	context: &'context FbxMeshImportContext<'scene>,
	source_corners: &'batch [u32],
	indices: &'batch [u32],
}

impl FbxSegment<'_, '_, '_> {
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

	/// Yields the segment's triangle indices, with each triangle's winding flipped for a mirrored instance.
	fn triangle_indices(&self) -> impl Iterator<Item = Result<u32, FbxImportError>> + '_ {
		(0..self.indices.len()).map(|index| {
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
		})
	}
}

/// The `FbxPrimitiveSource` struct lends remapped FBX corners to the common processor without materializing attributes.
///
/// Its first segment is full detail. Each later one is an authored coarser level, whose vertices follow the ones
/// before it, so its indices shift past them.
pub(crate) struct FbxPrimitiveSource<'context, 'scene, 'batch> {
	/// The index of the primitive's material in the mesh's list of used materials.
	material_slot: usize,
	segments: &'batch [FbxSegment<'context, 'scene, 'batch>],
	/// Where each segment after the first starts in the primitive's indices.
	level_starts: &'batch [usize],
}

impl FbxPrimitiveSource<'_, '_, '_> {
	/// The first segment, whose mesh decides which streams the primitive has.
	fn full_detail(&self) -> &FbxSegment<'_, '_, '_> {
		&self.segments[0]
	}

	/// Reads one value per vertex, segment after segment, with `read` given each vertex's segment and corner.
	fn per_vertex<'a, T: 'a>(
		&'a self,
		read: fn(&FbxSegment<'_, '_, '_>, usize) -> Result<T, FbxImportError>,
	) -> ExactLength<impl Iterator<Item = Result<T, FbxImportError>> + 'a> {
		let length = self.segments.iter().map(|segment| segment.source_corners.len()).sum();
		let values = self.segments.iter().flat_map(move |segment| {
			segment
				.source_corners
				.iter()
				.map(move |&source_corner| read(segment, segment.corner(source_corner)?))
		});
		ExactLength::new(values, length)
	}
}

impl MeshPrimitiveSource for FbxPrimitiveSource<'_, '_, '_> {
	type Error = FbxImportError;

	fn material_slot(&self) -> usize {
		self.material_slot
	}

	fn transform_node(&self) -> Option<u32> {
		self.full_detail().context.transform_node
	}

	fn skin(&self) -> Option<u32> {
		self.full_detail().context.skin_index
	}

	fn coarser_level_starts(&self) -> &[usize] {
		self.level_starts
	}

	fn indices(&self) -> Result<impl ExactSizeIterator<Item = Result<u32, Self::Error>> + '_, Self::Error> {
		let length = self.segments.iter().map(|segment| segment.indices.len()).sum();
		let indices = self
			.segments
			.iter()
			.scan(0u32, |first_vertex, segment| {
				let base = *first_vertex;
				*first_vertex += segment.source_corners.len() as u32;
				Some((base, segment))
			})
			.flat_map(|(base, segment)| segment.triangle_indices().map(move |index| index.map(|index| index + base)));
		Ok(ExactLength::new(indices, length))
	}

	fn positions(&self) -> Result<impl ExactSizeIterator<Item = Result<Point<ModelSpace>, Self::Error>> + '_, Self::Error> {
		Ok(self.per_vertex(|segment, corner| {
			let position = ufbx::transform_position(
				&segment.context.node.geometry_to_world,
				segment.context.mesh.vertex_position[corner],
			);
			vec3_to_f32(position, "mesh position").map(Point::from_array)
		}))
	}

	fn normals(
		&self,
	) -> Result<Option<impl ExactSizeIterator<Item = Result<Vector<ModelSpace>, Self::Error>> + '_>, Self::Error> {
		if self.full_detail().context.normal_matrix.is_none() {
			return Ok(None);
		}
		Ok(Some(self.per_vertex(|segment, corner| {
			segment.normal(corner)?.ok_or(FbxImportError::ZeroDirection)
		})))
	}

	fn tangents(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.full_detail().context.mesh.vertex_tangent.exists {
			return Ok(None);
		}
		Ok(Some(self.per_vertex(|segment, corner| {
			let [normal, tangent, bitangent] = segment.tangent_frame(corner)?;
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
		if !self.full_detail().context.mesh.vertex_bitangent.exists {
			return Ok(None);
		}
		Ok(Some(self.per_vertex(|segment, corner| match segment.tangent_frame(corner)? {
			[Some(normal), Some(tangent), Some(bitangent)] => {
				let handedness = tangent_handedness(normal, tangent, bitangent);
				Ok(normal.cross(tangent) * handedness)
			}
			[Some(normal), None, Some(bitangent)] => orthogonalized_direction(bitangent, normal),
			[_, _, Some(bitangent)] => Ok(bitangent),
			[_, _, None] => Err(FbxImportError::ZeroDirection),
		})))
	}

	fn uvs(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 2], Self::Error>> + '_>, Self::Error> {
		Ok(Some(self.per_vertex(|segment, corner| {
			if segment.context.mesh.vertex_uv.exists {
				let uv = segment.context.mesh.vertex_uv[corner];
				Ok([finite_f32(uv.x, "mesh UV")?, 1.0 - finite_f32(uv.y, "mesh UV")?])
			} else {
				Ok([0.0, 0.0])
			}
		})))
	}

	fn colors(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<[f32; 4], Self::Error>> + '_>, Self::Error> {
		if !self.full_detail().context.mesh.vertex_color.exists {
			return Ok(None);
		}
		Ok(Some(self.per_vertex(|segment, corner| {
			let color = segment.context.mesh.vertex_color[corner];
			Ok([
				finite_f32(color.x, "mesh color")?,
				finite_f32(color.y, "mesh color")?,
				finite_f32(color.z, "mesh color")?,
				finite_f32(color.w, "mesh color")?,
			])
		})))
	}

	fn vertex_skin(&self) -> Result<Option<impl ExactSizeIterator<Item = Result<VertexSkin, Self::Error>> + '_>, Self::Error> {
		if self.full_detail().context.skin.is_none() {
			return Ok(None);
		}
		Ok(Some(self.per_vertex(|segment, corner| {
			let skin = segment.context.skin.ok_or(FbxImportError::InvalidCornerIndex)?;
			let logical_vertex = *segment
				.context
				.mesh
				.vertex_indices
				.get(corner)
				.ok_or(FbxImportError::InvalidCornerIndex)? as usize;
			let (joints, weights) = skin_weights(skin, logical_vertex, segment.context.fallback_joint)?;
			Ok(VertexSkin { joints, weights })
		})))
	}
}

/// The `ExactLength` struct gives an iterator whose length is known in advance, such as one that chains a primitive's
/// segments, the exact size the mesh processor requires.
pub(crate) struct ExactLength<I> {
	inner: I,
	remaining: usize,
}

impl<I> ExactLength<I> {
	fn new(inner: I, length: usize) -> Self {
		Self {
			inner,
			remaining: length,
		}
	}
}

impl<I: Iterator> Iterator for ExactLength<I> {
	type Item = I::Item;

	fn next(&mut self) -> Option<Self::Item> {
		let item = self.inner.next()?;
		self.remaining = self.remaining.saturating_sub(1);
		Some(item)
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		(self.remaining, Some(self.remaining))
	}
}

impl<I: Iterator> ExactSizeIterator for ExactLength<I> {}

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
	.filter(|(_, present)| *present)
	.map(|(semantic, _)| VertexComponent::canonical(semantic))
	.collect()
}

/// The `FbxStage` struct is one set of primitives of a model: the model's own, or those of an authored level file
/// that uses other materials, which take the place of the set before them from a distance on.
///
/// Split a model's level files into stages with [`authored_stage_starts`].
pub(crate) struct FbxStage<'s, 'k> {
	/// The model's ID, or the level file's.
	pub(crate) id: &'s str,
	/// The model's scene, or the level file's.
	pub(crate) scene: &'s ufbx::Scene,
	/// The materials the stage's scene uses, which [`used_material_keys`] returns.
	pub(crate) material_keys: &'k [MaterialKey],
	/// The level files that follow the stage's scene, finest first, up to the next stage's.
	pub(crate) levels: &'k [(&'s str, &'s ufbx::Scene)],
}

/// Returns the index in `levels` of each level file that starts a stage of its own: one that uses other materials than
/// the stage before it, holds one mesh instance, and stores the model's vertex streams.
///
/// A file that can't follow the model at all ends the search, as it ends the model's authored levels.
pub(crate) fn authored_stage_starts(
	scene: &ufbx::Scene,
	levels: &[(&str, &ufbx::Scene)],
) -> Result<Vec<usize>, FbxImportError> {
	let mut starts = Vec::new();
	let instances = fbx_mesh_instances(scene).count();
	let Some((node, mesh)) = fbx_mesh_instances(scene).next().filter(|_| instances == 1) else {
		return Ok(starts);
	};
	let base = FbxMeshImportContext::new(node, mesh, None, None, None, None)?;
	let mut names = visible_part_names(&base);
	for (index, (_, level_scene)) in levels.iter().enumerate() {
		let level_instances = fbx_mesh_instances(level_scene).count();
		let Some((node, level_mesh)) = fbx_mesh_instances(level_scene).next().filter(|_| level_instances == 1) else {
			break;
		};
		if vertex_streams(level_mesh) != vertex_streams(mesh) {
			break;
		}
		let level_names = visible_part_names(&FbxMeshImportContext::new(node, level_mesh, None, None, None, None)?);
		if level_names != names {
			starts.push(index);
			names = level_names;
		}
	}
	Ok(starts)
}

/// Returns the ID and scene of each stage: the model `scene` named `id`, then each of `levels` that `starts` lists.
pub(crate) fn fbx_stage_scenes<'s>(
	id: &'s str,
	scene: &'s ufbx::Scene,
	levels: &[(&'s str, &'s ufbx::Scene)],
	starts: &[usize],
) -> Vec<(&'s str, &'s ufbx::Scene)> {
	std::iter::once((id, scene))
		.chain(starts.iter().map(|&start| levels[start]))
		.collect()
}

/// Lays out the stages of a model: each of `scenes`, which [`fbx_stage_scenes`] returns, with its entry of `keys`,
/// the materials [`used_material_keys`] returns for it, and the level files that follow it up to the next of `starts`.
pub(crate) fn fbx_stages<'s, 'k, A: Allocator>(
	scenes: &[(&'s str, &'s ufbx::Scene)],
	levels: &'k [(&'s str, &'s ufbx::Scene)],
	starts: &[usize],
	keys: &'k [Vec<MaterialKey, A>],
) -> Vec<FbxStage<'s, 'k>> {
	let firsts = std::iter::once(0).chain(starts.iter().map(|&start| start + 1));
	let ends = starts.iter().copied().chain([levels.len()]);
	scenes
		.iter()
		.zip(keys)
		.zip(firsts.zip(ends))
		.map(|((&(id, scene), material_keys), (first, end))| FbxStage {
			id,
			scene,
			material_keys,
			levels: &levels[first..end],
		})
		.collect()
}

/// Streams FBX primitives into a session that can write its final blocks directly to resource storage.
///
/// `stages` holds the model, then each level file that starts a stage, as [`fbx_stages`] lays them out. Each
/// primitive names its material by its key's position among the keys of every stage, in stage order, so the geometry
/// can be processed while those materials still bake. Each stage after the first replaces the primitives of the one
/// before it.
pub(crate) fn import_fbx_mesh_session<'a>(
	stages: &[FbxStage<'_, '_>],
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
	settings: MeshSettings,
	notes: &mut FbxAuthoredLevelNotes,
) -> Result<MeshProcessorSession, FbxMeshProcessingError> {
	let model = stages[0].scene;
	let mut processor = MeshProcessorSession::new(fbx_vertex_layout(model), skeleton, Vec::new(), settings)?;
	let mut material_base = 0;
	let mut previous = 0..0;
	for (index, stage) in stages.iter().enumerate() {
		let first = processor.primitive_count();
		// A level file has no skeleton of its own: authored levels follow rigid models only.
		let source_to_skeleton = if index == 0 { source_to_skeleton } else { &[] };
		import_fbx_stage(
			stage,
			material_base,
			source_to_skeleton,
			allocator,
			culled_polygons,
			notes,
			&mut processor,
		)?;
		let primitives = first..processor.primitive_count();
		if index > 0 {
			notes.info.push(format!(
				"Authored level '{}' uses other materials than the level before it, so its {} primitives take over from the {} before them.",
				stage.id,
				primitives.len(),
				previous.len()
			));
			processor.replace_primitives(previous, primitives.clone());
		}
		previous = primitives;
		material_base += stage.material_keys.len();
	}
	Ok(processor)
}

/// Streams the primitives of one stage into `processor`, each with the matching parts of the stage's level files as
/// its authored coarser levels. Its material slots start at `material_base`.
fn import_fbx_stage<'a>(
	stage: &FbxStage<'_, '_>,
	material_base: usize,
	source_to_skeleton: &[u32],
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
	notes: &mut FbxAuthoredLevelNotes,
	mut processor: &mut MeshProcessorSession,
) -> Result<(), FbxMeshProcessingError> {
	let (scene, material_keys) = (stage.scene, stage.material_keys);
	let chain = authored_level_chain(scene, stage.levels, allocator, notes)?;
	let (scratch_capacity, corner_capacity, remap_capacity) = fbx_mesh_allocation_estimates(scene);
	let mut primitive_count = 0usize;

	// Reuse triangulation and corner-remap storage across mesh instances and material parts to bound import allocations.
	let mut scratch = Vec::with_capacity_in(scratch_capacity, allocator);

	let mut corners = Vec::with_capacity_in(corner_capacity, allocator);

	let mut remap = Vec::with_capacity_in(remap_capacity, allocator);

	let mut remapped = RemappedCorners::with_capacity_in(corner_capacity, allocator);

	let mut weld = Vec::with_capacity_in(remap_capacity, allocator);

	let mut weld_table = Vec::with_capacity_in(weld_table_len(remap_capacity), allocator);

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

		weld_fbx_corners(mesh, skin.is_some(), &mut weld_table, &mut weld)?;

		// Triangulates the visible faces of one material part and processes them before the next part reuses the
		// corner storage.
		for_each_material_part(mesh, |part, faces, triangles| {
			corners.clear();

			corners.reserve(triangles.saturating_mul(3));

			append_welded_corners(mesh, faces, &weld, &mut scratch, &mut corners, culled_polygons)?;

			primitive_count += import_fbx_material_corners(
				&context,
				part,
				&corners,
				&mut remap,
				&mut remapped,
				material_keys,
				material_base,
				&chain,
				notes,
				&mut processor,
			)?;

			Ok::<_, FbxMeshProcessingError>(())
		})?;
	}

	if primitive_count == 0 {
		return Err(FbxImportError::NoMesh.into());
	}
	Ok(())
}

/// Processes one triangulated material part immediately so source-corner storage can be reused by the next part.
///
/// `part` is the part's material index within its FBX mesh, and `material_keys` lists the materials the whole mesh
/// uses, which the finished mesh receives in that order from slot `material_base` on. A part that fits in one
/// primitive takes the matching part of
/// each file in `chain` as its authored coarser levels, as long as they fit beside it.
pub(crate) fn import_fbx_material_corners(
	context: &FbxMeshImportContext<'_>,
	part: usize,
	corners: &[u32],
	remap: &mut [u32],
	remapped: &mut RemappedCorners<'_>,
	material_keys: &[MaterialKey],
	material_base: usize,
	chain: &[FbxLevelFile<'_, '_>],
	notes: &mut FbxAuthoredLevelNotes,
	processor: &mut MeshProcessorSession,
) -> Result<usize, FbxMeshProcessingError> {
	if corners.is_empty() {
		return Ok(0);
	}

	let key = material_key_for_slot(context.material_node, context.mesh, part);
	let material_slot = material_base
		+ material_keys
			.iter()
			.position(|used| *used == key)
			.ok_or(FbxImportError::MissingMaterial)?;
	remap_triangle_corners(context.mesh.num_indices, corners, remap, remapped)?;
	let primitive_count = remapped.ends.len();
	let name = || part_material_name(context.material_node, context.mesh, part);
	if primitive_count == 1 && !chain.is_empty() {
		let (source_corners, indices) = remapped.primitives().next().ok_or(FbxImportError::EmptyPrimitive)?;
		let full_detail = FbxSegment {
			context,
			source_corners,
			indices,
		};
		push_with_authored_levels(full_detail, material_slot, &name(), chain, notes, processor)?;
		return Ok(1);
	}
	if !chain.is_empty() {
		notes.warnings.push(format!(
			"Material '{}' splits into {primitive_count} primitives, but authored levels follow a material that fits in one, so the bake generates its levels instead.",
			name()
		));
	}
	for (source_corners, indices) in remapped.primitives() {
		if source_corners.is_empty() {
			return Err(FbxImportError::EmptyPrimitive.into());
		}
		let segment = FbxSegment {
			context,
			source_corners,
			indices,
		};
		processor.push_primitive(&FbxPrimitiveSource {
			material_slot,
			segments: std::slice::from_ref(&segment),
			level_starts: &[],
		})?;
	}
	Ok(primitive_count)
}

/// Pushes one primitive of `full_detail` followed by the part named `name` of each file in `chain`, as long as the
/// levels fit beside it in one primitive's vertices.
fn push_with_authored_levels(
	full_detail: FbxSegment<'_, '_, '_>,
	material_slot: usize,
	name: &str,
	chain: &[FbxLevelFile<'_, '_>],
	notes: &mut FbxAuthoredLevelNotes,
	processor: &mut MeshProcessorSession,
) -> Result<(), FbxMeshProcessingError> {
	let allocator = *chain[0].parts.allocator();
	// Each level's corners remapped into one primitive's vertices, kept until the primitive is pushed.
	let mut levels = Vec::with_capacity_in(chain.len(), allocator);
	let mut vertices = full_detail.source_corners.len();
	for file in chain {
		let level_corners = file
			.parts
			.iter()
			.find(|(part_name, _)| *part_name == name)
			.map(|(_, corners)| &corners[..])
			.ok_or(FbxImportError::MissingMaterial)?;
		let mut level_remap = Vec::with_capacity_in(file.context.mesh.num_indices, allocator);
		level_remap.resize(file.context.mesh.num_indices, u32::MAX);
		let mut level_remapped = RemappedCorners::with_capacity_in(level_corners.len(), allocator);
		remap_triangle_corners(file.context.mesh.num_indices, level_corners, &mut level_remap, &mut level_remapped)?;
		if level_remapped.ends.is_empty() {
			notes.info.push(format!(
				"Authored level '{}' has no triangles left for material '{name}' once its degenerate polygons are culled, so the authored levels end before it.",
				file.id
			));
			break;
		}
		let added = level_remapped.source_corners.len();
		if level_remapped.ends.len() != 1 || vertices + added > MAX_PRIMITIVE_VERTICES {
			notes.warnings.push(format!(
				"Authored level '{}' adds {added} vertices to the {vertices} of material '{name}', more than the 65,536 a primitive holds, so the bake generates that level and the coarser ones instead.",
				file.id
			));
			break;
		}
		vertices += added;
		levels.push(level_remapped);
	}
	let mut index_count = full_detail.indices.len();
	let mut segments = Vec::with_capacity_in(1 + levels.len(), allocator);
	segments.push(full_detail);
	let mut level_starts = Vec::with_capacity_in(levels.len(), allocator);
	for (file, level) in chain.iter().zip(&levels) {
		let (source_corners, indices) = level.primitives().next().ok_or(FbxImportError::EmptyPrimitive)?;
		level_starts.push(index_count);
		index_count += indices.len();
		segments.push(FbxSegment {
			context: &file.context,
			source_corners,
			indices,
		});
	}
	processor.push_primitive(&FbxPrimitiveSource {
		material_slot,
		segments: &segments,
		level_starts: &level_starts,
	})?;
	Ok(())
}

/// The `FbxAuthoredLevelNotes` struct collects what a mesh import decided about the model's authored levels, so the
/// bake can report it in the model's trace.
#[derive(Default)]
pub(crate) struct FbxAuthoredLevelNotes {
	/// Levels left out that the author likely meant to use.
	pub(crate) warnings: Vec<String>,
	/// Where and why the chain of level files ended.
	pub(crate) info: Vec<String>,
}

/// The `FbxLevelFile` struct holds one authored level file's triangles, welded and grouped by material name, so each
/// of the model's primitives can take its part as a coarser level.
pub(crate) struct FbxLevelFile<'scene, 'a> {
	id: &'scene str,
	context: FbxMeshImportContext<'scene>,
	/// Each material part's name and welded triangle corners.
	parts: Vec<(String, Vec<u32, &'a dyn Allocator>), &'a dyn Allocator>,
}

/// Returns the level files of `levels`, finest first, that can follow the model in `scene`, ending before the first
/// that can't: one holding another number of mesh instances than one, other materials, or other vertex streams.
fn authored_level_chain<'scene, 'a>(
	scene: &'scene ufbx::Scene,
	levels: &[(&'scene str, &'scene ufbx::Scene)],
	allocator: &'a dyn Allocator,
	notes: &mut FbxAuthoredLevelNotes,
) -> Result<Vec<FbxLevelFile<'scene, 'a>, &'a dyn Allocator>, FbxImportError> {
	let mut chain = Vec::new_in(allocator);
	let Some(&(first_id, _)) = levels.first() else {
		return Ok(chain);
	};
	let instances = fbx_mesh_instances(scene).count();
	let Some((node, mesh)) = fbx_mesh_instances(scene).next().filter(|_| instances == 1) else {
		notes.warnings.push(format!(
			"The model holds {instances} mesh instances, but authored levels such as '{first_id}' follow a model that holds one, so the bake generates its levels instead."
		));
		return Ok(chain);
	};
	let base = FbxMeshImportContext::new(node, mesh, None, None, None, None)?;
	let base_names = visible_part_names(&base);
	for &(id, level_scene) in levels {
		let level_instances = fbx_mesh_instances(level_scene).count();
		let Some((node, mesh)) = fbx_mesh_instances(level_scene).next().filter(|_| level_instances == 1) else {
			notes.info.push(format!(
				"Authored level '{id}' holds {level_instances} mesh instances instead of one, so the authored levels end before it."
			));
			break;
		};
		let context = FbxMeshImportContext::new(node, mesh, None, None, None, None)?;
		let names = visible_part_names(&context);
		if names != base_names {
			notes.info.push(format!(
				"Authored level '{id}' uses the materials {names:?} instead of the model's {base_names:?}, so the authored levels end before it."
			));
			break;
		}
		if vertex_streams(mesh) != vertex_streams(base.mesh) {
			notes.info.push(format!(
				"Authored level '{id}' stores other vertex streams than the model, so the authored levels end before it."
			));
			break;
		}
		let parts = welded_material_parts(&context, allocator)?;
		chain.push(FbxLevelFile { id, context, parts });
	}
	Ok(chain)
}

/// Triangulates and welds each material part of the context's mesh that has a visible face, keyed by material name.
fn welded_material_parts<'a>(
	context: &FbxMeshImportContext<'_>,
	allocator: &'a dyn Allocator,
) -> Result<Vec<(String, Vec<u32, &'a dyn Allocator>), &'a dyn Allocator>, FbxImportError> {
	let mesh = context.mesh;
	let mut weld = Vec::new_in(allocator);
	weld_fbx_corners(mesh, false, &mut Vec::new_in(allocator), &mut weld)?;
	let mut scratch = Vec::new_in(allocator);
	scratch.resize(mesh.max_face_triangles.saturating_mul(3).max(3), 0u32);
	// A level's culled polygons go unreported; the model's own count covers what its author sees.
	let mut culled = FbxCulledPolygonCounts::default();
	let mut parts = Vec::new_in(allocator);
	for_each_material_part(mesh, |part, faces, triangles| {
		let mut faces = faces.filter(|&face| is_visible_polygon_face(mesh, face)).peekable();
		if faces.peek().is_some() {
			let mut corners = Vec::with_capacity_in(triangles.saturating_mul(3), allocator);
			append_welded_corners(mesh, &mut faces, &weld, &mut scratch, &mut corners, &mut culled)?;
			parts.push((part_material_name(context.material_node, mesh, part), corners));
		}
		Ok::<_, FbxImportError>(())
	})?;
	Ok(parts)
}

/// Returns the sorted names of the materials of the context's mesh that have a visible face.
fn visible_part_names(context: &FbxMeshImportContext<'_>) -> Vec<String> {
	let mesh = context.mesh;
	let mut names = Vec::new();
	let _ = for_each_material_part(mesh, |part, mut faces, _| {
		if Iterator::any(&mut faces, |face| is_visible_polygon_face(mesh, face)) {
			names.push(part_material_name(context.material_node, mesh, part));
		}
		Ok::<_, ()>(())
	});
	names.sort();
	names.dedup();
	names
}

/// Calls `visit` with each material part's index, faces, and triangle count. A mesh without parts is one part.
fn for_each_material_part<E>(
	mesh: &ufbx::Mesh,
	mut visit: impl FnMut(usize, &mut dyn Iterator<Item = usize>, usize) -> Result<(), E>,
) -> Result<(), E> {
	if mesh.material_parts.is_empty() {
		return visit(0, &mut (0..mesh.faces.len()), mesh.num_triangles);
	}
	for part in &mesh.material_parts {
		visit(
			part.index as usize,
			&mut part.face_indices.iter().map(|&face| face as usize),
			part.num_triangles,
		)?;
	}
	Ok(())
}

/// Triangulates the visible faces among `faces` into `corners`, each corner replaced by its weld representative so
/// corners that repeat a vertex become one, and records the polygons culled as degenerate.
fn append_welded_corners<A: Allocator>(
	mesh: &ufbx::Mesh,
	faces: &mut dyn Iterator<Item = usize>,
	weld: &[u32],
	scratch: &mut [u32],
	corners: &mut Vec<u32, A>,
	culled: &mut FbxCulledPolygonCounts,
) -> Result<(), FbxImportError> {
	let start = corners.len();
	for face_index in faces {
		let face = mesh.faces.get(face_index).copied().ok_or(FbxImportError::InvalidFaceIndex)?;
		if is_visible_polygon_face(mesh, face_index) && append_triangulated_face(mesh, face, scratch, corners)? {
			culled.record(face.num_indices);
		}
	}
	for corner in &mut corners[start..] {
		*corner = weld[*corner as usize];
	}
	Ok(())
}

/// Returns which optional vertex streams a mesh stores: normals, tangents, bitangents, colors, and UVs.
fn vertex_streams(mesh: &ufbx::Mesh) -> [bool; 5] {
	[
		mesh.vertex_normal.exists,
		mesh.vertex_tangent.exists,
		mesh.vertex_bitangent.exists,
		mesh.vertex_color.exists,
		mesh.vertex_uv.exists,
	]
}

/// Returns the name of the material a part uses, or `default` when it has none, the key a BEAD file binds it by.
fn part_material_name(material_node: &ufbx::Node, mesh: &ufbx::Mesh, slot: usize) -> String {
	material_node
		.materials
		.as_ref()
		.get(slot)
		.or_else(|| mesh.materials.as_ref().get(slot))
		.map_or_else(|| "default".to_string(), |material| material.element.name.to_string())
}

/// Maps every corner of `mesh` to the first corner whose vertex data is bitwise equal to its own, into `weld`.
///
/// FBX stores vertex data per polygon corner, and a triangulated export often repeats a shared vertex's data in every
/// corner around it, sometimes without sharing attribute indices. Importing each corner as its own vertex turns such a
/// mesh into separate triangles, which wastes vertices and meshlets and leaves the simplifier no edges to collapse.
/// Corners are compared by the source values every vertex stream reads: position, normal, tangent, bitangent, UV, and
/// color, plus the control point when `skinned`, since skin weights belong to it. Exact equality keeps UV and normal
/// seams split. `table` is scratch for an open-addressing hash set of corners.
pub(crate) fn weld_fbx_corners<A: Allocator>(
	mesh: &ufbx::Mesh,
	skinned: bool,
	table: &mut Vec<u32, A>,
	weld: &mut Vec<u32, A>,
) -> Result<(), FbxImportError> {
	let corner_count = u32::try_from(mesh.num_indices).map_err(|_| FbxImportError::InvalidCornerIndex)?;
	table.clear();
	table.resize(weld_table_len(mesh.num_indices), u32::MAX);
	weld.clear();
	weld.reserve(mesh.num_indices);
	// The table holds at most half its slots, so linear probing ends quickly.
	let mask = table.len() - 1;

	for corner in 0..corner_count {
		let key = corner_weld_key(mesh, skinned, corner as usize);
		let mut slot = rapidhash::v3::rapidhash_v3(bytemuck::cast_slice(&key)) as usize & mask;
		let representative = loop {
			match table[slot] {
				u32::MAX => {
					table[slot] = corner;
					break corner;
				}
				other if corner_weld_key(mesh, skinned, other as usize) == key => break other,
				_ => slot = (slot + 1) & mask,
			}
		};
		weld.push(representative);
	}
	Ok(())
}

/// Returns the slot count of a weld table for meshes of up to `corners` corners: a power of two at least twice as
/// large, so probing stays short.
fn weld_table_len(corners: usize) -> usize {
	corners.saturating_mul(2).next_power_of_two().max(2)
}

/// Gathers the bits of every source value a corner's vertex streams read, so two corners with equal keys import as
/// equal vertices. Attributes the mesh lacks stay zero, which every corner shares.
fn corner_weld_key(mesh: &ufbx::Mesh, skinned: bool, corner: usize) -> [u64; 19] {
	let mut key = [0u64; 19];
	let mut write = |offset: usize, values: &[f64]| {
		for (slot, value) in key[offset..].iter_mut().zip(values) {
			*slot = value.to_bits();
		}
	};
	let position = mesh.vertex_position[corner];
	write(0, &[position.x, position.y, position.z]);
	for (offset, attribute) in [(3, &mesh.vertex_normal), (6, &mesh.vertex_tangent), (9, &mesh.vertex_bitangent)] {
		if attribute.exists {
			let value = attribute[corner];
			write(offset, &[value.x, value.y, value.z]);
		}
	}
	if mesh.vertex_uv.exists {
		let uv = mesh.vertex_uv[corner];
		write(12, &[uv.x, uv.y]);
	}
	if mesh.vertex_color.exists {
		let color = mesh.vertex_color[corner];
		write(14, &[color.x, color.y, color.z, color.w]);
	}
	if skinned {
		key[18] = u64::from(mesh.vertex_indices[corner]);
	}
	key
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

/// The `RemappedCorners` struct holds one material part split into u16-compatible primitives.
///
/// The import reuses one for every material part, so the bake arena does not grow with each part.
pub(crate) struct RemappedCorners<'a> {
	/// Every primitive's source corners, back to back.
	source_corners: Vec<u32, &'a dyn Allocator>,
	/// Every primitive's local triangle indices, back to back.
	indices: Vec<u32, &'a dyn Allocator>,
	/// Where each primitive ends in `source_corners` and `indices`.
	ends: Vec<(usize, usize), &'a dyn Allocator>,
}

impl<'a> RemappedCorners<'a> {
	/// Reserves room for material parts of up to `corners` triangle corners.
	pub(crate) fn with_capacity_in(corners: usize, allocator: &'a dyn Allocator) -> Self {
		Self {
			source_corners: Vec::with_capacity_in(corners, allocator),
			indices: Vec::with_capacity_in(corners, allocator),
			ends: Vec::new_in(allocator),
		}
	}

	/// Yields each primitive's source corners and local indices.
	pub(crate) fn primitives(&self) -> impl Iterator<Item = (&[u32], &[u32])> {
		let mut start = (0, 0);
		self.ends.iter().map(move |&end| {
			let primitive = (&self.source_corners[start.0..end.0], &self.indices[start.1..end.1]);
			start = end;
			primitive
		})
	}
}

/// Splits and remaps corner-indexed triangles into `remapped` so every processed primitive remains representable by
/// the engine's u16 index streams.
pub(crate) fn remap_triangle_corners(
	source_corner_count: usize,
	corners: &[u32],
	remap: &mut [u32],
	remapped: &mut RemappedCorners<'_>,
) -> Result<(), FbxImportError> {
	if !corners.len().is_multiple_of(3) {
		return Err(FbxImportError::InvalidTriangleCount);
	}

	if remap.len() != source_corner_count {
		return Err(FbxImportError::InvalidCornerIndex);
	}

	remapped.source_corners.clear();

	remapped.indices.clear();

	remapped.ends.clear();

	// Where the open primitive starts in `source_corners` and `indices`.
	let mut start = (0, 0);

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

		if remapped.indices.len() > start.1 && remapped.source_corners.len() - start.0 + new_corners > MAX_PRIMITIVE_VERTICES {
			for &corner in &remapped.source_corners[start.0..] {
				remap[corner as usize] = u32::MAX;
			}

			start = (remapped.source_corners.len(), remapped.indices.len());

			remapped.ends.push(start);
		}

		for &corner in triangle {
			let slot = &mut remap[corner as usize];

			if *slot == u32::MAX {
				*slot = (remapped.source_corners.len() - start.0) as u32;

				remapped.source_corners.push(corner);
			}

			remapped.indices.push(*slot);
		}
	}

	if remapped.indices.len() > start.1 {
		for &corner in &remapped.source_corners[start.0..] {
			remap[corner as usize] = u32::MAX;
		}

		remapped.ends.push((remapped.source_corners.len(), remapped.indices.len()));
	}

	Ok(())
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
