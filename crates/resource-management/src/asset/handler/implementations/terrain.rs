//! Terrain asset baking: a `.terrain.bead` file and its height map become a [`Terrain`] and one mesh per node.

use std::fmt;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset, ReferenceModel,
	processors::{
		mesh::{MeshLodSettings, MeshProcessorSession, MeshSettings, ProcessedMesh},
		terrain::{HeightField, NodeGeometry, TerrainLayout, TerrainPlan, TerrainSpace, decode_height_map_in, plan_terrain},
	},
	resources::{
		material::VariantModel,
		terrain::{TERRAIN_NODES_FRAGMENT_PREFIX, Terrain, TerrainNode, TerrainNodeKey, terrain_node_mesh_id},
	},
	types::{VertexComponent, VertexSemantics},
};

/// The `TerrainAssetHandler` struct lets an asset manager bake standalone `.terrain.bead` assets into terrains whose
/// nodes load one at a time.
///
/// Register it with [`AssetManager::add_asset_handler`](crate::asset::manager::AssetManager::add_asset_handler), then
/// request the terrain by its file's ID, or one node's mesh by
/// [`terrain_node_mesh_id`].
#[derive(Default)]
pub struct TerrainAssetHandler;

impl AssetHandler for TerrainAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type.eq_ignore_ascii_case("terrain.bead")
	}

	/// Reads the terrain asset and its height map, measures the whole node tree, and stores either every node's mesh and
	/// the terrain, or the one node's mesh a fragment names.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
		let terrain_id = url.get_base();
		let request = match url.get_fragment() {
			None => None,
			Some(fragment) => Some(parse_node_fragment(fragment.as_ref()).ok_or_else(|| {
				context.error(format_args!(
					"Terrain resource '{}' doesn't exist. The most likely cause is a fragment other than `nodes/<level>/<column>/<row>`. Request the terrain by its file's ID, or a node by the mesh ID its `Terrain` lists. See {}.",
					url.as_ref(),
					crate::online_docs_url(RESOURCES_DOCS_PATH)
				));
				LoadErrors::AssetCouldNotBeLoaded
			})?),
		};

		let manifest = context.resolve(ResourceId::new(terrain_id.as_ref())).await?;
		let manifest = parse_manifest(&manifest).map_err(|cause| {
			context.error(format_args!(
				"Terrain asset '{}' is invalid. The most likely cause is {cause}. Fix the field as the terrain asset fields describe. See {}.",
				terrain_id.as_ref(),
				crate::online_docs_url(ASSET_DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		})?;

		let material = context.bake_dependency::<VariantModel>(&manifest.material).await?;
		let encoded = context.resolve(ResourceId::new(&manifest.height_map)).await.inspect_err(|error| {
			if error.blames_source() {
				context.error(format_args!(
					"Height map '{}' of terrain '{}' couldn't be read. The most likely cause is a `height_map` ID that doesn't name a file below the assets directory. Set `height_map` to the image's path relative to that directory. See {}.",
					manifest.height_map,
					terrain_id.as_ref(),
					crate::online_docs_url(ASSET_DOCS_PATH)
				));
			}
		})?;
		let mut height_map = decode_height_map_in(encoded.as_slice(), context.allocator()).map_err(|error| {
			context.error(format_args!(
				"Height map '{}' of terrain '{}' can't be used. {error} See {}.",
				manifest.height_map,
				terrain_id.as_ref(),
				crate::online_docs_url(HEIGHT_MAP_DOCS_PATH)
			));
			LoadErrors::FailedToProcess
		})?;
		drop(encoded);

		let field = HeightField::from_height_map(&mut height_map, manifest.heights);
		let layout = TerrainLayout::new(field.extent(), manifest.tile_size);
		let plan = plan_terrain(&field, &layout, manifest.spacing);
		let nodes = NodeMeshes {
			terrain_id: terrain_id.as_ref(),
			field: &field,
			layout: &layout,
			plan: &plan,
			space: TerrainSpace::new(field.extent(), manifest.spacing),
			material: &material,
		};
		// Mesh processing errors come back as text, since the bake context can't cross to the threads that build them.
		let report = |error: String| {
			context.error(error);
			LoadErrors::FailedToProcess
		};

		if let Some(key) = request {
			if layout.index(key).is_none() {
				context.error(format_args!(
					"Terrain node '{}' doesn't exist. The most likely cause is a level, column, or row outside the terrain's node tree, which changes when its height map or `tile_size` changes. Request the mesh IDs the baked `Terrain` lists. See {}.",
					url.as_ref(),
					crate::online_docs_url(NODE_TREE_DOCS_PATH)
				));
				return Err(LoadErrors::AssetCouldNotBeLoaded);
			}
			let (node, mesh) = nodes.process(&mut NodeGeometry::new(), key).map_err(report)?;
			let resource = ProcessedAsset::new(ResourceId::new(&node.mesh), mesh.mesh).with_streams(mesh.stream_descriptions);
			return context.store_primary_owned(resource, mesh.buffer).await;
		}

		let keys = layout.keys().collect::<Vec<_>>();
		let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
		let mut terrain_nodes = Vec::with_capacity(keys.len());
		// Batches bound how many finished meshes wait in memory for storage.
		for batch in keys.chunks(threads * NODES_PER_THREAD) {
			for result in nodes.process_all(batch, threads) {
				let (node, mesh) = result.map_err(report)?;
				store_mesh(context, &node.mesh, mesh).await?;
				terrain_nodes.push(node);
			}
		}
		let terrain = Terrain {
			samples: field.extent(),
			spacing: manifest.spacing,
			heights: manifest.heights,
			tile_size: manifest.tile_size,
			levels: layout.levels().to_vec(),
			nodes: terrain_nodes,
		};
		context.store_primary(ProcessedAsset::new(url, terrain), &[]).await
	}
}

/// How many node meshes each thread builds before the batch is stored.
const NODES_PER_THREAD: usize = 4;

/// Stores a node's processed mesh under `id`, for a bake of the whole terrain.
async fn store_mesh(context: BakeContext<'_>, id: &str, mesh: ProcessedMesh) -> Result<(), LoadErrors> {
	let resource = ProcessedAsset::new(ResourceId::new(id), mesh.mesh).with_streams(mesh.stream_descriptions);
	context.store_resource_owned(resource, mesh.buffer).await.map(|_| ())
}

/// The `NodeMeshes` struct holds what every node's mesh shares, so the threads that build meshes take only keys.
struct NodeMeshes<'a> {
	terrain_id: &'a str,
	field: &'a HeightField<'a>,
	layout: &'a TerrainLayout,
	plan: &'a TerrainPlan,
	space: TerrainSpace,
	material: &'a ReferenceModel<VariantModel>,
}

