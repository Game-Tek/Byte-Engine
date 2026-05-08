/// The `VisibilityPipelineResourceManager` struct owns asynchronous visibility resource workloads.
pub(crate) struct VisibilityPipelineResourceManager {
	/// Loaded mesh resources.
	meshes: Vec<ResourceStates<MeshSource, MeshData>>,
	/// Mapping from resource ID to mesh index.
	meshes_by_resource: HashMap<String, usize>,
	/// Mapping from generated mesh hash to mesh index.
	meshes_by_generated_hash: HashMap<u64, usize>,
	/// Image resources used by material evaluation.
	images: Vec<ResourceStates<(), ()>>,
	/// Mapping from image resource ID to image index.
	images_by_resource: HashMap<String, usize>,
	/// Texture sampler cache used by material images.
	samplers: HashMap<SamplerState, ghi::SamplerHandle>,
	/// Loaded material textures keyed by resource ID.
	textures: HashMap<String, (ghi::BaseImageHandle, ghi::SamplerHandle)>,
	/// Mapping from mesh resource ID to mesh index.
	mesh_resources: HashMap<String, u32>,
	/// Material pipelines
	materials: Vec<ResourceStates<String, ()>>,
	/// Mapping from material ID to material index.
	material_by_name: HashMap<String, usize>,
	/// GPU vertex data manager (vertex positions, normals, UVs, indices, meshlets).
	pub(crate) gpu_vertex_data_manager: GPUVertexDataManager,
	/// Resource manager for loading assets.
	resource_manager: EntityHandle<ResourceManager>,
	pipelines: RwLock<HashMap<String, PipelineStatus>>,
	shaders: RwLock<StaleHashMap<String, u64, (ghi::ShaderHandle, ghi::ShaderTypes)>>,
	// Async requests cannot reload shader bytes after a sync load consumes the read target,
	// so we keep an owned backing for the shader payload keyed by resource hash.
	shader_requests: RwLock<StaleHashMap<String, u64, Arc<OwnedShader>>>,
	compute_pipeline_requests: Option<Sender<ComputePipelineRequest>>,
	compute_pipeline_results: Option<Receiver<ComputePipelineResult>>,
	resource_factory: Option<ghi::implementation::Factory>,
	work_requests: Receiver<VisibilityResourceRequest>,
	work_completions: Sender<VisibilityResourceCompletion>,
}

impl VisibilityPipelineResourceManager {
	pub(crate) fn spawn(
		device: &mut ghi::implementation::Device,
		resource_manager: EntityHandle<ResourceManager>,
	) -> VisibilityPipelineResourceManagerClient {
		let mesh_data_manager = GPUVertexDataManager::new(device);
		let gpu_vertex_data_manager = mesh_data_manager.clone();
		let (work_request_sender, work_requests) = mpsc::channel();
		let (work_completions, work_completion_receiver) = mpsc::channel();
		let worker = Self::new(device, resource_manager, mesh_data_manager, work_requests, work_completions);

		VisibilityPipelineResourceManagerClient {
			gpu_vertex_data_manager,
			requests: work_request_sender,
			completions: work_completion_receiver,
			worker,
		}
	}

	fn new(
		device: &mut ghi::implementation::Device,
		resource_manager: EntityHandle<ResourceManager>,
		mesh_data_manager: GPUVertexDataManager,
		work_requests: Receiver<VisibilityResourceRequest>,
		work_completions: Sender<VisibilityResourceCompletion>,
	) -> Self {
		let resource_factory = device.create_factory();
		let (compute_pipeline_requests, compute_pipeline_results) = if let Some(factory) = device.create_factory() {
			let (requests, results) = Self::spawn_compute_worker(factory);
			(Some(requests), Some(results))
		} else {
			(None, None)
		};

		Self {
			meshes: Vec::with_capacity(1024),
			meshes_by_resource: HashMap::with_capacity(1024),
			meshes_by_generated_hash: HashMap::with_capacity(128),
			images: Vec::with_capacity(4096),
			images_by_resource: HashMap::with_capacity(4096),
			samplers: HashMap::with_capacity(126),
			textures: HashMap::with_capacity(1024),
			mesh_resources: HashMap::new(),
			materials: Vec::with_capacity(4096),
			material_by_name: HashMap::with_capacity(4096),
			gpu_vertex_data_manager: mesh_data_manager,
			resource_manager,
			pipelines: RwLock::new(HashMap::with_capacity(1024)),
			shaders: RwLock::new(StaleHashMap::with_capacity(1024)),
			shader_requests: RwLock::new(StaleHashMap::with_capacity(1024)),
			compute_pipeline_requests,
			compute_pipeline_results,
			resource_factory,
			work_requests,
			work_completions,
		}
	}

