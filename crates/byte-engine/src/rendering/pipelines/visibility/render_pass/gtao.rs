//! Ground-truth ambient occlusion computed at half resolution from the linear depth pyramid, then denoised and upscaled.
//!
//! Material evaluation combines the result with the hit fraction of [`super::ssgi::SsgiPass`] when both run, and
//! darkens environment diffuse and specular light by it. Turn it off with `render.gtao.enabled`.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::depth_pyramid::DEPTH_PYRAMID_MIP_COUNT;
use super::{ComputeStage, Pipelines};
use crate::configuration::ConfigurationValue;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink};

/// The evaluate, blur, and upscale pipelines, named as [`Pipelines::request`] takes them.
pub(in crate::rendering::pipelines::visibility) const GTAO_PIPELINES: [&str; 3] = ["gtao", "gtao-blur-x", "gtao-upscale"];
const MIN_SAMPLES_PER_RAY: u32 = 1;
const MAX_SAMPLES_PER_RAY: u32 = 32;
const MIN_RADIAL_RAYS: u32 = 2;
const MAX_RADIAL_RAYS: u32 = 32;

/// The `GtaoSettings` struct defines the runtime quality and world-space search controls for GTAO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GtaoSettings {
	/// Whether the pass runs. Without it, material evaluation takes occlusion from SSGI alone, or applies none.
	pub(crate) enabled: bool,
	pub(crate) radius: f32,
	pub(crate) samples_per_ray: u32,
	pub(crate) radial_rays: u32,
}

