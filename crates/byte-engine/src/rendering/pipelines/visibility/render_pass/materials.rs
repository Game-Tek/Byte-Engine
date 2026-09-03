//! Material dispatch bookkeeping and evaluation: count pixels per material, prefix-sum offsets, map pixels, shade.

use ghi::context::{Context as _, ContextCreate as _};
use utils::{Extent, RGBA};

use super::super::layout::{ActiveMaterialMask, MAX_MATERIALS, MAX_PIXEL_MAPPING_ENTRIES};
use super::super::scene::MaterialEntry;
use super::visibility::VisibilityPhase;
use crate::rendering::PipelineManagerClient;
use crate::rendering::render_pass::RenderPassFunction;

/// Returns whether this frame contains geometry that uses one material in the requested visibility phase.
pub(super) fn material_is_active(active_materials: &ActiveMaterialMask, material_index: u32) -> bool {
	let material_index = material_index as usize;
	active_materials
		.get(material_index / u64::BITS as usize)
		.is_some_and(|word| word & (1u64 << (material_index % u64::BITS as usize)) != 0)
}

/// The `MaterialBuffers` struct owns the per-sink buffers the material prepasses write and evaluation reads.
pub(super) struct MaterialBuffers {
	pub(super) count: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pub(super) offset: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pub(super) offset_scratch: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pub(super) evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	pub(super) pixel_mapping: ghi::BufferHandle<[[u16; 2]; MAX_PIXEL_MAPPING_ENTRIES]>,
}

impl MaterialBuffers {
	pub(super) fn new(context: &mut ghi::implementation::Context) -> Self {
		let build = |name, extra_uses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | extra_uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		Self {
			count: context.build_buffer(build("Material Count", ghi::Uses::empty())),
			offset: context.build_buffer(build("Material Offset", ghi::Uses::empty())),
			offset_scratch: context.build_buffer(build("Material Offset Scratch", ghi::Uses::empty())),
			evaluation_dispatches: context.build_buffer(build("Material Evaluation Dispatches", ghi::Uses::Indirect)),
			pixel_mapping: context.build_buffer(build("Material XY", ghi::Uses::empty())),
		}
	}
}

/// The `MaterialPrepasses` struct runs the three compute passes that turn the visibility buffer into per-material pixel lists.
pub(super) struct MaterialPrepasses {
	base_descriptor_set: ghi::DescriptorSetHandle,
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	count_pipeline: crate::rendering::PipelineRef,
	offset_pipeline: crate::rendering::PipelineRef,
	pixel_mapping_pipeline: crate::rendering::PipelineRef,
}

#[derive(Clone, Copy)]
pub(super) struct MaterialPrepassPipelines {
	count: ghi::PipelineHandle,
	offset: ghi::PipelineHandle,
	pixel_mapping: ghi::PipelineHandle,
}

impl MaterialPrepasses {
	pub(super) fn new(
		pipeline_manager: &PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	) -> Self {
		Self {
			base_descriptor_set,
			visibility_descriptor_set,
			count_buffer,
			count_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/material-count.pipeline"),
			offset_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/material-offset.pipeline"),
			pixel_mapping_pipeline: pipeline_manager
				.request_pipeline("byte-engine/rendering/visibility/pixel-mapping.pipeline"),
		}
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<MaterialPrepassPipelines> {
		Some(MaterialPrepassPipelines {
			count: pipeline_manager.pipeline(self.count_pipeline)?,
			offset: pipeline_manager.pipeline(self.offset_pipeline)?,
			pixel_mapping: pipeline_manager.pipeline(self.pixel_mapping_pipeline)?,
		})
	}

	/// Records count, offset, and pixel-mapping for the visibility buffer currently in `extent`.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		pipelines: MaterialPrepassPipelines,
	) {
		use ghi::command_buffer::{
			BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
			CommonCommandBufferMode as _,
		};

		let descriptor_sets = [self.base_descriptor_set, self.visibility_descriptor_set];
		let dispatch = |c: &mut ghi::implementation::CommandBufferRecording, name, pipeline, extent, workgroup| {
			c.start_region(|label| label.write_str(name));
			let c = c.bind_compute_pipeline(pipeline);
			c.bind_descriptor_sets(&descriptor_sets);
			c.dispatch(ghi::DispatchExtent::new(extent, workgroup));
			c.end_region();
		};
		// The offset pass reads these counts without resetting them, so clear before every dispatch.
		c.clear_buffers(&[self.count_buffer.into()]);
		dispatch(c, "Material Count", pipelines.count, extent, Extent::square(8));
		dispatch(c, "Material Offset", pipelines.offset, Extent::line(1), Extent::line(1));
		dispatch(c, "Pixel Mapping", pipelines.pixel_mapping, extent, Extent::square(16));
	}
}

/// The `MaterialEvaluationPass` struct shades every material's pixel list into the lit target.
pub(super) struct MaterialEvaluationPass {
	lit: ghi::BaseImageHandle,
	base_descriptor_set: ghi::DescriptorSetHandle,
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	pub(super) fn new(
		lit: ghi::BaseImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		Self {
			lit,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			evaluation_dispatches,
		}
	}

	/// Prepares one material phase; the opaque phase clears the lit target, the transparent phase composites over it.
	pub(super) fn prepare<'a>(
		&self,
		materials: &'a [MaterialEntry],
		active_materials: &'a ActiveMaterialMask,
		phase: VisibilityPhase,
	) -> impl RenderPassFunction + use<'a> {
		let lit = self.lit;
		let descriptor_sets = [self.base_descriptor_set, self.visibility_descriptor_set, self.descriptor_set];
		let evaluation_dispatches = self.evaluation_dispatches;

		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			if phase == VisibilityPhase::Opaque {
				c.clear_images(&[(lit, ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0)))]);
			}
			let active = materials
				.iter()
				.filter(|(_, index, _)| material_is_active(active_materials, *index));
			c.start_region(|label| {
				label.write_str(phase.label())?;
				label.write_str(" Material Evaluation")
			});
			// Materials sharing a pipeline are adjacent, so the binding survives across consecutive dispatches.
			let mut bound_pipeline = None;
			for (name, index, pipeline) in active {
				c.start_region(|label| label.write_str(name));
				if bound_pipeline != Some(*pipeline) {
					let c = c.bind_compute_pipeline(*pipeline);
					c.bind_descriptor_sets(&descriptor_sets);
					bound_pipeline = Some(*pipeline);
				}
				c.write_push_constant(0, [*index, phase.blend_flag()]);
				c.indirect_dispatch(evaluation_dispatches, *index as usize);
				c.end_region();
			}
			c.end_region();
		}
	}
}
