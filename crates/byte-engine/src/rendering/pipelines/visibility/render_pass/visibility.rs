use super::*;

/// The `VisibilityPass` struct owns the depth-writing raster state used to populate visibility buffers.
#[derive(Clone)]
pub(crate) struct VisibilityPass {
	descriptor_set: ghi::DescriptorSetHandle,
	pub(super) pipeline: crate::rendering::PipelineRef,
	pub(super) masked_pipeline: crate::rendering::PipelineRef,
	opaque_attachments: [ghi::AttachmentInformation; 3],
	transparent_attachments: [ghi::AttachmentInformation; 3],
}

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

impl VisibilityPass {
	/// Creates phase-specific attachment behavior and requests the visibility pipeline.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		depth_target: ghi::BaseImageHandle,
	) -> Self {
		let pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/visibility.pipeline");
		let masked_pipeline = pipeline_manager.request_pipeline("byte-engine/rendering/visibility/masked-visibility.pipeline");

		VisibilityPass {
			descriptor_set,
			pipeline,
			masked_pipeline,
			opaque_attachments: [
				ghi::AttachmentInformation::new(
					primitive_index,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				),
			],
			transparent_attachments: [
				ghi::AttachmentInformation::new(
					primitive_index,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					true,
					true,
				),
			],
		}
	}

	/// Records the solid and masked visibility layers in one pass so depth and IDs are cleared once.
	///
	/// The transparent phase loads opaque depth, then writes the nearest transparent
	/// surface into it. This preserves opaque occlusion while resolving overlapping
	/// triangles within the single transparent layer represented by the visibility buffer.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		instances: &[Instance],
		mesh_dispatch: MeshDispatch,
		masked_instances: &[Instance],
		masked_mesh_dispatch: MeshDispatch,
		phase: VisibilityPhase,
		pipeline: ghi::PipelineHandle,
		masked_pipeline: ghi::PipelineHandle,
	) {
		let attachments: &[ghi::AttachmentInformation] = match phase {
			VisibilityPhase::Opaque => &self.opaque_attachments,
			VisibilityPhase::Transparent => &self.transparent_attachments,
		};
		let drawable_instances = instances
			.iter()
			.chain(masked_instances)
			.filter(|instance| instance.meshlet_count > 0)
			.count();
		let meshlet_count = instances
			.iter()
			.chain(masked_instances)
			.map(|instance| instance.meshlet_count)
			.sum::<u32>();

		log::debug!(
			"{} visibility pass executing: extent={}x{}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
			phase.label(),
			extent.width(),
			extent.height(),
			instances.len() + masked_instances.len(),
			drawable_instances,
			meshlet_count,
			mesh_dispatch.workgroup_count(),
		);
		c.start_region(|label| {
			label.write_str(phase.label())?;
			label.write_str(" Visibility Buffer")
		});

		let c = c.start_render_pass(extent, attachments);
		if !mesh_dispatch.is_empty() {
			let c = c.bind_raster_pipeline(pipeline);
			c.bind_descriptor_sets(&[self.descriptor_set]);
			c.write_push_constant(0, mesh_dispatch.work_item_base());
			c.write_push_constant(4, 0u32);
			c.write_push_constant(8, 0u32);
			c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
		}
		if !masked_mesh_dispatch.is_empty() {
			let c = c.bind_raster_pipeline(masked_pipeline);
			c.bind_descriptor_sets(&[self.descriptor_set]);
			c.write_push_constant(0, masked_mesh_dispatch.work_item_base());
			c.write_push_constant(4, 0u32);
			c.write_push_constant(8, 0u32);
			c.dispatch_meshes(masked_mesh_dispatch.workgroup_count(), 1, 1);
		}

		c.end_render_pass();
		c.end_region();
	}
}

/// Returns the one depth-resolved transparent layer supported by the visibility buffer.
pub(super) fn transparent_visibility_layer(instances: &[Instance]) -> Option<&[Instance]> {
	instances
		.iter()
		.any(|instance| instance.meshlet_count > 0)
		.then_some(instances)
}
