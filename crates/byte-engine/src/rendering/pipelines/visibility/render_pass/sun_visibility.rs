//! Sun visibility: each shadowed sun's filtered shadow-map visibility times its screen-space contact shadow, resolved
//! once per opaque pixel into one full-resolution image, with sun slot `s` in channel `s`.
//!
//! Opaque material evaluation multiplies each sun by its channel of one fetch of [`SUN_VISIBILITY_TARGET`]. Resolving the shadow map
//! in a pass of its own keeps its many fetches out of the material shader, where they hide their latency poorly, and
//! lets the GPU overlap them with the other screen-space passes. Transparent surfaces lie in front of the opaque depth
//! the pass resolves, so the material shader still resolves the map for them itself.
//!
//! The contact shadow fills the gaps the shadow map cannot resolve where objects touch, such as under a foot on a
//! floor: each pixel marches a short ray toward the sun through the depth buffer and records how much visible geometry
//! blocks it. The resolve smooths that dithered trace over each pixel's 3x3 neighbors on its surface and multiplies it
//! by the shadow map's visibility. The trace runs per pixel, not per block of pixels, so the changes camera jitter
//! makes in detail finer than a pixel stay a pixel wide, where temporal antialiasing averages them.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::pod::Vec3f;
use maths_rs::Vec4f;
use utils::Extent;

use super::super::layout::MAX_DIRECTIONAL_SHADOW_COUNT;
use super::super::shadow_selection::SunShadow;
use super::depth_pyramid::screen_view_data;
use super::shadows::ShadowMaps;
use super::{ComputeStage, Pipelines};
use crate::configuration::{Accepts, Read, Setting};
use crate::rendering::lookup_textures::blue_noise_slice;
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::{PipelineManagerClient, Sink, View};

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
			max_distance: 0.1,
		}
	}
}

impl ContactShadowSettings {
	/// The runtime settings, named inside the `render.contact-shadows.` port.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[
		Setting {
			name: "enabled",
			accepts: Accepts::Bool,
			value: |settings| Read::Bool(settings.enabled),
			apply: |settings, read| {
				let enabled = read.bool();
				Ok(Self { enabled, ..settings })
			},
		},
		Setting {
			name: "distance",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.max_distance)),
			apply: |settings, read| {
				let distance = read.number();
				Ok(Self {
					max_distance: distance as f32,
					..settings
				})
			},
		},
	];
}

/// The render-graph name of the full-resolution result: per sun slot, one where that sun reaches the pixel, falling
/// toward zero where its shadow map or visible geometry blocks it. Material evaluation reads it for every shadowed sun.
pub(crate) const SUN_VISIBILITY_TARGET: &str = "Sun Visibility";
/// The render-graph name of the unfiltered contact-shadow trace, which the resolve reads. Capture it to debug the trace
/// alone.
pub(crate) const CONTACT_SHADOW_TRACE_TARGET: &str = "Contact Shadow Trace";

/// The trace pipeline, named as [`Pipelines::request`] takes it. Only contact shadows use it.
pub(in super::super) const CONTACT_SHADOW_PIPELINES: [&str; 1] = ["contact-shadows"];

/// The `SunVisibilityTargets` struct holds the image the contact-shadow trace writes and the image the resolve writes,
/// so the visibility pass can hand them to [`SunVisibilityPass::new`] and bind the result in material evaluation.
#[derive(Clone, Copy)]
pub(crate) struct SunVisibilityTargets {
	/// The unfiltered contact-shadow trace, named [`CONTACT_SHADOW_TRACE_TARGET`].
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
	let mut target = |name| {
		render_pass_builder
			.create_render_target(
				// One channel per sun slot, so every shadowed sun resolves in one dispatch that shares its tile loads.
				ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Storage | ghi::Uses::Image).name(name),
			)
			.into()
	};
	SunVisibilityTargets {
		trace: target(CONTACT_SHADOW_TRACE_TARGET),
		visibility: target(SUN_VISIBILITY_TARGET),
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
	/// Nonzero when the sun samples its virtual shadow map instead of cascades.
	pub(crate) virtual_shadows: u32,
	/// Turn reversed device depth `d` into view depth as `numerator / (d + offset)`.
	pub(crate) depth_unproject_numerator: f32,
	pub(crate) depth_unproject_denominator_offset: f32,
	/// Offsets the trace's march noise; see [`blue_noise_slice`].
	pub(crate) noise_slice: u32,
	pub(crate) padding: u32,
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
	/// The trace's set, which holds the depth it marches.
	trace_descriptor_sets: [ghi::DescriptorSetHandle; 1],
	/// The scene set, whose views hold the camera and the sun's cascades or levels, the resolve's own set, and the
	/// virtual shadow map's sampling set.
	resolve_descriptor_sets: [ghi::DescriptorSetHandle; 3],
	virtual_shadows: bool,
	/// The contact-shadow trace pipeline, absent when contact shadows are left out.
	pub(super) trace_pipelines: Option<Pipelines<1>>,
	pub(super) resolve_pipelines: Pipelines<1>,
	parameters: ghi::BufferHandle<SunVisibilityShaderParameters>,
}

