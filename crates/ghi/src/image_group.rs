//! Backend-independent state for image groups: images that share device memory when their lifetimes do not overlap.
//!
//! Create a group with [`crate::context::ContextCreate::create_image_group`], add members with
//! [`crate::image::Builder::group`], and give them memory with [`crate::frame::Frame::place_image_group`].
//!
//! This module decides where each member lives and which members hold valid contents. Backends own the native
//! heaps and images, ask it for member offsets, and ask it which members a newly initialized member overwrites, so
//! they can order its first write after every earlier access to that memory.

use std::ops::RangeInclusive;

use smallvec::SmallVec;
use utils::{Extent, hash::HashMap};

use crate::{BaseImageHandle, DeviceAccesses, ImageGroupHandle, UseCases};

/// The `ImageGroupMember` struct gives one group member its size and the span of the frame that uses it.
///
/// Pass one for every member of a group to [`crate::frame::Frame::place_image_group`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageGroupMember {
	pub image: BaseImageHandle,
	pub extent: Extent,
	/// The positions in the frame where the image is used, first and last included.
	///
	/// The GHI only compares these ranges with each other. Members whose ranges overlap never share memory.
	pub lifetime: RangeInclusive<u32>,
}

/// The `MemoryRequirements` struct holds what a backend reports about one member before placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemoryRequirements {
	pub(crate) size: u64,
	pub(crate) alignment: u64,
	/// A backend-defined class of memory. Only members of the same category can share a heap.
	pub(crate) category: u32,
}

/// The `Slot` struct locates one member inside the heaps of a [`Placement`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
	pub(crate) heap: usize,
	pub(crate) offset: u64,
	pub(crate) size: u64,
}

/// The `HeapLayout` struct describes one heap a backend must create for a [`Placement`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeapLayout {
	pub(crate) category: u32,
	pub(crate) size: u64,
	/// The largest alignment among the heap's members, which the heap's base address must satisfy.
	pub(crate) alignment: u64,
}

/// The `Placement` struct is the result of [`pack`]: the heaps to create and each member's slot in them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Placement {
	pub(crate) heaps: Vec<HeapLayout>,
	/// One slot per member, in member order.
	pub(crate) slots: Vec<Slot>,
}

impl Placement {
	/// Reports whether two members occupy some of the same bytes.
	pub(crate) fn overlaps(&self, left: usize, right: usize) -> bool {
		let (Some(left), Some(right)) = (self.slots.get(left), self.slots.get(right)) else {
			return false;
		};
		left.heap == right.heap && ranges_intersect(left.offset, left.size, right.offset, right.size)
	}
}

/// Assigns each member a heap and offset so that members with overlapping lifetimes never overlap in memory.
///
/// Larger members are placed first, since they constrain the layout most, and each member takes the lowest aligned
/// offset that is free for its whole lifetime. Members of different categories go into different heaps.
pub(crate) fn pack(members: &[(MemoryRequirements, RangeInclusive<u32>)]) -> Placement {
	let mut order = (0..members.len()).collect::<SmallVec<[usize; 32]>>();
	order.sort_by(|&left, &right| members[right].0.size.cmp(&members[left].0.size).then(left.cmp(&right)));

	let mut heaps = Vec::<HeapLayout>::new();
	let mut slots = vec![None::<Slot>; members.len()];

	for (position, &member) in order.iter().enumerate() {
		let (requirements, lifetime) = &members[member];
		let alignment = requirements.alignment.max(1);
		let heap = heaps
			.iter()
			.position(|heap| heap.category == requirements.category)
			.unwrap_or_else(|| {
				heaps.push(HeapLayout {
					category: requirements.category,
					size: 0,
					alignment: 1,
				});
				heaps.len() - 1
			});

		// The byte ranges this member must avoid: members of its heap that are alive at the same time.
		let conflicts = order[..position]
			.iter()
			.filter(|&&other| lifetimes_intersect(lifetime, &members[other].1))
			.filter_map(|&other| slots[other])
			.filter(|slot| slot.heap == heap)
			.map(|slot| (slot.offset, slot.size))
			.collect::<SmallVec<[(u64, u64); 16]>>();

		// The lowest free offset is either the heap start or right after a conflicting member.
		let offset = std::iter::once(0)
			.chain(conflicts.iter().map(|(offset, size)| offset + size))
			.map(|offset| offset.next_multiple_of(alignment))
			.filter(|&offset| {
				conflicts
					.iter()
					.all(|&(other, other_size)| !ranges_intersect(offset, requirements.size, other, other_size))
			})
			.min()
			.expect("The offset after the last conflicting member is always free.");

		slots[member] = Some(Slot {
			heap,
			offset,
			size: requirements.size,
		});
		heaps[heap].size = heaps[heap].size.max(offset + requirements.size);
		heaps[heap].alignment = heaps[heap].alignment.max(alignment);
	}

	Placement {
		heaps,
		slots: slots.into_iter().map(|slot| slot.expect("Every member is placed.")).collect(),
	}
}