	fn run(&mut self) {
		while let Ok(request) = self.work_requests.recv() {
			if self.handle_request(request) == ResourceWorkerFlow::Stop {
				break;
			}
		}
	}

	fn run_pending(&mut self) {
		while let Ok(request) = self.work_requests.try_recv() {
			if self.handle_request(request) == ResourceWorkerFlow::Stop {
				break;
			}
		}
	}

	fn handle_request(&mut self, request: VisibilityResourceRequest) -> ResourceWorkerFlow {
		match request {
			VisibilityResourceRequest::Mesh { id: _, source: _ } => {}
			VisibilityResourceRequest::Material { id: _ } => {}
			VisibilityResourceRequest::Image { id } => self.handle_image_request(id),
			VisibilityResourceRequest::Variant { id: _ } => {}
			VisibilityResourceRequest::Shutdown => return ResourceWorkerFlow::Stop,
		}

		ResourceWorkerFlow::Continue
	}

	/// Loads one texture resource, creates detached GHI resources for it, and reports the result to the render thread.
	fn handle_image_request(&mut self, id: String) {
		let index = self.request_texture(&id);
		let result = self.load_texture_with_factory(&id, index);
		let completion = match result {
			Ok(texture) => VisibilityResourceCompletion::ImageReady {
				id,
				index,
				image: texture.image,
				sampler: texture.sampler,
				upload: texture.upload,
			},
			Err(()) => VisibilityResourceCompletion::Failed { id },
		};

		if self.work_completions.send(completion).is_err() {
			log::error!(
				"Visibility texture completion failed. The most likely cause is that the render thread stopped receiving worker results."
			);
		}
	}

	/// Builds detached image and sampler resources for a texture resource without touching the render thread's device tables.
	fn load_texture_with_factory(&mut self, id: &str, index: u32) -> Result<FactoryTexture, ()> {
		use ghi::factory::Factory as _;

		let factory = self.resource_factory.as_mut().ok_or_else(|| {
			log::error!(
				"Visibility texture factory is unavailable for {}. The most likely cause is that the active backend does not expose a generic resource factory.",
				id
			);
		})?;
		let mut reference: Reference<ResourceImage> = self.resource_manager.request(id).map_err(|_| {
			log::error!(
				"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
				id
			);
		})?;
		let texture = reference.resource();
		let format = resource_image_format_to_ghi(texture.format);
		let extent = Extent::from(texture.extent);
		let image = factory.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		let sampler = factory.build_sampler(default_material_sampler_builder());

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

		Ok(FactoryTexture {
			index,
			image,
			sampler,
			upload,
		})
	}

	/// Reserves a bindless texture slot for a texture resource.
	fn request_texture(&mut self, texture_id: &str) -> u32 {
		let texture_id = texture_id.to_string();

		match self.images_by_resource.entry(texture_id) {
			Entry::Occupied(v) => *v.get() as u32,
			Entry::Vacant(v) => {
				let idx = self.images.len() as u32;

				if idx as usize >= 1024 {
					panic!(
						"Visibility texture limit exceeded. The most likely cause is that the scene created more texture variants than the visibility pipeline supports."
					);
				}

				self.images.push(ResourceStates::pending(()));
				v.insert(idx as usize);

				idx
			}
		}
	}
}

/// The `VisibilityPipelineResourceManagerClient` struct connects render logic to the asynchronous visibility resource worker.
pub(crate) struct VisibilityPipelineResourceManagerClient {
	pub(crate) gpu_vertex_data_manager: GPUVertexDataManager,
	requests: Sender<VisibilityResourceRequest>,
	completions: Receiver<VisibilityResourceCompletion>,
	worker: VisibilityPipelineResourceManager,
}

impl VisibilityPipelineResourceManagerClient {
	/// Sends a resource request to the asynchronous visibility resource worker.
	pub(crate) fn request(&self, request: VisibilityResourceRequest) {
		if self.requests.send(request).is_err() {
			log::error!(
				"Visibility resource request failed. The most likely cause is that the resource worker thread terminated."
			);
		}
	}

	/// Drains completed resource work without blocking the render thread.
	pub(crate) fn drain_completions(&mut self) -> Vec<VisibilityResourceCompletion> {
		self.worker.run_pending();

		let mut completions = Vec::new();
		while let Ok(completion) = self.completions.try_recv() {
			completions.push(completion);
		}
		completions
	}
}

#[derive(PartialEq, Eq)]
enum ResourceWorkerFlow {
	Continue,
	Stop,
}

