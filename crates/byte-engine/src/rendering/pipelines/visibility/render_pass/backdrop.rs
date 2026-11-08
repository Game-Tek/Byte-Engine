//! The opaque scene behind blended surfaces, copied before the transparent phase draws over it.
//!
//! Blended materials read the scene behind them at any screen position, for refraction and depth tints. The
//! transparent dispatch writes `Lit` at its own pixel while other threads would read it at theirs, so [`BackdropPass`]
//! copies the light and the depth into images of their own first. Material evaluation reads them through the
//! `backdrop_color` and `backdrop_depth` bindings of its set.

use ghi::context::Context as _;
use utils::Extent;

use super::{ComputeStage, Pipelines};
use crate::rendering::PipelineManagerClient;
use crate::rendering::pipelines::visibility::set_layouts;

/// The `BackdropTargets` struct names the full-size images the backdrop pass writes and blended materials read.
#[derive(Clone, Copy)]
pub(crate) struct BackdropTargets {
	/// The opaque scene's light, already multiplied by exposure, in the scene color format.
	pub(crate) color: ghi::BaseImageHandle,
	/// The opaque scene's positive linear view depth, with the sky at the view's far depth.
	pub(crate) depth: ghi::BaseImageHandle,
}

/// Creates one sink's backdrop images. Next, give them to [`BackdropPass::new`] in the sink's [`super::SinkTargets`].
pub(crate) fn create_backdrop_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> BackdropTargets {
	let mut target = |format, name| {
		render_pass_builder
			.create_render_target(
				ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
			.into()
	};
	BackdropTargets {
		color: target(crate::rendering::SCENE_COLOR_FORMAT, "Backdrop Color"),
		depth: target(ghi::Formats::R32F, "Backdrop Depth"),
	}
}

/// The `BackdropPass` struct copies one sink's opaque scene for the blended materials that read the scene behind
/// them.
///
/// Create it with the sink's targets, then call [`Self::prepare`] in each frame whose transparent phase reads the
/// backdrop, and record the stage after the scene background and before the transparent visibility draw.
pub(super) struct BackdropPass {
	/// The screen view constants that unproject depth, then the backdrop set.
	descriptor_sets: [ghi::DescriptorSetHandle; 2],
	pipelines: Pipelines<1>,
}

impl BackdropPass {
	/// Binds `lit` and `depth`, this frame's copies of the sink's light and reverse-Z depth, as the sources, and
	/// `targets` as both the copies and the material evaluation set's backdrop.
	///
	/// `view_descriptor_set` holds the depth pyramid's screen view constants, whose depth unprojection holds at every
	/// resolution, and `point_sampler` reads one texel without filtering.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		view_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		point_sampler: ghi::SamplerHandle,
		lit: ghi::DynamicImageHandle,
		depth: ghi::BaseImageHandle,
		targets: BackdropTargets,
	) -> Self {
		let descriptor_set = set_layouts::BACKDROP.create_set(context, "Backdrop Descriptor Set");
		let backdrop = set_layouts::BACKDROP.binder(descriptor_set);
		let evaluation = set_layouts::MATERIAL_EVALUATION.binder(material_evaluation_descriptor_set);
		context.write(&[
			backdrop.sampled("source_lit", lit, point_sampler),
			backdrop.sampled("source_depth", depth, point_sampler),
			backdrop.image("color", targets.color),
			backdrop.image("depth", targets.depth),
			// The layer fixes both samplers: filtered light, and depth read at the pixel it falls in.
			evaluation.texture("backdrop_color", targets.color),
			evaluation.texture("backdrop_depth", targets.depth),
		]);

		Self {
			descriptor_sets: [view_descriptor_set, descriptor_set],
			pipelines: Pipelines::request(pipeline_manager, ["backdrop"]),
		}
	}

	/// Returns the copy of a sink of `extent`, or `None` while its pipeline is still compiling.
	pub(super) fn prepare(&self, pipeline_manager: &PipelineManagerClient, extent: Extent) -> Option<ComputeStage<2>> {
		let [pipeline] = self.pipelines.resolve(pipeline_manager)?;

		Some(ComputeStage {
			label: "Backdrop",
			pipeline,
			descriptor_sets: self.descriptor_sets,
			extent,
			workgroup: Extent::new(8, 8, 1),
		})
	}
}
