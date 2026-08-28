//! Render-thread request routing, dependency recovery, upload recording, and resident publication.
//!
//! This is the reciprocal half of [`super::preparation`]. Preparation returns
//! logical, storage-independent values; this module assigns Visibility buffer
//! offsets and shader-table slots, retains transfer inputs through frame
//! completion, and emits [`super::VisibilityResourceCompletion`] values for the
//! pipeline manager. No method here runs on a loading server lane.

use super::*;
use crate::rendering::resource_loading::{
	FrameUploadQueue, ResourceLoader, ResourceLoadingServer, ResourceRef, ResourceState, ResourceToken, ResourceUploadStore,
};

const VISIBILITY_RESOURCE_LIMIT: usize = MAX_MATERIALS + MAX_BINDLESS_TEXTURES + 4096;
const VISIBILITY_RESOURCE_QUEUE_CAPACITY: usize = 256;
const VISIBILITY_PREPARATION_LANE_COUNT: usize = 4;

pub(crate) type VisibilityResourceLoadingServer = ResourceLoadingServer<VisibilityRenderResource, VisibilityResourcePreparer>;

/// The `VisibilityPipelineResourceManagerClient` struct coordinates Visibility requests from scene demand through resident publication.
///
/// Keep this value on the render thread. It owns the shared loader, frame upload
/// queue, dependency graph, and Visibility store so lifecycle changes and exact
/// storage mutations occur at one boundary.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	loader: ResourceLoader<VisibilityRenderResource>,
	uploads: FrameUploadQueue<PreparedUpload, VisibilityResourceCompletion>,
	store: VisibilityResourceStore,
	completions: CompletionList,
	dependencies: VisibilityResourceDependencies,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resident_mesh_request_retries_failed_material_and_texture_dependencies() {
		let (mut loader, _endpoint) = ResourceLoader::<VisibilityRenderResource>::new(8, 8);
		let mesh = loader
			.request(
				VisibilityResourceKey::Material("mesh".to_string()),
				VisibilityResourceRequest::Material { id: "mesh".to_string() },
			)
			.unwrap_or_else(|_| panic!("Expected mesh registry capacity."));
		let material = loader
			.request(
				VisibilityResourceKey::Material("material".to_string()),
				VisibilityResourceRequest::Material {
					id: "material".to_string(),
				},
			)
			.unwrap_or_else(|_| panic!("Expected material registry capacity."));
		let texture = loader
			.request(
				VisibilityResourceKey::Texture(VisibilityTextureKey::new("texture".to_string())),
				VisibilityResourceRequest::Image {
					key: VisibilityTextureKey::new("texture".to_string()),
				},
			)
			.unwrap_or_else(|_| panic!("Expected texture registry capacity."));
		assert_eq!(loader.submit_requests(8), 3);
		assert!(loader.mark_ready(loader.token(mesh).expect("mesh token")));
		assert!(loader.mark_failed(loader.token(material).expect("material token")));
		assert!(loader.mark_failed(loader.token(texture).expect("texture token")));
		let material_revision = loader.token(material).expect("failed material token").revision();
		let texture_revision = loader.token(texture).expect("failed texture token").revision();

		let mut dependencies = VisibilityResourceDependencies::default();
		dependencies.mesh_materials.entry(mesh).or_default().push(material);
		dependencies.material_textures.entry(material).or_default().push(texture);
		dependencies.retry_for_mesh(&mut loader, mesh);

		assert_eq!(loader.state(mesh), ResourceState::Ready);
		assert_eq!(loader.state(material), ResourceState::Queued);
		assert_eq!(loader.state(texture), ResourceState::Queued);
		assert!(loader.token(material).expect("retried material token").revision() > material_revision);
		assert!(loader.token(texture).expect("retried texture token").revision() > texture_revision);
	}
}