/// The `VisibilityResourceRequest` enum describes work the render thread delegates to the resource worker.
pub(crate) enum VisibilityResourceRequest {
	Mesh { id: String, source: MeshSource },
	Material { id: String },
	Variant { id: String },
	Image { id: String },
	Shutdown,
}

/// The `VisibilityResourceCompletion` enum describes resource work that is ready for render-thread adoption.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		id: String,
	},
	MaterialReady {
		id: String,
		index: u32,
		pipeline: Option<ghi::PipelineHandle>,
	},
	ImageReady {
		id: String,
		index: u32,
		image: ghi::implementation::FactoryImage,
		sampler: ghi::implementation::FactorySampler,
		upload: TextureUpload,
	},
	Failed {
		id: String,
	},
}

/// The `FactoryTexture` struct packages detached texture resources with upload bytes for render-thread adoption.
struct FactoryTexture {
	index: u32,
	image: ghi::implementation::FactoryImage,
	sampler: ghi::implementation::FactorySampler,
	upload: TextureUpload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The `SamplerState` struct identifies reusable sampler settings for visibility material textures.
struct SamplerState {
	filtering_mode: ghi::FilteringModes,
	reduction_mode: ghi::SamplingReductionModes,
	mip_map_mode: ghi::FilteringModes,
	addressing_mode: ghi::SamplerAddressingModes,
	anisotropy: Option<NonZeroU8>,
	min_lod: u8,
	max_lod: u8,
}

/// The `TextureUpload` struct carries row-padded texture bytes until the transfer queue copies them.
pub(crate) struct TextureUpload {
	pub(crate) data: Vec<u8>,
	pub(crate) source_bytes_per_row: usize,
	pub(crate) source_bytes_per_image: usize,
}

impl VisibilityPipelineResourceManager {
	fn load_texture(
		&mut self,
		reference: &mut Reference<ResourceImage>,
		device: &mut ghi::implementation::Frame,
	) -> Option<(String, ghi::BaseImageHandle, ghi::SamplerHandle, Option<TextureUpload>)> {
		if let Some(r) = self.textures.get(reference.id()) {
			return Some((reference.id().to_string(), r.0, r.1, None));
		}

		let texture = reference.resource();

		let format = match texture.format {
			resource_management::types::Formats::RG8 => ghi::Formats::RG8UNORM,
			resource_management::types::Formats::RGB8 => ghi::Formats::RGB8UNORM,
			resource_management::types::Formats::RGB16 => ghi::Formats::RGB16UNORM,
			resource_management::types::Formats::RGBA8 => ghi::Formats::RGBA8UNORM,
			resource_management::types::Formats::RGBA16 => ghi::Formats::RGBA16UNORM,
			resource_management::types::Formats::BC5 => ghi::Formats::BC5,
			resource_management::types::Formats::BC7 => ghi::Formats::BC7,
			resource_management::types::Formats::BC7SRGB => ghi::Formats::BC7SRGB,
		};

		let extent = Extent::from(texture.extent);

		let image = device.build_image(
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(reference.id())
				.extent(extent)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);

		let mut source = vec![0u8; reference.size];
		let load_target = reference.load(source.as_mut_slice().into()).ok()?;
		let source = load_target.buffer()?;
		let upload = make_texture_upload(format, extent, source)?;
		let sampler = self.build_sampler(device);
		let texture = (image.into(), sampler);

		self.textures.insert(reference.id().to_string(), texture);

		Some((reference.id().to_string(), texture.0, texture.1, Some(upload)))
	}

	fn build_sampler(&mut self, device: &mut ghi::implementation::Frame) -> ghi::SamplerHandle {
		let sampler_state = SamplerState {
			filtering_mode: ghi::FilteringModes::Linear,
			reduction_mode: ghi::SamplingReductionModes::WeightedAverage,
			mip_map_mode: ghi::FilteringModes::Linear,
			addressing_mode: ghi::SamplerAddressingModes::Repeat,
			anisotropy: None,
			min_lod: 0,
			max_lod: 0,
		};

		match self.samplers.entry(sampler_state) {
			Entry::Occupied(v) => *v.get(),
			Entry::Vacant(v) => {
				let mut sampler_builder = ghi::sampler::Builder::new()
					.filtering_mode(sampler_state.filtering_mode)
					.reduction_mode(sampler_state.reduction_mode)
					.mip_map_mode(sampler_state.mip_map_mode)
					.addressing_mode(sampler_state.addressing_mode)
					.min_lod(sampler_state.min_lod as f32)
					.max_lod(sampler_state.max_lod as f32);

				if let Some(anisotropy) = sampler_state.anisotropy {
					sampler_builder = sampler_builder.anisotropy(anisotropy.get() as f32);
				}

				let sampler_handler = device.build_sampler(sampler_builder);
				v.insert(sampler_handler);
				sampler_handler
			}
		}
	}

