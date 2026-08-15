use super::*;

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
	pub(crate) fn write_prepared_gpu_mesh_data_and_return_mesh_object(
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

	pub(super) fn build_generated_meshlets(
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
			cone_axis: encode_octahedral_unit_vector((0.0, 0.0, 1.0)),
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
pub(crate) fn checked_visibility_capacity(current: u32, additional: usize, limit: usize, element: &str) -> Option<u32> {
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
