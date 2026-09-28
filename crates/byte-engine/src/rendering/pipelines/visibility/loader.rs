//! Pipeline-level loader-thread residency for every visibility resource.
//!
//! One request registry and one lane pool serve meshes, materials, textures, environments, and photometric
//! images. Resource-specific work stays in focused methods. Each load requests the resources its metadata names as
//! soon as it reads them, so meshes, materials, and textures load at the same time.
//!
//! The renderer interacts only with [`VisibilityLoaderClient`]. It submits domain requests and receives
//! typed ready or unavailable events; generic keys, requests, worker residents, lane types, and material
//! compilation state remain inside this module.

use std::sync::Mutex;

use resource_management::Reference;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::{
	image::{Image as ResourceImage, ImagePhotometry},
	material::{MaterialCoverage, Value, Variant as ResourceVariant},
	mesh::Mesh,
};
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::Extent;
use utils::hash::HashMap;

use super::geometry::{GENERATED_MESH_MATERIAL, GeometryBuffers, GeometryHandles, MeshData, PreparedMesh};
use super::layout::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use super::slots::assign_slot;
use crate::core::EntityHandle;
use crate::rendering::loading::{
	Event as LoaderEvent, ImageDescription, ImageUpload, LoadError, LoadPipeline, Loader, LoaderClient, LoaderLane,
	spawn as spawn_lanes,
};
use crate::rendering::pipeline_compilation::SpecializedComputePipelineRequest;
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::load_texture;
use crate::rendering::resource_loading::texture::{
	TextureUploadLayout, load_image_streams, resource_format_to_ghi, texture_mip_extent,
};
use crate::rendering::{PipelineManagerClient, PipelineRef, PipelineState};
use crate::rendering::{Query, Resource};

/// Number of prefiltered specular roughness levels stored by a baked environment.
pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;

const VISIBILITY_LANE_COUNT: usize = 4;
const VISIBILITY_RESULT_CAPACITY: usize = 64;

/// The `VisibilityLoadKey` enum names every logical resource in the visibility pipeline's shared registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum VisibilityLoadKey {
	Resource(&'static str),
	Query(Query),
	Mesh(MeshKey),
	Material(String),
	Texture(String),
	Environment(String),
}

impl std::fmt::Display for VisibilityLoadKey {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Resource(id) => write!(formatter, "resource {id}"),
			Self::Query(query) => write!(formatter, "query {query:?}"),
			Self::Mesh(key) => write!(formatter, "mesh {key}"),
			Self::Material(id) => write!(formatter, "material {id}"),
			Self::Texture(id) => write!(formatter, "texture {id}"),
			Self::Environment(id) => write!(formatter, "environment {id}"),
		}
	}
}

