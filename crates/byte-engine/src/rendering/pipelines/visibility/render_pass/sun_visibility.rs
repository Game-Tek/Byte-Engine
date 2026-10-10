//! Sun visibility: each shadowed sun's filtered shadow-map visibility times its screen-space contact shadow, resolved
//! once per opaque pixel into one full-resolution image, with sun slot `s` in channel `s`.
//!
//! Opaque material evaluation multiplies each sun by its channel of one fetch of [`SUN_VISIBILITY_TARGET`]. Resolving the shadow map
//! in a pass of its own keeps its many fetches out of the material shader, where they hide their latency poorly, and
//! lets the GPU overlap them with the other screen-space passes. Transparent surfaces lie in front of the opaque depth
//! the pass resolves, so the material shader still resolves the map for them itself.
//!
//! The contact shadow fills the gaps the shadow map cannot resolve where objects touch, such as under a foot on a
//! floor: each half-resolution texel marches a short ray toward the sun through the linear depth pyramid and records
//! how much visible geometry blocks it. The resolve smooths that dithered trace into full resolution with a
//! depth-aware filter and multiplies it by the shadow map's visibility.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::pod::Vec3f;
use maths_rs::Vec4f;
use utils::Extent;

use super::super::layout::MAX_DIRECTIONAL_SHADOW_COUNT;
use super::super::shadow_selection::SunShadow;
use super::depth_pyramid::{ScreenViewData, screen_view_data};
use super::gtao::{configuration_bool, configuration_float};
use super::shadows::{DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT, ShadowMaps};
use super::{ComputeStage, Pipelines};
use crate::configuration::ConfigurationValue;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The configuration namespace for contact-shadow runtime controls.
pub const CONTACT_SHADOWS_CONFIGURATION_PREFIX: &str = "render.contact-shadows.";

/// The `ContactShadowSettings` struct defines the runtime controls for the contact-shadow trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContactShadowSettings {
	/// Whether the trace runs. Without it, the sun's shadow comes from the shadow map alone.
	pub(crate) enabled: bool,
	/// The world-space reach of each ray toward the sun. Occluders further away are left to the shadow map.
	pub(crate) max_distance: f32,
}

impl Default for ContactShadowSettings {
	fn default() -> Self {
		Self {
			enabled: true,
			max_distance: 0.15,
		}
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
			"enabled" => {
				let enabled = configuration_bool(value).ok_or(
					"Contact shadows enabled was not set. The most likely cause is that the value is neither `true` nor `false`.",
				)?;
				Ok((Self { enabled, ..self }, ConfigurationValue::Bool(enabled)))
			}
			"distance" => {
				let max_distance = configuration_float(value)
					.filter(|distance| *distance >= 0.0 && *distance <= f32::MAX as f64)
					.ok_or(
						"Contact shadow distance was not set. The most likely cause is that the value is not a finite nonnegative number.",
					)?;
				let settings = Self {
					max_distance: max_distance as f32,
					..self
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

/// The render-graph name of the full-resolution result: per sun slot, one where that sun reaches the pixel, falling
/// toward zero where its shadow map or visible geometry blocks it. Material evaluation reads it for every shadowed sun.
pub(crate) const SUN_VISIBILITY_TARGET: &str = "Sun Visibility";
/// The render-graph name of the unfiltered half-resolution contact-shadow trace, which the resolve reads. Capture it
/// to debug the trace alone.
pub(crate) const CONTACT_SHADOW_TRACE_TARGET: &str = "Contact Shadow Trace";
/// The trace runs at half the sink resolution, against mip zero of the linear depth pyramid.
const CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR: u32 = 2;

/// The trace pipeline, named as [`Pipelines::request`] takes it. Only contact shadows use it.
pub(in crate::rendering::pipelines::visibility) const CONTACT_SHADOW_PIPELINES: [&str; 1] = ["contact-shadows"];

// The trace's set, and the slots the resolve's set shares with it.
const VIEW_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(0);
const PARAMETERS_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1);
const DEPTH_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1033);
const OUTPUT_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1034);
// The rest of the resolve's set, which is bound after the base set that holds the views.
const RESOLVE_TRACE_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1035);
const RESOLVE_TRACE_DEPTH_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1036);
const RESOLVE_VIEW_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1037);
const RESOLVE_PARAMETERS_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1038);
const RESOLVE_SHADOW_MAP_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1039);
const RESOLVE_SHADOW_DEPTH_PYRAMID_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1040);
const RESOLVE_SHADOW_DEPTH_MINIMUM_PYRAMID_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1041);
// The maximum pyramid once more, with the point sampler, for the blocker search's gathers.
const RESOLVE_SHADOW_DEPTH_CELLS_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1042);

