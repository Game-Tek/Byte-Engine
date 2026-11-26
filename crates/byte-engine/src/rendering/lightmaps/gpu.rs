//! The GPU half of the lightmap bake: it loads a level's meshes, materials, and environment as the renderer loads them,
//! traces paths from every lightmap texel, and reads the light back.
//!
//! Everything the renderer would do is done with the renderer's own code. The engine's loading timeline uploads the
//! meshes, textures, and environment through [`prepare_mesh_source`], [`load_texture`], and [`load_environment_maps`].
//! The meshes land in [`GeometryHandles`], the scene set holds them as it does in an application, and
//! [`SceneAccelerationStructures`] and [`HitShading`] build the structures and the pipeline that runs each material's
//! closest-hit shader. Only the texels, the trace, and the dilation are the bake's own.

use std::sync::Mutex;
use std::time::Duration;

use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRayTracingPipelineMode as _, CommandBufferRecording as _,
	CommonCommandBufferMode as _,
};
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::queue::{FrameRequest, Queue as _, QueueExecution as _};
use resource_management::Reference;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::{
	image::Image as ResourceImage, lightmap::LightmapInstance, material::Variant as ResourceVariant,
	mesh::Mesh as ResourceMesh,
};
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::Extent;
use utils::hash::{HashMap, HashSet};

use super::atlas::{self, Atlas};
use super::guides::{GuideGroups, group_texels};
use super::scene::LevelScene;
use super::texels::{LightmapTexel, NO_LIGHTMAP_TEXEL, TexelRaster, TexelSurface};
use crate::application::Parameter;
use crate::core::EntityHandle;
use crate::core::factory::Handle;
use crate::rendering::loading::{Event, LoadError, LoadPipeline, Loader, LoaderLane, spawn};
use crate::rendering::pipeline_compilation::{PipelineManager, PipelineState};
use crate::rendering::pipelines::visibility::{
	acceleration_structures::{SceneAccelerationStructures, TracedPrimitive},
	geometry::{
		GeometryBuffers, GeometryCapacity, GeometryCounts, GeometryHandles, MeshData, PreparedLevels, PreparedMesh,
		PreparedSurface, decode_octahedral_unit_vector,
	},
	layout::{MAX_BINDLESS_TEXTURES, MAX_LIGHTMAP_PAGES, MAX_LIGHTS, MAX_MATERIALS},
	loader::{
		EnvironmentMaps, IBL_SPECULAR_LEVEL_COUNT, MaterialDescription, assign_slot, describe_material, load_environment_maps,
		material_sampler, place_mesh, prepare_mesh_source,
	},
	manager::create_fallback_environment,
	material_parameters::MaterialParameters,
	ray_tracing::HitShading,
	set_layouts,
	shader_data::{LightingData, MESH_FLAG_DOUBLE_SIDED, MaterialData, ShaderMesh},
};
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::load_texture;
use crate::rendering::set_layout::{ENGINE_MODULES, SetLayout};
use crate::rendering::{GraphicsDevice, PipelineManagerClient};

/// The ray generation shader that traces the paths.
pub(crate) const LIGHTMAP_TRACE_SHADER: &str = "byte_engine/rendering/lightmaps/lightmap-trace.besl";
/// The pipeline that resolves the paths' light into the image and dilates it.
pub(crate) const LIGHTMAP_DILATE_PIPELINE: &str = "byte_engine/rendering/lightmaps/lightmap-dilate.pipeline";
/// The rings of empty texels the dilation fills around each chart. An even count leaves the result in the image.
const DILATE_PASSES: u32 = 8;
/// The side of the dilation's square workgroups. Matches `workgroup` in `lightmaps/lightmap-dilate.besl.bead`.
const DILATE_WORKGROUP: u32 = 8;
/// One in this many of a texel's paths learn its guide; the rest follow it.
const LEARNING_SHARE: u32 = 8;
/// The width of the cubes of world space whose texels share a guide, in texels.
const GROUP_TEXELS: f32 = 4.0;
/// The cells of a guide, and the values a guide holds. Match `LIGHTMAP_GUIDE_CELLS` and `LIGHTMAP_GUIDE_STRIDE` in
/// `lightmaps/layouts/lightmap_bake.besl`.
const GUIDE_CELLS: usize = 64;
const GUIDE_STRIDE: usize = 72;
/// The most ray segments one trace dispatch casts, so a dispatch ends well within the GPU's watchdog.
const SEGMENTS_PER_BATCH: u64 = 1 << 24;
const LANE_COUNT: usize = 4;
/// The bake needs everything it loads before it traces, so it ranks every request needed and every lane takes them.
const BACKGROUND_LANE_COUNT: usize = 1;
const RESULT_CAPACITY: usize = 64;
/// How long the bake yields to the loading tasks while it waits for them.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// The `TraceSettings` struct is how the bake traces a lightmap, from its `.lightmap.bead` file.
#[derive(Clone, Debug)]
pub(crate) struct TraceSettings {
	pub(crate) samples: u32,
	pub(crate) bounces: u32,
	pub(crate) texel_density: f32,
	pub(crate) max_extent: u32,
	pub(crate) environment: Option<String>,
	pub(crate) illuminance: Option<f32>,
}

/// The `TracedLightmap` struct is a traced lightmap, ready to store.
pub(crate) struct TracedLightmap {
	/// Every page, in order.
	pub(crate) pages: Vec<TracedPage>,
	/// Every lit primitive's page and rectangle, in ascending entity then primitive order.
	pub(crate) instances: Vec<LightmapInstance>,
	/// The factor the texel density shrank by to fit the most pages a lightmap may have, when it had to.
	pub(crate) shrunk: Option<f32>,
	/// How many rectangles were larger than a page and took the largest side a page holds.
	pub(crate) clamped: usize,
	/// Each mesh with opaque primitives left unlit because they have no lightmap UVs, and how many such primitives
	/// the level places.
	pub(crate) unlit: Vec<UnlitMesh>,
}

/// The `TracedPage` struct is one traced page of a lightmap: a square image of RGBA16F texels.
pub(crate) struct TracedPage {
	/// The page's side, in texels.
	pub(crate) extent: u32,
	/// Irradiance over π in RGB, and in alpha the share of paths whose first ray reached the environment, or zero where
	/// the texel holds no light, row after row, as little-endian half floats.
	pub(crate) light: Vec<u8>,
}

/// The `UnlitMesh` struct names a mesh whose opaque primitives the lightmap leaves to the environment's light, so the
/// bake can warn about each mesh once rather than about every instance of it.
pub(crate) struct UnlitMesh {
	/// The mesh's resource ID, or a description of a generated mesh.
	pub(crate) mesh: String,
	/// The placed primitives left unlit, across every entity that places the mesh.
	pub(crate) primitives: u32,
}

