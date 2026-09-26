//! Exercises image groups: images that share device memory when their lifetimes do not overlap.

use super::common::*;
use super::*;

const SIDE: u32 = 4;

/// The `TwoMemberGroup` struct holds a group whose two color members have disjoint lifetimes, so they may share memory.
struct TwoMemberGroup {
	group: ImageGroupHandle,
	members: [ImageHandle; 2],
}

impl TwoMemberGroup {
	fn new(device: &mut impl ghi::context::Context) -> Self {
		let group = device.create_image_group(Some("Test Image Group"));
		let members = ["First Member", "Second Member"].map(|name| {
			device.build_image(
				ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Clear | Uses::TransferSource)
					.name(name)
					.group(group),
			)
		});
		Self { group, members }
	}

	/// Returns the placement request: the first member is used at position 0 and the second at position 1.
	fn placement(&self) -> [ImageGroupMember; 2] {
		let [first, second] = self.members;
		[(first, 0..=0), (second, 1..=1)].map(|(image, lifetime)| ImageGroupMember {
			image: image.into(),
			extent: Extent::square(SIDE),
			lifetime,
		})
	}
}

fn color(r: f32, g: f32, b: f32) -> RGBA {
	RGBA { r, g, b, a: 1.0 }
}

/// Clears each member in turn and reads it back before the next member can reuse its memory, over several frames so
/// memory reuse across frames in flight is covered too.
pub(super) fn members_keep_their_contents_until_another_member_reuses_them(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let group = TwoMemberGroup::new(device);
	let colors = [color(1.0, 0.0, 0.0), color(0.0, 0.0, 1.0)];
	let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);

	for frame_index in 0..4u64 {
		let mut readbacks = Vec::new();
		device.queue(queue_handle).execute(
			Some(FrameRequest::new(frame_index, synchronizer)),
			&[],
			synchronizer,
			|execution| {
				execution.frame().unwrap().place_image_group(group.group, &group.placement());
				execution.record(command_buffer_handle, |recording| {
					for (image, color) in group.members.into_iter().zip(colors) {
						recording.clear_images(&[(image.into(), ClearValue::Color(color))]);
						readbacks.push(recording.transfer_texture(image.into()).expect(
							"Texture transfer failed. The most likely cause is that the member is not a valid transfer source.",
						));
					}
				});
				[]
			},
		);
		device.wait();
		assert!(!device.has_errors());

		for (readback, color) in readbacks.into_iter().zip(colors) {
			let expected = RGBAu8 {
				r: (color.r * 255.0) as u8,
				g: (color.g * 255.0) as u8,
				b: (color.b * 255.0) as u8,
				a: 255,
			};
			let pixels = rgba_pixels(device.get_image_data(readback).expect(
				"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
			));
			assert_eq!(pixels.len(), (SIDE * SIDE) as usize);
			assert!(
				pixels.iter().all(|pixel| *pixel == expected),
				"A group member lost its contents before its lifetime ended. The most likely cause is a missing barrier between members that share memory."
			);
		}
	}
}

/// Reads the first member after the second member reused its memory, which debug builds reject.
pub(super) fn reading_a_member_after_another_reused_its_memory_fails(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let group = TwoMemberGroup::new(device);
	let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let [first, second] = group.members;

	device
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.frame().unwrap().place_image_group(group.group, &group.placement());
			execution.record(command_buffer_handle, |recording| {
				recording.clear_images(&[(first.into(), ClearValue::Color(color(1.0, 0.0, 0.0)))]);
				recording.clear_images(&[(second.into(), ClearValue::Color(color(0.0, 0.0, 1.0)))]);
				let _ = recording.transfer_texture(first.into());
			});
			[]
		});
}
