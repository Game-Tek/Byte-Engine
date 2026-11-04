//! GPU geometry storage for the visibility pipeline: parallel vertex streams, meshlet records, skinning sources, and,
//! for ray tracing, flat triangle lists.
//!
//! [`PreparedMesh`] is built on a loader lane without touching GPU state. [`GeometryBuffers::append_mesh`] then
//! takes a free range of every stream from [`GeometryRanges`], and returns the [`MeshData`] that owns those ranges,
//! with the copies the loader records to fill them. Releasing a mesh hands its [`MeshRanges`] to [`RetiredRanges`],
//! which gives them back once no frame in flight reads them.

mod prepare;

use std::{
	ops::Range,
	sync::{Arc, Mutex},
};

use ghi::context::ContextCreate as _;
use resource_management::resources::skeleton::SkinBinding;
use smallvec::SmallVec;
use utils::RangeAllocator;

pub(crate) use self::prepare::{
	GENERATED_MESH_MATERIAL, PreparedLevels, PreparedMesh, PreparedSurface, decode_octahedral_unit_vector,
	encode_octahedral_unit_vector,
};
use super::layout::{
	MAX_ADDRESSABLE_MESHLETS, MAX_INSTANCE_MESHLETS, RuntimeVertexLightmapUv, RuntimeVertexNormal, RuntimeVertexUv,
	ShaderMeshletData, VERTEX_LIGHTMAP_UV_BUFFER_STRIDE, VERTEX_NORMAL_BUFFER_STRIDE, VERTEX_UV_BUFFER_STRIDE,
};
use super::lod::MeshLods;
use crate::rendering::loading::{BufferRegion, Loader};

/// Byte strides of the compact bind-pose streams consumed by GPU skinning.
pub(crate) const SKINNING_POSITION_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_NORMAL_STRIDE: usize = std::mem::size_of::<[f32; 3]>();
pub(crate) const SKINNING_JOINTS_STRIDE: usize = std::mem::size_of::<[u16; 4]>();
pub(crate) const SKINNING_WEIGHTS_STRIDE: usize = std::mem::size_of::<[f32; 4]>();

/// The `GeometryCapacity` struct sizes the scene-wide geometry buffers that every resident mesh is appended to.
///
/// All loaded meshes share these buffers, so size them for the most geometry an application keeps resident at once.
/// A mesh whose coarsest levels do not fit is rejected when it is uploaded. Finer levels load only while an instance
/// wants them, and the levels no instance wants give their room back when another mesh needs it. Pass the capacity to
/// [`super::VisibilityPipelineSettings::with_geometry_capacity`], or set it with the startup parameters under
/// [`super::GEOMETRY_CAPACITY_PARAMETER_PREFIX`], such as `render.geometry.triangle-capacity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryCapacity {
	/// Vertices across every mesh, plus each posed skinned instance's own copy of its primitive's vertices. Each vertex
	/// stores a position, a normal, and a UV.
	pub vertices: u32,
	/// Meshlet-local vertex indices across every mesh. Each meshlet stores one per vertex it references.
	pub vertex_indices: u32,
	pub triangles: u32,
	pub meshlets: u32,
	/// Bind-pose vertices of meshes that use GPU skinning.
	pub skinning_vertices: u32,
}

impl Default for GeometryCapacity {
	fn default() -> Self {
		Self {
			vertices: 65536 * 4,
			// Twice the vertices, so a scene's detail levels fit beside the full detail that fills the vertex budget.
			vertex_indices: 65536 * 8,
			triangles: 65536 * 8,
			meshlets: 1024 * 8,
			skinning_vertices: 65536 * 4,
		}
	}
}

