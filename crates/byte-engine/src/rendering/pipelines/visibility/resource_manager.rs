/// The `VisibilityPipelineResourceManager` struct owns asynchronous visibility resource workloads.
pub(crate) struct VisibilityPipelineResourceManager {
	/// Image resources used by material evaluation.
	images: Vec<ResourceStates<(), ()>>,
	/// Mapping from image resource ID to image index.
	images_by_resource: HashMap<String, usize>,
	/// Material pipelines
	materials: Vec<ResourceStates<String, ()>>,
	/// Mapping from material ID to material index.
	material_by_name: HashMap<String, usize>,
	/// GPU vertex data manager (vertex positions, normals, UVs, indices, meshlets).
	gpu_vertex_data_manager: GPUVertexDataManager,
	/// Resource manager for loading assets.
	resource_manager: EntityHandle<ResourceManager>,
	pipelines: RwLock<HashMap<String, PipelineStatus>>,
	// Async requests cannot reload shader bytes after a sync load consumes the read target,
	// so we keep an owned backing for the shader payload keyed by resource hash.
	shader_requests: RwLock<StaleHashMap<String, u64, Arc<OwnedShader>>>,
	compute_pipeline_requests: Option<Sender<ComputePipelineRequest>>,
	compute_pipeline_results: Option<Receiver<ComputePipelineResult>>,
	resource_factory: Option<ghi::implementation::Factory>,
	material_pipeline_config: Option<MaterialPipelineConfig>,
	work_completions: Sender<VisibilityResourceCompletion>,
}

impl VisibilityPipelineResourceManager {
	pub(crate) fn spawn(
		context: &mut ghi::implementation::Context,
		resource_manager: EntityHandle<ResourceManager>,
	) -> (
		VisibilityPipelineResourceManagerClient,
		VisibilityPipelineResourceManagerWorker,
	) {
		let mesh_data_manager = GPUVertexDataManager::new(context);
		let gpu_vertex_data_manager = mesh_data_manager.clone();
		let (commands, command_receiver) = mpsc::channel();
		let (work_completions, work_completion_receiver) = mpsc::channel();
		let resource_manager = Self::new(context, resource_manager, mesh_data_manager, work_completions.clone());

		(
			VisibilityPipelineResourceManagerClient {
				gpu_vertex_data_manager,
				commands,
				completions: work_completion_receiver,
			},
			VisibilityPipelineResourceManagerWorker {
				resource_manager,
				commands: command_receiver,
				completions: work_completions,
				pending_mesh_uploads: VecDeque::new(),
				pending_texture_uploads: VecDeque::new(),
				submitted_uploads: VecDeque::new(),
			},
		)
	}

	fn new(
		context: &mut ghi::implementation::Context,
		resource_manager: EntityHandle<ResourceManager>,
		mesh_data_manager: GPUVertexDataManager,
		work_completions: Sender<VisibilityResourceCompletion>,
	) -> Self {
		let resource_factory = context.create_factory();
		let (compute_pipeline_requests, compute_pipeline_results) = if let Some(factory) = context.create_factory() {
			let (requests, results) = Self::spawn_compute_worker(factory);
			(Some(requests), Some(results))
		} else {
			(None, None)
		};

		Self {
			images: Vec::with_capacity(4096),
			images_by_resource: HashMap::with_capacity(4096),
			materials: Vec::with_capacity(4096),
			material_by_name: HashMap::with_capacity(4096),
			gpu_vertex_data_manager: mesh_data_manager,
			resource_manager,
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shader_requests: RwLock::new(StaleHashMap::with_capacity(1024)),
			compute_pipeline_requests,
			compute_pipeline_results,
			resource_factory,
			material_pipeline_config: None,
			work_completions,
		}
	}

	fn handle_request(&mut self, request: VisibilityResourceRequest) -> ResourceWorkerFlow {
		match request {
			VisibilityResourceRequest::Mesh { key: _, source: _ } => {}
			VisibilityResourceRequest::Material { id } => self.handle_material_request(id),
			VisibilityResourceRequest::Image { key } => self.handle_image_request(key),
			VisibilityResourceRequest::Shutdown => return ResourceWorkerFlow::Stop,
		}

		ResourceWorkerFlow::Continue
	}

	/// Stores the descriptor layout data needed to compile material evaluation pipelines.
	pub(crate) fn configure_material_pipeline(&mut self, config: MaterialPipelineConfig) {
		self.material_pipeline_config = Some(config);
	}

