//! GPU light bucketing: splits each sink's view frustum into clusters and records which lights reach each one.
//!
//! Material evaluation shades a pixel with the lights of its cluster instead of the whole light table, so its cost
//! follows the lights near a pixel rather than the lights in the scene. Each light reaches as far as its exposed
//! illuminance stays above [`super::super::scene::LIGHT_REACH_THRESHOLD_LUX`].

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::super::layout::{
	LIGHT_CLUSTER_COUNT, LIGHT_CLUSTER_MASK_WORD_COUNT, LIGHT_CLUSTER_MASK_WORDS, LIGHT_CLUSTER_MASKS_BINDING,
	LIGHT_CLUSTER_PARAMETERS_BINDING,
};
use super::super::shader_data::{LightClusterParameters, LightingData};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

const LIGHTING_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const MASKS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::WRITE,
);

/// The `LightClusterPass` struct buckets the light table into one sink's clusters before material evaluation reads them.
///
/// It runs once per sink and frame, before opaque and transparent material evaluation. Create it with
/// [`LightClusterPass::new`], which also binds its output into the material-evaluation descriptor set.
pub(super) struct LightClusterPass {
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline: crate::rendering::PipelineRef,
	parameters: ghi::DynamicBufferHandle<LightClusterParameters>,
}

impl LightClusterPass {
	/// Creates the cluster masks, binds them for material evaluation, and requests the bucketing pipeline.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Light Cluster Descriptor Set"));
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Light Cluster Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let masks: ghi::BufferHandle<[u32; LIGHT_CLUSTER_MASK_WORD_COUNT]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Light Cluster Masks")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, LIGHTING_DATA_BINDING.slot(), lighting_buffer.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, PARAMETERS_BINDING.slot(), parameters.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, MASKS_BINDING.slot(), masks.into()),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHT_CLUSTER_MASKS_BINDING.slot(),
				masks.into(),
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHT_CLUSTER_PARAMETERS_BINDING.slot(),
				parameters.into(),
			),
		]);

		Self {
			descriptor_set,
			pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/light-clusters.pipeline"),
			parameters,
		}
	}

	pub(super) fn pipeline(&self, pipeline_manager: &PipelineManagerClient) -> Option<ghi::PipelineHandle> {
		pipeline_manager.pipeline(self.pipeline)
	}

	/// Uploads this frame's cluster layout for `sink` and returns the bucketing recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<> {
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = LightClusterParameters::from(sink.view());
		frame.sync_buffer(self.parameters);
		let descriptor_set = self.descriptor_set;

		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			c.start_region(|label| label.write_str("Light Clusters"));
			let c = c.bind_compute_pipeline(pipeline);
			c.bind_descriptor_sets(&[descriptor_set]);
			// One workgroup per cluster, and one thread per mask word of 32 lights.
			let workgroup_size = LIGHT_CLUSTER_MASK_WORDS as u32;
			c.dispatch(ghi::DispatchExtent::new(
				Extent::line(LIGHT_CLUSTER_COUNT as u32 * workgroup_size),
				Extent::line(workgroup_size),
			));
			c.end_region();
		}
	}
}
