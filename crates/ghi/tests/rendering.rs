//! Exercises rendering through the active native GHI backend.

// Native rendering scenarios keep complete setup, submission, and readback workflows together.
#![allow(
	clippy::cognitive_complexity,
	clippy::excessive_nesting,
	clippy::multiple_unsafe_ops_per_block,
	clippy::too_many_lines,
	clippy::undocumented_unsafe_blocks
)]

use ghi::implementation::{Context as BackendContext, Device as BackendDevice, Instance};
use ghi::{
	BufferDescriptor, BufferStridedRange, DataTypes, DeviceAccesses, Formats, Layouts, QueueHandle, ShaderTypes, UseCases,
	Uses,
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		BoundRayTracingPipelineMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
		RasterizationRenderPassMode as _,
	},
	frame::Frame as _,
	pipelines::{self, PushConstantRange, ShaderParameter, VertexElement, raster::AttachmentDescriptor},
	queue::{FrameRequest, Queue as _, QueueExecution as _},
	rt,
	shader::{CompiledShaderSource, ShaderSource},
	*,
};
use utils::{Extent, RGBA};

#[path = "rendering/common.rs"]
mod common;
#[path = "rendering/compute.rs"]
mod compute;
#[path = "rendering/counters.rs"]
mod counters;
#[path = "rendering/image_groups.rs"]
mod image_groups;
#[path = "rendering/raster.rs"]
mod raster;
#[path = "rendering/ray_tracing.rs"]
mod ray_tracing;
#[path = "rendering/resources.rs"]
mod resources;

/// Creates the native device and raster queue used by one rendering scenario.
///
/// Bind the tuple in this order so reverse local drop order releases the context,
/// then the device, and finally the instance.
fn create_default_device_setup() -> (Instance, BackendDevice, BackendContext, QueueHandle) {
	let features = ghi::device::Features::new().validation(true);
	create_default_device_setup_with_features(features)
}

/// Creates the native device with the capabilities required by one rendering scenario.
fn create_default_device_setup_with_features(
	features: ghi::device::Features,
) -> (Instance, BackendDevice, BackendContext, QueueHandle) {
	let mut instance = Instance::new(features).expect(
		"Failed to create the GHI test instance. The most likely cause is that the active backend has no available device.",
	);
	let mut queue_handle = None;
	let device = instance
		.create_device(
			features,
			&mut [(ghi::QueueSelection::new(ghi::types::WorkloadTypes::RASTER), &mut queue_handle)],
		)
		.expect("Failed to create the GHI test device. The most likely cause is unavailable raster queue support.");
	let context = ghi::device::Device::create_context(&device)
		.expect("Failed to create the GHI test context. The most likely cause is unavailable backend command support.");
	(instance, device, context, queue_handle.unwrap())
}

/// Runs one rendering scenario on a fresh validated device setup.
///
/// The setup locals drop in reverse order, so the context goes before the device and the instance.
fn run(scenario: impl FnOnce(&mut BackendContext, QueueHandle)) {
	let (_instance, _device, mut context, queue_handle) = create_default_device_setup();
	scenario(&mut context, queue_handle);
}

/// Runs one ray tracing scenario on a fresh validated device setup that enables ray tracing.
fn run_with_ray_tracing(scenario: impl FnOnce(&mut BackendContext, QueueHandle)) {
	let (_instance, _device, mut context, queue_handle) =
		create_default_device_setup_with_features(ghi::device::Features::new().validation(true).ray_tracing(true));
	assert!(
		context.supports_ray_tracing(),
		"The test device cannot trace rays. The most likely cause is a GPU without ray tracing support. Run the ray tracing tests on a GPU that supports it."
	);
	scenario(&mut context, queue_handle);
}

#[test]
fn counters_measure_completed_frames() {
	let (_instance, _device, mut device, queue_handle) = create_default_device_setup();
	counters::counters_measure_completed_frames(&mut device, queue_handle);
}

