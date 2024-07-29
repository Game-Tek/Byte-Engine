//! An [`Anchor`] is a object that holds a transformation and can have other objects attached to it.

use core::{Entity, EntityHandle};

use super::{object::Object, Positionable, Transform};

pub struct Anchor {
	transform: Transform,
	children: Vec<EntityHandle<dyn Positionable>>,
}

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

	pub fn add_child(&mut self, child: EntityHandle<dyn Positionable>) {
		self.children.push(child);
	}
}