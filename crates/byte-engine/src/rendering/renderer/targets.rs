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
	/// The pass that tiles the shown debug views over the presented image; see [`crate::rendering::debug_views`].
	DebugView,
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

/// The `Placement` enum is how a render target gets its memory and its first contents each frame.
///
/// Pass it to [`RenderTargets::insert`] for the image a render pass builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
	/// The image belongs to its sink's image group and shares memory with targets whose uses do not overlap. Its first
	/// node each frame writes it, or the renderer clears it when that node records nothing.
	Member,
	/// The image keeps its contents across frames, which its passes update themselves.
	Persistent,
	/// The image keeps one copy per frame in flight, so the next frame can read this frame's copy as history. Its first
	/// node writes this frame's copy, or the renderer clears it when that node records nothing. It holds the image's
	/// per-frame handle, which [`RenderTargets::history`] returns.
	PerFrame(ghi::ImageHandle),
}

/// The most render targets one sink may hold, which sizes [`TargetSet`].
const MAX_TARGETS_PER_SINK: usize = 256;

/// The `TargetId` struct names one render target among its sink's, so a scene pipeline can ask its sink whether an
/// enabled pass reads what it writes.
///
/// Get one from [`crate::rendering::RenderPassBuilder::target_id`] when the pipeline creates the target, and test it
/// with [`crate::rendering::Sink::reads`] each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetId(u8);

/// The `TargetSet` struct holds a set of one sink's render targets, such as the ones its enabled passes read this frame.
///
/// The renderer builds one per sink and frame and hands it to [`crate::rendering::Sink::with_read_targets`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetSet([u64; MAX_TARGETS_PER_SINK / 64]);

impl TargetSet {
	/// Adds `target` to the set.
	pub(crate) fn insert(&mut self, target: TargetId) {
		let bit = target.0 as usize;
		self.0[bit / 64] |= 1 << (bit % 64);
	}

	/// Returns whether the set holds `target`.
	pub fn contains(&self, target: TargetId) -> bool {
		let bit = target.0 as usize;
		self.0[bit / 64] & (1 << (bit % 64)) != 0
	}

	/// Returns the targets either set holds.
	pub(crate) fn union(self, other: Self) -> Self {
		Self(std::array::from_fn(|word| self.0[word] | other.0[word]))
	}
}

/// The `Target` struct describes one render-target image a sink's graph created.
struct Target {
	image: ghi::BaseImageHandle,
	format: ghi::Formats,
	sink: usize,
	/// Divides the sink extent to size the image. One means full sink resolution.
	resolution_divisor: u32,
	/// Whether the image follows the sink's output extent instead of its render extent: it was created by the
	/// upscaling pass or a pass after it; see [`RenderTargets::begin_output`].
	at_output: bool,
	/// How the image gets its memory and its first contents each frame.
	placement: Placement,
	/// The debug target pool slot this image is, if it is one. A free slot is sized `1×1`; see [`RenderTargets::debug_pool`].
	debug_slot: Option<u32>,
}

/// The `RenderTargets` struct tracks sink-scoped render images, their names, and the graph nodes that use them.
///
/// Render passes reach it through [`crate::rendering::RenderPassBuilder`], which registers every target a pass
/// creates or uses, and the renderer asks it each frame how to size and place those targets.
pub struct RenderTargets {
	/// Every render-target image, indexed by the image index the other lists refer to.
	targets: Vec<Target>,
	/// The image group of each sink that has one.
	groups: Vec<(usize, ghi::ImageGroupHandle)>,
	/// The images each node of a sink uses, by image index.
	node_accesses: Vec<(usize, RenderNode, SmallVec<[usize; 8]>)>,
	/// Maps a sink-scoped name to an image index. Later entries for a name replace earlier ones.
	pub(super) by_name: Vec<(usize, String, usize)>,
	/// One bit per debug pool slot, set while a view holds it; those slots follow the sink extent, the others stay `1×1`.
	debug_slots_taken: u32,
	/// The sinks whose upscaling pass is built, so the targets created from then on follow the output extent.
	output_sinks: SmallVec<[usize; 4]>,
}

impl Default for RenderTargets {
	fn default() -> Self {
		Self::new()
	}
}