impl GeometryCapacity {
	/// Checks that every stream can be allocated and that the visibility buffer can address every meshlet.
	pub(crate) fn validate(&self) -> Result<(), String> {
		let streams = [
			("vertex", self.vertices),
			("vertex index", self.vertex_indices),
			("triangle", self.triangles),
			("meshlet", self.meshlets),
			("skinning vertex", self.skinning_vertices),
		];
		if let Some((stream, _)) = streams.iter().find(|(_, capacity)| *capacity == 0) {
			return Err(format!(
				"Geometry capacity was not set. The most likely cause is that the {stream} capacity is zero; every geometry buffer needs at least one element."
			));
		}
		if self.meshlets as usize > MAX_ADDRESSABLE_MESHLETS {
			return Err(format!(
				"Geometry capacity was not set. The most likely cause is that {} meshlets exceed the {MAX_ADDRESSABLE_MESHLETS} the visibility buffer can address.",
				self.meshlets
			));
		}
		Ok(())
	}
}

/// The `GeometryCounts` struct measures one mesh in elements of each geometry stream, so the loader knows how much of
/// each stream to take before it places the mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GeometryCounts {
	pub(crate) vertices: u32,
	pub(crate) primitive_indices: u32,
	pub(crate) triangles: u32,
	pub(crate) meshlets: u32,
	pub(crate) skinning_vertices: u32,
	/// Full-detail triangles of the primitives rays can hit; see [`GeometryHandles::flat_triangle_indices`].
	pub(crate) flat_triangles: u32,
}

/// The `MeshRanges` struct names the elements of every geometry stream one resident mesh owns.
///
/// [`GeometryRanges::take`] hands them out when a mesh is placed. Release them with [`RetiredRanges::retire`] once no
/// renderable draws the mesh, so the loader can place other meshes there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshRanges {
	pub(crate) vertices: Range<u32>,
	/// Meshlet-local vertex indices.
	pub(crate) primitive_indices: Range<u32>,
	pub(crate) triangles: Range<u32>,
	pub(crate) meshlets: Range<u32>,
	/// Bind-pose vertices in the compact skinning buffers.
	pub(crate) skinning_vertices: Range<u32>,
	/// Full-detail triangles in [`GeometryHandles::flat_triangle_indices`].
	pub(crate) flat_triangles: Range<u32>,
}

/// The `GeometryRanges` struct tracks the free elements of every geometry stream, so meshes and posed skinned instances
/// can be placed in the space released ones leave.
///
/// The loader takes whole meshes with [`Self::take`]; the render thread takes posed skinned instances' vertices with
/// [`allocate_vertices`]. Both share one value through [`GeometryHandles::ranges`].
pub(crate) struct GeometryRanges {
	vertices: RangeAllocator,
	primitive_indices: RangeAllocator,
	triangles: RangeAllocator,
	meshlets: RangeAllocator,
	skinning_vertices: RangeAllocator,
	flat_triangles: RangeAllocator,
}

impl GeometryRanges {
	/// Starts with every stream of `capacity` free.
	pub(crate) fn new(capacity: GeometryCapacity) -> Self {
		let stream = |length: u32| RangeAllocator::new(length as usize, 1);
		Self {
			vertices: stream(capacity.vertices),
			primitive_indices: stream(capacity.vertex_indices),
			triangles: stream(capacity.triangles),
			meshlets: stream(capacity.meshlets),
			skinning_vertices: stream(capacity.skinning_vertices),
			// A primitive's full detail is one of its levels, so the triangle capacity bounds the flat lists too.
			flat_triangles: stream(capacity.triangles),
		}
	}

