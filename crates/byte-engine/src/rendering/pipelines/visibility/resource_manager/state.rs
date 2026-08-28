//! Values crossing Visibility's worker, render-thread, upload, and scene-publication boundaries.
//!
//! Read these types in order when implementing another renderer. A
//! [`VisibilityResourceRequest`] leaves the render thread, a
//! [`VisibilityPreparedResource`] returns from a preparer, a [`PreparedUpload`]
//! remains alive through GPU completion, and a
//! [`VisibilityResourceCompletion`] is finally adopted by scene-visible
//! Visibility state. Separate enums make each ownership transfer explicit and
//! prevent worker preparation from choosing resident renderer identities.

use super::*;

/// The `PreparedUpload` enum retains Visibility transfer sources and logical metadata through GPU completion.
///
/// Variants already have the format Visibility needs, but the store still owns
/// offsets, table slots, and final handles. The frame upload queue drops these
/// values only after the exact transfer frame completes.
pub(crate) enum PreparedUpload {
	ResourceMesh {
		key: VisibilityMeshKey,
		mesh: PreparedGpuMesh,
		material_ids: Vec<String>,
		primitive_skins: Vec<Option<u32>>,
		skin_bindings: Vec<Arc<resource_management::resources::skeleton::SkinBinding>>,
		skeleton_node_count: u32,
	},
	GeneratedMesh {
		key: VisibilityMeshKey,
		mesh: PreparedGpuMesh,
		material_id: String,
	},
	Texture {
		key: VisibilityTextureKey,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	Environment(PendingEnvironmentUpload),
}

/// The `VisibilityResourceCompletion` enum decouples shared lifecycle completion from scene-visible adoption.
///
/// The resource client produces these values and the Visibility pipeline
/// manager consumes them. Some variants still require render-thread interning or
/// native GPU-I/O submission; their loader token keeps readiness unpublished
/// until that second adoption step finishes.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		key: VisibilityMeshKey,
		mesh: crate::rendering::pipelines::visibility::pipeline_manager::MeshData,
	},
	MaterialReady {
		token: crate::rendering::resource_loading::ResourceToken,
		id: String,
		pipeline: crate::rendering::PipelineRef,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		textures: Vec<Option<String>>,
	},
	ImageReady {
		token: crate::rendering::resource_loading::ResourceToken,
		key: VisibilityTextureKey,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	GpuImageReady {
		token: crate::rendering::resource_loading::ResourceToken,
		key: VisibilityTextureKey,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		backing: resource_management::resource::ResourceGpuBacking,
		streams: Option<Vec<resource_management::StreamDescription>>,
		format: ghi::Formats,
		extent: Extent,
		mip_count: u32,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	EnvironmentReady {
		token: crate::rendering::resource_loading::ResourceToken,
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

/// The `VisibilityResourceRequest` enum owns everything one worker lane needs without borrowing scene state.
///
/// Keys are repeated in variants that need them in prepared output. This keeps
/// the worker independent from the render-thread loader registry.
#[derive(Clone)]
pub(crate) enum VisibilityResourceRequest {
	Mesh { key: VisibilityMeshKey, source: MeshSource },
	Material { id: String },
	Image { key: VisibilityTextureKey },
	Environment { id: String },
}

/// The `PreparedVisibilityImage` enum selects CPU transfer or native GPU-I/O without assigning a bindless slot.
///
/// Both paths return detached factory objects for render-thread interning. The
/// CPU path retains staged texture data; the GPU path retains validated backing
/// metadata for later native submission.
pub(crate) enum PreparedVisibilityImage {
	Cpu {
		key: VisibilityTextureKey,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		upload: TextureUpload,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
	Gpu {
		key: VisibilityTextureKey,
		image: ghi::factory::FactoryImage,
		sampler: ghi::factory::FactorySampler,
		backing: resource_management::resource::ResourceGpuBacking,
		streams: Option<Vec<resource_management::StreamDescription>>,
		format: ghi::Formats,
		extent: Extent,
		mip_count: u32,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	},
}

/// The `VisibilityPreparedResource` enum returns storage-independent results from one Visibility preparer lane.
///
/// Materials retain texture keys and meshes retain material IDs so the
/// render-thread client can discover and coalesce dependent requests without
/// exposing its dependency graph to workers.
pub(crate) enum VisibilityPreparedResource {
	Mesh(PreparedUpload),
	Material {
		id: String,
		alpha_mode: AlphaMode,
		coverage: resource_management::resources::material::MaterialCoverage,
		texture_keys: Vec<Option<VisibilityTextureKey>>,
		pipeline: crate::rendering::PipelineRef,
	},
	Image(PreparedVisibilityImage),
	Environment {
		id: String,
		environment: FactoryEnvironment,
	},
}

/// The `VisibilityResourceError` struct routes one worker failure back to its logical Visibility resource.
pub(crate) struct VisibilityResourceError {
	pub(crate) key: VisibilityResourceKey,
}

impl VisibilityResourceError {
	/// Creates a preparation failure for the logical key used by retry and reporting.
	pub(crate) fn new(key: VisibilityResourceKey) -> Self {
		Self { key }
	}
}

impl std::fmt::Display for VisibilityResourceError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			formatter,
			"Visibility resource preparation failed for {}. The most likely cause is missing, malformed, or incompatible resource data.",
			self.key
		)
	}
}

/// The `VisibilityRenderResource` struct binds Visibility's four resource families to one shared loader registry.
///
/// One protocol allows a common capacity, completion queue, and revision model.
/// The key and prepared enums preserve per-family behavior for the renderer.
pub(crate) struct VisibilityRenderResource;

impl crate::rendering::resource_loading::RenderResource for VisibilityRenderResource {
	type Key = VisibilityResourceKey;
	type Request = VisibilityResourceRequest;
	type Prepared = VisibilityPreparedResource;
	type Error = VisibilityResourceError;
}

/// The `VisibilityResourceKey` enum coalesces Visibility resources independently of scene instances and GPU slots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityResourceKey {
	Mesh(VisibilityMeshKey),
	Texture(VisibilityTextureKey),
	Material(String),
	Environment(String),
}

