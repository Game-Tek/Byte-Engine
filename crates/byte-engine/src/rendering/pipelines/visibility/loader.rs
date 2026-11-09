//! Pipeline-level loader-thread residency for every visibility resource.
//!
//! One request registry and one lane pool serve meshes, materials, textures, environments, and photometric
//! images. Resource-specific work stays in focused methods. Each load requests the resources its metadata names as
//! soon as it reads them, so meshes, materials, and textures load at the same time.
//!
//! The renderer interacts only with [`VisibilityLoaderClient`]. It submits domain requests and receives
//! typed ready or unavailable events; generic keys, requests, worker residents, lane types, and material
//! compilation state remain inside this module.

use std::sync::{Arc, Mutex};

use ghi::frame::Frame as _;
use resource_management::Reference;
use resource_management::bemx;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::{
	image::{Image as ResourceImage, ImagePhotometry},
	lightmap::{Lightmap as ResourceLightmap, LightmapInstance},
	material::{AssembledShader, Value, Variant as ResourceVariant},
	mesh::Mesh,
};
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::Extent;
use utils::hash::HashMap;

use super::features::{COVERAGE_SHADERS, MaterialEvaluationMode, UBER_MATERIAL_SHADERS};
use super::geometry::{
	GENERATED_MESH_MATERIAL, GeometryBuffers, GeometryHandles, GeometryRanges, MeshData, PreparedLevels, PreparedMesh,
	PrimitiveBlock, PrimitiveLevels, RetiredRanges,
};
use super::layout::{
	MAX_BINDLESS_TEXTURES, MAX_IES_PROFILES, MAX_INSTANCE_MESHLETS, MAX_LIGHTMAPS, MAX_MATERIALS,
	TRANSPARENT_SPECIALIZATION_ID, material_variable_specialization_id,
};
use super::lod::MeshLods;
use super::render_pass::MATERIAL_EVALUATION_PUSH_CONSTANT_SIZE;
use super::shader_generator::{coverage_arm_key, uber_arm_key};
use crate::core::EntityHandle;
use crate::rendering::loading::{
	Event as LoaderEvent, ImageDescription, ImageUpload, LoadError, LoadPipeline, Loader, LoaderClient, LoaderLane, Priority,
	spawn as spawn_lanes,
};
use crate::rendering::pipeline_compilation::{HitGroupShader, SpecializedComputePipelineRequest};
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::texture::{TextureUploadLayout, load_image_streams, resource_format_to_ghi};
use crate::rendering::resource_loading::{UploadStagingArena, load_texture};
use crate::rendering::{PipelineManagerClient, PipelineRef, PipelineState};
use crate::rendering::{Query, Resource};

/// Number of prefiltered specular roughness levels stored by a baked environment.
pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;

const VISIBILITY_LANE_COUNT: usize = 4;
/// How many of the [`VISIBILITY_LANE_COUNT`] lanes also load finer mesh levels, full mip chains, and lightmaps, so
/// the other two are always free for what a scene can't draw without.
const VISIBILITY_BACKGROUND_LANE_COUNT: usize = 2;
/// The largest side, in texels, of the mips a material texture loads before its full chain, so its materials draw
/// while full chains load. See [`coarse_first_level`].
const COARSE_TEXTURE_EXTENT: u32 = 128;
const VISIBILITY_RESULT_CAPACITY: usize = 64;

/// The `VisibilityLoadKey` enum names every logical resource in the visibility pipeline's shared registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityLoadKey {
	Resource(Arc<str>),
	Query(Query),
	Mesh(MeshKey),
	/// The block of a resource mesh's finer detail levels. It reloads whenever its finest level changes.
	MeshLevels(MeshKey),
	Material(String),
	Texture(String),
	/// The full mip chain of a texture that loaded its coarse mips first.
	TextureDetail(String),
	IesProfile(String),
	Environment(String),
	Lightmap(String),
	/// The uber material evaluation shader of the transparent phase when `true`, of the opaque phase otherwise.
	UberShader(bool),
	/// Both coverage shaders and the masked raster pipelines they are the fragments of.
	CoverageShaders,
}

impl std::fmt::Display for VisibilityLoadKey {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Resource(id) => write!(formatter, "resource {id}"),
			Self::Query(query) => write!(formatter, "query {query:?}"),
			Self::Mesh(key) => write!(formatter, "mesh {key}"),
			Self::Material(id) => write!(formatter, "material {id}"),
			Self::Texture(id) => write!(formatter, "texture {id}"),
			Self::TextureDetail(id) => write!(formatter, "full mip chain of texture {id}"),
			Self::IesProfile(id) => write!(formatter, "IES profile {id}"),
			Self::Environment(id) => write!(formatter, "environment {id}"),
			Self::Lightmap(id) => write!(formatter, "lightmap {id}"),
			Self::MeshLevels(key) => write!(formatter, "finer detail levels of mesh {key:?}"),
			Self::UberShader(transparent) => {
				write!(formatter, "uber shader {}", UBER_MATERIAL_SHADERS[usize::from(*transparent)])
			}
			Self::CoverageShaders => write!(formatter, "coverage shaders {COVERAGE_SHADERS:?}"),
		}
	}
}

/// The `VisibilityLoadRequest` enum carries owned work for every visibility resource family.
pub(crate) enum VisibilityLoadRequest {
	/// A resource of unknown class, routed to its family once its class is read.
	Resource(Arc<str>),
	/// Every resource that matches a query, each routed by the query's class.
	Query(Query),
	/// A mesh with each primitive's coarsest detail level.
	Mesh(MeshSource),
	/// Finer detail levels of some primitives of resource mesh `id`, in ascending primitive order. Each primitive's
	/// levels from its finest listed one down to its coarsest replace the finer levels it held.
	MeshLevels {
		id: Arc<str>,
		primitives: Vec<PrimitiveLevels>,
	},
	Material(String),
	/// A texture, which loads only its coarse mips first when it has larger ones; see [`coarse_first_level`].
	Texture(String),
	/// The full mip chain of a texture whose coarse mips loaded first. It replaces the coarse image in the same slot.
	TextureDetail(String),
	/// A light's calibrated IES intensity profile, which loads with its full mip chain.
	IesProfile(String),
	Environment(String),
	/// A baked lightmap and its image.
	Lightmap(String),
	/// The uber material evaluation shader of one phase, the transparent one when `transparent` is set. `rebake` bakes
	/// it again first, from every material the store now holds; only development builds can.
	UberShader {
		transparent: bool,
		rebake: bool,
	},
	/// Both coverage shaders, and the masked raster pipelines that take them as fragments. `rebake` bakes them again
	/// first, from every material the store now holds; only development builds can.
	CoverageShaders {
		rebake: bool,
	},
}

/// The `CoveragePipelines` struct holds the masked raster pipelines: each pass's one-sided and double-sided pipeline,
/// with a coverage shader as its fragment. The loader holds them as their requests, `PipelineRef`, and the renderer as
/// the compiled `ghi::PipelineHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CoveragePipelines<P> {
	/// The visibility pass's masked pipelines, one-sided then double-sided.
	pub(crate) visibility: [P; 2],
	/// The directional shadow pass's masked pipelines, one-sided then double-sided.
	pub(crate) directional: [P; 2],
	/// The local shadow pass's masked pipelines, one-sided then double-sided.
	pub(crate) local: [P; 2],
}

impl<P: Copy> CoveragePipelines<P> {
	/// Returns the pipelines with `f` applied to each, or `None` as soon as `f` returns `None` for one.
	fn try_map<Q>(self, mut f: impl FnMut(P) -> Option<Q>) -> Option<CoveragePipelines<Q>> {
		let mut pair = |[one_sided, double_sided]: [P; 2]| Some([f(one_sided)?, f(double_sided)?]);
		Some(CoveragePipelines {
			visibility: pair(self.visibility)?,
			directional: pair(self.directional)?,
			local: pair(self.local)?,
		})
	}
}

/// The `MaterialEvaluator` enum names what evaluates a material, by the mode the project chose. The loader holds a
/// pipeline as its request, `PipelineRef`, and the renderer as the compiled `ghi::PipelineHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaterialEvaluator<P> {
	/// The material's own specialized evaluation pipeline.
	Pipeline(P),
	/// The material's arm key in the uber shader of its phase; see [`super::shader_generator::uber_arm_key`].
	Arm(u32),
}

/// The `ResidentMaterial` struct carries one loaded material's table data from a loader lane to renderer-owned draw
/// state.
///
/// The loader keeps it until its specialized pipeline compiles, then publishes it with that compiled pipeline in
/// [`VisibilityLoaderEvent::MaterialReady`]. A material the uber shaders evaluate is published as soon as it loads.
#[derive(Clone)]
pub(crate) struct ResidentMaterial {
	pub(crate) id: String,
	pub(crate) index: u32,
	pub(crate) evaluation: MaterialEvaluator<PipelineRef>,
	pub(crate) alpha_mode: AlphaMode,
	pub(crate) double_sided: bool,
	/// The material's key in the coverage shaders, when it is masked; see [`coverage_arm_key`].
	pub(crate) coverage_arm: Option<u32>,
	pub(crate) texture_slots: Vec<Option<u32>>,
	/// The material's dynamic parameters with their default values, in the order its program reads them.
	pub(crate) parameters: Arc<[DynamicParameter]>,
	/// The hit group rays that hit the material run, when its bake built a closest-hit shader.
	pub(crate) hit_group: Option<HitGroupShader>,
	/// Whether the material's graph reads the opaque scene behind its surface, which the transparent phase copies only
	/// for such materials.
	pub(crate) reads_backdrop: bool,
}

/// The `DynamicParameter` struct is one value parameter of a material that the application can change at runtime.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DynamicParameter {
	pub(crate) name: String,
	/// The value the material starts with, which also fixes the parameter's type.
	pub(crate) default: bemx::Value,
}

/// The `TextureArray` enum names the texture arrays of the scene set. Each has its own slots and the sampler its
/// shader declaration fixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TextureArray {
	/// Material textures, which repeat outside their bounds.
	Material,
	/// Lights' calibrated IES intensity profiles.
	IesProfile,
	/// Baked lightmaps.
	Lightmap,
}

