//! Rasterizes meshlets into the visibility buffer: per-pixel triangle and instance identifiers plus depth.

use utils::Extent;

use super::super::mesh_dispatch::PhaseDispatches;
use super::{PhasePipelines, record_meshlet_dispatches};
use crate::rendering::PipelineManagerClient;

/// The `VisibilityPhase` enum selects between the opaque layer and the single depth-resolved transparent layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisibilityPhase {
	Opaque,
	Transparent,
}

impl VisibilityPhase {
	pub(super) fn label(self) -> &'static str {
		match self {
			Self::Opaque => "Opaque",
			Self::Transparent => "Transparent",
		}
	}

	pub(super) fn blend_flag(self) -> u32 {
		match self {
			Self::Opaque => 0,
			Self::Transparent => 1,
		}
	}
}

/// The `VisibilityPass` struct owns the depth-writing raster state used to populate the visibility buffers.
pub(super) struct VisibilityPass {
	descriptor_set: ghi::DescriptorSetHandle,
	/// The double-sided pipelines run without back-face culling, and only the masked ones run the alpha test.
	pipelines: PhasePipelines,
	primitive_index: ghi::BaseImageHandle,
	instance_id: ghi::BaseImageHandle,
	depth: ghi::BaseImageHandle,
}

impl VisibilityPass {
	pub(super) fn new(
		pipeline_manager: &PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		depth: ghi::BaseImageHandle,
	) -> Self {
		Self {
			descriptor_set,
			pipelines: PhasePipelines::request(
				pipeline_manager,
				[
					"byte-engine/rendering/visibility/visibility.pipeline",
					"byte-engine/rendering/visibility/masked-visibility.pipeline",
					"byte-engine/rendering/visibility/double-sided-visibility.pipeline",
					"byte-engine/rendering/visibility/double-sided-masked-visibility.pipeline",
				],
			),
			primitive_index,
			instance_id,
			depth,
		}
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<[ghi::PipelineHandle; 4]> {
		self.pipelines.resolve(pipeline_manager)
	}

	/// Records the work ranges of one phase into the visibility buffers: the solid, masked, and both double-sided
	/// ranges for the opaque phase, or the transparent range.
	///
	/// The transparent phase loads opaque depth, then writes the nearest transparent surface into it. This
	/// preserves opaque occlusion while resolving overlapping triangles within the single transparent layer.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		phase: VisibilityPhase,
		dispatches: PhaseDispatches,
		pipelines: [ghi::PipelineHandle; 4],
	) {
		use ghi::command_buffer::{
			CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
		};

		let identifier = |image| {
			ghi::AttachmentInformation::new(
				image,
				ghi::Layouts::RenderTarget,
				ghi::LoadOp::Clear(ghi::ClearValue::Integer(u32::MAX, 0, 0, 0)),
				ghi::StoreOp::Store,
			)
		};
		let attachments = [
			identifier(self.primitive_index),
			identifier(self.instance_id),
			ghi::AttachmentInformation::new(
				self.depth,
				ghi::Layouts::RenderTarget,
				if phase == VisibilityPhase::Transparent {
					ghi::LoadOp::Load
				} else {
					ghi::LoadOp::Clear(ghi::ClearValue::Depth(0.0))
				},
				ghi::StoreOp::Store,
			),
		];

		c.start_region(|label| {
			label.write_str(phase.label())?;
			label.write_str(" Visibility Buffer")
		});
		let c = c.start_render_pass(extent, &attachments);
		// The camera is view zero. Blend materials have no alpha test and keep back-face culling.
		match phase {
			VisibilityPhase::Opaque => record_meshlet_dispatches(
				c,
				self.descriptor_set,
				dispatches.opaque_layer().into_iter().zip(pipelines),
				0,
				1,
			),
			VisibilityPhase::Transparent => {
				record_meshlet_dispatches(c, self.descriptor_set, [(dispatches.transparent, pipelines[0])], 0, 1)
			}
		}
		c.end_render_pass();
		c.end_region();
	}
}
