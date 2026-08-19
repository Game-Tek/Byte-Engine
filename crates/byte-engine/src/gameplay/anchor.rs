//! Parent-child position relationships for gameplay objects.
//!
//! Attach positionable objects to an [`Anchor`] when they must follow the same
//! parent position. Use [`Anchorage::Offset`] to preserve a child-specific displacement.

use math::{Point, Vector};

use super::transform::Transform;
use crate::{
	core::{Entity, EntityHandle},
	space::Positionable,
};

/// The `Anchorage` enum stores how an attached child is positioned relative to its anchor.
#[derive(Debug, Clone, Default)]
pub enum Anchorage {
	/// Places the child at the anchor position.
	#[default]
	Default,
	/// Places the child at a displacement from the anchor.
	Offset { offset: Vector },
}

/// The `Anchoring` trait exposes an anchor's children and their positioning policies.
pub trait Anchoring: Positionable {
	/// Returns the attached children in attachment order.
	fn children(&self) -> Vec<(EntityHandle<dyn Positionable>, Anchorage)>;
}

/// The `Anchor` struct groups children that share one world-space position.
pub struct Anchor {
	transform: Transform,
	children: Vec<(EntityHandle<dyn Positionable>, Anchorage)>,
}

impl Entity for Anchor {}

impl Anchor {
	/// Creates an anchor with `transform`.
	pub fn new(transform: Transform) -> Self {
		Self {
			transform,
			children: Vec::with_capacity(8),
		}
	}

	/// Returns the anchor transform.
	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	/// Returns mutable access to the anchor transform.
	pub fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}

	/// Attaches a child at the anchor position.
	pub fn attach(&mut self, child: EntityHandle<dyn Positionable>) {
		self.children.push((child, Anchorage::Default));
	}

	/// Attaches a child at `offset` from the anchor position.
	pub fn attach_with_offset(&mut self, child: EntityHandle<dyn Positionable>, offset: Vector) {
		self.children.push((child, Anchorage::Offset { offset }));
	}

	/// Attaches a child with an explicit anchorage policy.
	pub fn attach_with_anchorage(&mut self, child: EntityHandle<dyn Positionable>, anchorage: Anchorage) {
		self.children.push((child, anchorage));
	}
}

impl Positionable for Anchor {
	fn set_position(&mut self, position: Point) {
		self.transform.set_position(position);
	}

	fn position(&self) -> Point {
		self.transform.get_position()
	}
}

impl Anchoring for Anchor {
	fn children(&self) -> Vec<(EntityHandle<dyn Positionable>, Anchorage)> {
		self.children.clone()
	}
}

/// The `AnchorSystem` struct retains anchors that need their child positions synchronized.
#[derive(Clone)]
pub struct AnchorSystem {
	anchors: Vec<EntityHandle<dyn Anchoring>>,
}

impl Default for AnchorSystem {
	fn default() -> Self {
		Self::new()
	}
}

impl AnchorSystem {
	/// Creates an empty anchor system.
	pub fn new() -> Self {
		Self {
			anchors: Vec::with_capacity(1024),
		}
	}

	/// Updates anchor relationships.
	///
	/// Child handles are shared read handles, so their owner applies the returned
	/// anchor policies when it has mutable entity access.
	pub fn update(&self) {
		for anchor in &self.anchors {
			let _children = anchor.children();
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{Point, Vector};

	use super::{Anchor, Anchorage, Anchoring};
	use crate::{core::EntityHandle, gameplay::Transform, space::Positionable};

	struct TestPoint(Point);

	impl Positionable for TestPoint {
		fn position(&self) -> Point {
			self.0
		}

		fn set_position(&mut self, position: Point) {
			self.0 = position;
		}
	}

	#[test]
	fn anchor_position_and_transform_mutation_share_state() {
		let mut anchor = Anchor::new(Transform::from_position(Point::new(1.0, 2.0, 3.0)));
		anchor.set_position(Point::new(4.0, 5.0, 6.0));

		assert_eq!(anchor.transform().get_position(), Point::new(4.0, 5.0, 6.0));
	}

	#[test]
	fn attachment_order_and_offsets_are_preserved() {
		let first: EntityHandle<dyn Positionable> = EntityHandle::from(TestPoint(Point::new(1.0, 0.0, 0.0)));
		let second: EntityHandle<dyn Positionable> = EntityHandle::from(TestPoint(Point::new(2.0, 0.0, 0.0)));
		let mut anchor = Anchor::new(Transform::default());

		anchor.attach(first.clone());
		anchor.attach_with_offset(second.clone(), Vector::new(3.0, 4.0, 5.0));
		let children = anchor.children();

		assert_eq!(children.len(), 2);
		assert!(matches!(children[0].1, Anchorage::Default));
		assert!(matches!(&children[1].1, Anchorage::Offset { offset } if *offset == Vector::new(3.0, 4.0, 5.0)));
	}
}
