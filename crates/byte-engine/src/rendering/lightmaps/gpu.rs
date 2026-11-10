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
		GeometryBuffers, GeometryCapacity, GeometryHandles, MeshData, PreparedLevels, PreparedSurface,
		decode_octahedral_unit_vector,
	},
	layout::{MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIALS},
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
	/// The image's side, in texels.
	pub(crate) extent: u32,
	/// Irradiance over π in RGB, and in alpha the share of paths whose first ray reached the environment, or -1 where
	/// the texel holds no light, row after row.
	pub(crate) light: Vec<[f32; 4]>,
	/// Every lit primitive's rectangle, in ascending entity then primitive order.
	pub(crate) instances: Vec<LightmapInstance>,
	/// The factor the texel density shrank by to fit the largest side, when it had to.
	pub(crate) shrunk: Option<f32>,
	/// Each primitive left unlit because it has no lightmap UVs, as entity and primitive.
	pub(crate) unlit: Vec<(u32, u32)>,
}

/// Traces the lightmap of `scene` with `settings`. The loading timeline reads the baked meshes, materials, and
/// environment through `resources`, and the pipeline compiler reads the baked shaders through `shaders`.
///
/// Blocks the calling thread until the GPU work finishes, so call it from a thread of its own. Returns the most likely
/// cause of what stopped the bake.
pub(crate) fn trace(
	resources: ResourceManager,
	shaders: ResourceManager,
	scene: LevelScene,
	settings: TraceSettings,
) -> Result<TracedLightmap, String> {
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
	// The bake traces rays and draws no meshlets, so it keeps no meshlet vertices.
	let capacity = GeometryCapacity {
		meshlet_vertices: false,
		..GeometryCapacity::default()
	};
	let handles = GeometryHandles::new(&mut render, capacity, true);
	let mut loader = Loader::new(&device, [].as_slice());
	let geometry = GeometryBuffers::import(&handles, &render, &mut loader);
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

	let runtime =
		compio::runtime::Runtime::new().map_err(|error| format!("that the bake's task runtime couldn't start: {error}"))?;
	loader.run(&runtime);
	for lane in lanes {
		runtime.spawn(lane.run()).detach();
	}
	let shader_resources = EntityHandle::from(shaders);
	for server in servers {
		runtime.spawn(server.run(shader_resources.clone())).detach();
	}

	runtime.block_on(async move {
		let loaded = load_scene(&mut client, &scene, &settings).await?;
		let texels = rasterize(&scene, &loaded, &settings)?;
		let mut bake = Bake::new(
			&mut render,
			&device,
			&handles,
			pipelines,
			pipeline_manager,
			&scene,
			loaded,
			&texels,
			&settings,
		);
		bake.prepare(&mut render).await?;
		bake.trace(&mut render, &texels, &settings);
		let light = bake.dilate(&mut render, texels.atlas.extent, settings.samples);
		Ok(TracedLightmap {
			extent: texels.atlas.extent,
			light,
			instances: texels.instances,
			shrunk: texels.atlas.shrunk,
			unlit: texels.unlit,
		})
	})
}

/* Loading */

/// The `BakeLoad` struct loads what the bake traces on the engine's loading timeline, as the visibility loader loads
/// it for an application.
struct BakeLoad {
	resources: ResourceManager,
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

/// The `BakeTexels` struct is the image's layout and the texels that trace paths.
struct BakeTexels {
	atlas: Atlas,
	texels: Vec<LightmapTexel>,
	/// The traced texel each image texel holds, or [`NO_LIGHTMAP_TEXEL`].
	pixels: Vec<u32>,
	/// The groups of texels that share a guide.
	groups: GuideGroups,
	instances: Vec<LightmapInstance>,
	unlit: Vec<(u32, u32)>,
}

/// Lays out a rectangle for every primitive of `scene` that has lightmap UVs and draws its triangles into it.
fn rasterize(scene: &LevelScene, loaded: &LoadedScene, settings: &TraceSettings) -> Result<BakeTexels, String> {
	struct Lit {
		entity: u32,
		primitive: u32,
		positions: Vec<[f32; 3]>,
		normals: Vec<[f32; 3]>,
		mesh: MeshKey,
	}
	let mut lit = Vec::new();
	let mut unlit = Vec::new();
	let mut sides = Vec::new();
	for mesh in &scene.meshes {
		let key = mesh.source.key();
		let Some((_, surfaces)) = loaded.meshes.get(&key) else {
			continue;
		};
		let rows = model_rows(&mesh.transform);
		for (primitive, surface) in surfaces.iter().enumerate() {
			let Some(surface) = surface else {
				continue;
			};
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
				unlit.push((mesh.entity, primitive as u32));
				continue;
			}
			sides.push(atlas::rectangle_side(world_area, uv_area, settings.texel_density));
			lit.push(Lit {
				entity: mesh.entity,
				primitive: primitive as u32,
				positions,
				normals,
				mesh: key,
			});
		}
	}
	let atlas = atlas::layout(&sides, settings.max_extent).ok_or_else(|| {
		format!(
			"that the level has more lit primitives than a {0} × {0} image holds",
			settings.max_extent
		)
	})?;

