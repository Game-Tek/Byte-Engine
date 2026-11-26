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
			alias_of: None,
		})
	}
}

/// Clears each member in turn and reads it back before the next member can reuse its memory, over several frames so
/// memory reuse across frames in flight is covered too.
pub(super) fn members_keep_their_contents_until_another_member_reuses_them(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let group = TwoMemberGroup::new(device);
	let colors = [RED, BLUE];
	let frames = Frames::new(device, queue_handle);

	for frame_index in 0..4u64 {
		let readbacks = record_frame!(
			device,
			frames,
			frame_index,
			frame | frame | frame.place_image_group(group.group, &group.placement()),
			|recording| group
				.members
				.into_iter()
				.zip(colors)
				.map(|(image, color)| {
					recording.clear_images(&[(image.into(), clear_color(color))]);
					transfer(recording, image)
				})
				.collect::<Vec<_>>()
		);
		device.wait();
		assert!(!device.has_errors());

		for (readback, color) in readbacks.into_iter().zip(colors) {
			let pixels = read_pixels(device, readback);
			assert_eq!(pixels.len(), (SIDE * SIDE) as usize);
			assert!(
				pixels.iter().all(|pixel| *pixel == color),
				"A group member lost its contents before its lifetime ended. The most likely cause is a missing barrier between members that share memory."
			);
		}
	}
}

/// Places the second member as the first member's image, clears the first, and reads the second back, then gives the
/// second its own image again, on a context that takes such members.
pub(super) fn a_member_placed_as_another_members_image_reads_that_image(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	if !device.supports_image_aliases() {
		return;
	}
	let group = TwoMemberGroup::new(device);
	let frames = Frames::new(device, queue_handle);
	let [first, second] = group.members;

	// Aliased, own image, and aliased again, so the member moves both ways between frames in flight.
	for (frame_index, aliased) in [true, false, true].into_iter().enumerate() {
		let mut placement = group.placement();
		if aliased {
			// The first member must live until the second's last use, since the second is its image.
			placement[0].lifetime = 0..=1;
			placement[1].alias_of = Some(first.into());
		}
		let readback = record_frame!(
			device,
			frames,
			frame_index as u64,
			frame | frame | frame.place_image_group(group.group, &placement),
			|recording| {
				recording.clear_images(&[(first.into(), clear_color(RED))]);
				if !aliased {
					recording.clear_images(&[(second.into(), clear_color(BLUE))]);
				}
				transfer(recording, second)
			}
		);
		device.wait();
		assert!(!device.has_errors());

		let expected = if aliased { RED } else { BLUE };
		assert!(
			read_pixels(device, readback).iter().all(|pixel| *pixel == expected),
			"A member placed as another member's image did not read that image. The most likely cause is a placement that gave the member an image of its own, or kept the other member's after the alias ended."
		);
	}
}

/// Reads the first member after the second member reused its memory, which debug builds reject.
#[cfg(debug_assertions)]
pub(super) fn reading_a_member_after_another_reused_its_memory_fails(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let group = TwoMemberGroup::new(device);
	let [first, second] = group.members;

	record_frame!(
		device,
		Frames::new(device, queue_handle),
		0,
		frame | frame | frame.place_image_group(group.group, &group.placement()),
		|recording| {
			recording.clear_images(&[(first.into(), clear_color(RED))]);
			recording.clear_images(&[(second.into(), clear_color(BLUE))]);
			let _ = recording.transfer_texture(first.into());
		}
	);
}
