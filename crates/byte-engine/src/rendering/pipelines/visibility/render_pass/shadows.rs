//! Depth-only rendering of the directional cascades, cone layers, and point cube faces selected this frame.

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::super::layout::{
	CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_POINT_SHADOW_POOL_CAPACITY,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_RESOLUTION, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT,
	SHADOW_MAP_RESOLUTION,
};
use super::super::mesh_dispatch::{MeshDispatch, PhaseDispatches};
use crate::rendering::PipelineManagerClient;
use crate::rendering::render_pass::RenderPassFunction;

/// Mip count of the packed cascade depth pyramid; one retained 4x4 max level.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT: u32 = 1;
const DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const DEPTH_PYRAMID_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);

/// The `ShadowWork` struct says which shadow views received lights this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowWork {
	pub(crate) directional: bool,
	pub(crate) cone_count: usize,
	pub(crate) point_count: usize,
}

impl ShadowWork {
	pub(crate) fn any(self) -> bool {
		self.directional || self.cone_count > 0 || self.point_count > 0
	}
}

/// Returns the cascade view indices that receive one batched shadow dispatch.
pub(super) fn directional_shadow_view_indices(mesh_dispatch: MeshDispatch) -> impl Iterator<Item = u32> {
	let has_work = !mesh_dispatch.is_empty();
	(1..=SHADOW_CASCADE_COUNT as u32).filter(move |_| has_work)
}

/// Returns the packed cone view and target-layer indices that receive shadow dispatches.
pub(super) fn cone_shadow_view_indices(mesh_dispatch: MeshDispatch, cone_count: usize) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		cone_count.min(MAX_CONE_SHADOW_POOL_CAPACITY)
	};
	(0..count).map(|layer| ((CONE_SHADOW_VIEW_OFFSET + layer) as u32, layer as u32))
}

/// Returns the packed point-cube view and target-face indices that receive shadow dispatches.
pub(super) fn point_shadow_view_indices(mesh_dispatch: MeshDispatch, point_count: usize) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		point_count.min(MAX_POINT_SHADOW_POOL_CAPACITY)
	};
	(0..count * POINT_SHADOW_FACE_COUNT).map(|face| ((POINT_SHADOW_VIEW_OFFSET + face) as u32, face as u32))
}

/// The `ShadowPass` struct owns the pipelines and depth targets used by directional, cone, and point shadow rendering.
pub(super) struct ShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	directional_pipeline: crate::rendering::PipelineRef,
	depth_pyramid_pipeline: crate::rendering::PipelineRef,
	local_pipeline: crate::rendering::PipelineRef,
	masked_directional_pipeline: crate::rendering::PipelineRef,
	masked_local_pipeline: crate::rendering::PipelineRef,
	pub(super) directional_shadow_map: ghi::BaseImageHandle,
	pub(super) depth_pyramid: ghi::BaseImageHandle,
	pub(super) cone_shadow_map: ghi::BaseImageHandle,
	pub(super) point_shadow_map: ghi::BaseImageHandle,
}

struct ShadowPipelines {
	directional: ghi::PipelineHandle,
	masked_directional: ghi::PipelineHandle,
	depth_pyramid: ghi::PipelineHandle,
	/// Cone and point maps share one perspective depth pipeline.
	local: ghi::PipelineHandle,
	masked_local: ghi::PipelineHandle,
}

