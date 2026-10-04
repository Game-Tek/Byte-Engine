//! Exercises GPU timing counters through frames on the active native backend.

use std::time::Duration;

use super::*;

/// Clears a large image inside a counter for a few frames, reads the GPU time once a frame completes, and checks
/// that frames which skip the counter report no sample.
pub(super) fn counters_measure_completed_frames(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear | Uses::TransferSource)
			.name("Counter Target")
			.extent(Extent::square(2048)),
	);
	let color = RGBA {
		r: 0.0,
		g: 0.5,
		b: 1.0,
		a: 1.0,
	};
	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let counter = device.create_counter(Some("Clears"));
	assert_eq!(device.counter_duration(counter), None);

	// One more frame than can be in flight, so the first frame has completed when the loop ends.
	let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
	for frame_index in 0..frames {
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					recording.counter(counter, |recording| {
						for _ in 0..8 {
							recording.clear_images(&[(image.into(), ClearValue::Color(color))]);
						}
					});
				});
				[]
			},
		);
		assert!(!device.has_errors());
	}
	let duration = device
		.counter_duration(counter)
		.expect("A completed frame that recorded the counter must report its duration.");
	assert!(
		duration > Duration::ZERO && duration < Duration::from_secs(1),
		"Counter duration is implausible: {duration:?}"
	);

	for frame_index in frames..2 * frames {
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					recording.clear_images(&[(image.into(), ClearValue::Color(color))]);
				});
				[]
			},
		);
		assert!(!device.has_errors());
	}
	assert_eq!(device.counter_duration(counter), None);
	device.wait();
}

/// Records one counter on every frame for several turns of every frame sequence, and checks that each completed
/// frame reports a duration, so a sequence's slots keep working after they were resolved once.
pub(super) fn counters_measure_every_completed_frame(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear | Uses::TransferSource)
			.name("Counter Target")
			.extent(Extent::square(256)),
	);
	let color = RGBA {
		r: 1.0,
		g: 0.0,
		b: 0.0,
		a: 1.0,
	};
	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let counter = device.create_counter(Some("Clears"));

	let frames = 4 * MAX_FRAMES_IN_FLIGHT as u64;
	let mut missing = Vec::new();
	for frame_index in 0..frames {
		let mut completed_frame = None;
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				completed_frame = execution.completed_frame();
				execution.record(command_buffer, |recording| {
					recording.counter(counter, |recording| {
						recording.clear_images(&[(image.into(), ClearValue::Color(color))]);
					});
				});
				[]
			},
		);
		assert!(!device.has_errors());
		if let Some(completed_frame) = completed_frame
			&& device.counter_duration(counter).is_none()
		{
			missing.push(completed_frame.frame_index());
		}
	}
	assert!(
		missing.is_empty(),
		"Completed frames {missing:?} report no counter duration although every frame recorded the counter."
	);
	device.wait();
}

/// Nests one counter inside another so the outer end is the last timestamp of the frame, right after the inner end,
/// and checks that both resolve. This is how a renderer times a whole frame around its last pass.
pub(super) fn nested_counters_resolve_when_the_outer_end_closes_the_frame(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear | Uses::TransferSource)
			.name("Counter Target")
			.extent(Extent::square(256)),
	);
	let color = RGBA {
		r: 0.0,
		g: 1.0,
		b: 0.0,
		a: 1.0,
	};
	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let frame_counter = device.create_counter(Some("Frame"));
	let pass_counter = device.create_counter(Some("Pass"));

	let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
	for frame_index in 0..frames {
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					recording.counter(frame_counter, |recording| {
						recording.counter(pass_counter, |recording| {
							recording.clear_images(&[(image.into(), ClearValue::Color(color))]);
						});
					});
				});
				[]
			},
		);
		assert!(!device.has_errors());
	}
	let pass = device.counter_duration(pass_counter);
	let frame = device.counter_duration(frame_counter);
	assert!(pass.is_some(), "The inner counter must resolve.");
	assert!(
		frame.is_some(),
		"The outer counter must resolve although its end is the frame's last timestamp."
	);
	assert!(
		frame >= pass,
		"The outer counter {frame:?} must cover the inner one {pass:?}."
	);
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
	let shader = ghi::shader::compile(
		"GHI counter test compute shader",
		ShaderSource::PlatformNative {
			glsl: "
				#version 450
				#pragma shader_stage(compute)
				layout(set=0,binding=0, rgba8) uniform image2D img;
				layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
				void main() {
					imageStore(img, ivec2(gl_GlobalInvocationID.xy), vec4(1.0, 0.0, 0.0, 1.0));
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
				RWTexture2D<float4> image : register(u0, space0);
				[numthreads(1, 1, 1)]
				void compute_main(uint3 gid : SV_DispatchThreadID) {
					image[gid.xy] = float4(1.0, 0.0, 0.0, 1.0);
				}
			"#,
			hlsl_entry_point: "compute_main",
		},
	)
	.expect("Failed to compile the counter test compute shader. The most likely cause is invalid native shader source.");
	let image_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(0),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
	let compute_shader = device
		.create_shader(None, shader.as_source(), ShaderTypes::Compute, [image_resource])
		.expect("Failed to create the counter test compute shader");
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
			.name("Counter Dispatch Target")
			.extent(Extent::square(64)),
	);
	let descriptor_set = device.create_descriptor_set(None);
	device.write(&[ghi::DescriptorWrite::image(
		descriptor_set,
		image_resource.slot(),
		image,
		Layouts::General,
	)]);

	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let frame_counter = device.create_counter(Some("Frame"));
	let pass_counter = device.create_counter(Some("Dispatch"));

	let frames = MAX_FRAMES_IN_FLIGHT as u64 + 1;
	for frame_index in 0..frames {
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.record(command_buffer, |recording| {
					recording.counter(frame_counter, |recording| {
						recording.counter(pass_counter, |recording| {
							recording
								.bind_compute_pipeline(pipeline)
								.bind_descriptor_sets(&[descriptor_set])
								.dispatch(DispatchExtent::new(Extent::square(64), Extent::square(1)));
						});
					});
				});
				[]
			},
		);
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