/// Traces the lightmap of `scene` with `settings`. The loading timeline reads the baked meshes, materials, and
/// environment through `resources`, and so does the pipeline compiler read the baked shaders.
///
/// Blocks the calling thread until the GPU work finishes, so call it from a thread of its own. Returns the most likely
/// cause of what stopped the bake.
pub(crate) fn trace(resources: ResourceManager, scene: LevelScene, settings: TraceSettings) -> Result<TracedLightmap, String> {
	let device = GraphicsDevice::try_new(
		[
			Parameter::new("debug", "false"),
			Parameter::new("ghi.features.ray-tracing", "true"),
			Parameter::new("ghi.features.mesh-shading", "false"),
		]
		.as_slice(),
	)?;
	let mut render = device.create_context();
	if !render.supports_ray_tracing() {
		return Err("that the GPU can't trace rays".to_string());
	}
	let runtime =
		compio::runtime::Runtime::new().map_err(|error| format!("that the bake's task runtime couldn't start: {error}"))?;
	// The geometry buffers hold every mesh's full detail at once, so they're sized to the level's meshes.
	let resources = EntityHandle::from(resources);
	let capacity = runtime.block_on(scene_capacity(&resources, &scene))?;
	let handles = GeometryHandles::new(&mut render, capacity, true);
	let mut loader = Loader::new(&device, [].as_slice());
	let geometry = GeometryBuffers::import(&handles, &mut render, &mut loader);
	// The loading lanes and the pipeline servers read through the same manager.
	let shader_resources = resources.clone();
	let (mut client, lanes) = spawn(
		&loader,
		BakeLoad {
			resources,
			geometry: Mutex::new(geometry),
			material_slots: Mutex::default(),
			texture_slots: Mutex::default(),
		},
		LANE_COUNT,
		BACKGROUND_LANE_COUNT,
		RESULT_CAPACITY,
	);
	let (pipelines, pipeline_manager, servers) = PipelineManager::new(&mut render, 1);

	loader.run(&runtime);
	for lane in lanes {
		runtime.spawn(lane.run()).detach();
	}
	for server in servers {
		runtime.spawn(server.run(shader_resources.clone())).detach();
	}

	runtime.block_on(async move {
		let mut loaded = load_scene(&mut client, &scene, &settings).await?;
		let layout = lay_out(&scene, &loaded, &settings)?;
		let mut bake = Bake::new(
			&mut render,
			&device,
			&handles,
			pipelines,
			pipeline_manager,
			&scene,
			&mut loaded,
			&settings,
		);
		bake.prepare(&mut render).await?;
		// Pages trace one after another through the same buffers, so the bake holds one page's texels at a time.
		let mut pages = Vec::with_capacity(layout.atlas.pages.len());
		for (page, &extent) in layout.atlas.pages.iter().enumerate() {
			let texels = rasterize_page(&layout, &loaded, page as u32, &settings);
			log::info!(
				"Lightmap bake traces page {} of {}, {extent} × {extent} texels.",
				page + 1,
				layout.atlas.pages.len()
			);
			bake.load_page(&mut render, &texels);
			bake.trace(&mut render, &texels, &settings);
			pages.push(TracedPage {
				extent,
				light: bake.dilate(&mut render, extent, settings.samples),
			});
		}
		Ok(TracedLightmap {
			pages,
			instances: layout.instances,
			shrunk: layout.atlas.shrunk,
			clamped: layout.atlas.clamped,
			unlit: layout.unlit,
		})
	})
}

/* Loading */

/// Returns the geometry capacity that holds the full detail of every distinct mesh `scene` places, measured from the
/// meshes' metadata before any of them loads. The bake traces rays and draws no meshlets, so it keeps no meshlet
/// vertices.
async fn scene_capacity(resources: &ResourceManager, scene: &LevelScene) -> Result<GeometryCapacity, String> {
	let mut total = GeometryCounts::default();
	let mut seen = HashSet::default();
	for mesh in &scene.meshes {
		if !seen.insert(mesh.source.key()) {
			continue;
		}
		let counts = match &mesh.source {
			MeshSource::Resource(id) => {
				let reference: Reference<ResourceMesh> = resources
					.request(id)
					.await
					.map_err(|error| format!("that mesh '{id}' isn't baked ({error})"))?;
				PreparedMesh::resource_counts(reference.resource(), PreparedLevels::Finest, true, false)
					.ok_or_else(|| format!("that mesh '{id}' has malformed geometry streams"))?
			}
			// A generated mesh is one level of one primitive; its meshlets index at most every corner of every
			// triangle, and hold at least one triangle each.
			MeshSource::Generated(generator) => {
				let triangles = (generator.indices().len() / 3) as u32;
				GeometryCounts {
					vertices: generator.positions().len() as u32,
					primitive_indices: triangles * 3,
					triangles,
					meshlets: triangles,
					levels: 1,
					..GeometryCounts::default()
				}
			}
		};
		total.vertices += counts.vertices;
		total.primitive_indices += counts.primitive_indices;
		total.triangles += counts.triangles;
		total.meshlets += counts.meshlets;
		total.levels += counts.levels;
		total.skinning_vertices += counts.skinning_vertices;
	}
	// Every buffer holds at least one element, even for a level without meshes.
	let capacity = GeometryCapacity {
		vertices: total.vertices.max(1),
		vertex_indices: total.primitive_indices.max(1),
		triangles: total.triangles.max(1),
		meshlets: total.meshlets.max(1),
		levels: total.levels.max(1),
		skinning_vertices: total.skinning_vertices.max(1),
		meshlet_vertices: false,
	};
	log::info!(
		"Lightmap bake holds {} vertices, {} triangles, and {} meshlets of {} distinct meshes.",
		capacity.vertices,
		capacity.triangles,
		capacity.meshlets,
		seen.len()
	);
	capacity.validate().map_err(|error| {
		format!("that the level's meshes need more geometry than the visibility pipeline addresses: {error}")
	})?;
	Ok(capacity)
}

/// The `BakeLoad` struct loads what the bake traces on the engine's loading timeline, as the visibility loader loads
/// it for an application.
struct BakeLoad {
	resources: EntityHandle<ResourceManager>,
	geometry: Mutex<GeometryBuffers>,
	material_slots: Mutex<HashMap<String, u32>>,
	texture_slots: Mutex<HashMap<String, u32>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum BakeKey {
	Mesh(MeshKey),
	Material(String),
	Texture(String),
	Environment(String),
}

enum BakeRequest {
	Mesh(MeshSource),
	Material(String),
	Texture(String),
	Environment(String),
}

/// The `BakeResident` enum is one loaded part of the scene, uploaded and ready to bind.
enum BakeResident {
	Mesh {
		key: MeshKey,
		mesh: MeshData,
		/// Each primitive's full-detail triangles in object space, or `None` for a skinned primitive.
		surfaces: Vec<Option<ObjectSurface>>,
		materials: SmallVec<[String; 8]>,
	},
	Material {
		id: String,
		slot: u32,
		record: MaterialData,
		description: MaterialDescription,
	},
	Texture {
		slot: u32,
		image: ghi::implementation::DetachedImage,
	},
	Environment(EnvironmentMaps),
}

impl LoadPipeline for BakeLoad {
	type Key = BakeKey;
	type Request = BakeRequest;
	type Resident = BakeResident;