/// The `SunVisibilityTargets` struct holds the image the contact-shadow trace writes and the image the resolve writes,
/// so the visibility pass can hand them to [`SunVisibilityPass::new`] and bind the result in material evaluation.
#[derive(Clone, Copy)]
pub(crate) struct SunVisibilityTargets {
	/// The unfiltered half-resolution contact-shadow trace, named [`CONTACT_SHADOW_TRACE_TARGET`].
	pub(crate) trace: ghi::BaseImageHandle,
	/// The result material evaluation reads, named [`SUN_VISIBILITY_TARGET`].
	pub(crate) visibility: ghi::BaseImageHandle,
}

/// Creates the sun visibility render targets for the sink that `render_pass_builder` sets up.
///
/// They are render targets so the renderer sizes them with the sink and they can be captured by name for debugging.
/// Next, pass them to the visibility render pass, which hands them to [`SunVisibilityPass::new`].
pub(crate) fn create_sun_visibility_targets(
	render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>,
) -> SunVisibilityTargets {
	let mut target = |name, resolution_divisor| {
		render_pass_builder
			.create_scaled_render_target(
				// One channel per sun slot, so every shadowed sun resolves in one dispatch that shares its tile loads.
				ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
					.name(name)
					.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				resolution_divisor,
			)
			.into()
	};
	SunVisibilityTargets {
		trace: target(CONTACT_SHADOW_TRACE_TARGET, CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR),
		visibility: target(SUN_VISIBILITY_TARGET, 1),
	}
}

/// The `SunVisibilityShaderParameters` struct carries what both stages need from the CPU each frame: every sun's
/// direction and the contact rays' reach for the trace, and every sun's size and the camera's full-resolution pixel
/// rays for the resolve.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SunVisibilityShaderParameters {
	/// Per sun slot, the view-space unit direction from a surface toward the sun in xyz and the tangent of the sun's
	/// angular radius, which sizes its penumbrae, in w.
	pub(crate) suns: [[f32; 4]; MAX_DIRECTIONAL_SHADOW_COUNT],
	/// The camera's full-resolution pixel-to-ray scale in xy and offset in zw.
	pub(crate) pixel_to_ray: [f32; 4],
	pub(crate) max_distance: f32,
	/// How many sun slots hold a shadowed sun.
	pub(crate) sun_count: u32,
	/// Nonzero when the resolve multiplies in the contact-shadow trace this frame.
	pub(crate) contact_shadows: u32,
	pub(crate) _padding: u32,
}

/// Returns the view-space unit direction from a surface toward a directional light whose light travels along
/// `light_direction` in world space.
pub(crate) fn view_space_direction_to_light(view: View, light_direction: math::UnitVector) -> Vec3f {
	let light_direction = light_direction.into_maths();
	let direction = view.view() * Vec4f::new(-light_direction.x, -light_direction.y, -light_direction.z, 0.0);
	Vec3f::new(direction.x, direction.y, direction.z)
}

