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
	/// Unified command channel used by callers and independently prepared resources.
	commands: kanal::Sender<VisibilityTransferCommand>,
	resource_factory: Option<ghi::implementation::Factory>,
	material_pipeline_config: Option<MaterialPipelineConfig>,
	work_completions: Sender<VisibilityResourceCompletion>,
	upload_staging: Arc<super::upload_staging::UploadStagingArena>,
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
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
	) -> (
		VisibilityPipelineResourceManagerClient,
		VisibilityPipelineResourceManagerWorker,
	) {
		let mesh_data_manager = GPUVertexDataManager::new(context);
		let gpu_vertex_data_manager = mesh_data_manager.clone();
		let (commands, command_receiver) = kanal::unbounded_async();
		let commands = commands.to_sync();
		let (work_completions, work_completion_receiver) = mpsc::channel();
		let resource_manager = Self::new(resource_manager, commands.clone(), work_completions.clone(), upload_staging);

		(
			VisibilityPipelineResourceManagerClient {
				gpu_vertex_data_manager,
				commands: commands.clone(),
				completions: work_completion_receiver,
			},
			VisibilityPipelineResourceManagerWorker {
				resource_manager,
				gpu_vertex_data_manager: mesh_data_manager,
				commands: command_receiver,
				completions: work_completions,
				pending_uploads: VecDeque::new(),
				submitted_uploads: VecDeque::new(),
			},
		)
	}

	fn new(
		resource_manager: EntityHandle<ResourceManager>,
		commands: kanal::Sender<VisibilityTransferCommand>,
		work_completions: Sender<VisibilityResourceCompletion>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
	) -> Self {
		Self {
			images: Vec::with_capacity(4096),
			images_by_resource: HashMap::with_capacity(4096),
			materials: Vec::with_capacity(4096),
			material_by_name: HashMap::with_capacity(4096),
			resource_manager,
			commands,
			resource_factory: None,
			material_pipeline_config: None,
			work_completions,
			upload_staging,
		}
	}

	/// Runs one CPU preparation job on the current resource runtime and returns its result through the unified queue.
	fn spawn_preparation(&self, preparation: impl std::future::Future<Output = VisibilityTransferCommand> + 'static) {
		let commands = self.commands.clone();
		resource_management::r#async::spawn(async move {
			let command = preparation.await;
			if commands.send(command).is_err() {
				log::error!(
					"Visibility preparation completion failed. The most likely cause is that the transfer worker stopped receiving resource work."
				);
			}
		})
		.detach();
	}

	/// Stores the descriptor layout data needed to compile material evaluation pipelines.
	pub(crate) fn configure_material_pipeline(&mut self, mut config: MaterialPipelineConfig) {
		self.resource_factory = config.resource_factory.take();
		self.material_pipeline_config = Some(config);
	}

	/// Starts mesh metadata preparation without waiting for any material dependency.
	fn request_mesh_preparation(&self, key: VisibilityMeshKey, source: MeshSource) {
		let resource_manager = self.resource_manager.clone();
		self.spawn_preparation(async move {
			match source {
				MeshSource::Resource(id) => match resource_manager.request::<ResourceMesh>(&id).await {
					Ok(resource) => VisibilityTransferCommand::ResourceMeshLoaded {
						key,
						resource,
					},
					Err(_) => {
						log::error!(
							"Visibility mesh resource request failed for {}. The most likely cause is that the mesh id is missing or the asset database is not loaded.",
							id
						);
						VisibilityTransferCommand::PreparationFailed {
							key: VisibilityResourceKey::Mesh(key),
						}
					}
				},
				MeshSource::Generated(generator) => VisibilityTransferCommand::GeneratedMeshLoaded {
					key,
					generator,
				},
			}
		});
	}

	/// Reserves render dependencies from mesh metadata, then prepares only that mesh's upload data.
	fn prepare_loaded_resource_mesh(&mut self, key: VisibilityMeshKey, resource: Reference<ResourceMesh>) {
		let resource_data = resource.resource();
		let material_indices = resource_data
			.primitives
			.iter()
			.map(|primitive| self.request_material(&primitive.material.id))
			.collect::<Vec<_>>();
		let primitive_skins = resource_data
			.primitives
			.iter()
			.map(|primitive| primitive.skin)
			.collect::<Vec<_>>();
		let skin_bindings = resource_data.skins.iter().cloned().map(Arc::new).collect::<Vec<_>>();
		let skeleton_node_count = resource_data
			.skeleton
			.as_ref()
			.map(|skeleton| skeleton.resource().nodes.len() as u32)
			.unwrap_or(0);
		let upload_staging = self.upload_staging.clone();
		self.spawn_preparation(async move {
			match PreparedGpuMesh::prepare_resource_mesh(resource, upload_staging).await {
				Some(mesh) => VisibilityTransferCommand::UploadPrepared(PreparedUpload::ResourceMesh {
					key,
					mesh,
					material_indices,
					primitive_skins,
					skin_bindings,
					skeleton_node_count,
				}),
				None => VisibilityTransferCommand::PreparationFailed {
					key: VisibilityResourceKey::Mesh(key),
				},
			}
		});
	}

	/// Reserves the default material and prepares generated geometry for the common upload queue.
	fn prepare_loaded_generated_mesh(
		&mut self,
		key: VisibilityMeshKey,
		generator: Arc<dyn crate::rendering::mesh::generator::MeshGenerator>,
	) {
		let material_index = self.request_material("white_solid.bema");
		let upload_staging = self.upload_staging.clone();
		self.spawn_preparation(async move {
			match PreparedGpuMesh::prepare_generated_mesh(generator.as_ref(), upload_staging).await {
				Some(mesh) => VisibilityTransferCommand::UploadPrepared(PreparedUpload::GeneratedMesh {
					key,
					mesh,
					material_index,
				}),
				None => VisibilityTransferCommand::PreparationFailed {
					key: VisibilityResourceKey::Mesh(key),
				},
			}
		});
	}

	/// Sends one loading result without blocking the resource task.
	fn send_completion(&self, completion: VisibilityResourceCompletion) {
		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility resource completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Adopts a requested pipeline reference and starts every texture dependency independently.
	fn adopt_prepared_material(
		&mut self,
		id: String,
		index: u32,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		texture_keys: Vec<Option<VisibilityTextureKey>>,
		pipeline: crate::rendering::PipelineRef,
	) {
		let textures = texture_keys
			.into_iter()
			.map(|key| {
				key.map(|key| {
					let index = self.request_texture_dependency(key.clone());
					(key.as_str().to_string(), index)
				})
			})
			.collect();
		self.send_completion(VisibilityResourceCompletion::MaterialReady {
			id,
			index,
			pipeline,
			alpha_mode,
			coverage,
			textures,
		});
	}

	/// Queues a texture dependency discovered while loading another resource.
	fn request_texture_dependency(&mut self, key: VisibilityTextureKey) -> u32 {
		let (index, inserted) = self.reserve_texture_slot(key.as_str());
		if inserted {
			self.request_image_preparation(key, index);
		}
		index
	}

	/// Starts one texture's CPU preparation without waiting for sibling textures or its material pipeline.
	fn request_image_preparation(&self, key: VisibilityTextureKey, index: u32) {
		let resource_manager = self.resource_manager.clone();
		let upload_staging = self.upload_staging.clone();
		let failure_key = key.clone();
		self.spawn_preparation(async move {
			match Self::prepare_texture(resource_manager, upload_staging, key, index).await {
				Ok(texture) => VisibilityTransferCommand::TexturePrepared { texture },
				Err(()) => VisibilityTransferCommand::PreparationFailed {
					key: VisibilityResourceKey::Texture(failure_key),
				},
			}
		});
	}

	/// Loads one texture into owned row-padded data without borrowing transfer memory.
	async fn prepare_texture(
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
		key: VisibilityTextureKey,
		index: u32,
	) -> Result<PreparedTexture, ()> {
		let id = key.as_str();
		let mut reference: Reference<ResourceImage> = resource_manager.request(id).await.map_err(|_| {
				log::error!(
					"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
					id
			);
		})?;
		let texture = reference.resource();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);

		let mip_count = texture.mip_count.max(1);
		let available_mip_count = resource_management::resources::mips::mip_level_count(extent.width(), extent.height())
			.map_err(|_| {
				log::error!(
					"Visibility texture dimensions are invalid for {}. The most likely cause is that the baked image has a zero width or height.", id
				);
			})?;
		if mip_count > available_mip_count {
			log::error!(
				"Visibility texture mip metadata is invalid for {}: declared {}, available {}. The most likely cause is that the baked mip count does not match the image dimensions.",
				id,
				mip_count,
				available_mip_count
			);
			return Err(());
		}
		let mut layouts = SmallVec::<[TextureUploadLayout; 16]>::new();
		let mut upload_byte_count = 0usize;
		for level in 0..mip_count {
			let mip_extent = texture_mip_extent(extent, level);
			let mut layout = texture_upload_layout(format, mip_extent, 1).ok_or(())?;
			layout.offset = upload_byte_count;
			upload_byte_count = upload_byte_count.checked_add(layout.padded_size).ok_or(())?;
			layouts.push(layout);
		}
		let mut staging = upload_staging.allocate(upload_byte_count, 256).await.ok_or_else(|| {
			log::error!(
				"Visibility texture exceeds the GPU upload arena. The most likely cause is that its complete padded mip chain is larger than the configured upload capacity."
			);
		})?;

		if mip_count == 1 {
			let layout = layouts[0];
			let loaded = reference
				.load((&mut staging.bytes_mut()[..layout.compact_size]).into())
				.await
				.map_err(|_| {
					log::error!(
						"Visibility texture load failed for {}. The most likely cause is that the texture payload could not be read from storage.", id
					);
				})?;
			if loaded.buffer().is_none() {
				log::error!(
					"Visibility texture load target is not CPU-readable for {}. The most likely cause is that the image resource did not load into a byte buffer.", id
				);
				return Err(());
			}
		} else {
			// Named reads keep every offline-generated level aligned with its independently padded GPU upload region.
			let mip_stream_names: [MipStreamName; u32::BITS as usize] =
				std::array::from_fn(|level| MipStreamName::new(level as u32));
			let mut streams = Vec::with_capacity(mip_count as usize);
			let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
			for (name, layout) in mip_stream_names.iter().zip(&layouts) {
				let region = allocator.take(layout.padded_size);
				streams.push(resource_management::stream::StreamMut::new(
					name.as_str(),
					&mut region[..layout.compact_size],
				));
			}
			let loaded = reference.load(streams.into()).await.map_err(|_| {
				log::error!(
					"Visibility texture mip load failed for {}. The most likely cause is that the baked image payload is missing one or more named mip streams.", id
				);
			})?;
			if !matches!(loaded, ReadTargets::Streams(_)) {
				return Err(());
			}
		}
		for layout in &layouts {
			let range = layout.offset..layout.offset + layout.padded_size;
			pack_texture_rows_in_place(&mut staging.bytes_mut()[range], layout);
		}
		let upload = TextureUpload { staging, layouts };

		Ok(PreparedTexture {
			key,
			index,
			name: reference.id().to_string(),
			format,
			extent,
			mip_count,
			upload,
		})
	}

	/// Creates detached texture objects after CPU preparation, then exposes them for render-thread interning.
	fn adopt_prepared_texture(&mut self, texture: PreparedTexture) {
		let PreparedTexture {
			key,
			index,
			name,
			format,
			extent,
			mip_count,
			upload,
		} = texture;
		let Some(device) = self.resource_factory.as_mut() else {
			log::error!(
				"Visibility texture creation failed for {}. The most likely cause is that material pipeline creation was configured without a factory.",
				name
			);
			self.send_completion(VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Texture(key),
			});
			return;
		};

		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(&name)
				.extent(extent)
				.mip_levels(mip_count)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);

		let sampler = device.build_sampler(default_material_sampler_builder().max_lod((mip_count - 1) as f32));

		self.send_completion(VisibilityResourceCompletion::ImageReady {
			index,
			image,
			sampler,
			upload,
		});
	}

	/// Starts one environment's complete CPU preparation without holding the transfer scheduler.
	fn request_environment_preparation(&self, id: String) {
		let resource_manager = self.resource_manager.clone();
		let upload_staging = self.upload_staging.clone();
		let failure_id = id.clone();
		self.spawn_preparation(async move {
			match Self::prepare_environment(resource_manager, upload_staging, id).await {
				Ok(environment) => VisibilityTransferCommand::EnvironmentPrepared { environment },
				Err(()) => VisibilityTransferCommand::PreparationFailed {
					key: VisibilityResourceKey::Environment(failure_id),
				},
			}
		});
	}

	/// Loads the diffuse and roughness-prefiltered streams into owned upload data.
	async fn prepare_environment(
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
		id: String,
	) -> Result<PreparedEnvironment, ()> {
		let mut reference: Reference<ResourceImage> = resource_manager.request(&id).await.map_err(|_| {
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

		let mut diffuse_upload = texture_upload_layout(diffuse_format, diffuse_extent, 6).ok_or(())?;
		let mut specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| texture_upload_layout(specular_format, specular_extents[level], 6).unwrap());
		let mut upload_byte_count = 0usize;
		diffuse_upload.offset = upload_byte_count;
		upload_byte_count = upload_byte_count.checked_add(diffuse_upload.padded_size).ok_or(())?;
		for upload in &mut specular_uploads {
			upload.offset = upload_byte_count;
			upload_byte_count = upload_byte_count.checked_add(upload.padded_size).ok_or(())?;
		}
		let mut staging = upload_staging.allocate(upload_byte_count, 256).await.ok_or_else(|| {
			log::error!(
				"Visibility environment exceeds the GPU upload arena. The most likely cause is that its complete padded IBL data is larger than the configured upload capacity."
			);
		})?;
		let specular_stream_names: [String; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|level| {
			resource_management::resources::image::ibl_prefiltered_specular_stream_name(level as u32)
		});

		// A single stream read keeps the parent image and all of its baked lighting subresources atomic.
		let mut streams = Vec::with_capacity(1 + IBL_SPECULAR_LEVEL_COUNT);
		let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
		let diffuse_region = allocator.take(diffuse_upload.padded_size);
		streams.push(resource_management::stream::StreamMut::new(
			resource_management::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
			&mut diffuse_region[..diffuse_upload.compact_size],
		));
		for (name, upload) in specular_stream_names.iter().zip(specular_uploads.iter()) {
			let region = allocator.take(upload.padded_size);
			streams.push(resource_management::stream::StreamMut::new(
				name,
				&mut region[..upload.compact_size],
			));
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
		for upload in std::iter::once(&diffuse_upload).chain(specular_uploads.iter()) {
			let range = upload.offset..upload.offset + upload.padded_size;
			pack_texture_rows_in_place(&mut staging.bytes_mut()[range], upload);
		}

		Ok(PreparedEnvironment {
			id,
			diffuse_format,
			diffuse_extent,
			specular_format,
			specular_extent: specular_extents[0],
			staging,
			diffuse_upload,
			specular_uploads,
		})
	}

	/// Creates detached environment objects after every baked stream is prepared.
	fn adopt_prepared_environment(&mut self, environment: PreparedEnvironment) {
		let PreparedEnvironment {
			id,
			diffuse_format,
			diffuse_extent,
			specular_format,
			specular_extent,
			staging,
			diffuse_upload,
			specular_uploads,
		} = environment;
		let Some(device) = self.resource_factory.as_mut() else {
			log::error!(
				"Visibility environment creation failed for {}. The most likely cause is that the resource worker was configured without a GPU factory.",
				id
			);
			self.send_completion(VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Environment(id),
			});
			return;
		};
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
				.extent(specular_extent)
				.cube_compatible()
				.mip_levels(IBL_SPECULAR_LEVEL_COUNT as u32)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let sampler = device.build_sampler(default_material_sampler_builder().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));

		self.send_completion(VisibilityResourceCompletion::EnvironmentReady {
			id,
			environment: FactoryEnvironment {
				diffuse_image,
				specular_image,
				sampler,
				staging,
				diffuse_upload,
				specular_uploads,
			},
		});
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

	/// Reserves a material slot and immediately starts its independent dependency preparation.
	fn request_material(&mut self, material_id: &str) -> u32 {
		let (index, inserted) = self.reserve_material_slot(material_id);
		if inserted {
			let id = material_id.to_string();
			let Some(config) = self.material_pipeline_config.as_ref() else {
				log::error!(
					"Visibility material pipeline configuration is unavailable for {}. The most likely cause is that the render pipeline manager did not configure the resource worker before requesting meshes.",
					id
				);
				self.send_completion(VisibilityResourceCompletion::Failed {
					key: VisibilityResourceKey::Material(id),
				});
				return index;
			};
			let push_constant_ranges = config.push_constant_ranges.clone();
			let pipeline_manager = config.pipeline_manager.clone();
			let resource_manager = self.resource_manager.clone();
			let commands = self.commands.clone();

			resource_management::r#async::spawn(async move {
				let result = async {
					let mut reference: Reference<ResourceVariant> = resource_manager.request(&id).await.map_err(|_| {
						log::error!(
							"Visibility material variant request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
							id
						);
					})?;
					let variant = reference.resource_mut();
					let alpha_mode = variant.alpha_mode.clone();
					let specialization_map_entries = variant
						.variables
						.iter()
						.enumerate()
						.filter_map(|(index, variable)| match &variable.value {
							Value::Scalar(value) => ghi::pipelines::SpecializationMapEntry::new(
								index as u32,
								"f32".to_string(),
								*value,
							)
							.into(),
							Value::Vector3(value) => ghi::pipelines::SpecializationMapEntry::new(
								index as u32,
								"vec3f".to_string(),
								*value,
							)
							.into(),
							Value::Vector4(value) => ghi::pipelines::SpecializationMapEntry::new(
								index as u32,
								"vec4f".to_string(),
								*value,
							)
							.into(),
							Value::Image(_) => None,
						})
						.collect::<Vec<_>>();
					let texture_keys = variant
						.variables
						.iter()
						.map(|parameter| match parameter.value {
							Value::Image(ref image) => Some(VisibilityTextureKey::new(image.id())),
							_ => None,
						})
						.collect::<Vec<_>>();
					let material = variant.material.resource_mut();
					let coverage = material.coverage;
					if material.model.name != "Visibility" || material.model.pass != "MaterialEvaluation" {
						log::error!(
							"Unsupported visibility material model for {}. The most likely cause is that this material targets a different render model or pass.",
							id
						);
						return Err(());
					}

					let shader_resource_id = material.shaders().first().map(|shader| shader.id().to_string()).ok_or_else(|| {
						log::error!(
							"Visibility material shader is missing for {}. The most likely cause is that the material was baked without a compute shader.",
							id
						);
					})?;
					// Pipeline submission is independent of GPU transfer availability. Send it
					// directly to the existing compiler workers as soon as the variant is known.
					let pipeline = pipeline_manager.request_specialized_compute_pipeline(
						crate::rendering::pipeline_compilation::SpecializedComputePipelineRequest::new(
							id.clone(),
							shader_resource_id,
							push_constant_ranges,
							specialization_map_entries,
						),
					);
					Ok(VisibilityTransferCommand::MaterialPrepared {
						id: id.clone(),
						index,
						alpha_mode,
						coverage,
						texture_keys,
						pipeline,
					})
				}
				.await;

				let command = match result {
					Ok(command) => command,
					Err(()) => VisibilityTransferCommand::PreparationFailed {
						key: VisibilityResourceKey::Material(id),
					},
				};
				if commands.send(command).is_err() {
					log::error!(
						"Visibility material preparation completion failed. The most likely cause is that the transfer worker stopped receiving resource work."
					);
				}
			})
			.detach();
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
	pending_uploads: VecDeque<PreparedUpload>,
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
		self.send(VisibilityTransferCommand::UploadPrepared(PreparedUpload::Texture {
			index,
			image,
			sampler,
			upload,
		}));
	}

	/// Enqueues every image in one environment as one transfer-frame completion.
	pub(crate) fn enqueue_environment_upload(&self, upload: PendingEnvironmentUpload) {
		self.send(VisibilityTransferCommand::UploadPrepared(PreparedUpload::Environment(upload)));
	}
}