	/// Loads a material variant resource, reserves its texture dependencies, and queues its material evaluation pipeline.
	fn handle_material_request(&mut self, id: String) {
		let index = self.reserve_material_slot(&id).0;
		let result = self.load_variant_metadata(&id, index);
		let completion = match result {
			Ok(material) => VisibilityResourceCompletion::MaterialReady {
				id,
				index,
				pipeline: material.pipeline,
				alpha: material.alpha,
				textures: material.textures,
			},
			Err(()) => VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Material(id),
			},
		};

		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility material completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Loads one texture resource and reports render-thread creation data.
	fn handle_image_request(&mut self, key: VisibilityTextureKey) {
		let index = self.reserve_texture_slot(key.as_str()).0;
		let result = self.load_texture_with_factory(key.as_str(), index);
		let completion = match result {
			Ok(texture) => VisibilityResourceCompletion::ImageReady {
				key,
				index,
				image: texture.image,
				sampler: texture.sampler,
				upload: texture.upload,
			},
			Err(()) => VisibilityResourceCompletion::Failed { key: key.into() },
		};

		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility texture completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Reads material variant metadata while scheduling texture and pipeline dependencies.
	fn load_variant_metadata(&mut self, id: &str, index: u32) -> Result<FactoryMaterial, ()> {
		let mut reference: Reference<ResourceVariant> = self.resource_manager.request(id).map_err(|_| {
			log::error!(
				"Visibility material variant request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
				id
			);
		})?;

		let variant = reference.resource_mut();
		let material = variant.material.resource_mut();
		if material.model.name != "Visibility" || material.model.pass != "MaterialEvaluation" {
			log::error!(
				"Unsupported visibility material model for {}. The most likely cause is that this material targets a different render model or pass.",
				id
			);
			return Err(());
		}

		let specialization_map_entries = variant
			.variables
			.iter()
			.enumerate()
			.filter_map(|(index, variable)| match &variable.value {
				Value::Scalar(value) => {
					ghi::pipelines::SpecializationMapEntry::new(index as u32, "f32".to_string(), *value).into()
				}
				Value::Vector3(value) => {
					ghi::pipelines::SpecializationMapEntry::new(index as u32, "vec3f".to_string(), *value).into()
				}
				Value::Vector4(value) => {
					ghi::pipelines::SpecializationMapEntry::new(index as u32, "vec4f".to_string(), *value).into()
				}
				Value::Image(_) => None,
			})
			.collect::<Vec<_>>();

		let textures = variant
			.variables
			.iter_mut()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => {
					let key = VisibilityTextureKey::new(image.id());
					let texture_index = self.request_texture_dependency(key.clone());
					Some((key.as_str().to_string(), texture_index))
				}
				_ => None,
			})
			.collect::<Vec<_>>();
		let alpha = !matches!(variant.alpha_mode, resource_management::types::AlphaMode::Opaque);
		let pipeline = self.queue_configured_variant_pipeline(id.to_string(), material, specialization_map_entries);

		Ok(FactoryMaterial {
			index,
			pipeline,
			alpha,
			textures,
		})
	}

	/// Queues a texture dependency discovered while loading another resource.
	fn request_texture_dependency(&mut self, key: VisibilityTextureKey) -> u32 {
		let (index, inserted) = self.reserve_texture_slot(key.as_str());
		if inserted {
			self.handle_image_request(key);
		}
		index
	}

	/// Queues a material evaluation pipeline with the descriptor configuration supplied by the render thread.
	fn queue_configured_material_pipeline(&self, id: String, material: &mut ResourceMaterial) -> Option<ghi::PipelineHandle> {
		let Some(config) = self.material_pipeline_config.as_ref() else {
			log::error!(
				"Visibility material pipeline configuration is unavailable for {}. The most likely cause is that the render pipeline manager has not configured the resource worker yet.",
				id
			);
			return None;
		};

		self.queue_material_pipeline(id, &config.descriptor_set_templates, &config.push_constant_ranges, material)
	}

	/// Queues a material variant pipeline with the descriptor configuration supplied by the render thread.
	fn queue_configured_variant_pipeline(
		&self,
		id: String,
		material: &mut ResourceMaterial,
		specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
	) -> Option<ghi::PipelineHandle> {
		let Some(config) = self.material_pipeline_config.as_ref() else {
			log::error!(
				"Visibility material pipeline configuration is unavailable for {}. The most likely cause is that the render pipeline manager has not configured the resource worker yet.",
				id
			);
			return None;
		};

		self.queue_material_pipeline_with_specialization(
			id,
			&config.descriptor_set_templates,
			&config.push_constant_ranges,
			material,
			specialization_map_entries,
		)
	}

	/// Loads texture bytes and builds detached GPU resources for render-thread adoption.
	fn load_texture_with_factory(&mut self, id: &str, index: u32) -> Result<FactoryTexture, ()> {
		let mut reference: Reference<ResourceImage> = self.resource_manager.request(id).map_err(|_| {
			log::error!(
				"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
				id
			);
		})?;
		let texture = reference.resource();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);

		let mut source = vec![0u8; reference.size];
		let load_target = reference.load(source.as_mut_slice().into()).map_err(|_| {
			log::error!(
				"Visibility texture load failed for {}. The most likely cause is that the texture payload could not be read from storage.",
				id
			);
		})?;
		let source = load_target.buffer().ok_or_else(|| {
			log::error!(
				"Visibility texture load target is not CPU-readable for {}. The most likely cause is that the image resource did not load into a byte buffer.",
				id
			);
		})?;
		let upload = make_texture_upload(format, extent, source).ok_or_else(|| {
			log::error!(
				"Visibility texture upload preparation failed for {}. The most likely cause is that the source bytes do not match the texture format and extent.",
				id
			);
		})?;
		let factory = self.resource_factory.as_mut().ok_or_else(|| {
			log::error!(
				"Visibility texture factory is unavailable for {}. The most likely cause is that the active backend does not expose a generic resource factory.",
				id
			);
		})?;
		let image = factory.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let sampler = factory.build_sampler(default_material_sampler_builder());

		Ok(FactoryTexture {
			index,
			image,
			sampler,
			upload,
		})
	}

	/// Reserves a bindless texture slot and reports whether the slot was newly created.
	fn reserve_texture_slot(&mut self, texture_id: &str) -> (u32, bool) {
		let texture_id = texture_id.to_string();

		match self.images_by_resource.entry(texture_id) {
			Entry::Occupied(v) => (*v.get() as u32, false),
			Entry::Vacant(v) => {
				let idx = self.images.len() as u32;

				if idx as usize >= 1024 {
					panic!(
						"Visibility texture limit exceeded. The most likely cause is that the scene created more texture variants than the visibility pipeline supports."
					);
				}

				self.images.push(ResourceStates::pending(()));
				v.insert(idx as usize);

				(idx, true)
			}
		}
	}

	/// Reserves a material slot for a mesh primitive.
	fn request_material(&mut self, material_id: &str) -> u32 {
		let (index, inserted) = self.reserve_material_slot(material_id);
		if inserted {
			self.handle_material_request(material_id.to_string());
		}
		index
	}

	/// Reserves a material slot and reports whether the slot was newly created.
	fn reserve_material_slot(&mut self, material_id: &str) -> (u32, bool) {
		let material_id = material_id.to_string();

		match self.material_by_name.entry(material_id.clone()) {
			Entry::Occupied(v) => (*v.get() as u32, false),
			Entry::Vacant(v) => {
				let idx = self.materials.len() as u32;

				if idx as usize >= MAX_MATERIALS {
					panic!(
						"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
					);
				}

				self.materials.push(ResourceStates::pending(material_id));
				v.insert(idx as usize);

				(idx, true)
			}
		}
	}

	/// Records a mesh source upload and returns render-facing mesh metadata for scene resolution.
	fn load_mesh_source_for_transfer<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		mesh_source: &MeshSource,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		match mesh_source {
			MeshSource::Resource(id) => {
				let mut resource: Reference<ResourceMesh> = self.resource_manager.request(*id).map_err(|_| {
					log::error!(
						"Visibility mesh resource request failed for {}. The most likely cause is that the mesh id is missing or the asset database is not loaded.",
						id
					);
				})?;
				self.load_mesh_resource_for_transfer(transfer, staging_data_buffer, slice, &mut resource)
			}
			MeshSource::Generated(generator) => {
				let mesh = self
					.gpu_vertex_data_manager
					.write_gpu_mesh_data_and_return_mesh_object_for_mesh_generator(
						generator.as_ref(),
						transfer,
						staging_data_buffer,
						slice,
					)
					.ok_or(())?;
				self.convert_generated_mesh_data(mesh)
			}
		}
	}

	/// Records a resource-backed mesh upload and maps primitive material references to material slots.
	fn load_mesh_resource_for_transfer<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		resource: &mut Reference<ResourceMesh>,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		let mesh = self
			.gpu_vertex_data_manager
			.write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource(transfer, staging_data_buffer, slice, resource)
			.ok_or(())?;

		let resource = resource.resource();
		let primitives = resource
			.primitives
			.iter()
			.zip(mesh.primitives.iter())
			.map(|(resource_primitive, primitive)| {
				let material_index = self.request_material(&resource_primitive.material.id);
				crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
				}
			})
			.collect::<Vec<_>>();

		Ok(crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		})
	}

	/// Maps generated mesh geometry to render-facing metadata using the default generated material.
	fn convert_generated_mesh_data(
		&mut self,
		mesh: GpuMeshData,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		let material_index = self.request_material("white_solid.bema");
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
				},
			)
			.collect();

		Ok(crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		})
	}
}

