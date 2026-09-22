//! Scene membership and cascading scene destruction.
//!
//! Create a scene root with [`Scene`], then attach entities to it, or to any
//! other scene member, by chaining [`SceneNode::under`] while spawning them.
//! Deleting a scene, or any member with children, through
//! [`DefaultWorld::delete`](crate::gameplay::DefaultWorld::delete) publishes a
//! [`DeleteMessage`](crate::core::message::DeleteMessage) for every nested
//! element.

use std::collections::HashMap;

use crate::core::factory::Handle;

/// The `Scene` struct gives a level or other group of entities an identity that its members attach to.
///
/// Spawn it with [`Creator::create`](crate::core::factory::Creator::create) and
/// pass the returned handle to [`SceneNode::under`]. Scenes carry no name; chain
/// a [`Name`](crate::gameplay::Name) when tooling needs one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene;

/// The `SceneNode` struct places an entity in a scene so the scene's destruction also destroys it.
///
/// Chain it with [`Creation::with`](crate::core::factory::Creation::with) when
/// spawning an entity. The parent can be a [`Scene`] or any other scene member,
/// which nests the entity under it. The parent is fixed for the entity's lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneNode {
	parent: Handle,
}

impl SceneNode {
	/// Creates membership under `parent`, which is a scene or another scene member.
	pub fn under(parent: Handle) -> Self {
		Self { parent }
	}

	/// Returns the scene or scene member that owns this entity.
	pub fn parent(&self) -> Handle {
		self.parent
	}
}

/// The `Links` struct stores one tree node's intrusive links so nodes never own a separate child list.
#[derive(Clone, Copy, Debug, Default)]
struct Links {
	parent: Option<Handle>,
	first_child: Option<Handle>,
	next_sibling: Option<Handle>,
}

/// The `SceneGraph` struct tracks which entities belong under which scene so a teardown can find every nested element.
///
/// It only tracks membership. [`DefaultWorld`](crate::gameplay::DefaultWorld)
/// feeds it [`SceneNode`] creations and publishes the deletions it reports.
#[derive(Debug, Default)]
pub(crate) struct SceneGraph {
	nodes: HashMap<Handle, Links>,
}

impl SceneGraph {
	/// Links `child` as the first child of `parent`, registering `parent` as a root when it is unknown.
	pub(crate) fn attach(&mut self, child: Handle, parent: Handle) {
		let parent_links = self.nodes.entry(parent).or_default();
		let next_sibling = parent_links.first_child.replace(child);
		self.nodes.insert(
			child,
			Links {
				parent: Some(parent),
				first_child: None,
				next_sibling,
			},
		);
	}

	/// Removes `root` and everything nested under it, reporting each handle children-first and `root` last.
	///
	/// `root` is reported even when it is not tracked.
	pub(crate) fn remove_subtree(&mut self, root: Handle, mut removed: impl FnMut(Handle)) {
		let Some(links) = self.nodes.remove(&root) else {
			return removed(root);
		};
		if let Some(parent) = links.parent {
			self.unlink_child(parent, root, links.next_sibling);
		}

		// Post-order walk without a stack: descend to a leaf, which is always its
		// parent's first child, pop it, and resume from the parent.
		let mut current = links.first_child;
		while let Some(mut node) = current {
			while let Some(child) = self.nodes[&node].first_child {
				node = child;
			}
			let leaf = self.nodes.remove(&node).expect("Walked scene nodes are tracked.");
			removed(node);

			let parent = leaf.parent.expect("Nested scene nodes have a parent.");
			if parent == root {
				current = leaf.next_sibling;
			} else {
				self.nodes.get_mut(&parent).expect("Parents are tracked.").first_child = leaf.next_sibling;
				current = Some(parent);
			}
		}
		removed(root);
	}

	/// Replaces `child` in `parent`'s sibling list with `next`.
	fn unlink_child(&mut self, parent: Handle, child: Handle, next: Option<Handle>) {
		let mut link = &mut self.nodes.get_mut(&parent).expect("Parents are tracked.").first_child;
		while *link != Some(child) {
			let sibling = link.expect("A child is in its parent's sibling list.");
			link = &mut self.nodes.get_mut(&sibling).expect("Siblings are tracked.").next_sibling;
		}
		*link = next;
	}
}
