//! Loader-thread mesh residency and shared scene storage for the Simple pipeline.
//!
//! Use this module as the smallest concrete guide for implementing
//! [`crate::rendering::loading`] in another renderer. Simple supports
//! generated meshes and baked static meshes. It consumes only `vec3f` positions
//! and triangle indices, converts indices to one mesh-local `u16` stream, and
//! has no texture path. These are renderer capabilities, not restrictions of
//! the shared loader.
//!
//! # From scene request to residency
//!
//! [`super::PipelineManager::request_mesh`] derives a renderer-independent
//! [`MeshKey`], coalesces it through [`SimpleLoader`], and retains each scene
//! instance as pending. A loader lane resolves the owned [`MeshSource`], writes
//! Simple's streams into a [`StagingLease`], reserves renderer-owned buffer
//! offsets, and waits for the loader to copy them before reporting
//! [`ResidentSimpleMesh`]. The manager then creates every pending instance for
//! that key.
//!
//! # Adapt this example
//!
//! Replace [`MeshSource`] and the private prepared value with the inputs and
//! transfer layout your renderer needs. Keep GPU placement inside the loader
//! integration and publish only values whose transfers have completed. Wire the
//! resulting manager during application startup as shown by
//! [`crate::application::graphics::setup_simple_render_pipeline`].

use std::{
	ops::Range,
	sync::{Arc, Mutex},
};

use ghi::context::ContextCreate as _;
use resource_management::{
	Reference,
	resource::ReadTargets,
	resources::mesh::Mesh as ResourceMesh,
	types::{IndexStreamTypes, Streams, VertexSemantics},
};
use smallvec::smallvec;

use crate::{
	core::EntityHandle,
	rendering::{
		loading::{BufferRegion, LoadError, LoadPipeline, Loader, LoaderClient, LoaderLane, spawn},
		mesh::generator::{GeneratedIndexError, MeshGenerator, validate_triangle_indices},
		renderable::mesh::{MeshKey, MeshSource},
		resource_loading::{StagingLease, UploadStagingArena},
	},
};

pub(super) const SIMPLE_VERTEX_CAPACITY: usize = 1024 * 1024;
pub(super) const SIMPLE_INDEX_CAPACITY: usize = 1024 * 1024;
const POSITION_STRIDE: usize = std::mem::size_of::<(f32, f32, f32)>();
const INDEX_STRIDE: usize = std::mem::size_of::<u16>();
const SIMPLE_LOADER_LANE_COUNT: usize = 1;
const SIMPLE_LOADER_RESULT_CAPACITY: usize = 64;

/// The `PreparedSimpleMesh` struct keeps Simple-formatted transfer bytes alive until its lane finishes the copies.
///
/// The staging ranges are relative to the lease. The loader retains this value
/// until the GPU stops reading it, then returns its [`StagingLease`]
/// automatically.
struct PreparedSimpleMesh {
	staging: StagingLease,
	positions: Range<usize>,
	indices: Range<usize>,
	vertex_count: usize,
	index_count: usize,
}

/// The `ResidentSimpleMesh` struct locates geometry whose loader transfer has completed, so scene instances can draw it.
///
/// Distinct meshes never share a range, so equal values name the same mesh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResidentSimpleMesh {
	pub(super) index_count: usize,
	pub(super) base_vertex: usize,
	pub(super) base_index: usize,
}

/// The `SimpleLoader` struct owns all resource loading and GPU placement for the Simple pipeline.
pub(crate) struct SimpleLoader {
	resources: EntityHandle<resource_management::ResourceManager>,
	store: SharedSimpleResourceStore,
	/// The store's position buffer, as the loader context imported it.
	vertex_positions: ghi::BaseBufferHandle,
	/// The store's index buffer, as the loader context imported it.
	indices: ghi::BaseBufferHandle,
}

pub(crate) type SimpleLoaderClient = LoaderClient<SimpleLoader>;
pub(crate) type SimpleLoaderLane = LoaderLane<SimpleLoader>;
pub(crate) type SharedSimpleResourceStore = Arc<Mutex<SimpleResourceStore>>;

