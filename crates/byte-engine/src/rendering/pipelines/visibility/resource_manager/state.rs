use super::*;

/// The `TransferUploadPrepareResult` struct tracks transfer work and resources handled by a recording.
pub(crate) struct TransferUploadPrepareResult {
	pub(crate) completions: CompletionList,
	pub(super) leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
}

/// The `SubmittedUploadBatch` struct holds resource completions until a transfer frame is complete.
pub(super) struct SubmittedUploadBatch {
	pub(super) frame_key: ghi::FrameKey,
	pub(super) completions: CompletionList,
	pub(super) _leases: SmallVec<[super::upload_staging::StagingLease; 16]>,
}

/// The `PreparedUpload` enum owns everything needed to record one independently ready GPU upload.
pub(crate) enum PreparedUpload {
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
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
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
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	GpuImageReady {
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		resource: Reference<ResourceImage>,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	EnvironmentReady {
		id: String,
		environment: FactoryEnvironment,
	},
	TextureUploadReady {
		key: VisibilityTextureKey,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
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

	TexturePrepared {
		texture: PreparedTexture,
	},
	TextureResourceLoaded {
		key: VisibilityTextureKey,
		index: u32,
		resource: Reference<ResourceImage>,
	},
	RequestImage {
		key: VisibilityTextureKey,
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

/// The `VisibilityTextureKey` struct identifies a bindless image resource across materials, local lights, and instances.
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

	/// Moves the stable resource ID into a completion-side lookup table.
	pub(crate) fn into_string(self) -> String {
		self.0
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
pub(crate) struct PreparedTexture {
	pub(super) key: VisibilityTextureKey,
	pub(super) index: u32,
	pub(super) name: String,
	pub(super) format: ghi::Formats,
	pub(super) extent: Extent,
	pub(super) mip_count: u32,
	pub(super) upload: TextureUpload,
	pub(super) photometry: Option<resource_management::resources::image::ImagePhotometry>,
}

/// The `PreparedEnvironment` struct keeps every CPU-ready IBL stream independent from GPU object creation.
pub(crate) struct PreparedEnvironment {
	pub(super) id: String,
	pub(super) diffuse_format: ghi::Formats,
	pub(super) diffuse_extent: Extent,
	pub(super) specular_format: ghi::Formats,
	pub(super) specular_extent: Extent,
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) diffuse_upload: TextureUploadLayout,
	pub(super) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
}

/// The `FactoryEnvironment` struct keeps one baked IBL set together until the render thread interns its GPU resources.
pub(crate) struct FactoryEnvironment {
	pub(super) diffuse_image: ghi::implementation::factory::Image,
	pub(super) specular_image: ghi::implementation::factory::Image,
	pub(super) sampler: ghi::implementation::factory::Sampler,
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) diffuse_upload: TextureUploadLayout,
	pub(super) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
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
	pub(super) id: String,
	pub(super) diffuse_image: ghi::BaseImageHandle,
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) diffuse_upload: TextureUploadLayout,
	pub(super) specular_image: ghi::BaseImageHandle,
	pub(super) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
	pub(super) sampler: ghi::SamplerHandle,
}

/// The `MaterialPipelineConfig` struct connects material specialization to shared pipeline and resource factories.
pub(crate) struct MaterialPipelineConfig {
	pub(super) push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	pub(super) resource_factory: Option<ghi::implementation::Factory>,
	pub(super) pipeline_manager: crate::rendering::PipelineManagerClient,
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
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) layouts: SmallVec<[TextureUploadLayout; 16]>,
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
