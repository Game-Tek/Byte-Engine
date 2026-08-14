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
	primitives: usize,
	scratch: usize,
	corners: usize,
	remap: usize,
}

/// Estimates common-case primitive count and worst-case reusable scratch sizes from ufbx metadata.
pub(crate) fn fbx_mesh_allocation_estimates(scene: &ufbx::Scene) -> FbxMeshAllocationEstimates {
	let mut estimates = FbxMeshAllocationEstimates {
		primitives: 0,
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
		let (mesh_corners, mesh_primitives) = if mesh.material_parts.is_empty() {
			let corners = mesh.num_triangles.saturating_mul(3);
			(corners, corners.div_ceil(MAX_PRIMITIVE_VERTICES).max(1))
		} else {
			let corners = mesh
				.material_parts
				.iter()
				.map(|part| part.num_triangles.saturating_mul(3))
				.max()
				.unwrap_or(0);
			let primitives = mesh.material_parts.iter().fold(0usize, |count, part| {
				count.saturating_add(part.num_triangles.saturating_mul(3).div_ceil(MAX_PRIMITIVE_VERTICES).max(1))
			});
			(corners, primitives)
		};
		estimates.primitives = estimates.primitives.saturating_add(mesh_primitives);
		estimates.corners = estimates.corners.max(mesh_corners);
	}
	estimates
}

/// Imports every mesh instance and material part into processor-owned, per-corner vertex data.
pub(crate) fn import_fbx_meshes<'a>(
	scene: &ufbx::Scene,
	materials: &ResolvedFbxMaterials,
	skeleton: Option<ReferenceModel<SkeletonModel>>,
	source_to_skeleton: &[u32],
	allocator: &'a dyn Allocator,
	culled_polygons: &mut FbxCulledPolygonCounts,
) -> Result<OwnedMeshSource<&'a dyn Allocator>, FbxImportError> {
	let estimates = fbx_mesh_allocation_estimates(scene);
	let mut layout = Vec::with_capacity_in(8, allocator);
	let mut layout_semantics = VertexAttributeMask::default();
	let mut primitives = Vec::with_capacity_in(estimates.primitives, allocator);
	let mut scratch = Vec::with_capacity_in(estimates.scratch, allocator);
	let mut corners = Vec::with_capacity_in(estimates.corners, allocator);
	let mut remap = Vec::with_capacity_in(estimates.remap, allocator);
	let skin_capacity = scene
		.nodes
		.iter()
		.filter_map(|node| node.mesh.as_ref())
		.filter(|mesh| !mesh.skin_deformers.is_empty())
		.count();
	let mut skins = Vec::with_capacity(skin_capacity);

	for node in &scene.nodes {
		let Some(mesh) = node.mesh.as_ref() else {
			continue;
		};
		if mesh.num_indices == 0 || mesh.num_faces == 0 || mesh.num_triangles == 0 {
			continue;
		}
		let skin = select_fbx_skin(mesh)?;
		let (skin_index, fallback_joint) = if let Some(skin) = skin {
			let skin_index = u32::try_from(skins.len()).map_err(|_| FbxImportError::TooManySkinBindings)?;
			let (binding, fallback_joint) = import_fbx_skin_binding(node, skin, source_to_skeleton)?;
			skins.push(binding);
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
			import_fbx_material_corners(
				&context,
				0,
				&corners,
				&mut remap,
				materials,
				&mut layout,
				&mut layout_semantics,
				&mut primitives,
				allocator,
			)?;
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
					&mut layout,
					&mut layout_semantics,
					&mut primitives,
					allocator,
				)?;
			}
		}
	}
	if primitives.is_empty() {
		return Err(FbxImportError::NoMesh);
	}
	let mut source = OwnedMeshSource::new(layout, primitives).with_skins(skins);
	source.set_skeleton(skeleton);
	Ok(source)
}