impl VisibilityPipelineResourceManagerWorker {
	/// Records one fully prepared mesh without resource I/O or dependency waits.
	fn record_resource_mesh(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
		mesh: &PreparedGpuMesh,
		material_indices: Vec<u32>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	) -> Result<crate::rendering::pipelines::visibility::pipeline_manager::MeshData, ()> {
		if !Self::resource_mesh_metadata_is_valid(mesh, &material_indices, &primitive_skins, skin_bindings.len()) {
			return Err(());
		}
		let mesh = self
			.gpu_vertex_data_manager
			.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, staging_data_buffer, mesh)
			.ok_or(())?;
		Ok(Self::convert_resource_mesh_data(
			mesh,
			material_indices,
			primitive_skins,
			skin_bindings,
			skeleton_node_count,
		))
	}

	/// Rejects render metadata before transfer recording can consume GPU capacity.
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

	/// Combines uploaded resource geometry with dependency slots reserved during metadata discovery.
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
				let skin = match skin_index {
					Some(skin_index) => Some(
						skin_bindings
							.get(skin_index as usize)
							.expect("Visibility skin indices were validated before transfer recording.")
							.clone(),
					),
					None => None,
				};

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
			// Observe every ready preparation before opening the next transfer frame so
			// unrelated resources share the earliest batch that has room for them.
			let Some(drained_command_count) = self.drain_ready_commands(256) else {
				break;
			};

			if self.has_active_transfer_work() {
				if self
					.advance_transfer_queue(
						&mut transfer_queue,
						transfer_finished_synchronizer,
						transfer_command_buffer,
						upload_buffer,
						&mut started_frame_count,
					)
					.is_none()
				{
					break;
				}
			}

			if drained_command_count > 0 {
				crate::core::async_runtime::yield_now().await;
			} else if self.has_active_transfer_work() {
				// Submitted GPU work needs periodic queue progress even when no new
				// resource has finished CPU preparation.
				compio::time::sleep(ACTIVE_TRANSFER_POLL_INTERVAL).await;
			} else {
				let Ok(command) = self.commands.recv().await else {
					break;
				};
				if !self.handle_command(command) {
					break;
				}
			}
		}
	}

	/// Advances one transfer frame and records all upload work already prepared by resource commands.
	fn advance_transfer_queue(
		&mut self,
		transfer_queue: &mut ghi::implementation::queue::Queue,
		transfer_finished_synchronizer: ghi::SynchronizerHandle,
		transfer_command_buffer: ghi::CommandBufferHandle,
		upload_buffer: ghi::BufferHandle<[u8; ASYNC_UPLOAD_BUFFER_BYTE_COUNT]>,
		started_frame_count: &mut u64,
	) -> Option<()> {
		let started_frame = transfer_queue.start_frame(*started_frame_count as _, transfer_finished_synchronizer);
		if let Some(completed_frame) = started_frame.completed_frame {
			self.signal_completed_frame(completed_frame);
		}

		// Frame acquisition can wait for an in-flight sequence. Adopt resources that
		// became ready during that wait before deciding what belongs in this batch.
		if self.drain_ready_commands(256).is_none() {
			return None;
		}

		if !self.has_pending_upload_work() {
			*started_frame_count += 1;
			return Some(());
		}

		let mut frame = started_frame.frame;
		let frame_key = frame.key();
		let mut transfer_recording = frame.create_command_buffer_recording_without_implicit_sync(transfer_command_buffer);
		let prepared_uploads = self.prepare_uploads(&mut transfer_recording, upload_buffer.into());

		if prepared_uploads.recorded_work {
			transfer_recording.execute(transfer_finished_synchronizer);
		} else {
			drop(transfer_recording);
		}

		self.track_submitted_uploads(frame_key, prepared_uploads.completions, prepared_uploads.leases);
		*started_frame_count += 1;
		Some(())
	}

	/// Adopts a bounded set of ready commands without waiting for more preparation work.
	fn drain_ready_commands(&mut self, max_commands: usize) -> Option<usize> {
		let mut count = 0usize;
		while count < max_commands {
			match self.commands.try_recv() {
				Ok(Some(command)) => {
					count += 1;
					if !self.handle_command(command) {
						return None;
					}
				}
				Ok(None) => break,
				Err(_) => return None,
			}
		}

		Some(count)
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
	pub(crate) fn track_submitted_uploads(
		&mut self,
		frame_key: ghi::FrameKey,
		completions: CompletionList,
		leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
	) {
		if completions.is_empty() {
			return;
		}

		self.submitted_uploads.push_back(SubmittedUploadBatch {
			frame_key,
			completions,
			_leases: leases,
		});
	}

	/// Records every currently fitting upload into the transfer command buffer.
	fn prepare_uploads(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> TransferUploadPrepareResult {
		self.record_uploads(transfer, staging_data_buffer)
	}

	/// Reports whether upload queues contain work that needs GPU transfer recording.
	fn has_pending_upload_work(&self) -> bool {
		!self.pending_uploads.is_empty()
	}

	/// Reports whether the queue must keep advancing submitted or pending transfers.
	fn has_active_transfer_work(&self) -> bool {
		self.has_pending_upload_work() || !self.submitted_uploads.is_empty()
	}

	/// Moves one request or preparation completion into worker-owned state without waiting.
	fn handle_command(&mut self, command: VisibilityTransferCommand) -> bool {
		match command {
			VisibilityTransferCommand::RequestMesh { key, source } => {
				self.resource_manager.request_mesh_preparation(key, source);
			}
			VisibilityTransferCommand::ResourceMeshLoaded { key, resource } => {
				self.resource_manager.prepare_loaded_resource_mesh(key, resource);
			}
			VisibilityTransferCommand::GeneratedMeshLoaded { key, generator } => {
				self.resource_manager.prepare_loaded_generated_mesh(key, generator);
			}
			VisibilityTransferCommand::UploadPrepared(upload) => {
				self.pending_uploads.push_back(upload);
			}
			VisibilityTransferCommand::MaterialPrepared {
				id,
				index,
				alpha_mode,
				coverage,
				texture_keys,
				pipeline,
			} => {
				self.resource_manager
					.adopt_prepared_material(id, index, alpha_mode, coverage, texture_keys, pipeline);
			}
			VisibilityTransferCommand::RequestImage { key } => {
				let (index, inserted) = self.resource_manager.reserve_texture_slot(key.as_str());
				if inserted {
					self.resource_manager.request_image_preparation(key, index);
				}
			}
			VisibilityTransferCommand::TexturePrepared { texture } => {
				self.resource_manager.adopt_prepared_texture(texture);
			}
			VisibilityTransferCommand::RequestEnvironment { id } => {
				self.resource_manager.request_environment_preparation(id);
			}
			VisibilityTransferCommand::EnvironmentPrepared { environment } => {
				self.resource_manager.adopt_prepared_environment(environment);
			}
			VisibilityTransferCommand::ConfigureMaterialPipeline(config) => {
				self.resource_manager.configure_material_pipeline(config);
			}
			VisibilityTransferCommand::PreparationFailed { key } => {
				self.resource_manager
					.send_completion(VisibilityResourceCompletion::Failed { key });
			}
			VisibilityTransferCommand::Shutdown => return false,
		}

		true
	}

	/// Records every ready lease and retains it until the submitted transfer frame completes.
	fn record_uploads(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording<'_>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> TransferUploadPrepareResult {
		let mut recorded_work = false;
		let mut completions = CompletionList::new();
		let mut leases = SmallVec::new();
		while let Some(upload) = self.pending_uploads.pop_front() {
			match upload {
				PreparedUpload::ResourceMesh {
					key,
					mesh: prepared_mesh,
					material_indices,
					primitive_skins,
					skin_bindings,
					skeleton_node_count,
				} => {
					match self.record_resource_mesh(
						transfer,
						staging_data_buffer,
						&prepared_mesh,
						material_indices,
						primitive_skins,
						skin_bindings,
						skeleton_node_count,
					) {
						Ok(mesh) => {
							let meshlet_count = mesh.primitives.iter().map(|primitive| primitive.meshlet_count).sum::<u32>();
							log::debug!(
								"Visibility mesh created: key={}, source={}, primitives={}, meshlets={}, vertex_offset={}, primitive_offset={}, triangle_offset={}, meshlet_offset={}",
								key,
								"resource",
								mesh.primitives.len(),
								meshlet_count,
								mesh.vertex_offset,
								mesh.primitive_offset,
								mesh.triangle_offset,
								mesh.meshlet_offset,
							);
							completions.push(VisibilityResourceCompletion::MeshReady { key, mesh });
							leases.push(prepared_mesh.into_staging());
							recorded_work = true;
						}
						Err(()) => self.resource_manager.send_completion(VisibilityResourceCompletion::Failed {
							key: VisibilityResourceKey::Mesh(key),
						}),
					}
				}
				PreparedUpload::GeneratedMesh {
					key,
					mesh: prepared_mesh,
					material_index,
				} => {
					let result = self
						.gpu_vertex_data_manager
						.write_prepared_gpu_mesh_data_and_return_mesh_object(transfer, staging_data_buffer, &prepared_mesh)
						.map(|mesh| Self::convert_generated_mesh_data(mesh, material_index));
					match result {
						Some(mesh) => {
							completions.push(VisibilityResourceCompletion::MeshReady { key, mesh });
							leases.push(prepared_mesh.into_staging());
							recorded_work = true;
						}
						None => self.resource_manager.send_completion(VisibilityResourceCompletion::Failed {
							key: VisibilityResourceKey::Mesh(key),
						}),
					}
				}
				PreparedUpload::Texture {
					index,
					image,
					sampler,
					upload,
				} => {
					let copies = upload
						.layouts
						.iter()
						.enumerate()
						.map(|(level, layout)| {
							staged_texture_copy(staging_data_buffer, upload.staging.offset(), image, layout, level as u32)
						})
						.collect::<SmallVec<[ghi::BufferImageCopyDescriptor; 16]>>();
					transfer.copy_buffer_to_images(&copies);
					completions.push(VisibilityResourceCompletion::TextureUploadReady { index, image, sampler });
					leases.push(upload.staging);
					recorded_work = true;
				}
				PreparedUpload::Environment(upload) => {
					let mut copies = SmallVec::<[ghi::BufferImageCopyDescriptor; 9]>::new();
					copies.push(staged_texture_copy(
						staging_data_buffer,
						upload.staging.offset(),
						upload.diffuse_image,
						&upload.diffuse_upload,
						0,
					));
					for (mip_level, mip) in upload.specular_uploads.iter().enumerate() {
						copies.push(staged_texture_copy(
							staging_data_buffer,
							upload.staging.offset(),
							upload.specular_image,
							mip,
							mip_level as u32,
						));
					}
					transfer.copy_buffer_to_images(&copies);
					completions.push(VisibilityResourceCompletion::EnvironmentUploadReady {
						id: upload.id,
						diffuse_image: upload.diffuse_image,
						specular_image: upload.specular_image,
						sampler: upload.sampler,
					});
					leases.push(upload.staging);
					recorded_work = true;
				}
			}
		}

		TransferUploadPrepareResult {
			recorded_work,
			completions,
			leases,
		}
	}
}

/// The `TransferUploadPrepareResult` struct tracks transfer work and resources handled by a recording.
pub(crate) struct TransferUploadPrepareResult {
	pub(crate) recorded_work: bool,
	pub(crate) completions: CompletionList,
	leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
}

/// The `SubmittedUploadBatch` struct holds resource completions until a transfer frame is complete.
struct SubmittedUploadBatch {
	frame_key: ghi::FrameKey,
	completions: CompletionList,
	_leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
}

/// The `PreparedUpload` enum owns everything needed to record one independently ready GPU upload.
enum PreparedUpload {
	ResourceMesh {
		key: VisibilityMeshKey,
		mesh: PreparedGpuMesh,
		material_indices: Vec<u32>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	},
	GeneratedMesh {
		key: VisibilityMeshKey,
		mesh: PreparedGpuMesh,
		material_index: u32,
	},
	Texture {
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
	},
	Environment(PendingEnvironmentUpload),
}

/// The `VisibilityResourceCompletion` enum describes resource work that is ready for render-thread adoption.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		key: VisibilityMeshKey,
		mesh: crate::rendering::pipelines::visibility::pipeline_manager::MeshData,
	},
	MaterialReady {
		id: String,
		index: u32,
		pipeline: crate::rendering::PipelineRef,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		textures: Vec<Option<(String, u32)>>,
	},
	ImageReady {
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
	ResourceMeshLoaded {
		key: VisibilityMeshKey,
		resource: Reference<ResourceMesh>,
	},
	GeneratedMeshLoaded {
		key: VisibilityMeshKey,
		generator: Arc<dyn crate::rendering::mesh::generator::MeshGenerator>,
	},
	MaterialPrepared {
		id: String,
		index: u32,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		texture_keys: Vec<Option<VisibilityTextureKey>>,
		pipeline: crate::rendering::PipelineRef,
	},
	RequestImage {
		key: VisibilityTextureKey,
	},
	TexturePrepared {
		texture: PreparedTexture,
	},
	RequestEnvironment {
		id: String,
	},
	EnvironmentPrepared {
		environment: PreparedEnvironment,
	},
	ConfigureMaterialPipeline(MaterialPipelineConfig),
	UploadPrepared(PreparedUpload),
	PreparationFailed {
		key: VisibilityResourceKey,
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

/// The `PreparedTexture` struct keeps CPU-ready texture data independent from GPU object creation.
struct PreparedTexture {
	key: VisibilityTextureKey,
	index: u32,
	name: String,
	format: ghi::Formats,
	extent: Extent,
	mip_count: u32,
	upload: TextureUpload,
}

/// The `PreparedEnvironment` struct keeps every CPU-ready IBL stream independent from GPU object creation.
struct PreparedEnvironment {
	id: String,
	diffuse_format: ghi::Formats,
	diffuse_extent: Extent,
	specular_format: ghi::Formats,
	specular_extent: Extent,
	staging: super::upload_staging::StagingLease,
	diffuse_upload: TextureUploadLayout,
	specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
}

/// The `FactoryEnvironment` struct keeps one baked IBL set together until the render thread interns its GPU resources.
pub(crate) struct FactoryEnvironment {
	diffuse_image: ghi::implementation::factory::Image,
	specular_image: ghi::implementation::factory::Image,
	sampler: ghi::implementation::factory::Sampler,
	staging: super::upload_staging::StagingLease,
	diffuse_upload: TextureUploadLayout,
	specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
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
			staging: self.staging,
			diffuse_image,
			diffuse_upload: self.diffuse_upload,
			specular_image,
			specular_uploads,
			sampler,
		}
	}
}