/// The `VisibilityPipelineResourceManagerClient` struct connects render logic to the asynchronous visibility resource worker.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	pub(super) gpu_vertex_data_manager: GPUVertexDataManager,
	commands: Sender<VisibilityTransferCommand>,
	completions: Receiver<VisibilityResourceCompletion>,
}

/// The `VisibilityPipelineResourceManagerWorker` struct owns visibility resource loading on the transfer thread.
pub(crate) struct VisibilityPipelineResourceManagerWorker {
	resource_manager: VisibilityPipelineResourceManager,
	commands: Receiver<VisibilityTransferCommand>,
	completions: Sender<VisibilityResourceCompletion>,
	pending_mesh_uploads: VecDeque<(VisibilityMeshKey, MeshSource)>,
	pending_texture_uploads: VecDeque<(ghi::BaseImageHandle, TextureUpload)>,
	submitted_uploads: VecDeque<SubmittedUploadBatch>,
}

impl VisibilityPipelineResourceManagerClient {
	/// Sends a command to the transfer-thread visibility resource worker.
	pub(crate) fn send(&self, command: VisibilityTransferCommand) {
		if self.commands.send(command).is_err() {
			log::error!(
				"Visibility resource request failed. The most likely cause is that the resource worker thread terminated."
			);
		}
	}

	/// Requests a mesh resource from the transfer-thread worker.
	pub(crate) fn request_mesh(&self, key: VisibilityMeshKey, source: MeshSource) {
		self.send(VisibilityTransferCommand::RequestMesh { key, source });
	}

	/// Configures material pipeline creation on the transfer-thread worker.
	pub(crate) fn configure_material_pipeline(&self, config: MaterialPipelineConfig) {
		self.send(VisibilityTransferCommand::ConfigureMaterialPipeline(config));
	}

	/// Drains completed resource work without blocking the render thread.
	pub(crate) fn drain_completions(&mut self) -> Vec<VisibilityResourceCompletion> {
		let mut completions = Vec::new();
		while let Ok(completion) = self.completions.try_recv() {
			completions.push(completion);
		}
		completions
	}

	/// Enqueues texture upload bytes for the transfer queue.
	pub(crate) fn enqueue_texture_upload(&self, image: ghi::BaseImageHandle, upload: TextureUpload) {
		self.send(VisibilityTransferCommand::EnqueueTextureUpload { image, upload });
	}
}

impl VisibilityPipelineResourceManagerWorker {
	/// Publishes upload completions for transfer frames reported as complete by the queue.
	pub(crate) fn complete_frame(&mut self, completed_frame: ghi::FrameKey) {
		while self
			.submitted_uploads
			.front()
			.is_some_and(|batch| batch.frame_key == completed_frame)
		{
			let Some(batch) = self.submitted_uploads.pop_front() else {
				break;
			};

			for completion in batch.completions {
				if self.completions.send(completion).is_err() {
					log::error!(
						"Visibility upload completion failed. The most likely cause is that the render thread stopped receiving worker results."
					);
				}
			}
		}
	}

