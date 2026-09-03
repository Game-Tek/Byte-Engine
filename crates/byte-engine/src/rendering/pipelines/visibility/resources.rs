//! Asynchronous resource loading for the visibility pipeline.
//!
//! Scene code calls [`ResourceClient::request_mesh`], [`ResourceClient::request_image`], or
//! [`ResourceClient::request_environment`]. Requests coalesce through the shared loader and are prepared on
//! independent [`VisibilityResourcePreparer`] lanes. Each frame, [`ResourceClient::begin_frame`] drains prepared
//! values: meshes and staged textures go through the frame upload queue into [`ResourceStore`], which owns
//! geometry placement and stable material and bindless texture slots. Everything else, and every finished
//! upload, is handed to the pipeline manager as a [`VisibilityResourceCompletion`].
//!
//! A mesh discovers its materials and a material discovers its textures. [`ResourceDependencies`] keeps that
//! graph so a repeated request for a resident mesh retries only its failed descendants.

mod preparer;
mod protocol;

use std::sync::Arc;

use ghi::command_buffer::CommandBufferRecording as _;
use ghi::context::ContextCreate as _;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::ImagePhotometry;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};

pub use self::preparer::{MaterialPipelineConfig, VisibilityResourcePreparer};
pub(crate) use self::protocol::{
	IBL_SPECULAR_LEVEL_COUNT, ImageSource, PendingEnvironmentUpload, PreparedImage, VisibilityResourceCompletion,
	VisibilityResourceKey,
};
use self::protocol::{PreparedUpload, VisibilityPreparedResource, VisibilityRenderResource, VisibilityResourceRequest};
use super::geometry::{GeometryBuffers, MeshData, PreparedMesh};
use super::layout::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::core::EntityHandle;
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::{
	FrameUploadQueue, ResourceLoader, ResourceLoadingServer, ResourceRef, ResourceState, ResourceToken, ResourceUploadStore,
	StagedTextureUpload, UploadStagingArena,
};

/// Size of the shared upload arena the application allocates for visibility transfers.
pub const ASYNC_UPLOAD_BUFFER_BYTE_COUNT: usize = 1024 * 1024 * 32;
const RESOURCE_LIMIT: usize = MAX_MATERIALS + MAX_BINDLESS_TEXTURES + 4096;
const REQUEST_QUEUE_CAPACITY: usize = 256;
const PREPARATION_LANE_COUNT: usize = 4;

pub(crate) type VisibilityResourceLoadingServer = ResourceLoadingServer<VisibilityRenderResource, VisibilityResourcePreparer>;
type CompletionList = SmallVec<[VisibilityResourceCompletion; 16]>;

/// The `ResourceClient` struct is the render-thread owner of visibility resource lifecycle and GPU placement.
pub(crate) struct ResourceClient {
	loader: ResourceLoader<VisibilityRenderResource>,
	uploads: FrameUploadQueue<PreparedUpload, VisibilityResourceCompletion>,
	store: ResourceStore,
	completions: CompletionList,
	dependencies: ResourceDependencies,
}

/// The `ResourceDependencies` struct preserves discovered mesh-to-material-to-texture demand across retries.
#[derive(Default)]
struct ResourceDependencies {
	mesh_materials: HashMap<ResourceRef, SmallVec<[ResourceRef; 8]>>,
	material_textures: HashMap<ResourceRef, SmallVec<[ResourceRef; 8]>>,
}

impl ResourceDependencies {
	/// Retries failed descendants without re-preparing an already resident mesh.
	fn retry_for_mesh(&self, loader: &mut ResourceLoader<VisibilityRenderResource>, mesh: ResourceRef) {
		let failed = |loader: &ResourceLoader<VisibilityRenderResource>, reference: ResourceRef| {
			matches!(loader.state(reference), ResourceState::Failed | ResourceState::Cancelled)
		};
		for material in self.mesh_materials.get(&mesh).into_iter().flatten() {
			if failed(loader, *material) {
				loader.retry(*material);
			}
			for texture in self.material_textures.get(material).into_iter().flatten() {
				if failed(loader, *texture) {
					loader.retry(*texture);
				}
			}
		}
	}
}

