//! Screen-space contact shadows for the sun, traced at half resolution against the linear depth pyramid.
//!
//! The directional shadow map cannot resolve shadows smaller than its texels, so small gaps of sunlight appear where
//! objects touch, such as under a foot on a floor. Each half-resolution texel marches a short ray toward the sun
//! through the depth pyramid and records how much visible geometry blocks it. A depth-aware filter then smooths the
//! dithered result into a full-resolution image, and material evaluation multiplies the sun's shadow by it.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use maths_rs::Vec4f;
use utils::Extent;

use super::depth_pyramid::ScreenViewData;
use super::gtao::configuration_float;
use super::{ComputeStage, Pipelines, record_compute_stages};
use crate::configuration::ConfigurationValue;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The configuration namespace for contact-shadow runtime controls.
pub const CONTACT_SHADOWS_CONFIGURATION_PREFIX: &str = "render.contact-shadows.";

/// The `ContactShadowSettings` struct defines the runtime controls for the contact-shadow trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContactShadowSettings {
	/// The world-space reach of each ray toward the sun. Occluders further away are left to the shadow map.
	pub(crate) max_distance: f32,
}

impl Default for ContactShadowSettings {
	fn default() -> Self {
		Self { max_distance: 0.15 }
	}
}

impl ContactShadowSettings {
	/// Applies one runtime parameter, returning the updated settings and the effective value, or leaves them unchanged.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"distance" => {
				let max_distance = configuration_float(value)
					.filter(|distance| *distance >= 0.0 && *distance <= f32::MAX as f64)
					.ok_or(
						"Contact shadow distance was not set. The most likely cause is that the value is not a finite nonnegative number.",
					)?;
				let settings = Self {
					max_distance: max_distance as f32,
				};
				Ok((settings, ConfigurationValue::Float(f64::from(settings.max_distance))))
			}
			_ => Err(
				"Contact shadow parameter was not set. The most likely cause is that the parameter name is unsupported."
					.to_string(),
			),
		}
	}
}

/// The render-graph name of the full-resolution filtered result: one where the ray toward the sun is clear, falling
/// toward zero where visible geometry blocks it. Material evaluation reads it only for the sun.
pub(crate) const CONTACT_SHADOWS_TARGET: &str = "Contact Shadows";
/// The render-graph name of the unfiltered half-resolution trace, which the filter reads. Capture it to debug the
/// trace alone.
pub(crate) const CONTACT_SHADOW_TRACE_TARGET: &str = "Contact Shadow Trace";
/// The trace runs at half the sink resolution, against mip zero of the linear depth pyramid.
const CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR: u32 = 2;

const VIEW_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
const PARAMETERS_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1);
const DEPTH_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1033);
const OUTPUT_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1034);
const FILTER_TRACE_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1035);
const FILTER_TRACE_DEPTH_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1036);

/// The `ContactShadowTargets` struct holds the images the contact-shadow trace writes and its filter smooths, so the
/// visibility pass can hand them to [`ContactShadowPass::new`] and bind the filtered one in material evaluation.
#[derive(Clone, Copy)]
pub(crate) struct ContactShadowTargets {
	/// The unfiltered half-resolution trace, named [`CONTACT_SHADOW_TRACE_TARGET`].
	pub(crate) trace: ghi::BaseImageHandle,
	/// The filtered result material evaluation reads, named [`CONTACT_SHADOWS_TARGET`].
	pub(crate) filtered: ghi::BaseImageHandle,
}

/// Creates the contact-shadow render targets for the sink that `render_pass_builder` sets up.
///
/// They are render targets so the renderer sizes them with the sink and they can be captured by name for debugging.
/// Next, pass them to the visibility render pass, which hands them to [`ContactShadowPass::new`].
pub(crate) fn create_contact_shadow_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> ContactShadowTargets {
	let mut target = |name, resolution_divisor| {
		render_pass_builder
			.create_scaled_render_target(
				ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				resolution_divisor,
			)
			.into()
	};
	ContactShadowTargets {
		trace: target(CONTACT_SHADOW_TRACE_TARGET, CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR),
		filtered: target(CONTACT_SHADOWS_TARGET, 1),
	}
}

/// The `ContactShadowShaderParameters` struct carries the per-frame light direction the trace marches toward and how
/// far it marches.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ContactShadowShaderParameters {
	/// The view-space unit direction from a surface toward the sun. W is unused.
	direction_to_light: [f32; 4],
	max_distance: f32,
}

/// Returns the view-space unit direction from a surface toward a directional light whose light travels along
/// `light_direction` in world space.
pub(crate) fn view_space_direction_to_light(view: View, light_direction: math::UnitVector) -> [f32; 4] {
	let light_direction = light_direction.into_maths();
	let direction = view.view() * Vec4f::new(-light_direction.x, -light_direction.y, -light_direction.z, 0.0);
	[direction.x, direction.y, direction.z, 0.0]
}

