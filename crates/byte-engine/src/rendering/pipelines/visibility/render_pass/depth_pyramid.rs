//! Positive linear view depth, reduced into a nearest-surface pyramid that screen-space passes share.
//!
//! [`DepthPyramidPass`] runs after the opaque visibility layer. Next, [`super::contact_shadows::ContactShadowPass`],
//! [`super::gtao::GtaoPass`] and [`super::ssgi::SsgiPass`] read [`DepthPyramidPass::depth_pyramid`] and
//! [`DepthPyramidPass::view_data`].

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use utils::Extent;

use super::Pipelines;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

/// Mips zero through two hold half, quarter, and eighth sink resolution. Nothing reads full-resolution linear depth, so
/// the pyramid starts at half resolution.
pub(super) const DEPTH_PYRAMID_MIP_COUNT: u32 = 3;

const VIEW_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
const INPUT_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1033);
const OUTPUT_BINDINGS: [ghi::ResourceSlot; 3] = [
	ghi::ResourceSlot::new(1034),
	ghi::ResourceSlot::new(1035),
	ghi::ResourceSlot::new(1036),
];

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
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Depth Pyramid Descriptor Set"));
		let view_data = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Screen View Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// Metal applies min/max reduction only when every sampler filter is linear, as the default ones are.
		// Centered samples then conservatively collapse each reversed-depth 2x2 footprint.
		let max_sampler = context.build_sampler(ghi::sampler::Builder::new().reduction_mode(ghi::SamplingReductionModes::Max));
		// The initial 8x8 allocation keeps all declared mips valid before the first sink resize.
		let depth_pyramid = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Linear Depth Pyramid")
				.extent(Extent::square(8))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(DEPTH_PYRAMID_MIP_COUNT),
		);
		let mut writes = vec![
			ghi::DescriptorWrite::buffer(descriptor_set, VIEW_BINDING, view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(descriptor_set, INPUT_BINDING, depth, max_sampler, ghi::Layouts::Read),
		];
		writes.extend(OUTPUT_BINDINGS.iter().enumerate().map(|(index, &binding)| {
			ghi::DescriptorWrite::image_mip(descriptor_set, binding, depth_pyramid, ghi::Layouts::General, index as u32)
		}));
		context.write(&writes);

		Self {
			descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, ["gtao-depth-pyramid"]),
			view_data,
			depth_pyramid,
		}
	}

	/// Uploads this frame's camera constants, resizes the pyramid, and returns the reduction recording.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<> {
		let half_extent = sink.extent().scaled_down(2);
		*frame.get_mut_dynamic_buffer_slice(self.view_data) = screen_view_data(sink, half_extent);
		frame.sync_buffer(self.view_data);
		frame.resize_image(self.depth_pyramid.into(), half_extent);
		let stage = super::ComputeStage {
			label: "Linear Depth Pyramid",
			pipeline,
			descriptor_sets: [self.descriptor_set],
			extent: half_extent,
			workgroup: Extent::new(8, 4, 1),
		};

		move |c| super::record_compute_stages(c, None, &[stage])
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
