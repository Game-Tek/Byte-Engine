//! Values crossing the worker, render-thread, upload, and scene-publication boundaries.
//!
//! A [`VisibilityResourceRequest`] leaves the render thread, a [`VisibilityPreparedResource`] returns from a
//! preparer lane, a [`PreparedUpload`] stays alive through GPU transfer completion, and a
//! [`VisibilityResourceCompletion`] is finally adopted by [`super::super::manager::VisibilityPipelineManager`].

use resource_management::resources::image::ImagePhotometry;
use resource_management::resources::material::MaterialCoverage;
use resource_management::types::AlphaMode;

use super::super::geometry::{MeshData, PreparedMesh};
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::texture::TextureUploadLayout;
use crate::rendering::resource_loading::{
	NativeTextureUpload, ResourceToken, StagedTextureUpload, StagingLease, TextureMetadata,
};
use crate::rendering::{PipelineRef, resource_loading};

/// Number of prefiltered specular roughness levels stored by a baked environment.
pub(crate) const IBL_SPECULAR_LEVEL_COUNT: usize =
	resource_management::resources::image::IBL_PREFILTERED_SPECULAR_MIP_COUNT as usize;

/// The `VisibilityResourceKey` enum identifies one logical resource independently of scene instances and GPU slots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum VisibilityResourceKey {
	Mesh(MeshKey),
	Texture(String),
	Material(String),
	Environment(String),
}

impl std::fmt::Display for VisibilityResourceKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Mesh(key) => key.fmt(f),
			Self::Texture(key) | Self::Material(key) | Self::Environment(key) => key.fmt(f),
		}
	}
}

/// The `VisibilityResourceRequest` enum carries everything one worker lane needs without borrowing scene state.
#[derive(Clone)]
pub(crate) enum VisibilityResourceRequest {
	Mesh { key: MeshKey, source: MeshSource },
	Material { id: String },
	Image { id: String },
	Environment { id: String },
}

/// The `VisibilityPreparedResource` enum is a storage-independent result from one preparer lane.
///
/// Materials keep their texture IDs and meshes their material IDs so the render thread can request dependencies.
pub(crate) enum VisibilityPreparedResource {
	Mesh {
		key: MeshKey,
		mesh: PreparedMesh,
	},
	Material {
		id: String,
		alpha_mode: AlphaMode,
		coverage: MaterialCoverage,
		texture_ids: Vec<Option<String>>,
		pipeline: PipelineRef,
	},
	Image(PreparedImage),
	Environment {
		id: String,
		environment: FactoryEnvironment,
	},
}

/// The `PreparedImage` struct is a detached texture whose bytes are either staged or await native GPU I/O.
pub(crate) struct PreparedImage {
	pub(crate) id: String,
	pub(crate) image: ghi::factory::FactoryImage,
	pub(crate) sampler: ghi::factory::FactorySampler,
	pub(crate) source: ImageSource,
	pub(crate) photometry: Option<ImagePhotometry>,
}

pub(crate) enum ImageSource {
	Staged(StagedTextureUpload),
	Native {
		metadata: TextureMetadata,
		source: NativeTextureUpload,
	},
}

/// The `FactoryEnvironment` struct keeps one detached IBL set atomic until render-thread interning.
pub(crate) struct FactoryEnvironment {
	pub(crate) diffuse_image: ghi::factory::FactoryImage,
	pub(crate) specular_image: ghi::factory::FactoryImage,
	pub(crate) sampler: ghi::factory::FactorySampler,
	pub(crate) staging: StagingLease,
	pub(crate) diffuse_upload: TextureUploadLayout,
	pub(crate) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
}

impl FactoryEnvironment {
	/// Interns the detached objects into the frame and returns one transfer batch for atomic publication.
	pub(crate) fn intern(self, id: String, frame: &mut ghi::implementation::Frame) -> PendingEnvironmentUpload {
		PendingEnvironmentUpload {
			id,
			diffuse_image: frame.intern_image(self.diffuse_image).into(),
			specular_image: frame.intern_image(self.specular_image).into(),
			sampler: frame.intern_sampler(self.sampler),
			staging: self.staging,
			diffuse_upload: self.diffuse_upload,
			specular_uploads: self.specular_uploads,
		}
	}
}

/// The `PendingEnvironmentUpload` struct keeps a complete environment on one transfer frame so descriptors never see half of it.
pub(crate) struct PendingEnvironmentUpload {
	pub(crate) id: String,
	pub(crate) diffuse_image: ghi::BaseImageHandle,
	pub(crate) specular_image: ghi::BaseImageHandle,
	pub(crate) sampler: ghi::SamplerHandle,
	pub(crate) staging: StagingLease,
	pub(crate) diffuse_upload: TextureUploadLayout,
	pub(crate) specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT],
}

/// The `PreparedUpload` enum retains transfer sources through GPU completion; the frame queue drops them afterwards.
pub(crate) enum PreparedUpload {
	Mesh {
		key: MeshKey,
		mesh: PreparedMesh,
	},
	Texture {
		id: String,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		upload: StagedTextureUpload,
		photometry: Option<ImagePhotometry>,
	},
	Environment(PendingEnvironmentUpload),
}

/// The `VisibilityResourceCompletion` enum is what the pipeline manager adopts into scene-visible state.
///
/// Variants carrying a token still need render-thread interning or native I/O; the manager must call
/// `mark_ready` or `mark_failed` on the client once that second step finishes.
pub(crate) enum VisibilityResourceCompletion {
	MeshReady {
		key: MeshKey,
		mesh: MeshData,
	},
	MaterialReady {
		token: ResourceToken,
		id: String,
		pipeline: PipelineRef,
		alpha_mode: AlphaMode,
		coverage: MaterialCoverage,
		texture_ids: Vec<Option<String>>,
	},
	/// A detached image whose bytes are staged; intern it and enqueue the transfer.
	ImageReady {
		token: ResourceToken,
		image: PreparedImage,
	},
	EnvironmentReady {
		token: ResourceToken,
		id: String,
		environment: FactoryEnvironment,
	},
	TextureUploadReady {
		id: String,
		index: u32,
		image: ghi::BaseImageHandle,
		sampler: ghi::SamplerHandle,
		photometry: Option<ImagePhotometry>,
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

/// The `VisibilityResourceError` struct routes one worker failure back to its logical resource.
pub(crate) struct VisibilityResourceError {
	pub(crate) key: VisibilityResourceKey,
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

/// The `VisibilityRenderResource` struct binds the four visibility resource families to one shared loader registry.
pub(crate) struct VisibilityRenderResource;

impl resource_loading::RenderResource for VisibilityRenderResource {
	type Key = VisibilityResourceKey;
	type Request = VisibilityResourceRequest;
	type Prepared = VisibilityPreparedResource;
	type Error = VisibilityResourceError;
}