	/// Tracks resources handled by a submitted transfer frame.
	pub(crate) fn track_submitted_uploads(&mut self, frame_key: ghi::FrameKey, completions: Vec<VisibilityResourceCompletion>) {
		if completions.is_empty() {
			return;
		}
		self.submitted_uploads
			.push_back(SubmittedUploadBatch { frame_key, completions });
	}

	/// Records pending mesh and texture uploads into the transfer command buffer.
	pub(crate) fn prepare_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> TransferUploadPrepareResult {
		self.drain_commands();
		self.resource_manager
			.drain_pipeline_completions(MAX_PIPELINE_ADOPTIONS_PER_FRAME);
		self.record_uploads(transfer, staging_data_buffer, slice)
	}

	/// Drains render-thread commands into worker-owned state.
	fn drain_commands(&mut self) -> bool {
		let mut should_stop = false;
		while let Ok(command) = self.commands.try_recv() {
			match command {
				VisibilityTransferCommand::RequestMesh { key, source } => {
					self.pending_mesh_uploads.push_back((key.clone(), source.clone()));
					self.resource_manager
						.handle_request(VisibilityResourceRequest::Mesh { key, source });
				}
				VisibilityTransferCommand::EnqueueTextureUpload { image, upload } => {
					self.pending_texture_uploads.push_back((image, upload));
				}
				VisibilityTransferCommand::ConfigureMaterialPipeline(config) => {
					self.resource_manager.configure_material_pipeline(config);
				}
				VisibilityTransferCommand::Shutdown => should_stop = true,
			}
		}

		should_stop
	}

	/// Records queued upload work into the transfer command buffer.
	fn record_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> TransferUploadPrepareResult {
		let mut recorded_work = false;
		let mut completions = Vec::new();
		const TEXTURE_UPLOAD_ALIGNMENT: usize = 256;

		while let Some((key, source)) = self.pending_mesh_uploads.pop_front() {
			let result = self
				.resource_manager
				.load_mesh_source_for_transfer(transfer, staging_data_buffer, slice, &source);
			match result {
				Ok(mesh) => {
					completions.push(VisibilityResourceCompletion::MeshReady { key, mesh });
					recorded_work = true;
				}
				Err(()) => {
					let _ = self
						.completions
						.send(VisibilityResourceCompletion::Failed { key: key.into() });
				}
			}
		}

		while let Some((image, upload)) = self.pending_texture_uploads.pop_front() {
			if upload.data.len() > slice.remaining_aligned(TEXTURE_UPLOAD_ALIGNMENT) {
				self.pending_texture_uploads.push_front((image, upload));
				break;
			}

			let (source_offset, source_buffer) = slice.take_with_offset_aligned(upload.data.len(), TEXTURE_UPLOAD_ALIGNMENT);
			source_buffer.copy_from_slice(&upload.data);
			transfer.copy_buffer_to_images(&[ghi::BufferImageCopyDescriptor::new(
				staging_data_buffer,
				source_offset,
				upload.source_bytes_per_row,
				upload.source_bytes_per_image,
				image,
			)]);
			recorded_work = true;
		}

		TransferUploadPrepareResult {
			recorded_work,
			completions,
		}
	}
}

/// The `TransferUploadPrepareResult` struct tracks transfer work and resources handled by a recording.
pub(crate) struct TransferUploadPrepareResult {
	pub(crate) recorded_work: bool,
	pub(crate) completions: Vec<VisibilityResourceCompletion>,
}

/// The `SubmittedUploadBatch` struct holds resource completions until a transfer frame is complete.
struct SubmittedUploadBatch {
	frame_key: ghi::FrameKey,
	completions: Vec<VisibilityResourceCompletion>,
}

#[derive(PartialEq, Eq)]
enum ResourceWorkerFlow {
	Continue,
	Stop,
}

/// The `VisibilityResourceRequest` enum describes work the render thread delegates to the resource worker.
pub(crate) enum VisibilityResourceRequest {
	Mesh { key: VisibilityMeshKey, source: MeshSource },
	Material { id: String },
	Image { key: VisibilityTextureKey },
	Shutdown,
}

/// The `VisibilityResourceCompletion` enum describes resource work that is ready for render-thread adoption.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		key: VisibilityMeshKey,
		mesh: crate::rendering::pipelines::visibility::pipeline_manager::MeshData,
	},
	PipelineReady {
		name: String,
		pipeline: ghi::implementation::ComputePipeline,
	},
	MaterialReady {
		id: String,
		index: u32,
		pipeline: Option<ghi::PipelineHandle>,
		alpha: bool,
		textures: Vec<Option<(String, u32)>>,
	},
	ImageReady {
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::implementation::FactoryImage,
		sampler: ghi::implementation::FactorySampler,
		upload: TextureUpload,
	},
	Failed {
		key: VisibilityResourceKey,
	},
}

/// The `VisibilityTransferCommand` enum describes commands sent from rendering to the transfer worker.
pub(crate) enum VisibilityTransferCommand {
	RequestMesh {
		key: VisibilityMeshKey,
		source: MeshSource,
	},
	EnqueueTextureUpload {
		image: ghi::BaseImageHandle,
		upload: TextureUpload,
	},
	ConfigureMaterialPipeline(MaterialPipelineConfig),
	Shutdown,
}

/// The `VisibilityResourceKey` enum identifies a visibility resource independently of scene instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityResourceKey {
	Mesh(VisibilityMeshKey),
	Texture(VisibilityTextureKey),
	Material(String),
}

