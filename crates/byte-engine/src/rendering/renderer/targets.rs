use std::ops::RangeInclusive;

use ghi::context::ContextCreate as _;
use smallvec::SmallVec;
use utils::Extent;
use utils::RGBA;

/// The `RenderNode` enum identifies one step of a sink's frame, so render targets can tell when each image is used.
///
/// The derived order is the order the renderer records a sink's steps in: every scene pipeline manager, then every
/// post-scene pass in registration order, then the presentation copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RenderNode {
	/// A scene pipeline manager, by registration index.
	Scene(usize),
	/// A post-scene render pass, by registration index.
	Pass(usize),
	/// The copy that presents scene color when no post-scene pass writes the swapchain.
	Presentation,
}

/// The `FirstUse` struct names a render target that a node is the first to use in a frame.
///
/// The renderer gives the target new contents before that node records: a discard when the node records commands,
/// or `clear` when it records nothing, so later nodes never read memory another target used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FirstUse {
	pub(crate) node: RenderNode,
	pub(crate) image: ghi::BaseImageHandle,
	pub(crate) clear: ghi::ClearValue,
}

/// The `SinkTargetPlan` struct is one frame's plan for a sink's shared-memory render targets.
///
/// Pass `members` to [`ghi::frame::Frame::place_image_group`], then initialize each [`FirstUse`] before its node.
pub(crate) struct SinkTargetPlan {
	pub(crate) group: ghi::ImageGroupHandle,
	pub(crate) members: SmallVec<[ghi::ImageGroupMember; 32]>,
	pub(crate) first_uses: SmallVec<[FirstUse; 32]>,
}

/// The `RenderTargets` struct tracks sink-scoped render images and attachment access.
pub struct RenderTargets {
	pub(super) images: Vec<(ghi::BaseImageHandle, ghi::Formats)>,
	/// Divides the sink extent to size each image in `images`, at the same index. One means full sink resolution.
	pub(super) resolution_divisors: Vec<u32>,
	/// Whether each image in `images`, at the same index, belongs to its sink's image group and so may share memory.
	members: Vec<bool>,
	/// The image group of each sink that has one.
	groups: Vec<(usize, ghi::ImageGroupHandle)>,
	/// The images each node of a sink uses, by image index.
	node_accesses: Vec<(usize, RenderNode, SmallVec<[usize; 8]>)>,
	/// Maps a sink-scoped name to an image index.
	pub(super) by_name: Vec<(usize, String, usize)>,
	/// Maps sink indices to image indices and access policies, making attachments.
	pub(super) by_sink_index: Vec<(usize, (usize, ghi::AccessPolicies))>,
	/// Maps a sink-scoped name to a per-frame image that later frames read as history.
	///
	/// History targets are never attachments, so no pass can clear them by accident.
	pub(super) histories: Vec<(usize, String, ghi::DynamicImageHandle, u32)>,
}

impl Default for RenderTargets {
	fn default() -> Self {
		Self::new()
	}
}

impl RenderTargets {
	pub fn new() -> Self {
		Self {
			images: Vec::with_capacity(32),
			resolution_divisors: Vec::with_capacity(32),
			members: Vec::with_capacity(32),
			groups: Vec::new(),
			node_accesses: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			by_sink_index: Vec::with_capacity(32),
			histories: Vec::with_capacity(8),
		}
	}

	pub fn alias(&mut self, sink_id: usize, orig: &str, alias: &str) {
		if let Some(index) = self.get_image_index(orig, sink_id) {
			self.by_name.push((sink_id, alias.to_string(), index));
		}
	}

	/// Returns the image group of a sink, creating it on first use.
	///
	/// Build shared-memory targets into it with [`ghi::image::Builder::group`], then register them with
	/// [`Self::insert`] as members.
	pub(crate) fn group(&mut self, sink_id: usize, context: &mut ghi::implementation::Context) -> ghi::ImageGroupHandle {
		if let Some((_, group)) = self.groups.iter().find(|(sink, _)| *sink == sink_id) {
			return *group;
		}
		let group = context.create_image_group(Some(&format!("Sink {sink_id} Render Targets")));
		self.groups.push((sink_id, group));
		group
	}

