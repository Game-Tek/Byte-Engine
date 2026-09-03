//! Worker-side resource I/O and conversion.
//!
//! A preparer loads assets, validates baked metadata, fills staging, requests material pipelines through a
//! thread-safe client, and creates detached GHI factory objects. Buffer offsets, bindless slots, object
//! interning, and scene-visible publication stay on the render thread.

use std::sync::Arc;

use ghi::Device as _;
use resource_management::Reference;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::{Image as ResourceImage, ImagePhotometry};
use resource_management::resources::material::{Value, Variant as ResourceVariant};
use resource_management::resources::mesh::Mesh as ResourceMesh;
use smallvec::SmallVec;
use utils::Extent;

use super::super::geometry::PreparedMesh;
use super::protocol::{
	FactoryEnvironment, IBL_SPECULAR_LEVEL_COUNT, ImageSource, PreparedImage, VisibilityPreparedResource,
	VisibilityRenderResource, VisibilityResourceError, VisibilityResourceKey, VisibilityResourceRequest,
};
use crate::core::EntityHandle;
use crate::rendering::renderable::mesh::{MeshKey, MeshSource};
use crate::rendering::resource_loading::texture::{
	TextureUploadLayout, load_image_streams, resource_format_to_ghi, texture_mip_extent,
};
use crate::rendering::resource_loading::{PreparedTextureSource, PreparedTextureTransfer, UploadStagingArena};
use crate::rendering::{PipelineManagerClient, pipeline_compilation::SpecializedComputePipelineRequest};

/// The `MaterialPipelineConfig` struct gives worker lanes the immutable inputs needed to request material pipelines.
#[derive(Clone)]
pub struct MaterialPipelineConfig {
	push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>,
	pipeline_manager: PipelineManagerClient,
}

impl MaterialPipelineConfig {
	pub fn new(push_constant_ranges: Vec<ghi::pipelines::PushConstantRange>, pipeline_manager: PipelineManagerClient) -> Self {
		Self {
			push_constant_ranges,
			pipeline_manager,
		}
	}
}

/// The `VisibilityResourcePreparer` struct is one sequential preparation lane with its own detached GPU factory.
///
/// Create lanes and their render-thread client together with [`VisibilityResourcePreparer::spawn`].
pub struct VisibilityResourcePreparer {
	resource_manager: EntityHandle<ResourceManager>,
	/// Detached GPU factory; `None` when the context cannot create factories, which fails image preparation.
	resource_factory: Option<ghi::implementation::Factory>,
	material_pipeline_config: MaterialPipelineConfig,
	upload_staging: Arc<UploadStagingArena>,
}

/// Builds the default sampler used by visibility material textures.
fn material_sampler() -> ghi::sampler::Builder {
	ghi::sampler::Builder::new()
		.filtering_mode(ghi::FilteringModes::Linear)
		.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
		.mip_map_mode(ghi::FilteringModes::Linear)
		.addressing_mode(ghi::SamplerAddressingModes::Repeat)
		.min_lod(0f32)
		.max_lod(0f32)
}

/// Returns whether an image can safely provide the normalized Type C IES intensity-map contract.
fn photometric_profile_metadata_is_valid(image: &ResourceImage, photometry: &ImagePhotometry) -> bool {
	image.format == resource_management::types::Formats::R16F
		&& image.gamma == resource_management::types::Gamma::Linear
		&& image.extent[2] == 0
		&& image.mip_count == 1
		&& photometry.intensity_scale_candela.is_finite()
		&& photometry.intensity_scale_candela > 0.0
}

impl VisibilityResourcePreparer {
	pub(super) fn new(
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<UploadStagingArena>,
		resource_factory: Option<ghi::implementation::Factory>,
		material_pipeline_config: MaterialPipelineConfig,
	) -> Self {
		Self {
			resource_manager,
			resource_factory,
			material_pipeline_config,
			upload_staging,
		}
	}