impl SimpleLoader {
	/// Creates the Simple pipeline's loader client and lane.
	///
	/// `render` is the context that created `store`'s buffers. The loader imports them so its copies append to
	/// them. Run the returned lane on the loading thread.
	pub(crate) fn spawn(
		loader: &mut Loader,
		render: &ghi::implementation::Context,
		resources: EntityHandle<resource_management::ResourceManager>,
		store: SharedSimpleResourceStore,
	) -> (SimpleLoaderClient, Vec<SimpleLoaderLane>) {
		let (vertex_positions, indices) = {
			let store = store.lock().unwrap_or_else(|error| error.into_inner());
			(store.vertex_positions_buffer, store.indices_buffer)
		};
		let vertex_positions = loader.import_buffer(render.share_buffer(vertex_positions)).into();
		let indices = loader.import_buffer(render.share_buffer(indices)).into();
		spawn(
			loader,
			Self {
				resources,
				store,
				vertex_positions,
				indices,
			},
			SIMPLE_LOADER_LANE_COUNT,
			SIMPLE_LOADER_RESULT_CAPACITY,
		)
	}
}

impl LoadPipeline for SimpleLoader {
	type Key = MeshKey;
	type Request = MeshSource;
	type Resident = ResidentSimpleMesh;

	fn key(request: &Self::Request) -> Self::Key {
		request.key()
	}

	/// Resolves, converts, places, and uploads one mesh before publishing it.
	async fn load(&self, request: MeshSource, lane: &mut LoaderLane<Self>) -> Result<ResidentSimpleMesh, LoadError> {
		let staging = lane.staging().clone();
		let prepared = match request {
			MeshSource::Generated(generator) => prepare_generated_mesh(generator.as_ref(), staging).await,
			MeshSource::Resource(id) => {
				let resource = self
					.resources
					.request::<ResourceMesh>(&id)
					.await
					.map_err(|source| LoadError(SimpleMeshError::ResourceRequest { id, source }.to_string()))?;
				prepare_resource_mesh(resource, staging).await
			}
		}
		.map_err(|error| LoadError(error.to_string()))?;

		let mesh = self
			.store
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.reserve_mesh(prepared.vertex_count, prepared.index_count)
			.map_err(|error| LoadError(error.to_string()))?;
		let regions = smallvec![
			BufferRegion {
				offset: prepared.positions.start,
				destination: self.vertex_positions,
				destination_offset: mesh.base_vertex * POSITION_STRIDE,
				size: prepared.positions.len(),
			},
			BufferRegion {
				offset: prepared.indices.start,
				destination: self.indices,
				destination_offset: mesh.base_index * INDEX_STRIDE,
				size: prepared.indices.len(),
			},
		];
		lane.upload(prepared.staging, [], regions).await?;
		Ok(mesh)
	}
}

/// The `SimpleResourceStore` struct centralizes Simple's GPU placement policy for the loader lanes.
///
/// Simple chooses fixed, append-only parallel position and index buffers. Loader
/// lanes reserve mesh ranges here; scene instances belong to the pipeline manager.
pub(crate) struct SimpleResourceStore {
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[[f32; 3]; SIMPLE_VERTEX_CAPACITY]>,
	pub(super) indices_buffer: ghi::BufferHandle<[u16; SIMPLE_INDEX_CAPACITY]>,
	/// The first free vertex of the position buffer.
	vertex_cursor: usize,
	/// The first free index of the index buffer.
	index_cursor: usize,
}

impl SimpleResourceStore {
	/// Creates fixed renderer-owned geometry buffers and empty allocation state.
	///
	/// Share this store between the loader and pipeline manager.
	pub(crate) fn new(context: &mut ghi::implementation::Context) -> Self {
		let vertex_positions_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::TransferDestination)
				.name("Vertex Positions")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let indices_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::TransferDestination)
				.name("Indices")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		Self {
			vertex_positions_buffer,
			indices_buffer,
			vertex_cursor: 0,
			index_cursor: 0,
		}
	}

	/// Reserves room for one mesh in Simple's parallel position and index streams.
	///
	/// Both capacities are checked before either cursor moves. The loader publishes the mesh only after its
	/// copies into the returned ranges completed.
	fn reserve_mesh(&mut self, vertex_count: usize, index_count: usize) -> Result<ResidentSimpleMesh, SimpleMeshError> {
		let vertex_end = self
			.vertex_cursor
			.checked_add(vertex_count)
			.filter(|end| *end <= SIMPLE_VERTEX_CAPACITY)
			.ok_or(SimpleMeshError::VertexCapacity)?;
		let index_end = self
			.index_cursor
			.checked_add(index_count)
			.filter(|end| *end <= SIMPLE_INDEX_CAPACITY)
			.ok_or(SimpleMeshError::IndexCapacity)?;
		let mesh = ResidentSimpleMesh {
			index_count,
			base_vertex: self.vertex_cursor,
			base_index: self.index_cursor,
		};
		self.vertex_cursor = vertex_end;
		self.index_cursor = index_end;
		Ok(mesh)
	}
}