fn lifetimes_intersect(left: &RangeInclusive<u32>, right: &RangeInclusive<u32>) -> bool {
	left.start() <= right.end() && right.start() <= left.end()
}

fn ranges_intersect(left: u64, left_size: u64, right: u64, right_size: u64) -> bool {
	left < right + right_size && right < left + left_size
}

/// The `ImageGroup` struct holds one group's members, their current placement, and which members hold valid contents.
pub(crate) struct ImageGroup {
	pub(crate) name: Option<String>,
	/// Members in the order they were built.
	pub(crate) members: Vec<BaseImageHandle>,
	/// The requests the current placement was made from, in member order. Empty until the group is first placed.
	placed: Vec<ImageGroupMember>,
	pub(crate) placement: Placement,
	/// Whether each member was initialized since the last placement and since another member last reused its memory.
	live: Vec<bool>,
}

/// The `ImageGroups` struct stores every image group of a context, so each backend shares one implementation of
/// membership, placement bookkeeping, and contents validation.
#[derive(Default)]
pub(crate) struct ImageGroups {
	groups: Vec<ImageGroup>,
	/// Maps each member image to its group and its index inside that group.
	membership: HashMap<BaseImageHandle, (usize, usize)>,
}

impl ImageGroups {
	pub(crate) fn create(&mut self, name: Option<&str>) -> ImageGroupHandle {
		self.groups.push(ImageGroup {
			name: name.map(str::to_owned),
			members: Vec::new(),
			placed: Vec::new(),
			placement: Placement::default(),
			live: Vec::new(),
		});
		ImageGroupHandle(self.groups.len() as u64 - 1)
	}

	/// Checks that an image builder describes an image that can share memory.
	///
	/// Backends call this before creating an image whose builder names a group.
	pub(crate) fn validate_member(builder: &crate::image::Builder) {
		let name = builder.name.unwrap_or("unnamed");
		assert!(
			builder.use_case == UseCases::STATIC,
			"Image '{name}' cannot join an image group. The most likely cause is a per-frame (dynamic) image, which keeps one copy per frame in flight and cannot share memory."
		);
		assert!(
			builder.device_accesses == DeviceAccesses::DeviceOnly,
			"Image '{name}' cannot join an image group. The most likely cause is an image the CPU can access, which needs host-visible memory that group heaps do not provide."
		);
	}

	/// Rejects a per-frame image that names a group. Backends call this from `build_dynamic_image`.
	pub(crate) fn reject_dynamic_member(builder: &crate::image::Builder) {
		assert!(
			builder.group.is_none(),
			"Image '{}' cannot join an image group. The most likely cause is a per-frame (dynamic) image, which keeps one copy per frame in flight and cannot share memory.",
			builder.name.unwrap_or("unnamed"),
		);
	}

	pub(crate) fn add_member(&mut self, group: ImageGroupHandle, image: BaseImageHandle) {
		let group_index = group.0 as usize;
		let members = &mut self.group_mut(group).members;
		members.push(image);
		let member = members.len() - 1;
		self.membership.insert(image, (group_index, member));
	}

	pub(crate) fn group(&self, group: ImageGroupHandle) -> &ImageGroup {
		self.groups
			.get(group.0 as usize)
			.expect("Image group does not exist. The most likely cause is a group handle created by a different context.")
	}

	fn group_mut(&mut self, group: ImageGroupHandle) -> &mut ImageGroup {
		self.groups
			.get_mut(group.0 as usize)
			.expect("Image group does not exist. The most likely cause is a group handle created by a different context.")
	}

