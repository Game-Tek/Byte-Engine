//! Mesh component module

use core::{entity::EntityBuilder, listener::Listener};

use maths_rs::{mat::{MatRotate3D, MatScale, MatTranslate}, normalize};

use crate::{core::{orchestrator, Entity}, math};

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> maths_rs::Mat4f;
	fn get_resource_id(&self) -> &'static str;
}

pub struct Transform {
	position: maths_rs::Vec3f,
	scale: maths_rs::Vec3f,
	rotation: maths_rs::Vec3f,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: maths_rs::Vec3f::new(0.0, 0.0, 0.0),
			scale: maths_rs::Vec3f::new(1.0, 1.0, 1.0),
			rotation: maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		}
	}
}

impl Transform {
	pub fn identity() -> Self {
		Self {
			position: maths_rs::Vec3f::new(0.0, 0.0, 0.0),
			scale: maths_rs::Vec3f::new(1.0, 1.0, 1.0),
			rotation: maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		}
	}

	pub fn new(position: maths_rs::Vec3f, scale: maths_rs::Vec3f, rotation: maths_rs::Vec3f) -> Self {
		Self {
			position,
			scale,
			rotation,
		}
	}

	pub fn position(self, position: maths_rs::Vec3f) -> Self {
		Self {
			position,
			..self
		}
	}

	pub fn scale(self, scale: maths_rs::Vec3f) -> Self {
		Self {
			scale,
			..self
		}
	}

	pub fn rotation(self, rotation: maths_rs::Vec3f) -> Self {
		Self {
			rotation,
			..self
		}
	}

	fn from_position(position: maths_rs::Vec3f) -> Self {
		Self {
			position,
			scale: maths_rs::Vec3f::new(1.0, 1.0, 1.0),
			rotation: maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		}
	}

	fn from_translation(position: maths_rs::Vec3f) -> Self {
		Self {
			position,
			scale: maths_rs::Vec3f::new(1.0, 1.0, 1.0),
			rotation: maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		}
	}

	fn from_scale(scale: maths_rs::Vec3f) -> Self {
		Self {
			position: maths_rs::Vec3f::new(0.0, 0.0, 0.0),
			scale,
			rotation: maths_rs::Vec3f::new(0.0, 0.0, 1.0),
		}
	}

	fn from_rotation(rotation: maths_rs::Vec3f) -> Self {
		Self {
			position: maths_rs::Vec3f::new(0.0, 0.0, 0.0),
			scale: maths_rs::Vec3f::new(1.0, 1.0, 1.0),
			rotation,
		}
	}

	fn get_transform(&self) -> maths_rs::Mat4f {
		maths_rs::Mat4f::from_translation(self.position) * math::from_normal(self.rotation) * maths_rs::Mat4f::from_scale(self.scale)
	}

	fn set_orientation(&mut self, orientation: maths_rs::Vec3f) {
		self.rotation = orientation;
	}
}

pub struct Mesh {
	resource_id: &'static str,
	transform: Transform,
}

impl Entity for Mesh {
	fn call_listeners(&self, listener: &core::listener::BasicListener, handle: core::EntityHandle<Self>) where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		listener.invoke_for(handle.clone() as core::EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
	}
}

impl RenderEntity for Mesh {
	fn get_transform(&self) -> maths_rs::Mat4f { self.transform.get_transform() }
	fn get_resource_id(&self) -> &'static str { self.resource_id }
}

impl Mesh {
	pub fn new(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			resource_id,
			transform,
		}.into()
	}

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }

	pub fn set_orientation(&mut self, orientation: maths_rs::Vec3f) {
		self.transform.set_orientation(normalize(orientation));
	}
}