	fn loaded_textures(&self) -> Vec<(String, ghi::BaseImageHandle, ghi::SamplerHandle)> {
		self.textures
			.iter()
			.map(|(name, (image, sampler))| (name.clone(), *image, *sampler))
			.collect()
	}
}

/// Request resources implementation block
#[cfg(any())]
impl VisibilityPipelineResourceManager {
	// Prepares the transfer buffer for the given frame.
	// Returns the remaining transfer buffer capacity and whether transfer work was recorded.
	// # Arguments
	// * `transfer` - The command buffer recording to prepare the transfer buffer for.
	// * `key` - The frame key identifying the frame to prepare the transfer buffer for.
	// * `slice` - The buffer slice to prepare the transfer buffer in.
	fn prepare_transfers<'a>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		key: ghi::FrameKey,
		completed_frame: Option<ghi::FrameKey>,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: utils::BufferAllocator<'a>,
	) {
		for (idx, mesh) in self.meshes.iter().enumerate() {
			if mesh.is_pending() {
				self.load_mesh(idx);
			}
		}
	}

	/// Requests a material be loaded and returns its index.
	/// For this material to effectively be loaded it must be "seen" by a load function.
	fn request_material(&mut self, material_id: &str) -> u32 {
		let material_id = material_id.to_string();

		match self.material_by_name.entry(material_id.clone()) {
			Entry::Occupied(v) => *v.get() as u32,
			Entry::Vacant(v) => {
				let idx = self.materials.len() as u32;

				if idx as usize >= MAX_MATERIALS {
					panic!(
						"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
					);
				}

				self.materials.push(ResourceStates::pending(material_id));

				v.insert(idx as usize);

				idx
			}
		}
	}

	/// Requests a mesh be loaded and returns its index.
	/// For this mesh to effectively be loaded it must be "seen" by a load function.
	fn request_mesh_resource(&mut self, mesh_id: &str) -> u32 {
		let mesh_id = mesh_id.to_string();

		match self.meshes_by_resource.entry(mesh_id.clone()) {
			Entry::Occupied(v) => *v.get() as u32,
			Entry::Vacant(v) => {
				let idx = self.meshes.len() as u32;

				if idx as usize >= 1024 {
					panic!(
						"Visibility mesh limit exceeded. The most likely cause is that the scene created more mesh variants than the visibility pipeline supports."
					);
				}

				self.meshes.push(ResourceStates::pending(todo!()));

				idx
			}
		}
	}

	/// Requests a texture be loaded and returns its index.
	/// For this texture to effectively be loaded it must be "seen" by a load function.
	fn request_texture(&mut self, texture_id: &str) -> u32 {
		let texture_id = texture_id.to_string();

		match self.images_by_resource.entry(texture_id.clone()) {
			Entry::Occupied(v) => *v.get() as u32,
			Entry::Vacant(v) => {
				let idx = self.images.len() as u32;

				if idx as usize >= 1024 {
					panic!(
						"Visibility texture limit exceeded. The most likely cause is that the scene created more texture variants than the visibility pipeline supports."
					);
				}

				self.images.push(ResourceStates::pending(()));
				v.insert(idx as usize);

				idx
			}
		}
	}
}

/// Loading resource implementation block
#[cfg(any())]
impl VisibilityPipelineResourceManager {
	fn load_mesh(&mut self, idx: usize) {
		let mesh = &mut self.meshes[idx];

		if !mesh.is_pending() {
			return;
		}

		let _ = self.load_mesh_source(c, staging_data_buffer, slice, mesh_source);
	}

	fn load_material(&mut self, idx: usize) {
		let material = &mut self.materials[idx];

		if !material.is_pending() {
			return;
		}

		let _ = self.load_material_source(c, staging_data_buffer, slice, material_source);
	}
}