	/// Takes room for a mesh of `counts` in every stream, or returns the name of the first stream that lacks a free
	/// range that large and takes nothing.
	///
	/// A mesh is placed whole or not at all, so every stream is checked before any is taken. The caller decides
	/// whether a full stream is an error: a base mesh that does not fit is, a finer detail level waits for room.
	pub(crate) fn take(&mut self, counts: &GeometryCounts) -> Result<MeshRanges, &'static str> {
		let streams = [
			(&self.vertices, counts.vertices, "vertex"),
			(&self.primitive_indices, counts.primitive_indices, "vertex index"),
			(&self.triangles, counts.triangles, "triangle"),
			(&self.meshlets, counts.meshlets, "meshlet"),
			(&self.skinning_vertices, counts.skinning_vertices, "skinning vertex"),
			(&self.flat_triangles, counts.flat_triangles, "flat triangle"),
		];
		// An empty request always succeeds, even in a full stream.
		if let Some((_, _, element)) = streams
			.iter()
			.find(|(allocator, length, _)| *length > 0 && !allocator.fits(*length as usize, 1))
		{
			return Err(element);
		}
		let take = |allocator: &mut RangeAllocator, length: u32| {
			let range = allocator
				.take(length as usize, 1)
				.expect("A checked geometry range could not be taken. The most likely cause is a bug in RangeAllocator::fits; make it agree with RangeAllocator::take.");
			range.start as u32..range.end as u32
		};
		Ok(MeshRanges {
			vertices: take(&mut self.vertices, counts.vertices),
			primitive_indices: take(&mut self.primitive_indices, counts.primitive_indices),
			triangles: take(&mut self.triangles, counts.triangles),
			meshlets: take(&mut self.meshlets, counts.meshlets),
			skinning_vertices: take(&mut self.skinning_vertices, counts.skinning_vertices),
			flat_triangles: take(&mut self.flat_triangles, counts.flat_triangles),
		})
	}

	/// Frees every stream range of a mesh no frame reads any more. [`RetiredRanges`] calls it after the frame fence.
	pub(crate) fn give_back(&mut self, ranges: MeshRanges) {
		let give_back = |allocator: &mut RangeAllocator, range: Range<u32>| {
			allocator.give_back(range.start as usize..range.end as usize);
		};
		give_back(&mut self.vertices, ranges.vertices);
		give_back(&mut self.primitive_indices, ranges.primitive_indices);
		give_back(&mut self.triangles, ranges.triangles);
		give_back(&mut self.meshlets, ranges.meshlets);
		give_back(&mut self.skinning_vertices, ranges.skinning_vertices);
		give_back(&mut self.flat_triangles, ranges.flat_triangles);
	}

	/// Frees the vertex range of a posed skinned instance that left the scene.
	pub(crate) fn give_back_vertices(&mut self, range: Range<u32>) {
		self.vertices.give_back(range.start as usize..range.end as usize);
	}
}

/// Takes `length` vertices from `ranges`, logging the error for `element` when the vertex capacity can't hold them.
pub(crate) fn allocate_vertices(ranges: &Mutex<GeometryRanges>, length: u32, element: &str) -> Option<Range<u32>> {
	let range = ranges
		.lock()
		.unwrap_or_else(|error| error.into_inner())
		.vertices
		.take(length as usize, 1)
		.map(|range| range.start as u32..range.end as u32);
	if range.is_none() {
		log::error!(
			"Visibility vertex buffer capacity exceeded. The most likely cause is that the resident meshes and posed skinned instances need more vertices than the geometry capacity holds, with {length} more for {element}; raise it with VisibilityPipelineSettings::with_geometry_capacity or render.geometry.vertex-capacity."
		);
	}
	range
}

/// The `RetiredRanges` struct holds the ranges of released meshes until no frame in flight can read them, so the
/// loader never overwrites geometry a submitted frame still draws.
///
/// The render thread retires a mesh's [`MeshRanges`] after its last renderable left the scene, then calls
/// [`Self::give_back_completed`] every frame once that frame waited for its fence.
#[derive(Default)]
pub(crate) struct RetiredRanges {
	/// Each mesh's ranges with the first frame that no longer reads them, in retirement order.
	retired: Vec<(u64, MeshRanges)>,
}

impl RetiredRanges {
	/// Holds `ranges` until every frame before `first_unread_frame` finished on the GPU.
	pub(crate) fn retire(&mut self, ranges: MeshRanges, first_unread_frame: u64) {
		self.retired.push((first_unread_frame, ranges));
	}