/// The `PendingEnvironmentUpload` struct keeps a complete environment on one transfer frame and completion boundary.
pub(crate) struct PendingEnvironmentUpload {
	id: String,
	diffuse_image: ghi::BaseImageHandle,
	staging: super::upload_staging::StagingLease,
	diffuse_upload: TextureUploadLayout,
	specular_image: ghi::BaseImageHandle,
	specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
	sampler: ghi::SamplerHandle,
}

/// The `MaterialPipelineConfig` struct connects material specialization to shared pipeline and resource factories.
pub(crate) struct MaterialPipelineConfig {
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	resource_factory: Option<ghi::implementation::Factory>,
	pipeline_manager: crate::rendering::PipelineManagerClient,
}

impl MaterialPipelineConfig {
	/// Creates a material pipeline configuration used by the visibility resource worker.
	pub(crate) fn new(
		push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
		resource_factory: Option<ghi::implementation::Factory>,
		pipeline_manager: crate::rendering::PipelineManagerClient,
	) -> Self {
		Self {
			push_constant_ranges,
			resource_factory,
			pipeline_manager,
		}
	}
}

/// The `TextureUpload` struct carries row-padded texture bytes until the transfer queue copies them.
pub(crate) struct TextureUpload {
	staging: super::upload_staging::StagingLease,
	layouts: SmallVec<[TextureUploadLayout; 16]>,
}

