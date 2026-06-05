pub struct Bounds {
	min: Vector3,
	max: Vector3,
}

impl Bounds {
	pub fn new(min: Vector3, max: Vector3) -> Self {
		Self { min, max }
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

use math::Vector3;
