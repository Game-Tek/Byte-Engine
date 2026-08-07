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
	/// Resource manager for loading assets.
	resource_manager: EntityHandle<ResourceManager>,
	pipelines: RwLock<HashMap<String, PipelineStatus>>,
	// Async requests cannot reload shader bytes after a sync load consumes the read target,
	// so we keep an owned backing for the shader payload keyed by resource hash.
	shader_requests: RwLock<StaleHashMap<String, u64, Arc<OwnedShader>>>,
	factory: Option<ghi::implementation::Factory>,
	material_pipeline_config: Option<MaterialPipelineConfig>,
	work_completions: Sender<VisibilityResourceCompletion>,
}

pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;
pub(crate) const ASYNC_UPLOAD_BUFFER_BYTE_COUNT: usize = 1024 * 1024 * 32;
const ACTIVE_TRANSFER_POLL_INTERVAL: Duration = Duration::from_millis(1);

type CompletionList = SmallVec<[VisibilityResourceCompletion; 16]>;

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
		let (commands, command_receiver) = kanal::unbounded_async();
		let (work_completions, work_completion_receiver) = mpsc::channel();
		let resource_manager = Self::new(resource_manager, work_completions.clone());

		(
			VisibilityPipelineResourceManagerClient {
				gpu_vertex_data_manager,
				commands: commands.to_sync(),
				completions: work_completion_receiver,
			},
			VisibilityPipelineResourceManagerWorker {
				resource_manager,
				gpu_vertex_data_manager: mesh_data_manager,
				commands: command_receiver,
				completions: work_completions,
				pending_mesh_uploads: VecDeque::new(),
				pending_texture_uploads: VecDeque::new(),
				pending_environment_uploads: VecDeque::new(),
				submitted_uploads: VecDeque::new(),
			},
		)
	}

	fn new(resource_manager: EntityHandle<ResourceManager>, work_completions: Sender<VisibilityResourceCompletion>) -> Self {
		Self {
			images: Vec::with_capacity(4096),
			images_by_resource: HashMap::with_capacity(4096),
			materials: Vec::with_capacity(4096),
			material_by_name: HashMap::with_capacity(4096),
			resource_manager,
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shader_requests: RwLock::new(StaleHashMap::with_capacity(1024)),
			factory: None,
			material_pipeline_config: None,
			work_completions,
		}
	}

	/// Stores the descriptor layout data needed to compile material evaluation pipelines.
	pub(crate) fn configure_material_pipeline(&mut self, mut config: MaterialPipelineConfig) {
		self.factory = config.pipeline_factory.take();
		self.material_pipeline_config = Some(config);
	}

	/// Resolves a mesh and its material slots before borrowing GPU transfer memory.
	async fn prepare_mesh_source(&mut self, source: MeshSource) -> Result<PreparedMeshSource, ()> {
		match source {
			MeshSource::Resource(id) => {
				let resource: Reference<ResourceMesh> = match self.resource_manager.request(id).await {
					Ok(resource) => resource,
					Err(_) => {
						log::error!(
							"Visibility mesh resource request failed for {}. The most likely cause is that the mesh id is missing or the asset database is not loaded.",
							id
						);
						return Err(());
					}
				};

				let primitive_count = resource.resource().primitives.len();
				for primitive_index in 0..primitive_count {
					// Own only the ID that crosses this await; the mesh reference
					// remains intact for its later borrowed staging load.
					let material_id = resource.resource().primitives[primitive_index].material.id.clone();
					self.request_material(&material_id).await;
				}

				Ok(PreparedMeshSource::Resource { resource })
			}
			MeshSource::Generated(generator) => Ok(PreparedMeshSource::Generated {
				generator,
				material_index: self.request_material("white_solid.bema").await,
			}),
		}
	}

	/// Loads a material variant resource, reserves its texture dependencies, and queues its material evaluation pipeline.
	async fn handle_material_request(&mut self, id: String) {
		let index = self.reserve_material_slot(&id).0;
		let result = self.load_variant_metadata(&id, index).await;
		let completion = match result {
			Ok(material) => VisibilityResourceCompletion::MaterialReady {
				id,
				index,
				pipeline: material.pipeline,
				pending_pipeline: material.pending_pipeline,
				alpha_mode: material.alpha_mode,
				textures: material.textures,
			},
			Err(()) => VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Material(id),
			},
		};

		self.send_completion(completion);
	}

	/// Loads one texture resource and reports render-thread creation data.
	async fn handle_image_request(&mut self, key: VisibilityTextureKey) {
		let index = self.reserve_texture_slot(key.as_str()).0;
		let result = self.load_texture_with_factory(key.as_str(), index).await;
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

		self.send_completion(completion);
	}

	/// Loads all baked IBL streams from one parent image without consuming material texture slots.
	async fn handle_environment_request(&mut self, id: String) {
		let completion = match self.load_environment_with_factory(&id).await {
			Ok(environment) => VisibilityResourceCompletion::EnvironmentReady { id, environment },
			Err(()) => VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Environment(id),
			},
		};

		self.send_completion(completion);
	}

	/// Sends one loading result without blocking the resource task.
	fn send_completion(&self, completion: VisibilityResourceCompletion) {
		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility resource completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Reads material variant metadata while scheduling texture and pipeline dependencies.
	async fn load_variant_metadata(&mut self, id: &str, index: u32) -> Result<FactoryMaterial, ()> {
		let mut reference: Reference<ResourceVariant> =
			self.resource_manager.request(id).await.map_err(|_| {
				log::error!(
					"Visibility material variant request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
					id
				);
			})?;

		let variant = reference.resource_mut();
		let alpha_mode = variant.alpha_mode.clone();
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

		let texture_keys = variant
			.variables
			.iter()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => {
					let key = VisibilityTextureKey::new(image.id());
					Some(key)
				}
				_ => None,
			})
			.collect::<Vec<_>>();
		let queued_pipeline = self
			.queue_configured_variant_pipeline(id.to_string(), material, specialization_map_entries)
			.await;
		let mut textures = Vec::with_capacity(texture_keys.len());
		for key in texture_keys {
			let texture = match key {
				Some(key) => {
					let texture_index = self.request_texture_dependency(key.clone()).await;
					Some((key.as_str().to_string(), texture_index))
				}
				None => None,
			};
			textures.push(texture);
		}

		Ok(FactoryMaterial {
			index,
			pipeline: queued_pipeline.pipeline,
			pending_pipeline: queued_pipeline.pending_pipeline,
			alpha_mode,
			textures,
		})
	}

	/// Queues a texture dependency discovered while loading another resource.
	async fn request_texture_dependency(&mut self, key: VisibilityTextureKey) -> u32 {
		let (index, inserted) = self.reserve_texture_slot(key.as_str());
		if inserted {
			self.handle_image_request(key).await;
		}
		index
	}

	/// Queues a material evaluation pipeline with the descriptor configuration supplied by the render thread.
	async fn queue_configured_material_pipeline(
		&mut self,
		id: String,
		material: &mut ResourceMaterial,
	) -> QueuedMaterialPipeline {
		let Some(config) = self.material_pipeline_config.as_ref() else {
			log::error!(
				"Visibility material pipeline configuration is unavailable for {}. The most likely cause is that the render pipeline manager has not configured the resource worker yet.",
				id
			);
			return QueuedMaterialPipeline::default();
		};
		let push_constant_ranges = config.push_constant_ranges.clone();

		self.queue_material_pipeline(id, &push_constant_ranges, material).await
	}

	/// Queues a material variant pipeline with the descriptor configuration supplied by the render thread.
	async fn queue_configured_variant_pipeline(
		&mut self,
		id: String,
		material: &mut ResourceMaterial,
		specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
	) -> QueuedMaterialPipeline {
		let Some(config) = self.material_pipeline_config.as_ref() else {
			log::error!(
				"Visibility material pipeline configuration is unavailable for {}. The most likely cause is that the render pipeline manager has not configured the resource worker yet.",
				id
			);
			return QueuedMaterialPipeline::default();
		};
		let push_constant_ranges = config.push_constant_ranges.clone();

		self.queue_material_pipeline_with_specialization(id, &push_constant_ranges, material, specialization_map_entries)
			.await
	}

	/// Loads texture bytes and builds detached GPU resources for render-thread adoption.
	async fn load_texture_with_factory(&mut self, id: &str, index: u32) -> Result<FactoryTexture, ()> {
		let mut reference: Reference<ResourceImage> =
			self.resource_manager.request(id).await.map_err(|_| {
				log::error!(
					"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
					id
			);
		})?;
		let texture = reference.resource();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);

		// Image resources may append mips or baked IBL streams after the base image; material textures upload only mip zero.
		let mut source = vec![0u8; compact_image_byte_size(format, extent)];
		let load_target = reference.load(source.as_mut_slice().into()).await.map_err(|_| {
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

		let device = self.factory.as_mut().ok_or_else(|| {
			log::error!(
				"Visibility texture creation failed for {}. The most likely cause is that material pipeline creation was configured without a factory.",
				id
			);
		})?;

		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);

		let sampler = device.build_sampler(default_material_sampler_builder());

		Ok(FactoryTexture {
			index,
			image,
			sampler,
			upload,
		})
	}

	/// Loads the diffuse and roughness-prefiltered streams, then creates one mipmapped specular image for adoption.
	async fn load_environment_with_factory(&mut self, id: &str) -> Result<FactoryEnvironment, ()> {
		let mut reference: Reference<ResourceImage> =
			self.resource_manager.request(id).await.map_err(|_| {
				log::error!(
					"Visibility environment request failed for {}. The most likely cause is that the image resource is missing or the asset database is not loaded.",
					id
			);
		})?;
		let ibl = reference.resource().ibl.clone().ok_or_else(|| {
			log::error!(
				"Visibility environment IBL data is missing for {}. The most likely cause is that the EXR was baked before IBL generation was enabled.",
				id
			);
		})?;

		if ibl.diffuse_irradiance.mip_count != 1
			|| ibl.prefiltered_specular.mip_count as usize != IBL_SPECULAR_LEVEL_COUNT
			|| ibl.diffuse_irradiance.gamma != resource_management::types::Gamma::Linear
			|| ibl.prefiltered_specular.gamma != resource_management::types::Gamma::Linear
			|| ibl.diffuse_irradiance.array_layers != 6
			|| ibl.prefiltered_specular.array_layers != 6
		{
			log::error!(
				"Visibility environment IBL metadata is unsupported for {}. The most likely cause is that the baked image does not contain one linear diffuse map and {} linear specular levels.",
				id,
				IBL_SPECULAR_LEVEL_COUNT
			);
			return Err(());
		}

		let diffuse_format = resource_image_format_to_ghi(ibl.diffuse_irradiance.format);
		let specular_format = resource_image_format_to_ghi(ibl.prefiltered_specular.format);
		let available_specular_mips = resource_management::resources::mips::mip_level_count(
			ibl.prefiltered_specular.extent[0],
			ibl.prefiltered_specular.extent[1],
		)
		.map_err(|_| {
			log::error!(
				"Visibility environment IBL dimensions are invalid for {}. The most likely cause is that the baked specular image has a zero dimension.",
				id
			);
		})?;
		if available_specular_mips < IBL_SPECULAR_LEVEL_COUNT as u32 {
			log::error!(
				"Visibility environment IBL mip chain is unsupported for {}. The most likely cause is that its base extent is too small for {} distinct mip levels.",
				id,
				IBL_SPECULAR_LEVEL_COUNT
			);
			return Err(());
		}
		let diffuse_extent = Extent::from(ibl.diffuse_irradiance.extent);
		let specular_extents: [Extent; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| environment_mip_extent(ibl.prefiltered_specular.extent, level as u32));
		if diffuse_extent.depth().max(1) != 1 || specular_extents.iter().any(|extent| extent.depth().max(1) != 1) {
			log::error!(
				"Visibility environment IBL extent is unsupported for {}. The most likely cause is that a baked IBL stream is not a two-dimensional lat-long image.",
				id
			);
			return Err(());
		}

		let mut diffuse_data = vec![0u8; compact_image_byte_size(diffuse_format, diffuse_extent) * 6];
		let mut specular_data: [Vec<u8>; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| vec![0u8; compact_image_byte_size(specular_format, specular_extents[level]) * 6]);
		let specular_stream_names: [String; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|level| {
			resource_management::resources::image::ibl_prefiltered_specular_stream_name(level as u32)
		});

		// A single stream read keeps the parent image and all of its baked lighting subresources atomic.
		let mut streams = Vec::with_capacity(1 + IBL_SPECULAR_LEVEL_COUNT);
		streams.push(resource_management::stream::StreamMut::new(
			resource_management::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
			diffuse_data.as_mut_slice(),
		));
		for (name, data) in specular_stream_names.iter().zip(specular_data.iter_mut()) {
			streams.push(resource_management::stream::StreamMut::new(name, data.as_mut_slice()));
		}
		let loaded = reference.load(streams.into()).await.map_err(|_| {
			log::error!(
				"Visibility environment IBL stream load failed for {}. The most likely cause is that the baked image payload is missing one or more named IBL streams.",
				id
			);
		})?;
		if !matches!(loaded, ReadTargets::Streams(_)) {
			log::error!(
				"Visibility environment IBL load returned an unexpected target for {}. The most likely cause is that the resource reader ignored the requested named streams.",
				id
			);
			return Err(());
		}
		drop(loaded);

		let diffuse_upload = make_layered_texture_upload(diffuse_format, diffuse_extent, 6, &diffuse_data).ok_or_else(|| {
			log::error!(
				"Visibility diffuse IBL upload preparation failed for {}. The most likely cause is that its stream size does not match its format and extent.",
				id
			);
		})?;
		let specular_uploads = specular_data
			.iter()
			.zip(specular_extents)
			.map(|(data, extent)| make_layered_texture_upload(specular_format, extent, 6, data))
			.collect::<Option<Vec<_>>>()
			.ok_or_else(|| {
				log::error!(
					"Visibility specular IBL upload preparation failed for {}. The most likely cause is that a stream size does not match its mip extent.",
					id
				);
			})?;
		let specular_uploads: [TextureUpload; IBL_SPECULAR_LEVEL_COUNT] = specular_uploads.try_into().map_err(|_| ())?;

		let device = self.factory.as_mut().ok_or_else(|| {
			log::error!(
				"Visibility environment creation failed for {}. The most likely cause is that the resource worker was configured without a GPU factory.",
				id
			);
		})?;
		let diffuse_name = format!("{id} diffuse irradiance");
		let diffuse_image = device.build_image(
			ghi::image::Builder::new(diffuse_format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(&diffuse_name)
				.extent(diffuse_extent)
				.cube_compatible()
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let specular_name = format!("{id} prefiltered specular");
		let specular_image = device.build_image(
			ghi::image::Builder::new(specular_format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(&specular_name)
				.extent(specular_extents[0])
				.cube_compatible()
				.mip_levels(IBL_SPECULAR_LEVEL_COUNT as u32)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let sampler = device.build_sampler(default_material_sampler_builder().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));

		Ok(FactoryEnvironment {
			diffuse_image,
			specular_image,
			sampler,
			diffuse_upload,
			specular_uploads,
		})
	}

	/// Reserves a bindless texture slot and reports whether the slot was newly created.
	fn reserve_texture_slot(&mut self, texture_id: &str) -> (u32, bool) {
		if let Some(index) = self.images_by_resource.get(texture_id) {
			return (*index as u32, false);
		}

		let idx = self.images.len() as u32;

		if idx as usize >= 1024 {
			panic!(
				"Visibility texture limit exceeded. The most likely cause is that the scene created more texture variants than the visibility pipeline supports."
			);
		}

		self.images.push(ResourceStates::pending(()));
		self.images_by_resource.insert(texture_id.to_string(), idx as usize);

		(idx, true)
	}

	/// Reserves a material slot for a mesh primitive.
	async fn request_material(&mut self, material_id: &str) -> u32 {
		let (index, inserted) = self.reserve_material_slot(material_id);
		if inserted {
			self.handle_material_request(material_id.to_string()).await;
		}
		index
	}

	/// Reserves a material slot and reports whether the slot was newly created.
	fn reserve_material_slot(&mut self, material_id: &str) -> (u32, bool) {
		if let Some(index) = self.material_by_name.get(material_id) {
			return (*index as u32, false);
		}

		let idx = self.materials.len() as u32;

		if idx as usize >= MAX_MATERIALS {
			panic!(
				"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
			);
		}

		let material_id = material_id.to_string();
		self.materials.push(ResourceStates::pending(material_id.clone()));
		self.material_by_name.insert(material_id, idx as usize);

		(idx, true)
	}

	/// Returns the material slot prepared before a mesh enters GPU transfer.
	fn material_index(&self, material_id: &str) -> Option<u32> {
		self.material_by_name.get(material_id).map(|index| *index as u32)
	}
}

/// The `VisibilityPipelineResourceManagerClient` struct connects render logic to the asynchronous visibility resource worker.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	pub(super) gpu_vertex_data_manager: GPUVertexDataManager,
	commands: kanal::Sender<VisibilityTransferCommand>,
	completions: Receiver<VisibilityResourceCompletion>,
}

/// The `VisibilityPipelineResourceManagerWorker` struct owns visibility resource loading and GPU transfer.
pub(crate) struct VisibilityPipelineResourceManagerWorker {
	resource_manager: VisibilityPipelineResourceManager,
	gpu_vertex_data_manager: GPUVertexDataManager,
	commands: kanal::AsyncReceiver<VisibilityTransferCommand>,
	completions: Sender<VisibilityResourceCompletion>,
	pending_mesh_uploads: VecDeque<(VisibilityMeshKey, PreparedMeshSource)>,
	pending_texture_uploads: VecDeque<(u32, ghi::BaseImageHandle, ghi::SamplerHandle, TextureUpload)>,
	pending_environment_uploads: VecDeque<PendingEnvironmentUpload>,
	submitted_uploads: VecDeque<SubmittedUploadBatch>,
}

impl VisibilityPipelineResourceManagerClient {
	/// Sends one ordered command to the asynchronous resource worker.
	fn send(&self, command: VisibilityTransferCommand) {
		if self.commands.send(command).is_err() {
			log::error!(
				"Visibility resource command failed. The most likely cause is that the asynchronous resource task terminated."
			);
		}
	}

	/// Requests a mesh from the asynchronous resource task.
	pub(crate) fn request_mesh(&self, key: VisibilityMeshKey, source: MeshSource) {
		self.send(VisibilityTransferCommand::RequestMesh { key, source });
	}

	/// Requests an image from the asynchronous resource task.
	pub(crate) fn request_image(&self, key: VisibilityTextureKey) {
		self.send(VisibilityTransferCommand::RequestImage { key });
	}

	/// Requests the baked lighting subresources stored with one environment image.
	pub(crate) fn request_environment(&self, id: String) {
		self.send(VisibilityTransferCommand::RequestEnvironment { id });
	}

	/// Configures material pipeline creation on the asynchronous resource task.
	pub(crate) fn configure_material_pipeline(&self, config: MaterialPipelineConfig) {
		self.send(VisibilityTransferCommand::ConfigureMaterialPipeline(config));
	}

	/// Drains completed resource work without blocking the render thread.
	pub(crate) fn drain_completions(&mut self) -> CompletionList {
		let mut completions = CompletionList::new();
		while let Ok(completion) = self.completions.try_recv() {
			completions.push(completion);
		}
		completions
	}

	/// Enqueues a texture upload and reports the descriptor data once the transfer frame completes.
	pub(crate) fn enqueue_texture_upload(
		&self,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
	) {
		self.send(VisibilityTransferCommand::EnqueueTextureUpload {
			index,
			image,
			sampler,
			upload,
		});
	}

	/// Enqueues every image in one environment as one transfer-frame completion.
	pub(crate) fn enqueue_environment_upload(&self, upload: PendingEnvironmentUpload) {
		self.send(VisibilityTransferCommand::EnqueueEnvironmentUpload { upload });
	}
}

impl VisibilityPipelineResourceManagerWorker {
	/// Records one prepared mesh without resolving storage or material dependencies on the transfer thread.
	async fn load_mesh_source_for_transfer<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		source: PreparedMeshSource,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		match source {
			PreparedMeshSource::Resource { mut resource } => {
				self.load_mesh_resource_for_transfer(transfer, staging_data_buffer, slice, &mut resource)
					.await
			}
			PreparedMeshSource::Generated {
				generator,
				material_index,
			} => {
				let mesh = self
					.gpu_vertex_data_manager
					.write_gpu_mesh_data_and_return_mesh_object_for_mesh_generator(
						generator.as_ref(),
						transfer,
						staging_data_buffer,
						slice,
					)
					.ok_or(())?;
				Ok(Self::convert_generated_mesh_data(mesh, material_index))
			}
		}
	}

	/// Records a resource-backed mesh and combines its GPU ranges with material slots prepared by the resource task.
	async fn load_mesh_resource_for_transfer<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		resource: &mut Reference<ResourceMesh>,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		let mesh = self
			.gpu_vertex_data_manager
			.write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource(transfer, staging_data_buffer, slice, resource)
			.await
			.ok_or(())?;

		let resource = resource.resource();
		if resource.primitives.len() != mesh.primitives.len() {
			log::error!(
				"Visibility mesh primitive count changed before transfer. The most likely cause is inconsistent mesh metadata."
			);
			return Err(());
		}

		// One shared binding per resource skin lets primitives of the same instance reuse an uploaded palette.
		let skin_bindings = resource.skins.iter().cloned().map(Arc::new).collect::<Vec<_>>();
		let primitives = resource
			.primitives
			.iter()
			.zip(mesh.primitives.iter())
			.enumerate()
			.map(|(primitive_index, (resource_primitive, primitive))| {
				let Some(material_index) = self.resource_manager.material_index(&resource_primitive.material.id) else {
					log::error!(
						"Visibility mesh material slot is missing for primitive {primitive_index}. The most likely cause is that mesh preparation did not finish its material dependencies."
					);
					return Err(());
				};
				let skin = match resource_primitive.skin {
					Some(skin_index) => {
						let Some(binding) = skin_bindings.get(skin_index as usize) else {
							log::error!(
								"Visibility mesh skin index is invalid for primitive {primitive_index}: {skin_index}. The most likely cause is that mesh validation was bypassed or the resource data is corrupted."
							);
							return Err(());
						};
						Some(binding.clone())
					}
					None => None,
				};

				Ok(crate::rendering::pipelines::visibility::pipeline_manager::MeshPrimitive {
					material_index,
					meshlet_count: primitive.meshlet_count,
					meshlet_offset: primitive.meshlet_offset,
					vertex_offset: primitive.vertex_offset,
					primitive_offset: primitive.primitive_offset,
					triangle_offset: primitive.triangle_offset,
					skinning_source_vertex_offset: primitive.skinning_source_vertex_offset,
					skinning_vertex_count: primitive.skinning_vertex_count,
					skin,
				})
			})
			.collect::<Result<Vec<_>, ()>>()?;

		Ok(crate::rendering::pipelines::visibility::pipeline_manager::MeshData {
			primitives,
			skeleton_node_count: resource
				.skeleton
				.as_ref()
				.map(|skeleton| skeleton.resource().nodes.len() as u32)
				.unwrap_or(0),
			vertex_offset: mesh.vertex_offset,
			primitive_offset: mesh.primitive_offset,
			triangle_offset: mesh.triangle_offset,
			meshlet_offset: mesh.meshlet_offset,
			acceleration_structure: mesh.acceleration_structure,
		})
	}

	/// Maps generated mesh geometry to render-facing metadata using its prepared material slot.
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

	/// Handles resource requests and transfer completion until the command channel closes.
	pub(crate) async fn run(
		mut self,
		mut transfer_queue: ghi::implementation::queue::Queue,
		transfer_finished_synchronizer: ghi::SynchronizerHandle,
		transfer_command_buffer: ghi::CommandBufferHandle,
		upload_buffer: ghi::BufferHandle<[u8; ASYNC_UPLOAD_BUFFER_BYTE_COUNT]>,
	) {
		let mut started_frame_count = 0;

		loop {
			// Advance submitted GPU work before starting another resource read. A
			// slow read can then delay only later queue observations.
			if self.has_active_transfer_work() {
				self.advance_transfer_queue(
					&mut transfer_queue,
					transfer_finished_synchronizer,
					transfer_command_buffer,
					upload_buffer,
					&mut started_frame_count,
				)
				.await;
			}

			let command = if self.has_active_transfer_work() {
				match self.commands.try_recv() {
					Ok(Some(command)) => command,
					Ok(None) => {
						// Submitted GPU work needs periodic queue progress even when
						// no new resource command arrives.
						compio::time::sleep(ACTIVE_TRANSFER_POLL_INTERVAL).await;
						continue;
					}
					Err(_) => break,
				}
			} else {
				let Ok(command) = self.commands.recv().await else {
					break;
				};
				command
			};

			if self.handle_command(command).await == ResourceWorkerFlow::Stop {
				break;
			}

			// Kanal can complete buffered receives synchronously. Yield after one
			// command so a backlog cannot monopolize an application tick.
			crate::core::async_runtime::yield_now().await;
		}
	}

	/// Advances one transfer frame and records all upload work already prepared by resource commands.
	async fn advance_transfer_queue(
		&mut self,
		transfer_queue: &mut ghi::implementation::queue::Queue,
		transfer_finished_synchronizer: ghi::SynchronizerHandle,
		transfer_command_buffer: ghi::CommandBufferHandle,
		upload_buffer: ghi::BufferHandle<[u8; ASYNC_UPLOAD_BUFFER_BYTE_COUNT]>,
		started_frame_count: &mut u64,
	) {
		let started_frame = transfer_queue.start_frame(*started_frame_count as _, transfer_finished_synchronizer);
		if let Some(completed_frame) = started_frame.completed_frame {
			self.signal_completed_frame(completed_frame);
		}

		if !self.has_pending_upload_work() {
			*started_frame_count += 1;
			return;
		}

		let mut frame = started_frame.frame;
		let frame_key = frame.key();
		let mut transfer_recording = frame.create_command_buffer_recording_without_implicit_sync(transfer_command_buffer);
		let buffer = transfer_recording.get_mut_buffer_slice(upload_buffer);
		let mut slice = utils::BufferAllocator::new(buffer.as_mut_slice());

		let prepared_uploads = self
			.prepare_uploads(&mut transfer_recording, upload_buffer.into(), &mut slice)
			.await;

		if prepared_uploads.recorded_work {
			// Resource loads write straight into the mapped upload buffer. Flush
			// those borrowed targets before the transfer commands consume them.
			transfer_recording.sync_buffer(upload_buffer);
			transfer_recording.execute(transfer_finished_synchronizer);
		} else {
			drop(transfer_recording);
		}

		self.track_submitted_uploads(frame_key, prepared_uploads.completions);
		*started_frame_count += 1;
	}

	/// Publishes upload completions for transfer frames reported as complete by the queue.
	pub(crate) fn signal_completed_frame(&mut self, completed_frame: ghi::FrameKey) {
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
	pub(crate) fn track_submitted_uploads(&mut self, frame_key: ghi::FrameKey, completions: CompletionList) {
		if completions.is_empty() {
			return;
		}

		self.submitted_uploads
			.push_back(SubmittedUploadBatch { frame_key, completions });
	}

	/// Records pending mesh and texture uploads into the transfer command buffer.
	async fn prepare_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> TransferUploadPrepareResult {
		self.record_uploads(transfer, staging_data_buffer, slice).await
	}

	/// Reports whether upload queues contain work that needs GPU transfer recording.
	fn has_pending_upload_work(&self) -> bool {
		!self.pending_mesh_uploads.is_empty()
			|| !self.pending_texture_uploads.is_empty()
			|| !self.pending_environment_uploads.is_empty()
	}

	/// Reports whether the queue must keep advancing submitted or pending transfers.
	fn has_active_transfer_work(&self) -> bool {
		self.has_pending_upload_work() || !self.submitted_uploads.is_empty()
	}

	/// Moves one ordered resource command into worker-owned state.
	async fn handle_command(&mut self, command: VisibilityTransferCommand) -> ResourceWorkerFlow {
		match command {
			VisibilityTransferCommand::RequestMesh { key, source } => {
				match self.resource_manager.prepare_mesh_source(source).await {
					Ok(source) => self.pending_mesh_uploads.push_back((key, source)),
					Err(()) => {
						let _ = self
							.completions
							.send(VisibilityResourceCompletion::Failed { key: key.into() });
					}
				}
			}
			VisibilityTransferCommand::RequestImage { key } => {
				self.resource_manager.handle_image_request(key).await;
			}
			VisibilityTransferCommand::RequestEnvironment { id } => {
				self.resource_manager.handle_environment_request(id).await;
			}
			VisibilityTransferCommand::ConfigureMaterialPipeline(config) => {
				self.resource_manager.configure_material_pipeline(config);
			}
			VisibilityTransferCommand::EnqueueTextureUpload {
				index,
				image,
				sampler,
				upload,
			} => {
				self.pending_texture_uploads.push_back((index, image, sampler, upload));
			}
			VisibilityTransferCommand::EnqueueEnvironmentUpload { upload } => {
				self.pending_environment_uploads.push_back(upload);
			}
			VisibilityTransferCommand::Shutdown => return ResourceWorkerFlow::Stop,
		}

		ResourceWorkerFlow::Continue
	}

	/// Records queued upload work into the transfer command buffer.
	async fn record_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> TransferUploadPrepareResult {
		let mut recorded_work = false;
		let mut completions = CompletionList::new();
		const TEXTURE_UPLOAD_ALIGNMENT: usize = 256;

		while let Some((key, source)) = self.pending_mesh_uploads.pop_front() {
			let source_kind = source.kind();
			let result = self
				.load_mesh_source_for_transfer(transfer, staging_data_buffer, slice, source)
				.await;
			match result {
				Ok(mesh) => {
					let meshlet_count = mesh.primitives.iter().map(|primitive| primitive.meshlet_count).sum::<u32>();

					// This logs unique visibility mesh resources as they are uploaded, not scene instances.
					log::debug!(
						"Visibility mesh created: key={}, source={}, primitives={}, meshlets={}, vertex_offset={}, primitive_offset={}, triangle_offset={}, meshlet_offset={}",
						key,
						source_kind,
						mesh.primitives.len(),
						meshlet_count,
						mesh.vertex_offset,
						mesh.primitive_offset,
						mesh.triangle_offset,
						mesh.meshlet_offset,
					);

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

		while let Some((index, image, sampler, upload)) = self.pending_texture_uploads.pop_front() {
			if upload.data.len() > slice.remaining_aligned(TEXTURE_UPLOAD_ALIGNMENT) {
				self.pending_texture_uploads.push_front((index, image, sampler, upload));
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
				0,
			)]);
			completions.push(VisibilityResourceCompletion::TextureUploadReady { index, image, sampler });
			recorded_work = true;
		}

		while let Some(upload) = self.pending_environment_uploads.pop_front() {
			let upload_size = upload
				.specular_uploads
				.iter()
				.try_fold(
					upload.diffuse.upload.data.len().next_multiple_of(TEXTURE_UPLOAD_ALIGNMENT),
					|total, mip| total.checked_add(mip.data.len().next_multiple_of(TEXTURE_UPLOAD_ALIGNMENT)),
				)
				.expect(
					"Visibility environment upload size overflowed. The most likely cause is malformed IBL stream metadata.",
				);
			if upload_size > slice.remaining_aligned(TEXTURE_UPLOAD_ALIGNMENT) {
				self.pending_environment_uploads.push_front(upload);
				break;
			}

			let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
			copies.push(stage_texture_upload(
				slice,
				staging_data_buffer,
				upload.diffuse.image,
				&upload.diffuse.upload,
				TEXTURE_UPLOAD_ALIGNMENT,
				0,
			));
			for (mip_level, mip) in upload.specular_uploads.iter().enumerate() {
				copies.push(stage_texture_upload(
					slice,
					staging_data_buffer,
					upload.specular_image,
					mip,
					TEXTURE_UPLOAD_ALIGNMENT,
					mip_level as u32,
				));
			}
			transfer.copy_buffer_to_images(&copies);

			completions.push(VisibilityResourceCompletion::EnvironmentUploadReady {
				id: upload.id,
				diffuse_image: upload.diffuse.image,
				specular_image: upload.specular_image,
				sampler: upload.sampler,
			});
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
	pub(crate) completions: CompletionList,
}

/// The `SubmittedUploadBatch` struct holds resource completions until a transfer frame is complete.
struct SubmittedUploadBatch {
	frame_key: ghi::FrameKey,
	completions: CompletionList,
}

/// The `PreparedMeshSource` enum keeps storage and material lifetimes alive until GPU transfer.
enum PreparedMeshSource {
	Resource {
		resource: Reference<ResourceMesh>,
	},
	Generated {
		generator: Arc<dyn crate::rendering::mesh::generator::MeshGenerator>,
		material_index: u32,
	},
}

impl PreparedMeshSource {
	/// Returns the source label used by visibility upload diagnostics.
	fn kind(&self) -> &'static str {
		match self {
			Self::Resource { .. } => "resource",
			Self::Generated { .. } => "generated",
		}
	}
}

#[derive(PartialEq, Eq)]
enum ResourceWorkerFlow {
	Continue,
	Stop,
}

/// The `VisibilityResourceCompletion` enum describes resource work that is ready for render-thread adoption.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		key: VisibilityMeshKey,
		mesh: crate::rendering::pipelines::visibility::pipeline_manager::MeshData,
	},
	PipelineReady {
		name: String,
		pipeline: ghi::factory::ComputePipeline,
	},
	MaterialReady {
		id: String,
		index: u32,
		pipeline: Option<ghi::PipelineHandle>,
		pending_pipeline: Option<PendingMaterialPipeline>,
		alpha_mode: AlphaMode,
		textures: Vec<Option<(String, u32)>>,
	},
	ImageReady {
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		upload: TextureUpload,
	},
	EnvironmentReady {
		id: String,
		environment: FactoryEnvironment,
	},
	TextureUploadReady {
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
	},
	EnvironmentUploadReady {
		id: String,
		diffuse_image: ghi::BaseImageHandle,
		specular_image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
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
	RequestImage {
		key: VisibilityTextureKey,
	},
	RequestEnvironment {
		id: String,
	},
	ConfigureMaterialPipeline(MaterialPipelineConfig),
	EnqueueTextureUpload {
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
	},
	EnqueueEnvironmentUpload {
		upload: PendingEnvironmentUpload,
	},
	Shutdown,
}

/// The `VisibilityResourceKey` enum identifies a visibility resource independently of scene instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityResourceKey {
	Mesh(VisibilityMeshKey),
	Texture(VisibilityTextureKey),
	Material(String),
	Environment(String),
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
			VisibilityResourceKey::Environment(key) => key.fmt(f),
		}
	}
}

