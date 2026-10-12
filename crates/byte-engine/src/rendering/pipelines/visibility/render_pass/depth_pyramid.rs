//! Positive linear view depth, reduced into a nearest-surface pyramid that screen-space passes share.
//!
//! [`DepthPyramidPass`] runs after the opaque visibility layer. Next, [`super::contact_shadows::ContactShadowPass`],
//! [`super::gtao::GtaoPass`] and [`super::ssgi::SsgiPass`] read [`DepthPyramidPass::depth_pyramid`] and
//! [`DepthPyramidPass::view_data`].

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::Pipelines;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

/// Mips zero through two hold half, quarter, and eighth sink resolution. Nothing reads full-resolution linear depth, so
/// the pyramid starts at half resolution.
pub(super) const DEPTH_PYRAMID_MIP_COUNT: u32 = 3;

/// The pyramid's mips, as `layouts/depth-pyramid.besl` names the image each one is written through.
const REDUCED_DEPTH_NAMES: [&str; DEPTH_PYRAMID_MIP_COUNT as usize] = ["reduced_depth_1", "reduced_depth_2", "reduced_depth_3"];

/// The `ScreenViewData` struct gives half-resolution screen-space passes compact camera reconstruction constants.
///
/// Shaders rebuild a view-space position from a pixel and its linear depth, and project a view-space position
/// back to a pixel, without a full matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ScreenViewData {
	pub(crate) pixel_to_ray_mul: [f32; 2],
	pub(crate) pixel_to_ray_add: [f32; 2],
	pub(crate) projection_pixels_y: f32,
	pub(crate) view_z_sign: f32,
	pub(crate) depth_unproject_numerator: f32,
	pub(crate) depth_unproject_denominator_offset: f32,
}