struct MipStreamName {
	bytes: [u8; 16],
	len: usize,
}

impl MipStreamName {
	/// Formats one bounded mip stream identifier into inline storage.
	fn new(level: u32) -> Self {
		let mut bytes = [0_u8; 16];
		bytes[..4].copy_from_slice(b"mip[");
		let mut digits = [0_u8; 10];
		let mut value = level;
		let mut digit_count = 0usize;
		loop {
			digits[digit_count] = b'0' + (value % 10) as u8;
			digit_count += 1;
			value /= 10;
			if value == 0 {
				break;
			}
		}
		for index in 0..digit_count {
			bytes[4 + index] = digits[digit_count - index - 1];
		}
		let len = 5 + digit_count;
		bytes[len - 1] = b']';
		Self { bytes, len }
	}

	fn as_str(&self) -> &str {
		std::str::from_utf8(&self.bytes[..self.len]).expect("Mip stream names contain only ASCII bytes.")
	}
}

#[derive(Clone, Copy)]
struct TextureUploadLayout {
	offset: usize,
	compact_bytes_per_row: usize,
	row_count: usize,
	compact_bytes_per_image: usize,
	compact_size: usize,
	source_bytes_per_row: usize,
	source_bytes_per_image: usize,
	padded_size: usize,
}

