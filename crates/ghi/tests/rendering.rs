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
	BufferDescriptor, BufferStridedRange, DataTypes, Formats, HostAccess, Layouts, QueueHandle, ShaderTypes, Uses,
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		BoundRayTracingPipelineMode as _, CommandBufferRecording, CommonCommandBufferMode as _,
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

/// Records frame `$index` through the [`common::Frames`] `$frames` and evaluates to what the recording returns. An
/// optional `frame |frame| ...,` closure gets the open frame before recording, for resizes, mappings, and placements.
macro_rules! record_frame {
	($device:expr, $frames:expr, $index:expr, $(frame |$frame:ident| $setup:expr,)? |$recording:ident| $record:expr) => {{
		let frames: common::Frames = $frames;
		let mut recorded = None;
		$device
			.queue(frames.queue)
			.execute(Some(FrameRequest::new($index, frames.synchronizer)), &[], frames.synchronizer, |execution| {
				$(let $frame = execution.frame().unwrap(); $setup;)?
				execution.record(frames.commands, |$recording| recorded = Some($record));
				[]
			});
		recorded.unwrap()
	}};
}

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
#[cfg(target_os = "linux")]
#[path = "rendering/ray_tracing_spikes.rs"]
mod ray_tracing_spikes;
#[path = "rendering/resources.rs"]
mod resources;
#[path = "rendering/upscaler.rs"]
mod upscaler;

/// Returns the features every scenario's device has: validation on.
fn validated() -> ghi::device::Features {
	ghi::device::Features::new().validation(true)
}

/// Runs one rendering scenario that also uses the device, on a fresh device setup with `features`.
///
/// The setup locals drop in reverse order, so the context goes before the device and the instance.
fn run_on(features: ghi::device::Features, scenario: impl FnOnce(&BackendDevice, &mut BackendContext, QueueHandle)) {
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
	let mut context = ghi::device::Device::create_context(&device)
		.expect("Failed to create the GHI test context. The most likely cause is unavailable backend command support.");
	scenario(&device, &mut context, queue_handle.unwrap());
}

/// Runs one rendering scenario on a fresh device setup with `features`.
fn run_with(features: ghi::device::Features, scenario: impl FnOnce(&mut BackendContext, QueueHandle)) {
	run_on(features, |_, context, queue_handle| scenario(context, queue_handle));
}

/// Runs one ray tracing scenario on a fresh device setup with `features` and ray tracing, which the GPU must support.
fn run_traced(features: ghi::device::Features, scenario: impl FnOnce(&mut BackendContext, QueueHandle)) {
	run_with(features.ray_tracing(true), |context, queue_handle| {
		assert!(
			context.supports_ray_tracing(),
			"The test device cannot trace rays. The most likely cause is a GPU without ray tracing support. Run the ray tracing tests on a GPU that supports it."
		);
		scenario(context, queue_handle);
	});
}

/// Runs one rendering scenario on a fresh validated device setup.
fn run(scenario: impl FnOnce(&mut BackendContext, QueueHandle)) {
	run_with(validated(), scenario);
}

/// Runs the unordered dispatch scenario on a first device, then again on a second device that loaded the first
/// one's pipeline cache, and checks that the second run still computes the right pixels with nothing new to save,
/// because nothing was compiled.
///
/// On a backend without an application-owned cache there is never anything to save, and this only checks that
/// asking for one leaves pipelines working.
fn pipeline_from_a_loaded_pipeline_cache_works(shader_key: Option<u64>) {
	let features = validated().pipeline_cache(true);
	let mut compiled = Vec::new();
	run_with(features, |context, queue_handle| {
		compute::unordered_dispatches_write_their_own_rows(context, queue_handle, shader_key);
		compiled = ghi::context::Context::pipeline_cache_data(context, |data| [data.loaded, data.added].concat());
	});
	run_with(features, |context, queue_handle| {
		ghi::context::Context::load_pipeline_cache(context, compiled.clone());
		compute::unordered_dispatches_write_their_own_rows(context, queue_handle, shader_key);
		ghi::context::Context::pipeline_cache_data(context, |data| {
			assert_eq!(data.loaded, compiled);
			assert!(data.added.is_empty());
		});
	});
}