impl SunVisibilityPass {
	/// Wires the depth images, the shadow maps, and the targets, and requests the trace and resolve pipelines.
	///
	/// `base_descriptor_set` is the sink's base visibility set, whose views the resolve reads the cascades from. The trace
	/// marches the opaque layer's `depth`, which the resolve also reads. `primitive_index` and `instance_id` are the visibility buffer, from which the resolve takes each receiver's
	/// triangle. `shadow_maps` holds the sun's cascades and their depth pyramid. Without `contact_shadows`, the trace
	/// pipeline is not requested.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		shadow_maps: &ShadowMaps,
		targets: SunVisibilityTargets,
		contact_shadows: bool,
	) -> Self {
		let trace_descriptor_set = set_layouts::CONTACT_SHADOWS.create_set(context, "Contact Shadow Trace Descriptor Set");
		let resolve_descriptor_set = set_layouts::SUN_VISIBILITY.create_set(context, "Sun Visibility Descriptor Set");
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Sun Visibility Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
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
		let trace = set_layouts::CONTACT_SHADOWS.binder(trace_descriptor_set);
		let resolve = set_layouts::SUN_VISIBILITY.binder(resolve_descriptor_set);
		context.write(&[
			trace.buffer("contact_shadow_parameters", parameters),
			trace.image("contact_shadow_output", targets.trace),
			trace.sampled("contact_shadow_depth", depth, point_sampler),
			resolve.sampled("visibility_depth", depth, point_sampler),
			resolve.image("triangle_index", primitive_index),
			resolve.image("instance_index_render_target", instance_id),
			resolve.image("sun_visibility_output", targets.visibility),
			resolve.sampled("contact_shadow_trace", targets.trace, point_sampler),
			resolve.buffer("sun_visibility_parameters", parameters),
			resolve.sampled("depth_shadow_map", shadow_maps.directional, shadow_sampler),
			// The layer fixes the pyramids' reducing samplers.
			resolve.texture("directional_shadow_depth_pyramid", shadow_maps.directional_depth_pyramid),
			resolve.texture(
				"directional_shadow_depth_minimum_pyramid",
				shadow_maps.directional_depth_minimum_pyramid,
			),
			// The maximum pyramid once more, with the point sampler, for the blocker search's gathers.
			resolve.sampled(
				"directional_shadow_depth_cells",
				shadow_maps.directional_depth_pyramid,
				point_sampler,
			),
		]);

		Self {
			trace_descriptor_sets: [trace_descriptor_set],
			resolve_descriptor_sets: [
				base_descriptor_set,
				resolve_descriptor_set,
				shadow_maps.virtual_maps.sampling_descriptor_set,
			],
			virtual_shadows: shadow_maps.virtual_maps.enabled(),
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
			virtual_shadows: u32::from(self.virtual_shadows),
			depth_unproject_numerator: screen.depth_unproject_numerator,
			depth_unproject_denominator_offset: screen.depth_unproject_denominator_offset,
			noise_slice: blue_noise_slice(sink, frame.key().frame_index()),
			..Default::default()
		};
		for (entry, sun) in parameters.suns.iter_mut().zip(suns) {
			let [x, y, z] = <[f32; 3]>::from(view_space_direction_to_light(sink.view(), sun.direction));
			*entry = [x, y, z, sun.angular_radius_tangent];
		}
		*frame.map(self.parameters) = parameters;
		Some(SunVisibilityStages {
			trace: trace.map(|pipeline| ComputeStage {
				label: "Contact Shadow Trace",
				pipeline,
				descriptor_sets: self.trace_descriptor_sets,
				extent,
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
	/// The contact-shadow trace, absent when contact shadows are off; it reads only the depth.
	pub(super) trace: Option<ComputeStage<1>>,
	/// The full-resolution resolve; it reads the trace, the shadow maps, and their pyramids.
	pub(super) resolve: ComputeStage<3>,
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
