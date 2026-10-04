//! Rasterizes meshlets into the visibility buffer: per-pixel triangle and instance identifiers plus depth.

use utils::Extent;

use super::super::mesh_dispatch::PhaseDispatches;
use super::{OcclusionPhase, Pipelines, record_meshlet_dispatches};

/// The `VisibilityPhase` enum selects between the opaque layer and the single depth-resolved transparent layer.
///
/// Material evaluation pushes the discriminant as its blend flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum VisibilityPhase {
	Opaque = 0,
	Transparent = 1,
}

impl VisibilityPhase {
	pub(super) fn label(self) -> &'static str {
		match self {
			Self::Opaque => "Opaque",
			Self::Transparent => "Transparent",
		}
	}
}

/// The `VisibilityPass` struct owns the depth-writing raster state used to populate the visibility buffers.
pub(super) struct VisibilityPass {
	/// The base set and the sink's occlusion culling set, which the shadow maps bind too.
	pub(super) descriptor_sets: [ghi::DescriptorSetHandle; 2],
	/// The double-sided pipelines run without back-face culling, and only the masked ones run the alpha test.
	pub(super) pipelines: Pipelines<4>,
	pub(super) primitive_index: ghi::BaseImageHandle,
	pub(super) instance_id: ghi::BaseImageHandle,
	pub(super) depth: ghi::BaseImageHandle,
}

impl VisibilityPass {
	/// Records the work ranges of one phase into the visibility buffers: the solid, masked, and both double-sided
	/// ranges for the opaque phase, or the transparent range.
	///
	/// The transparent phase loads opaque depth, then writes the nearest transparent surface into it. This
	/// preserves opaque occlusion while resolving overlapping triangles within the single transparent layer.
	///
	/// `occlusion` selects how the pass culls by occlusion; see [`record_meshlet_dispatches`]. The
	/// [`OcclusionPhase::Late`] opaque pass loads the early pass's identifiers and depth and draws over them.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		phase: VisibilityPhase,
		dispatches: PhaseDispatches,
		pipelines: [ghi::PipelineHandle; 4],
		occlusion: OcclusionPhase,
	) {
		use ghi::command_buffer::{
			CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
		};

		let continues_early_pass = occlusion == OcclusionPhase::Late;
		let identifier = |image| {
			ghi::AttachmentInformation::new(
				image,
				ghi::Layouts::RenderTarget,
				if continues_early_pass {
					ghi::LoadOp::Load
				} else {
					ghi::LoadOp::Clear(ghi::ClearValue::Integer(u32::MAX, 0, 0, 0))
				},
				ghi::StoreOp::Store,
			)
		};
		let attachments = [
			identifier(self.primitive_index),
			identifier(self.instance_id),
			ghi::AttachmentInformation::new(
				self.depth,
				ghi::Layouts::RenderTarget,
				if phase == VisibilityPhase::Transparent || continues_early_pass {
					ghi::LoadOp::Load
				} else {
					ghi::LoadOp::Clear(ghi::ClearValue::Depth(0.0))
				},
				ghi::StoreOp::Store,
			),
		];

		c.start_region(|label| {
			label.write_str(phase.label())?;
			label.write_str(" Visibility Buffer")?;
			label.write_str(match occlusion {
				OcclusionPhase::Early => " (Early)",
				OcclusionPhase::Late => " (Late)",
				OcclusionPhase::Disabled | OcclusionPhase::Test => "",
			})
		});
		let c = c.start_render_pass(extent, &attachments);
		// Blend materials have no alpha test and keep back-face culling, so they pair with the solid pipeline.
		let ranges = match phase {
			VisibilityPhase::Opaque => &dispatches.opaque_layer[..],
			VisibilityPhase::Transparent => std::slice::from_ref(&dispatches.transparent),
		};
		let ranges = ranges.iter().copied().zip(pipelines);
		// The camera is view zero.
		record_meshlet_dispatches(c, self.descriptor_sets, occlusion, ranges, 0, 1);
		c.end_render_pass();
		c.end_region();
	}
}