// Define each static failure and its recovery hint once while retaining a typed error API.
macro_rules! simple_mesh_errors {
	($($variant:ident => $message:literal),+ $(,)?) => {
		/// Errors produced while preparing or storing one simple-pipeline mesh.
		#[derive(Debug)]
		pub(crate) enum SimpleMeshError {
			ResourceRequest { id: Arc<str>, source: resource_management::RequestError },
			$($variant),+
		}

		impl std::fmt::Display for SimpleMeshError {
			fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				match self {
					Self::ResourceRequest { id, source } => write!(formatter, "Could not load simple mesh '{id}'. {source}"),
					$(Self::$variant => formatter.write_str($message)),+
				}
			}
		}
	};
}

simple_mesh_errors! {
	MissingPositionStream => "Simple mesh has no position stream. The most likely cause is that the resource was baked without vertex positions.",
	MissingIndexStream => "Simple mesh has no triangle-index stream. The most likely cause is that the resource was baked without triangle indices.",
	InvalidPositionStream => "Simple mesh position data is invalid. The most likely cause is malformed position stream metadata.",
	InvalidIndexStream => "Simple mesh index data is invalid. The most likely cause is malformed triangle-index stream metadata.",
	InvalidPrimitiveLayout => "Simple mesh primitive layout is invalid. The most likely cause is overlapping or incomplete baked primitive ranges.",
	UnsupportedMeshFeatures => "Simple mesh uses unsupported geometry features. The most likely cause is a transformed, skinned, or quantized baked primitive.",
	IndexOutOfRange => "Simple mesh index is outside its primitive. The most likely cause is corrupt generated or baked triangle data.",
	IndexLimit => "Simple mesh exceeds the u16 index limit. The most likely cause is geometry containing more than 65,536 addressable vertices.",
	StagingCapacity => "Simple mesh exceeds upload staging capacity. The most likely cause is geometry larger than the configured asynchronous upload arena.",
	ResourceLoad => "Simple mesh bytes could not be loaded. The most likely cause is missing, compressed, or corrupt resource payload data.",
	VertexCapacity => "Simple vertex storage is full. The most likely cause is resident geometry exceeding the fixed vertex capacity.",
	IndexCapacity => "Simple index storage is full. The most likely cause is resident geometry exceeding the fixed index capacity."
}

impl std::error::Error for SimpleMeshError {}

struct ResourcePrimitiveLayout {
	vertex_offset: usize,
	vertex_count: usize,
	index_range: Range<usize>,
	/// The primitive's position bytes in each level's stream, coarsest level first, which is its vertex order.
	position_slices: Vec<(u8, Range<usize>)>,
}

struct ResourceMeshLayout {
	/// The size of each detail level's position stream, finest first; zero for a level without one.
	level_position_sizes: Vec<usize>,
	position_size: usize,
	index_size: usize,
	primitives: Vec<ResourcePrimitiveLayout>,
}