	/// Inserts a render-target image for a sink and returns its storage index.
	///
	/// The renderer sizes the image to the sink extent divided by `resolution_divisor`, rounded down and at least one.
	/// A `member` image was built into the sink's image group, so the renderer places it with the group
	/// instead of resizing it alone.
	pub fn insert(
		&mut self,
		name: String,
		sink_id: usize,
		image: ghi::BaseImageHandle,
		format: ghi::Formats,
		resolution_divisor: u32,
		member: bool,
	) -> usize {
		assert!(
			resolution_divisor > 0,
			"Render target '{name}' has a zero resolution divisor. The most likely cause is a divisor computed from an empty value."
		);
		if self.get_image_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' already exists for sink {sink_id}. The most likely cause is that two render pipeline setup paths create the same named target."
			);
		};

		if self.get_attachment_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' is already registered as an attachment for sink {sink_id}. The most likely cause is that a target was manually added to the attachment list before insertion."
			);
		}

		let index = self.images.len();
		self.images.push((image, format));
		self.resolution_divisors.push(resolution_divisor);
		self.members.push(member);
		self.by_name.push((sink_id, name, index));
		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));

		index
	}

	/// Registers a per-frame history image for a sink, sized like [`Self::insert`] sizes images.
	pub fn insert_history(&mut self, name: String, sink_id: usize, image: ghi::DynamicImageHandle, resolution_divisor: u32) {
		assert!(
			resolution_divisor > 0,
			"History target '{name}' has a zero resolution divisor. The most likely cause is a divisor computed from an empty value."
		);
		if self.history(&name, sink_id).is_some() || self.get_image_index(&name, sink_id).is_some() {
			panic!(
				"Render target image '{name}' already exists for sink {sink_id}. The most likely cause is that two render pipeline setup paths create the same named target."
			);
		}

		self.histories.push((sink_id, name, image, resolution_divisor));
	}

	/// Returns the per-frame history image registered under `name` for a sink.
	pub fn history(&self, name: &str, sink_id: usize) -> Option<ghi::DynamicImageHandle> {
		self.histories
			.iter()
			.find(|(sink, history_name, ..)| *sink == sink_id && history_name == name)
			.map(|(_, _, image, _)| *image)
	}

	pub fn read_from(&mut self, name: &str, sink_id: usize) {
		if self.get_attachment_index(name, sink_id).is_some() {
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::warn!(
				"Render target image '{name}' does not exist for sink {sink_id}; read attachment was not registered. The most likely cause is that a render pass was added before the pipeline that creates this target."
			);
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::READ)));
	}

	pub fn write_to(&mut self, name: &str, sink_id: usize) {
		if self.get_attachment_index(name, sink_id).is_some() {
			return;
		}

		let Some(index) = self.get_image_index(name, sink_id) else {
			log::warn!(
				"Render target image '{name}' does not exist for sink {sink_id}; write attachment was not registered. The most likely cause is that a render pass was added before the pipeline that creates this target."
			);
			return;
		};

		self.by_sink_index.push((sink_id, (index, ghi::AccessPolicies::WRITE)));
	}

	pub fn get(&self, name: &str, sink_id: usize) -> Option<&(ghi::BaseImageHandle, ghi::Formats)> {
		self.get_image_index(name, sink_id).and_then(|index| self.images.get(index))
	}

	pub fn get_attachment_infos(&self, sink_id: usize) -> SmallVec<[ghi::AttachmentInformation; 8]> {
		let attachments = self
			.by_sink_index
			.iter()
			.filter_map(|(v, (i, ap))| {
				if *v == sink_id {
					let (image, format) = self.images.get(*i)?;
					Some((image, format, ap))
				} else {
					None
				}
			})
			.filter(|(_, _, access)| access.intersects(ghi::AccessPolicies::WRITE))
			.map(|(image, format, access)| {
				ghi::AttachmentInformation::new(
					*image,
					ghi::Layouts::RenderTarget,
					attachment_load(*access),
					ghi::StoreOp::Store,
				)
				// TODO: contionally pass format
			});

		attachments.collect()
	}

	/// Resolves attachments at pass registration, before later passes can rebind names.
	pub fn get_attachment_infos_for_resources(
		&self,
		sink_id: usize,
		resources: &[(&str, ghi::AccessPolicies)],
	) -> SmallVec<[ghi::AttachmentInformation; 8]> {
		let mut accesses_by_name = SmallVec::<[(&str, ghi::AccessPolicies); 8]>::new();
		for (name, access) in resources {
			if let Some((_, existing)) = accesses_by_name.iter_mut().find(|(existing_name, _)| *existing_name == *name) {
				*existing |= *access;
			} else {
				accesses_by_name.push((*name, *access));
			}
		}

		accesses_by_name
			.into_iter()
			.filter_map(|(name, access)| {
				if !access.intersects(ghi::AccessPolicies::WRITE) {
					return None;
				}

				let (image, _format) = self.get(name, sink_id)?;
				Some(ghi::AttachmentInformation::new(
					*image,
					ghi::Layouts::RenderTarget,
					attachment_load(access),
					ghi::StoreOp::Store,
				))
			})
			.collect()
	}

	fn get_image(&self, name: &str, sink_id: usize) -> &ghi::BaseImageHandle {
		let index = self.get_attachment_index(name, sink_id).unwrap();
		&self.images.get(index).unwrap().0
	}

	pub(crate) fn image(&self, index: usize) -> Option<(ghi::BaseImageHandle, ghi::Formats)> {
		self.images.get(index).copied()
	}

	pub(crate) fn get_image_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		self.by_name
			.iter()
			.rev()
			.find(|(sink, n, _)| *sink == sink_id && n == name)
			.map(|(_, _, i)| *i)
	}

	/// Snapshots current names that resolve to one of the selected image indices.
	pub(crate) fn names_for_images(&self, sink_id: usize, indices: &[usize]) -> Vec<(String, ghi::BaseImageHandle)> {
		self.by_name
			.iter()
			.enumerate()
			.filter(|(position, (sink, _, index))| {
				*sink == sink_id && indices.contains(index) && self.is_current_name_mapping(*position)
			})
			.filter_map(|(_, (_, name, index))| self.images.get(*index).map(|(image, _)| (name.clone(), *image)))
			.collect()
	}

	fn is_current_name_mapping(&self, position: usize) -> bool {
		let (sink, name, _) = &self.by_name[position];
		!self.by_name[position + 1..]
			.iter()
			.any(|(later_sink, later_name, _)| later_sink == sink && later_name == name)
	}

	#[cfg(test)]
	fn name_indices_for_images(&self, sink_id: usize, indices: &[usize]) -> Vec<(String, usize)> {
		self.by_name
			.iter()
			.enumerate()
			.filter(|(position, (sink, _, index))| {
				*sink == sink_id && indices.contains(index) && self.is_current_name_mapping(*position)
			})
			.map(|(_, (_, name, index))| (name.clone(), *index))
			.collect()
	}

	fn get_attachment_index(&self, name: &str, sink_id: usize) -> Option<usize> {
		let image_index = self.get_image_index(name, sink_id)?;

		self.by_sink_index
			.iter()
			.find_map(|(v, (i, _))| if *v == sink_id && *i == image_index { Some(*i) } else { None })
	}

	/// Records the images one node of a sink uses, by image index.
	///
	/// The renderer calls this once per node after building it, so [`Self::plan`] can tell when each target is used.
	pub(crate) fn record_node(&mut self, sink_id: usize, node: RenderNode, accesses: impl IntoIterator<Item = usize>) {
		let mut accesses = accesses.into_iter().collect::<SmallVec<[usize; 8]>>();
		accesses.sort_unstable();
		accesses.dedup();
		self.node_accesses.push((sink_id, node, accesses));
	}

	/// Plans one frame of a sink's shared-memory targets: each member's extent and lifetime, and the node that uses it
	/// first. Returns `None` when the sink has no image group.
	pub(crate) fn plan(&self, sink_id: usize, sink_extent: Extent) -> Option<SinkTargetPlan> {
		let (_, group) = *self.groups.iter().find(|(sink, _)| *sink == sink_id)?;
		let mut nodes = self
			.node_accesses
			.iter()
			.filter(|(sink, ..)| *sink == sink_id)
			.map(|(_, node, accesses)| (*node, accesses.as_slice()))
			.collect::<SmallVec<[_; 32]>>();
		nodes.sort_unstable_by_key(|(node, _)| *node);

		let mut plan = SinkTargetPlan {
			group,
			members: SmallVec::new(),
			first_uses: SmallVec::new(),
		};
		let sink_members = self
			.by_name
			.iter()
			.filter(|(sink, ..)| *sink == sink_id)
			.map(|(.., index)| *index)
			.filter(|&index| self.members[index]);
		for index in sink_members {
			// Aliases add names, not images, so each member is planned once.
			if plan.members.iter().any(|member| member.image == self.images[index].0) {
				continue;
			}
			let (image, format) = self.images[index];
			let lifetime = lifetime(&nodes, index);
			plan.members.push(ghi::ImageGroupMember {
				image,
				extent: scaled_extent(sink_extent, self.resolution_divisors[index]),
				lifetime: lifetime.clone(),
			});
			if let Some((node, _)) = nodes.get(*lifetime.start() as usize) {
				plan.first_uses.push(FirstUse {
					node: *node,
					image,
					clear: clear_value(format),
				});
			}
		}
		Some(plan)
	}

	/// Reports whether a target still holds what the scene wrote once every scene pipeline manager has recorded.
	///
	/// Shared-memory targets that no scene manager uses get their contents later in the frame, so a capture after the
	/// scene would read memory another target owns.
	pub(crate) fn holds_scene_output(&self, name: &str, sink_id: usize) -> bool {
		let Some(index) = self.get_image_index(name, sink_id) else {
			return false;
		};
		!self.members[index]
			|| self.node_accesses.iter().any(|(sink, node, accesses)| {
				*sink == sink_id && matches!(node, RenderNode::Scene(_)) && accesses.contains(&index)
			})
	}

	/// Returns every image that follows the sink's extent on its own, with the extent it needs.
	///
	/// These are history images and targets outside the sink's image group; [`Self::plan`] sizes group members.
	pub(super) fn get_images_for_sink(
		&self,
		index: usize,
		sink_extent: Extent,
	) -> impl Iterator<Item = (ghi::BaseImageHandle, Extent)> {
		let targets = self.by_sink_index.iter().filter_map(move |(v, (i, _))| {
			if *v != index || self.members[*i] {
				return None;
			}

			let (image, _) = self.images.get(*i)?;
			Some((*image, scaled_extent(sink_extent, self.resolution_divisors[*i])))
		});
		let histories = self
			.histories
			.iter()
			.filter(move |(sink, ..)| *sink == index)
			.map(move |(_, _, image, divisor)| ((*image).into(), scaled_extent(sink_extent, *divisor)));
		targets.chain(histories)
	}
}

