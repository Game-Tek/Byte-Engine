use super::*;

/// The `VisibilityPipelineResourceManager` struct owns asynchronous visibility resource workloads.
pub(crate) struct VisibilityPipelineResourceManager {
	/// Image resources used by material evaluation and local-light IES profiles.
	images: Vec<ResourceStates<(), ()>>,
	/// Mapping from shared material-texture or IES-profile resource ID to bindless image index.
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

/// Returns whether an image can safely provide the normalized Type C IES intensity-map contract.
fn photometric_profile_metadata_is_valid(
	image: &resource_management::resources::image::Image,
	photometry: &resource_management::resources::image::ImagePhotometry,
) -> bool {
	image.format == resource_management::types::Formats::R16F
		&& image.gamma == resource_management::types::Gamma::Linear
		&& image.extent[2] == 1
		&& image.mip_count == 1
		&& photometry.intensity_scale_candela.is_finite()
		&& photometry.intensity_scale_candela > 0.0
}

/// Copies one named range from a complete decoded payload into its upload destination.
fn copy_decoded_stream(
	decoded: &[u8],
	stream_descriptions: &[resource_management::StreamDescription],
	stream: &mut resource_management::stream::StreamMut<'_>,
	resource_id: &str,
) -> Result<(), ()> {
	let name = stream.name();
	let description = stream_descriptions
		.iter()
		.find(|description| description.name() == name)
		.ok_or_else(|| {
			log::error!(
				"Resource stream '{}' is missing for {}. The most likely cause is that the baked stream metadata does not match the image metadata.",
				name,
				resource_id
			);
		})?;
	if description.size() != stream.buffer().len() {
		log::error!(
			"Resource stream '{}' has the wrong size for {}. The most likely cause is that the baked stream size does not match the texture upload layout.",
			name,
			resource_id
		);
		return Err(());
	}

	let end = description.offset().checked_add(description.size()).ok_or_else(|| {
		log::error!(
			"Resource stream '{}' range overflowed for {}. The most likely cause is corrupt baked stream metadata.",
			name,
			resource_id
		);
	})?;
	let source = decoded.get(description.offset()..end).ok_or_else(|| {
		log::error!(
			"Resource stream '{}' is outside the decoded payload for {}. The most likely cause is corrupt compressed data or mismatched stream metadata.",
			name,
			resource_id
		);
	})?;
	stream.buffer_mut().copy_from_slice(source);
	Ok(())
}

/// Loads named image ranges directly or through one resource-owned decoded backing.
async fn load_image_streams<'a>(
	reference: &mut Reference<ResourceImage>,
	mut streams: SmallVec<[resource_management::stream::StreamMut<'a>; 16]>,
	resource_kind: &str,
	id: &str,
) -> Result<(), ()> {
	if reference.requires_cpu_decompression() {
		let loaded = reference
			.load(resource_management::resource::ReadTargetsMut::backing_storage())
			.await
			.map_err(|_| {
				log::error!(
					"Visibility {} decompression failed for {}. The most likely cause is corrupt compressed data or mismatched resource metadata.",
					resource_kind,
					id
				);
			})?;
		let stream_descriptions = reference.streams().ok_or_else(|| {
			log::error!(
				"Visibility {} stream metadata is missing for {}. The most likely cause is that the image was baked without its named payload ranges.",
				resource_kind,
				id
			);
		})?;
		let decoded = loaded.buffer().ok_or_else(|| {
			log::error!(
				"Visibility {} decompression returned no CPU buffer for {}. The most likely cause is that the resource used a GPU-only backing.",
				resource_kind,
				id
			);
		})?;
		for stream in &mut streams {
			copy_decoded_stream(decoded, stream_descriptions, stream, id)?;
		}
		return Ok(());
	}

	let loaded = reference.load(streams.into_vec().into()).await.map_err(|_| {
		log::error!(
			"Visibility {} stream load failed for {}. The most likely cause is that the baked image payload is missing one or more named ranges.",
			resource_kind,
			id
		);
	})?;
	if matches!(loaded, ReadTargets::Streams(_)) {
		Ok(())
	} else {
		log::error!(
			"Visibility {} load returned an unexpected target for {}. The most likely cause is that the resource reader ignored the requested named ranges.",
			resource_kind,
			id
		);
		Err(())
	}
}

/// Returns whether every mip occupies one contiguous complete decoded payload in level order.
fn texture_payload_is_compact(
	decoded_size: usize,
	stream_descriptions: Option<&[resource_management::StreamDescription]>,
	stream_names: &[MipStreamName],
	layouts: &[TextureUploadLayout],
) -> bool {
	let Some(stream_descriptions) = stream_descriptions else {
		return false;
	};
	let mut offset = 0usize;
	for (name, layout) in stream_names.iter().zip(layouts) {
		let Some(description) = stream_descriptions
			.iter()
			.find(|description| description.name() == name.as_str())
		else {
			return false;
		};
		if description.offset() != offset || description.size() != layout.compact_size {
			return false;
		}
		let Some(next_offset) = offset.checked_add(layout.compact_size) else {
			return false;
		};
		offset = next_offset;
	}
	offset == decoded_size
}

/// Moves a decoded compact mip chain backward into independently padded upload regions.
fn expand_compact_texture_levels(bytes: &mut [u8], decoded_size: usize, layouts: &[TextureUploadLayout]) -> Result<(), ()> {
	let mut source_end = decoded_size;
	for layout in layouts.iter().rev() {
		let source_start = source_end.checked_sub(layout.compact_size).ok_or(())?;
		let destination_end = layout.offset.checked_add(layout.compact_size).ok_or(())?;
		if layout.offset < source_start || destination_end > bytes.len() {
			return Err(());
		}
		if layout.offset != source_start {
			bytes.copy_within(source_start..source_end, layout.offset);
		}
		source_end = source_start;
	}
	(source_end == 0).then_some(()).ok_or(())
}

/// Loads one contiguous decoded texture range into caller-provided staging.
async fn load_texture_into(reference: &mut Reference<ResourceImage>, destination: &mut [u8], id: &str) -> Result<(), ()> {
	let expected_size = destination.len();
	let loaded = reference.load(destination.into()).await.map_err(|_| {
		log::error!(
			"Visibility texture load failed for {}. The most likely cause is that the requested texture bytes could not be decoded into staging.",
			id
		);
	})?;
	if loaded.buffer().is_none_or(|buffer| buffer.len() != expected_size) {
		log::error!(
			"Visibility texture load returned the wrong byte count for {}. The most likely cause is that the resource reader did not satisfy the requested payload range.",
			id
		);
		return Err(());
	}
	Ok(())
}

/// Loads a complete compact mip chain directly into its final staging lease.
async fn load_compact_texture_bytes(
	reference: &mut Reference<ResourceImage>,
	staging: &mut upload_staging::StagingLease,
	layouts: &[TextureUploadLayout],
	id: &str,
) -> Result<(), ()> {
	let decoded_size = reference.size;
	let destination = staging.bytes_mut().get_mut(..decoded_size).ok_or_else(|| {
		log::error!(
			"Visibility texture staging is too small for {}. The most likely cause is that the decoded mip chain exceeds its padded upload layout.",
			id
		);
	})?;
	load_texture_into(reference, destination, id).await?;
	expand_compact_texture_levels(staging.bytes_mut(), decoded_size, layouts).map_err(|_| {
		log::error!(
			"Visibility texture mip layout is invalid for {}. The most likely cause is that compact mip bytes cannot be expanded into their padded upload ranges.",
			id
		);
	})
}

/// Loads all texture levels while avoiding an intermediate decoded allocation for compact mip chains.
async fn load_texture_bytes(
	reference: &mut Reference<ResourceImage>,
	staging: &mut upload_staging::StagingLease,
	layouts: &[TextureUploadLayout],
	id: &str,
) -> Result<(), ()> {
	if let [layout] = layouts
		&& (!reference.requires_cpu_decompression() || reference.size == layout.compact_size)
	{
		return load_texture_into(reference, &mut staging.bytes_mut()[..layout.compact_size], id).await;
	}

	let stream_names: [MipStreamName; u32::BITS as usize] = std::array::from_fn(|level| MipStreamName::new(level as u32));
	if reference.requires_cpu_decompression()
		&& texture_payload_is_compact(reference.size, reference.streams(), &stream_names, layouts)
	{
		return load_compact_texture_bytes(reference, staging, layouts, id).await;
	}

	let mut streams = SmallVec::new();
	let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
	for (name, layout) in stream_names.iter().zip(layouts) {
		let region = allocator.take(layout.padded_size);
		streams.push(resource_management::stream::StreamMut::new(
			name.as_str(),
			&mut region[..layout.compact_size],
		));
	}
	load_image_streams(reference, streams, "texture", id).await
}

/// Loads all environment ranges through the read shape supported by the stored CPU encoding.
async fn load_environment_bytes(
	reference: &mut Reference<ResourceImage>,
	staging: &mut upload_staging::StagingLease,
	diffuse_upload: &TextureUploadLayout,
	specular_uploads: &[TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
	specular_stream_names: &[String; IBL_SPECULAR_LEVEL_COUNT],
	id: &str,
) -> Result<(), ()> {
	let mut streams = SmallVec::new();
	let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
	let diffuse_region = allocator.take(diffuse_upload.padded_size);
	streams.push(resource_management::stream::StreamMut::new(
		resource_management::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME,
		&mut diffuse_region[..diffuse_upload.compact_size],
	));
	for (name, upload) in specular_stream_names.iter().zip(specular_uploads) {
		let region = allocator.take(upload.padded_size);
		streams.push(resource_management::stream::StreamMut::new(
			name,
			&mut region[..upload.compact_size],
		));
	}
	load_image_streams(reference, streams, "environment", id).await
}

impl VisibilityPipelineResourceManager {
	pub(crate) fn spawn(
		context: &mut ghi::implementation::Context,
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
		staging_data_buffer: ghi::BaseBufferHandle,
	) -> (
		VisibilityPipelineResourceManagerClient,
		VisibilityPipelineResourceManagerWorker,
	) {
		let gpu_vertex_data_manager = GPUVertexDataManager::new(context);
		let (commands, command_receiver) = kanal::unbounded_async();
		let commands = commands.to_sync();
		let (work_completions, work_completion_receiver) = mpsc::channel();
		let (prepared_upload_sender, prepared_uploads) = mpsc::channel();
		let resource_manager = Self::new(resource_manager, commands.clone(), work_completions.clone(), upload_staging);

		(
			VisibilityPipelineResourceManagerClient {
				gpu_vertex_data_manager,
				commands: commands.clone(),
				completions: work_completion_receiver,
				upload_completions: CompletionList::new(),
				prepared_uploads,
				pending_uploads: VecDeque::new(),
				submitted_uploads: VecDeque::new(),
				staging_data_buffer,
			},
			VisibilityPipelineResourceManagerWorker {
				resource_manager,
				commands: command_receiver,
				prepared_uploads: prepared_upload_sender,
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
				MeshSource::Resource(id) => match resource_manager.request::<ResourceMesh>(id).await {
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

	/// Requests one image outside material dependency discovery, such as a light's IES profile.
	pub(super) fn request_image(&mut self, key: VisibilityTextureKey) {
		self.request_texture_dependency(key);
	}

	/// Starts one texture's CPU preparation without waiting for sibling textures or its material pipeline.
	pub(super) fn request_image_preparation(&self, key: VisibilityTextureKey, index: u32) {
		let resource_manager = self.resource_manager.clone();
		let upload_staging = self.upload_staging.clone();
		let failure_key = key.clone();
		self.spawn_preparation(async move {
			let resource: Result<Reference<ResourceImage>, ()> = resource_manager.request(key.as_str()).await.map_err(|error| {
				log::error!(
					"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing, its asset handler is not registered, or the asset database is not loaded. Request error: {}",
					key,
					error
				);
			});
			match resource {
				Ok(resource) if resource.is_gpu_backed() => VisibilityTransferCommand::TextureResourceLoaded {
					key,
					index,
					resource,
				},
				Ok(resource) => match Self::prepare_texture(resource, upload_staging, key, index).await {
					Ok(texture) => VisibilityTransferCommand::TexturePrepared { texture },
					Err(()) => VisibilityTransferCommand::PreparationFailed {
						key: VisibilityResourceKey::Texture(failure_key),
					},
				},
				Err(()) => VisibilityTransferCommand::PreparationFailed {
					key: VisibilityResourceKey::Texture(failure_key),
				},
			}
		});
	}

	/// Loads one texture into owned row-padded data without borrowing transfer memory.
	async fn prepare_texture(
		mut reference: Reference<ResourceImage>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
		key: VisibilityTextureKey,
		index: u32,
	) -> Result<PreparedTexture, ()> {
		let id = key.as_str();
		let texture = reference.resource();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);
		let photometry = texture
			.photometry
			.clone()
			.filter(|photometry| photometric_profile_metadata_is_valid(texture, photometry));

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

		load_texture_bytes(&mut reference, &mut staging, &layouts, id).await?;
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
			photometry,
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
			photometry,
		} = texture;
		let Some((image, sampler)) = self.build_texture_objects(&key, &name, format, extent, mip_count, photometry.is_some())
		else {
			return;
		};

		self.send_completion(VisibilityResourceCompletion::ImageReady {
			key,
			index,
			image,
			sampler,
			upload,
			photometry,
		});
	}

	/// Creates detached texture objects while retaining the compressed resource for direct GPU I/O.
	pub(super) fn adopt_loaded_gpu_texture(
		&mut self,
		key: VisibilityTextureKey,
		index: u32,
		resource: Reference<ResourceImage>,
	) {
		let texture = resource.resource();
		let name = resource.id().to_string();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);
		let mip_count = texture.mip_count.max(1);
		let photometry = texture
			.photometry
			.clone()
			.filter(|photometry| photometric_profile_metadata_is_valid(texture, photometry));
		let Some((image, sampler)) = self.build_texture_objects(&key, &name, format, extent, mip_count, photometry.is_some())
		else {
			return;
		};

		self.send_completion(VisibilityResourceCompletion::GpuImageReady {
			key,
			index,
			image,
			sampler,
			resource,
			photometry,
		});
	}

	/// Builds the detached image and sampler shared by raw and compressed texture paths.
	fn build_texture_objects(
		&mut self,
		key: &VisibilityTextureKey,
		name: &str,
		format: ghi::Formats,
		extent: Extent,
		mip_count: u32,
		photometric: bool,
	) -> Option<(ghi::factory::FactoryImage, ghi::factory::FactorySampler)> {
		let Some(device) = self.resource_factory.as_mut() else {
			log::error!(
				"Visibility texture creation failed for {}. The most likely cause is that material pipeline creation was configured without a factory.",
				name
			);
			self.send_completion(VisibilityResourceCompletion::Failed {
				key: VisibilityResourceKey::Texture(key.clone()),
			});
			return None;
		};
		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(name)
				.extent(extent)
				.mip_levels(mip_count)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let sampler_builder = if photometric {
			photometric_profile_sampler_builder()
		} else {
			default_material_sampler_builder()
		};
		let sampler = device.build_sampler(sampler_builder.max_lod((mip_count - 1) as f32));
		Some((image, sampler))
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

		load_environment_bytes(
			&mut reference,
			&mut staging,
			&diffuse_upload,
			&specular_uploads,
			&specular_stream_names,
			&id,
		)
		.await?;
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

		if idx as usize >= crate::rendering::pipelines::visibility::MAX_BINDLESS_TEXTURES {
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

#[cfg(test)]
mod tests {
	use resource_management::{
		StreamDescription,
		resources::image::{Image, ImagePhotometry},
		types::{Formats, Gamma},
	};

	use super::{
		MipStreamName, TextureUploadLayout, copy_decoded_stream, expand_compact_texture_levels,
		photometric_profile_metadata_is_valid, texture_payload_is_compact,
	};

	fn valid_profile_image() -> Image {
		Image {
			format: Formats::R16F,
			gamma: Gamma::Linear,
			extent: [721, 361, 1],
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
		let invalid_scale = ImagePhotometry {
			intensity_scale_candela: 0.0,
		};

		assert!(photometric_profile_metadata_is_valid(&valid, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&srgb, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&non_profile_format, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&mipmapped, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&valid, &invalid_scale));
	}

	#[test]
	fn decoded_stream_copy_uses_explicit_named_ranges() {
		let decoded = [10_u8, 11, 12, 13, 14, 15];
		let descriptions = [StreamDescription::new("mip[0]", 3, 2)];
		let mut destination = [0_u8; 3];
		{
			let mut stream = resource_management::stream::StreamMut::new("mip[0]", &mut destination);
			copy_decoded_stream(&decoded, &descriptions, &mut stream, "texture").unwrap();
		}

		assert_eq!(destination, [12, 13, 14]);
		{
			let mut missing = resource_management::stream::StreamMut::new("missing", &mut destination);
			assert!(copy_decoded_stream(&decoded, &descriptions, &mut missing, "texture").is_err());
		}
		let mut short = resource_management::stream::StreamMut::new("mip[0]", &mut destination[..2]);
		assert!(copy_decoded_stream(&decoded, &descriptions, &mut short, "texture").is_err());
	}

	fn upload_layout(offset: usize, compact_size: usize, padded_size: usize) -> TextureUploadLayout {
		TextureUploadLayout {
			offset,
			compact_bytes_per_row: compact_size,
			row_count: 1,
			compact_bytes_per_image: compact_size,
			compact_size,
			source_bytes_per_row: padded_size,
			source_bytes_per_image: padded_size,
			padded_size,
		}
	}

	#[test]
	fn compact_texture_payload_moves_levels_into_padded_regions_without_scratch_storage() {
		let layouts = [upload_layout(0, 4, 8), upload_layout(8, 2, 4)];
		let names = [MipStreamName::new(0), MipStreamName::new(1)];
		let descriptions = [StreamDescription::new("mip[0]", 4, 0), StreamDescription::new("mip[1]", 2, 4)];
		let mut staging = [1_u8, 2, 3, 4, 5, 6, 0, 0, 0, 0, 0, 0];

		assert!(texture_payload_is_compact(6, Some(&descriptions), &names, &layouts));
		expand_compact_texture_levels(&mut staging, 6, &layouts).unwrap();

		assert_eq!(&staging[..4], &[1, 2, 3, 4]);
		assert_eq!(&staging[8..10], &[5, 6]);
		assert!(!texture_payload_is_compact(7, Some(&descriptions), &names, &layouts));
	}
}