	fn key(request: &Self::Request) -> Self::Key {
		match request {
			BakeRequest::Mesh(source) => BakeKey::Mesh(source.key()),
			BakeRequest::Material(id) => BakeKey::Material(id.clone()),
			BakeRequest::Texture(id) => BakeKey::Texture(id.clone()),
			BakeRequest::Environment(id) => BakeKey::Environment(id.clone()),
		}
	}

	async fn load(&self, request: Self::Request, lane: &mut LoaderLane<Self>) -> Result<Self::Resident, LoadError> {
		match request {
			BakeRequest::Mesh(source) => self.load_mesh(source, lane).await,
			BakeRequest::Material(id) => self.load_material(id, lane).await,
			BakeRequest::Texture(id) => {
				let reference: Reference<ResourceImage> = self
					.resources
					.request(&id)
					.await
					.map_err(|error| LoadError(format!("that texture '{id}' isn't baked ({error})")))?;
				let slot = slot(&self.texture_slots, &id, MAX_BINDLESS_TEXTURES, "texture")?;
				let image = load_texture(reference, &id, 0, lane)
					.await
					.map_err(|error| LoadError(format!("that texture '{id}' couldn't upload: {error}")))?;
				Ok(BakeResident::Texture { slot, image })
			}
			BakeRequest::Environment(id) => Ok(BakeResident::Environment(
				load_environment_maps(&self.resources, &id, lane).await?,
			)),
		}
	}
}

impl BakeLoad {
	/// Prepares a mesh's full detail, reads its triangles back for the texel raster, and uploads it.
	async fn load_mesh(&self, source: MeshSource, lane: &LoaderLane<Self>) -> Result<BakeResident, LoadError> {
		let key = source.key();
		let mut slots = SmallVec::<[u32; 8]>::new();
		let mut materials = SmallVec::<[String; 8]>::new();
		let prepared = prepare_mesh_source(
			&self.resources,
			&source,
			lane.staging().clone(),
			true,
			false,
			PreparedLevels::Finest,
			|id| {
				lane.request(BakeRequest::Material(id.to_string()));
				slots.push(slot(&self.material_slots, id, MAX_MATERIALS, "material")?);
				materials.push(id.to_string());
				Ok(())
			},
		)
		.await?;
		let surfaces = (0..prepared.primitives.len())
			.map(|primitive| prepared.surface(primitive).map(ObjectSurface::decode))
			.collect();
		let mesh = place_mesh(&self.geometry, prepared, &slots, lane).await?;
		Ok(BakeResident::Mesh {
			key,
			mesh,
			surfaces,
			materials,
		})
	}