/// Builds pixel-ray and reversed-depth reconstruction constants for one perspective sink.
pub(crate) fn screen_view_data(sink: &Sink, extent: Extent) -> ScreenViewData {
	let view = sink.view();
	let projection = view.projection();
	let width = extent.width() as f32;
	let height = extent.height() as f32;
	let projection_x = projection[0];
	let projection_y = projection[5];
	let near = view.near();
	let far = view.far();
	let clip_range = far - near;
	debug_assert!(
		width > 0.0 && height > 0.0 && projection_x > 0.0 && projection_y > 0.0 && near > 0.0 && far > near,
		"Screen-space camera constants are invalid. The most likely cause is an empty target or a non-perspective sink."
	);
	ScreenViewData {
		pixel_to_ray_mul: [2.0 / (width * projection_x), -2.0 / (height * projection_y)],
		pixel_to_ray_add: [(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y],
		projection_pixels_y: height * projection_y * 0.5,
		// Byte Engine perspective views look down positive view-space Z.
		view_z_sign: 1.0,
		// projection_matrix() maps z to depth as a + b / z. These constants reconstruct positive z as b / (depth - a).
		depth_unproject_numerator: near * far / clip_range,
		depth_unproject_denominator_offset: near / clip_range,
	}
}

/// The `DepthPyramidPass` struct owns the linear depth pyramid that every half-resolution screen-space pass samples.
///
/// The pyramid is a per-frame image, so temporal passes can also read the previous frame's depth to detect
/// disocclusion.
pub(super) struct DepthPyramidPass {
	descriptor_set: ghi::DescriptorSetHandle,
	/// The set that holds [`Self::view_data`], which the GTAO passes bind too.
	pub(super) view_descriptor_set: ghi::DescriptorSetHandle,
	pub(super) pipelines: Pipelines<1>,
	/// The half-resolution camera constants that match the pyramid's mip zero.
	pub(super) view_data: ghi::DynamicBufferHandle<ScreenViewData>,
	/// The pyramid whose mips zero through two hold nearest positive linear depth from half resolution down.
	pub(super) depth_pyramid: ghi::DynamicImageHandle,
}

impl DepthPyramidPass {
	/// Creates the pyramid and the half-resolution camera constants, and requests the reduction pipeline.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		receiver_bounds: ghi::BaseBufferHandle,
		receiver_fit: ghi::BaseBufferHandle,
	) -> Self {
		let descriptor_set = set_layouts::DEPTH_PYRAMID.create_set(context, "Depth Pyramid Descriptor Set");
		let view_descriptor_set = set_layouts::GTAO_VIEW.create_set(context, "Screen View Descriptor Set");
		let view_data = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Screen View Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// The pass gathers texel quads, which no sampler filter or reduction touches.
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		// The initial 8x8 allocation keeps all declared mips valid before the first sink resize.
		let depth_pyramid = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Linear Depth Pyramid")
				.extent(Extent::square(8))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(DEPTH_PYRAMID_MIP_COUNT),
		);
		let pyramid = set_layouts::DEPTH_PYRAMID.binder(descriptor_set);
		let [first, second, third] =
			[0, 1, 2].map(|mip| pyramid.image_mip(REDUCED_DEPTH_NAMES[mip], depth_pyramid, mip as u32));
		context.write(&[
			set_layouts::GTAO_VIEW
				.binder(view_descriptor_set)
				.buffer("gtao_view", view_data),
			pyramid.sampled("source_depth", depth, point_sampler),
			// The directional shadow receiver bounds the pass finds, and the CPU's cascade fit they are measured against.
			pyramid.buffer("receiver_bounds", receiver_bounds),
			pyramid.buffer("receiver_fit", receiver_fit),
			first,
			second,
			third,
		]);

		Self {
			descriptor_set,
			view_descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, ["depth-pyramid"]),
			view_data,
			depth_pyramid,
		}
	}

	/// Uploads this frame's camera constants, resizes the pyramid, and returns the reduction recording. The pass also
	/// finds the directional shadow receiver bounds of the first `fitted_suns` sun slots, which the cascade fit reads
	/// after it.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		pipeline: ghi::PipelineHandle,
		fitted_suns: u32,
	) -> impl RenderPassFunction + use<> {
		let half_extent = sink.extent().scaled_down(2);
		*frame.get_mut_dynamic_buffer_slice(self.view_data) = screen_view_data(sink, half_extent);
		frame.sync_buffer(self.view_data);
		frame.resize_image(self.depth_pyramid.into(), half_extent);
		let descriptor_sets = [self.view_descriptor_set, self.descriptor_set];

		move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			c.start_region(|label| label.write_str("Linear Depth Pyramid"));
			let c = c.bind_compute_pipeline(pipeline);
			c.bind_descriptor_sets(&descriptor_sets);
			c.write_push_constant(0, [fitted_suns]);
			c.dispatch(ghi::DispatchExtent::new(half_extent, Extent::square(16)));
			c.end_region();
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Point, UnitVector};
	use maths_rs::Vec4f;

	use super::*;
	use crate::rendering::View;

	#[test]
	fn screen_view_reconstructs_pixel_rays_and_reversed_depth() {
		let extent = Extent::rectangle(1920, 1080);
		let view = View::new_perspective(
			math::Degrees::new(60.0),
			extent.width() as f32 / extent.height() as f32,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let sink = Sink::new(view, extent, 0);
		let half_extent = extent.scaled_down(2);
		let constants = screen_view_data(&sink, half_extent);
		let projection = view.projection();

		assert_eq!(std::mem::size_of_val(&constants), 32);

		for z in [0.1f32, 0.5, 1.0, 10.0, 100.0] {
			let clip = projection * Vec4f::new(0.0, 0.0, z, 1.0);
			let depth = clip.z / clip.w;
			let reconstructed = constants.depth_unproject_numerator / (depth + constants.depth_unproject_denominator_offset);
			assert!(
				(reconstructed - z).abs() <= z.max(1.0) * 0.00001,
				"Unexpected depth reconstruction for z={z}: {reconstructed}"
			);
		}

		for pixel in [[0.0f32, 0.0], [479.0, 269.0], [959.0, 539.0]] {
			let ray = [
				pixel[0] * constants.pixel_to_ray_mul[0] + constants.pixel_to_ray_add[0],
				pixel[1] * constants.pixel_to_ray_mul[1] + constants.pixel_to_ray_add[1],
			];
			let ndc = [
				2.0 * (pixel[0] + 0.5) / half_extent.width() as f32 - 1.0,
				1.0 - 2.0 * (pixel[1] + 0.5) / half_extent.height() as f32,
			];
			assert!((ray[0] - ndc[0] / projection[0]).abs() < 0.000001);
			assert!((ray[1] - ndc[1] / projection[5]).abs() < 0.000001);
		}

		assert_eq!(constants.view_z_sign, 1.0);
		assert_eq!(half_extent, Extent::rectangle(960, 540));
	}
}