	/// Gives back to `ranges` every retired mesh that no frame in flight reads once `frame_index` started.
	///
	/// Starting a frame waits for the fence of the frame [`ghi::MAX_FRAMES_IN_FLIGHT`] earlier, so a range retired
	/// for `first_unread_frame` is free from frame `first_unread_frame + MAX_FRAMES_IN_FLIGHT` on.
	pub(crate) fn give_back_completed(&mut self, frame_index: u64, ranges: &Mutex<GeometryRanges>) {
		if self.retired.is_empty() {
			return;
		}
		let mut ranges = ranges.lock().unwrap_or_else(|error| error.into_inner());
		self.retired.retain(|(first_unread_frame, retired)| {
			let completed = first_unread_frame + ghi::MAX_FRAMES_IN_FLIGHT as u64 <= frame_index;
			if completed {
				ranges.give_back(retired.clone());
			}
			!completed
		});
	}
}

/// The `MeshData` struct locates one resident mesh in the visibility geometry buffers.
#[derive(Debug, Clone)]
pub(crate) struct MeshData {
	pub(crate) primitives: Vec<MeshPrimitive>,
	/// Number of global pose matrices expected from a renderable using this mesh.
	pub(crate) skeleton_node_count: u32,
	/// The elements of every stream the mesh owns. Primitive offsets are relative to each range's start.
	pub(crate) ranges: MeshRanges,
}

/// The `PrimitiveBlock` struct locates one primitive's finer detail levels, placed in ranges of their own so they load
/// and release apart from the rest of its mesh.
///
/// [`GeometryBuffers::append_primitives`] places them. Overlay [`Self::lods`] on the primitive's base levels to draw
/// them, and release [`Self::ranges`] with [`RetiredRanges::retire`] once no instance draws them.
#[derive(Debug, Clone)]
pub(crate) struct PrimitiveBlock {
	pub(crate) ranges: MeshRanges,
	/// The primitive's level table, with the levels this block holds resident and naming buffer elements.
	pub(crate) lods: MeshLods,
}

/// The `PrimitiveLevels` struct names one primitive's part in a load of finer detail levels and carries back where
/// they landed.
///
/// [`super::residency::MeshResidencies::end_frame`] lists them, [`GeometryBuffers::append_primitives`] fills in each
/// [`Self::block`], and the same list returns to the residencies with the load's outcome, so a load allocates no list
/// of its own.
#[derive(Debug, Clone)]
pub(crate) struct PrimitiveLevels {
	/// The primitive's index in its mesh.
	pub(crate) primitive: u32,
	/// The finest level to load. The load holds the primitive's levels from it down to, but not including, its coarsest.
	pub(crate) finest: u8,
	/// Where the levels landed: `None` until the load places them, and after it when the geometry buffers had no room.
	pub(crate) block: Option<PrimitiveBlock>,
}

/// The `MeshPrimitive` struct locates one primitive's geometry, material slot, and optional skin inside its mesh.
#[derive(Debug, Clone, Default)]
pub(crate) struct MeshPrimitive {
	pub(crate) material_index: u32,
	/// The primitive's detail levels. The levels this mesh holds are resident; once the mesh is placed they name
	/// buffer elements.
	pub(crate) lods: MeshLods,
	/// Object-space sphere around full detail, as xyz center and w radius. Task shaders cull whole instances with it.
	pub(crate) bounding_sphere: [f32; 4],
	/// Offsets relative to the owning mesh.
	pub(crate) meshlet_offset: u32,
	pub(crate) vertex_offset: u32,
	pub(crate) primitive_offset: u32,
	pub(crate) triangle_offset: u32,
	/// The primitive's vertices this mesh holds: a prefix of its baked vertices, enough for its resident levels.
	pub(crate) vertex_count: u32,
	/// The primitive's full-detail triangles in [`GeometryHandles::flat_triangle_indices`], relative to the mesh's
	/// first. Zero triangles when the pipeline keeps no flat lists, or for a skinned primitive, which rays never hit.
	pub(crate) flat_triangle_offset: u32,
	pub(crate) flat_triangle_count: u32,
	/// First vertex in the compact bind-pose skinning buffers, once resident.
	pub(crate) skinning_source_vertex_offset: Option<u32>,
	pub(crate) skinning_vertex_count: u32,
	pub(crate) skin: Option<Arc<SkinBinding>>,
}