	/// Reads a material's record and hit shader, and requests its textures.
	async fn load_material(&self, id: String, lane: &LoaderLane<Self>) -> Result<BakeResident, LoadError> {
		let reference: Reference<ResourceVariant> = self
			.resources
			.request(&id)
			.await
			.map_err(|error| LoadError(format!("that material '{id}' isn't baked ({error})")))?;
		let description = describe_material(&id, reference.resource())?;
		let mut textures = SmallVec::<[Option<u32>; 8]>::new();
		for texture in &description.texture_ids {
			textures.push(match texture {
				Some(texture) => {
					lane.request(BakeRequest::Texture(texture.clone()));
					Some(slot(&self.texture_slots, texture, MAX_BINDLESS_TEXTURES, "texture")?)
				}
				None => None,
			});
		}
		let (record, overflow) = MaterialData::describe(&textures, description.coverage_arm, description.alpha_mode);
		if overflow {
			log::warn!(
				"Material '{id}' names more textures than a material record holds, so the lightmap bake leaves the last ones out. The most likely cause is a material with more image variables than the visibility pipeline supports. Use fewer images in the material."
			);
		}
		let slot = slot(&self.material_slots, &id, MAX_MATERIALS, "material")?;
		Ok(BakeResident::Material {
			id,
			slot,
			record,
			description,
		})
	}
}

/// Returns the table slot of `id`, assigning the next free one the first time.
fn slot(table: &Mutex<HashMap<String, u32>>, id: &str, limit: usize, kind: &str) -> Result<u32, LoadError> {
	let mut table = table.lock().unwrap_or_else(|error| error.into_inner());
	assign_slot(&mut table, id, limit, kind).ok_or_else(|| {
		LoadError(format!(
			"that the level uses more than {limit} {kind}s, which a lightmap bake holds"
		))
	})
}

/// The `ObjectSurface` struct is one primitive's full-detail triangles in object space, decoded for the texel raster.
struct ObjectSurface {
	positions: Vec<[f32; 3]>,
	normals: Vec<[f32; 3]>,
	lightmap_uvs: Vec<[f32; 2]>,
	triangles: Vec<[u32; 3]>,
}

impl ObjectSurface {
	fn decode(surface: PreparedSurface<'_>) -> Self {
		Self {
			positions: surface.positions.to_vec(),
			normals: surface
				.normals
				.iter()
				.map(|&normal| decode_octahedral_unit_vector(normal))
				.collect(),
			lightmap_uvs: surface
				.lightmap_uvs
				.iter()
				.map(|uv| uv.map(|value| f32::from(value) / f32::from(u16::MAX)))
				.collect(),
			triangles: surface.triangles.iter().map(|triangle| triangle.map(u32::from)).collect(),
		}
	}
}

/// The `LoadedScene` struct is everything the loading timeline uploaded for the bake.
struct LoadedScene {
	meshes: HashMap<MeshKey, (MeshData, Vec<Option<ObjectSurface>>)>,
	materials: Vec<(String, u32, MaterialData, MaterialDescription)>,
	textures: Vec<(u32, ghi::implementation::DetachedImage)>,
	environment: Option<EnvironmentMaps>,
}

/// Requests every mesh of `scene` and the environment, and waits until they, their materials, and the materials'
/// textures are uploaded.
async fn load_scene(
	client: &mut crate::rendering::loading::LoaderClient<BakeLoad>,
	scene: &LevelScene,
	settings: &TraceSettings,
) -> Result<LoadedScene, String> {
	let mut meshes_wanted = HashSet::default();
	for mesh in &scene.meshes {
		if meshes_wanted.insert(mesh.source.key()) {
			client.request(BakeRequest::Mesh(mesh.source.clone()));
		}
	}
	if let Some(environment) = &settings.environment {
		client.request(BakeRequest::Environment(environment.clone()));
	}
	let mut loaded = LoadedScene {
		meshes: HashMap::default(),
		materials: Vec::new(),
		textures: Vec::new(),
		environment: None,
	};
	let mut materials_wanted = HashSet::<String>::default();
	let mut textures_wanted = HashSet::<String>::default();
	let mut materials_loaded = HashSet::<String>::default();
	let mut textures_loaded = 0;
	loop {
		while let Some(event) = client.poll() {
			match event {
				Event::Ready { resident, .. } => match resident {
					BakeResident::Mesh {
						key,
						mesh,
						surfaces,
						materials,
					} => {
						materials_wanted.extend(materials);
						loaded.meshes.insert(key, (mesh, surfaces));
					}
					BakeResident::Material {
						id,
						slot,
						record,
						description,
					} => {
						textures_wanted.extend(description.texture_ids.iter().flatten().cloned());
						materials_loaded.insert(id.clone());
						loaded.materials.push((id, slot, record, description));
					}
					BakeResident::Texture { slot, image } => {
						textures_loaded += 1;
						loaded.textures.push((slot, image));
					}
					BakeResident::Environment(environment) => loaded.environment = Some(environment),
				},
				Event::Failed { key, error } => return Err(format!("that {key:?} couldn't load, likely {error}")),
			}
		}
		let done = loaded.meshes.len() == meshes_wanted.len()
			&& materials_wanted.iter().all(|id| materials_loaded.contains(id))
			&& textures_loaded == textures_wanted.len()
			&& (settings.environment.is_none() || loaded.environment.is_some());
		if done {
			return Ok(loaded);
		}
		compio::time::sleep(POLL_INTERVAL).await;
	}
}

/* Texels */

/// The `BakeLayout` struct is the lightmap's layout and the lit primitives placed in it, which each page's texels are
/// drawn from.
struct BakeLayout {
	atlas: Atlas,
	/// Each lit primitive, in the order of the atlas's rectangles.
	lit: Vec<LitPrimitive>,
	instances: Vec<LightmapInstance>,
	unlit: Vec<UnlitMesh>,
}

/// The `LitPrimitive` struct is one primitive the lightmap gives a rectangle, with its vertices in world space.
struct LitPrimitive {
	/// The spawn index of the primitive's entity.
	entity: u32,
	primitive: u32,
	positions: Vec<[f32; 3]>,
	normals: Vec<[f32; 3]>,
	mesh: MeshKey,
}

/// The `BakeTexels` struct is one page's texels that trace paths.
struct BakeTexels {
	/// The page's side, in texels.
	extent: u32,
	texels: Vec<LightmapTexel>,
	/// The traced texel each texel of the page holds, or [`NO_LIGHTMAP_TEXEL`].
	pixels: Vec<u32>,
	/// The groups of texels that share a guide.
	groups: GuideGroups,
}

/// Lays out a rectangle for every opaque primitive of `scene` that has lightmap UVs, across as many pages as they need.
///
/// Masked and blended primitives take no rectangle. The bake's rays pass through them, so their texels would gather
/// light from both sides, and they are usually small, many, and thin, like grass, so the renderer lights them with the
/// environment instead.
fn lay_out(scene: &LevelScene, loaded: &LoadedScene, settings: &TraceSettings) -> Result<BakeLayout, String> {
	let alpha_modes = alpha_modes(loaded);
	let mut lit = Vec::new();
	let mut unlit = HashMap::<MeshKey, UnlitMesh>::default();
	let mut sides = Vec::new();
	for mesh in &scene.meshes {
		let key = mesh.source.key();
		let Some((data, surfaces)) = loaded.meshes.get(&key) else {
			continue;
		};
		let rows = model_rows(&mesh.transform);
		for (primitive, surface) in surfaces.iter().enumerate() {
			let Some(surface) = surface else {
				continue;
			};
			if alpha_modes[data.primitives[primitive].material_index as usize] != AlphaMode::Opaque {
				continue;
			}
			let positions: Vec<[f32; 3]> = surface
				.positions
				.iter()
				.map(|&position| transform_point(&rows, position))
				.collect();
			let normals: Vec<[f32; 3]> = surface
				.normals
				.iter()
				.map(|&normal| transform_direction(&rows, normal))
				.collect();
			let (world_area, uv_area) = TexelSurface {
				positions: &positions,
				normals: &normals,
				uvs: &surface.lightmap_uvs,
				triangles: &surface.triangles,
			}
			.areas();
			// A primitive without lightmap UVs holds zeros, which cover no area.
			if uv_area <= 0.0 {
				unlit
					.entry(key)
					.or_insert_with(|| UnlitMesh {
						mesh: match &mesh.source {
							MeshSource::Resource(id) => id.to_string(),
							MeshSource::Generated(_) => "a generated mesh".to_string(),
						},
						primitives: 0,
					})
					.primitives += 1;
				continue;
			}
			sides.push(atlas::rectangle_side(world_area, uv_area, settings.texel_density));
			lit.push(LitPrimitive {
				entity: mesh.entity,
				primitive: primitive as u32,
				positions,
				normals,
				mesh: key,
			});
		}
	}
	let atlas = atlas::layout(&sides, settings.max_extent, MAX_LIGHTMAP_PAGES).ok_or_else(|| {
		format!(
			"that the level has more lit primitives than {MAX_LIGHTMAP_PAGES} pages of {0} × {0} texels hold",
			settings.max_extent
		)
	})?;
	let mut instances: Vec<LightmapInstance> = lit
		.iter()
		.enumerate()
		.map(|(index, lit)| LightmapInstance {
			entity: lit.entity,
			primitive: lit.primitive,
			page: atlas.rectangles[index].page,
			scale_offset: atlas.scale_offset(index),
		})
		.collect();
	instances.sort_by_key(|instance| (instance.entity, instance.primitive));
	Ok(BakeLayout {
		atlas,
		lit,
		instances,
		unlit: unlit.into_values().collect(),
	})
}

/// Returns the alpha mode of each material slot, opaque for a slot no loaded material takes, so the layout and the
/// traced scene agree on which primitives are opaque.
fn alpha_modes(loaded: &LoadedScene) -> Vec<AlphaMode> {
	let mut modes = vec![AlphaMode::Opaque; MAX_MATERIALS];
	for (_, slot, _, description) in &loaded.materials {
		modes[*slot as usize] = description.alpha_mode;
	}
	modes
}

/// Draws the triangles of every primitive whose rectangle lies on `page` into it, and groups the texels that share a
/// guide.
fn rasterize_page(layout: &BakeLayout, loaded: &LoadedScene, page: u32, settings: &TraceSettings) -> BakeTexels {
	let extent = layout.atlas.pages[page as usize];
	let mut texels = Vec::new();
	let mut pixels = vec![NO_LIGHTMAP_TEXEL; (extent * extent) as usize];
	let mut raster = TexelRaster::default();
	for (lit, &rectangle) in layout.lit.iter().zip(&layout.atlas.rectangles) {
		if rectangle.page != page {
			continue;
		}
		let surface = loaded.meshes[&lit.mesh].1[lit.primitive as usize]
			.as_ref()
			.expect("a lit primitive has a surface");
		raster.rasterize(
			&TexelSurface {
				positions: &lit.positions,
				normals: &lit.normals,
				uvs: &surface.lightmap_uvs,
				triangles: &surface.triangles,
			},
			rectangle,
			extent,
			&mut texels,
			&mut pixels,
		);
	}
	// A group spans four texels' widths at the density the layout kept.
	let density = settings.texel_density * layout.atlas.shrunk.unwrap_or(1.0);
	let groups = group_texels(&texels, GROUP_TEXELS / density);
	BakeTexels {
		extent,
		texels,
		pixels,
		groups,
	}
}

/// Returns the three rows of a transform's matrix, as an acceleration structure instance takes them.
fn model_rows(transform: &crate::gameplay::Transform) -> [[f32; 4]; 3] {
	crate::rendering::pipelines::visibility::acceleration_structures::instance_transform(transform.get_matrix().into())
}

fn transform_point(rows: &[[f32; 4]; 3], [x, y, z]: [f32; 3]) -> [f32; 3] {
	rows.map(|row| row[0] * x + row[1] * y + row[2] * z + row[3])
}

/// Transforms a normal by the matrix's linear part, as the closest-hit shaders do, and normalizes it.
fn transform_direction(rows: &[[f32; 4]; 3], [x, y, z]: [f32; 3]) -> [f32; 3] {
	let direction = rows.map(|row| row[0] * x + row[1] * y + row[2] * z);
	let length = direction.iter().map(|value| value * value).sum::<f32>().sqrt();
	if length > 0.0 {
		direction.map(|value| value / length)
	} else {
		direction
	}
}

/* GPU */

/// The `LightmapBakeParameters` struct is one trace batch's or dilation pass's settings, as the kernels read them.
///
/// Its layout matches `LightmapBakeParameters` in `lightmaps/layouts/lightmap_bake.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct LightmapBakeParameters {
	texel_count: u32,
	sample_offset: u32,
	samples_per_batch: u32,
	samples: u32,
	bounces: u32,
	environment_intensity: f32,
	extent: u32,
	pass: u32,
	learning_samples: u32,
	group_count: u32,
	launch_offset: u32,
}

