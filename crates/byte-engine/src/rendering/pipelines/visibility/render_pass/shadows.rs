use super::*;

/// Returns the directional cascade view indices that receive one batched shadow dispatch.
pub(crate) fn directional_shadow_view_indices(mesh_dispatch: MeshDispatch) -> impl Iterator<Item = u32> {
	let has_work = !mesh_dispatch.is_empty();
	(1..=SHADOW_CASCADE_COUNT as u32).filter(move |_| has_work)
}

/// Returns the packed cone view and target-layer indices that receive shadow dispatches.
pub(crate) fn cone_shadow_view_indices(
	mesh_dispatch: MeshDispatch,
	cone_shadow_count: usize,
) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		cone_shadow_count.min(MAX_CONE_SHADOW_POOL_CAPACITY)
	};
	(0..count).map(|layer| ((CONE_SHADOW_VIEW_OFFSET + layer) as u32, layer as u32))
}

/// Returns the packed point-cube view and target-face indices that receive shadow dispatches.
pub(crate) fn point_shadow_view_indices(
	mesh_dispatch: MeshDispatch,
	point_shadow_count: usize,
) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		point_shadow_count.min(MAX_POINT_SHADOW_POOL_CAPACITY)
	};
	(0..count).flat_map(|cube| {
		(0..POINT_SHADOW_FACE_COUNT).map(move |face| {
			(
				(POINT_SHADOW_VIEW_OFFSET + cube * POINT_SHADOW_FACE_COUNT + face) as u32,
				(cube * POINT_SHADOW_FACE_COUNT + face) as u32,
			)
		})
	})
}

/// The `ShadowPass` struct owns the shared pipeline and depth targets used by directional, cone, and point shadow rendering.
pub struct ShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	directional_shadow_depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) directional_shadow_pass_pipeline: crate::rendering::PipelineRef,
	pub(super) directional_shadow_depth_pyramid_pipeline: crate::rendering::PipelineRef,
	pub(super) cone_shadow_pass_pipeline: crate::rendering::PipelineRef,
	pub(super) masked_directional_shadow_pass_pipeline: crate::rendering::PipelineRef,
	pub(super) masked_cone_shadow_pass_pipeline: crate::rendering::PipelineRef,
	directional_shadow_map: ghi::BaseImageHandle,
	cone_shadow_map: ghi::BaseImageHandle,
	point_shadow_map: ghi::BaseImageHandle,
}