/// The `GeometryHandles` struct names the GPU streams every visibility stage reads.
///
/// Handles are stable for the life of the pipeline, so the render thread clones this value at construction. The loader
/// and the render thread share [`Self::ranges`], which decides where the next mesh lands.
#[derive(Clone)]
pub(crate) struct GeometryHandles {
	/// The element count of every stream below.
	pub(crate) capacity: GeometryCapacity,
	pub(crate) vertex_positions: ghi::BufferHandle<[[f32; 3]]>,
	/// Octahedrally encoded normals, two UNORM16 components each.
	pub(crate) vertex_normals: ghi::BufferHandle<[RuntimeVertexNormal]>,
	pub(crate) vertex_uvs: ghi::BufferHandle<[RuntimeVertexUv]>,
	/// UNORM16 lightmap UVs, zero for vertices of primitives without them.
	pub(crate) vertex_lightmap_uvs: ghi::BufferHandle<[RuntimeVertexLightmapUv]>,
	/// Meshlet-local indices into the vertex buffers.
	pub(crate) vertex_indices: ghi::BufferHandle<[u16]>,
	/// Triangles as indices into `vertex_indices`.
	pub(crate) primitive_indices: ghi::BufferHandle<[[u8; 3]]>,
	pub(crate) meshlets: ghi::BufferHandle<[ShaderMeshletData]>,
	/// Each primitive's full-detail triangles as three indices relative to its first vertex, the form acceleration
	/// structures and hit shaders read. Present only when the pipeline traces rays.
	pub(crate) flat_triangle_indices: Option<ghi::BufferHandle<[[u16; 3]]>>,
	/// Bind-pose attributes packed only for primitives that participate in GPU skinning.
	pub(crate) skinning_rest_positions: ghi::BufferHandle<[[f32; 3]]>,
	pub(crate) skinning_rest_normals: ghi::BufferHandle<[[f32; 3]]>,
	pub(crate) skinning_joints: ghi::BufferHandle<[[u16; 4]]>,
	pub(crate) skinning_weights: ghi::BufferHandle<[[f32; 4]]>,
	/// The free elements of every stream, which the loader takes mesh ranges from and the render thread takes posed
	/// skinned instance vertices from.
	pub(crate) ranges: Arc<Mutex<GeometryRanges>>,
}

/// The `GeometryBuffers` struct places meshes in the free space of the geometry streams as they become resident.
///
/// It lives with the loader. Its handles name the render context's streams as the loader context imported them.
pub(crate) struct GeometryBuffers {
	handles: GeometryHandles,
}