/// The `Bake` struct holds the GPU state of one lightmap bake: the scene's sets, its acceleration structures, and the
/// pipelines that trace and dilate.
struct Bake {
	queue: ghi::QueueHandle,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
	frame: u64,
	pipelines: PipelineManagerClient,
	pipeline_manager: PipelineManager,
	structures: SceneAccelerationStructures,
	shading: HitShading,
	dilate: crate::rendering::PipelineRef,
	traced: Vec<TracedPrimitive>,
	alpha_modes: Vec<AlphaMode>,
	/// The images the loading timeline uploaded, which the first frame interns and binds.
	pending_images: Vec<(u32, ghi::implementation::DetachedImage)>,
	environment: Option<EnvironmentMaps>,
	environment_sampler: ghi::SamplerHandle,
	/// The layout of [`Self::scene_set`], which names its texture array when the bake binds an image.
	scene_layout: SetLayout,
	scene_set: ghi::DescriptorSetHandle,
	lights_set: ghi::DescriptorSetHandle,
	bake_set: ghi::DescriptorSetHandle,
	bake_layout: SetLayout,
	parameters: ghi::BufferHandle<LightmapBakeParameters>,
	environment_intensity: f32,
	page: PageBuffers,
}

/// The `PageBuffers` struct holds the buffers one page of texels traces through. Pages reuse them, growing them for a
/// page larger than any before, so the bake's memory follows its largest page rather than the whole level.
struct PageBuffers {
	texels: ghi::BufferHandle<[LightmapTexel]>,
	accumulation: ghi::BufferHandle<[[f32; 4]]>,
	escapes: ghi::BufferHandle<[f32]>,
	group_texels: ghi::BufferHandle<[u32]>,
	texel_groups: ghi::BufferHandle<[u32]>,
	groups: ghi::BufferHandle<[super::guides::LightmapGuideGroup]>,
	guide_cells: ghi::BufferHandle<[f32]>,
	guides: ghi::BufferHandle<[f32]>,
	indices: ghi::BufferHandle<[u32]>,
	light: ghi::BufferHandle<[[f32; 4]]>,
	dilated: ghi::BufferHandle<[[f32; 4]]>,
	/// The elements each group of buffers holds: per texel, per guide group, and per texel of the page's image.
	texel_capacity: usize,
	group_capacity: usize,
	image_capacity: usize,
}

impl PageBuffers {
	/// Creates every buffer with one element; the first page grows them.
	fn new(render: &mut ghi::implementation::Context) -> Self {
		let host = |name| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.length(1)
				.host_access(ghi::HostAccess::Write)
		};
		let device = |name| ghi::buffer::Builder::new(ghi::Uses::Storage).name(name).length(1);
		Self {
			texels: render.build_buffer(host("Lightmap Texels")),
			accumulation: render.build_buffer(host("Lightmap Accumulation")),
			escapes: render.build_buffer(host("Lightmap Escapes")),
			group_texels: render.build_buffer(host("Lightmap Guide Group Texels")),
			texel_groups: render.build_buffer(host("Lightmap Texel Guide Groups")),
			groups: render.build_buffer(host("Lightmap Guide Groups")),
			// The learning batches fill both before any guided batch reads them, so they start unwritten.
			guide_cells: render.build_buffer(device("Lightmap Guide Cells").length(GUIDE_CELLS)),
			guides: render.build_buffer(device("Lightmap Guides").length(GUIDE_STRIDE)),
			indices: render.build_buffer(host("Lightmap Texel Indices")),
			light: render.build_buffer(
				ghi::buffer::Builder::new(ghi::Uses::Storage)
					.name("Lightmap Light")
					.length(1)
					.host_access(ghi::HostAccess::Read),
			),
			dilated: render.build_buffer(device("Lightmap Dilated")),
			texel_capacity: 1,
			group_capacity: 1,
			image_capacity: 1,
		}
	}
}

/// Calls `resize` with `needed` elements when they exceed `capacity`, and returns the capacity after.
fn grow(
	render: &mut ghi::implementation::Context,
	capacity: usize,
	needed: usize,
	resize: impl FnOnce(&mut ghi::implementation::Context, usize),
) -> usize {
	if needed <= capacity {
		return capacity;
	}
	resize(render, needed);
	needed
}

/// Writes `values` to the front of a host-written buffer that holds at least as many.
fn write<T: bytemuck::Pod>(render: &mut ghi::implementation::Context, buffer: ghi::BufferHandle<[T]>, values: &[T]) {
	// SAFETY: the bake waits for the GPU after every frame, so no submitted command uses the buffer while it's mapped,
	// and the mapping ends with this call.
	#[allow(unsafe_code)]
	let bytes = unsafe { render.map_unsynchronized(buffer) }.into_bytes();
	let bytes = &mut bytes[..std::mem::size_of_val(values)];
	bytes.copy_from_slice(bytemuck::cast_slice(values));
}

/// Clears the first `count` elements of `buffer` to zero bytes in place.
fn zero<T: bytemuck::Pod>(render: &mut ghi::implementation::Context, buffer: ghi::BufferHandle<[T]>, count: usize) {
	// SAFETY: as in `write`, no submitted command uses the buffer while it's mapped.
	#[allow(unsafe_code)]
	let bytes = unsafe { render.map_unsynchronized(buffer) }.into_bytes();
	bytes[..count * std::mem::size_of::<T>()].fill(0);
}