/// The `FactoryTexture` struct packages detached texture resources with upload bytes for render-thread adoption.
struct FactoryTexture {
	index: u32,
	image: ghi::implementation::factory::Image,
	sampler: ghi::implementation::factory::Sampler,
	upload: TextureUpload,
}

/// The `FactoryEnvironment` struct keeps one baked IBL set together until the render thread interns its GPU resources.
pub(crate) struct FactoryEnvironment {
	diffuse_image: ghi::implementation::factory::Image,
	specular_image: ghi::implementation::factory::Image,
	sampler: ghi::implementation::factory::Sampler,
	diffuse_upload: TextureUpload,
	specular_uploads: [TextureUpload; IBL_SPECULAR_LEVEL_COUNT],
}

impl FactoryEnvironment {
	/// Interns all detached resources while preserving the batch that the transfer worker will publish atomically.
	pub(crate) fn intern(self, id: String, frame: &mut ghi::implementation::Frame) -> PendingEnvironmentUpload {
		let diffuse_image = ghi::BaseImageHandle::from(frame.intern_image(self.diffuse_image));
		let specular_image = ghi::BaseImageHandle::from(frame.intern_image(self.specular_image));
		let sampler = frame.intern_sampler(self.sampler);
		let specular_uploads = self.specular_uploads;

		PendingEnvironmentUpload {
			id,
			diffuse: PendingEnvironmentImageUpload {
				image: diffuse_image,
				upload: self.diffuse_upload,
			},
			specular_image,
			specular_uploads,
			sampler,
		}
	}
}

