//! Rasterizes meshlets into the visibility buffer: per-pixel triangle and instance identifiers plus depth.

use utils::Extent;

use super::super::mesh_dispatch::MeshDispatch;
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
	pub(super) pipeline: crate::rendering::PipelineRef,
	pub(super) masked_pipeline: crate::rendering::PipelineRef,
	primitive_index: ghi::BaseImageHandle,
	instance_id: ghi::BaseImageHandle,
	depth: ghi::BaseImageHandle,
}

#[derive(Clone, Copy)]
pub(super) struct VisibilityPipelines {
	pub(super) opaque: ghi::PipelineHandle,
	pub(super) masked: ghi::PipelineHandle,
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
			pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/visibility.pipeline"),
			masked_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/masked-visibility.pipeline"),
			primitive_index,
			instance_id,
			depth,
		}
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<VisibilityPipelines> {
		Some(VisibilityPipelines {
			opaque: pipeline_manager.pipeline(self.pipeline)?,
			masked: pipeline_manager.pipeline(self.masked_pipeline)?,
		})
	}

	/// Records the solid and masked dispatches of one phase into the visibility buffers.
	///
	/// The transparent phase loads opaque depth, then writes the nearest transparent surface into it. This
	/// preserves opaque occlusion while resolving overlapping triangles within the single transparent layer.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		phase: VisibilityPhase,
		solid: MeshDispatch,
		masked: MeshDispatch,
		pipelines: VisibilityPipelines,
	) {
		use ghi::command_buffer::{
			BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
			CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
		};

		let identifier = |image| {
			ghi::AttachmentInformation::new(
				image,
				ghi::Layouts::RenderTarget,
				ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
				false,
				true,
			)
		};
		let attachments = [
			identifier(self.primitive_index),
			identifier(self.instance_id),
			ghi::AttachmentInformation::new(
				self.depth,
				ghi::Layouts::RenderTarget,
				ghi::ClearValue::Depth(0.0),
				phase == VisibilityPhase::Transparent,
				true,
			),
		];

		c.start_region(|label| {
			label.write_str(phase.label())?;
			label.write_str(" Visibility Buffer")
		});
		let c = c.start_render_pass(extent, &attachments);
		for (dispatch, pipeline) in [(solid, pipelines.opaque), (masked, pipelines.masked)] {
			if dispatch.is_empty() {
				continue;
			}
			let c = c.bind_raster_pipeline(pipeline);
			c.bind_descriptor_sets(&[self.descriptor_set]);
			c.write_push_constant(0, dispatch.work_item_base());
			c.write_push_constant(4, 0u32);
			c.write_push_constant(8, 0u32);
			c.dispatch_meshes(dispatch.workgroup_count(), 1, 1);
		}
		c.end_render_pass();
		c.end_region();
	}
}