impl GeometryHandles {
	/// Creates every geometry stream with the element count `capacity` gives it, and the flat triangle lists when
	/// `ray_tracing` is set.
	///
	/// `capacity` comes from [`super::VisibilityPipelineSettings::geometry_capacity`], which only holds validated values.
	/// Set `ray_tracing` when surfel GI is built in and the device traces rays. Next, hand the handles to
	/// [`GeometryBuffers::import`] so the loader can append meshes.
	pub(crate) fn new(context: &mut ghi::implementation::Context, capacity: GeometryCapacity, ray_tracing: bool) -> Self {
		debug_assert!(capacity.validate().is_ok());
		let geometry = ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage;
		let indices = ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage;
		let storage = ghi::Uses::Storage;
		let build = |name, uses, length: u32| {
			ghi::buffer::Builder::new(uses)
				.name(name)
				.length(length as usize)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let GeometryCapacity {
			vertices,
			vertex_indices,
			triangles,
			meshlets,
			skinning_vertices: skinning,
		} = capacity;
		Self {
			capacity,
			vertex_positions: context.build_buffer(build("Visibility Vertex Positions Buffer", geometry, vertices)),
			vertex_normals: context.build_buffer(build("Visibility Vertex Normals Buffer", geometry, vertices)),
			vertex_uvs: context.build_buffer(build("Visibility Vertex UV Buffer", geometry, vertices)),
			vertex_lightmap_uvs: context.build_buffer(build("Visibility Vertex Lightmap UV Buffer", geometry, vertices)),
			vertex_indices: context.build_buffer(build("Visibility Index Buffer", indices, vertex_indices)),
			primitive_indices: context.build_buffer(build("Visibility Primitive Indices Buffer", indices, triangles)),
			meshlets: context.build_buffer(build("Visibility Meshlets Data", storage, meshlets)),
			flat_triangle_indices: ray_tracing
				.then(|| context.build_buffer(build("Visibility Flat Triangle Indices", indices, triangles))),
			skinning_rest_positions: context.build_buffer(build("Visibility Skinning Rest Positions", storage, skinning)),
			skinning_rest_normals: context.build_buffer(build("Visibility Skinning Rest Normals", storage, skinning)),
			skinning_joints: context.build_buffer(build("Visibility Skinning Joints", storage, skinning)),
			skinning_weights: context.build_buffer(build("Visibility Skinning Weights", storage, skinning)),
			ranges: Arc::new(Mutex::new(GeometryRanges::new(capacity))),
		}
	}
}

impl GeometryBuffers {
	/// Imports every stream `handles` names from the render context into the loader.
	pub(crate) fn import(handles: &GeometryHandles, render: &ghi::implementation::Context, loader: &mut Loader) -> Self {
		fn import<T: ?Sized>(
			render: &ghi::implementation::Context,
			loader: &mut Loader,
			buffer: ghi::BufferHandle<T>,
		) -> ghi::BufferHandle<T> {
			loader.import_buffer(render.share_buffer(buffer))
		}
		Self {
			handles: GeometryHandles {
				capacity: handles.capacity,
				vertex_positions: import(render, loader, handles.vertex_positions),
				vertex_normals: import(render, loader, handles.vertex_normals),
				vertex_uvs: import(render, loader, handles.vertex_uvs),
				vertex_lightmap_uvs: import(render, loader, handles.vertex_lightmap_uvs),
				vertex_indices: import(render, loader, handles.vertex_indices),
				primitive_indices: import(render, loader, handles.primitive_indices),
				meshlets: import(render, loader, handles.meshlets),
				flat_triangle_indices: handles.flat_triangle_indices.map(|buffer| import(render, loader, buffer)),
				skinning_rest_positions: import(render, loader, handles.skinning_rest_positions),
				skinning_rest_normals: import(render, loader, handles.skinning_rest_normals),
				skinning_joints: import(render, loader, handles.skinning_joints),
				skinning_weights: import(render, loader, handles.skinning_weights),
				ranges: handles.ranges.clone(),
			},
		}
	}