impl ShadowPass {
	/// Creates shadow resources and requests pipelines that match the directional and cone depth formats.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &crate::rendering::PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		directional_shadow_depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
		point_shadow_map: ghi::BaseImageHandle,
	) -> Self {
		let directional_shadow_depth_pyramid_descriptor_set =
			context.create_descriptor_set(Some("Directional Shadow Depth Pyramid Descriptor Set"));
		let shadow_depth_sampler = context.build_sampler(
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
				directional_shadow_depth_pyramid_descriptor_set,
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				directional_shadow_map,
				shadow_depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				directional_shadow_depth_pyramid_descriptor_set,
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_OUTPUT_1_BINDING.slot(),
				directional_shadow_depth_pyramid,
				ghi::Layouts::General,
				0,
			),
		]);
		let directional_shadow_pass_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/directional-shadow.pipeline");
		let directional_shadow_depth_pyramid_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/directional-shadow-depth-pyramid.pipeline");
		let cone_shadow_pass_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/cone-shadow.pipeline");
		let masked_directional_shadow_pass_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/masked-directional-shadow.pipeline");
		let masked_cone_shadow_pass_pipeline =
			pipeline_manager.request_pipeline("byte-engine/rendering/visibility/masked-cone-shadow.pipeline");

		Self {
			descriptor_set,
			directional_shadow_depth_pyramid_descriptor_set,
			directional_shadow_pass_pipeline,
			directional_shadow_depth_pyramid_pipeline,
			cone_shadow_pass_pipeline,
			masked_directional_shadow_pass_pipeline,
			masked_cone_shadow_pass_pipeline,
			directional_shadow_map,
			cone_shadow_map,
			point_shadow_map,
		}
	}

	/// Prepares directional cascades, cone layers, and point-cube faces for the current scene geometry.
	pub(super) fn prepare<'a>(
		&self,
		frame: &mut ghi::implementation::Frame,
		instances: &'a [Instance],
		mesh_dispatch: MeshDispatch,
		masked_instances: &'a [Instance],
		masked_mesh_dispatch: MeshDispatch,
		directional_shadow_enabled: bool,
		cone_shadow_count: usize,
		point_shadow_count: usize,
		directional_pipeline: ghi::PipelineHandle,
		masked_directional_pipeline: ghi::PipelineHandle,
		directional_shadow_depth_pyramid_pipeline: ghi::PipelineHandle,
		cone_pipeline: ghi::PipelineHandle,
		masked_cone_pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<'a> {
		let descriptor_set = self.descriptor_set;
		let directional_shadow_depth_pyramid_descriptor_set = self.directional_shadow_depth_pyramid_descriptor_set;
		let directional_shadow_map = self.directional_shadow_map;
		let cone_shadow_map = self.cone_shadow_map;
		let point_shadow_map = self.point_shadow_map;
		let directional_extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let directional_shadow_depth_pyramid_extent = Extent::rectangle(
			SHADOW_MAP_RESOLUTION / 2,
			SHADOW_MAP_RESOLUTION / 2 * SHADOW_CASCADE_COUNT as u32,
		);
		let cone_extent = Extent::square(CONE_SHADOW_MAP_RESOLUTION);
		let point_extent = Extent::square(POINT_SHADOW_MAP_RESOLUTION);
		let drawable_instances = instances.iter().filter(|instance| instance.meshlet_count > 0).count();
		let meshlet_count = instances.iter().map(|instance| instance.meshlet_count).sum::<u32>();

		if directional_shadow_enabled {
			frame.resize_image(directional_shadow_map, directional_extent);
		}
		if cone_shadow_count > 0 {
			frame.resize_image(cone_shadow_map, cone_extent);
		}
		if point_shadow_count > 0 {
			frame.resize_image(point_shadow_map, point_extent);
		}

		move |c, _| {
			if directional_shadow_enabled {
				log::debug!(
					"Directional shadow pass executing: cascades={}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
					SHADOW_CASCADE_COUNT,
					instances.len(),
					drawable_instances,
					meshlet_count,
					mesh_dispatch.workgroup_count(),
				);
				c.start_region(|label| label.write_str("Directional Shadow Map"));
				let attachments = [ghi::AttachmentInformation::new(
					directional_shadow_map,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layers(SHADOW_CASCADE_COUNT as u32)];
				let c = c.start_render_pass(directional_extent, &attachments);
				let c = c.bind_raster_pipeline(directional_pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				for view_index in directional_shadow_view_indices(mesh_dispatch) {
					c.start_region(|label| label.write_str("Cascade"));
					c.write_push_constant(0, mesh_dispatch.work_item_base());
					c.write_push_constant(4, view_index);
					c.write_push_constant(8, view_index - 1);
					c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
					c.end_region();
				}
				if !masked_mesh_dispatch.is_empty() {
					let c = c.bind_raster_pipeline(masked_directional_pipeline);
					c.bind_descriptor_sets(&[descriptor_set]);
					for view_index in directional_shadow_view_indices(masked_mesh_dispatch) {
						c.write_push_constant(0, masked_mesh_dispatch.work_item_base());
						c.write_push_constant(4, view_index);
						c.write_push_constant(8, view_index - 1);
						c.dispatch_meshes(masked_mesh_dispatch.workgroup_count(), 1, 1);
					}
				}
				c.end_render_pass();
				c.end_region();

				// Each SIMD-width workgroup reduces two adjacent source tiles into 4x4 cells.
				c.start_region(|label| label.write_str("Directional Shadow Depth Pyramid"));
				let c = c.bind_compute_pipeline(directional_shadow_depth_pyramid_pipeline);
				c.bind_descriptor_sets(&[directional_shadow_depth_pyramid_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(
					directional_shadow_depth_pyramid_extent,
					Extent::new(8, 4, 1),
				));
				c.end_region();
			}

			if cone_shadow_count > 0 {
				log::debug!(
					"Cone shadow pass executing: lights={}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
					cone_shadow_count,
					instances.len(),
					drawable_instances,
					meshlet_count,
					mesh_dispatch.workgroup_count(),
				);
				c.start_region(|label| label.write_str("Cone Shadow Map"));
				let attachments = [ghi::AttachmentInformation::new(
					cone_shadow_map,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layers(cone_shadow_count as u32)];
				let c = c.start_render_pass(cone_extent, &attachments);
				let c = c.bind_raster_pipeline(cone_pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				for (view_index, layer) in cone_shadow_view_indices(mesh_dispatch, cone_shadow_count) {
					c.start_region(|label| label.write_str("Cone"));
					c.write_push_constant(0, mesh_dispatch.work_item_base());
					c.write_push_constant(4, view_index);
					c.write_push_constant(8, layer);
					c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
					c.end_region();
				}
				if !masked_mesh_dispatch.is_empty() {
					let c = c.bind_raster_pipeline(masked_cone_pipeline);
					c.bind_descriptor_sets(&[descriptor_set]);
					for (view_index, layer) in cone_shadow_view_indices(masked_mesh_dispatch, cone_shadow_count) {
						c.write_push_constant(0, masked_mesh_dispatch.work_item_base());
						c.write_push_constant(4, view_index);
						c.write_push_constant(8, layer);
						c.dispatch_meshes(masked_mesh_dispatch.workgroup_count(), 1, 1);
					}
				}
				c.end_render_pass();
				c.end_region();
			}

			if point_shadow_count > 0 {
				log::debug!(
					"Point shadow pass executing: lights={}, faces={}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
					point_shadow_count,
					point_shadow_count * POINT_SHADOW_FACE_COUNT,
					instances.len(),
					drawable_instances,
					meshlet_count,
					mesh_dispatch.workgroup_count(),
				);
				c.start_region(|label| label.write_str("Point Shadow Map"));
				let attachments = [ghi::AttachmentInformation::new(
					point_shadow_map,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layers((point_shadow_count * POINT_SHADOW_FACE_COUNT) as u32)];
				let c = c.start_render_pass(point_extent, &attachments);
				let c = c.bind_raster_pipeline(cone_pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				for (view_index, layer) in point_shadow_view_indices(mesh_dispatch, point_shadow_count) {
					c.start_region(|label| label.write_str("Cube Face"));
					c.write_push_constant(0, mesh_dispatch.work_item_base());
					c.write_push_constant(4, view_index);
					c.write_push_constant(8, layer);
					c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
					c.end_region();
				}
				if !masked_mesh_dispatch.is_empty() {
					let c = c.bind_raster_pipeline(masked_cone_pipeline);
					c.bind_descriptor_sets(&[descriptor_set]);
					for (view_index, layer) in point_shadow_view_indices(masked_mesh_dispatch, point_shadow_count) {
						c.write_push_constant(0, masked_mesh_dispatch.work_item_base());
						c.write_push_constant(4, view_index);
						c.write_push_constant(8, layer);
						c.dispatch_meshes(masked_mesh_dispatch.workgroup_count(), 1, 1);
					}
				}
				c.end_render_pass();
				c.end_region();
			}
		}
	}
}