/// Validates the baked ranges needed to collapse every resource primitive into one Simple draw.
fn resource_mesh_layout(mesh: &ResourceMesh) -> Result<ResourceMeshLayout, SimpleMeshError> {
	let mut position_components = mesh
		.vertex_components
		.iter()
		.filter(|component| component.semantic == VertexSemantics::Position);
	let position_component = position_components.next().ok_or(SimpleMeshError::InvalidPositionStream)?;
	if position_component.channel != 0 || position_component.format != "vec3f" || position_components.next().is_some() {
		return Err(SimpleMeshError::InvalidPositionStream);
	}
	if mesh.skeleton.is_some()
		|| !mesh.skins.is_empty()
		|| mesh
			.primitives
			.iter()
			.any(|primitive| primitive.transform_node.is_some() || primitive.skin.is_some())
	{
		return Err(SimpleMeshError::UnsupportedMeshFeatures);
	}
	// Simple draws full detail, which needs every level's vertices: each level stores the vertices it adds.
	let position_type = Streams::Vertices(VertexSemantics::Position);
	let level_positions = (0..mesh.level_count())
		.map(|level| mesh.stream(position_type, level))
		.collect::<Vec<_>>();
	if level_positions.iter().all(Option::is_none) {
		return Err(SimpleMeshError::MissingPositionStream);
	}
	let indices = mesh
		.stream(Streams::Indices(IndexStreamTypes::Triangles), 0)
		.ok_or(SimpleMeshError::MissingIndexStream)?;
	if level_positions
		.iter()
		.flatten()
		.any(|positions| positions.stride != POSITION_STRIDE || positions.size % POSITION_STRIDE != 0)
	{
		return Err(SimpleMeshError::InvalidPositionStream);
	}
	if indices.stride != INDEX_STRIDE || indices.size % (INDEX_STRIDE * 3) != 0 {
		return Err(SimpleMeshError::InvalidIndexStream);
	}
	let level_position_sizes = level_positions
		.iter()
		.map(|positions| positions.map_or(0, |positions| positions.size))
		.collect::<Vec<_>>();
	let position_size = level_position_sizes.iter().sum::<usize>();
	let vertex_count = position_size / POSITION_STRIDE;
	if vertex_count > usize::from(u16::MAX) + 1 {
		return Err(SimpleMeshError::IndexLimit);
	}

	// Each level's streams list the primitives in order, so every primitive's slice must start where the previous
	// primitive's slice of that level ended.
	let mut expected_level_offsets = vec![0usize; level_positions.len()];
	let mut expected_vertex_offset = 0usize;
	let mut expected_index_offset = 0usize;
	let mut primitives = Vec::with_capacity(mesh.primitives.len());
	for primitive in &mesh.primitives {
		let primitive_indices = primitive
			.stream(Streams::Indices(IndexStreamTypes::Triangles), 0)
			.ok_or(SimpleMeshError::InvalidPrimitiveLayout)?;
		let mut position_slices = Vec::with_capacity(primitive.lods.len());
		let mut primitive_vertex_size = 0usize;
		for level in (0..primitive.lods.len() as u8).rev() {
			let Some(slice) = primitive.stream(position_type, level) else {
				continue;
			};
			let expected_offset = expected_level_offsets
				.get_mut(level as usize)
				.ok_or(SimpleMeshError::InvalidPrimitiveLayout)?;
			if slice.offset != *expected_offset || slice.stride != POSITION_STRIDE || slice.size % POSITION_STRIDE != 0 {
				return Err(SimpleMeshError::InvalidPrimitiveLayout);
			}
			*expected_offset += slice.size;
			primitive_vertex_size += slice.size;
			position_slices.push((level, slice.offset..slice.offset + slice.size));
		}
		if primitive_vertex_size != primitive.vertex_count as usize * POSITION_STRIDE
			|| primitive_indices.offset != expected_index_offset
			|| primitive_indices.stride != INDEX_STRIDE
			|| primitive_indices.size % (INDEX_STRIDE * 3) != 0
		{
			return Err(SimpleMeshError::InvalidPrimitiveLayout);
		}
		let index_end = primitive_indices
			.offset
			.checked_add(primitive_indices.size)
			.ok_or(SimpleMeshError::InvalidPrimitiveLayout)?;
		primitives.push(ResourcePrimitiveLayout {
			vertex_offset: expected_vertex_offset / POSITION_STRIDE,
			vertex_count: primitive.vertex_count as usize,
			index_range: primitive_indices.offset..index_end,
			position_slices,
		});
		expected_vertex_offset += primitive_vertex_size;
		expected_index_offset = index_end;
	}
	if expected_level_offsets != level_position_sizes || expected_index_offset != indices.size {
		return Err(SimpleMeshError::InvalidPrimitiveLayout);
	}

	Ok(ResourceMeshLayout {
		level_position_sizes,
		position_size,
		index_size: indices.size,
		primitives,
	})
}

