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
