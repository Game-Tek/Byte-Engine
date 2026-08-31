//! Worker-side resource I/O and conversion for the Visibility protocol.
//!
//! The preparer deliberately stops before renderer storage. It can validate
//! baked metadata, allocate staging, convert texture row layouts, compile
//! material pipelines through a thread-safe client, and create detached GHI
//! factory objects. Buffer offsets, bindless slots, object interning, and
//! scene-visible publication remain render-thread responsibilities in
//! [`super::VisibilityResourceStore`] and the Visibility pipeline manager.
//!
//! To add a resource family, first add request, key, prepared, and completion
//! variants in [`super::state`]. Implement only I/O and conversion here. Then add
//! renderer placement in the store or explicit adoption in the pipeline manager.

use super::*;

/// The `VisibilityResourcePreparer` struct owns one lane's resource services, staging client, and detached GPU factory.
///
/// Each server owns a separate value, which permits sequential reuse of its
/// mutable factory without locks while cloned endpoints distribute work across
/// lanes.
pub(crate) struct VisibilityResourcePreparer {
	/// Resource manager for loading assets.
	resource_manager: EntityHandle<ResourceManager>,
	/// Detached GPU factory owned by this sequential preparation lane.
	resource_factory: Option<ghi::implementation::Factory>,
	material_pipeline_config: MaterialPipelineConfig,
	upload_staging: Arc<super::upload_staging::UploadStagingArena>,
}

/// Returns whether an image can safely provide the normalized Type C IES intensity-map contract.
fn photometric_profile_metadata_is_valid(
	image: &resource_management::resources::image::Image,
	photometry: &resource_management::resources::image::ImagePhotometry,
) -> bool {
	image.format == resource_management::types::Formats::R16F
		&& image.gamma == resource_management::types::Gamma::Linear
		&& image.extent[2] == 0
		&& image.mip_count == 1
		&& photometry.intensity_scale_candela.is_finite()
		&& photometry.intensity_scale_candela > 0.0
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
	crate::rendering::resource_loading::texture::load_image_streams(reference, streams)
		.await
		.map_err(|error| {
			log::error!(
				"Visibility environment load failed for {}. The most likely cause is missing, corrupt, or mismatched IBL stream data. Error: {}",
				id,
				error
			);
		})
}

impl VisibilityResourcePreparer {
	/// Creates one sequential preparation lane over shared resource and upload services.
	///
	/// Usually call [`Self::spawn`] instead; it creates the shared loader and the
	/// configured set of independent servers together.
	pub(crate) fn new(
		resource_manager: EntityHandle<ResourceManager>,
		upload_staging: Arc<super::upload_staging::UploadStagingArena>,
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

	/// Loads mesh metadata and stages the exact Visibility geometry streams without assigning renderer slots.
	async fn prepare_mesh(
		&mut self,
		key: VisibilityMeshKey,
		source: MeshSource,
	) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure = || VisibilityResourceError::new(VisibilityResourceKey::Mesh(key));
		match source {
			MeshSource::Resource(id) => {
				let resource: Reference<ResourceMesh> = self.resource_manager.request(id).await.map_err(|_| {
					log::error!(
						"Visibility mesh resource request failed for {}. The most likely cause is that the mesh id is missing or the asset database is not loaded.",
						id
					);
					failure()
				})?;
				let resource_data = resource.resource();
				let material_ids = resource_data
					.primitives
					.iter()
					.map(|primitive| primitive.material.id.clone())
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
				let mesh = PreparedGpuMesh::prepare_resource_mesh(resource, self.upload_staging.clone())
					.await
					.ok_or_else(failure)?;
				Ok(VisibilityPreparedResource::Mesh(PreparedUpload::ResourceMesh {
					key,
					mesh,
					material_ids,
					primitive_skins,
					skin_bindings,
					skeleton_node_count,
				}))
			}
			MeshSource::Generated(generator) => {
				let mesh = PreparedGpuMesh::prepare_generated_mesh(generator.as_ref(), self.upload_staging.clone())
					.await
					.ok_or_else(failure)?;
				Ok(VisibilityPreparedResource::Mesh(PreparedUpload::GeneratedMesh {
					key,
					mesh,
					material_id: "white_solid.bema".to_string(),
				}))
			}
		}
	}