impl NodeMeshes<'_> {
	/// Builds the meshes of `keys` on up to `threads` threads, and returns each with its node, in `keys` order.
	fn process_all(&self, keys: &[TerrainNodeKey], threads: usize) -> Vec<Result<(TerrainNode, ProcessedMesh), String>> {
		let chunk = keys.len().div_ceil(threads.max(1));
		std::thread::scope(|scope| {
			let workers = keys
				.chunks(chunk)
				.map(|keys| {
					scope.spawn(move || {
						let mut geometry = NodeGeometry::new();
						keys.iter().map(|&key| self.process(&mut geometry, key)).collect::<Vec<_>>()
					})
				})
				.collect::<Vec<_>>();
			workers
				.into_iter()
				.flat_map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
				.collect()
		})
	}

	/// Builds `key`'s mesh in `geometry`'s storage, and returns it with the node as the terrain lists it, or the text
	/// of the error that stopped it.
	fn process(&self, geometry: &mut NodeGeometry, key: TerrainNodeKey) -> Result<(TerrainNode, ProcessedMesh), String> {
		let id = terrain_node_mesh_id(self.terrain_id, key);
		let skirt_depth = self.plan.skirt_depths[key.level as usize];
		geometry.build(self.field, self.layout, self.space, key, skirt_depth);

		let settings = MeshSettings {
			// Neighbors meet only where their edges stay put at every detail level.
			lods: MeshLodSettings {
				lock_borders: true,
				..MeshLodSettings::default()
			},
			lightmap_uvs: None,
		};
		let mut session = MeshProcessorSession::new(node_vertex_layout(), None, Vec::new(), settings);
		for primitive in geometry.primitives() {
			session.push_primitive(primitive).map_err(|error| {
				format!(
					"Terrain node '{id}' couldn't be processed into a mesh: {error:?}. The most likely cause is a `tile_size` whose nodes exceed the mesh limits. Lower `tile_size`. See {}.",
					crate::online_docs_url(ASSET_DOCS_PATH)
				)
			})?;
		}
		let mesh = session.finish(std::slice::from_ref(self.material));

		let (min, max) = self.layout.bounds(self.space, key, self.plan);
		let error = self.plan.nodes[self.layout.index(key).expect("the key belongs to the layout")].error;
		Ok((
			TerrainNode {
				mesh: id,
				min,
				max,
				error,
			},
			mesh,
		))
	}
}