impl TextureArray {
	/// Returns the name of the array's binding in the scene set.
	pub(crate) fn binding(self) -> &'static str {
		match self {
			Self::Material => "textures",
			Self::IesProfile => "ies_profiles",
			Self::Lightmap => "lightmaps",
		}
	}

	/// Returns how many slots the array has.
	fn capacity(self) -> usize {
		match self {
			Self::Material => MAX_BINDLESS_TEXTURES,
			Self::IesProfile => MAX_IES_PROFILES,
			Self::Lightmap => MAX_LIGHTMAPS,
		}
	}

	/// Returns what the array holds, as load errors name it.
	fn kind(self) -> &'static str {
		match self {
			Self::Material => "texture",
			Self::IesProfile => "IES profile",
			Self::Lightmap => "lightmap",
		}
	}
}

/// The `ResidentTexture` struct carries one upload-complete texture into renderer-owned descriptors.
pub(crate) struct ResidentTexture {
	pub(crate) id: String,
	pub(crate) array: TextureArray,
	pub(crate) index: u32,
	pub(crate) image: ghi::BaseImageHandle,
	pub(crate) photometry: Option<ImagePhotometry>,
}

/// The `ResidentLightmap` struct carries an upload-complete lightmap: where each primitive of its level lies in it, and
/// its image, which the render thread places in its slot of the scene set's `lightmaps`.
pub(crate) struct ResidentLightmap {
	pub(crate) id: String,
	/// The spawn index of the level entity that holds the `lightmap` kind.
	pub(crate) holder: u32,
	pub(crate) instances: Vec<LightmapInstance>,
	pub(crate) texture: ResidentTexture,
}

/// The `ResidentEnvironment` struct carries upload-complete image-based lighting handles.
#[derive(Clone, Copy)]
pub(crate) struct ResidentEnvironment {
	pub(crate) diffuse_image: ghi::BaseImageHandle,
	pub(crate) specular_image: ghi::BaseImageHandle,
	pub(crate) sampler: ghi::SamplerHandle,
	/// The illuminance, in the map's own units, that the environment delivers to an upward-facing surface. It's what
	/// [`crate::rendering::Environment::with_illuminance`] scales to the requested lux.
	pub(crate) upward_illuminance: f32,
}

/// The `VisibilityResident` enum keeps generic loader results private to the loader boundary.
pub(crate) enum VisibilityResident {
	/// A routed resource; its family request travels as the only dependency.
	Routed,
	/// A resident mesh and the key it was requested by.
	Mesh(MeshKey, MeshData),
	/// Finer detail levels of some primitives of a mesh: the requested primitives, each with its block when the
	/// geometry buffers had room for it.
	MeshLevels {
		key: MeshKey,
		primitives: Vec<PrimitiveLevels>,
	},
	Material(ResidentMaterial),
	/// An upload-complete texture whose image the render thread still interns.
	Texture {
		id: String,
		array: TextureArray,
		index: u32,
		image: ghi::implementation::DetachedImage,
		/// The resource mip that is the image's base level: 0 for the full chain.
		first_level: u32,
		photometry: Option<ImagePhotometry>,
	},
	/// An upload-complete lightmap whose image the render thread still interns.
	Lightmap {
		id: String,
		holder: u32,
		instances: Vec<LightmapInstance>,
		index: u32,
		image: ghi::implementation::DetachedImage,
	},
	/// Upload-complete image-based lighting whose images the render thread still interns.
	Environment {
		id: String,
		diffuse_image: ghi::implementation::DetachedImage,
		specular_image: ghi::implementation::DetachedImage,
		upward_illuminance: f32,
	},
	/// A phase's uber shader, whose pipeline may still be compiling, and the arm keys it holds.
	UberShader {
		transparent: bool,
		keys: Arc<[u32]>,
		pipeline: PipelineRef,
	},
	/// The coverage shaders' arm keys and the masked raster pipelines, which may still be compiling.
	CoverageShaders {
		keys: Arc<[u32]>,
		pipelines: CoveragePipelines<PipelineRef>,
	},
}

/// The `VisibilityLoaderEvent` enum is the renderer's complete view of visibility resource loading.
pub(crate) enum VisibilityLoaderEvent {
	MeshReady {
		key: MeshKey,
		mesh: Arc<MeshData>,
	},
	/// A load of finer levels of `key`'s primitives finished. Each primitive with a block now holds its levels from its
	/// finest listed one down; read its table with [`VisibilityLoaderClient::primitive_lods`]. The geometry buffers had
	/// no room for the primitives without one, which placed nothing. Hand the list back to the residencies, which reuse
	/// it.
	MeshLevelsLoaded {
		key: MeshKey,
		primitives: Vec<PrimitiveLevels>,
	},
	/// A mesh's finer levels failed to load for a reason more room won't fix.
	MeshLevelsUnavailable {
		key: MeshKey,
		error: LoadError,
	},
	/// A mesh failed to load, so the renderables waiting for it never become resident.
	MeshUnavailable {
		key: MeshKey,
		error: LoadError,
	},
	/// A material that can be drawn, with what evaluates it: its compiled pipeline or its uber shader arm.
	MaterialReady(ResidentMaterial, MaterialEvaluator<ghi::PipelineHandle>),
	/// A phase's uber shader compiled: the transparent phase's when `transparent` is set. It holds the arms of `keys`,
	/// in ascending order.
	UberShaderReady {
		transparent: bool,
		keys: Arc<[u32]>,
		pipeline: ghi::PipelineHandle,
	},
	/// A phase's uber shader failed to load or compile; the transparent phase's when `transparent` is set. The phase
	/// keeps the shader it had.
	UberShaderUnavailable {
		transparent: bool,
		error: LoadError,
	},
	/// The coverage shaders compiled into every masked raster pipeline. They hold the arms of `keys`, in ascending
	/// order.
	CoverageShadersReady {
		keys: Arc<[u32]>,
		pipelines: CoveragePipelines<ghi::PipelineHandle>,
	},
	/// The coverage shaders failed to load or compile. The masked passes keep the pipelines they had.
	CoverageShadersUnavailable {
		error: LoadError,
	},
	MaterialUnavailable {
		index: u32,
	},
	TextureReady(ResidentTexture),
	LightmapReady(ResidentLightmap),
	EnvironmentReady {
		id: String,
		environment: ResidentEnvironment,
	},
	Unavailable {
		resource: String,
		error: LoadError,
	},
}

/// The `VisibilityLoader` struct owns all resource loading and GPU placement for the visibility pipeline.
pub(crate) struct VisibilityLoader {
	resource_manager: EntityHandle<ResourceManager>,
	/// Requests each material's specialized material-evaluation pipeline.
	pipeline_manager: PipelineManagerClient,
	geometry: Mutex<GeometryBuffers>,
	/// Whether meshes keep flat triangle lists for ray tracing; see [`GeometryHandles::flat_triangle_indices`].
	flat_triangles: bool,
	/// Whether a material without a closest-hit shader was reported while the pipeline traces rays.
	reported_missing_hit_shader: std::sync::atomic::AtomicBool,
	material_slots: Mutex<HashMap<String, u32>>,
	/// The slot of each resident texture, by texture ID, in each array of [`TextureArray`] order.
	texture_slots: [Mutex<HashMap<String, u32>>; 3],
	/// How the project evaluates materials, which decides what a material load requests.
	material_evaluation: MaterialEvaluationMode,
}

/// The `VisibilityLoaderClient` struct hides the generic loading protocol from the visibility renderer.
pub(crate) struct VisibilityLoaderClient {
	client: LoaderClient<VisibilityLoader>,
	pipeline_manager: PipelineManagerClient,
	/// Resident meshes, shared with every renderable that uses them. Each holds its primitives' coarsest detail levels.
	meshes: HashMap<MeshKey, Arc<MeshData>>,
	/// Each resident resource mesh's ID, which its finer levels load from.
	mesh_ids: HashMap<MeshKey, Arc<str>>,
	/// The finer detail levels of primitives some instance wanted, by mesh and primitive index, in ranges of their own.
	refined: HashMap<(MeshKey, u32), PrimitiveBlock>,
	/// The free space of the geometry streams, which released meshes return to.
	geometry_ranges: Arc<Mutex<GeometryRanges>>,
	/// Released meshes' ranges that frames in flight may still read.
	retired_ranges: RetiredRanges,
	/// The first frame that has not started yet, which is the first that cannot draw a mesh released now.
	next_frame: u64,
	/// The image in each texture array slot, with the resource mip that is its base level.
	texture_images: HashMap<(TextureArray, u32), (ghi::ImageHandle, u32)>,
	/// Images finer ones replaced in their slots this frame, deleted once the slots name their replacements.
	replaced_images: Vec<ghi::ImageHandle>,
	environments: HashMap<String, ResidentEnvironment>,
	materials: HashMap<u32, MaterialPublication>,
	/// Each phase's latest uber shader, opaque then transparent, until its pipeline compiles.
	uber_shaders: [Option<UberShaderPublication>; 2],
	/// The latest coverage shaders' keys and masked pipelines, until every pipeline compiles.
	coverage_shaders: Option<(Arc<[u32]>, CoveragePipelines<PipelineRef>)>,
	/// Reports rebaked material variants, so their materials load again.
	#[cfg(debug_assertions)]
	resource_updates: resource_management::resource::ResourceUpdateListener,
	/// Samples environment maps across their prefiltered roughness levels, and the one-level fallback environment.
	pub(super) environment_sampler: ghi::SamplerHandle,
}

/// The `MaterialPublication` struct tracks the last compilation state reported to the renderer.
struct MaterialPublication {
	material: ResidentMaterial,
	published: Option<PipelineState>,
}

/// The `UberShaderPublication` struct holds a phase's uber shader until its pipeline compiles.
struct UberShaderPublication {
	keys: Arc<[u32]>,
	pipeline: PipelineRef,
}

