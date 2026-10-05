//! Material dispatch bookkeeping and evaluation: count pixels per material, prefix-sum offsets, map pixels, shade.

use utils::{Extent, RGBA};

use super::super::layout::{ActiveMaterialMask, MAX_MATERIALS};
use super::super::scene::MaterialEntry;
use super::Pipelines;
use super::visibility::VisibilityPhase;
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

/// The `MaterialPrepasses` struct runs the three compute passes that turn the visibility buffer into per-material pixel lists.
pub(super) struct MaterialPrepasses {
	/// The base set and the sink's visibility set.
	pub(super) descriptor_sets: [ghi::DescriptorSetHandle; 2],
	pub(super) count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	/// The count, offset, and pixel-mapping pipelines.
	pub(super) pipelines: Pipelines<3>,
}

impl MaterialPrepasses {
	/// Records count, offset, and pixel-mapping for the visibility buffer currently in `extent`.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		[count, offset, pixel_mapping]: [ghi::PipelineHandle; 3],
	) {
		use ghi::command_buffer::CommandBufferRecording as _;

		let stage = |label, pipeline, extent, workgroup| super::ComputeStage {
			label,
			pipeline,
			descriptor_sets: self.descriptor_sets,
			extent,
			workgroup,
		};
		let offset_threads = Extent::line(MATERIAL_OFFSET_WORKGROUP_SIZE);
		// The offset pass reads these counts without resetting them, so clear before every dispatch.
		c.clear_buffers(&[self.count_buffer.into()]);
		super::record_compute_stages(
			c,
			None,
			&[
				stage("Material Count", count, extent, Extent::square(8)),
				stage("Material Offset", offset, offset_threads, offset_threads),
				stage("Pixel Mapping", pixel_mapping, extent, Extent::square(16)),
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
	pub(super) lit: ghi::BaseImageHandle,
	pub(super) diffuse_radiance_history: ghi::DynamicImageHandle,
	pub(super) radiance_history: ghi::DynamicImageHandle,
	pub(super) base_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// The material-evaluation-only set, which also carries the environment.
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	pub(super) evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	/// Clears the lit target and the histories opaque evaluation writes, so background pixels never hold light of
	/// an older frame. Record it before the opaque phase; `ssgi` says whether SSGI reads the diffuse radiance history
	/// this frame, since nothing else does.
	pub(super) fn clear_histories(&self, c: &mut ghi::implementation::CommandBufferRecording, ssgi: bool) {
		use ghi::command_buffer::CommandBufferRecording as _;

		let transparent_black = ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0));
		let clears = [
			(self.lit, transparent_black),
			(self.radiance_history.into(), transparent_black),
			(self.diffuse_radiance_history.into(), transparent_black),
		];
		// Only SSGI reads the diffuse radiance history, so it is left untouched while SSGI is off.
		c.clear_images(if ssgi { &clears } else { &clears[..2] });
	}

	/// Prepares one material phase; the transparent phase composites over the lit target the opaque phase wrote.
	pub(super) fn prepare<'a>(
		&self,
		materials: &'a [MaterialEntry],
		active_materials: &'a ActiveMaterialMask,
		phase: VisibilityPhase,
		screen_space_lighting: ScreenSpaceLighting,
	) -> impl RenderPassFunction + use<'a> {
		let descriptor_sets = [self.base_descriptor_set, self.visibility_descriptor_set, self.descriptor_set];
		let evaluation_dispatches = self.evaluation_dispatches;
		let ScreenSpaceLighting { gtao, ssgi } = screen_space_lighting;

		move |c| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			let active = materials
				.iter()
				.filter(|(_, index, _)| material_is_active(active_materials, *index));
			c.start_region(|label| {
				label.write_str(phase.label())?;
				label.write_str(" Material Evaluation")
			});
			// Pixel mapping gave every pixel to exactly one material, so no two materials touch the same pixel of
			// the lit target or the histories, and their dispatches can overlap instead of each waiting for the
			// last one's writes.
			c.unordered(|c| {
				// Materials sharing a pipeline are adjacent, so the binding survives across consecutive dispatches.
				let mut bound_pipeline = None;
				for (name, index, pipeline) in active {
					c.start_region(|label| label.write_str(name));
					if bound_pipeline != Some(*pipeline) {
						let c = c.bind_compute_pipeline(*pipeline);
						c.bind_descriptor_sets(&descriptor_sets);
						bound_pipeline = Some(*pipeline);
					}
					c.write_push_constant(0, [*index, phase as u32, u32::from(gtao), u32::from(ssgi)]);
					c.indirect_dispatch(evaluation_dispatches, *index as usize);
					c.end_region();
				}
			});
			c.end_region();
		}
	}
}