#[cfg(any())]
impl VisibilityPipelineResourceManager {
	/// Writes the mesh source into GPU memory.
	/// Returns `Ok(_)` if the mesh source was loaded successfully, `Err(_)` otherwise.
	fn load_mesh_source<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		mesh_source: &MeshSource,
	) -> Result<(), ()> {
		let mesh = match mesh_source {
			MeshSource::Resource(urid) => {
				let resource_request: Reference<ResourceMesh> = {
					let resource_manager = &self.resource_manager;
					let Ok(resource_request) = resource_manager.request(urid) else {
						log::error!("Failed to load mesh resource {}", urid);
						let mesh_idx = self.meshes.len();
						self.meshes.push(ResourceStates::Failed);
						self.meshes_by_resource.insert(urid.to_string(), mesh_idx);
						return Err(());
					};
					resource_request
				};

				self.load_mesh_resource(c, staging_data_buffer, slice, resource_request)
			}
			MeshSource::Generated(generator) => self.load_mesh_generator(c, staging_data_buffer, slice, generator.as_ref()),
		};

		Ok(())
	}

	/// Writes the mesh source into GPU memory.
	/// Returns `Ok(_)` if the mesh source was loaded successfully, `Err(_)` otherwise.
	fn load_mesh_resource<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		mesh_resource: Reference<ResourceMesh>,
	) -> Result<(), ()> {
		if let Some(mesh) = self
			.gpu_vertex_data_manager
			.write_gpu_mesh_data_and_return_mesh_object_for_mesh_resource(c, staging_data_buffer, slice, &mut mesh_resource)
		{
			let r = mesh_resource.resource();

			let primitives = r
				.primitives
				.iter()
				.zip(mesh.primitives)
				.map(|(rp, mp)| {
					let variant = self.request_material(&rp.material.id)?;

					Some(MeshPrimitive {
						material_index: variant,
						meshlet_count: mp.meshlet_count,
						meshlet_offset: mp.meshlet_offset,
						vertex_offset: mp.vertex_offset,
						primitive_offset: mp.primitive_offset,
						triangle_offset: mp.triangle_offset,
					})
				})
				.collect::<Option<Vec<_>>>();

			let Some(primitives) = primitives else {
				return Err(());
			};

			let mesh = MeshData {
				primitives,
				vertex_offset: mesh.vertex_offset,
				primitive_offset: mesh.primitive_offset,
				triangle_offset: mesh.triangle_offset,
				meshlet_offset: mesh.meshlet_offset,
				acceleration_structure: None,
			};

			Ok(())
		} else {
			Err(())
		}
	}

	/// Writes the mesh generator into GPU memory.
	/// Returns `Ok(_)` if the mesh generator was loaded successfully, `Err(_)` otherwise.
	fn load_mesh_generator<'slf, 'buffer>(
		&'slf mut self,
		c: &mut ghi::implementation::CommandBufferRecording,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
		mesh_generator: &dyn MeshGenerator,
	) -> Result<(), ()> {
		if let Some(mesh) = self
			.gpu_vertex_data_manager
			.write_gpu_mesh_data_and_return_mesh_object_for_mesh_generator(mesh_generator, c, staging_data_buffer, slice)
		{
			let primitives = mesh
				.primitives
				.iter()
				.map(|p| {
					let variant = self.request_material("white_solid.bema")?;

					Some(MeshPrimitive {
						material_index: variant,
						meshlet_count: p.meshlet_count,
						meshlet_offset: p.meshlet_offset,
						vertex_offset: p.vertex_offset,
						primitive_offset: p.primitive_offset,
						triangle_offset: p.triangle_offset,
					})
				})
				.collect::<Option<Vec<_>>>();

			let Some(primitives) = primitives else {
				return Err(());
			};

			let mesh = MeshData {
				primitives,
				vertex_offset: mesh.vertex_offset,
				primitive_offset: mesh.primitive_offset,
				triangle_offset: mesh.triangle_offset,
				meshlet_offset: mesh.meshlet_offset,
				acceleration_structure: None,
			};
		}

		Ok(())
	}

	fn load_material_resource<'a>(
		&'a mut self,
		resource: &mut resource_management::Reference<ResourceMaterial>,
		device: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let shader_names = resource
			.resource()
			.shaders()
			.iter()
			.map(|shader| shader.id().to_string())
			.collect::<Vec<_>>();

		let parameters = &mut resource.resource_mut().parameters;

		let textures = parameters
			.iter_mut()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => Some((image.id().to_string(), self.request_texture(image.id()))),
				_ => None,
			})
			.collect::<Vec<_>>();

		let texture_dependencies = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(name, _)| name.clone()))
			.collect::<Vec<_>>();

		match resource.resource().model.name.as_str() {
			"Visibility" => match resource.resource().model.pass.as_str() {
				"MaterialEvaluation" => {
					let pipeline = self.load_material_pipeline(
						&[
							self.descriptor_set_layout,
							self.visibility_descriptor_set_layout,
							self.material_evaluation_descriptor_set_layout,
						],
						&[ghi::pipelines::PushConstantRange::new(0, 4)],
						resource,
						device,
					);

					self.material_evaluation_materials.insert(
						material_id.clone(),
						ResourceStates::Loading(
							device.key(),
							RenderDescription {
								name: material_id,
								index,
								pipeline,
								alpha: false,
								textures: texture_dependencies,
								variant: RenderDescriptionVariants::Material { shaders: shader_names },
							},
						),
					);

					Ok(index)
				}
				_ => {
					error!("Unknown material pass: {}", resource.resource().model.pass);
					Err(())
				}
			},
			_ => {
				error!("Unknown material model");
				Err(())
			}
		}
	}

	fn create_variant_resources<'s, 'a>(
		&'s mut self,
		mut resource: resource_management::Reference<ResourceVariant>,
		device: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let variant_id = resource.id().to_string();
		let index = match self.material_evaluation_materials.get(&variant_id) {
			Some(ResourceStates::Pending(pending)) => pending.index,
			Some(ResourceStates::Failed) => return Err(()),
			Some(material) => return Ok(material.index()),
			None => self.material_evaluation_materials.len() as u32,
		};

		let specialization_constants: Vec<ghi::pipelines::SpecializationMapEntry> = resource
			.resource_mut()
			.variables
			.iter()
			.enumerate()
			.filter_map(|(i, variable)| match &variable.value {
				Value::Scalar(scalar) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "f32".to_string(), *scalar).into()
				}
				Value::Vector3(value) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec3f".to_string(), *value).into()
				}
				Value::Vector4(value) => {
					ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), *value).into()
				}
				_ => None,
			})
			.collect();

		let pipeline = self.load_variant_pipeline(
			&[
				self.descriptor_set_layout,
				self.visibility_descriptor_set_layout,
				self.material_evaluation_descriptor_set_layout,
			],
			&[ghi::pipelines::PushConstantRange::new(0, 4)],
			&specialization_constants,
			&mut resource,
			device,
		);

		let variant = resource.resource_mut();

		let _material_id = variant.material.id().to_string();

		self.create_material_resources(&mut variant.material, device)?;

		if index as usize >= MAX_MATERIALS {
			panic!(
				"Visibility material limit exceeded. The most likely cause is that the scene created more material variants than the visibility pipeline supports."
			);
		}

		let textures = variant
			.variables
			.iter_mut()
			.map(|parameter| match parameter.value {
				Value::Image(ref image) => Some((image.id().to_string(), self.reserve_image_resources(image.id()))),
				_ => None,
			})
			.collect::<Vec<_>>();
		let texture_dependencies = textures
			.iter()
			.filter_map(|texture| texture.as_ref().map(|(name, _)| name.clone()))
			.collect::<Vec<_>>();

		let alpha = variant.alpha_mode == resource_management::types::AlphaMode::Blend;

		let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);

		let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

		let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };
		material_data.textures.fill(u32::MAX);

		for (i, texture) in textures.iter().enumerate() {
			if i >= MAX_MATERIAL_TEXTURES {
				panic!(
					"Visibility material texture limit exceeded. The most likely cause is that a material variant references more textures than the fixed per-material indirection table supports."
				);
			}
			material_data.textures[i] = texture.as_ref().map(|(_, index)| *index).unwrap_or(0xFFFFFFFFu32) as u32;
		}

		device.sync_buffer(self.materials_data_buffer_handle);

		self.material_evaluation_materials.insert(
			variant_id.clone(),
			ResourceStates::Loading(
				device.key(),
				RenderDescription {
					name: variant_id,
					index,
					pipeline,
					alpha,
					textures: texture_dependencies,
					variant: RenderDescriptionVariants::Variant {},
				},
			),
		);

		Ok(index)
	}

	/// Registers a mesh instance for rendering and may load the mesh resource on the GPU if it is not already loaded.
	/// The mesh data may not be usable immediately after this call, as it is written to the GPU asynchronously.
	// pub fn create_renderable_mesh_instance_and_write_mesh_data_if_not_exists<'slf, 'buffer>(
	// 	&'slf mut self,
	// 	c: &mut ghi::implementation::CommandBufferRecording,
	// 	renderable: EntityHandle<dyn RenderableMesh>,
	// 	staging_data_buffer: ghi::BaseBufferHandle,
	// 	slice: &mut utils::BufferAllocator<'buffer>,
	// ) -> bool {
	// 	let mesh_source = renderable.get_mesh();

	// 	let Some((mesh_index, mesh)) = self
	// 		.create_render_mesh_if_mesh_source_does_not_exists_and_return_mesh_object(
	// 			c,
	// 			staging_data_buffer,
	// 			slice,
	// 			mesh_source,
	// 		)
	// 	else {
	// 		return false;
	// 	};

	// 	let model = renderable.transform().get_matrix().into();

	// 	self.ensure_instance_capacity(mesh.primitives.len());

	// 	for primitive in &mesh.primitives {
	// 		self.scene.render_entities.push(RenderEntity {
	// 			entity: renderable.clone(),
	// 			mesh_index,
	// 			shader_mesh: ShaderMesh {
	// 				model,
	// 				material_index: primitive.material_index,
	// 				base_vertex_index: mesh.vertex_offset + primitive.vertex_offset,
	// 				base_primitive_index: mesh.primitive_offset + primitive.primitive_offset,
	// 				base_triangle_index: mesh.triangle_offset + primitive.triangle_offset,
	// 				base_meshlet_index: mesh.meshlet_offset + primitive.meshlet_offset,
	// 				meshlet_count: primitive.meshlet_count,
	// 			},
	// 		});
	// 		self.scene.render_info.instances.push(Instance {
	// 			meshlet_count: primitive.meshlet_count,
	// 		});
	// 	}

	// 	true
	// }

	/// Records pending texture uploads into the transfer command buffer and marks them loading for this transfer frame.
	pub fn prepare_texture_uploads<'buffer>(
		&mut self,
		transfer: &mut ghi::implementation::CommandBufferRecording,
		key: ghi::FrameKey,
		staging_data_buffer: ghi::BaseBufferHandle,
		slice: &mut utils::BufferAllocator<'buffer>,
	) -> bool {
		let pending_images = self
			.shared
			.images
			.iter()
			.filter_map(|(name, image)| match image {
				ResourceStates::Pending(pending) if pending.image.is_some() && pending.upload.is_some() => Some(name.clone()),
				_ => None,
			})
			.collect::<Vec<_>>();

		let mut recorded_work = false;

		for name in pending_images {
			let Some(ResourceStates::Pending(mut pending)) = self.shared.images.remove(&name) else {
				continue;
			};
			let (Some(image), Some(upload)) = (pending.image.take(), pending.upload.take()) else {
				self.shared.images.insert(name, ResourceStates::Pending(pending));
				continue;
			};
			const TEXTURE_UPLOAD_ALIGNMENT: usize = 256;
			if upload.data.len() > slice.remaining_aligned(TEXTURE_UPLOAD_ALIGNMENT) {
				self.shared.images.insert(
					name,
					ResourceStates::Pending(PendingImage {
						index: image.index,
						image: Some(image),
						upload: Some(upload),
					}),
				);
				break;
			}

			let (source_offset, source_buffer) = slice.take_with_offset_aligned(upload.data.len(), TEXTURE_UPLOAD_ALIGNMENT);
			source_buffer.copy_from_slice(&upload.data);
			transfer.copy_buffer_to_images(&[ghi::BufferImageCopyDescriptor::new(
				staging_data_buffer,
				source_offset,
				upload.source_bytes_per_row,
				upload.source_bytes_per_image,
				image.image,
			)]);
			self.shared.images.insert(name, ResourceStates::Loading(key, image));
			recorded_work = true;
		}

		recorded_work
	}
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

	fn load_shader_handles(
		&self,
		material: &mut ResourceMaterial,
		device: &mut ghi::implementation::Frame,
	) -> Result<Vec<(ghi::ShaderHandle, ghi::ShaderTypes)>, ()> {
		material
			.shaders_mut()
			.iter_mut()
			.map(|shader: &mut Reference<Shader>| {
				if let StaleEntry::Fresh((old_shader, old_shader_type)) =
					self.shaders.read().entry(&shader.id, shader.get_hash())
				{
					return Ok((*old_shader, *old_shader_type));
				}

				let owned_shader = self.load_cached_shader_request(shader)?;

				let new_shader = device
					.create_shader(
						owned_shader.name.as_deref(),
						owned_shader.source.sources(),
						owned_shader.stage,
						owned_shader.binding_descriptors.clone(),
					)
					.unwrap();

				self.shaders
					.write()
					.insert(shader.id().to_string(), shader.get_hash(), (new_shader, owned_shader.stage));

				Ok((new_shader, owned_shader.stage))
			})
			.collect::<Result<Vec<_>, ()>>()
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
				specialization_map_entries: Vec::new(),
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

	fn queue_variant_pipeline(
		&self,
		resource_id: String,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		specialization_map_entries: &[ghi::pipelines::SpecializationMapEntry],
		variant: &mut Reference<ResourceVariant>,
	) -> Option<ghi::PipelineHandle> {
		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.pipelines.write().insert(resource_id.clone(), PipelineStatus::Pending);

		let request = match variant.resource_mut().material.resource_mut().shaders_mut().iter_mut().next() {
			Some(shader) => self.load_cached_shader_request(shader).map(|shader| ComputePipelineRequest {
				key: resource_id.clone(),
				descriptor_set_templates: descriptor_set_template_handles.to_vec(),
				push_constant_ranges: push_constant_ranges.to_vec(),
				shader,
				specialization_map_entries: specialization_map_entries.to_vec(),
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

	fn load_material_pipeline(
		&self,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		reference: &mut Reference<ResourceMaterial>,
		device: &mut ghi::implementation::Frame,
	) -> Option<ghi::PipelineHandle> {
		if self.compute_pipeline_requests.is_some() {
			return self.queue_material_pipeline(
				reference.id().to_string(),
				descriptor_set_template_handles,
				push_constant_ranges,
				reference.resource_mut(),
			);
		}

		let resource_id = reference.id().to_string();

		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		let material = reference.resource_mut();
		let shaders = self.load_shader_handles(material, device).ok()?;
		Self::sleep_for_debug_pipeline_delay();
		let handle = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			descriptor_set_template_handles,
			push_constant_ranges,
			ghi::ShaderParameter::new(&shaders[0].0, ghi::ShaderTypes::Compute),
		));

		self.pipelines.write().insert(resource_id, PipelineStatus::Ready(handle));
		Some(handle)
	}

	fn load_variant_pipeline(
		&self,
		descriptor_set_template_handles: &[ghi::DescriptorSetTemplateHandle],
		push_constant_ranges: &[ghi::pipelines::PushConstantRange],
		specialization_map_entries: &[ghi::pipelines::SpecializationMapEntry],
		reference: &mut Reference<ResourceVariant>,
		device: &mut ghi::implementation::Frame,
	) -> Option<ghi::PipelineHandle> {
		if self.compute_pipeline_requests.is_some() {
			return self.queue_variant_pipeline(
				reference.id().to_string(),
				descriptor_set_template_handles,
				push_constant_ranges,
				specialization_map_entries,
				reference,
			);
		}

		let resource_id = reference.id().to_string();

		if let Some(status) = self.pipelines.read().get(&resource_id) {
			return match status {
				PipelineStatus::Pending | PipelineStatus::Failed => None,
				PipelineStatus::Ready(handle) => Some(*handle),
			};
		}

		self.load_shader_handles(reference.resource_mut().material.resource_mut(), device)
			.ok()?;
		let variant = reference.resource_mut();
		let shader = variant.material.resource().shaders.get(0)?;
		let shader_handle = self
			.shaders
			.read()
			.get(&shader.id().to_string(), shader.hash())
			.map(|(handle, _)| *handle)?;
		Self::sleep_for_debug_pipeline_delay();
		let handle = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			descriptor_set_template_handles,
			push_constant_ranges,
			ghi::ShaderParameter::new(&shader_handle, ghi::ShaderTypes::Compute)
				.with_specialization_map(specialization_map_entries),
		));

		self.pipelines.write().insert(resource_id, PipelineStatus::Ready(handle));
		Some(handle)
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
fn default_material_sampler_builder() -> ghi::sampler::Builder {
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

use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::num::{NonZeroU32, NonZeroU8};
use std::ops::{Deref, DerefMut};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ::core::slice::SlicePattern;
use ghi::device::{Device as _, DeviceCreate as _};
use ghi::frame::Frame as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	graphics_hardware_interface, Size as _,
};
use log::{error, warn};
use math::{mat::MatInverse as _, ShaderMatrix4, ShaderMatrix4x3, Vector3};
use resource_management::asset::bema_asset_handler::ProgramGenerator;
use resource_management::glsl_shader_generator::GLSLShaderGenerator;
use resource_management::msl_shader_generator::MSLShaderGenerator;
use resource_management::resource::reader::ResourceReaderBacking;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resource::{ReadTargets, ReadTargetsMut};
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::Variant as ResourceVariant;
use resource_management::resources::material::{Material as ResourceMaterial, Parameter, Shader, Value, VariantVariable};
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::shader_generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::spirv_shader_generator::SPIRVShaderGenerator;
use resource_management::types::{IndexStreamTypes, IntegralTypes, ShaderTypes};
use resource_management::{glsl, Reference};
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::stale_map::{Entry as StaleEntry, StaleHashMap};
use utils::sync::{Rc, RwLock};
use utils::{Box, Extent, RGBA};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::core::{Entity, EntityHandle};
use crate::rendering::common_shader_generator::{CommonShaderGenerator, CommonShaderScope};
use crate::rendering::lights::{DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::{GPUVertexDataManager, MeshData};
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
use crate::rendering::pipelines::visibility::{
	ShaderMeshletData, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING,
	MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_BINDLESS_TEXTURES, MAX_INSTANCES,
	MAX_LIGHTS, MAX_MATERIALS, MAX_MATERIAL_TEXTURES, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES,
	MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION,
	TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassFunction, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::view::View;
use crate::rendering::{csm, make_perspective_view_from_camera, mesh, world_render_domain, RenderableMesh, Sink};
use crate::resource_management::{self};
use crate::space::Transformable as _;