/// The `VisibilityMeshKey` struct identifies a mesh resource or generated mesh across scene instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VisibilityMeshKey(String);

impl VisibilityMeshKey {
	/// Builds a stable mesh key from a mesh source.
	pub(crate) fn from_source(source: &MeshSource) -> Self {
		match source {
			MeshSource::Resource(id) => Self(format!("resource:{id}")),
			MeshSource::Generated(generator) => Self(format!("generated:{}", generator.hash())),
		}
	}
}

impl std::fmt::Display for VisibilityMeshKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<VisibilityMeshKey> for VisibilityResourceKey {
	fn from(value: VisibilityMeshKey) -> Self {
		Self::Mesh(value)
	}
}

/// The `VisibilityTextureKey` struct identifies a material texture resource across materials and instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VisibilityTextureKey(String);

impl VisibilityTextureKey {
	/// Creates a texture key from a resource id.
	pub(crate) fn new(id: impl Into<String>) -> Self {
		Self(id.into())
	}

	/// Returns the resource id backing this texture key.
	pub(crate) fn as_str(&self) -> &str {
		self.0.as_str()
	}
}

impl std::fmt::Display for VisibilityTextureKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		self.0.fmt(f)
	}
}

impl From<VisibilityTextureKey> for VisibilityResourceKey {
	fn from(value: VisibilityTextureKey) -> Self {
		Self::Texture(value)
	}
}

impl std::fmt::Display for VisibilityResourceKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			VisibilityResourceKey::Mesh(key) => key.fmt(f),
			VisibilityResourceKey::Texture(key) => key.fmt(f),
			VisibilityResourceKey::Material(key) => key.fmt(f),
		}
	}
}

/// The `FactoryTexture` struct packages detached texture resources with upload bytes for render-thread adoption.
struct FactoryTexture {
	index: u32,
	image: ghi::implementation::FactoryImage,
	sampler: ghi::implementation::FactorySampler,
	upload: TextureUpload,
}

/// The `FactoryMaterial` struct packages material metadata with pending render-thread pipeline state.
struct FactoryMaterial {
	index: u32,
	pipeline: Option<ghi::PipelineHandle>,
	alpha: bool,
	textures: Vec<Option<(String, u32)>>,
}

/// The `MaterialPipelineConfig` struct names the descriptor and push-constant contract for material evaluation pipelines.
pub(crate) struct MaterialPipelineConfig {
	descriptor_set_templates: [ghi::DescriptorSetTemplateHandle; 3],
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
}

impl MaterialPipelineConfig {
	/// Creates a material pipeline configuration from the visibility descriptor layouts.
	pub(crate) fn new(
		descriptor_set_templates: [ghi::DescriptorSetTemplateHandle; 3],
		push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	) -> Self {
		Self {
			descriptor_set_templates,
			push_constant_ranges,
		}
	}
}

/// The `TextureUpload` struct carries row-padded texture bytes until the transfer queue copies them.
pub(crate) struct TextureUpload {
	pub(crate) data: Vec<u8>,
	pub(crate) source_bytes_per_row: usize,
	pub(crate) source_bytes_per_image: usize,
}

enum PipelineStatus {
	Pending,
	Ready(ghi::PipelineHandle),
	Failed,
}

enum OwnedShaderSource {
	MTLB {
		binary: ResourceReaderBacking,
		entry_point: String,
	},
	MTL {
		source: String,
		entry_point: String,
	},
	SPIRV(ResourceReaderBacking),
}

impl OwnedShaderSource {
	fn sources(&self) -> ghi::shader::Sources<'_> {
		match self {
			OwnedShaderSource::MTLB { binary, entry_point } => ghi::shader::Sources::MTLB {
				binary: binary.as_slice(),
				entry_point,
			},
			OwnedShaderSource::MTL { source, entry_point } => ghi::shader::Sources::MTL { source, entry_point },
			OwnedShaderSource::SPIRV(binary) => ghi::shader::Sources::SPIRV(binary.as_slice()),
		}
	}
}

/// The `OwnedShader` struct keeps shader payloads reusable across synchronous and worker-thread pipeline creation.
struct OwnedShader {
	name: Option<String>,
	source: OwnedShaderSource,
	stage: ghi::ShaderTypes,
	binding_descriptors: Vec<ghi::shader::BindingDescriptor>,
}

/// The `ComputePipelineRequest` struct packages the resource data needed to compile a material compute pipeline off-thread.
struct ComputePipelineRequest {
	key: String,
	descriptor_set_templates: Vec<ghi::DescriptorSetTemplateHandle>,
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	shader: Arc<OwnedShader>,
	specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
}

enum ComputePipelineResult {
	Ready {
		key: String,
		pipeline: ghi::implementation::ComputePipeline,
	},
	Failed {
		key: String,
	},
}

#[cfg(debug_assertions)]
const DEBUG_PIPELINE_CREATION_DELAY: Duration = Duration::from_millis(250);

impl VisibilityPipelineResourceManager {
	fn spawn_compute_worker(
		factory: ghi::implementation::Factory,
	) -> (Sender<ComputePipelineRequest>, Receiver<ComputePipelineResult>) {
		let (request_sender, request_receiver) = mpsc::channel::<ComputePipelineRequest>();
		let (result_sender, result_receiver) = mpsc::channel::<ComputePipelineResult>();

		thread::spawn(move || {
			let mut factory = factory;

			while let Ok(request) = request_receiver.recv() {
				let key = request.key.clone();
				let result = catch_unwind(AssertUnwindSafe(|| Self::compile_compute_pipeline(&mut factory, request)));

				let message = match result {
					Ok(Ok(pipeline)) => ComputePipelineResult::Ready { key, pipeline },
					Ok(Err(())) | Err(_) => ComputePipelineResult::Failed { key },
				};

				if result_sender.send(message).is_err() {
					break;
				}
			}
		});

		(request_sender, result_receiver)
	}

