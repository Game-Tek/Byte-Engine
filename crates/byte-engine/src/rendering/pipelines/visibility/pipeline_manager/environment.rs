use super::*;

/// Builds the diffuse IBL write shared by context-time sink creation and frame-time environment adoption.
pub(crate) fn diffuse_environment_descriptor_write(
	descriptor_set: ghi::DescriptorSetHandle,
	environment: EnvironmentTexture,
) -> ghi::DescriptorWrite {
	ghi::DescriptorWrite::combined_image_sampler(
		descriptor_set,
		ENVIRONMENT_BINDING.slot(),
		environment.diffuse_image,
		environment.sampler,
		ghi::Layouts::Read,
	)
}

/// Builds the mipmapped prefiltered environment write.
pub(crate) fn specular_environment_descriptor_write(
	descriptor_set: ghi::DescriptorSetHandle,
	environment: EnvironmentTexture,
) -> ghi::DescriptorWrite {
	ghi::DescriptorWrite::combined_image_sampler(
		descriptor_set,
		SPECULAR_ENVIRONMENT_BINDING.slot(),
		environment.specular_image,
		environment.sampler,
		ghi::Layouts::Read,
	)
}

pub(crate) const DEFAULT_ENVIRONMENT_TEXEL: [u8; 4] = [0, 0, 0, u8::MAX];

/// Creates the black environment sampled while no HDR environment is configured or its upload is pending.
pub(crate) fn create_fallback_environment_texture(context: &mut ghi::implementation::Context) -> EnvironmentTexture {
	let image = context.build_image(
		ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name("Visibility Environment Fallback")
			.extent(Extent::square(1))
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	// Keep alpha opaque so material evaluation samples this black environment instead of selecting
	// the analytical environment reserved for explicitly transparent environment texels.
	context
		.get_texture_slice_mut(image)
		.copy_from_slice(&DEFAULT_ENVIRONMENT_TEXEL);
	context.sync_texture(image);

	let sampler = context.build_sampler(
		ghi::sampler::Builder::new()
			.filtering_mode(ghi::FilteringModes::Linear)
			.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
			.mip_map_mode(ghi::FilteringModes::Linear)
			.addressing_mode(ghi::SamplerAddressingModes::Repeat)
			.min_lod(0.0)
			.max_lod(0.0),
	);

	EnvironmentTexture {
		diffuse_image: image.into(),
		specular_image: image.into(),
		sampler,
	}
}
