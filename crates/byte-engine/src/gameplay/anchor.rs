//! An [`Anchor`] is a object that holds a transformation and can have other objects attached to it.

use crate::core::{entity::EntityBuilder, listener::EntitySubscriber, Entity, EntityHandle};

use crate::Vector3;

use super::{object::Object, Positionable, Transform};

#[derive(Debug, Clone)]
pub enum Anchorage {
	/// The object is attached to the anchor.
	Default,
	/// The anchorage is offset from the anchor.
	Offset {
		offset: Transform,
	},
}

impl Default for Anchorage {
	fn default() -> Self {
		Anchorage::Default
	}
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
		self.children.push((child, Anchorage::Offset { offset: Transform::from_position(offset) }));
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

	fn get_position(&self) -> Vector3 {
		self.transform.get_position()
	}
}

pub struct AnchorSystem {
	anchors: Vec<EntityHandle<Anchor>>,
}

impl Entity for AnchorSystem {}

impl AnchorSystem {
	pub fn new() -> EntityBuilder<'static, AnchorSystem> {
		EntityBuilder::new(AnchorSystem{ anchors: Vec::with_capacity(1024) }).listen_to::<Anchor>()
	}

	pub fn update(&self,) {
		for anchor in &self.anchors {
			let anchor = anchor.read();

			for (child, anchorage) in &anchor.children {
				let mut child = child.write();

				match anchorage {
					Anchorage::Default => {},
					Anchorage::Offset { offset } => {
						child.set_position(anchor.transform().get_position() + offset.get_position());
					},
				}
			}
		}
	}
}

impl EntitySubscriber<Anchor> for AnchorSystem {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<Anchor>, params: &'a Anchor) -> () {
		self.anchors.push(handle);
	}
}