/// The `ResourceStore` struct records prepared uploads into visibility GPU storage and hands out stable shader-table slots.
pub(crate) struct ResourceStore {
	pub(crate) geometry: GeometryBuffers,
	material_slots: HashMap<String, u32>,
	texture_slots: HashMap<String, u32>,
	staging_buffer: ghi::BaseBufferHandle,
}

impl VisibilityResourcePreparer {
	/// Creates the render-thread client plus independent preparation lanes.
	///
	/// `staging_buffer` must back `upload_staging`. Run every returned server and the staging worker on
	/// application-owned async tasks, and keep the client inside the visibility pipeline manager.
	pub(crate) fn spawn(
		context: &mut ghi::implementation::Context,
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<UploadStagingArena>,
		staging_buffer: ghi::BaseBufferHandle,
		material_pipeline_config: MaterialPipelineConfig,
	) -> (ResourceClient, Vec<VisibilityResourceLoadingServer>) {
		let (loader, endpoint) = ResourceLoader::new(RESOURCE_LIMIT, REQUEST_QUEUE_CAPACITY);
		let servers = (0..PREPARATION_LANE_COUNT)
			.map(|_| {
				endpoint.clone().server(VisibilityResourcePreparer::new(
					resource_manager.clone(),
					upload_staging.clone(),
					context.create_factory(),
					material_pipeline_config.clone(),
				))
			})
			.collect();
		let client = ResourceClient {
			loader,
			uploads: FrameUploadQueue::default(),
			store: ResourceStore {
				geometry: GeometryBuffers::new(context),
				material_slots: HashMap::with_capacity(MAX_MATERIALS),
				texture_slots: HashMap::with_capacity(MAX_BINDLESS_TEXTURES),
				staging_buffer,
			},
			completions: CompletionList::new(),
			dependencies: ResourceDependencies::default(),
		};
		(client, servers)
	}
}

