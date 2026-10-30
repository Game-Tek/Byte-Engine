//! Material dispatch bookkeeping and evaluation: count pixels per evaluation slot, prefix-sum offsets, map pixels, shade.
//!
//! Materials that compile to the same evaluation pipeline share one slot, so they share one pixel list and one dispatch.
//! See [`MaterialEvaluation`]. In the uber material evaluation mode, each phase's uber shader shades the whole screen in
//! tiles instead, and none of the prepasses run.

use utils::Extent;

use super::super::layout::{ActiveEvaluationMask, MAX_MATERIALS, MAX_PIXEL_MAPPING_ENTRIES};
use super::super::scene::MaterialEvaluation;
use super::super::set_layouts;
use super::super::shader_generator::UBER_WORKGROUP;
use super::visibility::VisibilityPhase;
use super::{IndirectDiffuseSource, Pipelines};
use crate::rendering::PipelineManagerClient;
use crate::rendering::render_pass::RenderPassFunction;

/// Threads of the one workgroup that scans every evaluation slot's count into offsets. `material-offset.besl` and its
/// `.bead` file assume this size and four slots per thread. There are never more slots than materials.
const MATERIAL_OFFSET_WORKGROUP_SIZE: u32 = 256;
const _: () = assert!(
	MAX_MATERIALS == MATERIAL_OFFSET_WORKGROUP_SIZE as usize * 4,
	"Update the material offset scan in `material-offset.besl` when the material limit changes."
);

/// Returns whether this frame contains geometry of the requested visibility phase whose material uses one evaluation slot.
pub(super) fn evaluation_is_active(active_evaluations: &ActiveEvaluationMask, evaluation_index: u32) -> bool {
	let evaluation_index = evaluation_index as usize;
	active_evaluations
		.get(evaluation_index / u64::BITS as usize)
		.is_some_and(|word| word & (1u64 << (evaluation_index % u64::BITS as usize)) != 0)
}

/// The count, offset, and pixel-mapping pipelines, named as [`Pipelines::request`] takes them. Only the pipelines
/// material evaluation mode runs them.
pub(in crate::rendering::pipelines::visibility) const MATERIAL_PREPASS_PIPELINES: [&str; 3] =
	["material-count", "material-offset", "pixel-mapping"];

/// The `MaterialPrepasses` struct runs the three compute passes that turn the visibility buffer into one pixel list per
/// evaluation slot.
pub(super) struct MaterialPrepasses {
	/// The base set, the sink's visibility set, and its material-evaluation set, whose lit target and histories pixel
	/// mapping zeroes where no surface was drawn.
	pub(super) descriptor_sets: [ghi::DescriptorSetHandle; 3],
	pub(super) count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	/// The indirect dispatch of every evaluation slot, which pixel mapping writes and material evaluation runs.
	pub(super) evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	/// The count, offset, and pixel-mapping pipelines.
	pub(super) pipelines: Pipelines<3>,
}

impl MaterialPrepasses {
	/// Creates the buffers the prepasses write and material evaluation reads, binds them in the sink's visibility buffer
	/// set, and requests the prepass pipelines.
	///
	/// `instance_id` is the sink's instance target, which the prepasses read the pixels' materials from.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_buffer_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		instance_id: ghi::BaseImageHandle,
	) -> Self {
		use ghi::context::{Context as _, ContextCreate as _};

		// The dispatch passes write the pixel lists through one set, and material evaluation reads them through
		// another, so evaluation is not an access that the next dispatch has to wait for.
		let material_dispatch_descriptor_set =
			set_layouts::MATERIAL_DISPATCH.create_set(context, "Material Dispatch Descriptor Set");
		let material_buffer = |name, extra_uses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | extra_uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let material_count = context.build_buffer(material_buffer("Material Count", ghi::Uses::empty()));
		let material_offset: ghi::BufferHandle<[u32; MAX_MATERIALS]> =
			context.build_buffer(material_buffer("Material Offset", ghi::Uses::empty()));
		let material_offset_scratch: ghi::BufferHandle<[u32; MAX_MATERIALS]> =
			context.build_buffer(material_buffer("Material Offset Scratch", ghi::Uses::empty()));
		let evaluation_dispatches =
			context.build_buffer(material_buffer("Material Evaluation Dispatches", ghi::Uses::Indirect));
		let pixel_mapping: ghi::BufferHandle<[[u16; 2]; MAX_PIXEL_MAPPING_ENTRIES]> =
			context.build_buffer(material_buffer("Material XY", ghi::Uses::empty()));
		let dispatch = set_layouts::MATERIAL_DISPATCH.binder(material_dispatch_descriptor_set);
		let visibility = set_layouts::VISIBILITY_BUFFER.binder(visibility_buffer_descriptor_set);
		context.write(&[
			dispatch.buffer("material_count_buffer", material_count),
			dispatch.buffer("material_offset_buffer", material_offset),
			dispatch.buffer("material_offset_scratch_buffer", material_offset_scratch),
			dispatch.buffer("material_evaluation_dispatches", evaluation_dispatches),
			dispatch.buffer("pixel_mapping_buffer", pixel_mapping),
			dispatch.image("instance_index_render_target", instance_id),
			visibility.buffer("material_count", material_count),
			visibility.buffer("material_offset", material_offset),
			visibility.buffer("pixel_mapping", pixel_mapping),
		]);

		Self {
			descriptor_sets: [
				base_descriptor_set,
				material_dispatch_descriptor_set,
				material_evaluation_descriptor_set,
			],
			count_buffer: material_count,
			evaluation_dispatches,
			pipelines: Pipelines::request(pipeline_manager, MATERIAL_PREPASS_PIPELINES),
		}
	}

	/// Records count, offset, and pixel-mapping for the visibility buffer currently in `extent`.
	///
	/// In the opaque `phase`, pixel mapping also zeroes the lit target and the histories at every pixel no surface
	/// covered, so the background and the next frame's rays never read light of an older frame there. The transparent
	/// phase keeps what the opaque phase and the background wrote.
	pub(super) fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		[count, offset, pixel_mapping]: [ghi::PipelineHandle; 3],
		phase: VisibilityPhase,
	) {
		use ghi::command_buffer::{
			BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
			CommonCommandBufferMode as _,
		};

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
			],
		);
		c.start_region(|label| label.write_str("Pixel Mapping"));
		let c = c.bind_compute_pipeline(pixel_mapping);
		c.bind_descriptor_sets(&self.descriptor_sets);
		c.write_push_constant(0, [phase as u32]);
		c.dispatch(ghi::DispatchExtent::new(extent, Extent::square(16)));
		c.end_region();
	}
}

