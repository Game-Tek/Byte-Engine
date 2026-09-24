//! Screen-space diffuse global illumination traced against the linear depth pyramid and last frame's diffuse radiance.
//!
//! Each half-resolution pixel shoots one cosine-weighted ray per frame. A ray that hits on-screen geometry takes its
//! light from the previous frame's [`DIFFUSE_RADIANCE_HISTORY_TARGET`], so bounces accumulate over frames. An edge-aware
//! spatial filter and an exponential moving average over reprojected history remove the noise, and an edge-aware
//! upscale writes the full-resolution result that material evaluation composites. Both compare surface normals
//! rebuilt from depth, because surfaces that touch, such as a foot on a floor, share the same depth at the contact.
//!
//! The result stores hit radiance in RGB and the fraction of rays that hit in alpha. Material evaluation lights the
//! missed fraction with the environment, so rays that leave the screen fall back to image-based lighting.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use math::{Matrix, ShaderMatrix};
use utils::Extent;

use super::depth_pyramid::{DEPTH_PYRAMID_MIP_COUNT, ScreenViewData, half_resolution_extent};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The render-graph name of the diffuse light leaving opaque surfaces, which rays sample one frame later.
///
/// It holds direct diffuse, indirect diffuse, and emitted light in RGB, but no specular. A surface receives the light
/// a neighbor sends toward it, not the view-dependent highlight the camera sees there. Alpha holds each pixel's view
/// depth, and zero where no opaque surface was drawn.
pub(crate) const DIFFUSE_RADIANCE_HISTORY_TARGET: &str = "Diffuse Radiance History";
/// The render-graph name of the half-resolution trace output: one ray's radiance and hit per pixel.
pub(crate) const SSGI_RAW_TARGET: &str = "SSGI Raw";
/// The render-graph name of the half-resolution view-space normals the trace rebuilds from depth.
///
/// RGB holds the normal, or zero where the depth neighborhood was degenerate or empty. The next frame's denoiser reads
/// it to tell whether its history belongs to the same surface.
pub(crate) const SSGI_NORMALS_TARGET: &str = "SSGI Normals";
/// The render-graph name of the half-resolution accumulated result that the next frame blends with.
pub(crate) const SSGI_HISTORY_TARGET: &str = "SSGI History";
/// The render-graph name of the full-resolution result that material evaluation composites.
pub(crate) const SSGI_INDIRECT_DIFFUSE_TARGET: &str = "SSGI Indirect Diffuse";
/// SSGI traces and accumulates at half the sink resolution.
const SSGI_RESOLUTION_DIVISOR: u32 = 2;

/// The `SsgiTargets` struct names the render-graph images SSGI reads and writes for one sink.
///
/// They are render targets so the renderer sizes them with the sink and they can be captured by name for debugging.
/// Create them with [`create_ssgi_targets`].
#[derive(Clone, Copy)]
pub(crate) struct SsgiTargets {
	pub(crate) raw: ghi::BaseImageHandle,
	pub(crate) normals: ghi::DynamicImageHandle,
	pub(crate) history: ghi::DynamicImageHandle,
	pub(crate) indirect_diffuse: ghi::BaseImageHandle,
	pub(crate) diffuse_radiance_history: ghi::DynamicImageHandle,
}

/// Creates the SSGI render-graph targets for the sink that `render_pass_builder` sets up.
///
/// Next, pass them to the visibility render pass, which hands them to [`SsgiPass::new`].
pub(crate) fn create_ssgi_targets(render_pass_builder: &mut crate::rendering::render_pass::RenderPassBuilder<'_>) -> SsgiTargets {
	let radiance_image = |name| {
		ghi::image::Builder::new(RADIANCE_FORMAT, ghi::Uses::Storage | ghi::Uses::Image)
			.name(name)
			.device_accesses(ghi::DeviceAccesses::DeviceOnly)
	};
	SsgiTargets {
		raw: render_pass_builder
			.create_scaled_render_target(radiance_image(SSGI_RAW_TARGET), SSGI_RESOLUTION_DIVISOR)
			.into(),
		normals: render_pass_builder.create_history_target(radiance_image(SSGI_NORMALS_TARGET), SSGI_RESOLUTION_DIVISOR),
		history: render_pass_builder.create_history_target(radiance_image(SSGI_HISTORY_TARGET), SSGI_RESOLUTION_DIVISOR),
		indirect_diffuse: render_pass_builder
			.create_render_target(radiance_image(SSGI_INDIRECT_DIFFUSE_TARGET))
			.into(),
		// Material evaluation clears and writes this image, so it also needs transfer-destination use.
		diffuse_radiance_history: render_pass_builder.create_history_target(
			radiance_image(DIFFUSE_RADIANCE_HISTORY_TARGET).additional_uses(ghi::Uses::TransferDestination),
			1,
		),
	}
}
/// The format of every SSGI image: HDR radiance in RGB and the ray hit fraction in alpha, or a normal in RGB.
const RADIANCE_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;