	/// Rejects resizing a member on its own. Backends call this from `resize_image`.
	pub(crate) fn assert_resizable(&self, image: BaseImageHandle) {
		if let Some((group, _)) = self.member(image) {
			panic!(
				"Image in group '{}' cannot be resized on its own. The most likely cause is a `resize_image` call on an image-group member; pass its new extent to `place_image_group` instead.",
				self.group(group).name.as_deref().unwrap_or("unnamed"),
			);
		}
	}

	/// Returns the group and member index of `image`, or `None` when it belongs to no group.
	pub(crate) fn member(&self, image: BaseImageHandle) -> Option<(ImageGroupHandle, usize)> {
		self.membership
			.get(&image)
			.map(|&(group, member)| (ImageGroupHandle(group as u64), member))
	}

	/// Orders `requests` like the group's members, or returns `None` when the group is already placed from them.
	///
	/// Every member must appear exactly once, and nothing else may appear.
	pub(crate) fn requests_in_member_order(
		&self,
		group: ImageGroupHandle,
		requests: &[ImageGroupMember],
	) -> Option<Vec<ImageGroupMember>> {
		let image_group = self.group(group);
		let group_name = image_group.name.as_deref().unwrap_or("unnamed");
		assert_eq!(
			requests.len(),
			image_group.members.len(),
			"Image group '{group_name}' was placed with {} members but has {}. The most likely cause is a member missing from, or repeated in, the list passed to `place_image_group`.",
			requests.len(),
			image_group.members.len(),
		);

		let ordered = image_group
			.members
			.iter()
			.map(|&member| {
				requests
					.iter()
					.find(|request| request.image == member)
					.cloned()
					.unwrap_or_else(|| {
						panic!(
							"Image group '{group_name}' was placed without one of its members. The most likely cause is an image that was built into the group but left out of the list passed to `place_image_group`."
						)
					})
			})
			.collect::<Vec<_>>();

		(ordered != image_group.placed).then_some(ordered)
	}

	/// Records a new placement. Every member starts without valid contents.
	pub(crate) fn commit(&mut self, group: ImageGroupHandle, requests: Vec<ImageGroupMember>, placement: Placement) {
		let image_group = self.group_mut(group);
		image_group.live = vec![false; requests.len()];
		image_group.placed = requests;
		image_group.placement = placement;
	}

	/// Marks `image` as holding valid contents and returns the other members whose memory it reuses.
	///
	/// Backends call this when a command initializes a member, and order that command after every earlier access
	/// to the returned members. Returns `None` when `image` belongs to no group.
	pub(crate) fn initialize(&mut self, image: BaseImageHandle) -> Option<SmallVec<[BaseImageHandle; 8]>> {
		let (group, member) = self.member(image)?;
		let image_group = self.group_mut(group);
		let overwritten = (0..image_group.members.len())
			.filter(|&other| other != member && image_group.placement.overlaps(member, other))
			.collect::<SmallVec<[usize; 8]>>();

		if let Some(live) = image_group.live.get_mut(member) {
			*live = true;
		}
		for &other in &overwritten {
			image_group.live[other] = false;
		}

		Some(overwritten.into_iter().map(|other| image_group.members[other]).collect())
	}

