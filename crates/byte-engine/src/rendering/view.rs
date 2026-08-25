use math::{Degrees, Matrix, Plane, Point, UnitVector, Vector, WorldSpace, inverse, orthographic_matrix, projection_matrix};
use maths_rs::{Vec3f, Vec4f, mat::MatTranslate as _};

use crate::gameplay::transform::Transform;

/// The `View` struct provides projection and orientation data shared by
/// cameras, lights, render sinks, and shader setup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
	projection: Matrix,
	view: Matrix,

	near: f32,
	far: f32,
	y_fov: Degrees,
	aspect_ratio: f32,
}

impl View {
	/// Creates a perspective view for camera-style scene rendering.
	pub fn new_perspective(
		fov: Degrees,
		aspect_ratio: f32,
		near: f32,
		far: f32,
		position: Point,
		direction: UnitVector,
	) -> Self {
		Self {
			projection: projection_matrix(fov, aspect_ratio, near, far),
			view: world_view_matrix(position, direction),
			near,
			far,
			y_fov: fov,
			aspect_ratio,
		}
	}

	/// Creates a perspective view with an explicit up direction when a stable image orientation matters.
	pub fn new_perspective_with_up(
		fov: Degrees,
		aspect_ratio: f32,
		near: f32,
		far: f32,
		position: Point,
		direction: UnitVector,
		up: UnitVector,
	) -> Self {
		assert!(
			direction.dot(up.into_vector()).abs() < 0.99,
			"Perspective view up direction is invalid. The most likely cause is that the up direction is parallel to the view direction."
		);
		Self {
			projection: projection_matrix(fov, aspect_ratio, near, far),
			view: world_view_matrix_with_up(position, direction, up),
			near,
			far,
			y_fov: fov,
			aspect_ratio,
		}
	}

	/// Creates an orthographic view for light, editor, or flat scene rendering.
	pub fn new_orthographic(
		left: f32,
		right: f32,
		bottom: f32,
		top: f32,
		near: f32,
		far: f32,
		position: Point,
		direction: UnitVector,
	) -> Self {
		Self {
			projection: orthographic_matrix(left, right, bottom, top, near, far),
			view: world_view_matrix(position, direction),
			near,
			far,
			y_fov: Degrees::new(0.0),
			aspect_ratio: 0.0,
		}
	}

	/// Creates a view that shares this projection but uses a caller-provided view matrix.
	pub fn from_view(&self, view: Matrix) -> Self {
		Self {
			projection: self.projection,
			view,
			near: self.near,
			far: self.far,
			y_fov: self.y_fov,
			aspect_ratio: self.aspect_ratio,
		}
	}

	/// Creates a perspective view with this view's settings and new clipping planes.
	pub fn from_from_z_planes(&self, near: f32, far: f32) -> Self {
		Self {
			projection: projection_matrix(self.y_fov, self.aspect_ratio, near, far),
			view: self.view,
			near,
			far,
			y_fov: self.y_fov,
			aspect_ratio: self.aspect_ratio,
		}
	}

	/// Returns the projection matrix.
	pub fn projection(&self) -> Matrix {
		self.projection
	}

	/// Returns the view matrix.
	pub fn view(&self) -> Matrix {
		self.view
	}

	/// Returns the projection matrix multiplied by the view matrix.
	pub fn projection_view(&self) -> Matrix {
		self.projection * self.view
	}

	/// Returns the projection matrix multiplied by the view matrix.
	pub fn view_projection(&self) -> Matrix {
		self.projection * self.view
	}

	/// Returns the horizontal field of view derived from the vertical field of view.
	pub fn x_fov(&self) -> Degrees {
		Degrees::new(self.y_fov.value() * self.aspect_ratio)
	}

	/// Returns the vertical field of view used by perspective projections.
	pub fn y_fov(&self) -> Degrees {
		self.y_fov
	}

	/// Returns the near clipping plane distance.
	pub fn near(&self) -> f32 {
		self.near
	}

	/// Returns the far clipping plane distance.
	pub fn far(&self) -> f32 {
		self.far
	}

	/// Returns the horizontal and vertical fields of view.
	pub fn fov(&self) -> [f32; 2] {
		[self.x_fov().value(), self.y_fov().value()]
	}

	/// Returns the width-to-height aspect ratio used by perspective projections.
	pub fn aspect_ratio(&self) -> f32 {
		self.aspect_ratio
	}

	/// Returns the view-frustum corners as world-space points.
	pub fn get_frustum_corners(&self) -> [Point; 8] {
		let inverse_view_projection = inverse(self.view_projection());
		let mut corners = [Point::origin(); 8];

		for (index, corner) in corners.iter_mut().enumerate() {
			let x = if index & 1 == 0 { -1.0 } else { 1.0 };
			let y = if index & 2 == 0 { -1.0 } else { 1.0 };
			let z = if index & 4 == 0 { 0.0 } else { 1.0 };
			let homogeneous_corner = inverse_view_projection * Vec4f::new(x, y, z, 1.0);

			// Perspective division converts the explicit clip-space boundary back into a world point.
			*corner = Point::from_maths(Vec3f::new(
				homogeneous_corner.x / homogeneous_corner.w,
				homogeneous_corner.y / homogeneous_corner.w,
				homogeneous_corner.z / homogeneous_corner.w,
			));
		}

		corners
	}