/// Returns the positions, in recording order, of the first and last of `nodes` that use the image at `index`.
///
/// An image no node uses gets the first position, so it still has memory.
fn lifetime(nodes: &[(RenderNode, &[usize])], index: usize) -> RangeInclusive<u32> {
	let uses = |(position, (_, accesses)): (usize, &(RenderNode, &[usize]))| {
		accesses.contains(&index).then_some(position as u32)
	};
	let first = nodes.iter().enumerate().find_map(uses).unwrap_or(0);
	let last = nodes.iter().enumerate().rev().find_map(uses).unwrap_or(first);
	first..=last
}

/// Loads an attachment a pass also reads, and clears one it only writes.
fn attachment_load(access: ghi::AccessPolicies) -> ghi::LoadOp {
	if access.intersects(ghi::AccessPolicies::READ) {
		ghi::LoadOp::Load
	} else {
		ghi::LoadOp::Clear(ghi::ClearValue::Color(RGBA::black()))
	}
}

/// Returns the value that clears an image of `format` to empty contents.
fn clear_value(format: ghi::Formats) -> ghi::ClearValue {
	if format.is_depth() {
		ghi::ClearValue::Depth(0.0)
	} else if format == ghi::Formats::U32 {
		ghi::ClearValue::Integer(0, 0, 0, 0)
	} else {
		ghi::ClearValue::Color(RGBA::black())
	}
}

