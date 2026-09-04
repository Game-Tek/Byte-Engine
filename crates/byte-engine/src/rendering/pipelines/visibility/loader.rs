//! Pipeline-level loader-thread residency for every visibility resource.
//!
//! One request registry and one lane pool serve meshes, materials, textures, environments, and photometric
//! images. Resource-specific work stays in focused methods, but dependencies return through the same
//! pipeline protocol so one client coalesces every logical resource.
//!
//! The renderer interacts only with [`VisibilityLoaderClient`]. It submits domain requests and receives
//! typed ready or unavailable events; generic keys, requests, worker residents, lane types, and material
//! compilation state remain inside this module.

use std::sync::{Arc, Mutex};

use ghi::Device as _;
use ghi::command_buffer::CommandBufferRecording as _;
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

use super::geometry::{GeometryBuffers, GeometryHandles, MeshData, PreparedMesh};
use super::layout::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use super::slots::assign_slot;
use crate::core::EntityHandle;
use crate::rendering::loading::{
	Event as LoaderEvent, LoadError, LoadPipeline, Loaded, LoaderClient, LoaderLane, spawn as spawn_lanes,
};
use crate::rendering::pipeline_compilation::SpecializedComputePipelineRequest;
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::texture::{
	TextureUploadLayout, load_image_streams, resource_format_to_ghi, texture_mip_extent,
};
use crate::rendering::resource_loading::{TextureAddressMode, TextureDescriptor, TextureTransfer, UploadStagingArena};
use crate::rendering::{PipelineManagerClient, PipelineRef, PipelineState, SharedContext};

/// Number of prefiltered specular roughness levels stored by a baked environment.
pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;

const VISIBILITY_LANE_COUNT: usize = 4;
const VISIBILITY_RESULT_CAPACITY: usize = 64;

/// The `VisibilityLoadKey` enum names every logical resource in the visibility pipeline's shared registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum VisibilityLoadKey {
	Mesh(MeshKey),
	Material(String),
	Texture(String),
	Environment(String),
}

impl std::fmt::Display for VisibilityLoadKey {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Mesh(key) => write!(formatter, "mesh {key}"),
			Self::Material(id) => write!(formatter, "material {id}"),
			Self::Texture(id) => write!(formatter, "texture {id}"),
			Self::Environment(id) => write!(formatter, "environment {id}"),
		}
	}
}

/// The `VisibilityLoadRequest` enum carries owned work for every visibility resource family.
enum VisibilityLoadRequest {
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
	coverage: MaterialCoverage,
	texture_slots: Vec<Option<u32>>,
}

/// The `ResidentMaterial` struct carries one fully ready material into renderer-owned draw state.
pub(crate) struct ResidentMaterial {
	pub(crate) id: String,
	pub(crate) index: u32,
	pub(crate) pipeline: ghi::PipelineHandle,
	pub(crate) alpha_mode: AlphaMode,
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
}

/// The `VisibilityResident` enum keeps generic loader results private to the loader boundary.
enum VisibilityResident {
	Mesh(MeshData),
	Material(PreparedMaterial),
	Texture(ResidentTexture),
	Environment { id: String, resident: ResidentEnvironment },
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
	staging_buffer: ghi::BaseBufferHandle,
	geometry: Mutex<GeometryBuffers>,
	material_slots: Mutex<HashMap<String, u32>>,
	texture_slots: Mutex<HashMap<String, u32>>,
	texture_transfer: TextureTransfer,
}

/// The `VisibilityLoaderClient` struct hides the generic loading protocol from the visibility renderer.
pub(crate) struct VisibilityLoaderClient {
	client: LoaderClient<VisibilityLoader>,
	pipeline_manager: PipelineManagerClient,
	meshes: HashMap<MeshKey, MeshData>,
	environments: HashMap<String, ResidentEnvironment>,
	materials: HashMap<u32, MaterialPublication>,
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