	fn compile_compute_pipeline(
		factory: &mut ghi::implementation::Factory,
		request: ComputePipelineRequest,
	) -> Result<ghi::implementation::ComputePipeline, ()> {
		use ghi::factory::Factory as _;

		Self::sleep_for_debug_pipeline_delay();

		let shader = request.shader;
		let shader_handle = factory.create_shader(
			shader.name.as_deref(),
			shader.source.sources(),
			shader.stage,
			shader.binding_descriptors.iter().copied(),
		)?;

		Ok(factory.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&request.descriptor_set_templates,
			&request.push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, shader.stage)
				.with_specialization_map(&request.specialization_map_entries),
		)))
	}

	fn queue_compute_pipeline(&self, request: ComputePipelineRequest) {
		let key = request.key.clone();

		let Some(compute_pipeline_requests) = self.compute_pipeline_requests.as_ref() else {
			self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
			log::error!(
				"Async pipeline requests are unavailable for {}. The most likely cause is that the active backend does not expose a pipeline factory.",
				key
			);
			return;
		};

		if compute_pipeline_requests.send(request).is_err() {
			self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
			log::error!(
				"Async pipeline request channel closed for {}. The most likely cause is that the compilation worker terminated unexpectedly.",
				key
			);
		}
	}

	fn sleep_for_debug_pipeline_delay() {
		#[cfg(debug_assertions)]
		thread::sleep(DEBUG_PIPELINE_CREATION_DELAY);
	}

	pub(crate) fn poll_pipelines(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		max_results: usize,
	) -> Vec<(String, ghi::PipelineHandle)> {
		let Some(compute_pipeline_results) = self.compute_pipeline_results.as_ref() else {
			return Vec::new();
		};

		let mut resolved_pipelines = Vec::with_capacity(max_results.min(16));

		while resolved_pipelines.len() < max_results {
			let Ok(result) = compute_pipeline_results.try_recv() else {
				break;
			};

			match result {
				ComputePipelineResult::Ready { key, pipeline } => {
					let handle = frame.intern_compute_pipeline(pipeline);
					self.pipelines.write().insert(key.clone(), PipelineStatus::Ready(handle));
					resolved_pipelines.push((key, handle));
				}
				ComputePipelineResult::Failed { key } => {
					self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
					log::error!(
						"Async pipeline compilation failed for {}. The most likely cause is that shader creation or pipeline specialization failed on the compilation thread.",
						key
					);
				}
			}
		}

		resolved_pipelines
	}

	pub(crate) fn drain_pipeline_completions(&mut self, max_results: usize) {
		let Some(compute_pipeline_results) = self.compute_pipeline_results.as_ref() else {
			return;
		};

		let mut result_count = 0;
		while result_count < max_results {
			let Ok(result) = compute_pipeline_results.try_recv() else {
				break;
			};

			match result {
				ComputePipelineResult::Ready { key, pipeline } => {
					self.pipelines.write().insert(key.clone(), PipelineStatus::Pending);
					if self
						.work_completions
						.send(VisibilityResourceCompletion::PipelineReady { name: key, pipeline })
						.is_err()
					{
						log::error!(
							"Visibility pipeline completion failed. The most likely cause is that the render thread stopped receiving worker results."
						);
					}
				}
				ComputePipelineResult::Failed { key } => {
					self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
					log::error!(
						"Async pipeline compilation failed for {}. The most likely cause is that shader creation or pipeline specialization failed on the compilation thread.",
						key
					);
				}
			}

			result_count += 1;
		}
	}

	fn map_shader_type(stage: ShaderTypes) -> ghi::ShaderTypes {
		match stage {
			ShaderTypes::AnyHit => ghi::ShaderTypes::AnyHit,
			ShaderTypes::ClosestHit => ghi::ShaderTypes::ClosestHit,
			ShaderTypes::Compute => ghi::ShaderTypes::Compute,
			ShaderTypes::Fragment => ghi::ShaderTypes::Fragment,
			ShaderTypes::Intersection => ghi::ShaderTypes::Intersection,
			ShaderTypes::Mesh => ghi::ShaderTypes::Mesh,
			ShaderTypes::Miss => ghi::ShaderTypes::Miss,
			ShaderTypes::RayGen => ghi::ShaderTypes::RayGen,
			ShaderTypes::Callable => ghi::ShaderTypes::Callable,
			ShaderTypes::Task => ghi::ShaderTypes::Task,
			ShaderTypes::Vertex => ghi::ShaderTypes::Vertex,
		}
	}

	/// Loads shader backing once so sync and async pipeline creation can reuse the same payload.
	fn load_cached_shader_request(&self, shader: &mut Reference<Shader>) -> Result<Arc<OwnedShader>, ()> {
		if let StaleEntry::Fresh(shader_request) = self.shader_requests.read().entry(&shader.id, shader.get_hash()) {
			return Ok(Arc::clone(shader_request));
		}

		let binding_descriptors = shader
			.resource()
			.interface
			.bindings
			.iter()
			.map(|binding| {
				ghi::shader::BindingDescriptor::new(
					binding.set,
					binding.binding,
					if binding.read {
						ghi::AccessPolicies::READ
					} else {
						ghi::AccessPolicies::empty()
					} | if binding.write {
						ghi::AccessPolicies::WRITE
					} else {
						ghi::AccessPolicies::empty()
					},
				)
			})
			.collect::<Vec<_>>();

		let stage = Self::map_shader_type(shader.resource().stage);
		let shader_backing = Self::load_shader_backing(shader)?;

		let shader_request = Arc::new(OwnedShader {
			name: Some(shader.id().to_string()),
			source: if ghi::implementation::USES_METAL {
				OwnedShaderSource::MTLB {
					binary: shader_backing,
					entry_point: "besl_main".to_string(),
				}
			} else {
				OwnedShaderSource::SPIRV(shader_backing)
			},
			stage,
			binding_descriptors,
		});

		self.shader_requests
			.write()
			.insert(shader.id().to_string(), shader.get_hash(), Arc::clone(&shader_request));

		Ok(shader_request)
	}

	/// Loads shader bytes from reader backing storage and falls back to an owned buffer when direct backing is unavailable.
	fn load_shader_backing(shader: &mut Reference<Shader>) -> Result<ResourceReaderBacking, ()> {
		match shader.consume_reader().into_backing_storage() {
			Ok(backing) => Ok(backing),
			Err(mut reader) => {
				let read_target = ReadTargetsMut::create_buffer(shader);
				let load_request = reader.read_into(None, read_target).map_err(|_| {
					log::error!(
						"Failed to load shader bytes for {}. The most likely cause is that the shader resource no longer has an available read target.",
						shader.id(),
					);
				})?;

				match load_request {
					ReadTargets::Box(buffer) => Ok(ResourceReaderBacking::Buffer(buffer)),
					ReadTargets::Buffer(buffer) => Ok(ResourceReaderBacking::Buffer(buffer.into())),
					ReadTargets::Backing(backing) => Ok(backing),
					ReadTargets::Streams(_) => {
						log::error!(
							"Shader {} produced stream-backed data. The most likely cause is that the shader resource was loaded with an unexpected read target.",
							shader.id(),
						);
						Err(())
					}
				}
			}
		}
	}

	fn queue_material_pipeline(
		&self,
		resource_id: String,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		material: &mut ResourceMaterial,
	) -> Option<ghi::PipelineHandle> {
		self.queue_material_pipeline_with_specialization(
			resource_id,
			descriptor_set_template_handles,
			push_constant_ranges,
			material,
			Vec::new(),
		)
	}

	/// Queues a material pipeline request with variant specialization constants.
	fn queue_material_pipeline_with_specialization(
		&self,
		resource_id: String,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		material: &mut ResourceMaterial,
		specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
	) -> Option<ghi::PipelineHandle> {
		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.pipelines.write().insert(resource_id.clone(), PipelineStatus::Pending);

		let request = match material.shaders_mut().iter_mut().next() {
			Some(shader) => self.load_cached_shader_request(shader).map(|shader| ComputePipelineRequest {
				key: resource_id.clone(),
				descriptor_set_templates: descriptor_set_template_handles.to_vec(),
				push_constant_ranges: push_constant_ranges.to_vec(),
				shader,
				specialization_map_entries,
			}),
			None => Err(()),
		};

		match request {
			Ok(request) => self.queue_compute_pipeline(request),
			Err(()) => {
				self.pipelines.write().insert(resource_id, PipelineStatus::Failed);
			}
		}

		None
	}
}