impl Bake {
	/// Creates the scene's buffers and sets from what loaded, and requests the pipelines.
	#[allow(clippy::too_many_arguments)]
	fn new(
		render: &mut ghi::implementation::Context,
		device: &GraphicsDevice,
		handles: &GeometryHandles,
		pipelines: PipelineManagerClient,
		pipeline_manager: PipelineManager,
		scene: &LevelScene,
		loaded: &mut LoadedScene,
		settings: &TraceSettings,
	) -> Self {
		let queue = device.graphics_queue();
		// The bake traces every primitive of every mesh entity that loaded, one instance each.
		let primitives = scene
			.meshes
			.iter()
			.filter_map(|mesh| loaded.meshes.get(&mesh.source.key()))
			.map(|(data, _)| data.primitives.len())
			.sum::<usize>()
			.max(1);
		let mut structures = SceneAccelerationStructures::new(render, handles, primitives as u32)
			.expect("the bake's geometry keeps flat triangle lists");
		let mut shading = HitShading::new(render, LIGHTMAP_TRACE_SHADER, "lightmap bake");
		let dilate = pipelines.request_pipeline(LIGHTMAP_DILATE_PIPELINE);

		// Materials: their records, hit groups, and parameter blocks.
		let mut materials = vec![MaterialData::default(); MAX_MATERIALS];
		let alpha_modes = alpha_modes(loaded);
		let mut double_sided = vec![false; MAX_MATERIALS];
		let mut parameters = MaterialParameters::new();
		for (id, slot, record, description) in &loaded.materials {
			let index = *slot as usize;
			materials[index] = *record;
			double_sided[index] = description.double_sided;
			shading.set_material(*slot, description.hit_group.as_ref());
			parameters.adopt_material(*slot, id.as_str().into(), description.parameters.clone());
		}

		// Instances: one record per primitive of every mesh entity, which hits read by their record.
		let mut meshes = Vec::with_capacity(primitives);
		let mut traced = Vec::new();
		for mesh in &scene.meshes {
			let key = mesh.source.key();
			let Some((data, _)) = loaded.meshes.get(&key) else {
				continue;
			};
			structures.register_mesh(key, data);
			let model = mesh.transform.get_matrix().into();
			// Parameter blocks are kept per entity; the bake sets none, so any handle the entity alone has will do, and
			// zero is reserved.
			let handle = Handle::from_id(mesh.entity + 1);
			for (primitive_index, primitive) in data.primitives.iter().enumerate() {
				let traced_record = traced.len() as u32;
				let mut record = ShaderMesh::new(model, primitive.material_index);
				let flags = if double_sided[primitive.material_index as usize] {
					MESH_FLAG_DOUBLE_SIDED
				} else {
					0
				};
				let level = primitive.lods.drawn(0).expect(
					"Lightmap bake mesh has no resident detail level. The most likely cause is a mesh that was placed without its full detail.",
				);
				record.set_level(level.level, flags);
				record.set_parameter_block(parameters.block(handle, primitive.material_index));
				meshes.push(record);
				traced.push(TracedPrimitive {
					renderable: handle,
					mesh: key,
					primitive: primitive_index as u32,
					record: traced_record,
					model,
				});
			}
		}
		// A scene without primitives still binds a one-record table.
		if meshes.is_empty() {
			meshes.push(bytemuck::Zeroable::zeroed());
		}

		// Lights, as the renderer uploads them.
		let environment_intensity = match (&loaded.environment, settings.illuminance) {
			(None, _) => 0.0,
			(Some(environment), Some(lux)) if environment.upward_illuminance > 0.0 => lux / environment.upward_illuminance,
			(Some(_), _) => 1.0,
		};
		let mut lighting: Box<LightingData> = bytemuck::allocation::zeroed_box();
		if scene.lights.len() > MAX_LIGHTS {
			log::warn!(
				"The lightmap bake leaves out {} of the level's {} lights. The most likely cause is a level with more lights than the {MAX_LIGHTS} a scene holds. Remove lights from the level, or merge nearby ones.",
				scene.lights.len() - MAX_LIGHTS,
				scene.lights.len()
			);
		}
		lighting.count = scene.lights.len().min(MAX_LIGHTS) as u32;
		lighting.exposure = 1.0;
		lighting.environment_intensity = environment_intensity;
		for (destination, light) in lighting.lights.iter_mut().zip(&scene.lights) {
			*destination = *light;
		}

		let meshes_buffer = upload(render, "Lightmap Bake Meshes", &meshes);
		let materials_buffer = upload(render, "Lightmap Bake Materials", &materials);
		let (first, values) = parameters.take_changes(0).unwrap_or((0, &[]));
		let mut parameter_values = vec![[0.0f32; 4]; (first + values.len()).max(1)];
		parameter_values[first..first + values.len()].copy_from_slice(values);
		let parameters_buffer = upload(render, "Lightmap Bake Material Parameters", &parameter_values);
		let lighting_data = render.build_buffer_with::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Bake Lights")
				.host_access(ghi::HostAccess::Write),
			|buffer| *buffer = *lighting,
		);
		// The hit shaders read no view, dispatch, or evaluation records, but the scene set declares them.
		let unused = upload(render, "Lightmap Bake Unused Records", &vec![0u32; 1 << 16]);

		let scene_layout = set_layouts::layout(set_layouts::SCENE);
		let scene_set = scene_layout.create_set(render, "Lightmap Bake Scene");
		let binder = scene_layout.binder(scene_set);
		render.write(&[
			binder.buffer("views", unused),
			binder.buffer("meshes", meshes_buffer),
			binder.buffer("vertex_positions", handles.vertex_positions),
			binder.buffer("vertex_normals", handles.vertex_normals),
			binder.buffer("vertex_uvs", handles.vertex_uvs),
			binder.buffer("vertex_indices", handles.vertex_indices),
			binder.buffer("meshlet_vertices", handles.meshlet_vertices),
			binder.buffer("primitive_indices", handles.primitive_indices),
			binder.buffer("meshlets", handles.meshlets),
			binder.buffer("mesh_levels", handles.mesh_levels),
			binder.buffer("materials", materials_buffer),
			binder.buffer("mesh_dispatch_work", unused),
			binder.buffer("material_evaluations", unused),
			binder.buffer("material_parameters", parameters_buffer),
			binder.buffer("vertex_lightmap_uvs", handles.vertex_lightmap_uvs),
		]);

		let lights_layout = SetLayout::from_layer(
			"::byte_engine::rendering::visibility::layouts::scene_lights::SceneLights",
			ENGINE_MODULES,
		);
		let lights_set = lights_layout.create_set(render, "Lightmap Bake Lights");
		render.write(&[lights_layout.binder(lights_set).buffer("lighting_data", lighting_data)]);

