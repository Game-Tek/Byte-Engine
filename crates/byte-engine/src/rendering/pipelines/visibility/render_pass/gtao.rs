//! Ground-truth ambient occlusion and its bent normal, computed at half resolution from the linear depth pyramid, then
//! denoised, and the occlusion upscaled.
//!
//! Material evaluation combines the occlusion with the hit fraction of the indirect diffuse source when both run, and
//! darkens environment diffuse light by it. Specular occlusion intersects the reflection's lobe with the cone of open
//! directions around the bent normal. Turn it off with `render.gtao.enabled`.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::depth_pyramid::DEPTH_PYRAMID_MIP_COUNT;
use super::{ComputeStage, Pipelines};
use crate::configuration::ConfigurationValue;
use crate::rendering::lookup_textures::create_blue_noise_texture;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink};

/// The evaluate, blur, and upscale pipelines, named as [`Pipelines::request`] takes them.
pub(in super::super) const GTAO_PIPELINES: [&str; 3] = ["gtao", "gtao-blur-x", "gtao-upscale"];
const MIN_SAMPLES_PER_RAY: u32 = 1;
const MAX_SAMPLES_PER_RAY: u32 = 32;
const MIN_RADIAL_RAYS: u32 = 2;
const MAX_RADIAL_RAYS: u32 = 32;

/// GTAO evaluates and denoises at half the sink resolution.
const GTAO_RESOLUTION_DIVISOR: u32 = 2;

/// The `GtaoTargets` struct holds the images the three GTAO stages write, so the visibility pass can hand them to
/// [`GtaoPass::new`] and bind the result in material evaluation.
#[derive(Clone, Copy)]
pub(crate) struct GtaoTargets {
	/// The noisy half-resolution evaluation, named `GTAO Raw`.
	pub(crate) raw: ghi::BaseImageHandle,
	/// The horizontally denoised half-resolution occlusion, named `GTAO Blur`.
	pub(crate) blurred: ghi::BaseImageHandle,
	/// The full-resolution result material evaluation reads, named `Ambient Occlusion`.
	pub(crate) ambient_occlusion: ghi::BaseImageHandle,
	/// The noisy half-resolution bent normal, as its two components across the view vector, named `GTAO Bent Normal`.
	pub(crate) raw_bent_normal: ghi::BaseImageHandle,
	/// The horizontally denoised bent normal material evaluation reads, named `GTAO Bent Normal Blur`.
	pub(crate) bent_normal: ghi::BaseImageHandle,
}

/// Creates the GTAO render targets for the sink that `render_pass_builder` sets up.
///
/// They are render targets so the renderer sizes them with the sink and the inspector can capture them by name.
/// Material evaluation binds the result even when GTAO is not built in, so every sink has them.
pub(crate) fn create_gtao_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> GtaoTargets {
	let mut target = |name, format, resolution_divisor| {
		render_pass_builder
			.create_scaled_render_target(
				ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				resolution_divisor,
			)
			.into()
	};
	GtaoTargets {
		raw: target("GTAO Raw", ghi::Formats::R8UNORM, GTAO_RESOLUTION_DIVISOR),
		blurred: target("GTAO Blur", ghi::Formats::R8UNORM, GTAO_RESOLUTION_DIVISOR),
		ambient_occlusion: target("Ambient Occlusion", ghi::Formats::R8UNORM, 1),
		raw_bent_normal: target("GTAO Bent Normal", ghi::Formats::RG8UNORM, GTAO_RESOLUTION_DIVISOR),
		bent_normal: target("GTAO Bent Normal Blur", ghi::Formats::RG8UNORM, GTAO_RESOLUTION_DIVISOR),
	}
}

/// The `GtaoSettings` struct defines the runtime quality and world-space search controls for GTAO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GtaoSettings {
	/// Whether the pass runs. Without it, material evaluation takes occlusion from the indirect diffuse source alone, or
	/// applies none.
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
}

impl GtaoPass {
	/// Wires the three-stage descriptor graph through `targets`, which [`create_gtao_targets`] creates.
	///
	/// `depth_pyramid` and `view_descriptor_set`, the set that holds the screen view constants, come from
	/// [`super::depth_pyramid::DepthPyramidPass`], which must run first.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		view_descriptor_set: ghi::DescriptorSetHandle,
		targets: GtaoTargets,
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
		// The evaluation reads it texel by texel at the pixel modulo its side, the same pattern every frame: nothing
		// accumulates GTAO over frames, so animating it would only flicker.
		let blue_noise = create_blue_noise_texture(context);
		let evaluate = set_layouts::GTAO.binder(gtao_descriptor_set);
		let blur = set_layouts::GTAO_BLUR.binder(blur_descriptor_set);
		let upscale = set_layouts::GTAO_UPSCALE.binder(upscale_descriptor_set);
		context.write(&[
			evaluate.buffer("gtao_parameters", parameters),
			evaluate.texture("depth_pyramid", depth_pyramid),
			evaluate.image("ao_output", targets.raw),
			evaluate.image("bent_normal_output", targets.raw_bent_normal),
			evaluate.sampled("blue_noise", blue_noise, ao_sampler),
			blur.sampled("linear_depth", depth_pyramid, depth_sampler),
			blur.sampled("ao_source", targets.raw, ao_sampler),
			blur.image("ao_output", targets.blurred),
			blur.sampled("bent_normal_source", targets.raw_bent_normal, ao_sampler),
			blur.image("bent_normal_output", targets.bent_normal),
			upscale.sampled("visibility_depth", depth, depth_sampler),
			upscale.texture("ao_source", targets.blurred),
			upscale.image("ao_output", targets.ambient_occlusion),
			upscale.texture("low_resolution_depth", depth_pyramid),
		]);

		Self {
			view_descriptor_set,
			gtao_descriptor_set,
			blur_descriptor_set,
			upscale_descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, GTAO_PIPELINES),
			parameters,
		}
	}

	/// Uploads this frame's `settings` and returns the three-stage recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		settings: GtaoSettings,
		[gtao, blur, upscale]: [ghi::PipelineHandle; 3],
	) -> GtaoStages {
		let extent = sink.extent();
		let gtao_extent = extent.scaled_down(GTAO_RESOLUTION_DIVISOR);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = GtaoShaderParameters {
			radius: settings.radius,
			samples_per_ray: settings.samples_per_ray,
			radial_rays: settings.radial_rays,
		};
		frame.sync_buffer(self.parameters);

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
			// Each upscale thread writes a 2x2 quad of the full-resolution image.
			upscale: ComputeStage {
				label: "GTAO Denoise and Depth-Aware Upscale",
				pipeline: upscale,
				descriptor_sets: [self.view_descriptor_set, self.upscale_descriptor_set],
				extent: Extent::rectangle(extent.width().div_ceil(2), extent.height().div_ceil(2)),
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