const fn buffer(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
	)
}
const fn sampled(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::CombinedImageSampler,
		ghi::AccessPolicies::READ,
	)
}
const fn storage(slot: u32) -> ghi::ShaderResourceDescriptor {
	ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(slot),
		ghi::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	)
}
const VIEW_BINDING: ghi::ShaderResourceDescriptor = buffer(0);
const PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = buffer(1);
// Every stage reads linear depth at 1033 and writes its output at 1034 or 1035. See each BESL asset for the rest.
const DEPTH_BINDING: ghi::ShaderResourceDescriptor = sampled(1033);
const TRACE_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = storage(1034);
const TRACE_PREVIOUS_RADIANCE_BINDING: ghi::ShaderResourceDescriptor = sampled(1035);
const TRACE_NORMALS_BINDING: ghi::ShaderResourceDescriptor = storage(1036);
const TEMPORAL_RAW_BINDING: ghi::ShaderResourceDescriptor = sampled(1034);
const TEMPORAL_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = storage(1035);
const TEMPORAL_PREVIOUS_HISTORY_BINDING: ghi::ShaderResourceDescriptor = sampled(1036);
const TEMPORAL_PREVIOUS_DEPTH_BINDING: ghi::ShaderResourceDescriptor = sampled(1037);
const TEMPORAL_NORMALS_BINDING: ghi::ShaderResourceDescriptor = sampled(1038);
const TEMPORAL_PREVIOUS_NORMALS_BINDING: ghi::ShaderResourceDescriptor = sampled(1039);
const UPSCALE_SOURCE_BINDING: ghi::ShaderResourceDescriptor = sampled(1034);
const UPSCALE_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = storage(1035);
const UPSCALE_LOW_RESOLUTION_DEPTH_BINDING: ghi::ShaderResourceDescriptor = sampled(1036);
const UPSCALE_NORMALS_BINDING: ghi::ShaderResourceDescriptor = sampled(1037);

/// The `SsgiShaderParameters` struct carries the per-frame values every SSGI stage needs to use history.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SsgiShaderParameters {
	/// Maps a current-frame view-space position to the previous frame's clip space.
	current_view_to_previous_clip: ShaderMatrix,
	/// Animates the interleaved gradient noise so each frame traces different directions.
	frame_index: u32,
	/// Nonzero when the previous frame's radiance, SSGI history, and depth pyramid hold this sink's data.
	history_valid: u32,
	_padding: [u32; 2],
}

/// Returns the matrix that maps a current-frame view-space position to the previous frame's clip space.
///
/// SSGI uses it to find where a surface visible now was on screen last frame.
pub(crate) fn current_view_to_previous_clip(current: View, previous: View) -> Matrix {
	previous.view_projection() * math::inverse(current.view())
}

/// The `SsgiPass` struct adds bounce light from visible geometry to the diffuse ambient term of opaque surfaces.
///
/// It runs after [`super::depth_pyramid::DepthPyramidPass`] and before opaque material evaluation, which reads
/// the full-resolution result. Opaque material evaluation writes [`DIFFUSE_RADIANCE_HISTORY_TARGET`] for the next
/// frame's rays.
pub(super) struct SsgiPass {
	trace_descriptor_set: ghi::DescriptorSetHandle,
	temporal_descriptor_set: ghi::DescriptorSetHandle,
	upscale_descriptor_set: ghi::DescriptorSetHandle,
	trace_pipeline: crate::rendering::PipelineRef,
	temporal_pipeline: crate::rendering::PipelineRef,
	upscale_pipeline: crate::rendering::PipelineRef,
	parameters: ghi::DynamicBufferHandle<SsgiShaderParameters>,
}