	/// Loads one material variant and requests its specialized pipeline without assigning material or texture slots.
	async fn prepare_material(&mut self, id: String) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure_key = VisibilityResourceKey::Material(id.clone());
		let mut reference: Reference<ResourceVariant> = self.resource_manager.request(&id).await.map_err(|_| {
			log::error!(
				"Visibility material variant request failed for {}. The most likely cause is that the resource id is missing or the asset database is not loaded.",
				id
			);
			VisibilityResourceError::new(failure_key.clone())
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
			return Err(VisibilityResourceError::new(failure_key));
		}
		if material.shaders().is_empty() {
			log::error!(
				"Visibility material shader is missing for {}. The most likely cause is that the material was baked without a compute shader.",
				id
			);
			return Err(VisibilityResourceError::new(failure_key));
		}
		let pipeline = self
			.material_pipeline_config
			.pipeline_manager
			.request_specialized_compute_pipeline(
				crate::rendering::pipeline_compilation::SpecializedComputePipelineRequest::new(
					id.clone(),
					self.material_pipeline_config.push_constant_ranges.clone(),
				),
			);
		Ok(VisibilityPreparedResource::Material {
			id,
			alpha_mode,
			coverage,
			texture_keys,
			pipeline,
		})
	}

	/// Loads one image and creates detached GPU objects while retaining either staged bytes or native backing.
	async fn prepare_image(
		&mut self,
		key: VisibilityTextureKey,
	) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure_key = VisibilityResourceKey::Texture(key.clone());
		let resource: Reference<ResourceImage> = self.resource_manager.request(key.as_str()).await.map_err(|error| {
			log::error!(
				"Visibility texture resource request failed for {}. The most likely cause is that the resource id is missing, its asset handler is not registered, or the asset database is not loaded. Request error: {}",
				key,
				error
			);
			VisibilityResourceError::new(failure_key.clone())
		})?;
		let name = resource.id().to_string();
		let texture = resource.resource();
		let photometry = texture
			.photometry
			.clone()
			.filter(|photometry| photometric_profile_metadata_is_valid(texture, photometry));
		let transfer = PreparedTextureTransfer::prepare(resource, self.upload_staging.clone())
			.await
			.map_err(|error| {
				log::error!("Visibility texture preparation failed for {}. {}", key, error);
				VisibilityResourceError::new(failure_key.clone())
			})?;
		let metadata = transfer.metadata();
		let (image, sampler) = self
			.build_texture_objects(
				&name,
				metadata.format(),
				metadata.extent(),
				metadata.mip_count(),
				photometry.is_some(),
			)
			.ok_or_else(|| VisibilityResourceError::new(failure_key))?;
		let prepared = match transfer.into_parts().1 {
			PreparedTextureSource::Staged(upload) => PreparedVisibilityImage::Cpu {
				key,
				image,
				sampler,
				upload,
				photometry,
			},
			PreparedTextureSource::Native(source) => PreparedVisibilityImage::Gpu {
				key,
				image,
				sampler,
				metadata,
				source,
				photometry,
			},
		};
		Ok(VisibilityPreparedResource::Image(prepared))
	}

	/// Builds the detached image and sampler shared by staged and native texture paths.
	fn build_texture_objects(
		&mut self,
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
		if ibl.diffuse_irradiance.extent[2] != 0 || ibl.prefiltered_specular.extent[2] != 0 {
			log::error!(
				"Visibility environment IBL extent is unsupported for {}. The most likely cause is that a baked IBL stream is not a two-dimensional lat-long image.",
				id
			);
			return Err(());
		}
		let diffuse_extent = Extent::from(ibl.diffuse_irradiance.extent);
		let specular_extents: [Extent; IBL_SPECULAR_LEVEL_COUNT] =
			std::array::from_fn(|level| environment_mip_extent(ibl.prefiltered_specular.extent, level as u32));

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

	/// Loads every IBL stream and creates detached cube resources without publishing renderer storage.
	async fn prepare_environment_resource(
		&mut self,
		id: String,
	) -> Result<VisibilityPreparedResource, VisibilityResourceError> {
		let failure_key = VisibilityResourceKey::Environment(id.clone());
		let environment = Self::prepare_environment(self.resource_manager.clone(), self.upload_staging.clone(), id)
			.await
			.map_err(|()| VisibilityResourceError::new(failure_key.clone()))?;
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
			return Err(VisibilityResourceError::new(failure_key));
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
	/// Prepares one logical Visibility resource without assigning renderer-owned storage.
	fn prepare(
		&mut self,
		request: VisibilityResourceRequest,
	) -> impl std::future::Future<Output = Result<VisibilityPreparedResource, VisibilityResourceError>> + '_ {
		async move {
			match request {
				VisibilityResourceRequest::Mesh { key, source } => self.prepare_mesh(key, source).await,
				VisibilityResourceRequest::Material { id } => self.prepare_material(id).await,
				VisibilityResourceRequest::Image { key } => self.prepare_image(key).await,
				VisibilityResourceRequest::Environment { id } => self.prepare_environment_resource(id).await,
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use resource_management::{
		resources::image::{Image, ImagePhotometry},
		types::{Formats, Gamma},
	};

	use super::photometric_profile_metadata_is_valid;

	fn valid_profile_image() -> Image {
		Image {
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
