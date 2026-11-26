//! Exercises GPU timing counters through frames on the active native backend.

use std::time::Duration;

use super::common::{Frames, GREEN, clear_color, shader};
use super::*;

/// One more frame than can be in flight, so the first frame has completed after the last one is submitted.
const FRAMES: u64 = MAX_FRAMES_IN_FLIGHT as u64 + 1;

/// Builds a square image of `side` pixels for counters to time clears of.
fn clear_target(device: &mut impl ghi::context::Context, side: u32) -> ImageHandle {
	device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear | Uses::TransferSource)
			.name("Counter Target")
			.extent(Extent::square(side)),
	)
}

/// Records a counter around GPU work on a device created without counters, and checks that completed frames still
/// report no duration, so a release build pays for no timestamps.
pub(super) fn disabled_counters_report_no_duration(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let image = clear_target(device, 256);
	let frames = Frames::new(device, queue_handle);
	let counter = device.create_counter(Some("Clears"), ghi::CounterKind::Timing);

	for frame_index in 0..FRAMES {
		record_frame!(device, frames, frame_index, |recording| recording.counter(counter, |recording| {
			recording.clear_images(&[(image.into(), ClearValue::Color(RGBA::black()))]);
		}));
		assert!(!device.has_errors());
	}
	assert_eq!(device.counter_duration(counter), None);
	device.wait();
}

/// Nests a pass counter inside a frame counter on every frame for several turns of every frame sequence, so the outer
/// end is the last timestamp of the frame, right after the inner end, as a renderer times a whole frame around its
/// last pass.
///
/// Checks that each completed frame reports both durations, so a sequence's slots keep working after they were
/// resolved once, and that frames which then skip the counters report no sample.
pub(super) fn counters_measure_every_completed_frame(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let image = clear_target(device, 2048);
	let frames = Frames::new(device, queue_handle);
	let frame_counter = device.create_counter(Some("Frame"), ghi::CounterKind::Timing);
	let pass_counter = device.create_counter(Some("Pass"), ghi::CounterKind::Timing);
	assert_eq!(device.counter_duration(pass_counter), None);

	let measured = 4 * MAX_FRAMES_IN_FLIGHT as u64;
	for frame_index in 0..measured + FRAMES {
		let mut completed_frame = None;
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, frames.synchronizer)),
			&[],
			frames.synchronizer,
			|execution| {
				completed_frame = execution.completed_frame();
				execution.record(frames.commands, |recording| {
					let clears = |recording: &mut _| {
						for _ in 0..8 {
							CommandBufferRecording::clear_images(recording, &[(image.into(), clear_color(GREEN))]);
						}
					};
					if frame_index < measured {
						recording.counter(frame_counter, |recording| recording.counter(pass_counter, clears));
					} else {
						clears(recording);
					}
				});
				[]
			},
		);
		assert!(!device.has_errors());
		let Some(completed_frame) = completed_frame.map(|frame| frame.frame_index()) else {
			continue;
		};
		let (pass, frame) = (device.counter_duration(pass_counter), device.counter_duration(frame_counter));
		if completed_frame < measured {
			let pass = pass.unwrap_or_else(|| panic!("Completed frame {completed_frame} reports no pass duration."));
			let frame = frame.unwrap_or_else(|| {
				panic!("Completed frame {completed_frame} reports no frame duration although its end is the frame's last timestamp.")
			});
			assert!(
				pass > Duration::ZERO && pass < Duration::from_secs(1) && frame >= pass,
				"Counter durations are implausible: pass {pass:?}, frame {frame:?}"
			);
		} else {
			assert_eq!((pass, frame), (None, None), "Completed frame {completed_frame} recorded no counter.");
		}
	}
	device.wait();
}

/// Ends a pass counter and then a frame counter right after a compute dispatch, as the last commands of the frame,
/// and checks that both resolve.
///
/// Two timestamps written back to back through an open compute encoder with nothing recorded after them is the
/// shape of every renderer's last pass end followed by its frame end.
pub(super) fn counters_ended_after_the_last_dispatch_resolve(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let (pipeline, descriptor_set) = image_fill_pipeline(device);
	let frames = Frames::new(device, queue_handle);
	let frame_counter = device.create_counter(Some("Frame"), ghi::CounterKind::Timing);
	let pass_counter = device.create_counter(Some("Dispatch"), ghi::CounterKind::Timing);

	for frame_index in 0..FRAMES {
		record_frame!(device, frames, frame_index, |recording| recording.counter(frame_counter, |recording| {
			recording.counter(pass_counter, |recording| {
				recording
					.bind_compute_pipeline(pipeline)
					.bind_descriptor_sets(&[descriptor_set])
					.dispatch(DispatchExtent::new(Extent::square(64), Extent::square(1)));
			});
		}));
		assert!(!device.has_errors());
	}
	let pass = device
		.counter_duration(pass_counter)
		.expect("A counter that ends after the frame's last dispatch must still report its duration.");
	let frame = device
		.counter_duration(frame_counter)
		.expect("A counter that ends right after another one, as the frame's last timestamp, must still report its duration.");
	assert!(
		pass > Duration::ZERO && frame >= pass,
		"Counter durations are implausible: pass {pass:?}, frame {frame:?}"
	);
	device.wait();
}