impl ShadowPass {
	/// Creates shadow targets and requests the depth pipelines matching their formats.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
		point_shadow_map: ghi::BaseImageHandle,
	) -> Self {
		let depth_pyramid_descriptor_set =
			context.create_descriptor_set(Some("Directional Shadow Depth Pyramid Descriptor Set"));
		let max_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod(0.0),
		);
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				depth_pyramid_descriptor_set,
				DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				directional_shadow_map,
				max_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				DEPTH_PYRAMID_OUTPUT_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				0,
			),
		]);
		let request = |name| pipeline_manager.request_pipeline(name);
		Self {
			descriptor_set,
			depth_pyramid_descriptor_set,
			directional_pipeline: request("byte-engine/rendering/visibility/directional-shadow.pipeline"),
			depth_pyramid_pipeline: request("byte-engine/rendering/visibility/directional-shadow-depth-pyramid.pipeline"),
			local_pipeline: request("byte-engine/rendering/visibility/cone-shadow.pipeline"),
			masked_directional_pipeline: request("byte-engine/rendering/visibility/masked-directional-shadow.pipeline"),
			masked_local_pipeline: request("byte-engine/rendering/visibility/masked-cone-shadow.pipeline"),
			directional_shadow_map,
			depth_pyramid,
			cone_shadow_map,
			point_shadow_map,
		}
	}

	fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<ShadowPipelines> {
		Some(ShadowPipelines {
			directional: pipeline_manager.pipeline(self.directional_pipeline)?,
			masked_directional: pipeline_manager.pipeline(self.masked_directional_pipeline)?,
			depth_pyramid: pipeline_manager.pipeline(self.depth_pyramid_pipeline)?,
			local: pipeline_manager.pipeline(self.local_pipeline)?,
			masked_local: pipeline_manager.pipeline(self.masked_local_pipeline)?,
		})
	}

	/// Prepares this frame's shadow maps, or `None` while a pipeline is still compiling.
	///
	/// Blend materials have no alpha-aware shadow shader, so only opaque and masked geometry casts shadows.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		dispatches: PhaseDispatches,
		work: ShadowWork,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let pipelines = self.pipelines(pipeline_manager)?;
		let descriptor_set = self.descriptor_set;
		let depth_pyramid_descriptor_set = self.depth_pyramid_descriptor_set;
		let directional_shadow_map = self.directional_shadow_map;
		let cone_shadow_map = self.cone_shadow_map;
		let point_shadow_map = self.point_shadow_map;
		let directional_extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let depth_pyramid_extent = Extent::rectangle(
			SHADOW_MAP_RESOLUTION / 2,
			SHADOW_MAP_RESOLUTION / 2 * SHADOW_CASCADE_COUNT as u32,
		);
		let cone_extent = Extent::square(CONE_SHADOW_MAP_RESOLUTION);
		let point_extent = Extent::square(POINT_SHADOW_MAP_RESOLUTION);

		if work.directional {
			frame.resize_image(directional_shadow_map, directional_extent);
		}
		if work.cone_count > 0 {
			frame.resize_image(cone_shadow_map, cone_extent);
		}
		if work.point_count > 0 {
			frame.resize_image(point_shadow_map, point_extent);
		}

		Some(
			move |c: &mut ghi::implementation::CommandBufferRecording, _: &[ghi::AttachmentInformation]| {
				use ghi::command_buffer::{
					BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
					CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
				};

				// Draws every solid and masked work range into the layers named by `views`.
				let record_maps = |c: &mut ghi::implementation::CommandBufferRecording,
				                   name: &str,
				                   target: ghi::BaseImageHandle,
				                   extent: Extent,
				                   layers: usize,
				                   solid_pipeline: ghi::PipelineHandle,
				                   masked_pipeline: ghi::PipelineHandle,
				                   views: &dyn Fn(MeshDispatch) -> Vec<(u32, u32)>| {
					c.start_region(|label| label.write_str(name));
					let attachments = [ghi::AttachmentInformation::new(
						target,
						ghi::Layouts::RenderTarget,
						ghi::ClearValue::Depth(0.0),
						false,
						true,
					)
					.layers(layers as u32)];
					let c = c.start_render_pass(extent, &attachments);
					for (dispatch, pipeline) in [(dispatches.opaque, solid_pipeline), (dispatches.masked, masked_pipeline)] {
						if dispatch.is_empty() {
							continue;
						}
						let c = c.bind_raster_pipeline(pipeline);
						c.bind_descriptor_sets(&[descriptor_set]);
						for (view_index, layer) in views(dispatch) {
							c.write_push_constant(0, dispatch.work_item_base());
							c.write_push_constant(4, view_index);
							c.write_push_constant(8, layer);
							c.dispatch_meshes(dispatch.workgroup_count(), 1, 1);
						}
					}
					c.end_render_pass();
					c.end_region();
				};

				if work.directional {
					record_maps(
						c,
						"Directional Shadow Map",
						directional_shadow_map,
						directional_extent,
						SHADOW_CASCADE_COUNT,
						pipelines.directional,
						pipelines.masked_directional,
						&|dispatch| {
							directional_shadow_view_indices(dispatch)
								.map(|view| (view, view - 1))
								.collect()
						},
					);
					// Each SIMD-width workgroup reduces two adjacent source tiles into 4x4 cells.
					c.start_region(|label| label.write_str("Directional Shadow Depth Pyramid"));
					let c = c.bind_compute_pipeline(pipelines.depth_pyramid);
					c.bind_descriptor_sets(&[depth_pyramid_descriptor_set]);
					c.dispatch(ghi::DispatchExtent::new(depth_pyramid_extent, Extent::new(8, 4, 1)));
					c.end_region();
				}
				if work.cone_count > 0 {
					record_maps(
						c,
						"Cone Shadow Map",
						cone_shadow_map,
						cone_extent,
						work.cone_count,
						pipelines.local,
						pipelines.masked_local,
						&|dispatch| cone_shadow_view_indices(dispatch, work.cone_count).collect(),
					);
				}
				if work.point_count > 0 {
					record_maps(
						c,
						"Point Shadow Map",
						point_shadow_map,
						point_extent,
						work.point_count * POINT_SHADOW_FACE_COUNT,
						pipelines.local,
						pipelines.masked_local,
						&|dispatch| point_shadow_view_indices(dispatch, work.point_count).collect(),
					);
				}
			},
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shadow_dispatches_preserve_directional_cascades_cone_layers_and_point_cube_faces() {
		let dispatch = MeshDispatch::with_workgroup_count(19);

		assert_eq!(directional_shadow_view_indices(dispatch).collect::<Vec<_>>(), [1, 2, 3, 4]);
		assert_eq!(
			cone_shadow_view_indices(dispatch, 4).collect::<Vec<_>>(),
			[(5, 0), (6, 1), (7, 2), (8, 3)]
		);
		assert_eq!(
			cone_shadow_view_indices(dispatch, MAX_CONE_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32,
				(MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32
			))
		);
		assert_eq!(directional_shadow_view_indices(MeshDispatch::default()).count(), 0);
		assert_eq!(cone_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
		assert_eq!(
			point_shadow_view_indices(dispatch, 2).collect::<Vec<_>>(),
			(0..12u32)
				.map(|face| (POINT_SHADOW_VIEW_OFFSET as u32 + face, face))
				.collect::<Vec<_>>()
		);
		assert_eq!(
			point_shadow_view_indices(dispatch, MAX_POINT_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
				(MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
			))
		);
		assert_eq!(point_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
	}
}