	/// Checks that a group member holds valid contents before a command uses it. Images outside groups always pass.
	///
	/// Only debug builds check. A failure means the caller used the member outside the lifetime it gave
	/// [`crate::frame::Frame::place_image_group`], or did not initialize it first.
	pub(crate) fn assert_initialized(&self, image: BaseImageHandle, image_name: impl FnOnce() -> Option<String>) {
		if !cfg!(debug_assertions) {
			return;
		}
		let Some((group, member)) = self.member(image) else {
			return;
		};
		let image_group = self.group(group);
		assert!(
			image_group.live.get(member).copied().unwrap_or(false),
			"Image '{}' in group '{}' was used without valid contents. The most likely cause is a use outside the lifetime given to `place_image_group`, or a first write that did not clear, discard, or call `discard_images` on it.",
			image_name().as_deref().unwrap_or("unnamed"),
			image_group.name.as_deref().unwrap_or("unnamed"),
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn member(size: u64, alignment: u64, lifetime: RangeInclusive<u32>) -> (MemoryRequirements, RangeInclusive<u32>) {
		(
			MemoryRequirements {
				size,
				alignment,
				category: 0,
			},
			lifetime,
		)
	}

	#[test]
	fn members_with_disjoint_lifetimes_share_memory() {
		let placement = pack(&[member(256, 256, 0..=1), member(256, 256, 2..=3)]);

		assert_eq!(placement.heaps.len(), 1);
		assert_eq!(placement.heaps[0].size, 256);
		assert!(placement.overlaps(0, 1));
	}

	#[test]
	fn members_with_overlapping_lifetimes_do_not_share_memory() {
		let placement = pack(&[member(256, 256, 0..=2), member(256, 256, 2..=3)]);

		assert_eq!(placement.heaps[0].size, 512);
		assert!(!placement.overlaps(0, 1));
	}

	#[test]
	fn offsets_respect_each_member_alignment() {
		let placement = pack(&[member(300, 256, 0..=1), member(100, 1024, 0..=1)]);

		assert_eq!(placement.slots[0].offset % 256, 0);
		assert_eq!(placement.slots[1].offset % 1024, 0);
		assert!(!placement.overlaps(0, 1));
		assert_eq!(placement.heaps[0].alignment, 1024);
	}

	#[test]
	fn a_small_member_fills_a_gap_left_by_larger_ones() {
		// `a` and `b` are alive together and `c` only overlaps `a`'s lifetime, so `c` reuses `b`'s bytes.
		let placement = pack(&[member(512, 1, 0..=3), member(256, 1, 0..=1), member(256, 1, 2..=3)]);

		assert_eq!(placement.heaps[0].size, 768);
		assert!(placement.overlaps(1, 2));
	}

	#[test]
	fn categories_are_placed_in_separate_heaps() {
		let mut other_category = member(256, 1, 2..=3);
		other_category.0.category = 1;
		let placement = pack(&[member(256, 1, 0..=1), other_category]);

		assert_eq!(placement.heaps.len(), 2);
		assert!(!placement.overlaps(0, 1));
	}

	fn groups_with_two_sharing_members() -> (ImageGroups, ImageGroupHandle, [BaseImageHandle; 2]) {
		let mut groups = ImageGroups::default();
		let group = groups.create(Some("Test"));
		let images = [BaseImageHandle(1), BaseImageHandle(2)];
		for image in images {
			groups.add_member(group, image);
		}
		let requests = images
			.into_iter()
			.zip([0..=1, 2..=3])
			.map(|(image, lifetime)| ImageGroupMember {
				image,
				extent: Extent::square(4),
				lifetime,
			})
			.collect::<Vec<_>>();
		let ordered = groups.requests_in_member_order(group, &requests).unwrap();
		groups.commit(group, ordered, pack(&[member(64, 1, 0..=1), member(64, 1, 2..=3)]));
		(groups, group, images)
	}

	#[test]
	fn initializing_a_member_reports_the_members_it_overwrites() {
		let (mut groups, _, [first, second]) = groups_with_two_sharing_members();

		assert_eq!(groups.initialize(first).unwrap().as_slice(), [second]);
		groups.assert_initialized(first, || None);
		assert_eq!(groups.initialize(BaseImageHandle(3)), None);
	}

	#[test]
	#[should_panic(expected = "was used without valid contents")]
	fn using_an_overwritten_member_fails_validation() {
		let (mut groups, _, [first, second]) = groups_with_two_sharing_members();

		groups.initialize(first);
		groups.initialize(second);
		groups.assert_initialized(first, || None);
	}

	#[test]
	fn placing_with_the_same_requests_changes_nothing() {
		let (groups, group, images) = groups_with_two_sharing_members();
		let requests = images
			.into_iter()
			.zip([0..=1, 2..=3])
			.rev()
			.map(|(image, lifetime)| ImageGroupMember {
				image,
				extent: Extent::square(4),
				lifetime,
			})
			.collect::<Vec<_>>();

		assert_eq!(groups.requests_in_member_order(group, &requests), None);
	}

	#[test]
	#[should_panic(expected = "was placed with 1 members but has 2")]
	fn placing_without_every_member_fails() {
		let (groups, group, [first, _]) = groups_with_two_sharing_members();

		groups.requests_in_member_order(
			group,
			&[ImageGroupMember {
				image: first,
				extent: Extent::square(4),
				lifetime: 0..=1,
			}],
		);
	}
}
