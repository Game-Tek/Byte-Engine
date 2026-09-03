//! Ground-truth ambient occlusion computed at half resolution from the camera depth, then denoised and upscaled.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use crate::configuration::ConfigurationValue;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

/// Configuration namespace of the runtime GTAO controls.
pub const GTAO_CONFIGURATION_PREFIX: &str = "render.gtao.";
const MIN_SAMPLES_PER_RAY: u32 = 1;
const MAX_SAMPLES_PER_RAY: u32 = 32;
const MIN_RADIAL_RAYS: u32 = 2;
const MAX_RADIAL_RAYS: u32 = 32;
/// Mip zero retains full sink resolution so levels one through three match the depth reductions.
const DEPTH_PYRAMID_MIP_COUNT: u32 = 4;

const fn buffer(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
	)
}
const fn sampled(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::CombinedImageSampler,
		ghi::AccessPolicies::READ,
	)
}
const fn storage(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	)
}
const VIEW_BINDING: ghi::ShaderResourceDescriptor = buffer(0);
const PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = buffer(1);
// Every GTAO stage reads its input at 1033 and writes its output at 1034/1035; the upscale adds low-res depth at 1036.
const INPUT_BINDING: ghi::ShaderResourceDescriptor = sampled(1033);
const OUTPUT_BINDING: ghi::ShaderResourceDescriptor = storage(1034);
const BLUR_SOURCE_BINDING: ghi::ShaderResourceDescriptor = sampled(1034);
const BLUR_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = storage(1035);
const UPSCALE_LOW_RESOLUTION_DEPTH_BINDING: ghi::ShaderResourceDescriptor = sampled(1036);
const DEPTH_PYRAMID_OUTPUT_BINDINGS: [ghi::ShaderResourceDescriptor; 3] = [storage(1034), storage(1035), storage(1036)];

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
	/// Applies one runtime parameter, returning the updated settings and the effective value, or leaves them unchanged.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"radius" => {
				let radius = configuration_float(value)
					.filter(|radius| *radius >= 0.0 && *radius <= f32::MAX as f64)
					.ok_or(
						"GTAO radius was not set. The most likely cause is that the value is not a finite nonnegative number.",
					)?;
				let settings = Self {
					radius: radius as f32,
					..self
				};
				Ok((settings, ConfigurationValue::Float(f64::from(settings.radius))))
			}
			"samples-per-ray" => {
				let samples_per_ray = configuration_u32(value)
					.filter(|samples| (MIN_SAMPLES_PER_RAY..=MAX_SAMPLES_PER_RAY).contains(samples))
					.ok_or(format!(
						"GTAO samples-per-ray was not set. The most likely cause is that the value is not a whole number in the range {MIN_SAMPLES_PER_RAY}..={MAX_SAMPLES_PER_RAY}."
					))?;
				Ok((
					Self { samples_per_ray, ..self },
					ConfigurationValue::Integer(i64::from(samples_per_ray)),
				))
			}
			"radial-rays" => {
				let radial_rays = configuration_u32(value)
					.filter(|rays| (MIN_RADIAL_RAYS..=MAX_RADIAL_RAYS).contains(rays) && rays % 2 == 0)
					.ok_or(format!(
						"GTAO radial-rays was not set. The most likely cause is that the value is not an even whole number in the range {MIN_RADIAL_RAYS}..={MAX_RADIAL_RAYS}."
					))?;
				Ok((
					Self { radial_rays, ..self },
					ConfigurationValue::Integer(i64::from(radial_rays)),
				))
			}
			_ => {
				Err("GTAO parameter was not set. The most likely cause is that the parameter name is unsupported.".to_string())
			}
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
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct GtaoShaderParameters {
	radius: f32,
	samples_per_ray: u32,
	radial_rays: u32,
}

