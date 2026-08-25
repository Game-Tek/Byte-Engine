use super::*;

pub(crate) const GTAO_CONFIGURATION_PREFIX: &str = "render.gtao.";
const GTAO_MIN_SAMPLES_PER_RAY: u32 = 1;
const GTAO_MAX_SAMPLES_PER_RAY: u32 = 32;
const GTAO_MIN_RADIAL_RAYS: u32 = 2;
const GTAO_MAX_RADIAL_RAYS: u32 = 32;

/// The `GtaoSettings` struct defines the runtime quality and world-space search controls for GTAO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GtaoSettings {
	pub(crate) radius: f32,
	pub(crate) samples_per_ray: u32,
	pub(crate) radial_rays: u32,
}

impl Default for GtaoSettings {
	fn default() -> Self {
		Self {
			radius: 1.0,
			samples_per_ray: 4,
			radial_rays: 6,
		}
	}
}

impl GtaoSettings {
	/// Applies one runtime parameter without changing the settings when validation fails.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"radius" => {
				let radius = configuration_float(value).ok_or_else(|| {
					"GTAO radius was not set. The most likely cause is that the value is not a finite number.".to_string()
				})?;
				if radius < 0.0 || radius > f32::MAX as f64 {
					return Err(
						"GTAO radius was not set. The most likely cause is that the value is outside the nonnegative f32 range."
							.to_string(),
					);
				}
				let settings = Self {
					radius: radius as f32,
					..self
				};
				Ok((settings, ConfigurationValue::Float(f64::from(settings.radius))))
			}
			"samples-per-ray" => {
				let samples_per_ray = configuration_u32(value).ok_or(
					"GTAO samples-per-ray was not set. The most likely cause is that the value is not a whole number.",
				)?;
				if !(GTAO_MIN_SAMPLES_PER_RAY..=GTAO_MAX_SAMPLES_PER_RAY).contains(&samples_per_ray) {
					return Err(format!(
						"GTAO samples-per-ray was not set. The most likely cause is that the value is outside the supported range {}..={}.",
						GTAO_MIN_SAMPLES_PER_RAY, GTAO_MAX_SAMPLES_PER_RAY
					));
				}
				let settings = Self { samples_per_ray, ..self };
				Ok((settings, ConfigurationValue::Integer(i64::from(samples_per_ray))))
			}
			"radial-rays" => {
				let radial_rays = configuration_u32(value)
					.ok_or("GTAO radial-rays was not set. The most likely cause is that the value is not a whole number.")?;
				if !(GTAO_MIN_RADIAL_RAYS..=GTAO_MAX_RADIAL_RAYS).contains(&radial_rays) || radial_rays % 2 != 0 {
					return Err(format!(
						"GTAO radial-rays was not set. The most likely cause is that the value must be an even number in the range {}..={}.",
						GTAO_MIN_RADIAL_RAYS, GTAO_MAX_RADIAL_RAYS
					));
				}
				let settings = Self { radial_rays, ..self };
				Ok((settings, ConfigurationValue::Integer(i64::from(radial_rays))))
			}
			_ => {
				Err("GTAO parameter was not set. The most likely cause is that the parameter name is unsupported.".to_string())
			}
		}
	}

	fn shader_parameters(self) -> GtaoShaderParameters {
		GtaoShaderParameters {
			radius: self.radius,
			samples_per_ray: self.samples_per_ray,
			radial_rays: self.radial_rays,
		}
	}
}

fn configuration_float(value: &ConfigurationValue) -> Option<f64> {
	match value {
		ConfigurationValue::Integer(value) => Some(*value as f64),
		ConfigurationValue::Float(value) if value.is_finite() => Some(*value),
		ConfigurationValue::Text(value) => value.parse().ok().filter(|value: &f64| value.is_finite()),
		ConfigurationValue::Bool(_) | ConfigurationValue::Float(_) => None,
	}
}

fn configuration_u32(value: &ConfigurationValue) -> Option<u32> {
	match value {
		ConfigurationValue::Integer(value) => (*value).try_into().ok(),
		ConfigurationValue::Float(value) if value.is_finite() && value.fract() == 0.0 => (*value as i128).try_into().ok(),
		ConfigurationValue::Text(value) => value.parse().ok(),
		ConfigurationValue::Bool(_) | ConfigurationValue::Float(_) => None,
	}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GtaoShaderParameters {
	radius: f32,
	samples_per_ray: u32,
	radial_rays: u32,
}

/// The `FastGtaoViewData` struct provides compact camera reconstruction constants to the GTAO compute passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FastGtaoViewData {
	pub(crate) pixel_to_ray_mul: [f32; 2],
	pub(crate) pixel_to_ray_add: [f32; 2],
	pub(crate) projection_pixels_y: f32,
	pub(crate) view_z_sign: f32,
	pub(crate) depth_unproject_numerator: f32,
	pub(crate) depth_unproject_denominator_offset: f32,
}