/// The `VisibilityResourceDependencies` struct preserves discovered mesh-to-material-to-texture demand across retries.
///
/// The graph uses stable loader references so descendant retries do not reload
/// an already resident parent. It belongs to Visibility because the shared
/// lifecycle has no knowledge of renderer-specific dependencies.
#[derive(Default)]
struct VisibilityResourceDependencies {
	mesh_materials: HashMap<ResourceRef, SmallVec<[ResourceRef; 8]>>,
	material_textures: HashMap<ResourceRef, SmallVec<[ResourceRef; 8]>>,
}

impl VisibilityResourceDependencies {
	/// Retries failed descendants without re-preparing an already resident mesh.
	fn retry_for_mesh(&self, loader: &mut ResourceLoader<VisibilityRenderResource>, mesh: ResourceRef) {
		let Some(materials) = self.mesh_materials.get(&mesh) else {
			return;
		};
		for material in materials {
			if matches!(loader.state(*material), ResourceState::Failed | ResourceState::Cancelled) {
				loader.retry(*material);
			}
			let Some(textures) = self.material_textures.get(material) else {
				continue;
			};
			for texture in textures {
				if matches!(loader.state(*texture), ResourceState::Failed | ResourceState::Cancelled) {
					loader.retry(*texture);
				}
			}
		}
	}
}

/// The `VisibilityResourceStore` struct centralizes Visibility GPU placement and stable shader-table identity.
///
/// This is the renderer policy seam: meshlets and each vertex property use
/// Visibility's parallel streams, while materials and textures receive stable
/// shader-table slots. The shared frame queue sees only prepared and resident
/// values.
pub(crate) struct VisibilityResourceStore {
	pub(crate) gpu_vertex_data_manager: GPUVertexDataManager,
	material_slots: HashMap<String, u32>,
	texture_slots: HashMap<String, u32>,
	staging_data_buffer: ghi::BaseBufferHandle,
}

impl VisibilityResourcePreparer {
	/// Creates the render-thread client plus independent sequential preparation lanes.
	///
	/// `staging_data_buffer` must back `upload_staging`. Run every returned server
	/// and the staging worker on application-owned async tasks. Keep the returned
	/// client inside the Visibility pipeline manager.
	pub(crate) fn spawn(
		context: &mut ghi::implementation::Context,
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
		staging_data_buffer: ghi::BaseBufferHandle,
		material_pipeline_config: MaterialPipelineConfig,
	) -> (VisibilityPipelineResourceManagerClient, Vec<VisibilityResourceLoadingServer>) {
		let (loader, endpoint) = ResourceLoader::new(VISIBILITY_RESOURCE_LIMIT, VISIBILITY_RESOURCE_QUEUE_CAPACITY);
		let mut servers = Vec::with_capacity(VISIBILITY_PREPARATION_LANE_COUNT);
		for _ in 0..VISIBILITY_PREPARATION_LANE_COUNT {
			let preparer = VisibilityResourcePreparer::new(
				resource_manager.clone(),
				upload_staging.clone(),
				context.create_factory(),
				material_pipeline_config.clone(),
			);
			servers.push(endpoint.clone().server(preparer));
		}

		(
			VisibilityPipelineResourceManagerClient {
				loader,
				uploads: FrameUploadQueue::default(),
				store: VisibilityResourceStore::new(context, staging_data_buffer),
				completions: CompletionList::new(),
				dependencies: VisibilityResourceDependencies::default(),
			},
			servers,
		)
	}
}

impl VisibilityResourceStore {
	/// Creates the renderer-owned storage policy used to record prepared Visibility resources.
	///
	/// Slot maps start empty and grow only on the render thread. Vertex storage is
	/// delegated to [`GPUVertexDataManager`] because that type owns the exact
	/// parallel-stream and meshlet allocation rules.
	fn new(context: &mut ghi::implementation::Context, staging_data_buffer: ghi::BaseBufferHandle) -> Self {
		Self {
			gpu_vertex_data_manager: GPUVertexDataManager::new(context),
			material_slots: HashMap::with_capacity(4096),
			texture_slots: HashMap::with_capacity(4096),
			staging_data_buffer,
		}
	}