/// Returns or assigns a stable shader-table slot for `id`, failing once `limit` slots exist.
pub(crate) fn assign_slot(slots: &mut HashMap<String, u32>, id: &str, limit: usize, kind: &str) -> Option<u32> {
	if let Some(index) = slots.get(id) {
		return Some(*index);
	}
	if slots.len() >= limit {
		log::error!(
			"Visibility {kind} limit exceeded. The most likely cause is that the scene created more {kind} variants than the visibility pipeline supports."
		);
		return None;
	}
	let index = slots.len() as u32;
	slots.insert(id.to_string(), index);
	Some(index)
}

/// The `TextureMips` enum chooses which mips one texture load uploads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureMips {
	/// The coarse mips alone when the image has larger ones, and its full chain later; see [`coarse_first_level`].
	CoarseFirst,
	/// Every mip at once.
	Full,
}

/// Returns the first mip of an `extent` image with `mip_count` mips whose larger side is at most
/// [`COARSE_TEXTURE_EXTENT`] texels, or 0 when the base level already is or no mip gets that small.
pub(super) fn coarse_first_level(extent: [u32; 3], mip_count: u32) -> u32 {
	let largest = extent[0].max(extent[1]);
	(0..mip_count)
		.find(|&level| largest.checked_shr(level).unwrap_or(0) <= COARSE_TEXTURE_EXTENT)
		.unwrap_or(0)
}

/// Builds the default sampler used by visibility material textures.
///
/// Anisotropic filtering keeps surfaces seen at grazing angles, like floors, sharp instead of blurring them to the mip
/// their steepest direction needs.
pub(crate) fn material_sampler() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.addressing_mode(ghi::SamplerAddressingModes::Repeat)
		.anisotropy(16.0)
}

/// Returns whether an image can safely provide the normalized Type C IES intensity-map contract.
fn photometric_profile_metadata_is_valid(image: &ResourceImage, photometry: &ImagePhotometry) -> bool {
	image.format == resource_management::types::Formats::R16F
		&& image.gamma == resource_management::types::Gamma::Linear
		&& image.extent[2] == 0
		&& image.mip_count == 1
		&& photometry.intensity_scale_candela.is_finite()
		&& photometry.intensity_scale_candela > 0.0
}

impl VisibilityLoaderClient {
	/// Requests one resource, or every query match, and loads each as whichever family its stored class names.
	pub(crate) fn request_resource(&mut self, resource: Resource) {
		self.client.request(match resource {
			Resource::Id(id) => VisibilityLoadRequest::Resource(id),
			Resource::Query(query) => VisibilityLoadRequest::Query(query),
		});
	}

	/// Requests one mesh and reports whether that mesh was already resident.
	pub(crate) fn request_mesh(&mut self, source: MeshSource) -> (MeshKey, Option<Arc<MeshData>>) {
		let key = source.key();
		let resident = self.meshes.get(&key).cloned();
		if let MeshSource::Resource(id) = &source {
			self.mesh_ids.entry(key).or_insert_with(|| id.clone());
		}
		self.client.request(VisibilityLoadRequest::Mesh(source));
		(key, resident)
	}

	/// Stops publishing `key`'s mesh and frees its geometry once no frame in flight reads it.
	///
	/// Call it only after every renderable that draws the mesh left the scene, since their instances still name its
	/// ranges. The loader still counts the key as requested, so load the mesh again with a reload, not
	/// [`Self::request_mesh`].
	#[expect(dead_code, reason = "Mesh detail residency, the next step, evicts meshes through it.")]
	pub(crate) fn release_mesh(&mut self, key: MeshKey) {
		if let Some(mesh) = self.meshes.remove(&key) {
			self.retired_ranges.retire(mesh.ranges.clone(), self.next_frame);
		}
	}

	/// Loads finer detail levels of some of `key`'s primitives in one load: each listed primitive from its finest listed
	/// level down to its coarsest, replacing the finer levels it holds.
	///
	/// The result arrives as [`VisibilityLoaderEvent::MeshLevelsLoaded`]. Returns `false`, and loads nothing, for a mesh
	/// that isn't a resident resource mesh.
	pub(crate) fn request_mesh_levels(&mut self, key: MeshKey, primitives: Vec<PrimitiveLevels>) -> bool {
		let Some(id) = self.mesh_ids.get(&key).filter(|_| self.meshes.contains_key(&key)) else {
			return false;
		};
		// A reload, because the key stays registered after each load and after a release.
		self.client.reload(VisibilityLoadRequest::MeshLevels {
			id: id.clone(),
			primitives,
		});
		true
	}

	/// Releases the finer detail levels of one primitive of `key`, which frees their ranges once no frame in flight
	/// reads them. The primitive keeps its coarsest level. Update the primitive's render entities from
	/// [`Self::primitive_lods`] before the next frame draws.
	pub(crate) fn release_mesh_levels(&mut self, key: MeshKey, primitive: u32) {
		if let Some(block) = self.refined.remove(&(key, primitive)) {
			self.retired_ranges.retire(block.ranges, self.next_frame);
		}
	}

	/// Returns the detail level table of one primitive of a resident mesh, with every level it holds marked resident.
	pub(crate) fn primitive_lods(&self, key: MeshKey, primitive: usize) -> Option<MeshLods> {
		let mut lods = self.meshes.get(&key)?.primitives.get(primitive)?.lods;
		if let Some(block) = self.refined.get(&(key, primitive as u32)) {
			lods.overlay(&block.lods);
		}
		Some(lods)
	}

	/// Requests a light's IES intensity profile. It reaches the renderer as a [`VisibilityLoaderEvent::TextureReady`] in
	/// [`TextureArray::IesProfile`].
	pub(crate) fn request_ies_profile(&mut self, id: String) {
		self.client.request(VisibilityLoadRequest::IesProfile(id));
	}

	/// Requests the uber material evaluation shader of each phase. Each reaches the renderer as
	/// [`VisibilityLoaderEvent::UberShaderReady`] once its pipeline compiles.
	pub(crate) fn request_uber_shaders(&mut self) {
		for transparent in [false, true] {
			self.client.request(VisibilityLoadRequest::UberShader {
				transparent,
				rebake: false,
			});
		}
	}

	/// Requests both coverage shaders. They reach the renderer as [`VisibilityLoaderEvent::CoverageShadersReady`] once
	/// every masked raster pipeline compiles.
	pub(crate) fn request_coverage_shaders(&mut self) {
		self.client.request(VisibilityLoadRequest::CoverageShaders { rebake: false });
	}

	/// Bakes both coverage shaders again, from every material the store now holds, and loads them. Development builds
	/// call it when a masked material the published shaders lack is ready.
	#[cfg(debug_assertions)]
	pub(crate) fn rebake_coverage_shaders(&mut self) {
		self.client.reload(VisibilityLoadRequest::CoverageShaders { rebake: true });
	}

	/// Bakes the uber material evaluation shader of one phase again, from every material the store now holds, and
	/// loads it. Development builds call it when a material the published shader lacks is ready.
	#[cfg(debug_assertions)]
	pub(crate) fn rebake_uber_shader(&mut self, transparent: bool) {
		self.client.reload(VisibilityLoadRequest::UberShader {
			transparent,
			rebake: true,
		});
	}

	/// Requests one baked lightmap and its image.
	pub(crate) fn request_lightmap(&mut self, id: String) {
		self.client.request(VisibilityLoadRequest::Lightmap(id));
	}

	/// Requests one environment and reports whether it was already resident.
	pub(crate) fn request_environment(&mut self, id: String) -> Option<ResidentEnvironment> {
		let resident = self.environments.get(&id).copied();
		self.client.request(VisibilityLoadRequest::Environment(id));
		resident
	}

	/// Interns a loaded image for slot `index` of `array` and returns its handle, or frees it and returns `None` when the
	/// slot already holds an image with more mips.
	///
	/// `first_level` is the resource mip that is the image's base level. An image that replaces the slot's image
	/// queues the old one for [`Self::delete_replaced_images`]. A coarser image than the resident one, such as coarse
	/// mips that arrive after their full chain, is dropped before interning, since no frame ever bound it.
	fn adopt_texture_image(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		array: TextureArray,
		index: u32,
		first_level: u32,
		image: ghi::implementation::DetachedImage,
	) -> Option<ghi::ImageHandle> {
		if self
			.texture_images
			.get(&(array, index))
			.is_some_and(|&(_, resident_first_level)| resident_first_level < first_level)
		{
			return None;
		}
		let image = frame.intern_image(image);
		if let Some((replaced, _)) = self.texture_images.insert((array, index), (image, first_level)) {
			self.replaced_images.push(replaced);
		}
		Some(image)
	}

	/// Deletes the images that finer ones replaced in their slots this frame.
	///
	/// Call it after writing every replacement into its slot, since a context refuses to delete an image a descriptor
	/// set still binds. The context frees each image once the frames in flight that may sample it complete.
	pub(crate) fn delete_replaced_images(&mut self, frame: &mut ghi::implementation::Frame) {
		for image in self.replaced_images.drain(..) {
			frame.delete_image(image);
		}
	}