impl Default for GtaoSettings {
	fn default() -> Self {
		Self {
			enabled: true,
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
			"enabled" => {
				let enabled = value.to_bool().ok_or(
					"GTAO enabled was not set. The most likely cause is that the value is neither `true` nor `false`.",
				)?;
				Ok((Self { enabled, ..self }, ConfigurationValue::Bool(enabled)))
			}
			"radius" => {
				let radius = value
					.to_f64()
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
				let samples_per_ray = value.to_u32()
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
				let radial_rays = value.to_u32()
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct GtaoShaderParameters {
	radius: f32,
	samples_per_ray: u32,
	radial_rays: u32,
}

/// The `GtaoPass` struct builds a depth-based ambient occlusion term before material evaluation shades the frame.
pub(super) struct GtaoPass {
	/// The set that holds the screen view constants, which the evaluate and upscale stages read.
	view_descriptor_set: ghi::DescriptorSetHandle,
	gtao_descriptor_set: ghi::DescriptorSetHandle,
	blur_descriptor_set: ghi::DescriptorSetHandle,
	upscale_descriptor_set: ghi::DescriptorSetHandle,
	/// The evaluate, blur, and upscale pipelines.
	pub(super) pipelines: Pipelines<3>,
	parameters: ghi::DynamicBufferHandle<GtaoShaderParameters>,
	raw_ao_map: ghi::DynamicImageHandle,
	blurred_ao_map: ghi::DynamicImageHandle,
	ao_map: ghi::BaseImageHandle,
}

impl GtaoPass {
	/// Creates the intermediate images and wires the three-stage descriptor graph.
	///
	/// `depth_pyramid` and `view_descriptor_set`, the set that holds the screen view constants, come from
	/// [`super::depth_pyramid::DepthPyramidPass`], which must run first.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		view_descriptor_set: ghi::DescriptorSetHandle,
		ao_map: ghi::BaseImageHandle,
	) -> Self {
		let gtao_descriptor_set = set_layouts::GTAO.create_set(context, "GTAO Descriptor Set");
		let blur_descriptor_set = set_layouts::GTAO_BLUR.create_set(context, "GTAO Blur X Descriptor Set");
		let upscale_descriptor_set = set_layouts::GTAO_UPSCALE.create_set(context, "GTAO Depth-Aware Upscale Descriptor Set");
		let dynamic_buffer = |name| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let parameters = context.build_dynamic_buffer(dynamic_buffer("GTAO Parameters"));
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let ao_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {}),
		);
		let half_resolution_image = |name| {
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let raw_ao_map = context.build_dynamic_image(half_resolution_image("GTAO Half-Resolution Raw"));
		let blurred_ao_map = context.build_dynamic_image(half_resolution_image("GTAO Half-Resolution Blur Intermediate"));
		let evaluate = set_layouts::GTAO.binder(gtao_descriptor_set);
		let blur = set_layouts::GTAO_BLUR.binder(blur_descriptor_set);
		let upscale = set_layouts::GTAO_UPSCALE.binder(upscale_descriptor_set);
		context.write(&[
			evaluate.buffer("gtao_parameters", parameters),
			evaluate.sampled("depth_pyramid", depth_pyramid, depth_sampler),
			evaluate.image("ao_output", raw_ao_map),
			blur.sampled("linear_depth", depth_pyramid, depth_sampler),
			blur.sampled("ao_source", raw_ao_map, ao_sampler),
			blur.image("ao_output", blurred_ao_map),
			upscale.sampled("visibility_depth", depth, depth_sampler),
			upscale.sampled("ao_source", blurred_ao_map, ao_sampler),
			upscale.image("ao_output", ao_map),
			upscale.sampled("low_resolution_depth", depth_pyramid, depth_sampler),
		]);

		Self {
			view_descriptor_set,
			gtao_descriptor_set,
			blur_descriptor_set,
			upscale_descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, GTAO_PIPELINES),
			parameters,
			raw_ao_map,
			blurred_ao_map,
			ao_map,
		}
	}

	/// Uploads this frame's `settings`, resizes the intermediates, and returns the three-stage recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		settings: GtaoSettings,
		[gtao, blur, upscale]: [ghi::PipelineHandle; 3],
	) -> GtaoStages {
		let extent = sink.extent();
		let gtao_extent = extent.scaled_down(2);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = GtaoShaderParameters {
			radius: settings.radius,
			samples_per_ray: settings.samples_per_ray,
			radial_rays: settings.radial_rays,
		};
		frame.sync_buffer(self.parameters);
		frame.resize_image(self.ao_map, extent);
		frame.resize_image(self.raw_ao_map.into(), gtao_extent);
		frame.resize_image(self.blurred_ao_map.into(), gtao_extent);

		GtaoStages {
			evaluate: ComputeStage {
				label: "GTAO Evaluate",
				pipeline: gtao,
				descriptor_sets: [self.view_descriptor_set, self.gtao_descriptor_set],
				extent: gtao_extent,
				workgroup: Extent::new(16, 8, 1),
			},
			blur: ComputeStage {
				label: "GTAO Denoise Horizontal",
				pipeline: blur,
				descriptor_sets: [self.blur_descriptor_set],
				extent: gtao_extent,
				workgroup: Extent::new(8, 8, 1),
			},
			upscale: ComputeStage {
				label: "GTAO Denoise and Depth-Aware Upscale",
				pipeline: upscale,
				descriptor_sets: [self.view_descriptor_set, self.upscale_descriptor_set],
				extent,
				workgroup: Extent::new(8, 8, 1),
			},
		}
	}
}

/// The `GtaoStages` struct holds one frame's three GTAO dispatches in order, so the visibility pass can record each
/// alongside the other screen-space passes' stage of the same rank.
#[derive(Clone, Copy)]
pub(super) struct GtaoStages {
	/// The half-resolution evaluation; it reads only the depth pyramid.
	pub(super) evaluate: ComputeStage<2>,
	/// The horizontal denoise; it reads the evaluation.
	pub(super) blur: ComputeStage<1>,
	/// The full-resolution denoise and upscale; it reads the blur.
	pub(super) upscale: ComputeStage<2>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn enabled_toggles_only_whether_the_pass_runs() {
		let settings = GtaoSettings::default();
		let (disabled, effective) = settings
			.with_parameter("enabled", &ConfigurationValue::Text("false".to_string()))
			.unwrap();
		assert_eq!(effective, ConfigurationValue::Bool(false));
		assert_eq!(
			disabled,
			GtaoSettings {
				enabled: false,
				..settings
			}
		);
		assert!(disabled.with_parameter("enabled", &ConfigurationValue::Integer(1)).is_err());
	}
}