/// Builds a compute pipeline that writes every texel of a 64-pixel square image, one invocation per texel, and the
/// set that binds the image, for the tests that measure dispatches.
fn image_fill_pipeline(device: &mut impl ghi::context::Context) -> (ghi::PipelineHandle, ghi::DescriptorSetHandle) {
	let image_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(0),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
	let layout = ghi::DescriptorSetLayout::new("Counter Target", [image_resource]);
	let compute_shader = shader(
		device,
		"Counter Image Fill",
		ShaderTypes::Compute,
		ShaderSource::PlatformNative {
			glsl: "
				#version 450
				#pragma shader_stage(compute)
				#extension GL_EXT_descriptor_heap : require
				#extension GL_EXT_nonuniform_qualifier : require
				// The pipeline index of the shader's set 0, and each set's heap base indices.
				layout(constant_id = 1984) const uint SET0 = 0u;
				layout(push_constant) uniform Sets { uvec2 sets[15]; } push_constants;
				layout(descriptor_heap, descriptor_stride = 64, rgba8) uniform image2D images[];
				layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
				void main() {
					imageStore(images[push_constants.sets[SET0].x], ivec2(gl_GlobalInvocationID.xy), vec4(1.0, 0.0, 0.0, 1.0));
				}
			",
			msl: r#"
				#include <metal_stdlib>
				using namespace metal;
				struct Resources {
					texture2d<float, access::write> image [[id(0)]];
				};
				kernel void compute_main(uint2 gid [[thread_position_in_grid]], constant Resources& resources [[buffer(16)]]) {
					resources.image.write(float4(1.0, 0.0, 0.0, 1.0), gid);
				}
			"#,
			msl_entry_point: "compute_main",
			hlsl: r#"
				cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
				static RWTexture2D<float4> image = ResourceDescriptorHeap[_besl_sets[0].x];
				[numthreads(1, 1, 1)]
				void compute_main(uint3 gid : SV_DispatchThreadID) {
					image[gid.xy] = float4(1.0, 0.0, 0.0, 1.0);
				}
			"#,
			hlsl_entry_point: "compute_main",
		},
		[ghi::ShaderDescriptorSet::new(layout.clone())],
	);
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
			.name("Counter Dispatch Target")
			.extent(Extent::square(64)),
	);
	let descriptor_set = device.create_descriptor_set(None, &layout);
	device.write(&[ghi::DescriptorWrite::image(
		descriptor_set,
		image_resource.slot(),
		image,
		Layouts::General,
	)]);

	(pipeline, descriptor_set)
}

/// Opens a statistics counter around one 64 by 64 dispatch, and checks that completed frames report every compute
/// invocation where the device has pipeline statistics, and nothing elsewhere.
pub(super) fn statistics_counters_count_compute_invocations(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let (pipeline, descriptor_set) = image_fill_pipeline(device);
	let frames = Frames::new(device, queue_handle);
	let counter = device.create_counter(Some("Dispatch Statistics"), ghi::CounterKind::PipelineStatistics);

	for frame_index in 0..FRAMES {
		record_frame!(device, frames, frame_index, |recording| recording.counter(counter, |recording| {
			recording
				.bind_compute_pipeline(pipeline)
				.bind_descriptor_sets(&[descriptor_set])
				.dispatch(DispatchExtent::new(Extent::square(64), Extent::square(1)));
		}));
		assert!(!device.has_errors());
	}
	let statistics = device.counter_statistics(counter);
	if device.supports_pipeline_statistics() {
		let statistics = statistics.expect("A completed frame that recorded the statistics counter must report it.");
		assert_eq!(statistics.compute_invocations, 64 * 64);
		assert_eq!(statistics.fragment_invocations, 0);
	} else {
		assert_eq!(statistics, None);
	}
	// A statistics counter writes no timestamps.
	assert_eq!(device.counter_duration(counter), None);
	device.wait();
}
