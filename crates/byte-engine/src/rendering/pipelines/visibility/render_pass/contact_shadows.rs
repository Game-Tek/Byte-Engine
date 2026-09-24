//! Screen-space contact shadows for the sun, traced against full-resolution depth.
//!
//! The directional shadow map cannot resolve shadows smaller than its texels, so small gaps of sunlight appear where
//! objects touch, such as under a foot on a floor. Each pixel marches a short ray toward the sun through the depth
//! buffer and records whether visible geometry blocks it. Material evaluation multiplies the sun's shadow by the
//! result.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use maths_rs::Vec4f;
use utils::Extent;

use super::depth_pyramid::{ScreenViewData, screen_view_data};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The render-graph name of the full-resolution result: one where the ray toward the sun is clear, zero where
/// visible geometry blocks it. Material evaluation reads it only for the sun.
pub(crate) const CONTACT_SHADOWS_TARGET: &str = "Contact Shadows";

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

/// Creates the contact-shadow render target for the sink that `render_pass_builder` sets up.
///
/// It is a render target so the renderer sizes it with the sink and it can be captured by name for debugging.
/// Next, pass it to the visibility render pass, which hands it to [`ContactShadowPass::new`].
pub(crate) fn create_contact_shadow_target(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> ghi::BaseImageHandle {
	render_pass_builder
		.create_render_target(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name(CONTACT_SHADOWS_TARGET)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		)
		.into()
}

/// The `ContactShadowShaderParameters` struct carries the per-frame light direction the trace marches toward.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ContactShadowShaderParameters {
	/// The view-space unit direction from a surface toward the sun. W is unused.
	direction_to_light: [f32; 4],
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
/// shadow by [`CONTACT_SHADOWS_TARGET`]. Create its target with [`create_contact_shadow_target`].
pub(super) struct ContactShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline: crate::rendering::PipelineRef,
	/// Full-resolution camera constants. The shared screen view data describes the half-resolution pyramid.
	view_data: ghi::DynamicBufferHandle<ScreenViewData>,
	parameters: ghi::DynamicBufferHandle<ContactShadowShaderParameters>,
}

impl ContactShadowPass {
	/// Wires full-resolution depth and the output target, and requests the trace pipeline.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		contact_shadows: ghi::BaseImageHandle,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Contact Shadow Descriptor Set"));
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
			ghi::DescriptorWrite::image(descriptor_set, OUTPUT_BINDING.slot(), contact_shadows, ghi::Layouts::General),
		]);

		Self {
			descriptor_set,
			pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/contact-shadows.pipeline"),
			view_data,
			parameters,
		}
	}

	pub(super) fn pipeline(&self, pipeline_manager: &PipelineManagerClient) -> Option<ghi::PipelineHandle> {
		pipeline_manager.pipeline(self.pipeline)
	}

	/// Uploads this frame's camera constants and sun direction, and returns the trace recording.
	///
	/// `sun_direction` is the world-space direction the sun's light travels. Without a sun the recording does
	/// nothing, because material evaluation reads the result only for the sun.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		sun_direction: Option<math::UnitVector>,
		pipeline: ghi::PipelineHandle,
	) -> impl RenderPassFunction + use<> {
		let extent = sink.extent();
		if let Some(sun_direction) = sun_direction {
			*frame.get_mut_dynamic_buffer_slice(self.view_data) = screen_view_data(sink, extent);
			frame.sync_buffer(self.view_data);
			*frame.get_mut_dynamic_buffer_slice(self.parameters) = ContactShadowShaderParameters {
				direction_to_light: view_space_direction_to_light(sink.view(), sun_direction),
			};
			frame.sync_buffer(self.parameters);
		}
		let descriptor_set = self.descriptor_set;
		let enabled = sun_direction.is_some();

		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			if !enabled {
				return;
			}
			c.start_region(|label| label.write_str("Contact Shadows"));
			let c = c.bind_compute_pipeline(pipeline);
			c.bind_descriptor_sets(&[descriptor_set]);
			c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
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
}