/// Builds row-padded upload data compatible with the transfer command buffer image copy path.
fn make_texture_upload(format: ghi::Formats, extent: Extent, source: &[u8]) -> Option<TextureUpload> {
	let (source_bytes_per_row, row_count, compact_bytes_per_image) = texture_upload_layout(format, extent)?;
	if source.len() < compact_bytes_per_image {
		return None;
	}
	assert_eq!(
		source.len(),
		compact_bytes_per_image,
		"Texture upload source size mismatch. The most likely cause is that the baked texture payload does not match the runtime texture layout. format={format:?}, extent={extent:?}, source_len={}, source_bytes_per_row={source_bytes_per_row}, row_count={row_count}, expected={compact_bytes_per_image}",
		source.len()
	);

	let padded_bytes_per_row = source_bytes_per_row.next_multiple_of(256);
	let source_bytes_per_image = padded_bytes_per_row * row_count;
	assert_eq!(
		padded_bytes_per_row % 256,
		0,
		"Texture upload row pitch alignment mismatch. The most likely cause is that the Metal upload layout was built without 256-byte row alignment. format={format:?}, extent={extent:?}, source_bytes_per_row={source_bytes_per_row}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	assert!(
		source_bytes_per_image >= compact_bytes_per_image,
		"Texture upload padded image is smaller than compact image. The most likely cause is an invalid row count or row pitch. format={format:?}, extent={extent:?}, compact_bytes_per_image={compact_bytes_per_image}, source_bytes_per_image={source_bytes_per_image}, row_count={row_count}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	let mut data = vec![0u8; source_bytes_per_image];

	for row in 0..row_count {
		let source_offset = row * source_bytes_per_row;
		let destination_offset = row * padded_bytes_per_row;
		let source_end = source_offset + source_bytes_per_row;
		let destination_end = destination_offset + source_bytes_per_row;
		assert!(
			source_end <= source.len(),
			"Texture upload source row is out of bounds. The most likely cause is a bad compact row pitch for this format. format={format:?}, extent={extent:?}, row={row}, row_count={row_count}, source_offset={source_offset}, source_end={source_end}, source_len={}, source_bytes_per_row={source_bytes_per_row}",
			source.len()
		);
		assert!(
			destination_end <= data.len(),
			"Texture upload padded row is out of bounds. The most likely cause is a bad padded row pitch for this format. format={format:?}, extent={extent:?}, row={row}, row_count={row_count}, destination_offset={destination_offset}, destination_end={destination_end}, data_len={}, padded_bytes_per_row={padded_bytes_per_row}",
			data.len()
		);
		let source_row = &source[source_offset..source_end];
		data[destination_offset..destination_end].copy_from_slice(source_row);
	}
	assert_eq!(
		data.len(),
		source_bytes_per_image,
		"Texture upload output size mismatch. The most likely cause is that the padded upload allocation changed during row copy. format={format:?}, extent={extent:?}, data_len={}, expected={source_bytes_per_image}",
		data.len()
	);

	Some(TextureUpload {
		data,
		source_bytes_per_row: padded_bytes_per_row,
		source_bytes_per_image,
	})
}

/// Converts a resource-management image format into the matching GHI image format.
fn resource_image_format_to_ghi(format: resource_management::types::Formats) -> ghi::Formats {
	match format {
		resource_management::types::Formats::RG8 => ghi::Formats::RG8UNORM,
		resource_management::types::Formats::RGB8 => ghi::Formats::RGB8UNORM,
		resource_management::types::Formats::RGB16 => ghi::Formats::RGB16UNORM,
		resource_management::types::Formats::RGBA8 => ghi::Formats::RGBA8UNORM,
		resource_management::types::Formats::RGBA16 => ghi::Formats::RGBA16UNORM,
		resource_management::types::Formats::BC5 => ghi::Formats::BC5,
		resource_management::types::Formats::BC7 => ghi::Formats::BC7,
		resource_management::types::Formats::BC7SRGB => ghi::Formats::BC7SRGB,
	}
}

/// Builds the default sampler used by visibility material textures.
pub(crate) fn default_material_sampler_builder() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.filtering_mode(ghi::FilteringModes::Linear)
		.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
		.mip_map_mode(ghi::FilteringModes::Linear)
		.addressing_mode(ghi::SamplerAddressingModes::Repeat)
		.min_lod(0f32)
		.max_lod(0f32)
}