/// The `FastGtaoViewData` struct provides compact camera reconstruction constants to the GTAO compute passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
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
		width > 0.0 && height > 0.0 && projection_x > 0.0 && projection_y > 0.0 && near > 0.0 && far > near,
		"GTAO camera constants are invalid. The most likely cause is an empty target or a non-perspective sink."
	);
	FastGtaoViewData {
		pixel_to_ray_mul: [2.0 / (width * projection_x), -2.0 / (height * projection_y)],
		pixel_to_ray_add: [(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y],
		projection_pixels_y: height * projection_y * 0.5,
		// Byte Engine perspective views look down positive view-space Z.
		view_z_sign: 1.0,
		// projection_matrix() maps z to depth as a + b / z. These constants reconstruct positive z as b / (depth - a).
		depth_unproject_numerator: near * far / clip_range,
		depth_unproject_denominator_offset: near / clip_range,
	}
}

/// Returns the nonzero extent of the first physical mip in the GTAO depth pyramid.
pub(crate) fn gtao_half_resolution_extent(extent: Extent) -> Extent {
	Extent::rectangle((extent.width() / 2).max(1), (extent.height() / 2).max(1))
}

/// The `GtaoPass` struct builds a depth-based ambient occlusion term before material evaluation shades the frame.
pub(super) struct GtaoPass {
	settings: GtaoSettings,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	gtao_descriptor_set: ghi::DescriptorSetHandle,
	blur_descriptor_set: ghi::DescriptorSetHandle,
	upscale_descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_pipeline: crate::rendering::PipelineRef,
	gtao_pipeline: crate::rendering::PipelineRef,
	blur_pipeline: crate::rendering::PipelineRef,
	upscale_pipeline: crate::rendering::PipelineRef,
	view_data: ghi::DynamicBufferHandle<FastGtaoViewData>,
	parameters: ghi::DynamicBufferHandle<GtaoShaderParameters>,
	depth_pyramid: ghi::DynamicImageHandle,
	raw_ao_map: ghi::DynamicImageHandle,
	blurred_ao_map: ghi::DynamicImageHandle,
	ao_map: ghi::BaseImageHandle,
}

pub(super) struct GtaoPipelines {
	depth_pyramid: ghi::PipelineHandle,
	gtao: ghi::PipelineHandle,
	blur: ghi::PipelineHandle,
	upscale: ghi::PipelineHandle,
}