	/// Appends readiness changes once per frame, after pipeline publication.
	///
	/// Interns every image that finished loading into `frame`'s context, and frees the geometry of released meshes no
	/// frame in flight reads. Drain and retain `events` after renderer adoption to reuse its allocation next frame.
	pub(crate) fn update(&mut self, frame: &mut ghi::implementation::Frame, events: &mut Vec<VisibilityLoaderEvent>) {
		// `frame` started after the fence of the frame that last used its resources, so older frames finished.
		let frame_index = frame.key().frame_index();
		self.retired_ranges.give_back_completed(frame_index, &self.geometry_ranges);
		self.next_frame = frame_index + 1;
		#[cfg(debug_assertions)]
		self.reload_rebaked_materials();

		while let Some(event) = self.client.poll() {
			events.push(match event {
				LoaderEvent::Ready {
					resident: VisibilityResident::Routed,
					..
				} => continue,
				LoaderEvent::Ready {
					resident: VisibilityResident::Mesh(key, mesh),
					..
				} => {
					let mesh = Arc::new(mesh);
					self.meshes.insert(key, mesh.clone());
					VisibilityLoaderEvent::MeshReady { key, mesh }
				}
				LoaderEvent::Ready {
					resident: VisibilityResident::MeshLevels { key, primitives },
					..
				} => {
					for levels in &primitives {
						// The new block holds every level the old one did and more, so the old one retires.
						if let Some(block) = &levels.block
							&& let Some(previous) = self.refined.insert((key, levels.primitive), block.clone())
						{
							self.retired_ranges.retire(previous.ranges, self.next_frame);
						}
					}
					VisibilityLoaderEvent::MeshLevelsLoaded { key, primitives }
				}
				LoaderEvent::Ready {
					resident: VisibilityResident::Material(material),
					..
				} => {
					// A material an uber shader evaluates has no pipeline of its own to wait for. It stays listed, so a
					// rebake of its variant loads it again.
					let ready = match material.evaluation {
						MaterialEvaluator::Arm(key) => Some(VisibilityLoaderEvent::MaterialReady(
							material.clone(),
							MaterialEvaluator::Arm(key),
						)),
						MaterialEvaluator::Pipeline(_) => None,
					};
					self.materials.insert(
						material.index,
						MaterialPublication {
							material,
							published: None,
						},
					);
					match ready {
						Some(ready) => ready,
						None => continue,
					}
				}
				LoaderEvent::Ready {
					resident: VisibilityResident::UberShader {
						transparent,
						keys,
						pipeline,
					},
					..
				} => {
					self.uber_shaders[usize::from(transparent)] = Some(UberShaderPublication { keys, pipeline });
					continue;
				}
				LoaderEvent::Ready {
					resident: VisibilityResident::CoverageShaders { keys, pipelines },
					..
				} => {
					self.coverage_shaders = Some((keys, pipelines));
					continue;
				}
				LoaderEvent::Ready {
					resident:
						VisibilityResident::Texture {
							id,
							array,
							index,
							image,
							first_level,
							photometry,
						},
					..
				} => {
					let Some(image) = self.adopt_texture_image(frame, array, index, first_level, image) else {
						continue;
					};
					VisibilityLoaderEvent::TextureReady(ResidentTexture {
						id,
						array,
						index,
						image: image.into(),
						photometry,
					})
				}
				LoaderEvent::Ready {
					resident:
						VisibilityResident::Lightmap {
							id,
							holder,
							instances,
							index,
							image,
						},
					..
				} => VisibilityLoaderEvent::LightmapReady(ResidentLightmap {
					texture: ResidentTexture {
						id: id.clone(),
						array: TextureArray::Lightmap,
						index,
						image: self
							.adopt_texture_image(frame, TextureArray::Lightmap, index, 0, image)
							.expect("A full mip chain always replaces the image in its slot.")
							.into(),
						photometry: None,
					},
					id,
					holder,
					instances,
				}),
				LoaderEvent::Ready {
					resident:
						VisibilityResident::Environment {
							id,
							diffuse_image,
							specular_image,
							upward_illuminance,
						},
					..
				} => {
					let resident = ResidentEnvironment {
						diffuse_image: frame.intern_image(diffuse_image).into(),
						specular_image: frame.intern_image(specular_image).into(),
						sampler: self.environment_sampler,
						upward_illuminance,
					};
					self.environments.insert(id.clone(), resident);
					VisibilityLoaderEvent::EnvironmentReady {
						id,
						environment: resident,
					}
				}
				LoaderEvent::Failed {
					key: VisibilityLoadKey::UberShader(transparent),
					error,
				} => VisibilityLoaderEvent::UberShaderUnavailable { transparent, error },
				LoaderEvent::Failed {
					key: VisibilityLoadKey::CoverageShaders,
					error,
				} => VisibilityLoaderEvent::CoverageShadersUnavailable { error },
				LoaderEvent::Failed {
					key: VisibilityLoadKey::MeshLevels(key),
					error,
				} => VisibilityLoaderEvent::MeshLevelsUnavailable { key, error },
				LoaderEvent::Failed {
					key: VisibilityLoadKey::Mesh(key),
					error,
				} => VisibilityLoaderEvent::MeshUnavailable { key, error },
				LoaderEvent::Failed { key, error } => VisibilityLoaderEvent::Unavailable {
					resource: key.to_string(),
					error,
				},
			});
		}

		// Visit each material once, even when many compilation results arrive together. A reloaded material replaced
		// its publication, so it reports its new state here too. One read lock covers every material's poll.
		let pipelines = self.pipeline_manager.compute_pipelines();
		for (transparent, slot) in [false, true].into_iter().zip(&mut self.uber_shaders) {
			let Some(publication) = slot else {
				continue;
			};
			match pipelines.state(publication.pipeline) {
				PipelineState::Pending => {}
				PipelineState::Ready(pipeline) => {
					events.push(VisibilityLoaderEvent::UberShaderReady {
						transparent,
						keys: publication.keys.clone(),
						pipeline,
					});
					*slot = None;
				}
				PipelineState::Failed => {
					events.push(VisibilityLoaderEvent::UberShaderUnavailable {
						transparent,
						error: LoadError(
							"Its pipeline failed to compile. The most likely cause is a material whose surface code the platform compiler rejects. Check the shader compilation errors logged above, fix that material, and bake again.".to_string(),
						),
					});
					*slot = None;
				}
			}
		}
		if let Some((keys, requested)) = &self.coverage_shaders {
			// The scan stops at the first pipeline that isn't ready, so a failure reports once the ones before it resolve.
			let mut failed = false;
			let ready = requested.try_map(|pipeline| match pipelines.state(pipeline) {
				PipelineState::Ready(pipeline) => Some(pipeline),
				PipelineState::Pending => None,
				PipelineState::Failed => {
					failed = true;
					None
				}
			});
			if failed {
				events.push(VisibilityLoaderEvent::CoverageShadersUnavailable {
					error: LoadError(
						"A masked raster pipeline failed to compile. The most likely cause is a masked material whose opacity code the platform compiler rejects. Check the shader compilation errors logged above, fix that material, and bake again.".to_string(),
					),
				});
				self.coverage_shaders = None;
			} else if let Some(ready) = ready {
				events.push(VisibilityLoaderEvent::CoverageShadersReady {
					keys: keys.clone(),
					pipelines: ready,
				});
				self.coverage_shaders = None;
			}
		}
		for publication in self.materials.values_mut() {
			let MaterialEvaluator::Pipeline(pipeline) = publication.material.evaluation else {
				continue;
			};
			let state = pipelines.state(pipeline);
			if publication.published == Some(state) {
				continue;
			}
			let was_ready = matches!(publication.published, Some(PipelineState::Ready(_)));
			publication.published = Some(state);
			match state {
				PipelineState::Pending if !was_ready => {}
				PipelineState::Ready(pipeline) => events.push(VisibilityLoaderEvent::MaterialReady(
					publication.material.clone(),
					MaterialEvaluator::Pipeline(pipeline),
				)),
				PipelineState::Pending | PipelineState::Failed => events.push(VisibilityLoaderEvent::MaterialUnavailable {
					index: publication.material.index,
				}),
			}
		}
	}

	/// Loads every resident material whose variant was rebaked again.
	///
	/// The reloaded material replaces the resident one once its pipeline is ready, so its textures, alpha mode,
	/// coverage, and pipeline all follow the new variant.
	#[cfg(debug_assertions)]
	fn reload_rebaked_materials(&mut self) {
		while let Some(update) = self.resource_updates.read() {
			if update.class() != "Variant" {
				continue;
			}
			if self
				.materials
				.values()
				.any(|publication| publication.material.id == update.id())
			{
				self.client.reload(VisibilityLoadRequest::Material(update.id().to_string()));
			}
		}
	}
}

/// Creates the visibility pipeline's single loader client and lane pool.
///
/// `render` is the context that created `geometry` and renders the loaded resources. The loader imports the
/// geometry streams so lanes append to them. `pipeline_manager` compiles each material's evaluation pipeline, or the
/// uber shaders when `material_evaluation` selects them. Run every returned lane on the loading thread.
pub(crate) fn spawn(
	loader: &mut Loader,
	render: &mut ghi::implementation::Context,
	resource_manager: EntityHandle<ResourceManager>,
	geometry: &GeometryHandles,
	pipeline_manager: PipelineManagerClient,
	material_evaluation: MaterialEvaluationMode,
) -> (VisibilityLoaderClient, Vec<LoaderLane<VisibilityLoader>>) {
	use ghi::context::ContextCreate as _;

	#[cfg(debug_assertions)]
	let resource_updates = resource_manager.resource_updates();
	let visibility_loader = VisibilityLoader {
		resource_manager,
		pipeline_manager: pipeline_manager.clone(),
		geometry: Mutex::new(GeometryBuffers::import(geometry, render, loader)),
		flat_triangles: geometry.flat_triangle_indices.is_some(),
		reported_missing_hit_shader: std::sync::atomic::AtomicBool::new(false),
		material_slots: Mutex::new(HashMap::default()),
		texture_slots: std::array::from_fn(|_| Mutex::new(HashMap::default())),
		material_evaluation,
	};
	let (client, lanes) = spawn_lanes(
		loader,
		visibility_loader,
		VISIBILITY_LANE_COUNT,
		VISIBILITY_BACKGROUND_LANE_COUNT,
		VISIBILITY_RESULT_CAPACITY,
	);
	(
		VisibilityLoaderClient {
			client,
			pipeline_manager,
			meshes: HashMap::default(),
			mesh_ids: HashMap::default(),
			refined: HashMap::default(),
			geometry_ranges: geometry.ranges.clone(),
			retired_ranges: RetiredRanges::default(),
			next_frame: 0,
			texture_images: HashMap::default(),
			replaced_images: Vec::new(),
			environments: HashMap::default(),
			materials: HashMap::default(),
			uber_shaders: [None, None],
			coverage_shaders: None,
			#[cfg(debug_assertions)]
			resource_updates,
			environment_sampler: render.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32)),
		},
		lanes,
	)
}

impl VisibilityLoader {
	/// Reads the stored class of `id` and forwards it as the matching family request.
	async fn load_resource(&self, id: Arc<str>, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let class = self.resource_manager.class(&id).await.map_err(|error| {
			LoadError(format!(
				"Visibility resource request failed for {id}. The most likely cause is that the resource id is missing or the asset database is not loaded. Request error: {error}"
			))
		})?;
		lane.request(if class == "Mesh" {
			VisibilityLoadRequest::Mesh(MeshSource::Resource(id))
		} else {
			self.route(id.to_string(), &class).await?
		});
		Ok(VisibilityResident::Routed)
	}