	fn factory(&mut self, name: &str) -> Option<&mut ghi::implementation::Factory> {
		self.resource_factory.as_mut().or_else(|| {
			log::error!(
				"Visibility GPU object creation failed for {name}. The most likely cause is that the resource worker was configured without a GPU factory."
			);
			None
		})
	}

	async fn prepare_mesh(
		&mut self,
		key: MeshKey,
		source: MeshSource,
	) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure = || VisibilityResourceError {
			key: VisibilityResourceKey::Mesh(key),
		};
		let mesh = match source {
			MeshSource::Resource(id) => {
				let resource: Reference<ResourceMesh> = self.resource_manager.request(id).await.map_err(|_| {
					log::error!(
						"Visibility mesh resource request failed for {id}. The most likely cause is that the mesh id is missing or the asset database is not loaded."
					);
					failure()
				})?;
				PreparedMesh::resource(resource, self.upload_staging.clone()).await
			}
			MeshSource::Generated(generator) => PreparedMesh::generated(generator.as_ref(), self.upload_staging.clone()).await,
		}
		.ok_or_else(failure)?;
		Ok(VisibilityPreparedResource::Mesh { key, mesh })
	}

	/// Loads one material variant and requests its specialized pipeline.
	async fn prepare_material(&mut self, id: String) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure = || VisibilityResourceError {
			key: VisibilityResourceKey::Material(id.clone()),
		};
		let mut reference: Reference<ResourceVariant> = self.resource_manager.request(&id).await.map_err(|_| {
			log::error!(
				"Visibility material variant request failed for {id}. The most likely cause is that the resource id is missing or the asset database is not loaded."
			);
			failure()
		})?;
		let variant = reference.resource_mut();
		let alpha_mode = variant.alpha_mode.clone();
		let texture_ids = variant
			.variables
			.iter()
			.map(|parameter| match &parameter.value {
				Value::Image(image) => Some(image.id().to_string()),
				_ => None,
			})
			.collect();
		let material = variant.material.resource_mut();
		if material.model.name != "Visibility" || material.model.pass != "MaterialEvaluation" {
			log::error!(
				"Unsupported visibility material model for {id}. The most likely cause is that this material targets a different render model or pass."
			);
			return Err(failure());
		}
		if material.shaders().is_empty() {
			log::error!(
				"Visibility material shader is missing for {id}. The most likely cause is that the material was baked without a compute shader."
			);
			return Err(failure());
		}
		let pipeline = self
			.material_pipeline_config
			.pipeline_manager
			.request_specialized_compute_pipeline(SpecializedComputePipelineRequest::new(
				id.clone(),
				self.material_pipeline_config.push_constant_ranges.clone(),
			));
		Ok(VisibilityPreparedResource::Material {
			id,
			alpha_mode,
			coverage: material.coverage,
			texture_ids,
			pipeline,
		})
	}

	/// Loads one image and creates detached GPU objects while retaining either staged bytes or native backing.
	async fn prepare_image(&mut self, id: String) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure = || VisibilityResourceError {
			key: VisibilityResourceKey::Texture(id.clone()),
		};
		let resource: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|error| {
			log::error!(
				"Visibility texture resource request failed for {id}. The most likely cause is that the resource id is missing, its asset handler is not registered, or the asset database is not loaded. Request error: {error}"
			);
			failure()
		})?;
		let texture = resource.resource();
		let photometry = texture
			.photometry
			.clone()
			.filter(|photometry| photometric_profile_metadata_is_valid(texture, photometry));
		let transfer = PreparedTextureTransfer::prepare(resource, self.upload_staging.clone())
			.await
			.map_err(|error| {
				log::error!("Visibility texture preparation failed for {id}. {error}");
				failure()
			})?;
		let metadata = transfer.metadata();
		let device = self.factory(&id).ok_or_else(failure)?;
		let image = device.build_image(
			ghi::image::Builder::new(metadata.format(), ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(&id)
				.extent(metadata.extent())
				.mip_levels(metadata.mip_count())
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC),
		);
		// Spherical IES profiles must clamp instead of wrapping around the seam.
		let sampler = material_sampler()
			.addressing_mode(if photometry.is_some() {
				ghi::SamplerAddressingModes::Clamp
			} else {
				ghi::SamplerAddressingModes::Repeat
			})
			.max_lod((metadata.mip_count() - 1) as f32);
		let sampler = device.build_sampler(sampler);
		let source = match transfer.into_parts().1 {
			PreparedTextureSource::Staged(upload) => ImageSource::Staged(upload),
			PreparedTextureSource::Native(source) => ImageSource::Native { metadata, source },
		};
		Ok(VisibilityPreparedResource::Image(PreparedImage {
			id,
			image,
			sampler,
			source,
			photometry,
		}))
	}

	/// Loads the diffuse and roughness-prefiltered IBL streams and creates detached cube resources.
	async fn prepare_environment(&mut self, id: String) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure = || VisibilityResourceError {
			key: VisibilityResourceKey::Environment(id.clone()),
		};
		let docs = crate::online_docs_url("develop/resource-management/assets#environment-maps");
		let mut reference: Reference<ResourceImage> = self.resource_manager.request(&id).await.map_err(|_| {
			log::error!(
				"Visibility environment request failed for {id}. The most likely cause is that the `.environment.bead` resource is missing or the asset database is not loaded. See {docs}."
			);
			failure()
		})?;
		let ibl = reference.resource().ibl.clone().ok_or_else(|| {
			log::error!(
				"Visibility environment maps are missing for {id}. The most likely cause is that the selected resource is a plain image instead of a standalone `.environment.bead` asset. See {docs}."
			);
			failure()
		})?;
		let (diffuse, specular) = (&ibl.diffuse_irradiance, &ibl.prefiltered_specular);
		let linear = resource_management::types::Gamma::Linear;
		let available_specular_mips =
			resource_management::resources::mips::mip_level_count(specular.extent[0], specular.extent[1]).unwrap_or(0);
		if diffuse.mip_count != 1
			|| specular.mip_count as usize != IBL_SPECULAR_LEVEL_COUNT
			|| diffuse.gamma != linear
			|| specular.gamma != linear
			|| diffuse.array_layers != 6
			|| specular.array_layers != 6
			|| diffuse.extent[2] != 0
			|| specular.extent[2] != 0
			|| (available_specular_mips as usize) < IBL_SPECULAR_LEVEL_COUNT
		{
			log::error!(
				"Visibility environment IBL metadata is unsupported for {id}. The most likely cause is that the baked image does not contain one linear six-layer diffuse map and {IBL_SPECULAR_LEVEL_COUNT} linear six-layer specular levels."
			);
			return Err(failure());
		}
		let diffuse_format = resource_format_to_ghi(diffuse.format);
		let specular_format = resource_format_to_ghi(specular.format);
		let diffuse_extent = Extent::from(diffuse.extent);
		let specular_extent = Extent::from(specular.extent);

		// Lay every level out back to back in one lease so the environment transfers as one batch.
		let mut byte_count = 0;
		let mut layout = |format, extent| {
			let upload = TextureUploadLayout::new(format, extent, 6, byte_count);
			byte_count += upload.as_ref().map_or(0, |upload| upload.padded_size);
			upload
		};
		let diffuse_upload = layout(diffuse_format, diffuse_extent).ok_or_else(failure)?;
		let mut specular_uploads: [TextureUploadLayout; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|_| diffuse_upload);
		for (level, upload) in specular_uploads.iter_mut().enumerate() {
			*upload = layout(specular_format, texture_mip_extent(specular_extent, level as u32)).ok_or_else(failure)?;
		}
		let mut staging = self.upload_staging.allocate(byte_count, 256).await.ok_or_else(|| {
			log::error!(
				"Visibility environment exceeds the GPU upload arena. The most likely cause is that its complete padded IBL data is larger than the configured upload capacity."
			);
			failure()
		})?;

		let specular_stream_names: [String; IBL_SPECULAR_LEVEL_COUNT] = std::array::from_fn(|level| {
			resource_management::resources::image::ibl_prefiltered_specular_stream_name(level as u32)
		});
		{
			let mut allocator = utils::BufferAllocator::new(staging.bytes_mut());
			let mut streams = SmallVec::<[_; 16]>::new();
			let names = std::iter::once(resource_management::resources::image::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME)
				.chain(specular_stream_names.iter().map(String::as_str));
			for (name, upload) in names.zip(std::iter::once(&diffuse_upload).chain(&specular_uploads)) {
				let region = &mut allocator.take(upload.padded_size)[..upload.compact_size];
				streams.push(resource_management::stream::StreamMut::new(name, region));
			}
			load_image_streams(&mut reference, streams).await.map_err(|error| {
				log::error!(
					"Visibility environment load failed for {id}. The most likely cause is missing, corrupt, or mismatched IBL stream data. Error: {error}"
				);
				failure()
			})?;
		}
		for upload in std::iter::once(&diffuse_upload).chain(&specular_uploads) {
			upload.pack_rows(&mut staging.bytes_mut()[upload.offset..upload.offset + upload.padded_size]);
		}

		let device = self.factory(&id).ok_or_else(failure)?;
		let diffuse_name = format!("{id} diffuse irradiance");
		let specular_name = format!("{id} prefiltered specular");
		fn cube<'a>(format: ghi::Formats, name: &'a str, extent: Extent, mips: u32) -> ghi::image::Builder<'a> {
			ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name(name)
				.extent(extent)
				.cube_compatible()
				.mip_levels(mips)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.use_case(ghi::UseCases::STATIC)
		}
		let diffuse_image = device.build_image(cube(diffuse_format, &diffuse_name, diffuse_extent, 1));
		let specular_image = device.build_image(cube(
			specular_format,
			&specular_name,
			specular_extent,
			IBL_SPECULAR_LEVEL_COUNT as u32,
		));
		let sampler = device.build_sampler(material_sampler().max_lod((IBL_SPECULAR_LEVEL_COUNT - 1) as f32));

		Ok(VisibilityPreparedResource::Environment {
			id,
			environment: FactoryEnvironment {
				diffuse_image,
				specular_image,
				sampler,
				staging,
				diffuse_upload,
				specular_uploads,
			},
		})
	}
}