/// Computes the independently uploaded extent for one material texture mip level.
fn texture_mip_extent(base_extent: Extent, level: u32) -> Extent {
	Extent::new(
		(base_extent.width() >> level).max(1),
		(base_extent.height() >> level).max(1),
		base_extent.depth().max(1),
	)
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

/// Builds one GPU image-copy descriptor that reads directly from a completed staging lease.
fn staged_texture_copy(
	staging_data_buffer: ghi::BaseBufferHandle,
	staging_offset: usize,
	image: ghi::BaseImageHandle,
	upload: &TextureUploadLayout,
	mip_level: u32,
) -> ghi::BufferImageCopyDescriptor {
	ghi::BufferImageCopyDescriptor::new(
		staging_data_buffer,
		staging_offset + upload.offset,
		upload.source_bytes_per_row,
		upload.source_bytes_per_image,
		image,
		mip_level,
	)
}

/// Computes the compact load target and row-padded GPU copy layout for one texture lease.
fn texture_upload_layout(format: ghi::Formats, extent: Extent, layer_count: usize) -> Option<TextureUploadLayout> {
	let (source_bytes_per_row, row_count, compact_bytes_per_image) =
		format.compact_copy_layout(extent.width().max(1), extent.height().max(1));
	let compact_size = compact_bytes_per_image.checked_mul(layer_count)?;
	let padded_bytes_per_row = source_bytes_per_row.next_multiple_of(256);
	let source_bytes_per_image = padded_bytes_per_row.checked_mul(row_count)?;
	let padded_size = source_bytes_per_image.checked_mul(layer_count)?;
	assert_eq!(
		padded_bytes_per_row % 256,
		0,
		"Texture upload row pitch alignment mismatch. The most likely cause is that the Metal upload layout was built without 256-byte row alignment. format={format:?}, extent={extent:?}, source_bytes_per_row={source_bytes_per_row}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	assert!(
		source_bytes_per_image >= compact_bytes_per_image,
		"Texture upload padded image is smaller than compact image. The most likely cause is an invalid row count or row pitch. format={format:?}, extent={extent:?}, compact_bytes_per_image={compact_bytes_per_image}, source_bytes_per_image={source_bytes_per_image}, row_count={row_count}, padded_bytes_per_row={padded_bytes_per_row}"
	);
	Some(TextureUploadLayout {
		offset: 0,
		compact_bytes_per_row: source_bytes_per_row,
		row_count,
		compact_bytes_per_image,
		compact_size,
		source_bytes_per_row: padded_bytes_per_row,
		source_bytes_per_image,
		padded_size,
	})
}

/// Expands compact rows backward inside their final leased range, avoiding a second CPU allocation or full-resource copy.
fn pack_texture_rows_in_place(bytes: &mut [u8], layout: &TextureUploadLayout) {
	assert_eq!(bytes.len(), layout.padded_size);
	let layer_count = layout.compact_size / layout.compact_bytes_per_image;
	for layer in (0..layer_count).rev() {
		for row in (0..layout.row_count).rev() {
			let source = layer * layout.compact_bytes_per_image + row * layout.compact_bytes_per_row;
			let destination = layer * layout.source_bytes_per_image + row * layout.source_bytes_per_row;
			bytes.copy_within(source..source + layout.compact_bytes_per_row, destination);
		}
	}
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

#[cfg(test)]
mod tests {
	use super::*;

	fn staged_texture_bytes(
		format: ghi::Formats,
		extent: Extent,
		layer_count: usize,
		source: &[u8],
	) -> (Vec<u8>, TextureUploadLayout) {
		let layout = texture_upload_layout(format, extent, layer_count).expect("texture layout");
		assert_eq!(source.len(), layout.compact_size);
		let mut bytes = vec![0u8; layout.padded_size];
		bytes[..source.len()].copy_from_slice(source);
		pack_texture_rows_in_place(&mut bytes, &layout);
		(bytes, layout)
	}

	#[test]
	fn resource_mesh_metadata_is_rejected_before_transfer_recording() {
		let bytes = Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice());
		let staging = super::super::upload_staging::UploadStagingArena::new(bytes);
		let executor = resource_management::r#async::Executor::new().expect("mesh metadata test executor");
		let mesh = executor
			.block_on(PreparedGpuMesh::prepare_generated_mesh(
				&crate::rendering::mesh::generator::BoxMeshGenerator::new(),
				staging,
			))
			.expect("generated mesh preparation");
		let mut material_indices = Vec::new();
		let mut primitive_skins = Vec::new();

		assert!(!VisibilityPipelineResourceManagerWorker::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
		));

		material_indices.push(0);
		primitive_skins.push(None);
		assert!(VisibilityPipelineResourceManagerWorker::resource_mesh_metadata_is_valid(
			&mesh,
			&material_indices,
			&primitive_skins,
			0,
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

		let (data, upload) = staged_texture_bytes(ghi::Formats::BC7, extent, 1, &source);

		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 256 * 2);
		assert_eq!(&data[0..compact_row], &source[0..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..compact_row * 2]);

		let (zero_data, zero_extent) =
			staged_texture_bytes(ghi::Formats::RGBA8UNORM, Extent::rectangle(0, 0), 1, &[1, 2, 3, 4]);
		assert_eq!(zero_extent.source_bytes_per_row, 256);
		assert_eq!(zero_extent.source_bytes_per_image, 256);
		assert_eq!(&zero_data[..4], &[1, 2, 3, 4]);
	}

	/// Ensures half-float HDR pixels reach the transfer buffer without normalization or channel conversion.
	#[test]
	fn texture_upload_preserves_rgba16f_environment_rows() {
		let extent = Extent::rectangle(2, 2);
		let compact_row = 2 * 8;
		let source = (0..compact_row * 2).map(|value| value as u8).collect::<Vec<_>>();

		let (data, upload) = staged_texture_bytes(ghi::Formats::RGBA16F, extent, 1, &source);

		assert_eq!(
			resource_image_format_to_ghi(resource_management::types::Formats::RGBA16F),
			ghi::Formats::RGBA16F
		);
		assert_eq!(upload.source_bytes_per_row, 256);
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(&data[..compact_row], &source[..compact_row]);
		assert_eq!(&data[256..256 + compact_row], &source[compact_row..]);
	}

	#[test]
	fn cubemap_upload_preserves_every_face_and_image_pitch() {
		let extent = Extent::square(2);
		let compact_face_size = 2 * 2 * 8;
		let source = (0..compact_face_size * 6).map(|value| value as u8).collect::<Vec<_>>();
		let (data, upload) = staged_texture_bytes(ghi::Formats::RGBA16F, extent, 6, &source);
		assert_eq!(upload.source_bytes_per_image, 512);
		assert_eq!(data.len(), 512 * 6);
		for face in 0..6 {
			for row in 0..2 {
				let source_start = face * compact_face_size + row * 16;
				let upload_start = face * 512 + row * 256;
				assert_eq!(
					&data[upload_start..upload_start + 16],
					&source[source_start..source_start + 16]
				);
			}
		}
	}

	#[test]
	fn environment_specular_streams_form_one_gpu_mip_chain() {
		let extents: [Extent; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| environment_mip_extent([256, 256, 1], level as u32));

		assert_eq!(extents[0], Extent::new(256, 256, 1));
		assert_eq!(extents[1], Extent::new(128, 128, 1));
		assert_eq!(extents[7], Extent::new(2, 2, 1));
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
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resource::ReadTargets;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::{Value, Variant as ResourceVariant};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use resource_management::types::AlphaMode;
use resource_management::Reference;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::Extent;

use crate::core::EntityHandle;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{
	GPUVertexDataManager, MeshData as GpuMeshData, PreparedGpuMesh,
};
use crate::rendering::pipelines::visibility::{MAX_BINDLESS_TEXTURES, MAX_MATERIALS};
use crate::rendering::renderable::mesh::MeshSource;
use crate::resource_management::{self};