/// Builds pixel-ray and reversed-depth reconstruction constants for one perspective sink.
pub(crate) fn fast_gtao_view_data(sink: &Sink, extent: Extent) -> FastGtaoViewData {
	let view = sink.view();
	let projection = view.projection();
	let width = extent.width() as f32;
	let height = extent.height() as f32;
	let projection_x = projection[0];
	let projection_y = projection[5];
	let near = view.near();
	let far = view.far();
	let clip_range = far - near;

	debug_assert!(
		width > 0.0 && height > 0.0 && projection_x > 0.0 && projection_y > 0.0,
		"GTAO camera constants are invalid. The most likely cause is an empty target or a non-perspective sink."
	);
	debug_assert!(
		near > 0.0 && far > near,
		"GTAO clipping planes are invalid. The most likely cause is a perspective view with an empty depth range."
	);

	FastGtaoViewData {
		pixel_to_ray_mul: [2.0 / (width * projection_x), -2.0 / (height * projection_y)],
		pixel_to_ray_add: [(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y],
		projection_pixels_y: height * projection_y * 0.5,
		// Byte Engine perspective views look down positive view-space Z.
		view_z_sign: 1.0,
		// projection_matrix() maps z to depth as a + b / z. These constants
		// reconstruct positive z as b / (depth - a).
		depth_unproject_numerator: near * far / clip_range,
		depth_unproject_denominator_offset: near / clip_range,
	}
}
/// The `GtaoPass` struct builds a depth-based ambient occlusion term before material evaluation shades the frame.
pub struct GtaoPass {
	settings: GtaoSettings,
	gtao_descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	blur_descriptor_set_x: ghi::DescriptorSetHandle,
	upscale_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) gtao_pipeline: crate::rendering::PipelineRef,
	pub(super) depth_pyramid_pipeline: crate::rendering::PipelineRef,
	pub(super) blur_pipeline_x: crate::rendering::PipelineRef,
	pub(super) upscale_pipeline: crate::rendering::PipelineRef,
	ao_map: ghi::BaseImageHandle,
	view_data: ghi::DynamicBufferHandle<FastGtaoViewData>,
	gtao_parameters: ghi::DynamicBufferHandle<GtaoShaderParameters>,
	depth_pyramid: ghi::DynamicImageHandle,
	raw_ao_map: ghi::DynamicImageHandle,
	temp_ao_map: ghi::DynamicImageHandle,
}