/// Returns the vertex streams every node mesh holds.
fn node_vertex_layout() -> Vec<VertexComponent> {
	[
		VertexSemantics::Position,
		VertexSemantics::Normal,
		VertexSemantics::Tangent,
		VertexSemantics::UV,
	]
	.map(VertexComponent::canonical)
	.to_vec()
}

/// Reads `nodes/<level>/<column>/<row>`, or returns `None` for any other fragment.
fn parse_node_fragment(fragment: &str) -> Option<TerrainNodeKey> {
	let mut parts = fragment.strip_prefix(TERRAIN_NODES_FRAGMENT_PREFIX)?.split('/');
	let mut next = || parts.next()?.parse::<u32>().ok();
	let key = TerrainNodeKey {
		level: next()?,
		column: next()?,
		row: next()?,
	};
	parts.next().is_none().then_some(key)
}

/// The `TerrainManifest` struct is the authored form of a terrain asset, with its defaults filled in.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TerrainManifest {
	#[serde(rename = "$schema")]
	_schema: Option<String>,
	height_map: String,
	material: String,
	#[serde(default = "default_spacing")]
	spacing: f32,
	#[serde(default = "default_heights")]
	heights: [f32; 2],
	#[serde(default = "default_tile_size")]
	tile_size: u32,
}

fn default_spacing() -> f32 {
	1.0
}

fn default_heights() -> [f32; 2] {
	[0.0, 100.0]
}

fn default_tile_size() -> u32 {
	64
}

/// The `ManifestError` enum names the field of a terrain asset that is wrong, to complete a bake error's cause.
#[derive(Debug, PartialEq)]
enum ManifestError {
	Syntax(String),
	Spacing,
	Heights,
	TileSize,
}

impl fmt::Display for ManifestError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Syntax(error) => write!(formatter, "a missing, unknown, or mistyped field: {error}"),
			Self::Spacing => formatter.write_str("a `spacing` that isn't a finite number greater than zero"),
			Self::Heights => formatter.write_str("`heights` that aren't two finite numbers, the first lower than the second"),
			Self::TileSize => formatter.write_str("a `tile_size` that isn't a power of two from 4 through 128"),
		}
	}
}