#[test]
fn counters_measure_every_completed_frame() {
	let (_instance, _device, mut device, queue_handle) = create_default_device_setup();
	counters::counters_measure_every_completed_frame(&mut device, queue_handle);
}

#[test]
fn nested_counters_resolve_when_the_outer_end_closes_the_frame() {
	let (_instance, _device, mut device, queue_handle) = create_default_device_setup();
	counters::nested_counters_resolve_when_the_outer_end_closes_the_frame(&mut device, queue_handle);
}

#[test]
fn counters_ended_after_the_last_dispatch_resolve() {
	let (_instance, _device, mut device, queue_handle) = create_default_device_setup();
	counters::counters_ended_after_the_last_dispatch_resolve(&mut device, queue_handle);
}

#[test]
fn unordered_dispatches_write_their_own_rows() {
	run(|context, queue_handle| compute::unordered_dispatches_write_their_own_rows(context, queue_handle, None));
}

/// Runs the unordered dispatch scenario on a first device, then again on a second device that loaded the first
/// one's pipeline cache, and checks that the second run still computes the right pixels with nothing new to save,
/// because nothing was compiled.
///
/// On a backend without an application-owned cache there is never anything to save, and this only checks that
/// asking for one leaves pipelines working.
fn pipeline_from_a_loaded_pipeline_cache_works(shader_key: Option<u64>) {
	let features = ghi::device::Features::new().validation(true).pipeline_cache(true);

	let compiled = {
		let (_instance, _device, mut context, queue_handle) = create_default_device_setup_with_features(features);
		compute::unordered_dispatches_write_their_own_rows(&mut context, queue_handle, shader_key);
		ghi::context::Context::pipeline_cache_data(&context, |data| [data.loaded, data.added].concat())
	};

	let (_instance, _device, mut context, queue_handle) = create_default_device_setup_with_features(features);
	ghi::context::Context::load_pipeline_cache(&context, compiled.clone());
	compute::unordered_dispatches_write_their_own_rows(&mut context, queue_handle, shader_key);
	ghi::context::Context::pipeline_cache_data(&context, |data| {
		assert_eq!(data.loaded, compiled);
		assert!(data.added.is_empty());
	});
}

/// A pipeline whose shader the backend keeps the code of comes from a loaded pipeline cache.
#[test]
fn pipelines_created_from_a_loaded_pipeline_cache_work() {
	pipeline_from_a_loaded_pipeline_cache_works(None);
}

/// A pipeline whose shader is named by a key and lends its code comes from a loaded pipeline cache, which also
/// remembers the shader, so neither the shader nor the pipeline adds anything to save.
#[test]
fn keyed_shader_pipelines_created_from_a_loaded_pipeline_cache_work() {
	pipeline_from_a_loaded_pipeline_cache_works(Some(0x5EED));
}

#[test]
fn image_group_members_keep_their_contents_until_reused() {
	run(image_groups::members_keep_their_contents_until_another_member_reuses_them);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "was used without valid contents")]
fn image_group_member_read_after_reuse_fails_validation() {
	run(image_groups::reading_a_member_after_another_reused_its_memory_fails);
}

#[test]
fn render_triangle_pixels() {
	run(|context, queue_handle| raster::render_triangle(context, queue_handle, raster::TriangleDraw::Mesh));
}

#[test]
fn render_triangle_with_indirect_draw() {
	run(|context, queue_handle| raster::render_triangle(context, queue_handle, raster::TriangleDraw::Indirect));
}

#[cfg(target_os = "macos")]
#[test]
fn raster_pipeline_can_disable_depth_writes() {
	run(raster::render_without_depth_writes);
}

#[test]
fn change_frames_in_flight() {
	run(resources::change_frames);
}

#[test]
fn resize_render_target() {
	run(|context, queue_handle| resources::resize(context, queue_handle, UseCases::DYNAMIC));
}

#[test]
fn resize_static_render_target() {
	run(|context, queue_handle| resources::resize(context, queue_handle, UseCases::STATIC));
}