impl RenderTargets {
	pub fn new() -> Self {
		Self {
			targets: Vec::with_capacity(32),
			groups: Vec::new(),
			node_accesses: Vec::with_capacity(32),
			by_name: Vec::with_capacity(32),
			debug_slots_taken: 0,
			output_sinks: SmallVec::new(),
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

	/// Makes every target `sink_id` creates from now on follow its output extent instead of its render extent. The
	/// upscaling pass calls it, through [`crate::rendering::RenderPassBuilder::create_output_main_render_target`],
	/// before it creates `main`; see the [render scale](/docs/reference/rendering#render-scale) specification.
	///
	/// # Panics
	///
	/// Panics when the sink already has an upscaling pass.
	pub(crate) fn begin_output(&mut self, sink_id: usize) {
		assert!(
			!self.is_output(sink_id),
			"Sink {sink_id} has two upscaling passes. The most likely cause is that two setup functions install a pass that calls `RenderPassBuilder::create_output_main_render_target`. Install only one of them."
		);
		self.output_sinks.push(sink_id);
	}

	/// Reports whether `sink_id`'s upscaling pass is built, so the nodes built from now on render at its output extent.
	pub(crate) fn is_output(&self, sink_id: usize) -> bool {
		self.output_sinks.contains(&sink_id)
	}

	/// Inserts a render-target image for a sink and returns its storage index.
	///
	/// The renderer sizes the image to the sink extent divided by `resolution_divisor`, rounded down and at least one.
	/// A [`Placement::Member`] image was built into the sink's image group, so the renderer places it with the group
	/// instead of resizing it alone.
	pub fn insert(
		&mut self,
		name: String,
		sink_id: usize,
		image: ghi::BaseImageHandle,
		format: ghi::Formats,
		resolution_divisor: u32,
		placement: Placement,
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

		let sink_targets = self.targets.iter().filter(|target| target.sink == sink_id).count();
		assert!(
			sink_targets < MAX_TARGETS_PER_SINK,
			"Render target '{name}' is sink {sink_id}'s target number {}, more than a sink holds. The most likely cause is a pass that creates a target per frame or per item. Create the target once, or raise `MAX_TARGETS_PER_SINK`.",
			sink_targets + 1
		);
		let index = self.targets.len();
		let at_output = self.is_output(sink_id);
		self.targets.push(Target {
			image,
			format,
			sink: sink_id,
			resolution_divisor,
			at_output,
			placement,
			debug_slot: None,
		});
		self.by_name.push((sink_id, name, index));

		index
	}

	/// Returns a sink's debug target pool in slot order, creating its `pool_size` images on the first call.
	///
	/// The pool images are output-sized, sink-color-format storage images named `Debug View <slot>`, so screenshots
	/// capture them by name; `build_image` builds each from its builder. A slot stays `1×1` until
	/// [`Self::set_debug_slots_taken`] marks it taken, so an idle pool costs almost nothing. Returns the image
	/// indices too, for the caller to record as accesses.
	pub(crate) fn debug_pool(
		&mut self,
		sink_id: usize,
		pool_size: u32,
		mut build_image: impl FnMut(ghi::image::Builder<'_>) -> ghi::BaseImageHandle,
	) -> SmallVec<[(usize, ghi::BaseImageHandle); 15]> {
		let existing = self
			.targets
			.iter()
			.enumerate()
			.filter(|(_, target)| target.sink == sink_id && target.debug_slot.is_some())
			.map(|(index, target)| (index, target.image))
			.collect::<SmallVec<[_; 15]>>();
		if !existing.is_empty() {
			return existing;
		}
		(0..pool_size)
			.map(|slot| {
				let name = format!("Debug View {slot}");
				let image = build_image(
					ghi::image::Builder::new(
						crate::rendering::SCENE_COLOR_FORMAT,
						ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferSource,
					)
					.name(&name),
				);
				let index = self.insert(
					name,
					sink_id,
					image,
					crate::rendering::SCENE_COLOR_FORMAT,
					1,
					Placement::Persistent,
				);
				// The tiling pass presents the slots over the sink's final output, so they follow the output extent even
				// when a pass before the upscaler creates the pool.
				self.targets[index].debug_slot = Some(slot);
				self.targets[index].at_output = true;
				(index, image)
			})
			.collect()
	}

	/// Marks which debug pool slots a view holds, one bit per slot, so [`Self::get_images_for_sink`] sizes them.
	pub(crate) fn set_debug_slots_taken(&mut self, taken: u32) {
		self.debug_slots_taken = taken;
	}

	/// Returns the per-frame image a sink's `name` resolves to, when it is a [`Placement::PerFrame`] target.
	pub fn history(&self, name: &str, sink_id: usize) -> Option<ghi::ImageHandle> {
		match self.targets[self.get_image_index(name, sink_id)?].placement {
			Placement::PerFrame(image) => Some(image),
			Placement::Member | Placement::Persistent => None,
		}
	}

	/// Returns the image and format a sink's `name` currently resolves to.
	pub fn get(&self, name: &str, sink_id: usize) -> Option<(ghi::BaseImageHandle, ghi::Formats)> {
		self.get_image_index(name, sink_id).map(|index| self.image(index))
	}

	/// Returns the identity the image stored at `index` has among its sink's targets.
	pub(crate) fn id(&self, index: usize) -> TargetId {
		let sink = self.targets[index].sink;
		// Targets are only ever added, so a target's position among its sink's never changes.
		let ordinal = self.targets[..index].iter().filter(|target| target.sink == sink).count();
		TargetId(ordinal as u8)
	}

	/// Returns the image and format stored at `index`.
	pub(crate) fn image(&self, index: usize) -> (ghi::BaseImageHandle, ghi::Formats) {
		let target = &self.targets[index];
		(target.image, target.format)
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
			.map(|(_, (_, name, index))| (name.clone(), self.targets[*index].image))
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

	/// Records the images one node of a sink uses, by image index.
	///
	/// The renderer calls this once per node after building it, so [`Self::plan`] can tell when each target is used.
	pub(crate) fn record_node(&mut self, sink_id: usize, node: RenderNode, accesses: impl IntoIterator<Item = usize>) {
		let mut accesses = accesses.into_iter().collect::<SmallVec<[usize; 8]>>();
		accesses.sort_unstable();
		accesses.dedup();
		self.node_accesses.push((sink_id, node, accesses));
	}

	/// Plans one frame of a sink's shared-memory targets: each member's extent and lifetime, and the node that uses each
	/// member or per-frame target first. Returns `None` when the sink has no image group.
	pub(crate) fn plan(&self, sink_id: usize, extents: SinkExtents) -> Option<SinkTargetPlan> {
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
		for (index, target) in self.targets.iter().enumerate().filter(|(_, target)| target.sink == sink_id) {
			let lifetime = lifetime(&nodes, index);
			match target.placement {
				Placement::Member => plan.members.push(ghi::ImageGroupMember {
					image: target.image,
					extent: target.extent(extents),
					lifetime: lifetime.clone(),
				}),
				// A persistent image keeps what its passes wrote, so it is neither placed nor initialized.
				Placement::Persistent => continue,
				Placement::PerFrame(_) => {}
			}
			if let Some((node, _)) = nodes.get(*lifetime.start() as usize) {
				plan.first_uses.push(FirstUse {
					node: *node,
					image: target.image,
					clear: clear_value(target.format),
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
		self.targets[index].placement != Placement::Member
			|| self.node_accesses.iter().any(|(sink, node, accesses)| {
				*sink == sink_id && matches!(node, RenderNode::Scene(_)) && accesses.contains(&index)
			})
	}

	/// Returns every image that follows the sink's extent on its own, with the extent it needs.
	///
	/// These are the targets outside the sink's image group; [`Self::plan`] sizes group members.
	pub(super) fn get_images_for_sink(
		&self,
		index: usize,
		extents: SinkExtents,
	) -> impl Iterator<Item = (ghi::BaseImageHandle, Extent)> {
		let debug_slots_taken = self.debug_slots_taken;
		self.targets
			.iter()
			.filter(move |target| target.sink == index && target.placement != Placement::Member)
			.map(move |target| {
				let extent = match target.debug_slot {
					// A debug slot no view holds keeps the smallest image, so an idle pool costs almost nothing.
					Some(slot) if debug_slots_taken & (1 << slot) == 0 => Extent::rectangle(1, 1),
					_ => target.extent(extents),
				};
				(target.image, extent)
			})
	}
}

/// The `SinkExtents` struct holds the two extents a sink's targets follow this frame, so the renderer sizes each
/// target from the one its node renders at; see the [render scale](/docs/reference/rendering#render-scale)
/// specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SinkExtents {
	/// The extent scene pipelines and the passes before the upscaling pass render at.
	pub(crate) render: Extent,
	/// The sink's own extent, which the upscaling pass and the passes after it render at.
	pub(crate) output: Extent,
}

impl SinkExtents {
	/// Returns the extents of a sink that renders and outputs at `extent`, with no upscaling.
	#[cfg(test)]
	pub(crate) fn uniform(extent: Extent) -> Self {
		Self {
			render: extent,
			output: extent,
		}
	}
}

impl Target {
	/// Returns the extent the image needs this frame: the extent its node renders at, divided by its divisor.
	fn extent(&self, extents: SinkExtents) -> Extent {
		let sink_extent = if self.at_output { extents.output } else { extents.render };
		sink_extent.scaled_down(self.resolution_divisor)
	}
}

/// Returns the positions, in recording order, of the first and last of `nodes` that use the image at `index`.
///
/// An image no node uses gets the first position, so it still has memory.
fn lifetime(nodes: &[(RenderNode, &[usize])], index: usize) -> RangeInclusive<u32> {
	let uses =
		|(position, (_, accesses)): (usize, &(RenderNode, &[usize]))| accesses.contains(&index).then_some(position as u32);
	let first = nodes.iter().enumerate().find_map(uses).unwrap_or(0);
	let last = nodes.iter().enumerate().rev().find_map(uses).unwrap_or(first);
	first..=last
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

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{Placement, RenderNode, RenderTargets, SinkExtents};
	use crate::rendering::renderer::tests::image_handle;

	/// Registers three shared-memory targets for sink 0: the scene writes `a`, the first pass reads `a` and writes `b`,
	/// and the second pass reads `b` and writes `c`. Nodes are recorded out of order, as later managers can be.
	fn chained_targets() -> (RenderTargets, [ghi::BaseImageHandle; 3]) {
		let mut targets = RenderTargets::new();
		targets.groups.push((0, ghi::debug::Device::new().create_image_group(None)));
		let images = [1, 2, 3].map(image_handle);
		let [a, b, c] = [("a", images[0]), ("b", images[1]), ("c", images[2])]
			.map(|(name, image)| targets.insert(name.into(), 0, image, ghi::Formats::RGBA16F, 1, Placement::Member));
		targets.record_node(0, RenderNode::Pass(1), [b, c]);
		targets.record_node(0, RenderNode::Scene(0), [a]);
		targets.record_node(0, RenderNode::Pass(0), [a, b]);
		(targets, images)
	}

	#[test]
	fn shared_targets_live_from_their_first_to_their_last_node_in_recording_order() {
		let (targets, [a, b, c]) = chained_targets();
		let plan = targets.plan(0, SinkExtents::uniform(Extent::rectangle(8, 4))).unwrap();

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
		assert!(targets.plan(1, SinkExtents::uniform(Extent::rectangle(8, 4))).is_none());
	}

	#[test]
	fn persistent_targets_resize_alone_while_shared_targets_are_placed_with_their_group() {
		let (mut targets, _) = chained_targets();
		let persistent = image_handle(4);
		targets.insert(
			"persistent".into(),
			0,
			persistent,
			ghi::Formats::RGBA16F,
			2,
			Placement::Persistent,
		);
		let extent = Extent::rectangle(8, 4);

		let plan = targets.plan(0, SinkExtents::uniform(extent)).unwrap();
		assert!(plan.members.iter().all(|member| member.image != persistent));
		assert!(plan.members.iter().all(|member| member.extent == extent));
		assert_eq!(
			targets
				.get_images_for_sink(0, SinkExtents::uniform(extent))
				.collect::<Vec<_>>(),
			[(persistent, Extent::rectangle(4, 2))]
		);
	}

	#[test]
	fn only_targets_the_scene_uses_hold_scene_output() {
		let (mut targets, _) = chained_targets();
		targets.insert(
			"persistent".into(),
			0,
			image_handle(4),
			ghi::Formats::RGBA16F,
			1,
			Placement::Persistent,
		);

		assert!(targets.holds_scene_output("a", 0));
		assert!(!targets.holds_scene_output("c", 0));
		assert!(targets.holds_scene_output("persistent", 0));
		assert!(!targets.holds_scene_output("missing", 0));
	}

	#[test]
	fn history_targets_follow_the_sink_extent() {
		let history = ghi::debug::Device::new().build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image)
				.name("History")
				.copies(ghi::Copies::PerFrame),
		);
		let mut targets = RenderTargets::new();

		targets.insert(
			"History".into(),
			0,
			history.into(),
			ghi::Formats::RGBA16F,
			2,
			Placement::PerFrame(history),
		);

		assert_eq!(targets.history("History", 0), Some(history));
		assert_eq!(targets.history("History", 1), None);
		assert_eq!(
			targets
				.get_images_for_sink(0, SinkExtents::uniform(Extent::rectangle(1919, 1080)))
				.collect::<Vec<_>>(),
			[(history.into(), Extent::rectangle(959, 540))]
		);
	}

	#[test]
	fn debug_pool_slots_follow_the_sink_extent_only_while_taken() {
		let mut targets = RenderTargets::new();
		let mut next = 10;
		let mut build = |_: ghi::image::Builder<'_>| {
			next += 1;
			image_handle(next)
		};
		let pool = targets.debug_pool(0, 2, &mut build);
		assert_eq!(pool.len(), 2);
		// The pool is created once per sink.
		assert_eq!(targets.debug_pool(0, 2, &mut build), pool);
		assert_eq!(targets.get("Debug View 1", 0).map(|(image, _)| image), Some(pool[1].1));
		let extent = Extent::rectangle(8, 4);

		let extents = |targets: &RenderTargets| {
			targets
				.get_images_for_sink(0, SinkExtents::uniform(extent))
				.map(|(_, extent)| extent)
				.collect::<Vec<_>>()
		};
		assert_eq!(extents(&targets), [Extent::rectangle(1, 1), Extent::rectangle(1, 1)]);
		targets.set_debug_slots_taken(0b10);
		assert_eq!(extents(&targets), [Extent::rectangle(1, 1), extent]);
		assert!(targets.holds_scene_output("Debug View 0", 0));
	}

	#[test]
	fn targets_created_from_the_upscaling_pass_on_follow_the_output_extent() {
		let mut targets = RenderTargets::new();
		let scene = image_handle(1);
		let upscaled = image_handle(2);
		targets.insert("scene".into(), 0, scene, ghi::Formats::RGBA16F, 1, Placement::Persistent);
		targets.begin_output(0);
		targets.insert(
			"upscaled".into(),
			0,
			upscaled,
			ghi::Formats::RGBA16F,
			2,
			Placement::Persistent,
		);
		let extents = SinkExtents {
			render: Extent::rectangle(1280, 720),
			output: Extent::rectangle(1920, 1080),
		};

		assert_eq!(
			targets.get_images_for_sink(0, extents).collect::<Vec<_>>(),
			[(scene, Extent::rectangle(1280, 720)), (upscaled, Extent::rectangle(960, 540))]
		);
		assert!(targets.is_output(0));
		assert!(!targets.is_output(1));
	}

	#[test]
	#[should_panic(expected = "two upscaling passes")]
	fn a_sink_has_at_most_one_upscaling_pass() {
		let mut targets = RenderTargets::new();
		targets.begin_output(0);
		targets.begin_output(0);
	}

	#[test]
	fn the_debug_view_node_records_last() {
		assert!(RenderNode::Presentation < RenderNode::DebugView);
		assert!(RenderNode::Pass(usize::MAX) < RenderNode::Presentation);
	}

	#[test]
	fn current_snapshot_excludes_an_overwritten_alias() {
		let mut targets = RenderTargets::new();
		targets.by_name = vec![(0, "image-a".into(), 3), (0, "main".into(), 3), (0, "main".into(), 4)];

		assert_eq!(targets.name_indices_for_images(0, &[3]), [("image-a".into(), 3)]);
	}
}