/// The `VisibilityLoadRequest` enum carries owned work for every visibility resource family.
enum VisibilityLoadRequest {
	/// A resource of unknown class, routed to its family once its class is read.
	Resource(&'static str),
	/// Every resource that matches a query, each routed by the query's class.
	Query(Query),
	Mesh(MeshSource),
	Material(String),
	Texture(String),
	Environment(String),
}

/// The `PreparedMaterial` struct keeps loader-ready material data private until its pipeline is available.
struct PreparedMaterial {
	id: String,
	index: u32,
	pipeline: PipelineRef,
	alpha_mode: AlphaMode,
	double_sided: bool,
	coverage: MaterialCoverage,
	texture_slots: Vec<Option<u32>>,
}

/// The `ResidentMaterial` struct carries one fully ready material into renderer-owned draw state.
pub(crate) struct ResidentMaterial {
	pub(crate) id: String,
	pub(crate) index: u32,
	pub(crate) pipeline: ghi::PipelineHandle,
	pub(crate) alpha_mode: AlphaMode,
	pub(crate) double_sided: bool,
	pub(crate) coverage: MaterialCoverage,
	pub(crate) texture_slots: Vec<Option<u32>>,
}

/// The `ResidentTexture` struct carries one upload-complete texture into renderer-owned descriptors.
pub(crate) struct ResidentTexture {
	pub(crate) id: String,
	pub(crate) index: u32,
	pub(crate) image: ghi::BaseImageHandle,
	pub(crate) sampler: ghi::SamplerHandle,
	pub(crate) photometry: Option<ImagePhotometry>,
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
enum VisibilityResident {
	/// A routed resource; its family request travels as the only dependency.
	Routed,
	Mesh(MeshData),
	Material(PreparedMaterial),
	/// An upload-complete texture whose image the render thread still interns.
	Texture {
		id: String,
		index: u32,
		image: ghi::implementation::DetachedImage,
		photometry: Option<ImagePhotometry>,
	},
	/// Upload-complete image-based lighting whose images the render thread still interns.
	Environment {
		id: String,
		diffuse_image: ghi::implementation::DetachedImage,
		specular_image: ghi::implementation::DetachedImage,
		upward_illuminance: f32,
	},
}

/// The `VisibilityLoaderEvent` enum is the renderer's complete view of visibility resource loading.
pub(crate) enum VisibilityLoaderEvent {
	MeshReady { key: MeshKey, mesh: MeshData },
	MaterialReady(ResidentMaterial),
	MaterialUnavailable { index: u32 },
	TextureReady(ResidentTexture),
	EnvironmentReady { id: String, environment: ResidentEnvironment },
	Unavailable { resource: String, error: LoadError },
}

/// The `MaterialPipelineConfig` struct gives visibility lanes the immutable inputs used to request compute pipelines.
#[derive(Clone)]
pub struct MaterialPipelineConfig {
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	pipeline_manager: PipelineManagerClient,
}

impl MaterialPipelineConfig {
	/// Creates the compute-pipeline inputs shared by every visibility lane.
	pub fn new(push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>, pipeline_manager: PipelineManagerClient) -> Self {
		Self {
			push_constant_ranges,
			pipeline_manager,
		}
	}
}

/// The `VisibilityLoader` struct owns all resource loading and GPU placement for the visibility pipeline.
struct VisibilityLoader {
	resource_manager: EntityHandle<ResourceManager>,
	pipeline_config: MaterialPipelineConfig,
	geometry: Mutex<GeometryBuffers>,
	material_slots: Mutex<HashMap<String, u32>>,
	texture_slots: Mutex<HashMap<String, u32>>,
}

/// The `VisibilityLoaderClient` struct hides the generic loading protocol from the visibility renderer.
pub(crate) struct VisibilityLoaderClient {
	client: LoaderClient<VisibilityLoader>,
	pipeline_manager: PipelineManagerClient,
	meshes: HashMap<MeshKey, MeshData>,
	environments: HashMap<String, ResidentEnvironment>,
	materials: HashMap<u32, MaterialPublication>,
	/// Samples material textures, which repeat outside their bounds.
	repeat_sampler: ghi::SamplerHandle,
	/// Samples spherical IES profiles, which must clamp instead of wrapping around the seam.
	clamp_sampler: ghi::SamplerHandle,
	/// Samples environment maps across their prefiltered roughness levels, and the one-level fallback environment.
	pub(super) environment_sampler: ghi::SamplerHandle,
}

/// The `MaterialPublication` struct tracks the last compilation state reported to the renderer.
struct MaterialPublication {
	material: PreparedMaterial,
	published: Option<PipelineState>,
}

/// The `VisibilityLoaderLane` struct hides one generic worker lane from application setup.
pub(crate) struct VisibilityLoaderLane(LoaderLane<VisibilityLoader>);

impl VisibilityLoaderLane {
	/// Runs this lane until the visibility loader client is dropped.
	pub(crate) async fn run(self) {
		self.0.run().await;
	}
}

/// Builds the default sampler used by visibility material textures.
fn material_sampler() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.filtering_mode(ghi::FilteringModes::Linear)
		.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
		.mip_map_mode(ghi::FilteringModes::Linear)
		.addressing_mode(ghi::SamplerAddressingModes::Repeat)
		.min_lod(0f32)
		.max_lod(0f32)
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
	pub(crate) fn request_mesh(&mut self, source: MeshSource) -> (MeshKey, Option<MeshData>) {
		let key = source.key();
		let resident = self.meshes.get(&key).cloned();
		self.client.request(VisibilityLoadRequest::Mesh(source));
		(key, resident)
	}

	/// Requests one material texture or photometric profile.
	pub(crate) fn request_texture(&mut self, id: String) {
		self.client.request(VisibilityLoadRequest::Texture(id));
	}

	/// Requests one environment and reports whether it was already resident.
	pub(crate) fn request_environment(&mut self, id: String) -> Option<ResidentEnvironment> {
		let resident = self.environments.get(&id).copied();
		self.client.request(VisibilityLoadRequest::Environment(id));
		resident
	}