/// Declares one test per scenario, each a call of the runner that sets up its device.
macro_rules! scenarios {
	($($(#[$meta:meta])* $name:ident => $run:expr;)*) => {
		$(
			$(#[$meta])*
			#[test]
			fn $name() {
				$run;
			}
		)*
	};
}

scenarios! {
	disabled_counters_report_no_duration => run(counters::disabled_counters_report_no_duration);
	counters_measure_every_completed_frame =>
		run_with(validated().counters(true), counters::counters_measure_every_completed_frame);
	statistics_counters_count_compute_invocations =>
		run_with(validated().counters(true), counters::statistics_counters_count_compute_invocations);
	counters_ended_after_the_last_dispatch_resolve =>
		run_with(validated().counters(true), counters::counters_ended_after_the_last_dispatch_resolve);

	unordered_dispatches_write_their_own_rows =>
		run(|context, queue_handle| compute::unordered_dispatches_write_their_own_rows(context, queue_handle, None));
	bound_buffer_struct_members_read_at_their_offsets => run(compute::bound_buffer_struct_members_read_at_their_offsets);
	/// A pipeline whose shader the backend keeps the code of comes from a loaded pipeline cache.
	pipelines_created_from_a_loaded_pipeline_cache_work => pipeline_from_a_loaded_pipeline_cache_works(None);
	/// A pipeline whose shader is named by a key and lends its code comes from a loaded pipeline cache, which also
	/// remembers the shader, so neither the shader nor the pipeline adds anything to save.
	keyed_shader_pipelines_created_from_a_loaded_pipeline_cache_work =>
		pipeline_from_a_loaded_pipeline_cache_works(Some(0x5EED));

	image_group_members_keep_their_contents_until_reused =>
		run(image_groups::members_keep_their_contents_until_another_member_reuses_them);
	image_group_member_placed_as_another_reads_its_image =>
		run(image_groups::a_member_placed_as_another_members_image_reads_that_image);
	#[cfg(debug_assertions)]
	#[should_panic(expected = "was used without valid contents")]
	image_group_member_read_after_reuse_fails_validation =>
		run(image_groups::reading_a_member_after_another_reused_its_memory_fails);

	render_triangle_pixels =>
		run(|context, queue_handle| raster::render_triangle(context, queue_handle, raster::TriangleDraw::Mesh));
	render_triangle_with_indirect_draw =>
		run(|context, queue_handle| raster::render_triangle(context, queue_handle, raster::TriangleDraw::Indirect));
	#[cfg(target_os = "macos")]
	raster_pipeline_can_disable_depth_writes => run(raster::render_without_depth_writes);
	render_pass_without_attachments_writes_through_image_atomics =>
		run(raster::render_without_attachments_through_image_atomics);

	resize_render_target => run(|context, queue_handle| resources::resize(context, queue_handle, ghi::Copies::PerFrame));
	resize_static_render_target => run(|context, queue_handle| resources::resize(context, queue_handle, ghi::Copies::One));
	resize_render_target_layers => run(resources::resize_layers);
	resize_static_render_target_in_flight => run(resources::resize_render_target_in_flight);
	array_buffer_round_trip => run(resources::array_buffer_round_trip);
	resize_dynamic_buffer => run(resources::resize_dynamic_buffer);
	update_dynamic_data => run(resources::dynamic_data);
	dynamic_buffer_created_in_a_frame_has_a_copy_per_frame =>
		run(resources::dynamic_buffer_created_in_a_frame_has_a_copy_per_frame);
	transfer_previous_frame_textures => run(resources::previous_frame_transfers);
	render_with_multiframe_resources => run(resources::multiframe_resources);
	round_trip_texture3d_lut => run(resources::texture3d_lut_round_trip);
	update_texture_regions => run(resources::texture_region_uploads);
	exported_image_keeps_contents_and_mips => run_on(validated(), resources::exported_image_keeps_contents_and_mips);
	imported_buffer_receives_copies => run_on(validated(), resources::imported_buffer_receives_copies);
	imported_private_buffer_keeps_copies_made_before_its_owner_records =>
		run_on(validated(), resources::imported_private_buffer_keeps_copies_made_before_its_owner_records);
	imported_private_buffer_keeps_copies_made_while_its_zero_fill_runs =>
		run_on(validated(), resources::imported_private_buffer_keeps_copies_made_while_its_zero_fill_runs);
	transferred_mapping_feeds_every_later_copy => run(resources::transferred_mapping_feeds_every_later_copy);

	traced_structures_stay_resident_without_rebuilds =>
		run_traced(validated(), ray_tracing::traced_structures_stay_resident_without_rebuilds);
	counters_time_bottom_level_builds =>
		run_traced(validated().counters(true), ray_tracing::counters_time_bottom_level_builds);
	top_level_structures_have_a_copy_per_frame =>
		run_traced(validated(), ray_tracing::top_level_structures_have_a_copy_per_frame);
	instances_report_custom_indices_beyond_16_bits =>
		run_traced(validated(), ray_tracing::instances_report_custom_indices_beyond_16_bits);
	bottom_level_structures_hold_several_geometries =>
		run_traced(validated(), ray_tracing::bottom_level_structures_hold_several_geometries);
	compacted_structures_trace_like_the_original =>
		run_traced(validated(), ray_tracing::compacted_structures_trace_like_the_original);
	destroyed_structures_stay_valid_for_frames_in_flight =>
		run_traced(validated(), ray_tracing::destroyed_structures_stay_valid_for_frames_in_flight);
	stale_structure_handles_are_rejected =>
		run_traced(validated(), ray_tracing::stale_structure_handles_are_rejected);
	invoke_runs_the_hit_group_of_the_record_the_intersector_reports => run_traced(
		validated(),
		ray_tracing::invoke_runs_the_hit_group_of_the_record_the_intersector_reports
	);
	closest_hit_reads_sets_the_ray_generation_shader_does_not_use => run_traced(
		validated(),
		ray_tracing::closest_hit_reads_sets_the_ray_generation_shader_does_not_use
	);
	trace_rays_launches_exactly_the_requested_grid =>
		run_traced(validated(), ray_tracing::trace_rays_launches_exactly_the_requested_grid);
	rebuilt_pipeline_with_more_hit_groups_invokes_the_new_group =>
		run_traced(validated(), ray_tracing::rebuilt_pipeline_with_more_hit_groups_invokes_the_new_group);
	// The measurement spikes run without validation, which would add its own time to every pipeline creation.
	#[cfg(target_os = "linux")]
	#[ignore = "Measurement spike; run it explicitly."]
	ray_tracing_spikes_building_a_pipeline_again_compiles_only_the_new_hit_group => run_traced(
		ghi::device::Features::new(),
		ray_tracing_spikes::building_a_pipeline_again_compiles_only_the_new_hit_group
	);
	#[cfg(target_os = "linux")]
	#[ignore = "Measurement spike; run it explicitly."]
	ray_tracing_spikes_sorted_closest_hit_calls_against_random_ones => run_traced(
		ghi::device::Features::new(),
		ray_tracing_spikes::sorted_closest_hit_calls_against_random_ones
	);

	a_still_frame_upscales_to_its_own_color => run(upscaler::a_still_frame_upscales_to_its_own_color);
}