/// Divides a sink extent for a reduced-resolution target, keeping every dimension at least one.
pub(crate) fn scaled_extent(extent: Extent, resolution_divisor: u32) -> Extent {
	Extent::rectangle(
		(extent.width() / resolution_divisor).max(1),
		(extent.height() / resolution_divisor).max(1),
	)
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{RenderNode, RenderTargets};
	use crate::rendering::renderer::tests::image_handle;

	/// Registers three shared-memory targets for sink 0: the scene writes `a`, the first pass reads `a` and writes `b`,
	/// and the second pass reads `b` and writes `c`. Nodes are recorded out of order, as later managers can be.
	fn chained_targets() -> (RenderTargets, [ghi::BaseImageHandle; 3]) {
		let mut targets = RenderTargets::new();
		targets.groups.push((0, ghi::debug::Device::new().create_image_group(None)));
		let images = [1, 2, 3].map(image_handle);
		let [a, b, c] = [("a", images[0]), ("b", images[1]), ("c", images[2])]
			.map(|(name, image)| targets.insert(name.into(), 0, image, ghi::Formats::RGBA16F, 1, true));
		targets.record_node(0, RenderNode::Pass(1), [b, c]);
		targets.record_node(0, RenderNode::Scene(0), [a]);
		targets.record_node(0, RenderNode::Pass(0), [a, b]);
		(targets, images)
	}

	#[test]
	fn shared_targets_live_from_their_first_to_their_last_node_in_recording_order() {
		let (targets, [a, b, c]) = chained_targets();
		let plan = targets.plan(0, Extent::rectangle(8, 4)).unwrap();

		let lifetimes = plan
			.members
			.iter()
			.map(|member| (member.image, member.lifetime.clone()))
			.collect::<Vec<_>>();
		assert_eq!(lifetimes, [(a, 0..=1), (b, 1..=2), (c, 2..=2)]);
		let first_uses = plan
			.first_uses
			.iter()
			.map(|first_use| (first_use.image, first_use.node))
			.collect::<Vec<_>>();
		assert_eq!(
			first_uses,
			[(a, RenderNode::Scene(0)), (b, RenderNode::Pass(0)), (c, RenderNode::Pass(1))]
		);
		assert!(targets.plan(1, Extent::rectangle(8, 4)).is_none());
	}

	#[test]
	fn persistent_targets_resize_alone_while_shared_targets_are_placed_with_their_group() {
		let (mut targets, _) = chained_targets();
		let persistent = image_handle(4);
		targets.insert("persistent".into(), 0, persistent, ghi::Formats::RGBA16F, 2, false);
		let extent = Extent::rectangle(8, 4);

		let plan = targets.plan(0, extent).unwrap();
		assert!(plan.members.iter().all(|member| member.image != persistent));
		assert!(plan.members.iter().all(|member| member.extent == extent));
		assert_eq!(
			targets.get_images_for_sink(0, extent).collect::<Vec<_>>(),
			[(persistent, Extent::rectangle(4, 2))]
		);
	}

	#[test]
	fn only_targets_the_scene_uses_hold_scene_output() {
		let (mut targets, _) = chained_targets();
		targets.insert("persistent".into(), 0, image_handle(4), ghi::Formats::RGBA16F, 1, false);

		assert!(targets.holds_scene_output("a", 0));
		assert!(!targets.holds_scene_output("c", 0));
		assert!(targets.holds_scene_output("persistent", 0));
		assert!(!targets.holds_scene_output("missing", 0));
	}

	#[test]
	fn history_targets_follow_the_sink_extent_without_becoming_attachments() {
		let history = ghi::debug::Device::new()
			.build_dynamic_image(ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image).name("History"));
		let mut targets = RenderTargets::new();

		targets.insert_history("History".into(), 0, history, 2);

		assert_eq!(targets.history("History", 0), Some(history));
		assert_eq!(targets.history("History", 1), None);
		assert_eq!(
			targets
				.get_images_for_sink(0, Extent::rectangle(1919, 1080))
				.collect::<Vec<_>>(),
			[(history.into(), Extent::rectangle(959, 540))]
		);
		assert!(targets.get_attachment_infos(0).is_empty());
	}

	#[test]
	fn writable_snapshot_excludes_read_only_images() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "written".into(), 3), (0, "read-only".into(), 4)];

		assert_eq!(targets.name_indices_for_images(0, &[3]), [("written".into(), 3)]);
	}

	#[test]
	fn alias_snapshot_is_unchanged_by_a_later_alias() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "Bloom Output".into(), 3), (0, "main".into(), 3)];
		let bloom = targets.name_indices_for_images(0, &[3]);

		targets.by_name.push((0, "main".into(), 4));

		assert_eq!(bloom, [("Bloom Output".into(), 3), ("main".into(), 3)]);
		assert_eq!(targets.get_image_index("main", 0), Some(4));
	}

	#[test]
	fn current_snapshot_excludes_an_overwritten_alias() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "image-a".into(), 3), (0, "main".into(), 3), (0, "main".into(), 4)];

		assert_eq!(targets.name_indices_for_images(0, &[3]), [("image-a".into(), 3)]);
	}
}