	/// Appends readiness changes once per frame, after pipeline publication.
	///
	/// Interns every image that finished loading into `frame`'s context. Drain and retain `events` after renderer
	/// adoption to reuse its allocation next frame.
	pub(crate) fn update(&mut self, frame: &mut ghi::implementation::Frame, events: &mut Vec<VisibilityLoaderEvent>) {
		while let Some(event) = self.client.poll() {
			events.push(match event {
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Resource(_) | VisibilityLoadKey::Query(_),
					resident: VisibilityResident::Routed,
				} => continue,
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Mesh(key),
					resident: VisibilityResident::Mesh(mesh),
				} => {
					self.meshes.insert(key, mesh.clone());
					VisibilityLoaderEvent::MeshReady { key, mesh }
				}
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Material(_),
					resident: VisibilityResident::Material(material),
				} => {
					self.materials.insert(
						material.index,
						MaterialPublication {
							material,
							published: None,
						},
					);
					continue;
				}
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Texture(_),
					resident:
						VisibilityResident::Texture {
							id,
							index,
							image,
							photometry,
						},
				} => VisibilityLoaderEvent::TextureReady(ResidentTexture {
						id,
						index,
						image: frame.intern_image(image).into(),
						sampler: if photometry.is_some() {
							self.clamp_sampler
						} else {
							self.repeat_sampler
						},
						photometry,
					}),
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Environment(_),
					resident:
						VisibilityResident::Environment {
							id,
							diffuse_image,
							specular_image,
							upward_illuminance,
						},
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
				LoaderEvent::Failed { key, error } => VisibilityLoaderEvent::Unavailable {
					resource: key.to_string(),
					error,
				},
				LoaderEvent::Ready { .. } => unreachable!(
					"Visibility loader returned a mismatched key and resident. The most likely cause is an incorrect route inside VisibilityLoader."
				),
			});
		}

		// Visit each material once, even when many compilation results arrive together.
		for publication in self.materials.values_mut() {
			let state = self.pipeline_manager.get(publication.material.pipeline);
			if publication.published == Some(state) {
				continue;
			}
			let was_ready = matches!(publication.published, Some(PipelineState::Ready(_)));
			publication.published = Some(state);
			match state {
				PipelineState::Pending if was_ready => {
					events.push(VisibilityLoaderEvent::MaterialUnavailable {
						index: publication.material.index,
					});
				}
				PipelineState::Pending => {}
				PipelineState::Ready(pipeline) => {
					let material = &publication.material;
					events.push(VisibilityLoaderEvent::MaterialReady(ResidentMaterial {
						id: material.id.clone(),
						index: material.index,
						pipeline,
						alpha_mode: material.alpha_mode.clone(),
						double_sided: material.double_sided,
						coverage: material.coverage,
						texture_slots: material.texture_slots.clone(),
					}));
				}
				PipelineState::Failed => {
					events.push(VisibilityLoaderEvent::MaterialUnavailable {
						index: publication.material.index,
					});
				}
			}
		}
	}
}

/// Creates the visibility pipeline's single loader client and lane pool.
///
/// `render` is the context that created `geometry` and renders the loaded resources. The loader imports the
/// geometry streams so lanes append to them. Run every returned lane on the loading thread.
pub(crate) fn spawn(
	loader: &mut Loader,
	render: &mut ghi::implementation::Context,
	resource_manager: EntityHandle<ResourceManager>,
	geometry: &GeometryHandles,
	pipeline_config: MaterialPipelineConfig,
) -> (VisibilityLoaderClient, Vec<VisibilityLoaderLane>) {
	use ghi::context::ContextCreate as _;

	let pipeline_manager = pipeline_config.pipeline_manager.clone();
	let visibility_loader = VisibilityLoader {
		resource_manager,
		pipeline_config,
		geometry: Mutex::new(GeometryBuffers::import(geometry, render, loader)),
		material_slots: Mutex::new(HashMap::default()),
		texture_slots: Mutex::new(HashMap::default()),
	};
	let (client, lanes) = spawn_lanes(loader, visibility_loader, VISIBILITY_LANE_COUNT, VISIBILITY_RESULT_CAPACITY);
	(
		VisibilityLoaderClient {
			client,
			pipeline_manager,
			meshes: HashMap::default(),
			environments: HashMap::default(),
			materials: HashMap::default(),
			repeat_sampler: render.build_sampler(material_sampler().max_lod(ghi::sampler::UNCLAMPED_MAX_LOD)),
			clamp_sampler: render.build_sampler(
				material_sampler()
					.addressing_mode(ghi::SamplerAddressingModes::Clamp)
					.max_lod(ghi::sampler::UNCLAMPED_MAX_LOD),
			),
			environment_sampler: render.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32)),
		},
		lanes.into_iter().map(VisibilityLoaderLane).collect(),
	)
}