/// Builds the exact Simple staging layout from generated geometry.
async fn prepare_generated_mesh(
	generator: &dyn MeshGenerator,
	staging: Arc<UploadStagingArena>,
) -> Result<PreparedSimpleMesh, SimpleMeshError> {
	let positions = generator.positions();
	let source_indices = generator.indices();
	validate_triangle_indices(&source_indices, positions.len()).map_err(|error| match error {
		GeneratedIndexError::NotTriangles => SimpleMeshError::InvalidIndexStream,
		GeneratedIndexError::OutOfRange { .. } => SimpleMeshError::IndexOutOfRange,
		GeneratedIndexError::IndexLimit { .. } => SimpleMeshError::IndexLimit,
	})?;
	// Simple draws one mesh-local index stream, so the whole mesh must fit in 16-bit indices.
	if positions.len() > usize::from(u16::MAX) + 1 {
		return Err(SimpleMeshError::IndexLimit);
	}
	let position_size = positions
		.len()
		.checked_mul(POSITION_STRIDE)
		.ok_or(SimpleMeshError::StagingCapacity)?;
	let index_size = source_indices
		.len()
		.checked_mul(INDEX_STRIDE)
		.ok_or(SimpleMeshError::StagingCapacity)?;
	let index_start = position_size.next_multiple_of(4);
	let byte_count = index_start.checked_add(index_size).ok_or(SimpleMeshError::StagingCapacity)?;
	let mut staging = staging
		.allocate(byte_count, 256)
		.await
		.ok_or(SimpleMeshError::StagingCapacity)?;
	let bytes = staging.bytes_mut();
	for (destination, &(x, y, z)) in bytes[..position_size]
		.as_chunks_mut::<POSITION_STRIDE>()
		.0
		.iter_mut()
		.zip(positions.iter())
	{
		*destination = bytemuck::cast([x, y, z]);
	}
	for (destination, &index) in bytes[index_start..][..index_size]
		.as_chunks_mut::<INDEX_STRIDE>()
		.0
		.iter_mut()
		.zip(source_indices.iter())
	{
		destination.copy_from_slice(&(index as u16).to_ne_bytes());
	}

	Ok(PreparedSimpleMesh {
		staging,
		positions: 0..position_size,
		indices: index_start..index_start + index_size,
		vertex_count: positions.len(),
		index_count: source_indices.len(),
	})
}

/// Loads and converts only the baked streams consumed by the Simple renderer.
///
/// Every level's position stream loads into the front of the staging lease. Each primitive's slices are then gathered
/// behind them in its vertex order, so the copied positions list every primitive's vertices one after another.
async fn prepare_resource_mesh(
	mut resource: Reference<ResourceMesh>,
	staging: Arc<UploadStagingArena>,
) -> Result<PreparedSimpleMesh, SimpleMeshError> {
	let layout = resource_mesh_layout(resource.resource())?;
	let mut cursor = 0usize;
	let level_sources = layout
		.level_position_sizes
		.iter()
		.map(|&size| {
			let start = cursor.next_multiple_of(4);
			cursor = start + size;
			start..cursor
		})
		.collect::<Vec<_>>();
	let position_start = cursor.next_multiple_of(4);
	let index_start = (position_start + layout.position_size).next_multiple_of(4);
	let byte_count = index_start
		.checked_add(layout.index_size)
		.ok_or(SimpleMeshError::StagingCapacity)?;
	let mut staging = staging
		.allocate(byte_count, 256)
		.await
		.ok_or(SimpleMeshError::StagingCapacity)?;

	let stream_names = (0..level_sources.len())
		.map(|level| format!("level[{level}].Vertex.Position"))
		.collect::<Vec<_>>();
	let loaded = {
		let bytes = staging.bytes_mut();
		let (sources, remainder) = bytes.split_at_mut(position_start);
		let indices = &mut remainder[index_start - position_start..][..layout.index_size];
		let mut targets = Vec::with_capacity(level_sources.len() + 1);
		// The allocator walks the same 4-byte aligned ranges `level_sources` reserved.
		let mut allocator = utils::BufferAllocator::new(sources);
		for (name, range) in stream_names.iter().zip(&level_sources) {
			let (offset, target) = allocator.take_with_offset_aligned(range.len(), 4);
			debug_assert_eq!(offset, range.start);
			if !target.is_empty() {
				targets.push(resource_management::stream::StreamMut::new(name, target));
			}
		}
		targets.push(resource_management::stream::StreamMut::new(
			"level[0].TriangleIndices",
			indices,
		));
		resource
			.load(targets.into())
			.await
			.map_err(|_| SimpleMeshError::ResourceLoad)?
	};
	if !matches!(loaded, ReadTargets::Streams(_)) {
		return Err(SimpleMeshError::ResourceLoad);
	}

	let (sources, positions) = staging.bytes_mut().split_at_mut(position_start);
	let mut written = 0;
	for primitive in &layout.primitives {
		for (level, slice) in &primitive.position_slices {
			let source = &sources[level_sources[*level as usize].start..][slice.clone()];
			let destination = &mut positions[written..][..source.len()];
			for (destination, component) in destination
				.as_chunks_mut::<{ std::mem::size_of::<f32>() }>()
				.0
				.iter_mut()
				.zip(source.as_chunks::<{ std::mem::size_of::<f32>() }>().0)
			{
				*destination = f32::from_le_bytes(*component).to_ne_bytes();
			}
			written += source.len();
		}
	}
	rebase_resource_indices(
		&mut staging.bytes_mut()[index_start..][..layout.index_size],
		&layout.primitives,
	)?;

	Ok(PreparedSimpleMesh {
		staging,
		positions: position_start..position_start + layout.position_size,
		indices: index_start..index_start + layout.index_size,
		vertex_count: layout.position_size / POSITION_STRIDE,
		index_count: layout.index_size / INDEX_STRIDE,
	})
}