	/// Runs `query` and forwards every match as the request for the query's class.
	async fn load_query(&self, query: Query, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		if query.class == "Mesh" {
			return Err(LoadError(
				"Visibility cannot load mesh query results. The most likely cause is that mesh sources only accept static resource ids."
					.to_string(),
			));
		}
		let class = query.class.clone();
		let ids = self.resource_manager.query_ids(query).await.map_err(|error| {
			LoadError(format!(
				"Visibility query for {class} resources failed. The most likely cause is that the asset database is not loaded. Query error: {error:?}"
			))
		})?;
		for id in ids.items {
			// One unroutable match must not discard the rest of the query.
			match self.route(id, &class).await {
				Ok(request) => lane.request(request),
				Err(error) => log::warn!("{error}"),
			}
		}
		Ok(VisibilityResident::Routed)
	}

	/// Maps an owned resource ID of a known non-mesh class to its family request.
	async fn route(&self, id: String, class: &str) -> Result<VisibilityLoadRequest, LoadError> {
		match class {
			"Variant" => Ok(VisibilityLoadRequest::Material(id)),
			"Image" => {
				let image: Reference<ResourceImage> = self
					.resource_manager
					.request(&id)
					.await
					.map_err(|error| LoadError(format!("Visibility could not load image {id}. {error}")))?;
				if image.resource().ibl.is_some() {
					Ok(VisibilityLoadRequest::Environment(id))
				} else {
					Ok(VisibilityLoadRequest::Texture(id))
				}
			}
			_ => Err(LoadError(format!(
				"Visibility cannot load {id} of class {class}. The most likely cause is that the resource is not a mesh, material variant, or image."
			))),
		}
	}

	/// Requests one mesh's materials, then resolves, converts, places, and uploads its geometry.
	async fn load_mesh(&self, source: MeshSource, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let key = source.key();
		// Every primitive gets its material's stable table slot, and materials load while this mesh is read, converted,
		// and uploaded.
		let mut slots = SmallVec::<[u32; 8]>::new();
		// The base mesh holds each primitive's coarsest level, so every instance can draw from the start.
		let prepared = prepare_mesh_source(
			&self.resource_manager,
			&source,
			lane.staging().clone(),
			self.flat_triangles,
			PreparedLevels::Coarsest,
			|id| {
				lane.request(VisibilityLoadRequest::Material(id.to_string()));
				slots.push(self.material_slot(id)?);
				Ok(())
			},
		)
		.await?;
		let mesh = place_mesh(&self.geometry, prepared, &slots, lane).await?;
		Ok(VisibilityResident::Mesh(key, mesh))
	}

	/// Returns the material table slot of material `id`, assigning the next free one the first time.
	fn material_slot(&self, id: &str) -> Result<u32, LoadError> {
		let mut table = self.material_slots.lock().unwrap_or_else(|error| error.into_inner());
		assign_slot(&mut table, id, MAX_MATERIALS, "material").ok_or_else(|| {
			LoadError(
				"Visibility mesh material slots could not be assigned. The most likely cause is that the material table is full."
					.to_string(),
			)
		})
	}

	/// Loads finer detail levels of the listed primitives of resource mesh `id`, each from its finest listed level down
	/// to, but not including, its coarsest, with every vertex they index, and places each primitive in ranges of its
	/// own.
	///
	/// Every listed primitive loads from one read of each level stream. The base mesh already holds the coarsest levels,
	/// which rays hit, so these blocks keep no flat triangles. A primitive without room places nothing and comes back
	/// without a block, which lets the renderer free other primitives' levels and ask again; the others still land.
	async fn load_mesh_levels(
		&self,
		id: Arc<str>,
		mut primitives: Vec<PrimitiveLevels>,
		lane: &LoaderLane<Self>,
	) -> Result<VisibilityResident, LoadError> {
		let key = MeshSource::Resource(id.clone()).key();
		let resource: Reference<Mesh> = self.resource_manager.request(&id).await.map_err(|_| {
			LoadError(format!(
				"Visibility mesh resource request failed for {id}. The most likely cause is that the mesh id is missing or the asset database is not loaded."
			))
		})?;
		// The list ascends, so its last primitive is the highest index.
		let primitive_count = resource.resource().primitives.len();
		if let Some(levels) = primitives
			.last()
			.filter(|levels| levels.primitive as usize >= primitive_count)
		{
			return Err(LoadError(format!(
				"Visibility detail levels were requested for primitive {} of {id}, which has {primitive_count} primitives. The most likely cause is a residency request for a mesh that was rebaked with fewer primitives.",
				levels.primitive
			)));
		}
		let mut prepared = PreparedMesh::resource(resource, lane.staging().clone(), false, PreparedLevels::Finer(&primitives))
			.await
			.ok_or_else(|| {
				LoadError(format!(
					"Visibility detail level conversion failed for {id}. The most likely cause is an unsupported or malformed vertex stream."
				))
			})?;
		let copies = self
			.geometry
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.append_primitives(&mut prepared, &mut primitives)
			.map_err(|meshlets| {
				LoadError(format!(
					"Visibility detail level placement failed for {id}. The most likely cause is that a primitive level has {meshlets} meshlets, more than the {MAX_INSTANCE_MESHLETS} one instance can address; split the primitive."
				))
			})?;
		if !copies.is_empty() {
			lane.upload(prepared.staging, [], copies).await?;
		}
		Ok(VisibilityResident::MeshLevels { key, primitives })
	}

	/// Assigns every shader-table slot a material needs before publishing it to the render thread.
	fn assign_material_slots(&self, id: &str, texture_ids: &[Option<String>]) -> Option<(u32, Vec<Option<u32>>)> {
		let index = {
			let mut slots = self.material_slots.lock().unwrap_or_else(|error| error.into_inner());
			assign_slot(&mut slots, id, MAX_MATERIALS, "material")?
		};
		let texture_slots = {
			let mut slots = self.texture_slots[TextureArray::Material as usize]
				.lock()
				.unwrap_or_else(|error| error.into_inner());
			texture_ids
				.iter()
				.map(|texture| match texture {
					Some(texture) => assign_slot(&mut slots, texture, MAX_BINDLESS_TEXTURES, "texture").map(Some),
					None => Some(None),
				})
				.collect::<Option<Vec<_>>>()?
		};
		Some((index, texture_slots))
	}

	/// Loads and validates one material and requests its textures, which load while the material finishes.
	async fn load_material(&self, id: String, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let mut reference: Reference<ResourceVariant> = self.resource_manager.request(&id).await.map_err(|_| {
			LoadError(format!(
				"Visibility material variant request failed for {id}. The most likely cause is that the resource id is missing or the asset database is not loaded."
			))
		})?;
		let variant = reference.resource_mut();
		let MaterialDescription {
			alpha_mode,
			texture_ids,
			hit_group,
			parameters,
			double_sided,
			coverage_arm,
		} = describe_material(&id, variant)?;
		let material = variant.material.resource();
		let reads_backdrop = material.program.reads_backdrop;
		// A material bakes its evaluation shader, then a closest-hit shader when the project traces rays.
		let shader_of = |stage| {
			material
				.shaders()
				.iter()
				.find(|shader| shader.resource().stage == stage)
				.map(|shader| shader.id().to_string())
		};
		// The pipelines mode compiles the material's own evaluation shader. The uber mode keys the material into the
		// uber shader of its phase, which the bake built from the same program and static values.
		let evaluation = match self.material_evaluation {
			MaterialEvaluationMode::Pipelines => {
				let Some(shader_id) = shader_of(resource_management::types::ShaderTypes::Compute) else {
					return Err(LoadError(format!(
						"Visibility material shader is missing for {id}. The most likely cause is that the material was baked with `render.material-evaluation` set to `uber`. Bake it again with `beld bake --force`."
					)));
				};
				MaterialEvaluator::Pipeline(self.pipeline_manager.request_specialized_compute_pipeline(
					SpecializedComputePipelineRequest::new(
						shader_id,
						specialization_entries(alpha_mode, &variant.variables),
						vec![ghi::pipelines::PushConstantRange::new(
							0,
							MATERIAL_EVALUATION_PUSH_CONSTANT_SIZE,
						)],
					),
				))
			}
			MaterialEvaluationMode::Uber => {
				// The bake keys the material by the stored variant's static values, which solving keeps.
				MaterialEvaluator::Arm(uber_arm_key(
					material.program.identity,
					variant.static_values().flatten().copied(),
					matches!(alpha_mode, AlphaMode::Blend),
				))
			}
		};
		if self.flat_triangles
			&& hit_group.is_none()
			&& !self
				.reported_missing_hit_shader
				.swap(true, std::sync::atomic::Ordering::Relaxed)
		{
			log::warn!(
				"Material {id} has no closest-hit shader, so the rays surfel GI traces see it as a black occluder. The most likely cause is that its resources were baked before surfel GI was built in. Bake them again with `render.surfel-gi.enabled` set to `true` in `config.json`. See {}.",
				crate::online_docs_url("use/resources/bake#leave-render-features-out-of-the-bake")
			);
		}
		for texture in texture_ids.iter().flatten() {
			lane.request(VisibilityLoadRequest::Texture(texture.clone()));
		}
		let (index, texture_slots) = self.assign_material_slots(&id, &texture_ids).ok_or_else(|| {
			LoadError(format!(
				"Visibility material slots could not be assigned for {id}. The most likely cause is that the material or texture table is full."
			))
		})?;
		Ok(VisibilityResident::Material(ResidentMaterial {
			id,
			index,
			evaluation,
			alpha_mode,
			double_sided,
			coverage_arm,
			texture_slots,
			parameters,
			hit_group,
			reads_backdrop,
		}))
	}

