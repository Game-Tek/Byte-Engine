//! Screen-space reflections, traced by material evaluation itself.
//!
//! A visibility buffer has no surface normal or roughness before material evaluation, so a separate pass could not
//! aim the rays. Instead, each material-evaluation invocation traces one mirror ray through this frame's linear depth
//! pyramid and reads the light at the hit from the previous frame's lit image, using the previous frame's depth to find
//! the pixel that saw the hit. Rays that miss, and rough surfaces, keep the prefiltered environment. Transparent
//! surfaces trace too, so glass and water reflect the opaque scene behind the camera's view.
//!
//! This module owns the per-sink inputs those rays read.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;

use super::SinkHistory;
use super::depth_pyramid::{DEPTH_PYRAMID_MIP_COUNT, depth_unprojection};
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::pipelines::visibility::shader_data::ReflectionShaderParameters;

/// The `ScreenSpaceReflections` struct keeps the per-sink data material evaluation's reflection rays read.
///
/// Create it with the material-evaluation descriptor set, then call [`Self::prepare`] every frame before material
/// evaluation records.
pub(super) struct ScreenSpaceReflections {
	parameters: ghi::DynamicBufferHandle<ReflectionShaderParameters>,
}

impl ScreenSpaceReflections {
	/// Binds the depth pyramid, the previous frame's copies of `lit` and `depth`, and the per-frame parameters into
	/// `material_evaluation_descriptor_set`.
	///
	/// `depth_pyramid` comes from [`super::depth_pyramid::DepthPyramidPass`]. Create `lit` and `depth` with
	/// [`crate::rendering::RenderPassBuilder::create_history_render_target`].
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		lit: ghi::DynamicImageHandle,
		depth: ghi::DynamicImageHandle,
	) -> Self {
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Reflection Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// Point sampling keeps a hit's light from blending with the background next to the hit object.
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let evaluation = set_layouts::MATERIAL_EVALUATION.binder(material_evaluation_descriptor_set);
		context.write(&[
			evaluation.buffer("reflection_parameters", parameters),
			evaluation.sampled("depth_pyramid", depth_pyramid, point_sampler),
			evaluation.sampled_with_frame("previous_lit", lit, point_sampler, -1),
			evaluation.sampled_with_frame("previous_depth", depth, point_sampler, -1),
		]);

		Self { parameters }
	}

	/// Uploads this frame's reprojection into the previous frame's lit image and depth.
	///
	/// `history` describes the previous frame's images, or is `None` when they do not hold this sink's data. Without
	/// it every ray misses, and material evaluation keeps the environment.
	pub(super) fn prepare(&self, frame: &mut ghi::implementation::Frame, history: Option<SinkHistory>) {
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = history
			.map(|history| {
				let (numerator, denominator_offset) = depth_unprojection(&history.view);
				ReflectionShaderParameters {
					world_to_previous_clip: history.view.view_projection().into(),
					previous_exposure: history.exposure,
					history_valid: 1,
					previous_depth_unproject_numerator: numerator,
					previous_depth_unproject_denominator_offset: denominator_offset,
				}
			})
			.unwrap_or_default();
		frame.sync_buffer(self.parameters);
	}
}