		// The texels, their sums, and the image. Each page fills them in turn; see `load_page`.
		let bake_layout = SetLayout::from_layer(
			"::byte_engine::rendering::lightmaps::layouts::lightmap_bake::LightmapBake",
			ENGINE_MODULES,
		);
		let bake_set = bake_layout.create_set(render, "Lightmap Bake");
		let page = PageBuffers::new(render);
		let parameters = render.build_buffer::<LightmapBakeParameters>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Bake Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let bake_binder = bake_layout.binder(bake_set);
		render.write(&[
			bake_binder.buffer("lightmap_parameters", parameters),
			bake_binder.buffer("lightmap_texels", page.texels),
			bake_binder.buffer("lightmap_accumulation", page.accumulation),
			bake_binder.buffer("lightmap_texel_indices", page.indices),
			bake_binder.buffer("lightmap_light", page.light),
			bake_binder.buffer("lightmap_dilated", page.dilated),
			bake_binder.buffer("lightmap_escapes", page.escapes),
			bake_binder.buffer("lightmap_groups", page.groups),
			bake_binder.buffer("lightmap_group_texels", page.group_texels),
			bake_binder.buffer("lightmap_texel_groups", page.texel_groups),
			bake_binder.buffer("lightmap_guide_cells", page.guide_cells),
			bake_binder.buffer("lightmap_guides", page.guides),
		]);

		let environment_sampler = render.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));
		if loaded.environment.is_none() {
			// Paths that leave a scene without an environment bring back no light.
			let fallback = create_fallback_environment(render, queue, environment_sampler);
			render.write(&[bake_binder.sampled("lightmap_environment_specular", fallback.specular_image, environment_sampler)]);
		}

		let command_buffer = render.queue(queue).create_command_buffer(Some("Lightmap Bake"));
		let synchronizer = render.create_synchronizer(Some("Lightmap Bake"), true);
		Self {
			queue,
			command_buffer,
			synchronizer,
			frame: 0,
			pipelines,
			pipeline_manager,
			structures,
			shading,
			dilate,
			traced,
			alpha_modes,
			pending_images: std::mem::take(&mut loaded.textures),
			environment: loaded.environment.take(),
			environment_sampler,
			scene_layout,
			scene_set,
			lights_set,
			bake_set,
			bake_layout,
			parameters,
			environment_intensity,
			page,
		}
	}

	/// Fills the page buffers with the texels of one page, growing any that is too small, and clears the sums of the
	/// page before.
	///
	/// Every frame waits for the GPU, so no submitted command reads the buffers while this writes them.
	fn load_page(&mut self, render: &mut ghi::implementation::Context, texels: &BakeTexels) {
		let page = &mut self.page;
		let texel_count = texels.texels.len().max(1);
		let group_count = texels.groups.groups.len().max(1);
		let image_texels = texels.pixels.len();
		page.texel_capacity = grow(render, page.texel_capacity, texel_count, |render, length| {
			render.resize_buffer(page.texels, length);
			render.resize_buffer(page.accumulation, length);
			render.resize_buffer(page.escapes, length);
			render.resize_buffer(page.group_texels, length);
			render.resize_buffer(page.texel_groups, length);
		});
		page.group_capacity = grow(render, page.group_capacity, group_count, |render, length| {
			render.resize_buffer(page.groups, length);
			render.resize_buffer(page.guide_cells, length * GUIDE_CELLS);
			render.resize_buffer(page.guides, length * GUIDE_STRIDE);
		});
		page.image_capacity = grow(render, page.image_capacity, image_texels, |render, length| {
			render.resize_buffer(page.indices, length);
			render.resize_buffer(page.light, length);
			render.resize_buffer(page.dilated, length);
		});
		write(render, page.texels, &texels.texels);
		write(render, page.indices, &texels.pixels);
		write(render, page.groups, &texels.groups.groups);
		write(render, page.group_texels, &texels.groups.texels);
		write(render, page.texel_groups, &texels.groups.texel_groups);
		// The trace adds every path's light to these sums, so each page starts them at zero.
		zero(render, page.accumulation, texel_count);
		zero(render, page.escapes, texel_count);
	}

	/// Runs frames until every bottom level is built and both pipelines compiled, which the pipeline servers do while
	/// this yields to them.
	async fn prepare(&mut self, render: &mut ghi::implementation::Context) -> Result<(), String> {
		loop {
			self.run_frame(render, LightmapBakeParameters::default(), FrameWork::Settle);
			match self.pipelines.get(self.dilate) {
				PipelineState::Failed => return Err("that the dilation pipeline failed to compile".to_string()),
				_ if self.structures.built() && self.shading.settled() && self.pipelines.pipeline(self.dilate).is_some() => {
					return Ok(());
				}
				_ => compio::time::sleep(POLL_INTERVAL).await,
			}
		}
	}

	/// Traces every texel's paths, in batches that each end well within the GPU's watchdog: first the learning paths,
	/// one invocation per guide group, then the guided ones, one invocation per texel.
	fn trace(&mut self, render: &mut ghi::implementation::Context, texels: &BakeTexels, settings: &TraceSettings) {
		let texel_count = texels.texels.len() as u32;
		if texel_count == 0 {
			return;
		}
		let group_count = texels.groups.groups.len() as u32;
		log::info!("Lightmap bake traces {texel_count} texels, in {group_count} groups that share guides.");
		let learning_samples = settings.samples / LEARNING_SHARE;
		// A learning batch ends where learning does, so the group's last one builds the guide before any guided batch.
		for (start, end, launches) in [
			(0, learning_samples, group_count),
			(learning_samples, settings.samples, texel_count),
		] {
			let split = BatchSplit::new(texel_count, launches, settings.bounces, end - start);
			let mut sample_offset = start;
			while sample_offset < end {
				let batch = split.samples.min(end - sample_offset);
				let mut launch_offset = 0;
				while launch_offset < launches {
					let range = split.launches.min(launches - launch_offset);
					let parameters = LightmapBakeParameters {
						texel_count,
						sample_offset,
						samples_per_batch: batch,
						samples: settings.samples,
						bounces: settings.bounces,
						environment_intensity: self.environment_intensity,
						extent: texels.extent,
						pass: 0,
						learning_samples,
						group_count,
						launch_offset,
					};
					self.run_frame(render, parameters, FrameWork::Trace { launches: range });
					launch_offset += range;
				}
				sample_offset += batch;
				log::info!(
					"Lightmap bake traced {sample_offset} of {} paths per texel.",
					settings.samples
				);
			}
		}
	}

	/// Resolves the paths' light into the image, dividing each texel's sum by its `samples` paths, dilates it, and
	/// reads it back.
	fn dilate(&mut self, render: &mut ghi::implementation::Context, extent: u32, samples: u32) -> Vec<u8> {
		let pipeline = self
			.pipelines
			.pipeline(self.dilate)
			.expect("the dilation pipeline compiled before the trace");
		for pass in 0..=DILATE_PASSES {
			let parameters = LightmapBakeParameters {
				extent,
				pass,
				samples,
				..LightmapBakeParameters::default()
			};
			self.run_frame(
				render,
				parameters,
				FrameWork::Dilate { pipeline, extent },
			);
		}
		// SAFETY: `run_frame` waited for the GPU after the last dilation, so no submitted command uses the buffer.
		#[allow(unsafe_code)]
		let light = unsafe { render.map_unsynchronized(self.page.light) }.into_bytes();
		let light: &[[f32; 4]] = bytemuck::cast_slice(light);
		// Stored as half floats. A texel without light stores no view of the environment either.
		let mut encoded = Vec::with_capacity((extent * extent) as usize * 8);
		for &[red, green, blue, visibility] in &light[..(extent * extent) as usize] {
			for channel in [red, green, blue, visibility.max(0.0)] {
				encoded.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
			}
		}
		encoded
	}

	/// Records one frame: it binds the images that arrived, publishes compiled pipelines, places the instances and
	/// builds what the frame's budget allows, writes `parameters`, then records `work`. Waits for the GPU to finish.
	fn run_frame(&mut self, render: &mut ghi::implementation::Context, parameters: LightmapBakeParameters, work: FrameWork) {
		let bump = bumpalo::Bump::new();
		let frame_index = self.frame;
		self.frame += 1;
		let Self {
			queue,
			command_buffer,
			synchronizer,
			pipelines,
			pipeline_manager,
			structures,
			shading,
			traced,
			alpha_modes,
			pending_images,
			environment,
			environment_sampler,
			scene_layout,
			scene_set,
			lights_set,
			bake_set,
			bake_layout,
			parameters: parameters_buffer,
			..
		} = self;
		render.queue(*queue).execute(
			Some(FrameRequest::new(frame_index, *synchronizer)),
			&[],
			*synchronizer,
			|execution| {
				let frame = execution.frame().expect("the bake requests a frame");
				for (slot, image) in pending_images.drain(..) {
					let image = frame.intern_image(image);
					frame.write(&[scene_layout.binder(*scene_set).texture_element("textures", image, slot)]);
				}
				if let Some(environment) = environment.take() {
					let specular = frame.intern_image(environment.specular_image);
					frame.write(&[bake_layout.binder(*bake_set).sampled(
						"lightmap_environment_specular",
						specular,
						*environment_sampler,
					)]);
				}
				pipeline_manager.publish(frame);
				let alpha_modes = &*alpha_modes;
				let record_materials = structures.admit(frame, traced, |slot| &alpha_modes[slot as usize]);
				let hit = shading.prepare(frame, pipelines, record_materials);
				let structure_work = structures.prepare(frame, traced, &bump);
				let ray_traced_scene = structures.descriptor_set();
				*frame.map(*parameters_buffer) = parameters;
				execution.record(*command_buffer, |recording| {
					if !structure_work.builds.is_empty() {
						recording.build_bottom_level_acceleration_structures(structure_work.builds);
					}
					recording.build_top_level_acceleration_structure(structure_work.top_level, structure_work.instances);
					match work {
						FrameWork::Settle => {}
						FrameWork::Trace { launches } => {
							if let Some((pipeline, table)) = hit {
								recording
									.bind_ray_tracing_pipeline(pipeline)
									.bind_descriptor_sets(&[*scene_set, ray_traced_scene, *lights_set, *bake_set])
									.trace_rays(table, Extent::line(launches));
							}
						}
						FrameWork::Dilate { pipeline, extent } => {
							recording
								.bind_compute_pipeline(pipeline)
								.bind_descriptor_sets(&[*bake_set])
								.dispatch(ghi::DispatchExtent::new(
									Extent::rectangle(extent, extent),
									Extent::square(DILATE_WORKGROUP),
								));
						}
					}
				});
				[]
			},
		);
		structures.commit(true);
		render.wait();
	}
}

