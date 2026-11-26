//! GPU light bucketing: splits each sink's view frustum into clusters and records which lights reach each one.
//!
//! Material evaluation shades a pixel with the lights of its cluster instead of the whole light table, so its cost
//! follows the lights near a pixel rather than the lights in the scene. Each light reaches as far as its exposed
//! illuminance stays above [`super::super::scene::LIGHT_REACH_THRESHOLD_LUX`].

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::super::layout::{LIGHT_CLUSTER_COUNT, LIGHT_CLUSTER_MASK_WORD_COUNT, LIGHT_CLUSTER_MASK_WORDS};
use super::super::shader_data::{LightClusterParameters, LightingData};
use super::Pipelines;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, SetBinder, Sink};

/// The `LightClusterPass` struct buckets the light table into one sink's clusters before material evaluation reads them.
///
/// It runs once per sink and frame, before opaque and transparent material evaluation. Create it with
/// [`LightClusterPass::new`], which also binds its output into the material-evaluation descriptor set.
pub(super) struct LightClusterPass {
	descriptor_set: ghi::DescriptorSetHandle,
	pub(super) pipelines: Pipelines<1>,
	parameters: ghi::BufferHandle<LightClusterParameters>,
}

impl LightClusterPass {
	/// Creates the cluster masks, binds them for material evaluation, and requests the bucketing pipeline.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		lighting_buffer: ghi::BufferHandle<LightingData>,
		evaluation: SetBinder<'_>,
	) -> Self {
		let parameters = context.build_buffer(super::frame_buffer("Light Cluster Parameters"));
		let masks: ghi::BufferHandle<[u32; LIGHT_CLUSTER_MASK_WORD_COUNT]> =
			context.build_buffer(ghi::buffer::Builder::new(ghi::Uses::Storage).name("Light Cluster Masks"));
		context.write(&[
			evaluation.buffer("light_cluster_masks", masks),
			evaluation.buffer("light_cluster_parameters", parameters),
		]);
		let descriptor_set =
			set_layouts::create_set(context, set_layouts::LIGHT_CLUSTERS, "Light Cluster Descriptor Set", |set| {
				[
					set.buffer("lighting_data", lighting_buffer),
					set.buffer("light_cluster_parameters", parameters),
					set.buffer("light_cluster_masks", masks),
				]
			});

		Self {
			descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, ["light-clusters"]),
			parameters,
		}
	}

	/// Uploads this frame's cluster layout for `sink` and returns the bucketing recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<> {
		*frame.map(self.parameters) = LightClusterParameters::from(sink.view());
		// One workgroup per cluster, and one thread per mask word of 32 lights.
		let workgroup_size = LIGHT_CLUSTER_MASK_WORDS as u32;
		let stage = super::ComputeStage {
			label: "Light Clusters",
			pipeline,
			descriptor_sets: [self.descriptor_set],
			extent: Extent::line(LIGHT_CLUSTER_COUNT as u32 * workgroup_size),
			workgroup: Extent::line(workgroup_size),
		};

		move |c| super::record_compute_stages(c, None, &[stage])
	}
}