/// The `ScreenSpaceLighting` struct names the screen-space lighting passes that ran this frame, so opaque material
/// evaluation reads only the images they wrote.
#[derive(Clone, Copy)]
pub(super) struct ScreenSpaceLighting {
	/// GTAO wrote the ambient occlusion map.
	pub(super) gtao: bool,
	/// SSGI lit the frame, so material evaluation must write the diffuse radiance its next frame's rays read.
	pub(super) ssgi: bool,
	/// The source that wrote the indirect diffuse images, if one did.
	pub(super) indirect_diffuse: Option<IndirectDiffuseSource>,
}

/// The `MaterialEvaluationPass` struct shades every evaluation slot's pixel list into the lit target.
///
/// The opaque phase also writes diffuse-only radiance into this frame's copy of the diffuse radiance history while SSGI
/// lights the frame, and the lit color into this frame's copy of the radiance history. The next frame's SSGI and
/// reflection rays read them.
pub(super) struct MaterialEvaluationPass {
	pub(super) base_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// The material-evaluation-only set, which also carries the environment.
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
}

impl MaterialEvaluationPass {
	/// Prepares one material phase; the transparent phase composites over the lit target the opaque phase wrote.
	///
	/// Records one dispatch for each of `evaluations` that `active_evaluations` marks. `debug_views` is what
	/// [`crate::rendering::DebugViews::shown`] packs for the manager's views this frame.
	///
	/// With `pixel_lists`, the indirect dispatches the material prepasses wrote, each evaluation shades its slot's
	/// pixel list. Without them, each phase's uber shader shades the sink's `extent` in [`UBER_WORKGROUP`] tiles, and
	/// the opaque one dispatches even when no opaque instance is active, because it also writes the background's zeros,
	/// which pixel mapping writes in the pipelines mode.
	pub(super) fn prepare<'a>(
		&self,
		evaluations: &'a [MaterialEvaluation],
		active_evaluations: &'a ActiveEvaluationMask,
		phase: VisibilityPhase,
		pixel_lists: Option<ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>>,
		extent: Extent,
		screen_space_lighting: ScreenSpaceLighting,
		debug_views: u32,
	) -> impl RenderPassFunction + use<'a> {
		let descriptor_sets = [self.base_descriptor_set, self.visibility_descriptor_set, self.descriptor_set];
		let clears_background = pixel_lists.is_none() && phase == VisibilityPhase::Opaque;
		// The push constant's GTAO and SSGI flags, and its indirect diffuse source.
		let flags = [
			u32::from(screen_space_lighting.gtao),
			u32::from(screen_space_lighting.ssgi),
			screen_space_lighting
				.indirect_diffuse
				.map_or(0, IndirectDiffuseSource::push_constant),
		];

		move |c| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			let active = evaluations
				.iter()
				.filter(|evaluation| clears_background || evaluation_is_active(active_evaluations, evaluation.index));
			c.start_region(|label| {
				label.write_str(phase.label())?;
				label.write_str(" Material Evaluation")
			});
			// Pixel mapping gave every pixel to exactly one evaluation slot, so no two dispatches touch the same pixel
			// of the lit target or the histories, and they can overlap instead of each waiting for the last one's
			// writes. A phase's uber shader is its only dispatch.
			c.unordered(|c| {
				// Every evaluation of a phase has its own pipeline.
				for evaluation in active {
					c.start_region(|label| label.write_str(&evaluation.name));
					let c = c.bind_compute_pipeline(evaluation.pipeline);
					c.bind_descriptor_sets(&descriptor_sets);
					let [gtao, ssgi, indirect_diffuse] = flags;
					c.write_push_constant(0, [evaluation.index, gtao, ssgi, indirect_diffuse, debug_views]);
					match pixel_lists {
						Some(dispatches) => c.indirect_dispatch(dispatches, evaluation.index as usize),
						None => c.dispatch(ghi::DispatchExtent::new(extent, UBER_WORKGROUP)),
					}
					c.end_region();
				}
			});
			c.end_region();
		}
	}
}