#[test]
fn resize_render_target_layers() {
	run(resources::resize_layers);
}

#[test]
fn resize_static_render_target_in_flight() {
	run(|context, queue_handle| resources::resize_render_target_in_flight(context, queue_handle, UseCases::STATIC));
}

#[test]
fn array_buffer_round_trip() {
	run(resources::array_buffer_round_trip);
}

#[test]
fn resize_dynamic_buffer() {
	run(resources::resize_dynamic_buffer);
}

#[test]
fn update_dynamic_data() {
	run(resources::dynamic_data);
}

#[test]
fn dynamic_buffer_created_in_a_frame_has_a_copy_per_frame() {
	run(resources::dynamic_buffer_created_in_a_frame_has_a_copy_per_frame);
}

#[test]
fn update_dynamic_textures() {
	run(resources::dynamic_textures);
}

#[test]
fn transfer_previous_frame_textures() {
	run(resources::previous_frame_transfers);
}

#[test]
fn render_with_multiframe_resources() {
	run(resources::multiframe_resources);
}

#[cfg(target_os = "windows")]
#[test]
fn round_trip_texture3d_lut() {
	run(resources::texture3d_lut_round_trip);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn traced_structures_stay_resident_without_rebuilds() {
	run_with_ray_tracing(ray_tracing::traced_structures_stay_resident_without_rebuilds);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn counters_time_bottom_level_builds() {
	run_with_ray_tracing(ray_tracing::counters_time_bottom_level_builds);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn top_level_structures_have_a_copy_per_frame() {
	run_with_ray_tracing(ray_tracing::top_level_structures_have_a_copy_per_frame);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn instances_report_custom_indices_beyond_16_bits() {
	run_with_ray_tracing(ray_tracing::instances_report_custom_indices_beyond_16_bits);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn bottom_level_structures_hold_several_geometries() {
	run_with_ray_tracing(ray_tracing::bottom_level_structures_hold_several_geometries);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn compacted_structures_trace_like_the_original() {
	run_with_ray_tracing(ray_tracing::compacted_structures_trace_like_the_original);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn destroyed_structures_stay_valid_for_frames_in_flight() {
	run_with_ray_tracing(ray_tracing::destroyed_structures_stay_valid_for_frames_in_flight);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn stale_structure_handles_are_rejected() {
	run_with_ray_tracing(ray_tracing::stale_structure_handles_are_rejected);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn invoke_runs_the_hit_group_of_the_record_the_intersector_reports() {
	run_with_ray_tracing(ray_tracing::invoke_runs_the_hit_group_of_the_record_the_intersector_reports);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn closest_hit_reads_sets_the_ray_generation_shader_does_not_use() {
	run_with_ray_tracing(ray_tracing::closest_hit_reads_sets_the_ray_generation_shader_does_not_use);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn trace_rays_launches_exactly_the_requested_grid() {
	run_with_ray_tracing(ray_tracing::trace_rays_launches_exactly_the_requested_grid);
}

#[test]
#[cfg_attr(not(target_os = "macos"), ignore = "only the Metal backend traces rays")]
fn rebuilt_pipeline_with_more_hit_groups_invokes_the_new_group() {
	run_with_ray_tracing(ray_tracing::rebuilt_pipeline_with_more_hit_groups_invokes_the_new_group);
}

#[test]
fn exported_image_keeps_contents_and_mips() {
	let (_instance, device, mut context, queue_handle) = create_default_device_setup();
	resources::exported_image_keeps_contents_and_mips(&device, &mut context, queue_handle);
}

#[test]
fn imported_buffer_receives_copies() {
	let (_instance, device, mut context, queue_handle) = create_default_device_setup();
	resources::imported_buffer_receives_copies(&device, &mut context, queue_handle);
}

#[test]
fn polled_synchronizer_reports_completion() {
	run(resources::polled_synchronizer_reports_completion);
}

#[test]
fn update_texture_regions() {
	run(resources::texture_region_uploads);
}
