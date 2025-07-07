pub mod space;
pub mod object;
pub mod positionable;
pub mod transformable;
pub mod anchor;
pub mod collider;
pub mod killer;
pub mod timer;

pub use anchor::Anchor;
use math::{mat::{MatScale, MatTranslate}, Matrix4, Vector3};
pub use object::Object;
pub use positionable::Positionable;
pub use transformable::Transformable;

#[derive(Debug, Clone)]
pub struct Transform {
	position: Vector3,
	scale: Vector3,
	rotation: Vector3,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Vector3::new(0.0, 0.0, 1.0),
		}
	}
}

impl Transform {
	pub fn identity() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Vector3::new(0.0, 0.0, 1.0),
		}
	}

	pub fn new(position: Vector3, scale: Vector3, rotation: Vector3) -> Self {
		Self {
			position,
			scale,
			rotation,
		}
	}

	pub fn position(self, position: Vector3) -> Self {
		Self {
			position,
			..self
		}
	}

	pub fn scale(self, scale: Vector3) -> Self {
		Self {
			scale,
			..self
		}
	}

	pub fn rotation(self, rotation: Vector3) -> Self {
		Self {
			rotation,
			..self
		}
	}

	pub fn from_position(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Vector3::new(0.0, 0.0, 1.0),
		}
	}

	fn from_translation(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Vector3::new(0.0, 0.0, 1.0),
		}
	}

	fn from_scale(scale: Vector3) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale,
			rotation: Vector3::new(0.0, 0.0, 1.0),
		}
	}

	fn from_rotation(rotation: Vector3) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation,
		}
	}

	pub fn get_matrix(&self) -> Matrix4 {
		Matrix4::from_translation(self.position) * math::from_normal(self.rotation) * Matrix4::from_scale(self.scale)
	}

	pub fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
	pub fn get_position(&self) -> Vector3 { self.position }

	pub fn set_scale(&mut self, scale: Vector3) {
		self.scale = scale;
	}
	pub fn get_scale(&self) -> Vector3 { self.scale }

	pub fn set_orientation(&mut self, orientation: Vector3) {
		self.rotation = orientation;
	}
	pub fn get_orientation(&self) -> Vector3 { self.rotation }
}

impl From<&Transform> for Matrix4 {
	fn from(transform: &Transform) -> Self {
		transform.get_matrix()
	}
}
