use super::*;

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
	pub(super) fn request_mesh_preparation(&self, key: VisibilityMeshKey, source: MeshSource) {
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
	pub(super) fn prepare_loaded_resource_mesh(&mut self, key: VisibilityMeshKey, resource: Reference<ResourceMesh>) {
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
	pub(super) fn prepare_loaded_generated_mesh(
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
	pub(super) fn send_completion(&self, completion: VisibilityResourceCompletion) {
		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility resource completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Adopts a requested pipeline reference and starts every texture dependency independently.
	pub(super) fn adopt_prepared_material(
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
	pub(super) fn request_image_preparation(&self, key: VisibilityTextureKey, index: u32) {
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
	pub(super) fn adopt_prepared_texture(&mut self, texture: PreparedTexture) {
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
	pub(super) fn request_environment_preparation(&self, id: String) {
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
	pub(super) fn adopt_prepared_environment(&mut self, environment: PreparedEnvironment) {
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
	pub(super) fn reserve_texture_slot(&mut self, texture_id: &str) -> (u32, bool) {
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
							push_constant_ranges,
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
