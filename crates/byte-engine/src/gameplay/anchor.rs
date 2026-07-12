//! An [`Anchor`] is a object that holds a transformation and can have other objects attached to it.

use math::Vector3;

use super::transform::Transform;
use crate::core::listener::Listener;
use crate::core::{Entity, EntityHandle};
use crate::space::Positionable;

#[derive(Debug, Clone, Default)]
pub enum Anchorage {
	/// The object is attached to the anchor.
	#[default]
	Default,
	/// The anchorage is offset from the anchor.
	Offset { offset: Transform },
}

pub trait Anchoring: Positionable {
	fn children(&self) -> Vec<(EntityHandle<dyn Positionable>, Anchorage)>;
}

pub struct Anchor {
	transform: Transform,
	children: Vec<(EntityHandle<dyn Positionable>, Anchorage)>,
}

impl Entity for Anchor {}

impl Anchor {
	pub fn new(transform: Transform) -> Self {
		Self {
			transform,
			children: Vec::with_capacity(8),
		}
	}

	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	pub fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}

	/// Attaches a child to the anchor.
	pub fn attach(&mut self, child: EntityHandle<dyn Positionable>) {
		self.children.push((child, Default::default()));
	}

	/// Attaches a child to the anchor.
	pub fn attach_with_offset(&mut self, child: EntityHandle<dyn Positionable>, offset: Vector3) {
		self.children.push((
			child,
			Anchorage::Offset {
				offset: Transform::from_position(offset),
			},
		));
	}

	/// Attaches a child to the anchor.
	pub fn attach_with_anchorage(&mut self, child: EntityHandle<dyn Positionable>, anchorage: Anchorage) {
		self.children.push((child, anchorage));
	}
}

impl Positionable for Anchor {
	fn set_position(&mut self, position: Vector3) {
		self.transform.set_position(position);
	}

	fn position(&self) -> Vector3 {
		self.transform.get_position()
	}
}

impl Anchoring for Anchor {
	fn children(&self) -> Vec<(EntityHandle<dyn Positionable>, Anchorage)> {
		self.children.clone()
	}
}

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
	pub fn new() -> AnchorSystem {
		AnchorSystem {
			anchors: Vec::with_capacity(1024),
		}
	}

	pub fn update(&self) {
		for anchor in &self.anchors {
			let children = anchor.children();

			for (child, anchorage) in children {
				match anchorage {
					Anchorage::Default => {
						// child.set_position(anchor.position());
					}
					Anchorage::Offset { offset } => {
						// child.set_position(anchor.position() + offset.get_position());
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::{Anchor, Anchorage, Anchoring};
	use crate::{core::EntityHandle, gameplay::Transform, space::Positionable};

	struct Point(Vector3);

	impl Positionable for Point {
		fn position(&self) -> Vector3 {
			self.0
		}

		fn set_position(&mut self, position: Vector3) {
			self.0 = position;
		}
	}

	#[test]
	fn anchor_position_and_transform_mutation_share_the_same_state() {
		let mut anchor = Anchor::new(Transform::from_position(Vector3::new(1.0, 2.0, 3.0)));
		assert_eq!(anchor.position(), Vector3::new(1.0, 2.0, 3.0));

		anchor.set_position(Vector3::new(4.0, 5.0, 6.0));
		assert_eq!(anchor.transform().get_position(), Vector3::new(4.0, 5.0, 6.0));
		anchor.transform_mut().set_position(Vector3::new(7.0, 8.0, 9.0));
		assert_eq!(anchor.position(), Vector3::new(7.0, 8.0, 9.0));
	}

	#[test]
	fn attachment_order_identity_and_offsets_are_preserved() {
		let first_concrete = EntityHandle::from(Point(Vector3::new(1.0, 0.0, 0.0)));
		let second_concrete = EntityHandle::from(Point(Vector3::new(2.0, 0.0, 0.0)));
		let first: EntityHandle<dyn Positionable> = first_concrete.clone();
		let second: EntityHandle<dyn Positionable> = second_concrete.clone();
		let mut anchor = Anchor::new(Transform::default());

		anchor.attach(first.clone());
		anchor.attach_with_offset(second.clone(), Vector3::new(3.0, 4.0, 5.0));
		let children = anchor.children();

		assert_eq!(children.len(), 2);
		assert!(children[0].0 == first);
		assert!(matches!(children[0].1, Anchorage::Default));
		assert!(children[1].0 == second);
		assert!(matches!(
			&children[1].1,
			Anchorage::Offset { offset } if offset.get_position() == Vector3::new(3.0, 4.0, 5.0)
		));
	}
}