	/// Returns the stable material slot assigned by this Visibility implementation.
	fn material_slot(&mut self, id: &str) -> Result<u32, VisibilityResourceKey> {
		if let Some(index) = self.material_slots.get(id) {
			return Ok(*index);
		}
		let index = self.material_slots.len();
		if index >= MAX_MATERIALS {
			log::error!(
				"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
			);
			return Err(VisibilityResourceKey::Material(id.to_string()));
		}
		let index = index as u32;
		self.material_slots.insert(id.to_string(), index);
		Ok(index)
	}

	/// Returns the stable bindless texture slot assigned by this Visibility implementation.
	fn texture_slot(&mut self, key: &VisibilityTextureKey) -> Result<u32, VisibilityResourceKey> {
		if let Some(index) = self.texture_slots.get(key.as_str()) {
			return Ok(*index);
		}
		let index = self.texture_slots.len();
		if index >= MAX_BINDLESS_TEXTURES {
			log::error!(
				"Visibility texture limit exceeded. The most likely cause is that the scene created more texture variants than the visibility pipeline supports."
			);
			return Err(VisibilityResourceKey::Texture(key.clone()));
		}
		let index = index as u32;
		self.texture_slots.insert(key.as_str().to_string(), index);
		Ok(index)
	}

	/// Rejects inconsistent metadata before transfer recording consumes GPU capacity.
	fn resource_mesh_metadata_is_valid(
		mesh: &PreparedGpuMesh,
		material_indices: &[u32],
		primitive_skins: &[Option<u32>],
		skin_binding_count: usize,
	) -> bool {
		let expected = mesh.render_primitive_count();
		if material_indices.len() != expected || primitive_skins.len() != expected {
			log::error!(
				"Visibility mesh primitive count changed before transfer. The most likely cause is inconsistent mesh metadata."
			);
			return false;
		}
		if let Some(skin_index) = primitive_skins
			.iter()
			.flatten()
			.find(|skin_index| **skin_index as usize >= skin_binding_count)
		{
			log::error!(
				"Visibility mesh skin index is invalid before transfer: {}. The most likely cause is that mesh validation was bypassed or the resource data is corrupted.",
				skin_index
			);
			return false;
		}
		true
	}

	/// Records one resource mesh after resolving its logical material IDs to stable renderer slots.
	fn record_resource_mesh(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		mesh: &PreparedGpuMesh,
		material_ids: &[String],
		primitive_skins: &[Option<u32>],
		skin_bindings: &[Arc<resource_management::resources::skeleton::SkinBinding>],
		skeleton_node_count: u32,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, VisibilityResourceKey> {
		let material_indices = material_ids
			.iter()
			.map(|id| self.material_slot(id))
			.collect::<Result<Vec<_>, _>>()?;
		if !Self::resource_mesh_metadata_is_valid(mesh, &material_indices, primitive_skins, skin_bindings.len()) {
			return Err(VisibilityResourceKey::Material(
				material_ids.first().cloned().unwrap_or_default(),
			));
		}
		let mesh_data = self
			.gpu_vertex_data_manager
			.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, self.staging_data_buffer, mesh)
			.ok_or_else(|| VisibilityResourceKey::Material(material_ids.first().cloned().unwrap_or_default()))?;
		Ok(Self::convert_resource_mesh_data(
			mesh_data,
			material_indices,
			primitive_skins.to_vec(),
			skin_bindings.to_vec(),
			skeleton_node_count,
		))
	}