	/// Loads the uber material evaluation shader of one phase, after baking it again when `rebake` is set, and requests
	/// its pipeline.
	async fn load_uber_shader(&self, transparent: bool, rebake: bool) -> Result<VisibilityResident, LoadError> {
		let id = UBER_MATERIAL_SHADERS[usize::from(transparent)];
		#[cfg(debug_assertions)]
		if rebake {
			self.resource_manager
				.rebake(id)
				.await
				.map_err(|error| LoadError(format!("Visibility could not bake {id} again. {error}")))?;
		}
		#[cfg(not(debug_assertions))]
		let _ = rebake;
		let reference: Reference<AssembledShader> = self.resource_manager.request(id).await.map_err(|error| {
			LoadError(format!(
				"Visibility could not load {id}. The most likely cause is resources baked with `render.material-evaluation` set to `pipelines`. Bake them again with `beld bake --force`. Request error: {error}"
			))
		})?;
		let assembled = reference.resource();
		// The uber shaders push the same five u32s as a material's own pipeline; the evaluation index goes unread.
		let pipeline = self
			.pipeline_manager
			.request_specialized_compute_pipeline(SpecializedComputePipelineRequest::new(
				assembled.shader.clone(),
				Vec::new(),
				vec![ghi::pipelines::PushConstantRange::new(
					0,
					MATERIAL_EVALUATION_PUSH_CONSTANT_SIZE,
				)],
			));
		Ok(VisibilityResident::UberShader {
			transparent,
			keys: assembled.keys.as_slice().into(),
			pipeline,
		})
	}

	/// Loads both coverage shaders, after baking them again when `rebake` is set, and requests every masked raster
	/// pipeline: each pass's solid pipelines with a coverage shader as their fragment.
	async fn load_coverage_shaders(&self, rebake: bool) -> Result<VisibilityResident, LoadError> {
		#[cfg(debug_assertions)]
		if rebake {
			for id in COVERAGE_SHADERS {
				self.resource_manager
					.rebake(id)
					.await
					.map_err(|error| LoadError(format!("Visibility could not bake {id} again. {error}")))?;
			}
		}
		#[cfg(not(debug_assertions))]
		let _ = rebake;
		let mut shaders = Vec::with_capacity(COVERAGE_SHADERS.len());
		for id in COVERAGE_SHADERS {
			let reference: Reference<AssembledShader> = self.resource_manager.request(id).await.map_err(|error| {
				LoadError(format!(
					"Visibility could not load {id}. The most likely cause is resources baked before masked materials were tested in coverage shaders. Bake them again with `beld bake --force`. Request error: {error}"
				))
			})?;
			shaders.push(reference.resource().clone());
		}
		let [visibility, depth] = <[AssembledShader; 2]>::try_from(shaders).expect("Two coverage shaders were loaded.");
		// Both shaders are baked from the same materials; a material is tested only where both hold its arm.
		let keys = visibility
			.keys
			.iter()
			.copied()
			.filter(|key| depth.keys.binary_search(key).is_ok())
			.collect::<Arc<[u32]>>();
		let request = |pipelines: [&str; 2], fragment: &str| {
			pipelines.map(|pipeline| {
				self.pipeline_manager
					.request_pipeline_with_fragment(&format!("byte_engine/rendering/visibility/{pipeline}.pipeline"), fragment)
			})
		};
		Ok(VisibilityResident::CoverageShaders {
			keys,
			pipelines: CoveragePipelines {
				visibility: request(["visibility", "double-sided-visibility"], &visibility.shader),
				// Masked casters draw with their own mesh stage, which passes the alpha test its UV and instance.
				directional: request(
					["masked-directional-shadow", "double-sided-masked-directional-shadow"],
					&depth.shader,
				),
				local: request(["masked-cone-shadow", "double-sided-masked-cone-shadow"], &depth.shader),
			},
		})
	}

	/// Loads one image, places it in a slot of `array`, and completes its transfer before returning.
	///
	/// With [`TextureMips::CoarseFirst`], an image larger than [`COARSE_TEXTURE_EXTENT`] loads only its mips from
	/// [`coarse_first_level`] on, then requests its full chain as [`VisibilityLoadRequest::TextureDetail`], which
	/// replaces the coarse image in the same slot. Only material textures load coarse mips first.
	async fn load_texture(
		&self,
		id: String,
		array: TextureArray,
		mips: TextureMips,
		lane: &LoaderLane<Self>,
	) -> Result<VisibilityResident, LoadError> {
		let resource: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|error| {
			LoadError(format!(
				"Visibility texture resource request failed for {id}. The most likely cause is that the resource id is missing, its asset handler is not registered, or the asset database is not loaded. Request error: {error}"
			))
		})?;
		let texture = resource.resource();
		// Only an IES profile's calibration scales its light.
		let photometry = texture.photometry.clone().filter(|photometry| {
			array == TextureArray::IesProfile && photometric_profile_metadata_is_valid(texture, photometry)
		});
		// The slot guard drops at the end of this statement, before the transfer awaits.
		let index = assign_slot(
			&mut self.texture_slots[array as usize].lock().unwrap_or_else(|error| error.into_inner()),
			&id,
			array.capacity(),
			array.kind(),
		)
		.ok_or_else(|| {
			let kind = array.kind();
			LoadError(format!(
				"Visibility {kind} limit exceeded. The most likely cause is that the scene referenced more than the {} {kind}s the visibility pipeline can hold at once.",
				array.capacity()
			))
		})?;

		let first_level = match mips {
			TextureMips::CoarseFirst => coarse_first_level(texture.extent, texture.mip_count),
			TextureMips::Full => 0,
		};

		let image = load_texture(resource, &id, first_level, lane)
			.await
			.map_err(|error| LoadError(format!("Visibility texture transfer failed for {id}. {error}")))?;
		if first_level != 0 {
			// Requested only once the coarse mips are uploaded, so the full chain queues behind the coarse mips of the
			// textures already waiting, and normally reaches the render thread after its own coarse image.
			lane.request(VisibilityLoadRequest::TextureDetail(id.clone()));
		}

		Ok(VisibilityResident::Texture {
			id,
			array,
			index,
			image,
			first_level,
			photometry,
		})
	}

	/// Loads a baked lightmap, then its image into a slot of the scene set's `lightmaps`.
	async fn load_lightmap(&self, id: String, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let docs = crate::online_docs_url("reference/assets#lightmaps");
		let lightmap: Reference<ResourceLightmap> = self.resource_manager.request(&id).await.map_err(|error| {
			LoadError(format!(
				"Visibility could not load lightmap {id}. The most likely cause is that the lightmap wasn't baked, or was baked without a GPU that traces rays. Bake it with `beld bake`, and check its log. See {docs}. Request error: {error}"
			))
		})?;
		let ResourceLightmap {
			holder,
			image,
			instances,
		} = lightmap.into_resource();
		let image_id = image.id().to_string();
		// Lightmap texels are lighting, so a coarse lightmap would light the scene wrongly until its chain loads.
		let VisibilityResident::Texture { index, image, .. } = self
			.load_texture(image_id, TextureArray::Lightmap, TextureMips::Full, lane)
			.await?
		else {
			unreachable!("a texture load returns a texture");
		};
		Ok(VisibilityResident::Lightmap {
			id,
			holder,
			instances,
			index,
			image,
		})
	}

	/// Loads the diffuse and roughness-prefiltered IBL streams and transfers them as one batch.
	async fn load_environment(&self, id: String, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let EnvironmentMaps {
			diffuse_image,
			specular_image,
			upward_illuminance,
		} = load_environment_maps(&self.resource_manager, &id, lane).await?;
		Ok(VisibilityResident::Environment {
			id,
			diffuse_image,
			specular_image,
			upward_illuminance,
		})
	}
}

/// The `EnvironmentMaps` struct is an environment's image-based light once uploaded: its diffuse irradiance cube, its
/// roughness-prefiltered specular cube, and the illuminance it delivers to an upward-facing surface.
pub(crate) struct EnvironmentMaps {
	pub(crate) diffuse_image: ghi::implementation::DetachedImage,
	pub(crate) specular_image: ghi::implementation::DetachedImage,
	/// Lux, or zero when the map can't be calibrated.
	pub(crate) upward_illuminance: f32,
}

