#[derive(Debug, Clone, Copy)]
pub struct Bounds {
	min: Vector3,
	max: Vector3,
}

impl Bounds {
	pub fn zero() -> Self {
		Self::new(Vector3::zero(), Vector3::zero())
	}

	pub fn new(min: Vector3, max: Vector3) -> Self {
		Self {
			min: Vector3::new(min.x.min(max.x), min.y.min(max.y), min.z.min(max.z)),
			max: Vector3::new(min.x.max(max.x), min.y.max(max.y), min.z.max(max.z)),
		}
	}

	pub fn intersects(&self, other: &Bounds) -> bool {
		self.max.x >= other.min.x
			&& self.min.x <= other.max.x
			&& self.max.y >= other.min.y
			&& self.min.y <= other.max.y
			&& self.max.z >= other.min.z
			&& self.min.z <= other.max.z
	}

	pub fn expand_to_fit_point(&mut self, point: Vector3) {
		self.min.x = self.min.x.min(point.x);
		self.min.y = self.min.y.min(point.y);
		self.min.z = self.min.z.min(point.z);
		self.max.x = self.max.x.max(point.x);
		self.max.y = self.max.y.max(point.y);
		self.max.z = self.max.z.max(point.z);
	}

	pub fn expand_to_fit_points(&mut self, points: &[Vector3]) {
		for point in points {
			self.expand_to_fit_point(*point);
		}
	}

	pub fn expand(&mut self, other: &Bounds) {
		self.min.x = self.min.x.min(other.min.x);
		self.min.y = self.min.y.min(other.min.y);
		self.min.z = self.min.z.min(other.min.z);
		self.max.x = self.max.x.max(other.max.x);
		self.max.y = self.max.y.max(other.max.y);
		self.max.z = self.max.z.max(other.max.z);
	}

	pub fn translated(&self, offset: Vector3) -> Self {
		Self::new(self.min + offset, self.max + offset)
	}

	pub fn size(&self) -> Vector3 {
		self.max - self.min
	}

	pub fn expanded_by(&self, other: Vector3) -> Self {
		Self::new(self.min - other, self.max + other)
	}

	pub(crate) fn min(&self) -> Vector3 {
		self.min
	}

	pub(crate) fn max(&self) -> Vector3 {
		self.max
	}
}

impl Add<Vector3> for Bounds {
	type Output = Self;

	fn add(self, other: Vector3) -> Self {
		self.translated(other)
	}
}

use std::ops::Add;

use math::{Base as _, Vector3};

#[cfg(test)]
mod tests {
	use math::assert_vec3f_near;

	use super::*;

	#[test]
	fn new_normalizes_inverted_bounds() {
		let bounds = Bounds::new(Vector3::new(2.0, -1.0, 4.0), Vector3::new(-3.0, 5.0, 1.0));

		assert_vec3f_near!(bounds.min(), Vector3::new(-3.0, -1.0, 1.0));
		assert_vec3f_near!(bounds.max(), Vector3::new(2.0, 5.0, 4.0));
	}

	#[test]
	fn intersects_handles_touching_and_inverted_inputs() {
		let a = Bounds::new(Vector3::new(1.0, 1.0, 1.0), Vector3::new(-1.0, -1.0, -1.0));
		let touching = Bounds::new(Vector3::new(1.0, -0.5, -0.5), Vector3::new(2.0, 0.5, 0.5));
		let separate = Bounds::new(Vector3::new(2.1, 0.0, 0.0), Vector3::new(3.0, 1.0, 1.0));

		assert!(a.intersects(&touching));
		assert!(!a.intersects(&separate));
	}

	#[test]
	fn expand_to_fit_points_tolerates_empty_input() {
		let mut bounds = Bounds::new(Vector3::new(1.0, 2.0, 3.0), Vector3::new(1.0, 2.0, 3.0));
		bounds.expand_to_fit_points(&[]);

		assert_vec3f_near!(bounds.min(), Vector3::new(1.0, 2.0, 3.0));
		assert_vec3f_near!(bounds.max(), Vector3::new(1.0, 2.0, 3.0));
	}
}