	/// Returns the view-frustum planes in world space.
	pub fn get_frustum_planes(&self) -> [Plane; 6] {
		let pv = self.view_projection();

		let r0 = Vec4f::new(pv[0], pv[1], pv[2], pv[3]); // Right
		let r1 = Vec4f::new(pv[4], pv[5], pv[6], pv[7]); // Up
		let r2 = Vec4f::new(pv[8], pv[9], pv[10], pv[11]); // Forward
		let r3 = Vec4f::new(pv[12], pv[13], pv[14], pv[15]); // Clip space

		[
			plane_from_coefficients(Vec4f::new(r3.x + r0.x, r3.y + r0.y, r3.z + r0.z, r3.w + r0.w)), // Left
			plane_from_coefficients(Vec4f::new(r3.x - r0.x, r3.y - r0.y, r3.z - r0.z, r3.w - r0.w)), // Right
			plane_from_coefficients(Vec4f::new(r3.x + r1.x, r3.y + r1.y, r3.z + r1.z, r3.w + r1.w)), // Bottom
			plane_from_coefficients(Vec4f::new(r3.x - r1.x, r3.y - r1.y, r3.z - r1.z, r3.w - r1.w)), // Top
			plane_from_coefficients(Vec4f::new(r3.x + r2.x, r3.y + r2.y, r3.z + r2.z, r3.w + r2.w)), // Near
			plane_from_coefficients(Vec4f::new(r3.x - r2.x, r3.y - r2.y, r3.z - r2.z, r3.w - r2.w)), // Far
		]
	}
}

/// Builds a view matrix from branded world-space camera state at the matrix boundary.
fn world_view_matrix(position: Point, direction: UnitVector) -> Matrix {
	let up = UnitVector::<WorldSpace>::y_axis();
	let vertical = direction.dot(up.into_vector()).abs() > 0.99;
	let reference = if vertical { UnitVector::<WorldSpace>::z_axis() } else { up };
	// `UnitVector` excludes zero and non-finite directions, and this reference is selected to be non-colinear with it.
	let x_basis = maths_rs::normalize(maths_rs::cross(reference.into_maths(), direction.into_maths()));
	let y_basis = maths_rs::normalize(if vertical {
		maths_rs::cross(x_basis, direction.into_maths())
	} else {
		maths_rs::cross(direction.into_maths(), x_basis)
	});
	let orientation = Matrix::from((
		Vec4f::from((x_basis, 0.0)),
		Vec4f::from((y_basis, 0.0)),
		Vec4f::from((direction.into_maths(), 0.0)),
		Vec4f::new(0.0, 0.0, 0.0, 1.0),
	));

	orientation * Matrix::from_translation(-position.into_maths())
}

/// Builds a view matrix from explicit, non-colinear forward and up directions.
fn world_view_matrix_with_up(position: Point, direction: UnitVector, up: UnitVector) -> Matrix {
	let x_basis = maths_rs::normalize(maths_rs::cross(up.into_maths(), direction.into_maths()));
	let y_basis = maths_rs::normalize(maths_rs::cross(direction.into_maths(), x_basis));
	let orientation = Matrix::from((
		Vec4f::from((x_basis, 0.0)),
		Vec4f::from((y_basis, 0.0)),
		Vec4f::from((direction.into_maths(), 0.0)),
		Vec4f::new(0.0, 0.0, 0.0, 1.0),
	));

	orientation * Matrix::from_translation(-position.into_maths())
}

/// Converts an extracted homogeneous plane into the unit-normal representation required by [`Plane`].
fn plane_from_coefficients(coefficients: Vec4f) -> Plane {
	let unnormalized_normal = Vector::<WorldSpace>::from_maths(Vec3f::new(coefficients.x, coefficients.y, coefficients.z));
	let (normal, length) = unnormalized_normal
		.normalize_with_length()
		.expect("Frustum plane normal is invalid. The most likely cause is a non-invertible view-projection matrix.");
	Plane::new(normal, coefficients.w / length)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn assert_point_near(actual: Point, expected: Point) {
		assert!((actual.x() - expected.x()).abs() < 0.001);
		assert!((actual.y() - expected.y()).abs() < 0.001);
		assert!((actual.z() - expected.z()).abs() < 0.001);
	}

	#[test]
	fn perspective_view_returns_world_space_frustum_points() {
		let view = View::new_perspective(Degrees::new(90.0), 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let corners = view.get_frustum_corners();

		assert_point_near(corners[0], Point::new(-100.0, -100.0, 100.0));
		assert_point_near(corners[7], Point::new(0.1, 0.1, 0.1));
	}

	#[test]
	fn orthographic_view_returns_world_space_frustum_points() {
		let view = View::new_orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let corners = view.get_frustum_corners();

		assert_point_near(corners[0], Point::new(-1.0, -1.0, 100.0));
		assert_point_near(corners[7], Point::new(1.0, 1.0, 0.1));
	}

	#[test]
	fn frustum_planes_keep_checked_world_space_normals() {
		let view = View::new_perspective(Degrees::new(90.0), 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let planes = view.get_frustum_planes();

		assert!((planes[0].normal().x() - 0.707).abs() < 0.001);
		assert!((planes[0].normal().z() - 0.707).abs() < 0.001);
		assert!((planes[4].distance() - 0.1).abs() < 0.001);
	}
}