/// Reads a terrain asset and checks every value against its range.
fn parse_manifest(manifest: &[u8]) -> Result<TerrainManifest, ManifestError> {
	let text = std::str::from_utf8(manifest).map_err(|error| ManifestError::Syntax(error.to_string()))?;
	let manifest: TerrainManifest = json5::from_str(text).map_err(|error| ManifestError::Syntax(error.to_string()))?;
	if !(manifest.spacing.is_finite() && manifest.spacing > 0.0) {
		return Err(ManifestError::Spacing);
	}
	let [low, high] = manifest.heights;
	if !(low.is_finite() && high.is_finite() && low < high) {
		return Err(ManifestError::Heights);
	}
	if !(manifest.tile_size.is_power_of_two() && (MIN_TILE_SIZE..=MAX_TILE_SIZE).contains(&manifest.tile_size)) {
		return Err(ManifestError::TileSize);
	}
	Ok(manifest)
}

/// The fewest quads along a node's side.
const MIN_TILE_SIZE: u32 = 4;

/// The most quads along a node's side: 129 by 129 vertices is the largest square grid within a primitive's 65,536.
const MAX_TILE_SIZE: u32 = 128;

#[cfg(test)]
mod tests {
	use super::TerrainAssetHandler;
	use crate::{
		asset::{
			ResourceId,
			handler::implementations::bema::{BEMAAssetHandler, tests::MinimalTestShaderGenerator},
			manager::AssetManager,
			storage_backend::tests::TestStorageBackend,
		},
		r#async,
		processors::terrain::height_map::tests::grayscale_png,
		resource::storage_backend::MemoryStorageBackend,
		resources::{
			mesh::MeshModel,
			terrain::{Terrain, TerrainNodeKey},
		},
	};

