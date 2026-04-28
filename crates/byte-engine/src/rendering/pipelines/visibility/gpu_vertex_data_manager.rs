/// The `GPUVertexDataManager` is responsible for managing the vertex data buffers used in the visibility pipeline.
/// It tracks buffer offsets and counts for various resources, and provides handles to the vertex data buffers.
/// It performs uploads to the GPU of mesh resources.
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
	pub fn new(device: &mut ghi::implementation::Device) -> Self {
		let vertex_positions_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Positions Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_normals_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Normals Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_uv_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex UV Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_indices_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Index Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let primitive_indices_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Primitive Indices Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let meshlets_data_buffer = device.build_buffer::<[ShaderMeshletData; MAX_MESHLETS]>(
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
	pub fn write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource<'a, 'slf: 'a, 'buffer>(
		&'slf mut self,
		id: &'a str,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		resource_request: &mut Reference<Mesh>,
	) -> Option<MeshData> {
		let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

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
		assert_eq!(meshlets_stream.stride, 2, "Meshlet stream stride is not of size 2");
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

		struct Meshlet {
			primitive_count: u8,
			triangle_count: u8,
		}

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

						let meshlet_stream = unsafe {
							std::slice::from_raw_parts(
								m.buffer().as_ptr().byte_add(stream.offset) as *const Meshlet,
								stream.count(),
							)
						};

						meshlet_stream
							.iter()
							.scan(
								(0, 0),
								|(primitive_primitive_counter, primitive_triangle_counter), meshlet| {
									let meshlet_primitive_count = meshlet.primitive_count;
									let meshlet_triangle_count = meshlet.triangle_count;

									let primitive_offset = *primitive_primitive_counter as u16;
									let triangle_offset = *primitive_triangle_counter as u16;

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

		let meshlets_data_size = total_meshlet_count * std::mem::size_of::<ShaderMeshletData>();
		let (meshlets_data_staging_offset, meshlets_data_bytes) = slice.take_with_offset(meshlets_data_size);
		let meshlets_data_slice = unsafe {
			std::slice::from_raw_parts_mut(
				meshlets_data_bytes.as_mut_ptr() as *mut ShaderMeshletData,
				total_meshlet_count,
			)
		};
		for (primitive, meshlets) in meshlets_per_primitive.iter() {
			for (j, meshlet) in meshlets.iter().enumerate() {
				meshlets_data_slice[primitive.meshlet_offset as usize + j] = *meshlet;
			}
		}
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

		let (vertex_indices, primitive_indices, meshlets) = Self::build_generated_meshlets(&indices).ok()?;

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

	fn build_generated_meshlets(indices: &[u16]) -> Result<(Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>), ()> {
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
				&& (meshlet_vertex_indices.len() + unique_vertices > u8::MAX as usize
					|| meshlet_triangles.len() >= u8::MAX as usize)
			{
				Self::push_generated_meshlet(
					&mut vertex_indices,
					&mut primitive_indices,
					&mut meshlets,
					&mut meshlet_vertex_indices,
					&mut meshlet_triangles,
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
		)?;

		Ok((vertex_indices, primitive_indices, meshlets))
	}

	fn push_generated_meshlet(
		vertex_indices: &mut Vec<u16>,
		primitive_indices: &mut Vec<[u8; 3]>,
		meshlets: &mut Vec<ShaderMeshletData>,
		meshlet_vertex_indices: &mut Vec<u16>,
		meshlet_triangles: &mut Vec<[u8; 3]>,
	) -> Result<(), ()> {
		if meshlet_triangles.is_empty() {
			return Ok(());
		}

		let primitive_offset = u16::try_from(vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh exceeds primitive index limits. The most likely cause is that the visibility pipeline buffers are too small for the generated mesh data."
			);
		})?;
		let triangle_offset = u16::try_from(primitive_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh exceeds triangle index limits. The most likely cause is that the visibility pipeline buffers are too small for the generated mesh data."
			);
		})?;
		let primitive_count = u8::try_from(meshlet_vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds vertex limits. The most likely cause is that too many unique vertices were packed into a single meshlet."
			);
		})?;
		let triangle_count = u8::try_from(meshlet_triangles.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds triangle limits. The most likely cause is that too many triangles were packed into a single meshlet."
			);
		})?;

		vertex_indices.extend(meshlet_vertex_indices.iter().copied());
		primitive_indices.extend(meshlet_triangles.iter().copied());
		meshlets.push(ShaderMeshletData {
			primitive_offset,
			triangle_offset,
			primitive_count,
			triangle_count,
		});

		meshlet_vertex_indices.clear();
		meshlet_triangles.clear();

		Ok(())
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
};