/// The `ContactShadowPass` struct fills the gaps the sun's shadow map leaves where objects touch.
///
/// It runs after [`super::depth_pyramid::DepthPyramidPass`] and before opaque material evaluation, which multiplies
/// the sun's shadow by [`CONTACT_SHADOWS_TARGET`]. Create its targets with [`create_contact_shadow_targets`].
pub(super) struct ContactShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	filter_descriptor_set: ghi::DescriptorSetHandle,
	/// The trace and filter pipelines.
	pub(super) pipelines: Pipelines<2>,
	parameters: ghi::DynamicBufferHandle<ContactShadowShaderParameters>,
}

impl ContactShadowPass {
	/// Wires the depth images and the targets, and requests the trace and filter pipelines.
	///
	/// `depth_pyramid` and `view_data` come from [`super::depth_pyramid::DepthPyramidPass`]: the trace marches mip
	/// zero with the half-resolution camera constants, and the filter reads the full-resolution `depth` with them.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		view_data: ghi::DynamicBufferHandle<ScreenViewData>,
		targets: ContactShadowTargets,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Contact Shadow Descriptor Set"));
		let filter_descriptor_set = context.create_descriptor_set(Some("Contact Shadow Filter Descriptor Set"));
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Contact Shadow Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		let sampled = |set, slot, image: ghi::BaseImageHandle| {
			ghi::DescriptorWrite::combined_image_sampler(set, slot, image, point_sampler, ghi::Layouts::Read)
		};
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, VIEW_BINDING, view_data.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, PARAMETERS_BINDING, parameters.into()),
			sampled(descriptor_set, DEPTH_BINDING, depth_pyramid.into()),
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_BINDING, targets.trace, ghi::Layouts::General),
			ghi::DescriptorWrite::buffer(filter_descriptor_set, VIEW_BINDING, view_data.into()),
			sampled(filter_descriptor_set, DEPTH_BINDING, depth),
			sampled(filter_descriptor_set, FILTER_TRACE_BINDING, targets.trace),
			sampled(filter_descriptor_set, FILTER_TRACE_DEPTH_BINDING, depth_pyramid.into()),
			ghi::DescriptorWrite::image(filter_descriptor_set, OUTPUT_BINDING, targets.filtered, ghi::Layouts::General),
		]);

		Self {
			descriptor_set,
			filter_descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, ["contact-shadows", "contact-shadows-filter"]),
			parameters,
		}
	}

	/// Uploads this frame's sun direction and ray reach, and returns the trace and filter recording, or `None` without
	/// a sun, because material evaluation reads the result only for the sun.
	///
	/// `sun_direction` is the world-space direction the sun's light travels. `settings` sets the ray reach.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		sun_direction: Option<math::UnitVector>,
		settings: ContactShadowSettings,
		[trace, filter]: [ghi::PipelineHandle; 2],
	) -> Option<impl RenderPassFunction + use<>> {
		let sun_direction = sun_direction?;
		let extent = sink.extent();
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = ContactShadowShaderParameters {
			direction_to_light: view_space_direction_to_light(sink.view(), sun_direction),
			max_distance: settings.max_distance,
		};
		frame.sync_buffer(self.parameters);
		let stage = |label, pipeline, descriptor_set, extent| ComputeStage {
			label,
			pipeline,
			descriptor_sets: [descriptor_set],
			extent,
			workgroup: Extent::new(8, 8, 1),
		};
		let stages = [
			stage(
				"Contact Shadow Trace",
				trace,
				self.descriptor_set,
				extent.scaled_down(CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR),
			),
			stage("Contact Shadow Filter", filter, self.filter_descriptor_set, extent),
		];

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			record_compute_stages(c, Some("Contact Shadows"), &stages)
		})
	}
}

#[cfg(test)]
mod tests {
	use math::{Degrees, Point, UnitVector};

	use super::*;

	#[test]
	fn a_sun_shining_straight_down_lies_above_every_surface_in_view_space() {
		let view = View::new_perspective(
			Degrees::new(60.0),
			1.0,
			0.1,
			100.0,
			Point::new(0.0, 1.0, -5.0),
			UnitVector::z_axis(),
		);
		let straight_down = math::Vector::new(0.0, -1.0, 0.0).normalized().expect("unit direction");

		let direction = view_space_direction_to_light(view, straight_down);

		for (actual, expected) in direction.into_iter().zip([0.0, 1.0, 0.0, 0.0]) {
			assert!((actual - expected).abs() < 0.0001, "{direction:?}");
		}
	}
}