/// The `PendingEnvironmentImageUpload` struct pairs one interned IBL image with its row-padded transfer bytes.
struct PendingEnvironmentImageUpload {
	image: ghi::BaseImageHandle,
	upload: TextureUpload,
}

/// The `PendingEnvironmentUpload` struct keeps a complete environment on one transfer frame and completion boundary.
pub(crate) struct PendingEnvironmentUpload {
	id: String,
	diffuse: PendingEnvironmentImageUpload,
	specular_image: ghi::BaseImageHandle,
	specular_uploads: [TextureUpload; IBL_SPECULAR_LEVEL_COUNT],
	sampler: ghi::SamplerHandle,
}

/// The `FactoryMaterial` struct packages material metadata with pending render-thread pipeline state.
struct FactoryMaterial {
	index: u32,
	pipeline: Option<ghi::PipelineHandle>,
	pending_pipeline: Option<PendingMaterialPipeline>,
	alpha_mode: AlphaMode,
	textures: Vec<Option<(String, u32)>>,
}

/// The `PendingMaterialPipeline` struct carries a material-evaluation pipeline
/// request that must be completed on the render thread.
pub(crate) struct PendingMaterialPipeline {
	request: ComputePipelineRequest,
}

impl PendingMaterialPipeline {
	pub(crate) fn create(self, frame: &mut ghi::implementation::Frame) -> Option<ghi::PipelineHandle> {
		let shader = self.request.shader;
		let shader_handle = frame
			.create_shader(
				shader.name.as_deref(),
				shader.source.sources(),
				shader.stage,
				shader.resource_descriptors.iter().copied(),
			)
			.map_err(|_| {
				log::error!(
					"Material shader creation failed for {}. The most likely cause is invalid shader payload data.",
					self.request.key
				);
			})
			.ok()?;

		let pipeline_builder = ghi::pipelines::compute::Builder::new(
			&self.request.push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, shader.stage)
				.with_specialization_map(&self.request.specialization_map_entries),
		);
		let pipeline_builder = if let Some(name) = shader.name.as_deref() {
			pipeline_builder.name(name)
		} else {
			pipeline_builder
		};