	/// Places a prepared mesh in free ranges of every stream and returns where it lives with the copies that fill it.
	///
	/// `material_indices` supplies the render-thread material slot for each prepared primitive. Every resident level
	/// of the returned primitives names buffer elements. Nothing is reserved when the mesh does not fit, so partial
	/// residency is never published; the error says which stream was full, or `None` for a primitive no instance can
	/// draw. Hand the copies to the loader with the mesh's staging lease, and publish the mesh only once they finished.
	pub(crate) fn append_mesh(
		&mut self,
		prepared: &PreparedMesh,
		material_indices: &[u32],
	) -> Result<(MeshData, SmallVec<[BufferRegion; 16]>), Option<&'static str>> {
		// Task shaders address an instance's meshlets relative to the first one of the level it draws, with a bounded
		// index width. So the limit applies to each level, not to the primitive's levels together.
		let levels = prepared
			.primitives
			.iter()
			.flat_map(|prepared| prepared.primitive.lods.levels());
		if let Some(level) = levels
			.into_iter()
			.find(|level| level.meshlet_count as usize > MAX_INSTANCE_MESHLETS)
		{
			log::error!(
				"Visibility mesh upload was rejected. The most likely cause is that a primitive has {} meshlets, more than the {MAX_INSTANCE_MESHLETS} one instance can address; split the primitive.",
				level.meshlet_count
			);
			return Err(None);
		}
		let ranges = self
			.handles
			.ranges
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.take(&prepared.counts)
			.map_err(Some)?;
		let base_vertex = ranges.vertices.start;
		let streams = &prepared.streams;
		let copy =
			|source: &std::ops::Range<usize>, destination: ghi::BaseBufferHandle, destination_offset: usize| BufferRegion {
				offset: source.start,
				destination,
				destination_offset,
				size: source.len(),
			};
		let mut copies = SmallVec::<[BufferRegion; 16]>::from_iter([
			copy(
				&streams.positions,
				self.handles.vertex_positions.into(),
				base_vertex as usize * 12,
			),
			copy(
				&streams.normals,
				self.handles.vertex_normals.into(),
				base_vertex as usize * VERTEX_NORMAL_BUFFER_STRIDE as usize,
			),
			copy(
				&streams.uvs,
				self.handles.vertex_uvs.into(),
				base_vertex as usize * VERTEX_UV_BUFFER_STRIDE as usize,
			),
			copy(
				&streams.lightmap_uvs,
				self.handles.vertex_lightmap_uvs.into(),
				base_vertex as usize * VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize,
			),
			copy(
				&streams.vertex_indices,
				self.handles.vertex_indices.into(),
				ranges.primitive_indices.start as usize * 2,
			),
			copy(
				&streams.primitive_indices,
				self.handles.primitive_indices.into(),
				ranges.triangles.start as usize * 3,
			),
			copy(
				&streams.meshlets,
				self.handles.meshlets.into(),
				ranges.meshlets.start as usize * std::mem::size_of::<ShaderMeshletData>(),
			),
		]);
		if let (Some(source), Some(destination)) = (&streams.flat_triangles, self.handles.flat_triangle_indices) {
			copies.push(copy(
				source,
				destination.into(),
				ranges.flat_triangles.start as usize * std::mem::size_of::<[u16; 3]>(),
			));
		}

		let primitives = prepared
			.primitives
			.iter()
			.zip(material_indices)
			.map(|(prepared, material_index)| {
				let mut primitive = prepared.primitive.clone();
				primitive.material_index = *material_index;
				// The prepared levels count from the start of the prepared streams; the mesh starts at its ranges.
				for level in primitive.lods.levels_mut().iter_mut().filter(|level| level.resident) {
					level.first_meshlet += ranges.meshlets.start + primitive.meshlet_offset;
					level.vertex_base += ranges.vertices.start;
					level.primitive_index_base += ranges.primitive_indices.start + primitive.primitive_offset;
					level.triangle_base += ranges.triangles.start + primitive.triangle_offset;
				}
				if let (Some(skinning), Some(relative_offset)) =
					(&prepared.skinning, primitive.skinning_source_vertex_offset.as_mut())
				{
					let destination_vertex = (ranges.skinning_vertices.start + *relative_offset) as usize;
					*relative_offset += ranges.skinning_vertices.start;
					copies.extend([
						copy(
							&skinning.positions,
							self.handles.skinning_rest_positions.into(),
							destination_vertex * SKINNING_POSITION_STRIDE,
						),
						copy(
							&skinning.normals,
							self.handles.skinning_rest_normals.into(),
							destination_vertex * SKINNING_NORMAL_STRIDE,
						),
						copy(
							&skinning.joints,
							self.handles.skinning_joints.into(),
							destination_vertex * SKINNING_JOINTS_STRIDE,
						),
						copy(
							&skinning.weights,
							self.handles.skinning_weights.into(),
							destination_vertex * SKINNING_WEIGHTS_STRIDE,
						),
					]);
				}
				primitive
			})
			.collect();

		let mesh = MeshData {
			primitives,
			skeleton_node_count: prepared.skeleton_node_count,
			ranges,
		};
		Ok((mesh, copies))
	}