	/// Returns the next renderer-facing readiness change.
	pub(crate) fn poll(&mut self) -> Option<VisibilityLoaderEvent> {
		loop {
			let Some(event) = self.client.poll() else {
				return self.poll_material();
			};
			return Some(match event {
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
					resident: VisibilityResident::Texture(texture),
				} => VisibilityLoaderEvent::TextureReady(texture),
				LoaderEvent::Ready {
					key: VisibilityLoadKey::Environment(_),
					resident: VisibilityResident::Environment { id, resident },
				} => {
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
	}

	/// Publishes the next material whose compiled pipeline state changed.
	fn poll_material(&mut self) -> Option<VisibilityLoaderEvent> {
		for publication in self.materials.values_mut() {
			let state = self.pipeline_manager.get(publication.material.pipeline);
			if publication.published == Some(state) {
				continue;
			}
			let was_ready = matches!(publication.published, Some(PipelineState::Ready(_)));
			publication.published = Some(state);
			match state {
				PipelineState::Pending if was_ready => {
					return Some(VisibilityLoaderEvent::MaterialUnavailable {
						index: publication.material.index,
					});
				}
				PipelineState::Pending => {}
				PipelineState::Ready(pipeline) => {
					let material = &publication.material;
					return Some(VisibilityLoaderEvent::MaterialReady(ResidentMaterial {
						id: material.id.clone(),
						index: material.index,
						pipeline,
						alpha_mode: material.alpha_mode.clone(),
						coverage: material.coverage,
						texture_slots: material.texture_slots.clone(),
					}));
				}
				PipelineState::Failed => {
					return Some(VisibilityLoaderEvent::MaterialUnavailable {
						index: publication.material.index,
					});
				}
			}
		}
		None
	}
}

/// Creates the visibility pipeline's single loader client and lane pool.
///
/// `staging_buffer` must back `staging`. Run every returned lane on an application-owned async task.
pub(crate) fn spawn(
	context: &SharedContext,
	queue: ghi::QueueHandle,
	resource_manager: EntityHandle<ResourceManager>,
	staging: Arc<UploadStagingArena>,
	staging_buffer: ghi::BaseBufferHandle,
	geometry: GeometryHandles,
	pipeline_config: MaterialPipelineConfig,
) -> (VisibilityLoaderClient, Vec<VisibilityLoaderLane>) {
	let pipeline_manager = pipeline_config.pipeline_manager.clone();
	let loader = VisibilityLoader {
		resource_manager,
		pipeline_config,
		staging_buffer,
		geometry: Mutex::new(GeometryBuffers::new(geometry)),
		material_slots: Mutex::new(HashMap::default()),
		texture_slots: Mutex::new(HashMap::default()),
		texture_transfer: TextureTransfer::new(context, staging_buffer, "Visibility Texture I/O"),
	};
	let (client, lanes) = spawn_lanes(
		context,
		queue,
		loader,
		staging,
		VISIBILITY_LANE_COUNT,
		VISIBILITY_RESULT_CAPACITY,
	);
	(
		VisibilityLoaderClient {
			client,
			pipeline_manager,
			meshes: HashMap::default(),
			environments: HashMap::default(),
			materials: HashMap::default(),
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

	/// Resolves, converts, places, and transfers one mesh before publishing its material dependencies.
	async fn load_mesh(&self, source: MeshSource, lane: &mut LoaderLane<Self>) -> Result<Loaded<Self>, LoadError> {
		let staging = lane.staging().clone();
		let prepared = match source {
			MeshSource::Resource(id) => {
				let resource: Reference<Mesh> = self.resource_manager.request(id).await.map_err(|_| {
					LoadError(format!(
						"Visibility mesh resource request failed for {id}. The most likely cause is that the mesh id is missing or the asset database is not loaded."
					))
				})?;
				PreparedMesh::resource(resource, staging).await
			}
			MeshSource::Generated(generator) => PreparedMesh::generated(generator.as_ref(), staging).await,
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

		// The copies must complete before `prepared` drops, because dropping it returns its staging memory.
		let mesh = lane
			.transfer(|recording| {
				self.geometry.lock().unwrap_or_else(|error| error.into_inner()).write_mesh(
					recording,
					self.staging_buffer,
					&prepared,
					&slots,
				)
			})
			.ok_or_else(|| {
				LoadError(
					"Visibility geometry placement failed. The most likely cause is that a geometry buffer is full."
						.to_string(),
				)
			})?;

		// Dependencies return through the same client so one pipeline registry coalesces all resource families.
		let dependencies = prepared
			.primitives
			.iter()
			.map(|primitive| VisibilityLoadRequest::Material(primitive.material_id.clone()))
			.collect();
		Ok(Loaded {
			resident: VisibilityResident::Mesh(mesh),
			dependencies,
		})
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

	/// Loads and validates one material, then publishes its textures through the pipeline dependency stream.
	async fn load_material(&self, id: String) -> Result<Loaded<Self>, LoadError> {
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
		let coverage = material.coverage;
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

		let dependencies = texture_ids
			.into_iter()
			.flatten()
			.map(VisibilityLoadRequest::Texture)
			.collect();
		Ok(Loaded {
			resident: VisibilityResident::Material(PreparedMaterial {
				id,
				index,
				pipeline,
				alpha_mode,
				coverage,
				texture_slots,
			}),
			dependencies,
		})
	}

	/// Returns or assigns the stable bindless slot for `id`, failing once the texture table is full.
	fn texture_slot(&self, id: &str) -> Option<u32> {
		let mut slots = self.texture_slots.lock().unwrap_or_else(|error| error.into_inner());
		assign_slot(&mut slots, id, MAX_BINDLESS_TEXTURES, "texture")
	}

	/// Loads one image, places it in a bindless slot, and completes its transfer before returning.
	async fn load_texture(&self, id: String, lane: &mut LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
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

		// Spherical IES profiles must clamp instead of wrapping around the seam.
		let texture = self
			.texture_transfer
			.load(
				resource,
				TextureDescriptor::new(&id).address_mode(if photometry.is_some() {
					TextureAddressMode::Clamp
				} else {
					TextureAddressMode::Repeat
				}),
				lane,
			)
			.await
			.map_err(|error| LoadError(format!("Visibility texture transfer failed for {id}. {error}")))?;

		Ok(VisibilityResident::Texture(ResidentTexture {
			id,
			index,
			image: texture.image(),
			sampler: texture.sampler(),
			photometry,
		}))
	}

	/// Loads the diffuse and roughness-prefiltered IBL streams and transfers them as one batch.
	async fn load_environment(&self, id: String, lane: &mut LoaderLane<Self>) -> Result<VisibilityResident, LoadError> {
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
		for upload in std::iter::once(&diffuse_upload).chain(&specular_uploads) {
			upload.pack_rows(&mut staging.bytes_mut()[upload.offset..upload.offset + upload.padded_size]);
		}

		let diffuse_name = format!("{id} diffuse irradiance");
		let specular_name = format!("{id} prefiltered specular");
		fn cube<'a>(format: ghi::Formats, name: &'a str, extent: Extent, mips: u32) -> ghi::image::Builder<'a> {
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(name)
				.extent(extent)
				.cube_compatible()
				.mip_levels(mips)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC)
		}
		let factory = lane.factory();
		let diffuse_image = factory.build_image(cube(diffuse_format, &diffuse_name, diffuse_extent, 1));
		let specular_image = factory.build_image(cube(
			specular_format,
			&specular_name,
			specular_extent,
			IBL_SPECULAR_LEVEL_COUNT as u32,
		));
		let sampler = factory.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));
		let (diffuse_image, specular_image, sampler) = lane.commit(|context| {
			(
				context.intern_image(diffuse_image).into(),
				context.intern_image(specular_image).into(),
				context.intern_sampler(sampler),
			)
		});

		let staging_offset = staging.offset();
		let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
		copies.push(diffuse_upload.copy_descriptor(self.staging_buffer, staging_offset, diffuse_image, 0));
		for (mip_level, mip) in specular_uploads.iter().enumerate() {
			copies.push(mip.copy_descriptor(self.staging_buffer, staging_offset, specular_image, mip_level as u32));
		}
		// The copies must complete before `staging` drops, because dropping it returns its lease to the arena.
		lane.transfer(|recording| recording.copy_buffer_to_images(&copies));
		drop(staging);

		Ok(VisibilityResident::Environment {
			id,
			resident: ResidentEnvironment {
				diffuse_image,
				specular_image,
				sampler,
			},
		})
	}
}

impl LoadPipeline for VisibilityLoader {
	type Key = VisibilityLoadKey;
	type Request = VisibilityLoadRequest;
	type Resident = VisibilityResident;

	fn key(request: &Self::Request) -> Self::Key {
		match request {
			VisibilityLoadRequest::Mesh(source) => VisibilityLoadKey::Mesh(source.key()),
			VisibilityLoadRequest::Material(id) => VisibilityLoadKey::Material(id.clone()),
			VisibilityLoadRequest::Texture(id) => VisibilityLoadKey::Texture(id.clone()),
			VisibilityLoadRequest::Environment(id) => VisibilityLoadKey::Environment(id.clone()),
		}
	}

	/// Routes every visibility resource family through one request stream and one dependency registry.
	async fn load(&self, request: VisibilityLoadRequest, lane: &mut LoaderLane<Self>) -> Result<Loaded<Self>, LoadError> {
		match request {
			VisibilityLoadRequest::Mesh(source) => self.load_mesh(source, lane).await,
			VisibilityLoadRequest::Material(id) => self.load_material(id).await,
			VisibilityLoadRequest::Texture(id) => Ok(Loaded::new(self.load_texture(id, lane).await?)),
			VisibilityLoadRequest::Environment(id) => Ok(Loaded::new(self.load_environment(id, lane).await?)),
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
}