	/// Returns asset storage that holds a 9 by 5 terrain with 4-quad nodes, a material, and the terrain asset `bead`.
	fn sources(bead: &str) -> TestStorageBackend {
		let storage = TestStorageBackend::new();
		let samples = (0..45).map(|sample| (sample * 1_000) as u16).collect::<Vec<_>>();
		storage.add_file("hills.png", &grayscale_png(9, 5, png::BitDepth::Sixteen, &samples));
		storage.add_file(
			"ground_material.bema",
			br#"{ "domain": "World", "graph": "ground.bemx", "variables": [] }"#,
		);
		storage.add_file("ground.bemx", br#"{ nodes: [{ name: "surface", node: "surface" }] }"#);
		storage.add_file("ground.bema", br#"{ "parent": "ground_material.bema", "variables": [] }"#);
		storage.add_file("hills.terrain.bead", bead.as_bytes());
		storage
	}

	const TERRAIN: &str = r#"{
		"$schema": "byte_engine/schemas/terrain.bead.schema.json",
		height_map: "hills.png",
		material: "ground.bema",
		spacing: 2,
		heights: [-10, 50],
		tile_size: 4,
	}"#;

	fn asset_manager(sources: TestStorageBackend, resources: MemoryStorageBackend) -> AssetManager {
		let mut asset_manager = AssetManager::new(sources, resources);
		asset_manager.set_program_generator("World", MinimalTestShaderGenerator);
		asset_manager.add_asset_handler(BEMAAssetHandler);
		asset_manager.add_asset_handler(TerrainAssetHandler);
		asset_manager
	}

	/// Returns the IDs of every stored node mesh, sorted.
	fn stored_meshes(resources: &MemoryStorageBackend) -> Vec<String> {
		let mut meshes = resources
			.get_resources()
			.into_iter()
			.map(|resource| resource.id().to_string())
			.filter(|id| id.contains("#nodes/"))
			.collect::<Vec<_>>();
		meshes.sort();
		meshes
	}

	#[r#async::test]
	async fn a_terrain_bakes_its_node_tree_and_every_node_mesh() {
		let resources = MemoryStorageBackend::new();
		let asset_manager = asset_manager(sources(TERRAIN), resources);

		asset_manager.bake("hills.terrain.bead").await.expect("the terrain must bake");

		let stored = asset_manager
			.memory_storage()
			.get_resource(ResourceId::new("hills.terrain.bead"))
			.unwrap();
		let terrain: Terrain = crate::from_slice(&stored.resource).unwrap();
		assert_eq!(stored.class, "Terrain");
		assert_eq!(terrain.samples, [9, 5]);
		assert_eq!(terrain.heights, [-10.0, 50.0]);
		assert_eq!(terrain.levels.len(), 2);
		let meshes = terrain.nodes.iter().map(|node| node.mesh.as_str()).collect::<Vec<_>>();
		assert_eq!(
			meshes,
			[
				"hills.terrain.bead#nodes/0/0/0",
				"hills.terrain.bead#nodes/0/1/0",
				"hills.terrain.bead#nodes/1/0/0",
			]
		);
		// The root spans the whole map: 16 by 8 meters around the center, from the lowest sample down its skirt.
		let root = terrain.node(terrain.root()).unwrap();
		assert_eq!([root.min[0], root.min[2], root.max[0], root.max[2]], [-8.0, -4.0, 8.0, 4.0]);
		assert_eq!(root.max[1], -10.0 + 44_000.0 / 65_535.0 * 60.0);
		assert!(root.min[1] <= -12.0);

		assert_eq!(stored_meshes(asset_manager.memory_storage()), meshes);
		for mesh in meshes {
			let mesh: MeshModel = crate::from_slice(
				&asset_manager
					.memory_storage()
					.get_resource(ResourceId::new(mesh))
					.unwrap()
					.resource,
			)
			.unwrap();
			assert_eq!(mesh.primitives.len(), 2);
			assert_eq!(mesh.materials[0].id().as_ref(), "ground.bema");
		}
	}

	#[r#async::test]
	async fn a_node_request_bakes_only_that_node() {
		let resources = MemoryStorageBackend::new();
		let asset_manager = asset_manager(sources(TERRAIN), resources);

		asset_manager
			.bake("hills.terrain.bead#nodes/0/1/0")
			.await
			.expect("the node must bake");

		assert_eq!(
			stored_meshes(asset_manager.memory_storage()),
			["hills.terrain.bead#nodes/0/1/0"]
		);
		assert!(
			asset_manager
				.memory_storage()
				.get_resource(ResourceId::new("hills.terrain.bead"))
				.is_none()
		);
	}

	#[r#async::test]
	async fn fragments_that_name_no_node_fail() {
		for fragment in ["nodes/0/2/0", "nodes/2/0/0", "nodes/0/0", "nodes/0/0/0/0", "image"] {
			let resources = MemoryStorageBackend::new();
			let asset_manager = asset_manager(sources(TERRAIN), resources);

			let result = asset_manager.bake(&format!("hills.terrain.bead#{fragment}")).await;

			assert!(result.is_err(), "`{fragment}` must fail");
			assert!(stored_meshes(asset_manager.memory_storage()).is_empty());
		}
	}

	#[r#async::test]
	async fn fields_outside_their_range_fail_the_bake() {
		let fields = [
			"spacing: 0",
			"heights: [5, 5]",
			"tile_size: 48",
			"tile_size: 256",
			"tile_size: 2",
			"levels: 3",
		];
		for field in fields {
			let bead = format!(r#"{{ height_map: "hills.png", material: "ground.bema", {field} }}"#);
			let asset_manager = asset_manager(sources(&bead), MemoryStorageBackend::new());

			assert!(asset_manager.bake("hills.terrain.bead").await.is_err(), "`{field}` must fail");
		}
	}

	#[test]
	fn node_fragments_name_a_level_column_and_row() {
		assert_eq!(
			super::parse_node_fragment("nodes/3/14/15"),
			Some(TerrainNodeKey {
				level: 3,
				column: 14,
				row: 15
			})
		);
		assert_eq!(super::parse_node_fragment("nodes/3/14/-1"), None);
	}
}

const ASSET_DOCS_PATH: &str = "reference/terrain#terrain-assets";
const HEIGHT_MAP_DOCS_PATH: &str = "reference/terrain#height-maps";
const NODE_TREE_DOCS_PATH: &str = "reference/terrain#node-tree";
const RESOURCES_DOCS_PATH: &str = "reference/terrain#resources";

use crate::asset::handler::PrimaryStored;