/// The `FrameWork` enum is what one bake frame records after the acceleration structures.
enum FrameWork {
	/// Nothing: the frame only builds and compiles.
	Settle,
	/// One batch of paths, from `launches` guide groups while learning and from as many texels after.
	Trace { launches: u32 },
	/// One dilation pass over the `extent` × `extent` image.
	Dilate { pipeline: ghi::PipelineHandle, extent: u32 },
}

/// The `BatchSplit` struct is how one phase of the trace divides its paths so that no launch casts more than
/// [`SEGMENTS_PER_BATCH`] ray segments: how many paths each texel traces per batch, and how many of the phase's
/// launches run at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BatchSplit {
	samples: u32,
	launches: u32,
}

impl BatchSplit {
	/// Splits a phase that launches `launches` invocations over `texel_count` texels, with paths of at most `bounces`
	/// hits, and traces `phase_samples` paths per texel.
	///
	/// A launch traces its share of the texels: one texel while guided, a whole guide group while learning. When one
	/// path from every texel already overruns the budget, each batch traces one path and runs over ranges of launches.
	fn new(texel_count: u32, launches: u32, bounces: u32, phase_samples: u32) -> Self {
		let launches = launches.max(1);
		let texels_per_launch = u64::from(texel_count.div_ceil(launches).max(1));
		// A path casts a ray and a shadow ray at each hit.
		let segments_per_launch = texels_per_launch * u64::from(bounces) * 2;
		let launch_budget = (SEGMENTS_PER_BATCH / segments_per_launch.max(1)).max(1);
		if launch_budget >= u64::from(launches) {
			let samples = (launch_budget / u64::from(launches)).clamp(1, u64::from(phase_samples.max(1)));
			Self {
				samples: samples as u32,
				launches,
			}
		} else {
			Self {
				samples: 1,
				launches: launch_budget as u32,
			}
		}
	}
}

/// Builds a host-written storage buffer named `name` that holds `values`.
fn upload<T: bytemuck::Pod>(render: &mut ghi::implementation::Context, name: &str, values: &[T]) -> ghi::BufferHandle<[T]> {
	let builder = ghi::buffer::Builder::new(ghi::Uses::Storage)
		.name(name)
		.length(values.len())
		.host_access(ghi::HostAccess::Write);
	// A backend may round the allocation up, which gives the buffer more elements than were asked for.
	render.build_buffer_with::<[T]>(builder, |buffer| buffer[..values.len()].copy_from_slice(values))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The ray segments one launch range of `split` casts per batch.
	fn segments(split: BatchSplit, texel_count: u32, launches: u32, bounces: u32) -> u64 {
		let texels_per_launch = u64::from(texel_count.div_ceil(launches));
		u64::from(split.launches) * texels_per_launch * u64::from(bounces) * 2 * u64::from(split.samples)
	}

	#[test]
	fn a_small_lightmap_traces_many_paths_in_one_launch() {
		let split = BatchSplit::new(100_000, 100_000, 3, 256);

		assert_eq!(split.launches, 100_000);
		assert!(split.samples > 1);
		assert!(segments(split, 100_000, 100_000, 3) <= SEGMENTS_PER_BATCH);
	}

	#[test]
	fn a_lightmap_past_the_budget_splits_one_path_over_several_launches() {
		// Three million texels at three bounces cast more segments than one batch allows for a single path each.
		let split = BatchSplit::new(3_000_000, 3_000_000, 3, 256);

		assert_eq!(split.samples, 1);
		assert!(split.launches < 3_000_000);
		assert!(segments(split, 3_000_000, 3_000_000, 3) <= SEGMENTS_PER_BATCH);
	}

	#[test]
	fn learning_launches_count_their_groups_texels() {
		// Learning launches one invocation per group of about 32 texels, so each launch weighs 32 texels' paths.
		let split = BatchSplit::new(3_200_000, 100_000, 3, 32);

		assert!(segments(split, 3_200_000, 100_000, 3) <= SEGMENTS_PER_BATCH);
	}
}