/// Converts primitive-local baked indices into the one mesh-local stream expected by Simple draws.
fn rebase_resource_indices(indices: &mut [u8], primitives: &[ResourcePrimitiveLayout]) -> Result<(), SimpleMeshError> {
	for primitive in primitives {
		let primitive_indices = indices
			.get_mut(primitive.index_range.clone())
			.ok_or(SimpleMeshError::InvalidPrimitiveLayout)?;
		for encoded in primitive_indices.as_chunks_mut::<INDEX_STRIDE>().0 {
			let local = u16::from_le_bytes(*encoded) as usize;
			if local >= primitive.vertex_count {
				return Err(SimpleMeshError::IndexOutOfRange);
			}
			let rebased = primitive
				.vertex_offset
				.checked_add(local)
				.and_then(|index| u16::try_from(index).ok())
				.ok_or(SimpleMeshError::IndexLimit)?;
			encoded.copy_from_slice(&rebased.to_ne_bytes());
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::mesh::generator::BoxMeshGenerator;

	#[test]
	fn generated_mesh_preparation_writes_simple_positions_and_u16_indices() {
		let mut bytes = vec![0u8; 4096];
		let executor = resource_management::r#async::Executor::new().expect("Simple preparation test executor");
		executor.block_on(async {
			let (staging, worker) = UploadStagingArena::new_for_test(&mut bytes);
			resource_management::r#async::spawn(worker.run()).detach();
			let generator = BoxMeshGenerator::new();
			let mut prepared = prepare_generated_mesh(&generator, staging)
				.await
				.expect("generated Simple mesh");
			let expected_positions = generator.positions();
			let expected_indices = generator.indices();
			let position_range = prepared.positions.clone();
			let index_range = prepared.indices.clone();
			let prepared_bytes = prepared.staging.bytes_mut();

			let actual_positions = prepared_bytes[position_range]
				.as_chunks::<POSITION_STRIDE>()
				.0
				.iter()
				.map(|bytes| {
					let [x, y, z] = bytemuck::pod_read_unaligned::<[f32; 3]>(bytes);
					(x, y, z)
				})
				.collect::<Vec<_>>();
			assert_eq!(actual_positions, expected_positions.as_ref());
			let actual_indices = prepared_bytes[index_range]
				.as_chunks::<INDEX_STRIDE>()
				.0
				.iter()
				.map(|bytes| u16::from_ne_bytes(*bytes))
				.collect::<Vec<_>>();
			assert_eq!(
				actual_indices,
				expected_indices.iter().map(|&index| index as u16).collect::<Vec<_>>()
			);
		});
	}

	#[test]
	fn two_primitive_indices_are_rebased_for_one_simple_draw() {
		let mut indices = [0u16, 1, 2, 0, 1, 2]
			.into_iter()
			.flat_map(u16::to_le_bytes)
			.collect::<Vec<_>>();
		let primitives = [
			ResourcePrimitiveLayout {
				vertex_offset: 0,
				vertex_count: 3,
				index_range: 0..6,
				position_slices: Vec::new(),
			},
			ResourcePrimitiveLayout {
				vertex_offset: 3,
				vertex_count: 3,
				index_range: 6..12,
				position_slices: Vec::new(),
			},
		];

		rebase_resource_indices(&mut indices, &primitives).expect("valid primitive-local indices");

		let actual = indices
			.as_chunks::<INDEX_STRIDE>()
			.0
			.iter()
			.map(|bytes| u16::from_ne_bytes(*bytes))
			.collect::<Vec<_>>();
		assert_eq!(actual, [0, 1, 2, 3, 4, 5]);
		let mut indices = [3u16].into_iter().flat_map(u16::to_le_bytes).collect::<Vec<_>>();
		let primitives = [ResourcePrimitiveLayout {
			vertex_offset: 0,
			vertex_count: 3,
			index_range: 0..2,
			position_slices: Vec::new(),
		}];
		assert!(matches!(
			rebase_resource_indices(&mut indices, &primitives),
			Err(SimpleMeshError::IndexOutOfRange)
		));
	}
}
