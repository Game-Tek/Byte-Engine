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