	/// Places each primitive of `primitives` in free ranges of its own, so each loads and releases apart from the
	/// others, by filling in its block, and returns the copies that fill them from `prepared`.
	///
	/// A primitive without room keeps no block and takes nothing, while the others still land. Returns `Err` with the
	/// meshlet count of a level no instance can address, and places nothing. Hand the copies to the loader with the
	/// prepared mesh's staging lease, and publish the blocks only once they finished.
	pub(crate) fn append_primitives(
		&mut self,
		prepared: &PreparedMesh,
		primitives: &mut [PrimitiveLevels],
	) -> Result<SmallVec<[BufferRegion; 16]>, u32> {
		let unaddressable = prepared
			.primitives
			.iter()
			.flat_map(|prepared| prepared.primitive.lods.levels())
			.find(|level| level.resident && level.meshlet_count as usize > MAX_INSTANCE_MESHLETS);
		if let Some(level) = unaddressable {
			return Err(level.meshlet_count);
		}
		let streams = &prepared.streams;
		let handles = &self.handles;
		// Seven stream copies per primitive, reserved at once.
		let mut copies = SmallVec::with_capacity(primitives.len() * 7);
		let mut ranges = handles.ranges.lock().unwrap_or_else(|error| error.into_inner());
		for levels in primitives.iter_mut() {
			let prepared = &prepared.primitives[levels.primitive as usize];
			let elements = &prepared.elements;
			if elements.meshlets.is_empty() {
				continue;
			}
			let length = |range: &Range<u32>| range.end - range.start;
			let counts = GeometryCounts {
				vertices: length(&elements.vertices),
				primitive_indices: length(&elements.primitive_indices),
				triangles: length(&elements.triangles),
				meshlets: length(&elements.meshlets),
				..GeometryCounts::default()
			};
			let Ok(block) = ranges.take(&counts) else {
				continue;
			};
			// Copies this primitive's `elements` of a prepared stream, of `stride` bytes each, to `destination`.
			let copy = |source: &Range<usize>,
			            elements: &Range<u32>,
			            stride: usize,
			            destination: ghi::BaseBufferHandle,
			            first: u32| {
				BufferRegion {
					offset: source.start + elements.start as usize * stride,
					destination,
					destination_offset: first as usize * stride,
					size: (elements.end - elements.start) as usize * stride,
				}
			};
			copies.extend([
				copy(
					&streams.positions,
					&elements.vertices,
					12,
					handles.vertex_positions.into(),
					block.vertices.start,
				),
				copy(
					&streams.normals,
					&elements.vertices,
					VERTEX_NORMAL_BUFFER_STRIDE as usize,
					handles.vertex_normals.into(),
					block.vertices.start,
				),
				copy(
					&streams.uvs,
					&elements.vertices,
					VERTEX_UV_BUFFER_STRIDE as usize,
					handles.vertex_uvs.into(),
					block.vertices.start,
				),
				copy(
					&streams.lightmap_uvs,
					&elements.vertices,
					VERTEX_LIGHTMAP_UV_BUFFER_STRIDE as usize,
					handles.vertex_lightmap_uvs.into(),
					block.vertices.start,
				),
				copy(
					&streams.vertex_indices,
					&elements.primitive_indices,
					2,
					handles.vertex_indices.into(),
					block.primitive_indices.start,
				),
				copy(
					&streams.primitive_indices,
					&elements.triangles,
					3,
					handles.primitive_indices.into(),
					block.triangles.start,
				),
				copy(
					&streams.meshlets,
					&elements.meshlets,
					std::mem::size_of::<ShaderMeshletData>(),
					handles.meshlets.into(),
					block.meshlets.start,
				),
			]);
			// The prepared levels count from the start of the prepared streams; the block starts at its ranges.
			let mut lods = prepared.primitive.lods;
			for level in lods.levels_mut().iter_mut().filter(|level| level.resident) {
				level.first_meshlet = block.meshlets.start + level.first_meshlet - elements.meshlets.start;
				level.vertex_base = block.vertices.start + level.vertex_base - elements.vertices.start;
				level.primitive_index_base =
					block.primitive_indices.start + level.primitive_index_base - elements.primitive_indices.start;
				level.triangle_base = block.triangles.start + level.triangle_base - elements.triangles.start;
			}
			levels.block = Some(PrimitiveBlock { ranges: block, lods });
		}
		Ok(copies)
	}
}