/// Returns or assigns a stable slot for `id`, failing once `limit` slots exist.
fn assign_slot(slots: &mut HashMap<String, u32>, id: &str, limit: usize, kind: &str) -> Option<u32> {
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

impl ResourceStore {
	pub(crate) fn material_slot(&mut self, id: &str) -> Option<u32> {
		assign_slot(&mut self.material_slots, id, MAX_MATERIALS, "material")
	}

	pub(crate) fn texture_slot(&mut self, id: &str) -> Option<u32> {
		assign_slot(&mut self.texture_slots, id, MAX_BINDLESS_TEXTURES, "texture")
	}

	fn record_mesh(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		key: MeshKey,
		mesh: &PreparedMesh,
	) -> Result<MeshData, VisibilityResourceKey> {
		let material_indices = mesh
			.primitives
			.iter()
			.map(|primitive| self.material_slot(&primitive.material_id))
			.collect::<Option<SmallVec<[u32; 8]>>>()
			.ok_or(VisibilityResourceKey::Mesh(key))?;
		self.geometry
			.write_mesh(transfer, self.staging_buffer, mesh, &material_indices)
			.ok_or(VisibilityResourceKey::Mesh(key))
	}
}

impl ResourceUploadStore for ResourceStore {
	type Upload = PreparedUpload;
	type Resident = VisibilityResourceCompletion;
	type Error = VisibilityResourceKey;

	fn record(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		upload: &PreparedUpload,
	) -> Result<VisibilityResourceCompletion, VisibilityResourceKey> {
		match upload {
			PreparedUpload::Mesh { key, mesh } => Ok(VisibilityResourceCompletion::MeshReady {
				key: *key,
				mesh: self.record_mesh(transfer, *key, mesh)?,
			}),
			PreparedUpload::Texture {
				id,
				image,
				sampler,
				upload,
				photometry,
			} => {
				let index = self
					.texture_slot(id)
					.ok_or_else(|| VisibilityResourceKey::Texture(id.clone()))?;
				transfer.copy_buffer_to_images(&upload.copy_descriptors(self.staging_buffer, *image));
				Ok(VisibilityResourceCompletion::TextureUploadReady {
					id: id.clone(),
					index,
					image: *image,
					sampler: *sampler,
					photometry: photometry.clone(),
				})
			}
			PreparedUpload::Environment(upload) => {
				let staging_offset = upload.staging.offset();
				let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
				copies.push(upload.diffuse_upload.copy_descriptor(
					self.staging_buffer,
					staging_offset,
					upload.diffuse_image,
					0,
				));
				for (mip_level, mip) in upload.specular_uploads.iter().enumerate() {
					copies.push(mip.copy_descriptor(
						self.staging_buffer,
						staging_offset,
						upload.specular_image,
						mip_level as u32,
					));
				}
				transfer.copy_buffer_to_images(&copies);
				Ok(VisibilityResourceCompletion::EnvironmentUploadReady {
					id: upload.id.clone(),
					diffuse_image: upload.diffuse_image,
					specular_image: upload.specular_image,
					sampler: upload.sampler,
				})
			}
		}
	}
}

impl ResourceClient {
	pub(crate) fn store(&mut self) -> &mut ResourceStore {
		&mut self.store
	}

	pub(crate) fn geometry(&self) -> &GeometryBuffers {
		&self.store.geometry
	}

	/// Coalesces one mesh request and retries any failed material or texture descendants.
	pub(crate) fn request_mesh(&mut self, key: MeshKey, source: MeshSource) {
		if let Some(reference) = self.request(
			VisibilityResourceKey::Mesh(key),
			VisibilityResourceRequest::Mesh { key, source },
		) {
			self.dependencies.retry_for_mesh(&mut self.loader, reference);
			self.loader.submit_requests(REQUEST_QUEUE_CAPACITY);
		}
	}

	pub(crate) fn request_image(&mut self, id: String) -> Option<ResourceRef> {
		self.request(
			VisibilityResourceKey::Texture(id.clone()),
			VisibilityResourceRequest::Image { id },
		)
	}

	pub(crate) fn request_environment(&mut self, id: String) {
		self.request(
			VisibilityResourceKey::Environment(id.clone()),
			VisibilityResourceRequest::Environment { id },
		);
	}

	fn request_material(&mut self, id: String) -> Option<ResourceRef> {
		self.request(
			VisibilityResourceKey::Material(id.clone()),
			VisibilityResourceRequest::Material { id },
		)
	}

	/// Coalesces and submits one logical request without blocking the render thread.
	fn request(&mut self, key: VisibilityResourceKey, request: VisibilityResourceRequest) -> Option<ResourceRef> {
		let Ok(reference) = self.loader.request(key.clone(), request) else {
			log::error!(
				"Visibility resource request limit exceeded for {key}. The most likely cause is that the renderer retained more logical resources than its registry supports."
			);
			return None;
		};
		if matches!(self.loader.state(reference), ResourceState::Failed | ResourceState::Cancelled) {
			self.loader.retry(reference);
		}
		self.loader.submit_requests(REQUEST_QUEUE_CAPACITY);
		Some(reference)
	}

	/// Retires completed frame uploads, adopts worker results, requests discovered dependencies, and reports whether uploads are pending.
	///
	/// Call this before scene draw preparation.
	pub(crate) fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool {
		for (_, resident) in self.uploads.retire_frame(completed_frame, &mut self.loader) {
			self.completions.push(resident);
		}
		for _ in 0..REQUEST_QUEUE_CAPACITY {
			let Some(completion) = self.loader.take_completion() else {
				break;
			};
			let token = completion.token();
			match completion.into_result() {
				Ok(VisibilityPreparedResource::Mesh { key, mesh }) => {
					let materials = mesh
						.primitives
						.iter()
						.filter_map(|primitive| self.request_material(primitive.material_id.clone()))
						.collect();
					self.dependencies.mesh_materials.insert(token.reference(), materials);
					self.uploads.enqueue(token, PreparedUpload::Mesh { key, mesh });
				}
				Ok(VisibilityPreparedResource::Material {
					id,
					alpha_mode,
					coverage,
					texture_ids,
					pipeline,
				}) => {
					let textures = texture_ids
						.iter()
						.flatten()
						.filter_map(|id| self.request_image(id.clone()))
						.collect();
					self.dependencies.material_textures.insert(token.reference(), textures);
					self.completions.push(VisibilityResourceCompletion::MaterialReady {
						token,
						id,
						pipeline,
						alpha_mode,
						coverage,
						texture_ids,
					});
				}
				Ok(VisibilityPreparedResource::Image(image)) => {
					self.completions
						.push(VisibilityResourceCompletion::ImageReady { token, image });
				}
				Ok(VisibilityPreparedResource::Environment { id, environment }) => {
					self.completions
						.push(VisibilityResourceCompletion::EnvironmentReady { token, id, environment });
				}
				Err(error) => self.completions.push(VisibilityResourceCompletion::Failed { key: error.key }),
			}
		}
		self.loader.submit_requests(REQUEST_QUEUE_CAPACITY);
		self.uploads.has_pending()
	}

	/// Records pending transfers through the store; store failures become logical failure completions.
	pub(crate) fn record_frame_uploads(
		&mut self,
		frame: ghi::FrameKey,
		recording: &mut ghi::implementation::CommandBufferRecording<'_>,
	) {
		let failures = self.uploads.record_frame(frame, recording, &mut self.loader, &mut self.store);
		for (token, fallback_key) in failures {
			let key = self.loader.key(token.reference()).cloned().unwrap_or(fallback_key);
			self.completions.push(VisibilityResourceCompletion::Failed { key });
		}
	}

	/// Moves every adopted outcome to the pipeline manager. Drain once per frame after [`Self::begin_frame`].
	pub(crate) fn drain_completions(&mut self) -> CompletionList {
		std::mem::take(&mut self.completions)
	}

	/// Enqueues one interned CPU texture for frame-tracked transfer recording.
	pub(crate) fn enqueue_texture_upload(
		&mut self,
		token: ResourceToken,
		id: String,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: StagedTextureUpload,
		photometry: Option<ImagePhotometry>,
	) {
		self.uploads.enqueue(
			token,
			PreparedUpload::Texture {
				id,
				image,
				sampler,
				upload,
				photometry,
			},
		);
	}

	/// Enqueues one interned environment batch for atomic frame-tracked transfer.
	pub(crate) fn enqueue_environment_upload(&mut self, token: ResourceToken, upload: PendingEnvironmentUpload) {
		self.uploads.enqueue(token, PreparedUpload::Environment(upload));
	}

	/// Claims one prepared resource before renderer-owned storage is changed.
	pub(crate) fn mark_uploading(&mut self, token: ResourceToken) -> bool {
		self.loader.mark_uploading(token)
	}

	/// Publishes one resource after its renderer-owned state is usable.
	pub(crate) fn mark_ready(&mut self, token: ResourceToken) -> bool {
		self.loader.mark_ready(token)
	}

	/// Rejects one claimed or prepared resource after renderer-side adoption fails.
	pub(crate) fn mark_failed(&mut self, token: ResourceToken) -> bool {
		self.loader.mark_failed(token)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resident_mesh_request_retries_failed_material_and_texture_dependencies() {
		let (mut loader, _endpoint) = ResourceLoader::<VisibilityRenderResource>::new(8, 8);
		let mut request = |id: &str| {
			loader
				.request(
					VisibilityResourceKey::Material(id.to_string()),
					VisibilityResourceRequest::Material { id: id.to_string() },
				)
				.unwrap_or_else(|_| panic!("Expected registry capacity."))
		};
		let mesh = request("mesh");
		let material = request("material");
		let texture = request("texture");
		assert_eq!(loader.submit_requests(8), 3);
		assert!(loader.mark_ready(loader.token(mesh).expect("mesh token")));
		assert!(loader.mark_failed(loader.token(material).expect("material token")));
		assert!(loader.mark_failed(loader.token(texture).expect("texture token")));
		let material_revision = loader.token(material).expect("failed material token").revision();
		let texture_revision = loader.token(texture).expect("failed texture token").revision();

		let mut dependencies = ResourceDependencies::default();
		dependencies.mesh_materials.entry(mesh).or_default().push(material);
		dependencies.material_textures.entry(material).or_default().push(texture);
		dependencies.retry_for_mesh(&mut loader, mesh);

		assert_eq!(loader.state(mesh), ResourceState::Ready);
		assert_eq!(loader.state(material), ResourceState::Queued);
		assert_eq!(loader.state(texture), ResourceState::Queued);
		assert!(loader.token(material).expect("retried material token").revision() > material_revision);
		assert!(loader.token(texture).expect("retried texture token").revision() > texture_revision);
	}

	#[test]
	fn slots_are_stable_and_bounded() {
		let mut slots = HashMap::default();
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "b", 2, "test"), Some(1));
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "c", 2, "test"), None);
	}
}