/// Computes the compact source layout for one mip of the given texture format.
fn texture_upload_layout(format: ghi::Formats, extent: Extent) -> Option<(usize, usize, usize)> {
	let width = extent.width().max(1) as usize;
	let height = extent.height().max(1) as usize;

	match format {
		ghi::Formats::BC5 | ghi::Formats::BC7 | ghi::Formats::BC7SRGB => {
			let layout = format.bc_layout(width as u32, height as u32)?;
			Some((
				layout.bytes_per_row as usize,
				layout.blocks_h as usize,
				layout.bytes_per_image as usize,
			))
		}
		_ => {
			let bytes_per_row = width * format.size();
			Some((bytes_per_row, height, bytes_per_row * height))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bc_texture_upload_pads_between_block_rows_without_changing_row_contents() {
		let extent = Extent::rectangle(5, 7);
		let compact_row = 2 * 16;
		let source = (0..(compact_row * 2)).map(|value| value as u8).collect::<Vec<_>>();

		let upload = make_texture_upload(ghi::Formats::BC7, extent, &source).unwrap();

		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 256 * 2);
		assert_eq!(&upload.data[0..compact_row], &source[0..compact_row]);
		assert!(upload.data[compact_row..256].iter().all(|byte| *byte == 0));
		assert_eq!(&upload.data[256..256 + compact_row], &source[compact_row..compact_row * 2]);
	}
}

pub enum ResourceStates<P, L> {
	/// The resource is pending handling.
	Pending(P),
	/// The resource is currently being loaded.
	Loading(ghi::FrameKey, L),
	/// The resource has been loaded successfully and is available for use.
	Loaded(L),
	/// The resource failed to load and should not be retried.
	Failed,
}

impl<P, L> ResourceStates<P, L> {
	pub fn pending(v: P) -> Self {
		ResourceStates::Pending(v)
	}

	pub fn is_ready(&self) -> bool {
		match self {
			ResourceStates::Loaded(_) => true,
			_ => false,
		}
	}

	pub fn is_pending(&self) -> bool {
		matches!(self, ResourceStates::Pending(_))
	}

	pub fn is_failed(&self) -> bool {
		matches!(self, ResourceStates::Failed)
	}

	pub fn get(&self) -> &L {
		match self {
			ResourceStates::Loading(_, v) => v,
			ResourceStates::Loaded(v) => v,
			_ => panic!(),
		}
	}

	pub fn get_mut(&mut self) -> &mut L {
		match self {
			ResourceStates::Loading(_, v) => v,
			ResourceStates::Loaded(v) => v,
			_ => panic!(),
		}
	}

	pub fn frame_finished(self, frame_key: ghi::FrameKey) -> Self {
		match self {
			ResourceStates::Loading(loading_frame_key, v) => {
				if loading_frame_key == frame_key {
					ResourceStates::Loaded(v)
				} else {
					ResourceStates::Loading(loading_frame_key, v)
				}
			}
			_ => self,
		}
	}
}

const MAX_PIPELINE_ADOPTIONS_PER_FRAME: usize = 8;

use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ghi::context::Context as _;
use ghi::factory::Factory as _;
use ghi::frame::Frame as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	Size as _,
};
use math::Vector3;
use resource_management::resource::reader::ResourceReaderBacking;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resource::{ReadTargets, ReadTargetsMut};
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::{Material as ResourceMaterial, Shader, Value, Variant as ResourceVariant};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use resource_management::types::ShaderTypes;
use resource_management::Reference;
use utils::hash::{HashMap, HashMapExt};
use utils::stale_map::{Entry as StaleEntry, StaleHashMap};
use utils::sync::RwLock;
use utils::Extent;

use crate::core::EntityHandle;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{GPUVertexDataManager, MeshData as GpuMeshData};
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::rendering::renderable::mesh::MeshSource;
use crate::resource_management::{self};