	/// Combines uploaded resource geometry with renderer-owned dependency slots.
	fn convert_resource_mesh_data(
		mesh: GpuMeshData,
		material_indices: Vec<u32>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	) -> crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
		let primitives = material_indices
			.into_iter()
			.zip(primitive_skins)
			.zip(mesh.primitives.iter())
			.map(|((material_index, skin_index), primitive)| {
				let skin = skin_index.map(|skin_index| {
					skin_bindings
						.get(skin_index as usize)
						.expect("Visibility skin indices were validated before transfer recording.")
						.clone()
				});
				crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
					skinning_source_vertex_offset: primitive.skinning_source_vertex_offset,
					skinning_vertex_count: primitive.skinning_vertex_count,
					skin,
				}
			})
			.collect();
		crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			skeleton_node_count,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		}
	}

	/// Maps generated mesh geometry to render-facing metadata using its renderer-owned material slot.
	fn convert_generated_mesh_data(
		mesh: GpuMeshData,
		material_index: u32,
	) -> crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
		let primitives = mesh
			.primitives
			.iter()
			.map(
				|primitive| crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
					skinning_source_vertex_offset: primitive.skinning_source_vertex_offset,
					skinning_vertex_count: primitive.skinning_vertex_count,
					skin: None,
				},
			)
			.collect();
		crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			skeleton_node_count: 0,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		}
	}
}

impl ResourceUploadStore for VisibilityResourceStore {
	type Upload = PreparedUpload;
	type Resident = VisibilityResourceCompletion;
	type Error = VisibilityResourceKey;