impl GtaoPass {
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		settings: GtaoSettings,
	) -> Self {
		let gtao_descriptor_set = context.create_descriptor_set(Some("GTAO Descriptor Set"));
		let depth_pyramid_descriptor_set = context.create_descriptor_set(Some("GTAO Depth Pyramid Descriptor Set"));
		let blur_descriptor_set_x = context.create_descriptor_set(Some("GTAO Blur X Descriptor Set"));
		let upscale_descriptor_set = context.create_descriptor_set(Some("GTAO Depth-Aware Upscale Descriptor Set"));
		let view_data = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("GTAO View Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let gtao_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("GTAO Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// Metal applies min/max reduction only when every sampler filter is linear.
		// Centered samples then conservatively collapse each reversed-depth 2x2 footprint.
		let depth_pyramid_source_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod((GTAO_DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let ao_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let temp_ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Half-Resolution Blur Intermediate")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let raw_ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Half-Resolution Raw")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		// Mip zero retains full sink resolution so levels one through three match the depth reductions.
		// The initial 8x8 allocation keeps all declared mips valid before the first sink resize.
		let depth_pyramid = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Depth Pyramid")
				.extent(Extent::square(8))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(GTAO_DEPTH_PYRAMID_MIP_COUNT),
		);
		context.write(&[
			ghi::DescriptorWrite::buffer(depth_pyramid_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				depth,
				depth_pyramid_source_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_1_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				1,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_2_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				2,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_3_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				3,
			),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, GTAO_PARAMETERS_BINDING.slot(), gtao_parameters.into()),
			ghi::DescriptorWrite::image(
				gtao_descriptor_set,
				GTAO_OUTPUT_BINDING.slot(),
				raw_ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				gtao_descriptor_set,
				GTAO_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_descriptor_set_x,
				GTAO_BLUR_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_descriptor_set_x,
				GTAO_BLUR_SOURCE_BINDING.slot(),
				raw_ao_map,
				ao_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_descriptor_set_x,
				GTAO_BLUR_OUTPUT_BINDING.slot(),
				temp_ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(upscale_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_BLUR_DEPTH_BINDING.slot(),
				depth,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_BLUR_SOURCE_BINDING.slot(),
				temp_ao_map,
				ao_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				upscale_descriptor_set,
				GTAO_BLUR_OUTPUT_BINDING.slot(),
				ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_UPSCALE_LOW_RESOLUTION_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
		]);
		let depth_pyramid_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/gtao-depth-pyramid.pipeline");
		let gtao_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/gtao.pipeline");
		let blur_pipeline_x = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/gtao-blur-x.pipeline");
		let upscale_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/gtao-upscale.pipeline");

		Self {
			settings,
			gtao_descriptor_set,
			depth_pyramid_descriptor_set,
			blur_descriptor_set_x,
			upscale_descriptor_set,
			gtao_pipeline,
			depth_pyramid_pipeline,
			blur_pipeline_x,
			upscale_pipeline,
			ao_map,
			view_data,
			gtao_parameters,
			depth_pyramid,
			raw_ao_map,
			temp_ao_map,
		}
	}

	pub(super) fn set_settings(&mut self, settings: GtaoSettings) {
		self.settings = settings;
	}

	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		gtao_pipeline: ghi::PipelineHandle,
		depth_pyramid_pipeline: ghi::PipelineHandle,
		blur_pipeline_x: ghi::PipelineHandle,
		upscale_pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<> {
		let gtao_descriptor_set = self.gtao_descriptor_set;
		let depth_pyramid_descriptor_set = self.depth_pyramid_descriptor_set;
		let blur_descriptor_set_x = self.blur_descriptor_set_x;
		let upscale_descriptor_set = self.upscale_descriptor_set;
		let ao_map = self.ao_map;
		let view_data = self.view_data;
		let gtao_parameters = self.gtao_parameters;
		let depth_pyramid = self.depth_pyramid;
		let raw_ao_map = self.raw_ao_map;
		let temp_ao_map = self.temp_ao_map;
		let extent = sink.extent();
		let gtao_extent = gtao_half_resolution_extent(extent);

		*frame.get_mut_dynamic_buffer_slice(view_data) = fast_gtao_view_data(sink, gtao_extent);
		frame.sync_buffer(view_data);
		*frame.get_mut_dynamic_buffer_slice(gtao_parameters) = self.settings.shader_parameters();
		frame.sync_buffer(gtao_parameters);
		frame.resize_image(ao_map, extent);
		frame.resize_image(raw_ao_map.into(), gtao_extent);
		frame.resize_image(temp_ao_map.into(), gtao_extent);
		frame.resize_image(depth_pyramid.into(), extent);

		move |c, _| {
			c.start_region(|label| label.write_str("GTAO"));

			c.start_region(|label| label.write_str("GTAO Depth Pyramid"));
			let c = c.bind_compute_pipeline(depth_pyramid_pipeline);
			c.bind_descriptor_sets(&[depth_pyramid_descriptor_set]);
			c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(8, 4, 1)));
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Evaluate"));
			{
				let c = c.bind_compute_pipeline(gtao_pipeline);
				c.bind_descriptor_sets(&[gtao_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(16, 8, 1)));
			}
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Denoise"));
			c.start_region(|label| label.write_str("GTAO Denoise Horizontal"));
			{
				let c = c.bind_compute_pipeline(blur_pipeline_x);
				c.bind_descriptor_sets(&[blur_descriptor_set_x]);
				c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(8, 8, 1)));
			}
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Denoise and Depth-Aware Upscale"));
			{
				let c = c.bind_compute_pipeline(upscale_pipeline);
				c.bind_descriptor_sets(&[upscale_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			}
			c.end_region();
			c.end_region();

			c.end_region();
		}
	}
}

/// Returns the nonzero extent of the first physical mip in the GTAO depth pyramid.
pub(crate) fn gtao_half_resolution_extent(extent: Extent) -> Extent {
	Extent::rectangle((extent.width() / 2).max(1), (extent.height() / 2).max(1))
}
const GTAO_VIEW_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const GTAO_PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_BLUR_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_BLUR_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_BLUR_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_UPSCALE_LOW_RESOLUTION_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_1_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_2_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_3_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_MIP_COUNT: u32 = 4;
pub(super) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor =
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(1033),
		ghi::ResourceKind::CombinedImageSampler,
		ghi::AccessPolicies::READ,
	)
	.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
pub(super) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_OUTPUT_1_BINDING: ghi::ShaderResourceDescriptor =
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(1034),
		ghi::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT: u32 = 1;