	let extent = atlas.extent;
	let mut texels = Vec::new();
	let mut pixels = vec![NO_LIGHTMAP_TEXEL; (extent * extent) as usize];
	let mut raster = TexelRaster::default();
	let mut instances = Vec::with_capacity(lit.len());
	for (index, lit) in lit.iter().enumerate() {
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
			atlas.rectangles[index],
			extent,
			&mut texels,
			&mut pixels,
		);
		instances.push(LightmapInstance {
			entity: lit.entity,
			primitive: lit.primitive,
			scale_offset: atlas.scale_offset(index),
		});
	}
	instances.sort_by_key(|instance| (instance.entity, instance.primitive));
	// A group spans four texels' widths at the density the atlas kept.
	let density = settings.texel_density * atlas.shrunk.unwrap_or(1.0);
	let groups = group_texels(&texels, GROUP_TEXELS / density);
	Ok(BakeTexels {
		atlas,
		texels,
		pixels,
		groups,
		instances,
		unlit,
	})
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
	scene_set: ghi::DescriptorSetHandle,
	lights_set: ghi::DescriptorSetHandle,
	bake_set: ghi::DescriptorSetHandle,
	bake_layout: SetLayout,
	parameters: ghi::DynamicBufferHandle<LightmapBakeParameters>,
	environment_intensity: f32,
	light: ghi::BufferHandle<[[f32; 4]]>,
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
		loaded: LoadedScene,
		texels: &BakeTexels,
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
		let mut alpha_modes = vec![AlphaMode::Opaque; MAX_MATERIALS];
		let mut double_sided = vec![false; MAX_MATERIALS];
		let mut parameters = MaterialParameters::new();
		for (id, slot, record, description) in &loaded.materials {
			let index = *slot as usize;
			materials[index] = *record;
			alpha_modes[index] = description.alpha_mode;
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
		lighting.count = scene.lights.len().min(MAX_LIGHTS) as u32;
		lighting.exposure = 1.0;
		lighting.environment_intensity = environment_intensity;
		for (destination, light) in lighting.lights.iter_mut().zip(&scene.lights) {
			*destination = *light;
		}

		let host = |name, length: usize| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.length(length)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let meshes_buffer = upload(render, host("Lightmap Bake Meshes", meshes.len()), &meshes);
		let materials_buffer = upload(render, host("Lightmap Bake Materials", MAX_MATERIALS), &materials);
		let (first, values) = parameters.take_changes(0).unwrap_or((0, &[]));
		let mut parameter_values = vec![[0.0f32; 4]; (first + values.len()).max(1)];
		parameter_values[first..first + values.len()].copy_from_slice(values);
		let parameters_buffer = upload(
			render,
			host("Lightmap Bake Material Parameters", parameter_values.len()),
			&parameter_values,
		);
		let lighting_buffer = render.build_buffer::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Bake Lights")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		*render.get_mut_buffer_slice(lighting_buffer) = *lighting;
		render.sync_buffer(lighting_buffer);
		// The hit shaders read no view, dispatch, or evaluation records, but the scene set declares them.
		let unused = upload(render, host("Lightmap Bake Unused Records", 1 << 16), &vec![0u32; 1 << 16]);

		let scene_set = set_layouts::SCENE.create_set(render, "Lightmap Bake Scene");
		let binder = set_layouts::SCENE.binder(scene_set);
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
		render.write(&[lights_layout.binder(lights_set).buffer("scene_lights", lighting_buffer)]);

		// The texels, their sums, and the image.
		let bake_layout = SetLayout::from_layer(
			"::byte_engine::rendering::lightmaps::layouts::lightmap_bake::LightmapBake",
			ENGINE_MODULES,
		);
		let bake_set = bake_layout.create_set(render, "Lightmap Bake");
		let texel_count = texels.texels.len().max(1);
		let image_texels = texels.pixels.len();
		let texels_buffer = upload(
			render,
			host("Lightmap Texels", texel_count),
			&pad(&texels.texels, texel_count),
		);
		let indices_buffer = upload(render, host("Lightmap Texel Indices", image_texels), &texels.pixels);
		let accumulation = upload(
			render,
			host("Lightmap Accumulation", texel_count),
			&vec![[0.0f32; 4]; texel_count],
		);
		let escapes = upload(render, host("Lightmap Escapes", texel_count), &vec![0.0f32; texel_count]);
		let groups = &texels.groups;
		let group_count = groups.groups.len().max(1);
		let groups_buffer = upload(
			render,
			host("Lightmap Guide Groups", group_count),
			&pad(&groups.groups, group_count),
		);
		let group_texels_buffer = upload(
			render,
			host("Lightmap Guide Group Texels", texel_count),
			&pad(&groups.texels, texel_count),
		);
		let texel_groups_buffer = upload(
			render,
			host("Lightmap Texel Guide Groups", texel_count),
			&pad(&groups.texel_groups, texel_count),
		);
		let device = |name, length: usize| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.length(length)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		// The learning batches fill both before any guided batch reads them, so they start unwritten.
		let guide_cells = render.build_buffer::<[f32]>(device("Lightmap Guide Cells", group_count * GUIDE_CELLS));
		let guides = render.build_buffer::<[f32]>(device("Lightmap Guides", group_count * GUIDE_STRIDE));
		let light = render.build_buffer::<[[f32; 4]]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Light")
				.length(image_texels)
				.device_accesses(ghi::DeviceAccesses::DeviceToHost),
		);
		let dilated = render.build_buffer::<[[f32; 4]]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Dilated")
				.length(image_texels)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let parameters = render.build_dynamic_buffer::<LightmapBakeParameters>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lightmap Bake Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let bake_binder = bake_layout.binder(bake_set);
		render.write(&[
			bake_binder.buffer("lightmap_parameters", parameters),
			bake_binder.buffer("lightmap_texels", texels_buffer),
			bake_binder.buffer("lightmap_accumulation", accumulation),
			bake_binder.buffer("lightmap_texel_indices", indices_buffer),
			bake_binder.buffer("lightmap_light", light),
			bake_binder.buffer("lightmap_dilated", dilated),
			bake_binder.buffer("lightmap_escapes", escapes),
			bake_binder.buffer("lightmap_groups", groups_buffer),
			bake_binder.buffer("lightmap_group_texels", group_texels_buffer),
			bake_binder.buffer("lightmap_texel_groups", texel_groups_buffer),
			bake_binder.buffer("lightmap_guide_cells", guide_cells),
			bake_binder.buffer("lightmap_guides", guides),
		]);

		let environment_sampler = render.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));
		if loaded.environment.is_none() {
			// Paths that leave a scene without an environment bring back no light.
			let fallback = create_fallback_environment(render, environment_sampler);
			render.write(&[bake_binder.sampled("lightmap_environment", fallback.specular_image, environment_sampler)]);
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
			pending_images: loaded.textures,
			environment: loaded.environment,
			environment_sampler,
			scene_set,
			lights_set,
			bake_set,
			bake_layout,
			parameters,
			environment_intensity,
			light,
		}
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
		let segments_per_sample = u64::from(texel_count) * u64::from(settings.bounces) * 2;
		let samples_per_batch = (SEGMENTS_PER_BATCH / segments_per_sample).clamp(1, u64::from(settings.samples)) as u32;
		// A learning batch ends where learning does, so the group's last one builds the guide before any guided batch.
		for (start, end, launches) in [
			(0, learning_samples, group_count),
			(learning_samples, settings.samples, texel_count),
		] {
			let mut sample_offset = start;
			while sample_offset < end {
				let batch = samples_per_batch.min(end - sample_offset);
				let parameters = LightmapBakeParameters {
					texel_count,
					sample_offset,
					samples_per_batch: batch,
					samples: settings.samples,
					bounces: settings.bounces,
					environment_intensity: self.environment_intensity,
					extent: texels.atlas.extent,
					pass: 0,
					learning_samples,
					group_count,
				};
				self.run_frame(render, parameters, FrameWork::Trace { launches });
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
	fn dilate(&mut self, render: &mut ghi::implementation::Context, extent: u32, samples: u32) -> Vec<[f32; 4]> {
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
				FrameWork::Dilate {
					pipeline,
					image_texels: extent * extent,
				},
			);
		}
		render.get_buffer_slice(self.light).to_vec()
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
					frame.write(&[set_layouts::SCENE.binder(*scene_set).texture_element("textures", image, slot)]);
				}
				if let Some(environment) = environment.take() {
					let specular = frame.intern_image(environment.specular_image);
					frame.write(&[bake_layout.binder(*bake_set).sampled(
						"lightmap_environment",
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
				*frame.get_mut_dynamic_buffer_slice(*parameters_buffer) = parameters;
				frame.sync_buffer(*parameters_buffer);
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
						FrameWork::Dilate { pipeline, image_texels } => {
							recording
								.bind_compute_pipeline(pipeline)
								.bind_descriptor_sets(&[*bake_set])
								.dispatch(ghi::DispatchExtent::new(Extent::line(image_texels), Extent::line(64)));
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
	/// One dilation pass over the image's `image_texels` texels.
	Dilate {
		pipeline: ghi::PipelineHandle,
		image_texels: u32,
	},
}

/// Builds a host-written buffer from `builder` and fills it with `values`.
fn upload<T: bytemuck::Pod>(
	render: &mut ghi::implementation::Context,
	builder: ghi::buffer::Builder,
	values: &[T],
) -> ghi::BufferHandle<[T]> {
	let buffer = render.build_buffer::<[T]>(builder.length(values.len()));
	// A backend may round the allocation up, which gives the buffer more elements than were asked for.
	render.get_mut_buffer_slice(buffer)[..values.len()].copy_from_slice(values);
	render.sync_buffer(buffer);
	buffer
}

/// Returns `values` padded with zeroed elements to `length`, since a buffer holds at least one element.
fn pad<T: bytemuck::Pod>(values: &[T], length: usize) -> Vec<T> {
	let mut padded = values.to_vec();
	padded.resize(length, bytemuck::Zeroable::zeroed());
	padded
}