/// Loads the diffuse and roughness-prefiltered IBL streams of environment `id` and transfers them through `lane` as
/// one batch.
///
/// The visibility loader lights scenes with them, and the lightmap bake lights the paths that leave its scene.
pub(crate) async fn load_environment_maps<P: LoadPipeline>(
	resource_manager: &ResourceManager,
	id: &str,
	lane: &LoaderLane<P>,
) -> Result<EnvironmentMaps, LoadError> {
	let docs = crate::online_docs_url("reference/assets#environment-maps");
	let mut reference: Reference<ResourceImage> = resource_manager.request(id).await.map_err(|error| {
		LoadError(format!(
			"Visibility could not load environment {id}. {error}\nEnvironment maps must come from a `.environment.bead` asset. See {docs}."
		))
	})?;
	let ibl = reference.resource().ibl.clone().ok_or_else(|| {
		LoadError(format!(
			"Visibility environment maps are missing for {id}. The most likely cause is that the selected resource is a plain image instead of a standalone `.environment.bead` asset. See {docs}."
		))
	})?;
	let (diffuse, specular) = (&ibl.diffuse_irradiance, &ibl.prefiltered_specular);
	let linear = resource_management::types::Gamma::Linear;
	let available_specular_mips =
		resource_management::resources::mips::mip_level_count(specular.extent[0], specular.extent[1]).unwrap_or(0);
	if diffuse.mip_count != 1
		|| specular.mip_count as usize != IBL_SPECULAR_LEVEL_COUNT
		|| diffuse.gamma != linear
		|| specular.gamma != linear
		|| diffuse.array_layers != 6
		|| specular.array_layers != 6
		|| diffuse.extent[2] != 0
		|| specular.extent[2] != 0
		|| (available_specular_mips as usize) < IBL_SPECULAR_LEVEL_COUNT
	{
		return Err(LoadError(format!(
			"Visibility environment IBL metadata is unsupported for {id}. The most likely cause is that the baked image does not contain one linear six-layer diffuse map and {IBL_SPECULAR_LEVEL_COUNT} linear six-layer specular levels."
		)));
	}
	let diffuse_format = resource_format_to_ghi(diffuse.format);
	let specular_format = resource_format_to_ghi(specular.format);
	let diffuse_extent = Extent::from(diffuse.extent);
	let specular_extent = Extent::from(specular.extent);

	let layout_failure = || {
		LoadError(format!(
			"Visibility environment layout is unsupported for {id}. The most likely cause is a baked extent or format the upload path cannot describe."
		))
	};
	// Lay every level out back to back in one lease so the environment transfers as one batch.
	let mut byte_count = 0;
	let mut layout = |format, extent| {
		let upload = TextureUploadLayout::new(format, extent, 6, byte_count);
		byte_count += upload.as_ref().map_or(0, |upload| upload.padded_size);
		upload
	};
	let diffuse_upload = layout(diffuse_format, diffuse_extent).ok_or_else(layout_failure)?;
	let mut specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|_| diffuse_upload);
	for (level, upload) in specular_uploads.iter_mut().enumerate() {
		*upload = layout(specular_format, specular_extent.mip(level as u32)).ok_or_else(layout_failure)?;
	}
	let mut staging = lane.staging().allocate(byte_count, 256).await.ok_or_else(|| {
		LoadError(format!(
			"Visibility environment {id} exceeds the GPU upload arena. The most likely cause is that its complete padded IBL data is larger than the configured upload capacity."
		))
	})?;

	let specular_stream_names: [String; IBL_SPECULAR_LEVEL_COUNT] =
		std::array::from_fn(|level| resource_management::resources::image::ibl_prefiltered_specular_stream_name(level as u32));
	{
		let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
		let mut streams = SmallVec::<[_; 16]>::new();
		let names = std::iter::once(resource_management::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME)
			.chain(specular_stream_names.iter().map(String::as_str));
		for (name, upload) in names.zip(std::iter::once(&diffuse_upload).chain(&specular_uploads)) {
			let region = &mut allocator.take(upload.padded_size)[..upload.compact_size];
			streams.push(resource_management::stream::StreamMut::new(name, region));
		}
		load_image_streams(&mut reference, streams).await.map_err(|error| {
			LoadError(format!(
				"Visibility environment load failed for {id}. The most likely cause is missing, corrupt, or mismatched IBL stream data. Error: {error}"
			))
		})?;
	}
	// Measure before packing rows, while the diffuse cube is still tightly packed face after face.
	let upward_illuminance = if diffuse_format == ghi::Formats::RGBA16F {
		upward_illuminance(
			&staging.bytes_mut()[diffuse_upload.offset..diffuse_upload.offset + diffuse_upload.compact_size],
			diffuse_extent.width(),
			diffuse_extent.height(),
		)
	} else {
		log::warn!(
			"Visibility environment {id} can't be calibrated to an illuminance. The most likely cause is a diffuse irradiance map that isn't RGBA16F, so its own values are used."
		);
		0.0
	};
	for upload in std::iter::once(&diffuse_upload).chain(&specular_uploads) {
		upload.pack_rows(&mut staging.bytes_mut()[upload.offset..upload.offset + upload.padded_size]);
	}

	let cube = |name: String, format, extent, mip_levels, regions| ImageUpload {
		description: ImageDescription {
			name,
			format,
			extent,
			mip_levels,
			cube: true,
		},
		regions,
	};
	let [diffuse_image, specular_image] = lane
		.upload(
			staging,
			[
				cube(
					format!("{id} diffuse irradiance"),
					diffuse_format,
					diffuse_extent,
					1,
					smallvec::smallvec![diffuse_upload.region(0)],
				),
				cube(
					format!("{id} prefiltered specular"),
					specular_format,
					specular_extent,
					IBL_SPECULAR_LEVEL_COUNT as u32,
					specular_uploads
						.iter()
						.enumerate()
						.map(|(mip_level, upload)| upload.region(mip_level as u32))
						.collect(),
				),
			],
			SmallVec::new(),
		)
		.await?;

	Ok(EnvironmentMaps {
		diffuse_image,
		specular_image,
		upward_illuminance,
	})
}

/// Returns the specialization constants of a material variant's evaluation pipeline: its phase, then its scalar and
/// vector variables, at the ids the visibility shader generator declared them with.
///
/// Image parameters bind through texture slots and dynamic ones through the parameter buffer instead, so neither
/// splits pipelines.
fn specialization_entries(
	alpha_mode: AlphaMode,
	variables: &[resource_management::resources::material::VariantVariable],
) -> Vec<ghi::pipelines::SpecializationMapEntry> {
	use ghi::pipelines::SpecializationMapEntry;

	let transparent = SpecializationMapEntry::new(TRANSPARENT_SPECIALIZATION_ID, matches!(alpha_mode, AlphaMode::Blend));
	std::iter::once(transparent)
		.chain(variable_specialization_entries(variables))
		.collect()
}

/// Reads or generates the geometry of `source` and converts it into the visibility geometry formats, holding the
/// detail `levels` and the flat triangle lists rays hit when `flat_triangles` is set.
///
/// Calls `material` with the material ID of each primitive, in primitive order, before the geometry loads, so the
/// caller can assign table slots and start loading the materials. The visibility loader and the lightmap bake both
/// prepare meshes here. Next, place and upload the result with [`place_mesh`].
pub(crate) async fn prepare_mesh_source(
	resource_manager: &ResourceManager,
	source: &MeshSource,
	staging: Arc<UploadStagingArena>,
	flat_triangles: bool,
	levels: PreparedLevels<'_>,
	mut material: impl FnMut(&str) -> Result<(), LoadError>,
) -> Result<PreparedMesh, LoadError> {
	let prepared = match source {
		MeshSource::Resource(id) => {
			let resource: Reference<Mesh> = resource_manager.request(id).await.map_err(|_| {
				LoadError(format!(
					"Visibility mesh resource request failed for {id}. The most likely cause is that the mesh id is missing or the asset database is not loaded."
				))
			})?;
			let mesh = resource.resource();
			for primitive in &mesh.primitives {
				material(mesh.material(primitive).id().as_ref())?;
			}
			PreparedMesh::resource(resource, staging, flat_triangles, levels).await
		}
		MeshSource::Generated(generator) => {
			material(generator.material().unwrap_or(GENERATED_MESH_MATERIAL))?;
			PreparedMesh::generated(generator.as_ref(), staging, flat_triangles).await
		}
	};
	prepared.ok_or_else(|| {
		LoadError(
			"Visibility mesh conversion failed. The most likely cause is an unsupported or malformed vertex stream."
				.to_string(),
		)
	})
}

/// Places `prepared` in the geometry buffers with the material table slot of each primitive in `slots`, uploads it
/// through `lane`, and returns where it landed.
pub(crate) async fn place_mesh<P: LoadPipeline>(
	geometry: &Mutex<GeometryBuffers>,
	mut prepared: PreparedMesh,
	slots: &[u32],
	lane: &LoaderLane<P>,
) -> Result<MeshData, LoadError> {
	let (mesh, copies) = geometry
		.lock()
		.unwrap_or_else(|error| error.into_inner())
		.append_mesh(&mut prepared, slots)
		.map_err(|full| {
			LoadError(match full {
				Some(element) => format!(
					"Visibility geometry placement failed. The most likely cause is that the {element} buffer has no free range for the mesh's coarsest detail levels; raise the capacity with VisibilityPipelineSettings::with_geometry_capacity or the render.geometry startup parameters."
				),
				None => "Visibility geometry placement failed. The most likely cause is a primitive no instance can address; split the primitive.".to_string(),
			})
		})?;
	lane.upload(prepared.staging, [], copies).await?;
	Ok(mesh)
}

/// The `MaterialDescription` struct is what a material variant tells every pass that shades its surfaces, apart from
/// how material evaluation runs it.
///
/// [`describe_material`] reads it. The visibility loader adds the evaluation pipeline; the lightmap bake uses it as it
/// is, to shade its rays' hits.
pub(crate) struct MaterialDescription {
	pub(crate) alpha_mode: AlphaMode,
	/// The image each variable names, by variable, which take texture slots in the material record.
	pub(crate) texture_ids: Vec<Option<String>>,
	/// The closest-hit shader rays run on the material's surfaces, when the material baked one.
	pub(crate) hit_group: Option<HitGroupShader>,
	pub(crate) parameters: Arc<[DynamicParameter]>,
	pub(crate) double_sided: bool,
	/// The material's key in the coverage shaders, when it is masked; see [`coverage_arm_key`].
	pub(crate) coverage_arm: Option<u32>,
}

/// Reads what a material variant tells the passes that shade it, or why the visibility pipeline can't draw it.
pub(crate) fn describe_material(id: &str, variant: &ResourceVariant) -> Result<MaterialDescription, LoadError> {
	let material = variant.material.resource();
	if material.model.name != "Visibility" || material.model.pass != "MaterialEvaluation" {
		return Err(LoadError(format!(
			"Unsupported visibility material model for {id}. The most likely cause is that this material targets a different render model or pass."
		)));
	}
	let texture_ids = variant
		.variables
		.iter()
		.map(|parameter| match &parameter.value {
			Value::Image(image) => Some(image.id().to_string()),
			_ => None,
		})
		.collect();
	let hit_group = material
		.shaders()
		.iter()
		.find(|shader| shader.resource().stage == resource_management::types::ShaderTypes::ClosestHit)
		.map(|shader| HitGroupShader {
			shader_id: shader.id().to_string(),
			specialization: variable_specialization_entries(&variant.variables).collect(),
		});
	Ok(MaterialDescription {
		alpha_mode: variant.alpha_mode,
		texture_ids,
		hit_group,
		parameters: dynamic_parameters(&variant.variables),
		double_sided: variant.double_sided,
		// The bake keys a masked material by the stored variant's static values, which solving keeps.
		coverage_arm: matches!(variant.alpha_mode, AlphaMode::Mask(_))
			.then(|| coverage_arm_key(material.program.identity, variant.static_values().flatten().copied())),
	})
}