impl GtaoPass {
	/// Creates the intermediate images and wires the four-stage descriptor graph.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		settings: GtaoSettings,
	) -> Self {
		let depth_pyramid_descriptor_set = context.create_descriptor_set(Some("GTAO Depth Pyramid Descriptor Set"));
		let gtao_descriptor_set = context.create_descriptor_set(Some("GTAO Descriptor Set"));
		let blur_descriptor_set = context.create_descriptor_set(Some("GTAO Blur X Descriptor Set"));
		let upscale_descriptor_set = context.create_descriptor_set(Some("GTAO Depth-Aware Upscale Descriptor Set"));
		let dynamic_buffer = |name| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let view_data = context.build_dynamic_buffer(dynamic_buffer("GTAO View Data"));
		let parameters = context.build_dynamic_buffer(dynamic_buffer("GTAO Parameters"));
		// Metal applies min/max reduction only when every sampler filter is linear.
		// Centered samples then conservatively collapse each reversed-depth 2x2 footprint.
		let max_sampler = context.build_sampler(
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
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let ao_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let half_resolution_image = |name| {
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let raw_ao_map = context.build_dynamic_image(half_resolution_image("GTAO Half-Resolution Raw"));
		let blurred_ao_map = context.build_dynamic_image(half_resolution_image("GTAO Half-Resolution Blur Intermediate"));
		// The initial 8x8 allocation keeps all declared mips valid before the first sink resize.
		let depth_pyramid = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Depth Pyramid")
				.extent(Extent::square(8))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(DEPTH_PYRAMID_MIP_COUNT),
		);
		let sampled = |set, binding: ghi::ShaderResourceDescriptor, image: ghi::BaseImageHandle, sampler| {
			ghi::DescriptorWrite::combined_image_sampler(set, binding.slot(), image, sampler, ghi::Layouts::Read)
		};
		let mut writes = vec![
			ghi::DescriptorWrite::buffer(depth_pyramid_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			sampled(depth_pyramid_descriptor_set, INPUT_BINDING, depth, max_sampler),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, PARAMETERS_BINDING.slot(), parameters.into()),
			sampled(gtao_descriptor_set, INPUT_BINDING, depth_pyramid.into(), depth_sampler),
			ghi::DescriptorWrite::image(gtao_descriptor_set, OUTPUT_BINDING.slot(), raw_ao_map, ghi::Layouts::General),
			sampled(blur_descriptor_set, INPUT_BINDING, depth_pyramid.into(), depth_sampler),
			sampled(blur_descriptor_set, BLUR_SOURCE_BINDING, raw_ao_map.into(), ao_sampler),
			ghi::DescriptorWrite::image(
				blur_descriptor_set,
				BLUR_OUTPUT_BINDING.slot(),
				blurred_ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(upscale_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			sampled(upscale_descriptor_set, INPUT_BINDING, depth, depth_sampler),
			sampled(upscale_descriptor_set, BLUR_SOURCE_BINDING, blurred_ao_map.into(), ao_sampler),
			ghi::DescriptorWrite::image(
				upscale_descriptor_set,
				BLUR_OUTPUT_BINDING.slot(),
				ao_map,
				ghi::Layouts::General,
			),
			sampled(
				upscale_descriptor_set,
				UPSCALE_LOW_RESOLUTION_DEPTH_BINDING,
				depth_pyramid.into(),
				depth_sampler,
			),
		];
		writes.extend(DEPTH_PYRAMID_OUTPUT_BINDINGS.iter().enumerate().map(|(index, binding)| {
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				binding.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				index as u32 + 1,
			)
		}));
		context.write(&writes);
		let request = |name| pipeline_manager.request_pipeline(name);

		Self {
			settings,
			depth_pyramid_descriptor_set,
			gtao_descriptor_set,
			blur_descriptor_set,
			upscale_descriptor_set,
			depth_pyramid_pipeline: request("byte-engine/rendering/visibility/gtao-depth-pyramid.pipeline"),
			gtao_pipeline: request("byte-engine/rendering/visibility/gtao.pipeline"),
			blur_pipeline: request("byte-engine/rendering/visibility/gtao-blur-x.pipeline"),
			upscale_pipeline: request("byte-engine/rendering/visibility/gtao-upscale.pipeline"),
			view_data,
			parameters,
			depth_pyramid,
			raw_ao_map,
			blurred_ao_map,
			ao_map,
		}
	}

	pub(super) fn set_settings(&mut self, settings: GtaoSettings) {
		self.settings = settings;
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<GtaoPipelines> {
		Some(GtaoPipelines {
			depth_pyramid: pipeline_manager.pipeline(self.depth_pyramid_pipeline)?,
			gtao: pipeline_manager.pipeline(self.gtao_pipeline)?,
			blur: pipeline_manager.pipeline(self.blur_pipeline)?,
			upscale: pipeline_manager.pipeline(self.upscale_pipeline)?,
		})
	}

	/// Uploads this frame's constants, resizes the intermediates, and returns the four-stage recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		pipelines: GtaoPipelines,
	) -> impl RenderPassFunction + use<> {
		let extent = sink.extent();
		let gtao_extent = gtao_half_resolution_extent(extent);
		*frame.get_mut_dynamic_buffer_slice(self.view_data) = fast_gtao_view_data(sink, gtao_extent);
		frame.sync_buffer(self.view_data);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = GtaoShaderParameters {
			radius: self.settings.radius,
			samples_per_ray: self.settings.samples_per_ray,
			radial_rays: self.settings.radial_rays,
		};
		frame.sync_buffer(self.parameters);
		frame.resize_image(self.ao_map, extent);
		frame.resize_image(self.raw_ao_map.into(), gtao_extent);
		frame.resize_image(self.blurred_ao_map.into(), gtao_extent);
		frame.resize_image(self.depth_pyramid.into(), extent);

		let stages = [
			(
				"GTAO Depth Pyramid",
				pipelines.depth_pyramid,
				self.depth_pyramid_descriptor_set,
				gtao_extent,
				Extent::new(8, 4, 1),
			),
			(
				"GTAO Evaluate",
				pipelines.gtao,
				self.gtao_descriptor_set,
				gtao_extent,
				Extent::new(16, 8, 1),
			),
			(
				"GTAO Denoise Horizontal",
				pipelines.blur,
				self.blur_descriptor_set,
				gtao_extent,
				Extent::new(8, 8, 1),
			),
			(
				"GTAO Denoise and Depth-Aware Upscale",
				pipelines.upscale,
				self.upscale_descriptor_set,
				extent,
				Extent::new(8, 8, 1),
			),
		];
		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			c.start_region(|label| label.write_str("GTAO"));
			for (name, pipeline, descriptor_set, extent, workgroup) in stages {
				c.start_region(|label| label.write_str(name));
				let c = c.bind_compute_pipeline(pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(extent, workgroup));
				c.end_region();
			}
			c.end_region();
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Point, UnitVector};
	use maths_rs::Vec4f;

	use super::*;
	use crate::rendering::View;

	#[test]
	fn gtao_runtime_parameters_update_quality_controls_without_partial_state() {
		let defaults = GtaoSettings::default();
		let (settings, radius) = defaults
			.with_parameter("radius", &ConfigurationValue::Text("2.5".to_string()))
			.expect("radius should parse");
		let (settings, samples) = settings
			.with_parameter("samples-per-ray", &ConfigurationValue::Integer(12))
			.expect("sample count should parse");
		let (settings, rays) = settings
			.with_parameter("radial-rays", &ConfigurationValue::Integer(16))
			.expect("ray count should parse");

		assert_eq!(settings.radius, 2.5);
		assert_eq!(settings.samples_per_ray, 12);
		assert_eq!(settings.radial_rays, 16);
		assert_eq!(radius, ConfigurationValue::Float(2.5));
		assert_eq!(samples, ConfigurationValue::Integer(12));
		assert_eq!(rays, ConfigurationValue::Integer(16));
		assert!(
			settings
				.with_parameter("radial-rays", &ConfigurationValue::Integer(7))
				.is_err()
		);
		assert!(settings.with_parameter("radius", &ConfigurationValue::Float(-1.0)).is_err());
		assert_eq!(settings.radial_rays, 16);
	}

	#[test]
	fn fast_gtao_view_reconstructs_pixel_rays_and_reversed_depth() {
		let extent = Extent::rectangle(1920, 1080);
		let view = View::new_perspective(
			math::Degrees::new(60.0),
			extent.width() as f32 / extent.height() as f32,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let sink = Sink::new(view, extent, 0);
		let gtao_extent = gtao_half_resolution_extent(extent);
		let constants = fast_gtao_view_data(&sink, gtao_extent);
		let projection = view.projection();

		assert_eq!(std::mem::size_of_val(&constants), 32);

		for z in [0.1f32, 0.5, 1.0, 10.0, 100.0] {
			let clip = projection * Vec4f::new(0.0, 0.0, z, 1.0);
			let depth = clip.z / clip.w;
			let reconstructed = constants.depth_unproject_numerator / (depth + constants.depth_unproject_denominator_offset);
			assert!(
				(reconstructed - z).abs() <= z.max(1.0) * 0.00001,
				"Unexpected GTAO depth reconstruction for z={z}: {reconstructed}"
			);
		}

		for pixel in [[0.0f32, 0.0], [479.0, 269.0], [959.0, 539.0]] {
			let ray = [
				pixel[0] * constants.pixel_to_ray_mul[0] + constants.pixel_to_ray_add[0],
				pixel[1] * constants.pixel_to_ray_mul[1] + constants.pixel_to_ray_add[1],
			];
			let ndc = [
				2.0 * (pixel[0] + 0.5) / gtao_extent.width() as f32 - 1.0,
				1.0 - 2.0 * (pixel[1] + 0.5) / gtao_extent.height() as f32,
			];
			assert!((ray[0] - ndc[0] / projection[0]).abs() < 0.000001);
			assert!((ray[1] - ndc[1] / projection[5]).abs() < 0.000001);
		}

		assert_eq!(constants.view_z_sign, 1.0);
		assert_eq!(gtao_extent, Extent::rectangle(960, 540));
		assert_eq!(
			gtao_half_resolution_extent(Extent::rectangle(1919, 1079)),
			Extent::rectangle(959, 539)
		);
		assert_eq!(gtao_half_resolution_extent(Extent::square(1)), Extent::square(1));
	}
}