/// Visibility uses the allocation-free mesh key shared by every renderer.
pub(crate) type VisibilityMeshKey = crate::rendering::renderable::mesh::MeshKey;

impl From<VisibilityMeshKey> for VisibilityResourceKey {
	fn from(value: VisibilityMeshKey) -> Self {
		Self::Mesh(value)
	}
}

/// The `VisibilityTextureKey` struct gives one image stable logical identity across all Visibility consumers.
///
/// It wraps the resource ID so texture identity cannot be confused with
/// material or environment strings at dependency and slot-assignment seams.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VisibilityTextureKey(String);

impl VisibilityTextureKey {
	/// Creates a texture key from a resource ID before requesting or linking the image.
	pub(crate) fn new(id: impl Into<String>) -> Self {
		Self(id.into())
	}

	/// Returns the resource ID for resource I/O or renderer lookup.
	pub(crate) fn as_str(&self) -> &str {
		self.0.as_str()
	}

	/// Moves the stable resource ID into a scene-visible completion value.
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

/// The `PreparedTexture` struct keeps CPU-ready texture layout and bytes independent from resident image creation.
pub(crate) struct PreparedTexture {
	pub(super) key: VisibilityTextureKey,
	pub(super) name: String,
	pub(super) format: ghi::Formats,
	pub(super) extent: Extent,
	pub(super) mip_count: u32,
	pub(super) upload: TextureUpload,
	pub(super) photometry: Option<resource_management::resources::image::ImagePhotometry>,
}

/// The `PreparedGpuTexture` struct keeps native-I/O backing metadata independent from resident image creation.
pub(crate) struct PreparedGpuTexture {
	pub(super) key: VisibilityTextureKey,
	pub(super) name: String,
	pub(super) format: ghi::Formats,
	pub(super) extent: Extent,
	pub(super) mip_count: u32,
	pub(super) backing: resource_management::resource::ResourceGpuBacking,
	pub(super) streams: Option<Vec<resource_management::StreamDescription>>,
	pub(super) photometry: Option<resource_management::resources::image::ImagePhotometry>,
}

/// The `PreparedEnvironment` struct keeps every CPU-ready IBL stream independent from resident image creation.
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

/// The `FactoryEnvironment` struct keeps one detached IBL set atomic until render-thread interning.
pub(crate) struct FactoryEnvironment {
	pub(super) diffuse_image: ghi::implementation::factory::Image,
	pub(super) specular_image: ghi::implementation::factory::Image,
	pub(super) sampler: ghi::implementation::factory::Sampler,
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) diffuse_upload: TextureUploadLayout,
	pub(super) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
}

impl FactoryEnvironment {
	/// Interns all detached resources and returns one upload batch for atomic environment publication.
	///
	/// Call this on the render thread, then enqueue the result through the
	/// resource client. The environment becomes scene-visible only after the
	/// shared frame queue retires its transfer.
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

/// The `PendingEnvironmentUpload` struct keeps a complete environment on one transfer frame and publication boundary.
///
/// Grouping diffuse and specular images prevents scene descriptors from
/// observing only part of the selected environment.
pub(crate) struct PendingEnvironmentUpload {
	pub(super) id: String,
	pub(super) diffuse_image: ghi::BaseImageHandle,
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) diffuse_upload: TextureUploadLayout,
	pub(super) specular_image: ghi::BaseImageHandle,
	pub(super) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
	pub(super) sampler: ghi::SamplerHandle,
}

/// The `MaterialPipelineConfig` struct gives worker lanes the immutable inputs needed to request material pipelines.
///
/// Clone this into each preparer lane. Pipeline compilation remains a separate
/// service; resource preparation retains only its client and shared push-constant
/// contract.
#[derive(Clone)]
pub(crate) struct MaterialPipelineConfig {
	pub(super) push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	pub(super) pipeline_manager: crate::rendering::PipelineManagerClient,
}

impl MaterialPipelineConfig {
	/// Creates the immutable material-pipeline inputs shared by Visibility preparer lanes.
	pub(crate) fn new(
		push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
		pipeline_manager: crate::rendering::PipelineManagerClient,
	) -> Self {
		Self {
			push_constant_ranges,
			pipeline_manager,
		}
	}
}

/// The `TextureUpload` struct retains row-padded image bytes and copy layouts through GPU completion.
///
/// One staging lease may contain several mip levels. The frame upload queue owns
/// this value until every recorded copy from that lease has completed.
pub(crate) struct TextureUpload {
	pub(super) staging: super::upload_staging::StagingLease,
	pub(super) layouts: SmallVec<[TextureUploadLayout; 16]>,
}