/// The `SunVisibilityPass` struct resolves the sun's shadow for every opaque pixel, so material evaluation reads one
/// value instead of filtering the shadow map and the contact shadows itself.
///
/// It runs after [`super::depth_pyramid::DepthPyramidPass`] and the shadow maps, and before opaque material
/// evaluation, which multiplies the sun's light by [`SUN_VISIBILITY_TARGET`]. Create its targets with
/// [`create_sun_visibility_targets`].
pub(super) struct SunVisibilityPass {
	trace_descriptor_set: ghi::DescriptorSetHandle,
	/// The base visibility set, whose views hold the camera and the sun's cascades, then the resolve's own set.
	resolve_descriptor_sets: [ghi::DescriptorSetHandle; 2],
	/// The contact-shadow trace pipeline, absent when contact shadows are left out.
	pub(super) trace_pipelines: Option<Pipelines<1>>,
	pub(super) resolve_pipelines: Pipelines<1>,
	parameters: ghi::DynamicBufferHandle<SunVisibilityShaderParameters>,
}

impl SunVisibilityPass {
	/// Wires the depth images, the shadow maps, and the targets, and requests the trace and resolve pipelines.
	///
	/// `base_descriptor_set` is the sink's base visibility set, whose views the resolve reads the cascades from.
	/// `depth_pyramid` and `view_data` come from [`super::depth_pyramid::DepthPyramidPass`]: the trace marches mip
	/// zero with the half-resolution camera constants, and the resolve reads the full-resolution `depth` with them.
	/// `shadow_maps` holds the sun's cascades and their depth pyramid. Without `contact_shadows`, the trace pipeline is
	/// not requested.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		view_data: ghi::DynamicBufferHandle<ScreenViewData>,
		shadow_maps: &ShadowMaps,
		targets: SunVisibilityTargets,
		contact_shadows: bool,
	) -> Self {
		let trace_descriptor_set = context.create_descriptor_set(Some("Contact Shadow Trace Descriptor Set"));
		let resolve_descriptor_set = context.create_descriptor_set(Some("Sun Visibility Descriptor Set"));
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sun Visibility Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		// Point sampling keeps a shadow edge from bleeding one texel onto the lit surface beside it.
		let shadow_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {}),
		);
		// Reduction sampling gives the fully-lit probe the nearest occluder of four pyramid cells in one sample, and
		// the fully-shadowed probe the farthest.
		let pyramid_sampler = |reduction_mode| {
			ghi::sampler::Builder::new()
				.reduction_mode(reduction_mode)
				.max_lod((DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT - 1) as f32)
		};
		let shadow_pyramid_sampler = context.build_sampler(pyramid_sampler(ghi::SamplingReductionModes::Max));
		let shadow_minimum_pyramid_sampler = context.build_sampler(pyramid_sampler(ghi::SamplingReductionModes::Min));
		let sampled = |set, slot, image: ghi::BaseImageHandle, sampler| {
			ghi::DescriptorWrite::combined_image_sampler(set, slot, image, sampler, ghi::Layouts::Read)
		};
		context.write(&[
			ghi::DescriptorWrite::buffer(trace_descriptor_set, VIEW_BINDING, view_data.into()),
			ghi::DescriptorWrite::buffer(trace_descriptor_set, PARAMETERS_BINDING, parameters.into()),
			sampled(trace_descriptor_set, DEPTH_BINDING, depth_pyramid.into(), point_sampler),
			ghi::DescriptorWrite::image(trace_descriptor_set, OUTPUT_BINDING, targets.trace, ghi::Layouts::General),
			sampled(resolve_descriptor_set, DEPTH_BINDING, depth, point_sampler),
			ghi::DescriptorWrite::image(
				resolve_descriptor_set,
				OUTPUT_BINDING,
				targets.visibility,
				ghi::Layouts::General,
			),
			sampled(resolve_descriptor_set, RESOLVE_TRACE_BINDING, targets.trace, point_sampler),
			sampled(
				resolve_descriptor_set,
				RESOLVE_TRACE_DEPTH_BINDING,
				depth_pyramid.into(),
				point_sampler,
			),
			ghi::DescriptorWrite::buffer(resolve_descriptor_set, RESOLVE_VIEW_BINDING, view_data.into()),
			ghi::DescriptorWrite::buffer(resolve_descriptor_set, RESOLVE_PARAMETERS_BINDING, parameters.into()),
			sampled(
				resolve_descriptor_set,
				RESOLVE_SHADOW_MAP_BINDING,
				shadow_maps.directional,
				shadow_sampler,
			),
			sampled(
				resolve_descriptor_set,
				RESOLVE_SHADOW_DEPTH_PYRAMID_BINDING,
				shadow_maps.directional_depth_pyramid,
				shadow_pyramid_sampler,
			),
			sampled(
				resolve_descriptor_set,
				RESOLVE_SHADOW_DEPTH_MINIMUM_PYRAMID_BINDING,
				shadow_maps.directional_depth_minimum_pyramid,
				shadow_minimum_pyramid_sampler,
			),
			sampled(
				resolve_descriptor_set,
				RESOLVE_SHADOW_DEPTH_CELLS_BINDING,
				shadow_maps.directional_depth_pyramid,
				point_sampler,
			),
		]);

		Self {
			trace_descriptor_set,
			resolve_descriptor_sets: [base_descriptor_set, resolve_descriptor_set],
			trace_pipelines: contact_shadows.then(|| Pipelines::request(pipeline_manager, CONTACT_SHADOW_PIPELINES)),
			resolve_pipelines: Pipelines::request(pipeline_manager, ["sun-visibility"]),
			parameters,
		}
	}

	/// Uploads this frame's sun and camera constants, and returns the trace and resolve recording, or `None` without a
	/// shadowed sun, because material evaluation reads the result only for shadowed suns.
	///
	/// `suns` are the shadowed suns by sun slot. `trace` is the contact-shadow trace pipeline, or `None` to resolve the
	/// shadow map alone, and `settings` sets the contact rays' reach.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		suns: &[SunShadow],
		settings: ContactShadowSettings,
		trace: Option<ghi::PipelineHandle>,
		resolve: ghi::PipelineHandle,
	) -> Option<SunVisibilityStages> {
		if suns.is_empty() {
			return None;
		}
		let extent = sink.extent();
		let screen = screen_view_data(sink, extent);
		let mut parameters = SunVisibilityShaderParameters {
			pixel_to_ray: [
				screen.pixel_to_ray_mul[0],
				screen.pixel_to_ray_mul[1],
				screen.pixel_to_ray_add[0],
				screen.pixel_to_ray_add[1],
			],
			max_distance: settings.max_distance,
			sun_count: suns.len() as u32,
			contact_shadows: u32::from(trace.is_some()),
			..Default::default()
		};
		for (entry, sun) in parameters.suns.iter_mut().zip(suns) {
			let [x, y, z] = <[f32; 3]>::from(view_space_direction_to_light(sink.view(), sun.direction));
			*entry = [x, y, z, sun.angular_radius_tangent];
		}
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = parameters;
		frame.sync_buffer(self.parameters);
		Some(SunVisibilityStages {
			trace: trace.map(|pipeline| ComputeStage {
				label: "Contact Shadow Trace",
				pipeline,
				descriptor_sets: [self.trace_descriptor_set],
				extent: extent.scaled_down(CONTACT_SHADOW_TRACE_RESOLUTION_DIVISOR),
				workgroup: Extent::new(8, 8, 1),
			}),
			resolve: ComputeStage {
				label: "Sun Visibility Resolve",
				pipeline: resolve,
				descriptor_sets: self.resolve_descriptor_sets,
				extent,
				workgroup: Extent::new(8, 8, 1),
			},
		})
	}
}

/// The `SunVisibilityStages` struct holds one frame's two sun visibility dispatches, so the visibility pass can record
/// the trace alongside the other screen-space passes' first stages and the resolve, which reads it, alongside their
/// second stages.
#[derive(Clone, Copy)]
pub(super) struct SunVisibilityStages {
	/// The half-resolution contact-shadow trace, absent when contact shadows are off; it reads only the depth pyramid.
	pub(super) trace: Option<ComputeStage<1>>,
	/// The full-resolution resolve; it reads the trace, the shadow maps, and their pyramids.
	pub(super) resolve: ComputeStage<2>,
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

		for (actual, expected) in <[f32; 3]>::from(direction).into_iter().zip([0.0, 1.0, 0.0]) {
			assert!((actual - expected).abs() < 0.0001, "{direction:?}");
		}
	}
}
