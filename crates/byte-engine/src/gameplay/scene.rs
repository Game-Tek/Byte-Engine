//! Scene membership and cascading scene destruction.
//!
//! Create a scene root with [`Scene`], then attach entities to it, or to any
//! other scene member, by chaining [`SceneNode::under`] while spawning them.
//! [`DefaultWorld::delete`](crate::gameplay::DefaultWorld::delete) publishes the
//! deleted handle immediately. The next
//! [`DefaultWorld::update`](crate::gameplay::DefaultWorld::update) publishes every
//! nested element, children before their parents.

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
	/// Records `scene` so members can attach under it.
	///
	/// Registration survives until the scene is deleted.
	pub(crate) fn register(&mut self, scene: Handle) {
		self.nodes.entry(scene).or_default();
	}

	/// Links `child` as the first child of `parent`.
	///
	/// `parent` is a registered scene or a member already linked under one. A missing parent is a
	/// handle whose scene was deleted.
	pub(crate) fn attach(&mut self, child: Handle, parent: Handle) {
		debug_assert!(
			self.nodes.contains_key(&parent),
			"Scene parent {} is not alive. The most likely cause is a stale scene handle.",
			parent.id()
		);
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

	/// Removes `root` and reports every nested member, children before their parents.
	///
	/// `root` itself is not reported. An unknown root reports nothing.
	pub(crate) fn remove_subtree(&mut self, root: Handle, mut removed: impl FnMut(Handle)) {
		let Some(links) = self.nodes.remove(&root) else {
			return;
		};
		if let Some(parent) = links.parent {
			self.unlink_child(parent, root, links.next_sibling);
		}

		// Record a preorder walk, then report it backwards so every child precedes its parent.
		let mut stack = Vec::new();
		let mut sibling = links.first_child;
		while let Some(node) = sibling {
			sibling = self.nodes[&node].next_sibling;
			stack.push(node);
		}
		let mut preorder = Vec::new();
		while let Some(node) = stack.pop() {
			preorder.push(node);
			let mut child = self.nodes[&node].first_child;
			while let Some(next) = child {
				child = self.nodes[&next].next_sibling;
				stack.push(next);
			}
		}
		for node in preorder.into_iter().rev() {
			self.nodes.remove(&node).expect("Walked scene nodes are tracked.");
			removed(node);
		}
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