/// Imports one triangulated material part immediately so source-corner storage can be reused by the next part.
pub(crate) fn import_fbx_material_corners<'a>(
	context: &FbxMeshImportContext<'_>,
	material_slot: usize,
	corners: &[u32],
	remap: &mut [u32],
	materials: &ResolvedFbxMaterials,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	primitives: &mut Vec<OwnedMeshPrimitive<&'a dyn Allocator>, &'a dyn Allocator>,
	allocator: &'a dyn Allocator,
) -> Result<(), FbxImportError> {
	if corners.is_empty() {
		return Ok(());
	}
	let material = materials.get(material_key_for_slot(context.material_node, context.mesh, material_slot))?;
	for batch in remap_triangle_corners(context.mesh.num_indices, corners, remap, allocator)? {
		primitives.push(import_fbx_primitive(
			context,
			material.clone(),
			batch,
			layout,
			layout_semantics,
			allocator,
		)?);
	}
	Ok(())
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

/// Extracts one remapped FBX primitive while respecting independent per-corner attribute indices.
pub(crate) fn import_fbx_primitive<'a>(
	context: &FbxMeshImportContext<'_>,
	material: ReferenceModel<VariantModel>,
	batch: RemappedCorners<'a>,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	allocator: &'a dyn Allocator,
) -> Result<OwnedMeshPrimitive<&'a dyn Allocator>, FbxImportError> {
	if batch.source_corners.is_empty() {
		return Err(FbxImportError::EmptyPrimitive);
	}
	let mesh = context.mesh;
	let mut positions = Vec::with_capacity_in(batch.source_corners.len(), allocator);
	let mut minimum = [f32::INFINITY; 3];
	let mut maximum = [f32::NEG_INFINITY; 3];
	let mut normals = context
		.normal_matrix
		.is_some()
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut tangents = context
		.source_attributes
		.contains(VertexSemantics::Tangent)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut bitangents = context
		.source_attributes
		.contains(VertexSemantics::BiTangent)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	// Visibility rendering requires a UV stream even for untextured materials, so absent FBX UVs use the origin.
	let mut uvs = Vec::with_capacity_in(batch.source_corners.len(), allocator);
	let mut colors = context
		.source_attributes
		.contains(VertexSemantics::Color)
		.then(|| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut joints = context
		.skin
		.map(|_| Vec::with_capacity_in(batch.source_corners.len(), allocator));
	let mut weights = context
		.skin
		.map(|_| Vec::with_capacity_in(batch.source_corners.len(), allocator));

	for &source_corner in &batch.source_corners {
		let corner = source_corner as usize;
		let position = ufbx::transform_position(&context.node.geometry_to_world, mesh.vertex_position[corner]);
		let position = vec3_to_f32(position, "mesh position")?;
		for axis in 0..3 {
			minimum[axis] = minimum[axis].min(position[axis]);
			maximum[axis] = maximum[axis].max(position[axis]);
		}
		positions.push(position);

		// Build a world-space orthonormal tangent frame so non-uniform instance scales do not skew shading inputs.
		let normal = context
			.normal_matrix
			.as_ref()
			.map(|normal_matrix| normalized_direction(normal_matrix, mesh.vertex_normal[corner]))
			.transpose()?;
		let transformed_bitangent = context
			.source_attributes
			.contains(VertexSemantics::BiTangent)
			.then(|| normalized_direction(&context.node.geometry_to_world, mesh.vertex_bitangent[corner]))
			.transpose()?;
		let tangent = context
			.source_attributes
			.contains(VertexSemantics::Tangent)
			.then(|| normalized_direction(&context.node.geometry_to_world, mesh.vertex_tangent[corner]))
			.transpose()?
			.map(|tangent| match normal {
				Some(normal) => orthogonalized_direction(tangent, normal),
				None => Ok(tangent),
			})
			.transpose()?;

		if let Some(values) = normals.as_mut() {
			values.push(normal.expect("normal output exists only when the FBX normal attribute exists"));
		}
		if let Some(values) = tangents.as_mut() {
			let tangent = tangent.expect("tangent output exists only when the FBX tangent attribute exists");
			let handedness = match (normal, transformed_bitangent) {
				(Some(normal), Some(bitangent)) => tangent_handedness(normal, tangent, bitangent),
				_ => 1.0,
			};
			values.push([tangent[0], tangent[1], tangent[2], handedness]);
		}
		if let Some(values) = bitangents.as_mut() {
			let bitangent = match (normal, tangent, transformed_bitangent) {
				(Some(normal), Some(tangent), Some(bitangent)) => {
					let handedness = tangent_handedness(normal, tangent, bitangent);
					scale_vec3(cross_vec3(normal, tangent), handedness)
				}
				(Some(normal), None, Some(bitangent)) => orthogonalized_direction(bitangent, normal)?,
				(_, _, Some(bitangent)) => bitangent,
				(_, _, None) => unreachable!("bitangent output exists only when the FBX bitangent attribute exists"),
			};
			values.push(bitangent);
		}
		if context.source_attributes.contains(VertexSemantics::UV) {
			let uv = mesh.vertex_uv[corner];
			// FBX UVs are bottom-left based, while baked image rows and material sampling are top-left based.
			uvs.push([finite_f32(uv.x, "mesh UV")?, 1.0 - finite_f32(uv.y, "mesh UV")?]);
		} else {
			uvs.push([0.0, 0.0]);
		}
		if let Some(values) = colors.as_mut() {
			let color = mesh.vertex_color[corner];
			values.push([
				finite_f32(color.x, "mesh color")?,
				finite_f32(color.y, "mesh color")?,
				finite_f32(color.z, "mesh color")?,
				finite_f32(color.w, "mesh color")?,
			]);
		}
		if let (Some(skin), Some(joints), Some(weights)) = (context.skin, joints.as_mut(), weights.as_mut()) {
			let logical_vertex = *mesh.vertex_indices.get(corner).ok_or(FbxImportError::InvalidCornerIndex)? as usize;
			let (vertex_joints, vertex_weights) = skin_weights(skin, logical_vertex, context.fallback_joint)?;
			joints.push(vertex_joints);
			weights.push(vertex_weights);
		}
	}

	let bounds = [minimum, maximum];
	let mut triangle_indices = batch.indices;
	if context.mirrored {
		// Preserve the configured global front face when an authored instance mirrors its flattened geometry.
		for triangle in triangle_indices.chunks_exact_mut(3) {
			triangle.swap(1, 2);
		}
	}
	let mut primitive = OwnedMeshPrimitive::new_in(material, bounds, triangle_indices, allocator);
	primitive.set_transform_node(context.transform_node);
	primitive.set_skin(context.skin_index);
	add_mesh_attribute(
		&mut primitive,
		layout,
		layout_semantics,
		VertexSemantics::Position,
		"vec3f",
		OwnedMeshAttributeData::F32x3(positions),
	);
	if let Some(values) = normals {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Normal,
			"vec3f",
			OwnedMeshAttributeData::F32x3(values),
		);
	}
	if let Some(values) = tangents {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Tangent,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	if let Some(values) = bitangents {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::BiTangent,
			"vec3f",
			OwnedMeshAttributeData::F32x3(values),
		);
	}
	add_mesh_attribute(
		&mut primitive,
		layout,
		layout_semantics,
		VertexSemantics::UV,
		"vec2f",
		OwnedMeshAttributeData::F32x2(uvs),
	);
	if let Some(values) = colors {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Color,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	if let Some(values) = joints {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Joints,
			"vec4u16",
			OwnedMeshAttributeData::U16x4(values),
		);
	}
	if let Some(values) = weights {
		add_mesh_attribute(
			&mut primitive,
			layout,
			layout_semantics,
			VertexSemantics::Weights,
			"vec4f",
			OwnedMeshAttributeData::F32x4(values),
		);
	}
	Ok(primitive)
}

/// Adds attribute payload and records its shared layout declaration on first use.
pub(crate) fn add_mesh_attribute<'a>(
	primitive: &mut OwnedMeshPrimitive<&'a dyn Allocator>,
	layout: &mut Vec<VertexComponent, &'a dyn Allocator>,
	layout_semantics: &mut VertexAttributeMask,
	semantic: VertexSemantics,
	format: &str,
	data: OwnedMeshAttributeData<&'a dyn Allocator>,
) {
	if layout_semantics.insert(semantic) {
		layout.push(VertexComponent {
			semantic,
			format: format.to_string(),
			channel: 0,
		});
	}
	primitive.add_attribute(OwnedMeshAttribute::new(semantic, 0, data));
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
	if alignment < 0.0 {
		-1.0
	} else {
		1.0
	}
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
