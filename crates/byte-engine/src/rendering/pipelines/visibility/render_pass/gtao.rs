//! Ground-truth ambient occlusion and its bent normal, computed at half resolution from the linear depth pyramid, then
//! denoised, and the occlusion upscaled.
//!
//! Material evaluation combines the occlusion with the hit fraction of the indirect diffuse source when both run, and
//! darkens environment diffuse light by it. Specular occlusion intersects the reflection's lobe with the cone of open
//! directions around the bent normal. Turn it off with `render.gtao.enabled`.

use ghi::context::ContextCreate as _;
use ghi::frame::Frame as _;
use utils::Extent;

use super::min_depth_pyramid::MIN_DEPTH_PYRAMID_MIP_COUNT;
use super::{ComputeStage, Pipelines};
use crate::configuration::{Accepts, Read, Setting};
use crate::rendering::lookup_textures::blue_noise_slice;
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
				ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image).name(name),
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
	/// The runtime settings, named inside the `render.gtao.` port.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[
		Setting {
			name: "enabled",
			accepts: Accepts::Bool,
			value: |settings| Read::Bool(settings.enabled),
			apply: |settings, read| {
				let enabled = read.bool();
				Ok(Self { enabled, ..settings })
			},
		},
		Setting {
			name: "radius",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.radius)),
			apply: |settings, read| {
				let radius = read.number();
				Ok(Self {
					radius: radius as f32,
					..settings
				})
			},
		},
		Setting {
			name: "samples-per-ray",
			accepts: Accepts::Whole {
				min: MIN_SAMPLES_PER_RAY,
				max: MAX_SAMPLES_PER_RAY,
				step: 1,
			},
			value: |settings| Read::Whole(settings.samples_per_ray),
			apply: |settings, read| {
				let samples_per_ray = read.whole();
				Ok(Self {
					samples_per_ray,
					..settings
				})
			},
		},
		Setting {
			name: "radial-rays",
			accepts: Accepts::Whole {
				min: MIN_RADIAL_RAYS,
				max: MAX_RADIAL_RAYS,
				step: 2,
			},
			value: |settings| Read::Whole(settings.radial_rays),
			apply: |settings, read| {
				let radial_rays = read.whole();
				Ok(Self { radial_rays, ..settings })
			},
		},
	];
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct GtaoShaderParameters {
	radius: f32,
	samples_per_ray: u32,
	radial_rays: u32,
	/// The slice of the engine's blue noise the evaluation reads; see [`blue_noise_slice`].
	noise_slice: u32,
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
	parameters: ghi::BufferHandle<GtaoShaderParameters>,
}

impl GtaoPass {
	/// Wires the three-stage descriptor graph through `targets`, which [`create_gtao_targets`] creates.
	///
	/// `min_depth_pyramid` and `view_descriptor_set`, the set that holds the screen view constants, come from
	/// [`super::min_depth_pyramid::MinDepthPyramidPass`], which must run first.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		min_depth_pyramid: ghi::ImageHandle,
		view_descriptor_set: ghi::DescriptorSetHandle,
		targets: GtaoTargets,
		blue_noise: ghi::ImageHandle,
	) -> Self {
		let parameters = context.build_buffer(super::frame_buffer("GTAO Parameters"));
		let depth_sampler = context.build_sampler(super::border_sampler().max_lod((MIN_DEPTH_PYRAMID_MIP_COUNT - 1) as f32));
		let ao_sampler = context.build_sampler(super::border_sampler());
		// The evaluation reads it texel by texel at the pixel modulo its side. GTAO keeps no history, so the slice changes
		// only while the sink is jittered, when temporal antialiasing averages the frames; see [`blue_noise_slice`].
		let gtao_descriptor_set = set_layouts::create_set(context, set_layouts::GTAO, "GTAO Descriptor Set", |set| {
			[
				set.buffer("gtao_parameters", parameters),
				set.texture("depth_pyramid", min_depth_pyramid),
				set.image("ao_output", targets.raw),
				set.image("bent_normal_output", targets.raw_bent_normal),
				set.sampled("blue_noise", blue_noise, ao_sampler),
			]
		});
		let blur_descriptor_set =
			set_layouts::create_set(context, set_layouts::GTAO_BLUR, "GTAO Blur X Descriptor Set", |set| {
				[
					set.sampled("linear_depth", min_depth_pyramid, depth_sampler),
					set.sampled("ao_source", targets.raw, ao_sampler),
					set.image("ao_output", targets.blurred),
					set.sampled("bent_normal_source", targets.raw_bent_normal, ao_sampler),
					set.image("bent_normal_output", targets.bent_normal),
				]
			});
		let upscale_descriptor_set = set_layouts::create_set(
			context,
			set_layouts::GTAO_UPSCALE,
			"GTAO Depth-Aware Upscale Descriptor Set",
			|set| {
				[
					set.sampled("visibility_depth", depth, depth_sampler),
					set.texture("ao_source", targets.blurred),
					set.image("ao_output", targets.ambient_occlusion),
					set.texture("low_resolution_depth", min_depth_pyramid),
				]
			},
		);

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
		*frame.map(self.parameters) = GtaoShaderParameters {
			radius: settings.radius,
			samples_per_ray: settings.samples_per_ray,
			radial_rays: settings.radial_rays,
			noise_slice: blue_noise_slice(sink, frame.key().frame_index()),
		};

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
