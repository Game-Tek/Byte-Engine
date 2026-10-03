//! Material dispatch bookkeeping and evaluation: count pixels per material, prefix-sum offsets, map pixels, shade.

use ghi::context::ContextCreate as _;
use utils::{Extent, RGBA};

use super::super::layout::{ActiveMaterialMask, MAX_MATERIALS, MAX_PIXEL_MAPPING_ENTRIES};
use super::super::scene::MaterialEntry;
use super::visibility::VisibilityPhase;
use crate::rendering::PipelineManagerClient;
use crate::rendering::render_pass::RenderPassFunction;

/// Threads of the one workgroup that scans every material's count into offsets. `material-offset.besl` and its `.bead`
/// file assume this size and four materials per thread.
const MATERIAL_OFFSET_WORKGROUP_SIZE: u32 = 256;
const _: () = assert!(
	MAX_MATERIALS == MATERIAL_OFFSET_WORKGROUP_SIZE as usize * 4,
	"Update the material offset scan in `material-offset.besl` when the material limit changes."
);

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
		use ghi::command_buffer::CommandBufferRecording as _;

		let stage = |label, pipeline, extent, workgroup| super::ComputeStage {
			label,
			pipeline,
			descriptor_sets: [self.base_descriptor_set, self.visibility_descriptor_set],
			extent,
			workgroup,
		};
		// The offset pass reads these counts without resetting them, so clear before every dispatch.
		c.clear_buffers(&[self.count_buffer.into()]);
		super::record_compute_stages(
			c,
			None,
			&[
				stage("Material Count", pipelines.count, extent, Extent::square(8)),
				stage(
					"Material Offset",
					pipelines.offset,
					Extent::line(MATERIAL_OFFSET_WORKGROUP_SIZE),
					Extent::line(MATERIAL_OFFSET_WORKGROUP_SIZE),
				),
				stage("Pixel Mapping", pipelines.pixel_mapping, extent, Extent::square(16)),
			],
		);
	}
}

/// The `ScreenSpaceLighting` struct names the screen-space lighting passes that ran this frame, so opaque material
/// evaluation reads only the images they wrote.
#[derive(Clone, Copy)]
pub(super) struct ScreenSpaceLighting {
	/// GTAO wrote the ambient occlusion map.
	pub(super) gtao: bool,
	/// SSGI wrote the indirect diffuse map, and material evaluation must write the diffuse radiance its next frame reads.
	pub(super) ssgi: bool,
}

/// The `MaterialEvaluationPass` struct shades every material's pixel list into the lit target.
///
/// The opaque phase also writes diffuse-only radiance into this frame's copy of the SSGI history while SSGI runs, and
/// the lit color into this frame's copy of the radiance history. The next frame's SSGI and reflection rays read them.
pub(super) struct MaterialEvaluationPass {
	lit: ghi::BaseImageHandle,
	diffuse_radiance_history: ghi::DynamicImageHandle,
	radiance_history: ghi::DynamicImageHandle,
	base_descriptor_set: ghi::DescriptorSetHandle,
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	pub(super) fn new(
		lit: ghi::BaseImageHandle,
		diffuse_radiance_history: ghi::DynamicImageHandle,
		radiance_history: ghi::DynamicImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		Self {
			lit,
			diffuse_radiance_history,
			radiance_history,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			evaluation_dispatches,
		}
	}

	/// Prepares one material phase; the opaque phase clears its targets, the transparent phase composites over the lit one.
	pub(super) fn prepare<'a>(
		&self,
		materials: &'a [MaterialEntry],
		active_materials: &'a ActiveMaterialMask,
		phase: VisibilityPhase,
		screen_space_lighting: ScreenSpaceLighting,
	) -> impl RenderPassFunction + use<'a> {
		let lit = self.lit;
		let diffuse_radiance_history = self.diffuse_radiance_history.into();
		let radiance_history = self.radiance_history.into();
		let descriptor_sets = [self.base_descriptor_set, self.visibility_descriptor_set, self.descriptor_set];
		let evaluation_dispatches = self.evaluation_dispatches;
		let ScreenSpaceLighting { gtao, ssgi } = screen_space_lighting;

		move |c| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			if phase == VisibilityPhase::Opaque {
				// Clearing the histories keeps background pixels from holding light of an older frame.
				let transparent_black = ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0));
				let clears = [
					(lit, transparent_black),
					(radiance_history, transparent_black),
					(diffuse_radiance_history, transparent_black),
				];
				// Only SSGI reads the diffuse radiance history, so it is left untouched while SSGI is off.
				c.clear_images(if ssgi { &clears } else { &clears[..2] });
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
				c.write_push_constant(0, [*index, phase.blend_flag(), u32::from(gtao), u32::from(ssgi)]);
				c.indirect_dispatch(evaluation_dispatches, *index as usize);
				c.end_region();
			}
			c.end_region();
		}
	}
}