		Some(frame.create_compute_pipeline(pipeline_builder))
	}
}

#[derive(Default)]
struct QueuedMaterialPipeline {
	pipeline: Option<ghi::PipelineHandle>,
	pending_pipeline: Option<PendingMaterialPipeline>,
}

/// The `MaterialPipelineConfig` struct names the push-constant and factory contract for material evaluation pipelines.
pub(crate) struct MaterialPipelineConfig {
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	pipeline_factory: Option<ghi::implementation::Factory>,
}

impl MaterialPipelineConfig {
	/// Creates a material pipeline configuration used by the visibility resource worker.
	pub(crate) fn new(
		push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
		pipeline_factory: Option<ghi::implementation::Factory>,
	) -> Self {
		Self {
			push_constant_ranges,
			pipeline_factory,
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
	DXIL(ResourceReaderBacking),
	HLSL {
		source: String,
		entry_point: String,
	},
	MTLB {
		binary: ResourceReaderBacking,
		entry_point: String,
		threadgroup_size: Option<Extent>,
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
			OwnedShaderSource::DXIL(binary) => ghi::shader::Sources::DXIL(binary.as_slice()),
			OwnedShaderSource::HLSL { source, entry_point } => ghi::shader::Sources::HLSL { source, entry_point },
			OwnedShaderSource::MTLB {
				binary,
				entry_point,
				threadgroup_size,
			} => ghi::shader::Sources::MTLB {
				binary: binary.as_slice(),
				entry_point,
				threadgroup_size: *threadgroup_size,
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
	resource_descriptors: Vec<ghi::ShaderResourceDescriptor>,
}

/// The `ComputePipelineRequest` struct packages the resource data needed to compile a material compute pipeline off-thread.
struct ComputePipelineRequest {
	key: String,
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
		reason: String,
	},
}

impl VisibilityPipelineResourceManager {
	fn compile_compute_pipeline(
		device: &mut ghi::implementation::Factory,
		request: ComputePipelineRequest,
	) -> Result<ghi::implementation::ComputePipeline, String> {
		use ghi::Device as _;

		let shader = request.shader;
		let shader_handle = device.create_shader(
			shader.name.as_deref(),
			shader.source.sources(),
			shader.stage,
			shader.resource_descriptors.iter().copied(),
		)
		.map_err(|_| {
			format!(
				"shader creation failed for {}. The most likely cause is that the active backend does not support detached shader creation for this shader source or the shader payload is invalid.",
				request.key
			)
		})?;

		let pipeline_builder = ghi::pipelines::compute::Builder::new(
			&request.push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, shader.stage)
				.with_specialization_map(&request.specialization_map_entries),
		);
		let pipeline_builder = if let Some(name) = shader.name.as_deref() {
			pipeline_builder.name(name)
		} else {
			pipeline_builder
		};

		Ok(device.create_compute_pipeline(pipeline_builder))
	}

	fn queue_compute_pipeline(&mut self, request: ComputePipelineRequest) {
		let key = request.key.clone();
		let Some(pipeline_factory) = self.factory.as_mut() else {
			self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
			log::error!(
				"Pipeline compilation failed for {}. The most likely cause is that material pipeline creation was configured without a pipeline factory.",
				key
			);
			return;
		};
		let result = catch_unwind(AssertUnwindSafe(|| Self::compile_compute_pipeline(pipeline_factory, request)));

		match result {
			Ok(Ok(pipeline)) => {
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
			Ok(Err(reason)) => {
				self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
				log::error!(
					"Pipeline compilation failed for {}: {}. The most likely cause is that shader creation or pipeline specialization failed on the resource-manager thread.",
					key,
					reason
				);
			}
			Err(_) => {
				self.pipelines.write().insert(key.clone(), PipelineStatus::Failed);
				log::error!(
					"Pipeline compilation panicked for {}. The most likely cause is that shader creation or pipeline specialization failed on the resource-manager thread.",
					key
				);
			}
		}
	}

	pub(crate) fn poll_pipelines(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		_max_results: usize,
	) -> Vec<(String, ghi::PipelineHandle)> {
		Vec::new()
	}

	pub(crate) fn drain_pipeline_completions(&mut self, _max_results: usize) {}

	/// Loads shader backing once so sync and async pipeline creation can reuse the same payload.
	async fn load_cached_shader_request(&self, shader: &mut Reference<Shader>) -> Result<Arc<OwnedShader>, ()> {
		if let StaleEntry::Fresh(shader_request) = self.shader_requests.read().entry(&shader.id, shader.get_hash()) {
			return Ok(Arc::clone(shader_request));
		}

		let resource_descriptors = shader
			.resource()
			.interface
			.bindings
			.iter()
			.map(crate::rendering::resource_loading::binding_to_descriptor)
			.collect::<Vec<_>>();

		let stage = crate::rendering::resource_loading::shader_type_to_ghi(shader.resource().stage);
		let shader_backing = Self::load_shader_backing(shader).await?;

		let source = match &shader.resource().artifact {
			ShaderArtifact::Dxil => OwnedShaderSource::DXIL(shader_backing),
			ShaderArtifact::Hlsl { entry_point } => OwnedShaderSource::HLSL {
				source: std::str::from_utf8(shader_backing.as_slice())
					.map_err(|_| {
						log::error!(
							"Failed to load HLSL shader {}. The most likely cause is invalid UTF-8 shader bytes.",
							shader.id()
						);
					})?
					.to_string(),
				entry_point: entry_point.clone(),
			},
			ShaderArtifact::Msl { entry_point } => OwnedShaderSource::MTL {
				source: std::str::from_utf8(shader_backing.as_slice())
					.map_err(|_| {
						log::error!(
							"Failed to load MSL shader {}. The most likely cause is invalid UTF-8 shader bytes.",
							shader.id()
						);
					})?
					.to_string(),
				entry_point: entry_point.clone(),
			},
			ShaderArtifact::Mtlb { entry_point } => OwnedShaderSource::MTLB {
				binary: shader_backing,
				entry_point: entry_point.clone(),
				threadgroup_size: shader
					.resource()
					.interface
					.workgroup_size
					.map(|(width, height, depth)| Extent::new(width, height, depth)),
			},
			ShaderArtifact::Spirv => OwnedShaderSource::SPIRV(shader_backing),
		};

		let shader_request = Arc::new(OwnedShader {
			name: Some(shader.id().to_string()),
			source,
			stage,
			resource_descriptors,
		});

		self.shader_requests
			.write()
			.insert(shader.id().to_string(), shader.get_hash(), Arc::clone(&shader_request));

		Ok(shader_request)
	}

	/// Loads shader bytes from reader backing storage and falls back to an owned buffer when direct backing is unavailable.
	async fn load_shader_backing(shader: &mut Reference<Shader>) -> Result<ResourceReaderBacking, ()> {
		match shader.consume_reader().into_backing_storage().await {
			Ok(backing) => Ok(backing),
			Err(mut reader) => {
				let read_target = ReadTargetsMut::create_buffer(shader);
				let load_request = reader.read_into(None, read_target).await.map_err(|_| {
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

	async fn queue_material_pipeline(
		&mut self,
		resource_id: String,
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		material: &mut ResourceMaterial,
	) -> QueuedMaterialPipeline {
		self.queue_material_pipeline_with_specialization(resource_id, push_constant_ranges, material, Vec::new())
			.await
	}

	/// Queues a material pipeline request with variant specialization constants.
	async fn queue_material_pipeline_with_specialization(
		&mut self,
		resource_id: String,
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		material: &mut ResourceMaterial,
		specialization_map_entries: Vec<ghi::pipelines::SpecializationMapEntry>,
	) -> QueuedMaterialPipeline {
		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => QueuedMaterialPipeline::default(),
				PipelineStatus::Ready(handle) => QueuedMaterialPipeline {
					pipeline: Some(*handle),
					pending_pipeline: None,
				},
			};
		}

		self.pipelines.write().insert(resource_id.clone(), PipelineStatus::Pending);

		let request = match material.shaders_mut().iter_mut().next() {
			Some(shader) => self
				.load_cached_shader_request(shader)
				.await
				.map(|shader| ComputePipelineRequest {
					key: resource_id.clone(),
					push_constant_ranges: push_constant_ranges.to_vec(),
					shader,
					specialization_map_entries,
				}),
			None => Err(()),
		};

		match request {
			Ok(request) => {
				if Self::supports_async_material_pipeline_creation() {
					self.queue_compute_pipeline(request);
				} else {
					return QueuedMaterialPipeline {
						pipeline: None,
						pending_pipeline: Some(PendingMaterialPipeline { request }),
					};
				}
			}
			Err(()) => {
				self.pipelines.write().insert(resource_id, PipelineStatus::Failed);
			}
		}

		QueuedMaterialPipeline::default()
	}

	fn supports_async_material_pipeline_creation() -> bool {
		ghi::implementation::USES_DX12 || ghi::implementation::USES_METAL
	}
}

/// Computes the independently uploaded 2D extent for one baked specular roughness level.
fn environment_mip_extent(base_extent: [u32; 3], level: u32) -> Extent {
	Extent::new(
		(base_extent[0] >> level).max(1),
		(base_extent[1] >> level).max(1),
		base_extent[2].max(1),
	)
}

/// Returns the compact byte count expected for one ordinary single-mip IBL image.
fn compact_image_byte_size(format: ghi::Formats, extent: Extent) -> usize {
	format.compact_copy_layout(extent.width().max(1), extent.height().max(1)).2
}

/// Copies one prepared texture into the shared staging allocation and returns its GPU copy descriptor.
fn stage_texture_upload(
	slice: &mut utils::BufferAllocator<'_>,
	staging_data_buffer: ghi::BaseBufferHandle,
	image: ghi::BaseImageHandle,
	upload: &TextureUpload,
	alignment: usize,
	mip_level: u32,
) -> ghi::BufferImageCopyDescriptor {
	let (source_offset, source_buffer) = slice.take_with_offset_aligned(upload.data.len(), alignment);
	source_buffer.copy_from_slice(&upload.data);
	ghi::BufferImageCopyDescriptor::new(
		staging_data_buffer,
		source_offset,
		upload.source_bytes_per_row,
		upload.source_bytes_per_image,
		image,
		mip_level,
	)
}

/// Builds row-padded upload data compatible with the transfer command buffer image copy path.
fn make_texture_upload(format: ghi::Formats, extent: Extent, source: &[u8]) -> Option<TextureUpload> {
	make_layered_texture_upload(format, extent, 1, source)
}

/// Builds row-padded upload data with one image pitch per array layer.
fn make_layered_texture_upload(
	format: ghi::Formats,
	extent: Extent,
	layer_count: usize,
	source: &[u8],
) -> Option<TextureUpload> {
	let (source_bytes_per_row, row_count, compact_bytes_per_image) =
		format.compact_copy_layout(extent.width().max(1), extent.height().max(1));
	let compact_size = compact_bytes_per_image.checked_mul(layer_count)?;
	if source.len() < compact_size {
		return None;
	}
	assert_eq!(
		source.len(),
		compact_size,
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
	let mut data = vec![0u8; source_bytes_per_image.checked_mul(layer_count)?];

	for layer in 0..layer_count {
		for row in 0..row_count {
			let source_offset = layer * compact_bytes_per_image + row * source_bytes_per_row;
			let destination_offset = layer * source_bytes_per_image + row * padded_bytes_per_row;
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
	}
	assert_eq!(
		data.len(),
		source_bytes_per_image * layer_count,
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
		resource_management::types::Formats::RGBA16F => ghi::Formats::RGBA16F,
		resource_management::types::Formats::BC5 => ghi::Formats::BC5,
		resource_management::types::Formats::BC5SNORM => ghi::Formats::BC5SNORM,
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

/// Converts a worker panic into a useful error reason for async pipeline diagnostics.
fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
	if let Some(message) = payload.downcast_ref::<&str>() {
		return (*message).to_string();
	}

	if let Some(message) = payload.downcast_ref::<String>() {
		return message.clone();
	}

	"pipeline worker panicked with a non-string payload. The most likely cause is that backend pipeline creation hit an unexpected assertion.".to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn owned_dxil_source_maps_to_native_ghi_bytecode() {
		let source = OwnedShaderSource::DXIL(ResourceReaderBacking::Buffer(vec![1, 2, 3, 4].into_boxed_slice()));

		assert!(matches!(
			source.sources(),
			ghi::shader::Sources::DXIL(bytes) if bytes == [1, 2, 3, 4]
		));
	}

	#[test]
	fn resource_commands_reach_the_async_worker_in_fifo_order() {
		let executor = resource_management::r#async::Executor::new().expect("expected test value");
		let (sender, receiver) = kanal::unbounded_async();
		let sender = sender.to_sync();

		for id in ["first", "second", "third"] {
			sender
				.send(VisibilityTransferCommand::RequestEnvironment { id: id.to_string() })
				.expect("expected test value");
		}

		let received = executor.block_on(async {
			let mut ids = Vec::new();
			for _ in 0..3 {
				let VisibilityTransferCommand::RequestEnvironment { id } = receiver.recv().await.expect("expected test value")
				else {
					panic!(
						"Unexpected visibility command. The most likely cause is that the FIFO test enqueued the wrong variant."
					);
				};
				ids.push(id);
			}
			ids
		});

		assert_eq!(received, ["first", "second", "third"]);
	}

	#[test]
	fn texture_upload_preserves_minimum_extent_and_bc_row_contents() {
		let extent = Extent::rectangle(5, 7);
		let compact_row = 2 * 16;
		let source = (0..(compact_row * 2)).map(|value| value as u8).collect::<Vec<_>>();

		let upload = make_texture_upload(ghi::Formats::BC7, extent, &source).expect("expected test value");

		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 256 * 2);
		assert_eq!(&upload.data[0..compact_row], &source[0..compact_row]);
		assert!(upload.data[compact_row..256].iter().all(|byte| *byte == 0));
		assert_eq!(&upload.data[256..256 + compact_row], &source[compact_row..compact_row * 2]);

		let zero_extent =
			make_texture_upload(ghi::Formats::RGBA8UNORM, Extent::rectangle(0, 0), &[1, 2, 3, 4]).expect("expected test value");
		assert_eq!(zero_extent.source_bytes_per_row, 256);
		assert_eq!(zero_extent.source_bytes_per_image, 256);
		assert_eq!(&zero_extent.data[..4], &[1, 2, 3, 4]);
	}

	/// Ensures half-float HDR pixels reach the transfer buffer without normalization or channel conversion.
	#[test]
	fn texture_upload_preserves_rgba16f_environment_rows() {
		let extent = Extent::rectangle(2, 2);
		let compact_row = 2 * 8;
		let source = (0..compact_row * 2).map(|value| value as u8).collect::<Vec<_>>();

		let upload = make_texture_upload(ghi::Formats::RGBA16F, extent, &source).expect("expected test value");

		assert_eq!(
			resource_image_format_to_ghi(resource_management::types::Formats::RGBA16F),
			ghi::Formats::RGBA16F
		);
		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(&upload.data[..compact_row], &source[..compact_row]);
		assert_eq!(&upload.data[256..256 + compact_row], &source[compact_row..]);
	}

	#[test]
	fn cubemap_upload_preserves_every_face_and_image_pitch() {
		let extent = Extent::square(2);
		let compact_face_size = 2 * 2 * 8;
		let source = (0..compact_face_size * 6).map(|value| value as u8).collect::<Vec<_>>();
		let upload = make_layered_texture_upload(ghi::Formats::RGBA16F, extent, 6, &source).expect("cubemap upload");
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(upload.data.len(), 512 * 6);
		for face in 0..6 {
			for row in 0..2 {
				let source_start = face * compact_face_size + row * 16;
				let upload_start = face * 512 + row * 256;
				assert_eq!(
					&upload.data[upload_start..upload_start + 16],
					&source[source_start..source_start + 16]
				);
			}
		}
	}

	#[test]
	fn environment_specular_streams_form_one_gpu_mip_chain() {
		let extents: [Extent; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| environment_mip_extent([256, 256, 1], level as u32));

		assert_eq!(extents[0], Extent::square(256));
		assert_eq!(extents[1], Extent::square(128));
		assert_eq!(extents[7], Extent::square(2));
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[0]), 256 * 256 * 8);
		assert_eq!(compact_image_byte_size(ghi::Formats::RGBA16F, extents[7]), 2 * 2 * 8);
	}
}

pub enum ResourceStates<P, L> {
	/// The resource is waiting to be processed.
	Pending(P),
	/// The resource is loading.
	Loading(ghi::FrameKey, L),
	/// The resource is ready for use.
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

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::Device as _;
use ghi::Queue as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	Size as _,
};
use resource_management::resource::reader::ResourceReaderBacking;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resource::{ReadTargets, ReadTargetsMut};
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::{
	Material as ResourceMaterial, Shader, ShaderArtifact, Value, Variant as ResourceVariant,
};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use resource_management::types::{AlphaMode, ShaderTypes};
use resource_management::Reference;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::stale_map::{Entry as StaleEntry, StaleHashMap};
use utils::sync::RwLock;
use utils::Extent;

use crate::core::EntityHandle;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{GPUVertexDataManager, MeshData as GpuMeshData};
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::rendering::renderable::mesh::MeshSource;
use crate::resource_management::{self};
