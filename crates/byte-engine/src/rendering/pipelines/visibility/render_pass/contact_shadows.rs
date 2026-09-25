//! Screen-space contact shadows for the sun, traced against full-resolution depth.
//!
//! The directional shadow map cannot resolve shadows smaller than its texels, so small gaps of sunlight appear where
//! objects touch, such as under a foot on a floor. Each pixel marches a short ray toward the sun through the depth
//! buffer and records how much visible geometry blocks it. A depth-aware filter then smooths the dithered result, and
//! material evaluation multiplies the sun's shadow by it.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use maths_rs::Vec4f;
use utils::Extent;

use super::depth_pyramid::{ScreenViewData, screen_view_data};
use super::gtao::configuration_float;
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
/// The render-graph name of the unfiltered trace, which the filter reads. Capture it to debug the trace alone.
pub(crate) const CONTACT_SHADOW_TRACE_TARGET: &str = "Contact Shadow Trace";

const VIEW_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const FILTER_TRACE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);

/// The `ContactShadowTargets` struct holds the images the contact-shadow trace writes and its filter smooths, so the
/// visibility pass can hand them to [`ContactShadowPass::new`] and bind the filtered one in material evaluation.
#[derive(Clone, Copy)]
pub(crate) struct ContactShadowTargets {
	/// The unfiltered trace, named [`CONTACT_SHADOW_TRACE_TARGET`].
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
	let mut target = |name| {
		render_pass_builder
			.create_render_target(
				ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
			)
			.into()
	};
	ContactShadowTargets {
		trace: target(CONTACT_SHADOW_TRACE_TARGET),
		filtered: target(CONTACT_SHADOWS_TARGET),
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
/// It runs after the opaque visibility layer and before opaque material evaluation, which multiplies the sun's
/// shadow by [`CONTACT_SHADOWS_TARGET`]. Create its targets with [`create_contact_shadow_targets`].
pub(super) struct ContactShadowPass {
	settings: ContactShadowSettings,
	descriptor_set: ghi::DescriptorSetHandle,
	filter_descriptor_set: ghi::DescriptorSetHandle,
	pipeline: crate::rendering::PipelineRef,
	filter_pipeline: crate::rendering::PipelineRef,
	/// Full-resolution camera constants. The shared screen view data describes the half-resolution pyramid.
	view_data: ghi::DynamicBufferHandle<ScreenViewData>,
	parameters: ghi::DynamicBufferHandle<ContactShadowShaderParameters>,
}

/// The `ContactShadowPipelines` struct holds the trace and filter pipelines once both have compiled.
pub(super) struct ContactShadowPipelines {
	trace: ghi::PipelineHandle,
	filter: ghi::PipelineHandle,
}

impl ContactShadowPass {
	/// Wires full-resolution depth and the targets, and requests the trace and filter pipelines.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		targets: ContactShadowTargets,
		settings: ContactShadowSettings,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Contact Shadow Descriptor Set"));
		let filter_descriptor_set = context.create_descriptor_set(Some("Contact Shadow Filter Descriptor Set"));
		let host_buffer = |name| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let view_data = context.build_dynamic_buffer(host_buffer("Contact Shadow View Data"));
		let parameters = context.build_dynamic_buffer(host_buffer("Contact Shadow Parameters"));
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(descriptor_set, PARAMETERS_BINDING.slot(), parameters.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				DEPTH_BINDING.slot(),
				depth,
				point_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_BINDING.slot(), targets.trace, ghi::Layouts::General),
			ghi::DescriptorWrite::buffer(filter_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				filter_descriptor_set,
				DEPTH_BINDING.slot(),
				depth,
				point_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				filter_descriptor_set,
				FILTER_TRACE_BINDING.slot(),
				targets.trace,
				point_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				filter_descriptor_set,
				OUTPUT_BINDING.slot(),
				targets.filtered,
				ghi::Layouts::General,
			),
		]);

		Self {
			settings,
			descriptor_set,
			filter_descriptor_set,
			pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/contact-shadows.pipeline"),
			filter_pipeline: pipeline_manager
				.request_pipeline("byte-engine/rendering/visibility/contact-shadows-filter.pipeline"),
			view_data,
			parameters,
		}
	}

	pub(super) fn set_settings(&mut self, settings: ContactShadowSettings) {
		self.settings = settings;
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<ContactShadowPipelines> {
		Some(ContactShadowPipelines {
			trace: pipeline_manager.pipeline(self.pipeline)?,
			filter: pipeline_manager.pipeline(self.filter_pipeline)?,
		})
	}

	/// Uploads this frame's camera constants, sun direction and ray reach, and returns the trace and filter recording.
	///
	/// `sun_direction` is the world-space direction the sun's light travels. Without a sun the recording does
	/// nothing, because material evaluation reads the result only for the sun.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		sun_direction: Option<math::UnitVector>,
		pipelines: ContactShadowPipelines,
	) -> impl RenderPassFunction + use<> {
		let extent = sink.extent();
		if let Some(sun_direction) = sun_direction {
			*frame.get_mut_dynamic_buffer_slice(self.view_data) = screen_view_data(sink, extent);
			frame.sync_buffer(self.view_data);
			*frame.get_mut_dynamic_buffer_slice(self.parameters) = ContactShadowShaderParameters {
				direction_to_light: view_space_direction_to_light(sink.view(), sun_direction),
				max_distance: self.settings.max_distance,
			};
			frame.sync_buffer(self.parameters);
		}
		let stages = [
			("Contact Shadow Trace", pipelines.trace, self.descriptor_set),
			("Contact Shadow Filter", pipelines.filter, self.filter_descriptor_set),
		];
		let enabled = sun_direction.is_some();

		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			if !enabled {
				return;
			}
			c.start_region(|label| label.write_str("Contact Shadows"));
			for (name, pipeline, descriptor_set) in stages {
				c.start_region(|label| label.write_str(name));
				let c = c.bind_compute_pipeline(pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
				c.end_region();
			}
			c.end_region();
		}
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
		let straight_down = UnitVector::try_from_vector(math::Vector::new(0.0, -1.0, 0.0)).expect("unit direction");

		let direction = view_space_direction_to_light(view, straight_down);

		for (actual, expected) in direction.into_iter().zip([0.0, 1.0, 0.0, 0.0]) {
			assert!((actual - expected).abs() < 0.0001, "{direction:?}");
		}
	}

	#[test]
	fn distance_parameter_sets_the_ray_reach_and_rejects_negative_values() {
		let (settings, effective) = ContactShadowSettings::default()
			.with_parameter("distance", &ConfigurationValue::Text("0.4".to_string()))
			.expect("distance should parse");

		assert_eq!(settings.max_distance, 0.4);
		assert_eq!(effective, ConfigurationValue::Float(f64::from(0.4f32)));
		assert!(settings.with_parameter("distance", &ConfigurationValue::Float(-1.0)).is_err());
		assert!(settings.with_parameter("reach", &ConfigurationValue::Float(1.0)).is_err());
	}
}
