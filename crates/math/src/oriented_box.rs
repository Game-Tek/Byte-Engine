use crate::{Orientation, Point, UnitVector, Vector, WorldSpace};

/// The `OrientedBox` struct represents a box that is turned any way in one coordinate space, for collision queries
/// against boxes that rotate.
///
/// Pass two to [`crate::collision::obb_vs_obb`] to find where they touch. A box that lies along the coordinate axes
/// is an [`crate::AABB`], which has quicker tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientedBox<Space = WorldSpace> {
	center: Point<Space>,
	axes: [UnitVector<Space>; 3],
	half_extents: [f32; 3],
}

impl<Space> OrientedBox<Space> {
	/// Creates a box around `center` whose sides are `half_extents` from it along each of the box's own axes, which
	/// are the coordinate axes turned by `orientation`.
	pub fn new(center: Point<Space>, half_extents: [f32; 3], orientation: Orientation) -> Self {
		let turned = |axis: UnitVector<Space>| {
			// Turning a direction keeps its length.
			UnitVector::unchecked(orientation.rotate_vector(axis.into_vector()).into_maths())
		};
		Self {
			center,
			axes: [UnitVector::x_axis(), UnitVector::y_axis(), UnitVector::z_axis()].map(turned),
			half_extents: half_extents.map(f32::abs),
		}
	}

	/// Returns the middle of the box.
	pub fn center(&self) -> Point<Space> {
		self.center
	}

	/// Returns the directions that the box's sides face: its own x, y, and z axes.
	pub fn axes(&self) -> [UnitVector<Space>; 3] {
		self.axes
	}

	/// Returns how far each pair of sides is from the center, along the axis in [`Self::axes`] with the same index.
	pub fn half_extents(&self) -> [f32; 3] {
		self.half_extents
	}

	/// Returns the middle of the part of the box that is furthest along `direction`: a corner, the middle of an
	/// edge when the direction is square to one of the box's axes, or the middle of a side when it is square to two.
	pub fn furthest_middle(&self, direction: Vector<Space>) -> Point<Space> {
		// A direction this close to square to an axis counts as square to it, so that a box lying on a side is not
		// answered with whichever corner rounding favors.
		const SQUARE: f32 = 1e-4;
		let length = direction.length();
		(0..3).fold(self.center, |point, axis| {
			let along = self.axes[axis].dot(direction);
			if along.abs() <= SQUARE * length {
				point
			} else {
				point + self.axes[axis] * self.half_extents[axis].copysign(along)
			}
		})
	}
}

#[cfg(test)]
mod tests {
	use super::OrientedBox;
	use crate::{Orientation, Point, Vector, WorldSpace};

	#[test]
	fn a_turned_box_reaches_furthest_with_the_part_it_turned_that_way() {
		// A box of 2 by 4 by 6, turned a quarter turn about z, so that its long y side lies along x.
		let quarter_turn =
			Orientation::try_from_rotation_vector(Vector::<WorldSpace>::new(0.0, 0.0, std::f32::consts::FRAC_PI_2)).unwrap();
		let turned: OrientedBox<WorldSpace> = OrientedBox::new(Point::new(10.0, 0.0, 0.0), [1.0, 2.0, 3.0], quarter_turn);
		let near = |point: Point<WorldSpace>, expected: [f32; 3]| (point - Point::from_array(expected)).length() <= 1e-5;

		// A corner, the middle of an edge, and the middle of a side.
		assert!(near(turned.furthest_middle(Vector::new(1.0, 1.0, 1.0)), [12.0, 1.0, 3.0]));
		assert!(near(turned.furthest_middle(Vector::new(-1.0, 0.0, 1.0)), [8.0, 0.0, 3.0]));
		assert!(near(turned.furthest_middle(Vector::new(0.0, -5.0, 0.0)), [10.0, -1.0, 0.0]));
		// Its own x-axis points along y now.
		assert!((turned.axes()[0].into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-6);
	}
}