	/// Records one prepared value into Visibility's parallel GPU streams or image layout.
	fn record(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		upload: &PreparedUpload,
	) -> Result<VisibilityResourceCompletion, VisibilityResourceKey> {
		match upload {
			PreparedUpload::ResourceMesh {
				key,
				mesh,
				material_ids,
				primitive_skins,
				skin_bindings,
				skeleton_node_count,
			} => self
				.record_resource_mesh(
					transfer,
					mesh,
					material_ids,
					primitive_skins,
					skin_bindings,
					*skeleton_node_count,
				)
				.map(|mesh| VisibilityResourceCompletion::MeshReady { key: *key, mesh })
				.map_err(|_| VisibilityResourceKey::Mesh(*key)),
			PreparedUpload::GeneratedMesh { key, mesh, material_id } => {
				let material_index = self
					.material_slot(material_id)
					.map_err(|_| VisibilityResourceKey::Mesh(*key))?;
				let mesh = self
					.gpu_vertex_data_manager
					.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, self.staging_data_buffer, mesh)
					.map(|mesh| Self::convert_generated_mesh_data(mesh, material_index))
					.ok_or(VisibilityResourceKey::Mesh(*key))?;
				Ok(VisibilityResourceCompletion::MeshReady { key: *key, mesh })
			}
			PreparedUpload::Texture {
				key,
				image,
				sampler,
				upload,
				photometry,
			} => {
				let index = self.texture_slot(key)?;
				let copies = upload
					.layouts
					.iter()
					.enumerate()
					.map(|(level, layout)| {
						staged_texture_copy(
							self.staging_data_buffer,
							upload.staging.offset(),
							*image,
							layout,
							level as u32,
						)
					})
					.collect::<SmallVec<[ghi::BufferImageCopyDescriptor; 16]>>();
				transfer.copy_buffer_to_images(&copies);
				Ok(VisibilityResourceCompletion::TextureUploadReady {
					key: key.clone(),
					index,
					image: *image,
					sampler: *sampler,
					photometry: photometry.clone(),
				})
			}
			PreparedUpload::Environment(upload) => {
				let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
				copies.push(staged_texture_copy(
					self.staging_data_buffer,
					upload.staging.offset(),
					upload.diffuse_image,
					&upload.diffuse_upload,
					0,
				));
				for (mip_level, mip) in upload.specular_uploads.iter().enumerate() {
					copies.push(staged_texture_copy(
						self.staging_data_buffer,
						upload.staging.offset(),
						upload.specular_image,
						mip,
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

impl VisibilityPipelineResourceManagerClient {
	/// Returns read-only Visibility storage for draw and statistics consumers.
	pub(crate) fn store(&self) -> &VisibilityResourceStore {
		&self.store
	}

	/// Returns or assigns the stable shader-table slot for one material ID.
	pub(crate) fn material_slot(&mut self, id: &str) -> Result<u32, VisibilityResourceKey> {
		self.store.material_slot(id)
	}

	/// Returns or assigns the stable bindless slot for one texture key.
	pub(crate) fn texture_slot(&mut self, key: &VisibilityTextureKey) -> Result<u32, VisibilityResourceKey> {
		self.store.texture_slot(key)
	}

	/// Coalesces one mesh request and retries any failed material or texture descendants.
	pub(crate) fn request_mesh(&mut self, key: VisibilityMeshKey, source: MeshSource) {
		let reference = self.request(
			VisibilityResourceKey::Mesh(key),
			VisibilityResourceRequest::Mesh { key, source },
		);
		if let Some(reference) = reference {
			self.dependencies.retry_for_mesh(&mut self.loader, reference);
			self.loader.submit_requests(VISIBILITY_RESOURCE_QUEUE_CAPACITY);
		}
	}

	/// Coalesces one image request by resource ID.
	pub(crate) fn request_image(&mut self, id: String) {
		self.request_image_key(VisibilityTextureKey::new(id));
	}

	/// Coalesces one baked environment request by resource ID.
	pub(crate) fn request_environment(&mut self, id: String) {
		self.request(
			VisibilityResourceKey::Environment(id.clone()),
			VisibilityResourceRequest::Environment { id },
		);
	}

	/// Coalesces and submits one logical request without blocking the render thread.
	fn request(&mut self, key: VisibilityResourceKey, request: VisibilityResourceRequest) -> Option<ResourceRef> {
		let reference = match self.loader.request(key.clone(), request) {
			Ok(reference) => reference,
			Err(_) => {
				log::error!(
					"Visibility resource request limit exceeded for {}. The most likely cause is that the renderer retained more logical resources than its configured registry supports.",
					key
				);
				return None;
			}
		};
		if matches!(self.loader.state(reference), ResourceState::Failed | ResourceState::Cancelled) {
			self.loader.retry(reference);
		}
		self.loader.submit_requests(VISIBILITY_RESOURCE_QUEUE_CAPACITY);
		Some(reference)
	}

	fn request_material(&mut self, id: String) -> Option<ResourceRef> {
		self.request(
			VisibilityResourceKey::Material(id.clone()),
			VisibilityResourceRequest::Material { id },
		)
	}

	fn request_image_key(&mut self, key: VisibilityTextureKey) -> Option<ResourceRef> {
		self.request(
			VisibilityResourceKey::Texture(key.clone()),
			VisibilityResourceRequest::Image { key },
		)
	}

	/// Releases completed staging leases and adopts current preparation results at the frame boundary.
	///
	/// Call this before scene draw preparation. It first returns completed frame
	/// uploads, then drains worker results, discovers dependent material and image
	/// requests, and reports whether the renderer must record new uploads.
	pub(crate) fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool {
		for (_, resident) in self.uploads.retire_frame(completed_frame, &mut self.loader) {
			self.completions.push(resident);
		}

		for _ in 0..VISIBILITY_RESOURCE_QUEUE_CAPACITY {
			let Some(completion) = self.loader.take_completion() else {
				break;
			};
			let token = completion.token();
			match completion.into_result() {
				Ok(VisibilityPreparedResource::Mesh(upload)) => {
					let mut dependencies = SmallVec::<[ResourceRef; 8]>::new();
					match &upload {
						PreparedUpload::ResourceMesh { material_ids, .. } => {
							for material_id in material_ids {
								if let Some(reference) = self.request_material(material_id.clone()) {
									dependencies.push(reference);
								}
							}
						}
						PreparedUpload::GeneratedMesh { material_id, .. } => {
							if let Some(reference) = self.request_material(material_id.clone()) {
								dependencies.push(reference);
							}
						}
						PreparedUpload::Texture { .. } | PreparedUpload::Environment(_) => unreachable!(
							"Visibility mesh preparation returned a non-mesh upload. The most likely cause is a mismatched resource protocol."
						),
					}
					self.dependencies.mesh_materials.insert(token.reference(), dependencies);
					self.uploads.enqueue(token, upload);
				}
				Ok(VisibilityPreparedResource::Material {
					id,
					alpha_mode,
					coverage,
					texture_keys,
					pipeline,
				}) => {
					let mut dependencies = SmallVec::<[ResourceRef; 8]>::new();
					for key in texture_keys.iter().flatten() {
						if let Some(reference) = self.request_image_key(key.clone()) {
							dependencies.push(reference);
						}
					}
					self.dependencies.material_textures.insert(token.reference(), dependencies);
					self.completions.push(VisibilityResourceCompletion::MaterialReady {
						token,
						id,
						pipeline,
						alpha_mode,
						coverage,
						textures: texture_keys
							.into_iter()
							.map(|key| key.map(VisibilityTextureKey::into_string))
							.collect(),
					});
				}
				Ok(VisibilityPreparedResource::Image(image)) => match image {
					PreparedVisibilityImage::Cpu {
						key,
						image,
						sampler,
						upload,
						photometry,
					} => self.completions.push(VisibilityResourceCompletion::ImageReady {
						token,
						key,
						image,
						sampler,
						upload,
						photometry,
					}),
					PreparedVisibilityImage::Gpu {
						key,
						image,
						sampler,
						backing,
						streams,
						format,
						extent,
						mip_count,
						photometry,
					} => self.completions.push(VisibilityResourceCompletion::GpuImageReady {
						token,
						key,
						image,
						sampler,
						backing,
						streams,
						format,
						extent,
						mip_count,
						photometry,
					}),
				},
				Ok(VisibilityPreparedResource::Environment { id, environment }) => {
					self.completions
						.push(VisibilityResourceCompletion::EnvironmentReady { token, id, environment });
				}
				Err(error) => self.completions.push(VisibilityResourceCompletion::Failed { key: error.key }),
			}
		}
		self.loader.submit_requests(VISIBILITY_RESOURCE_QUEUE_CAPACITY);
		self.uploads.has_pending()
	}

	/// Records pending Visibility transfers through the renderer-owned store.
	///
	/// Call this only when [`Self::begin_frame`] reports work. Store failures are
	/// converted to logical failure completions so scene adoption follows the
	/// same reporting path as preparation failures.
	pub(crate) fn record_frame_uploads(
		&mut self,
		frame: ghi::FrameKey,
		recording: &mut ghi::implementation::CommandBufferRecording<'_>,
	) {
		let failures = self.uploads.record_frame(frame, recording, &mut self.loader, &mut self.store);
		for (token, fallback_key) in failures {
			// The loader token owns logical routing identity. Store failures only
			// supply a fallback when an internal invariant has already been broken.
			let key = self.loader.key(token.reference()).cloned().unwrap_or(fallback_key);
			self.completions.push(VisibilityResourceCompletion::Failed { key });
		}
	}

	/// Moves all adopted outcomes to the Visibility pipeline manager.
	///
	/// Drain once per frame after [`Self::begin_frame`]. The pipeline manager must
	/// finish interning or native-I/O adoption and then call [`Self::mark_ready`]
	/// or [`Self::mark_failed`] for variants that carry a token.
	pub(crate) fn drain_completions(&mut self) -> CompletionList {
		self.completions.drain(..).collect()
	}

	/// Enqueues one interned CPU texture for frame-tracked transfer recording.
	pub(crate) fn enqueue_texture_upload(
		&mut self,
		token: ResourceToken,
		key: VisibilityTextureKey,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	) {
		self.uploads.enqueue(
			token,
			PreparedUpload::Texture {
				key,
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

	#[cfg(test)]
	pub(crate) fn resource_mesh_metadata_is_valid(
		mesh: &PreparedGpuMesh,
		material_indices: &[u32],
		primitive_skins: &[Option<u32>],
		skin_binding_count: usize,
	) -> bool {
		VisibilityResourceStore::resource_mesh_metadata_is_valid(mesh, material_indices, primitive_skins, skin_binding_count)
	}
}
