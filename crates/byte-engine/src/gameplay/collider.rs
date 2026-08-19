use math::{Point, Vector};

use crate::{
	core::Entity,
	physics::{
		collider::{Collider, Shapes},
		LocalSpace,
	},
	space::Positionable,
};

/// The `Sphere` struct provides a simple positioned spherical collider entity.
pub struct Sphere {
	radius: f32,
	position: Point,
}

/// The `Cube` struct provides a simple positioned box collider entity.
pub struct Cube {
	/// Local half-extents of the cube.
	size: Vector<LocalSpace>,
	position: Point,
}

impl Sphere {
	/// Creates a sphere at the world origin.
	pub fn new(radius: f32) -> Self {
		Self {
			radius,
			position: Point::origin(),
		}
	}
}

impl Cube {
	/// Creates a cube at the world origin with local half-extents.
	pub fn new(size: Vector<LocalSpace>) -> Self {
		Self {
			size,
			position: Point::origin(),
		}
	}
}

impl Entity for Sphere {}
impl Entity for Cube {}

impl Positionable for Sphere {
	fn position(&self) -> Point {
		self.position
	}
	fn set_position(&mut self, position: Point) {
		self.position = position;
	}
}

impl Positionable for Cube {
	fn position(&self) -> Point {
		self.position
	}
	fn set_position(&mut self, position: Point) {
		self.position = position;
	}
}

impl Collider for Sphere {
	fn shape(&self) -> Shapes {
		Shapes::Sphere { radius: self.radius }
	}
}

impl Collider for Cube {
	fn shape(&self) -> Shapes {
		Shapes::Cube { size: self.size }
	}
}

#[cfg(test)]
mod tests {
	use math::{Point, Vector};

	use super::{Cube, Sphere};
	use crate::{
		physics::{Collider, LocalSpace, Shapes},
		space::Positionable,
	};

	#[test]
	fn primitive_colliders_keep_world_positions_separate_from_local_extents() {
		let mut sphere = Sphere::new(2.5);
		let mut cube = Cube::new(Vector::<LocalSpace>::new(1.0, 2.0, 3.0));
		sphere.set_position(Point::new(4.0, 5.0, 6.0));
		cube.set_position(Point::new(-1.0, -2.0, -3.0));

		assert_eq!(sphere.position(), Point::new(4.0, 5.0, 6.0));
		assert_eq!(cube.position(), Point::new(-1.0, -2.0, -3.0));
		assert!(matches!(cube.shape(), Shapes::Cube { size } if size == Vector::new(1.0, 2.0, 3.0)));
	}
}