/// Returns the specialization constants of a variant's static values, at the IDs of their variables, which material
/// evaluation and closest-hit shaders both declare.
fn variable_specialization_entries(
	variables: &[resource_management::resources::material::VariantVariable],
) -> impl Iterator<Item = ghi::pipelines::SpecializationMapEntry> + '_ {
	use ghi::pipelines::SpecializationMapEntry;

	(0u32..).zip(variables).filter_map(|(index, variable)| {
		let id = material_variable_specialization_id(index);
		if variable.dynamic {
			return None;
		}
		match &variable.value {
			Value::Scalar(value) => SpecializationMapEntry::new(id, *value).into(),
			Value::Vector3(value) => SpecializationMapEntry::new(id, ghi::pod::Vec3f::from(*value)).into(),
			Value::Vector4(value) => SpecializationMapEntry::new(id, ghi::pod::Vec4f::from(*value)).into(),
			Value::Image(_) => None,
		}
	})
}

/// Returns a variant's dynamic parameters with the values it gives them, in declaration order.
fn dynamic_parameters(variables: &[resource_management::resources::material::VariantVariable]) -> Arc<[DynamicParameter]> {
	variables
		.iter()
		.filter(|variable| variable.dynamic)
		.filter_map(|variable| {
			let default = match &variable.value {
				Value::Scalar(value) => (*value).into(),
				Value::Vector3(value) => (*value).into(),
				Value::Vector4(value) => (*value).into(),
				// The material handler never bakes a dynamic texture.
				Value::Image(_) => return None,
			};
			Some(DynamicParameter {
				name: variable.name.clone(),
				default,
			})
		})
		.collect()
}

/// Returns the illuminance that a baked diffuse irradiance cube delivers to an upward-facing surface.
///
/// The cube holds six tightly packed RGBA16F faces of irradiance divided by π, with +Y as the third face. The center
/// of that face is the irradiance for a normal pointing straight up, so the luminance there times π is the upward
/// illuminance. Even-sized faces have no center texel, so the four texels around the center are averaged.
fn upward_illuminance(diffuse_cube: &[u8], face_width: u32, face_height: u32) -> f32 {
	const BYTES_PER_TEXEL: usize = 8;
	const POSITIVE_Y_FACE: usize = 2;
	let (width, height) = (face_width as usize, face_height as usize);
	let face = &diffuse_cube[POSITIVE_Y_FACE * width * height * BYTES_PER_TEXEL..][..width * height * BYTES_PER_TEXEL];
	let channel = |texel: &[u8], index: usize| half::f16::from_le_bytes([texel[index * 2], texel[index * 2 + 1]]).to_f32();
	let center_rows = [(height - 1) / 2, height / 2];
	let center_columns = [(width - 1) / 2, width / 2];
	let mut luminance = 0.0;
	for y in center_rows {
		for x in center_columns {
			let texel = &face[(y * width + x) * BYTES_PER_TEXEL..][..BYTES_PER_TEXEL];
			luminance += utils::color::rec709_luminance(channel(texel, 0), channel(texel, 1), channel(texel, 2));
		}
	}
	std::f32::consts::PI * luminance / 4.0
}

impl LoadPipeline for VisibilityLoader {
	type Key = VisibilityLoadKey;
	type Request = VisibilityLoadRequest;
	type Resident = VisibilityResident;

	fn key(request: &Self::Request) -> Self::Key {
		match request {
			VisibilityLoadRequest::Resource(id) => VisibilityLoadKey::Resource(id.clone()),
			VisibilityLoadRequest::Query(query) => VisibilityLoadKey::Query(query.clone()),
			VisibilityLoadRequest::Mesh(source) => VisibilityLoadKey::Mesh(source.key()),
			VisibilityLoadRequest::MeshLevels { id, .. } => {
				VisibilityLoadKey::MeshLevels(MeshSource::Resource(id.clone()).key())
			}
			VisibilityLoadRequest::Material(id) => VisibilityLoadKey::Material(id.clone()),
			VisibilityLoadRequest::Texture(id) => VisibilityLoadKey::Texture(id.clone()),
			VisibilityLoadRequest::TextureDetail(id) => VisibilityLoadKey::TextureDetail(id.clone()),
			VisibilityLoadRequest::IesProfile(id) => VisibilityLoadKey::IesProfile(id.clone()),
			VisibilityLoadRequest::Environment(id) => VisibilityLoadKey::Environment(id.clone()),
			VisibilityLoadRequest::Lightmap(id) => VisibilityLoadKey::Lightmap(id.clone()),
			VisibilityLoadRequest::UberShader { transparent, .. } => VisibilityLoadKey::UberShader(*transparent),
			VisibilityLoadRequest::CoverageShaders { .. } => VisibilityLoadKey::CoverageShaders,
		}
	}

	/// Ranks the requests that only refine something that already draws as background.
	///
	/// A mesh draws with its coarsest levels and a texture with its coarse mips, so their finer levels and full mip
	/// chains wait. An instance draws once its renderable, materials, and textures are ready, and adopts its lightmap
	/// whenever it arrives, so lightmaps wait too.
	fn priority(request: &Self::Request) -> Priority {
		match request {
			VisibilityLoadRequest::MeshLevels { .. }
			| VisibilityLoadRequest::TextureDetail(_)
			| VisibilityLoadRequest::Lightmap(_) => Priority::Background,
			VisibilityLoadRequest::Resource(_)
			| VisibilityLoadRequest::Query(_)
			| VisibilityLoadRequest::Mesh(_)
			| VisibilityLoadRequest::Material(_)
			| VisibilityLoadRequest::Texture(_)
			| VisibilityLoadRequest::IesProfile(_)
			| VisibilityLoadRequest::Environment(_)
			| VisibilityLoadRequest::UberShader { .. }
			| VisibilityLoadRequest::CoverageShaders { .. } => Priority::Needed,
		}
	}

	/// Routes every visibility resource family through one request stream and one dependency registry.
	async fn load(&self, request: VisibilityLoadRequest, lane: &mut LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		match request {
			VisibilityLoadRequest::Resource(id) => self.load_resource(id, lane).await,
			VisibilityLoadRequest::Query(query) => self.load_query(query, lane).await,
			VisibilityLoadRequest::Mesh(source) => self.load_mesh(source, lane).await,
			VisibilityLoadRequest::MeshLevels { id, primitives } => self.load_mesh_levels(id, primitives, lane).await,
			VisibilityLoadRequest::Material(id) => self.load_material(id, lane).await,
			VisibilityLoadRequest::Texture(id) => {
				self.load_texture(id, TextureArray::Material, TextureMips::CoarseFirst, lane)
					.await
			}
			VisibilityLoadRequest::TextureDetail(id) => {
				self.load_texture(id, TextureArray::Material, TextureMips::Full, lane).await
			}
			VisibilityLoadRequest::IesProfile(id) => {
				self.load_texture(id, TextureArray::IesProfile, TextureMips::Full, lane).await
			}
			VisibilityLoadRequest::Environment(id) => self.load_environment(id, lane).await,
			VisibilityLoadRequest::Lightmap(id) => self.load_lightmap(id, lane).await,
			VisibilityLoadRequest::UberShader { transparent, rebake } => self.load_uber_shader(transparent, rebake).await,
			VisibilityLoadRequest::CoverageShaders { rebake } => self.load_coverage_shaders(rebake).await,
		}
	}
}

#[cfg(test)]
mod tests {
	use resource_management::types::{Formats, Gamma};

	use super::*;

	fn valid_profile_image() -> ResourceImage {
		ResourceImage {
			format: Formats::R16F,
			gamma: Gamma::Linear,
			extent: [721, 361, 0],
			mip_count: 1,
			ibl: None,
			photometry: None,
		}
	}

	#[test]
	fn slots_are_stable_and_bounded() {
		let mut slots = HashMap::default();
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "b", 2, "test"), Some(1));
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "c", 2, "test"), None);
	}

	#[test]
	fn coarse_mips_start_at_the_first_level_no_larger_than_the_coarse_extent() {
		// 4096 halves to 128 at level 5; the larger side decides for rectangles.
		assert_eq!(coarse_first_level([4096, 4096, 0], 13), 5);
		assert_eq!(coarse_first_level([300, 2048, 0], 12), 4);
		// Non-powers of two round down at each level, as their mips do: 1000 >> 3 = 125.
		assert_eq!(coarse_first_level([1000, 1000, 0], 10), 3);
		// Images already this small, and chains that stop before getting there, load whole.
		assert_eq!(coarse_first_level([128, 64, 0], 8), 0);
		assert_eq!(coarse_first_level([4096, 4096, 0], 1), 0);
		assert_eq!(coarse_first_level([4096, 4096, 0], 5), 0);
	}

	#[test]
	fn photometric_profile_metadata_requires_the_baked_ies_contract() {
		let photometry = ImagePhotometry {
			intensity_scale_candela: 180.0,
		};
		let valid = valid_profile_image();
		let mut srgb = valid_profile_image();
		srgb.gamma = Gamma::SRGB;
		let mut non_profile_format = valid_profile_image();
		non_profile_format.format = Formats::RGBA16F;
		let mut mipmapped = valid_profile_image();
		mipmapped.mip_count = 2;
		let mut volume = valid_profile_image();
		volume.extent[2] = 1;
		let invalid_scale = ImagePhotometry {
			intensity_scale_candela: 0.0,
		};

		assert!(photometric_profile_metadata_is_valid(&valid, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&srgb, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&non_profile_format, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&mipmapped, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&volume, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&valid, &invalid_scale));
	}

	/// Verifies that the measurement reads the +Y face the IBL baker writes third, at the texels around its center.
	#[test]
	fn upward_illuminance_reads_the_center_of_the_positive_y_face() {
		const FACE_SIZE: u32 = 8;
		let mut cube = Vec::new();
		for face in 0..6 {
			for y in 0..FACE_SIZE {
				for x in 0..FACE_SIZE {
					// Irradiance over π of 2 faces straight up; every other texel holds a distinct decoy.
					let center = (3..=4).contains(&x) && (3..=4).contains(&y);
					let value = if face == 2 && center { 2.0 } else { 10.0 + face as f32 };
					for channel in [value, value, value, 1.0] {
						cube.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
					}
				}
			}
		}

		let illuminance = upward_illuminance(&cube, FACE_SIZE, FACE_SIZE);
		assert!(
			(illuminance - 2.0 * std::f32::consts::PI).abs() < 1e-3,
			"upward illuminance = {illuminance}"
		);
	}
}