impl crate::rendering::resource_loading::ResourcePreparer<VisibilityRenderResource> for VisibilityResourcePreparer {
	async fn prepare(
		&mut self,
		request: VisibilityResourceRequest,
	) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		match request {
			VisibilityResourceRequest::Mesh { key, source } => self.prepare_mesh(key, source).await,
			VisibilityResourceRequest::Material { id } => self.prepare_material(id).await,
			VisibilityResourceRequest::Image { id } => self.prepare_image(id).await,
			VisibilityResourceRequest::Environment { id } => self.prepare_environment(id).await,
		}
	}
}

#[cfg(test)]
mod tests {
	use resource_management::types::{Formats, Gamma};

	use super::*;

	fn valid_profile_image() -> ResourceImage {
		ResourceImage {
			format: Formats::R16F,
			gamma: Gamma::Linear,
			extent: [721, 361, 0],
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
		let mut volume = valid_profile_image();
		volume.extent[2] = 1;
		let invalid_scale = ImagePhotometry {
			intensity_scale_candela: 0.0,
		};

		assert!(photometric_profile_metadata_is_valid(&valid, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&srgb, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&non_profile_format, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&mipmapped, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&volume, &photometry));
		assert!(!photometric_profile_metadata_is_valid(&valid, &invalid_scale));
	}
}