impl VisibilityLoader {
	/// Returns or assigns the stable material-table slot for each primitive of a prepared mesh.
	fn mesh_material_slots(&self, mesh: &PreparedMesh) -> Option<SmallVec<[u32; 8]>> {
		let mut slots = self.material_slots.lock().unwrap_or_else(|error| error.into_inner());
		mesh.primitives
			.iter()
			.map(|primitive| assign_slot(&mut slots, &primitive.material_id, MAX_MATERIALS, "material"))
			.collect()
	}

	/// Reads the stored class of `id` and forwards it as the matching family request.
	async fn load_resource(&self, id: &'static str, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let class = self.resource_manager.class(id).await.map_err(|error| {
			LoadError(format!(
				"Visibility resource request failed for {id}. The most likely cause is that the resource id is missing or the asset database is not loaded. Request error: {error}"
			))
		})?;
		lane.request(if class == "Mesh" {
			VisibilityLoadRequest::Mesh(MeshSource::Resource(id))
		} else {
			self.route(id.to_owned(), &class).await?
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
				let image: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|error| {
					LoadError(format!("Visibility image request failed for {id}. Request error: {error}"))
				})?;
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
		let staging = lane.staging().clone();
		// Materials load while this mesh is read, converted, and uploaded.
		let prepared = match source {
			MeshSource::Resource(id) => {
				let resource: Reference<Mesh> = self.resource_manager.request(id).await.map_err(|_| {
					LoadError(format!(
						"Visibility mesh resource request failed for {id}. The most likely cause is that the mesh id is missing or the asset database is not loaded."
					))
				})?;
				let mesh = resource.resource();
				for primitive in &mesh.primitives {
					lane.request(VisibilityLoadRequest::Material(
						mesh.material(primitive).id().as_ref().to_string(),
					));
				}
				PreparedMesh::resource(resource, staging).await
			}
			MeshSource::Generated(generator) => {
				lane.request(VisibilityLoadRequest::Material(GENERATED_MESH_MATERIAL.to_string()));
				PreparedMesh::generated(generator.as_ref(), staging).await
			}
		}
		.ok_or_else(|| {
			LoadError(
				"Visibility mesh conversion failed. The most likely cause is an unsupported or malformed vertex stream."
					.to_string(),
			)
		})?;

		let slots = self.mesh_material_slots(&prepared).ok_or_else(|| {
			LoadError(
				"Visibility mesh material slots could not be assigned. The most likely cause is that the material table is full."
					.to_string(),
			)
		})?;

		let (mesh, copies) = self
			.geometry
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.append_mesh(&prepared, &slots)
			.ok_or_else(|| {
				LoadError(
					"Visibility geometry placement failed. The most likely cause is that a geometry buffer is full."
						.to_string(),
				)
			})?;
		lane.upload(prepared.staging, [], copies).await?;
		Ok(VisibilityResident::Mesh(mesh))
	}

	/// Assigns every shader-table slot a material needs before publishing it to the render thread.
	fn assign_material_slots(&self, id: &str, texture_ids: &[Option<String>]) -> Option<(u32, Vec<Option<u32>>)> {
		let index = {
			let mut slots = self.material_slots.lock().unwrap_or_else(|error| error.into_inner());
			assign_slot(&mut slots, id, MAX_MATERIALS, "material")?
		};
		let texture_slots = {
			let mut slots = self.texture_slots.lock().unwrap_or_else(|error| error.into_inner());
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
		let alpha_mode = variant.alpha_mode.clone();
		let texture_ids: Vec<Option<String>> = variant
			.variables
			.iter()
			.map(|parameter| match &parameter.value {
				Value::Image(image) => Some(image.id().to_string()),
				_ => None,
			})
			.collect();
		let material = variant.material.resource_mut();
		if material.model.name != "Visibility" || material.model.pass != "MaterialEvaluation" {
			return Err(LoadError(format!(
				"Unsupported visibility material model for {id}. The most likely cause is that this material targets a different render model or pass."
			)));
		}
		if material.shaders().is_empty() {
			return Err(LoadError(format!(
				"Visibility material shader is missing for {id}. The most likely cause is that the material was baked without a compute shader."
			)));
		}
		for texture in texture_ids.iter().flatten() {
			lane.request(VisibilityLoadRequest::Texture(texture.clone()));
		}
		let coverage = material.coverage;
		let double_sided = material.double_sided();
		let (index, texture_slots) = self.assign_material_slots(&id, &texture_ids).ok_or_else(|| {
			LoadError(format!(
				"Visibility material slots could not be assigned for {id}. The most likely cause is that the material or texture table is full."
			))
		})?;
		let pipeline =
			self.pipeline_config
				.pipeline_manager
				.request_specialized_compute_pipeline(SpecializedComputePipelineRequest::new(
					id.clone(),
					self.pipeline_config.push_constant_ranges.clone(),
				));
		Ok(VisibilityResident::Material(PreparedMaterial {
			id,
			index,
			pipeline,
			alpha_mode,
			double_sided,
			coverage,
			texture_slots,
		}))
	}

	/// Returns or assigns the stable bindless slot for `id`, failing once the texture table is full.
	fn texture_slot(&self, id: &str) -> Option<u32> {
		let mut slots = self.texture_slots.lock().unwrap_or_else(|error| error.into_inner());
		assign_slot(&mut slots, id, MAX_BINDLESS_TEXTURES, "texture")
	}

	/// Loads one image, places it in a bindless slot, and completes its transfer before returning.
	async fn load_texture(&self, id: String, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let resource: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|error| {
			LoadError(format!(
				"Visibility texture resource request failed for {id}. The most likely cause is that the resource id is missing, its asset handler is not registered, or the asset database is not loaded. Request error: {error}"
			))
		})?;
		let texture = resource.resource();
		let photometry = texture
			.photometry
			.clone()
			.filter(|photometry| photometric_profile_metadata_is_valid(texture, photometry));
		let index = self.texture_slot(&id).ok_or_else(|| {
			LoadError(
				"Visibility texture limit exceeded. The most likely cause is that the scene referenced more textures than the visibility pipeline supports."
					.to_string(),
			)
		})?;

		let image = load_texture(resource, &id, lane)
			.await
			.map_err(|error| LoadError(format!("Visibility texture transfer failed for {id}. {error}")))?;

		Ok(VisibilityResident::Texture {
			id,
			index,
			image,
			photometry,
		})
	}

	/// Loads the diffuse and roughness-prefiltered IBL streams and transfers them as one batch.
	async fn load_environment(&self, id: String, lane: &LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		let docs = crate::online_docs_url("develop/resource-management/assets#environment-maps");
		let mut reference: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|_| {
			LoadError(format!(
				"Visibility environment request failed for {id}. The most likely cause is that the `.environment.bead` resource is missing or the asset database is not loaded. See {docs}."
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
			*upload = layout(specular_format, texture_mip_extent(specular_extent, level as u32)).ok_or_else(layout_failure)?;
		}
		let mut staging = lane.staging().allocate(byte_count, 256).await.ok_or_else(|| {
			LoadError(format!(
				"Visibility environment {id} exceeds the GPU upload arena. The most likely cause is that its complete padded IBL data is larger than the configured upload capacity."
			))
		})?;

		let specular_stream_names: [String; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|level| {
			resource_management::resources::image::ibl_prefiltered_specular_stream_name(level as u32)
		});
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

		Ok(VisibilityResident::Environment {
			id,
			diffuse_image,
			specular_image,
			upward_illuminance,
		})
	}
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
			luminance += 0.2126 * channel(texel, 0) + 0.7152 * channel(texel, 1) + 0.0722 * channel(texel, 2);
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
			VisibilityLoadRequest::Resource(id) => VisibilityLoadKey::Resource(id),
			VisibilityLoadRequest::Query(query) => VisibilityLoadKey::Query(query.clone()),
			VisibilityLoadRequest::Mesh(source) => VisibilityLoadKey::Mesh(source.key()),
			VisibilityLoadRequest::Material(id) => VisibilityLoadKey::Material(id.clone()),
			VisibilityLoadRequest::Texture(id) => VisibilityLoadKey::Texture(id.clone()),
			VisibilityLoadRequest::Environment(id) => VisibilityLoadKey::Environment(id.clone()),
		}
	}

	/// Routes every visibility resource family through one request stream and one dependency registry.
	async fn load(&self, request: VisibilityLoadRequest, lane: &mut LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
		match request {
			VisibilityLoadRequest::Resource(id) => self.load_resource(id, lane).await,
			VisibilityLoadRequest::Query(query) => self.load_query(query, lane).await,
			VisibilityLoadRequest::Mesh(source) => self.load_mesh(source, lane).await,
			VisibilityLoadRequest::Material(id) => self.load_material(id, lane).await,
			VisibilityLoadRequest::Texture(id) => self.load_texture(id, lane).await,
			VisibilityLoadRequest::Environment(id) => self.load_environment(id, lane).await,
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
