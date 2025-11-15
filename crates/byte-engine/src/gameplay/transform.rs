use math::{Matrix4, Quaternion, Vector3, Vector4, mat::{MatScale as _, MatTranslate as _}};

#[derive(Debug, Clone)]
pub struct Transform {
	position: Vector3,
	scale: Vector3,
	rotation: Quaternion,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}
}

impl Transform {
	pub fn identity() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	pub fn new(position: Vector3, scale: Vector3, rotation: Quaternion) -> Self {
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

	pub fn rotation(self, rotation: Quaternion) -> Self {
		Self {
			rotation,
			..self
		}
	}

	pub fn from_position(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	fn from_translation(position: Vector3) -> Self {
		Self {
			position,
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation: Quaternion::identity(),
		}
	}

	fn from_scale(scale: Vector3) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale,
			rotation: Quaternion::identity(),
		}
	}

	fn from_rotation(rotation: Quaternion) -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			scale: Vector3::new(1.0, 1.0, 1.0),
			rotation,
		}
	}

	pub fn get_matrix(&self) -> Matrix4 {
		let rotation = self.rotation.get_matrix();
		let x = Vector4::from((rotation.get_row(0), 0.0));
		let y = Vector4::from((rotation.get_row(1), 0.0));
		let z = Vector4::from((rotation.get_row(2), 0.0));
		Matrix4::from_translation(self.position) * Matrix4::from((x, y, z, Vector4::new(0.0, 0.0, 0.0, 1.0))) * Matrix4::from_scale(self.scale)
	}

	pub fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
	pub fn get_position(&self) -> Vector3 { self.position }

	pub fn set_scale(&mut self, scale: Vector3) {
		self.scale = scale;
	}
	pub fn get_scale(&self) -> Vector3 { self.scale }

	pub fn set_orientation(&mut self, orientation: Quaternion) {
		self.rotation = orientation;
	}
	pub fn get_orientation(&self) -> Quaternion { self.rotation }
}

impl From<&Transform> for Matrix4 {
	fn from(transform: &Transform) -> Self {
		transform.get_matrix()
	}
}