pub(super) struct SsgiPipelines {
	trace: ghi::PipelineHandle,
	temporal: ghi::PipelineHandle,
	upscale: ghi::PipelineHandle,
}

impl SsgiPass {
	/// Creates the radiance images and wires the trace, temporal, and upscale descriptor sets.
	///
	/// `depth_pyramid` and `view_data` come from [`super::depth_pyramid::DepthPyramidPass`].
	/// Create `targets` with [`create_ssgi_targets`]. Next, bind [`SsgiTargets::indirect_diffuse`] in material evaluation.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
		depth_pyramid: ghi::DynamicImageHandle,
		view_data: ghi::DynamicBufferHandle<ScreenViewData>,
		targets: SsgiTargets,
	) -> Self {
		let trace_descriptor_set = context.create_descriptor_set(Some("SSGI Trace Descriptor Set"));
		let temporal_descriptor_set = context.create_descriptor_set(Some("SSGI Temporal Descriptor Set"));
		let upscale_descriptor_set = context.create_descriptor_set(Some("SSGI Upscale Descriptor Set"));
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("SSGI Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod((DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let SsgiTargets {
			raw,
			normals,
			history,
			indirect_diffuse,
			diffuse_radiance_history,
		} = targets;
		let sampled = |set, binding: ghi::ShaderResourceDescriptor, image: ghi::BaseImageHandle, sampler| {
			ghi::DescriptorWrite::combined_image_sampler(set, binding.slot(), image, sampler, ghi::Layouts::Read)
		};
		let previous = |set, binding: ghi::ShaderResourceDescriptor, image: ghi::DynamicImageHandle, sampler| {
			ghi::DescriptorWrite::combined_image_sampler_with_frame(set, binding.slot(), image, sampler, ghi::Layouts::Read, -1)
		};
		let storage = |set, binding: ghi::ShaderResourceDescriptor, image: ghi::BaseImageHandle| {
			ghi::DescriptorWrite::image(set, binding.slot(), image, ghi::Layouts::General)
		};
		context.write(&[
			ghi::DescriptorWrite::buffer(trace_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(trace_descriptor_set, PARAMETERS_BINDING.slot(), parameters.into()),
			sampled(trace_descriptor_set, DEPTH_BINDING, depth_pyramid.into(), point_sampler),
			storage(trace_descriptor_set, TRACE_OUTPUT_BINDING, raw),
			storage(trace_descriptor_set, TRACE_NORMALS_BINDING, normals.into()),
			// Point sampling keeps a hit's light from blending with the background next to the hit object.
			previous(
				trace_descriptor_set,
				TRACE_PREVIOUS_RADIANCE_BINDING,
				diffuse_radiance_history,
				point_sampler,
			),
			ghi::DescriptorWrite::buffer(temporal_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(temporal_descriptor_set, PARAMETERS_BINDING.slot(), parameters.into()),
			sampled(temporal_descriptor_set, DEPTH_BINDING, depth_pyramid.into(), point_sampler),
			sampled(temporal_descriptor_set, TEMPORAL_RAW_BINDING, raw, point_sampler),
			storage(temporal_descriptor_set, TEMPORAL_OUTPUT_BINDING, history.into()),
			previous(temporal_descriptor_set, TEMPORAL_PREVIOUS_HISTORY_BINDING, history, point_sampler),
			previous(temporal_descriptor_set, TEMPORAL_PREVIOUS_DEPTH_BINDING, depth_pyramid, point_sampler),
			sampled(temporal_descriptor_set, TEMPORAL_NORMALS_BINDING, normals.into(), point_sampler),
			previous(temporal_descriptor_set, TEMPORAL_PREVIOUS_NORMALS_BINDING, normals, point_sampler),
			ghi::DescriptorWrite::buffer(upscale_descriptor_set, VIEW_BINDING.slot(), view_data.into()),
			sampled(upscale_descriptor_set, DEPTH_BINDING, depth, point_sampler),
			sampled(upscale_descriptor_set, UPSCALE_SOURCE_BINDING, history.into(), point_sampler),
			storage(upscale_descriptor_set, UPSCALE_OUTPUT_BINDING, indirect_diffuse),
			sampled(
				upscale_descriptor_set,
				UPSCALE_LOW_RESOLUTION_DEPTH_BINDING,
				depth_pyramid.into(),
				point_sampler,
			),
			sampled(upscale_descriptor_set, UPSCALE_NORMALS_BINDING, normals.into(), point_sampler),
		]);
		let request = |name| pipeline_manager.request_pipeline(name);

		Self {
			trace_descriptor_set,
			temporal_descriptor_set,
			upscale_descriptor_set,
			trace_pipeline: request("byte-engine/rendering/visibility/ssgi-trace.pipeline"),
			temporal_pipeline: request("byte-engine/rendering/visibility/ssgi-temporal.pipeline"),
			upscale_pipeline: request("byte-engine/rendering/visibility/ssgi-upscale.pipeline"),
			parameters,
		}
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<SsgiPipelines> {
		Some(SsgiPipelines {
			trace: pipeline_manager.pipeline(self.trace_pipeline)?,
			temporal: pipeline_manager.pipeline(self.temporal_pipeline)?,
			upscale: pipeline_manager.pipeline(self.upscale_pipeline)?,
		})
	}

	/// Uploads this frame's reprojection and noise seed, resizes the images, and returns the three-stage recording.
	///
	/// `previous_view` is the view this pass recorded the sink with in the previous frame, or `None` when the previous
	/// frame's SSGI and radiance images do not hold this sink's data. Without it the stages ignore history.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		previous_view: Option<View>,
		pipelines: SsgiPipelines,
	) -> impl RenderPassFunction + use<> {
		let extent = sink.extent();
		let half_extent = half_resolution_extent(extent);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = SsgiShaderParameters {
			current_view_to_previous_clip: previous_view
				.map(|previous| current_view_to_previous_clip(sink.view(), previous))
				.unwrap_or_default()
				.into(),
			// Only the low bits animate the noise, so wrapping the frame index is harmless.
			frame_index: frame.key().frame_index() as u32,
			history_valid: previous_view.is_some() as u32,
			_padding: [0; 2],
		};
		frame.sync_buffer(self.parameters);

		let stages = [
			("SSGI Trace", pipelines.trace, self.trace_descriptor_set, half_extent),
			(
				"SSGI Denoise and Accumulate",
				pipelines.temporal,
				self.temporal_descriptor_set,
				half_extent,
			),
			(
				"SSGI Depth-Aware Upscale",
				pipelines.upscale,
				self.upscale_descriptor_set,
				extent,
			),
		];
		move |c, _| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _,
			};

			c.start_region(|label| label.write_str("SSGI"));
			for (name, pipeline, descriptor_set, extent) in stages {
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
	use maths_rs::Vec4f;

	use super::*;

	fn view_at(position: Point) -> View {
		View::new_perspective(Degrees::new(60.0), 16.0 / 9.0, 0.1, 100.0, position, UnitVector::z_axis())
	}

	#[test]
	fn reprojection_maps_a_view_space_point_to_where_the_previous_camera_saw_it() {
		let world_point = Vec4f::new(0.5, -0.25, 6.0, 1.0);
		let previous = view_at(Point::new(-1.0, 0.0, 0.0));
		let current = view_at(Point::new(1.0, 0.5, 2.0));
		let current_view_point = current.view() * world_point;

		let reprojected = current_view_to_previous_clip(current, previous) * current_view_point;
		let expected = previous.view_projection() * world_point;

		for (reprojected, expected) in [
			(reprojected.x, expected.x),
			(reprojected.y, expected.y),
			(reprojected.z, expected.z),
			(reprojected.w, expected.w),
		] {
			assert!((reprojected - expected).abs() < 0.0001, "{reprojected} != {expected}");
		}
		// Positive clip w is the previous camera's view depth, which the temporal stage compares against.
		assert!((reprojected.w - (6.0 - 0.0)).abs() < 0.0001);
	}
}
