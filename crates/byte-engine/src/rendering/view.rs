use math::{look_down, mat::{MatInverse as _, MatTranslate as _}, orthographic_matrix, plane::Plane, projection_matrix, Base as _, Matrix4, Vector3, Vector4};

use crate::gameplay::Transform;

/// A view represents a viewport into the world. It can be used to render a scene from a specific perspective.
/// It's used to represent cameras, lights, and other objects that can be used to render a scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
	projection: Matrix4,
	view: Matrix4,

	near: f32,
	far: f32,
	y_fov: f32,
	aspect_ratio: f32,
}

impl View {
	/// Creates a new view with the given parameters.
	pub fn new_perspective(fov: f32, aspect_ratio: f32, near: f32, far: f32, position: Vector3, rotation: Vector3) -> Self {
		Self {
			projection: projection_matrix(fov, aspect_ratio, near, far),
			view: look_down(rotation) * Matrix4::from_translation(-position),
			near,
			far,
			y_fov: fov,
			aspect_ratio,
		}
	}

	pub fn new_orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32, position: Vector3, rotation: Vector3) -> Self {
		Self {
			projection: orthographic_matrix(left, right, bottom, top, near, far),
			view: look_down(rotation) * Matrix4::from_translation(-position),
			near,
			far,
			y_fov: 0.0,
			aspect_ratio: 0.0,
		}
	}

	pub fn from_view(&self, view: Matrix4) -> Self {
		Self {
			projection: self.projection,
			view,
			near: self.near,
			far: self.far,
			y_fov: self.y_fov,
			aspect_ratio: self.aspect_ratio,
		}
	}

	/// Creates a new view with the same variables as the current view, but with the given near and far planes.
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

	/// Returns the projection matrix of the view.
	pub fn projection(&self) -> Matrix4 {
		self.projection
	}

	/// Returns the view matrix of the view.
	pub fn view(&self) -> Matrix4 {
		self.view
	}

	/// Returns the PV matrix of the view.
	pub fn projection_view(&self) -> Matrix4 {
		self.projection * self.view
	}

	/// Returns the PV matrix of the view.
	pub fn view_projection(&self) -> Matrix4 {
		self.projection * self.view
	}

	pub fn x_fov(&self) -> f32 {
		self.y_fov * self.aspect_ratio
	}

	pub fn y_fov(&self) -> f32 {
		self.y_fov
	}

	pub fn near(&self) -> f32 {
		self.near
	}

	pub fn far(&self) -> f32 {
		self.far
	}

	pub fn fov(&self) -> [f32; 2] {
		[self.x_fov(), self.y_fov()]
	}

	pub fn aspect_ratio(&self) -> f32 {
		self.aspect_ratio
	}

	/// Returns the frustum corners of the view, in world space.
	pub fn get_frustum_corners(&self) -> [Vector4; 8] {
		let inv = self.view_projection().inverse();

		let mut corners = [Vector4::zero(); 8];

		for i in 0..8 {
			let x = if i & 1 == 0 { -1.0 } else { 1.0 };
			let y = if i & 2 == 0 { -1.0 } else { 1.0 };
			let z = if i & 4 == 0 { 0f32 } else { 1f32 };

			let corner = inv * Vector4::new(x, y, z, 1.0);
			corners[i] = corner / corner.w;
		}

		return corners;
	}

	/// Returns the frustum planes of the view, in world space.
	pub fn get_frustum_planes(&self) -> [Plane; 6] {
		let pv = self.view_projection();

		let r0 = Vector4::new(pv[0], pv[1], pv[2], pv[3]); // Right
		let r1 = Vector4::new(pv[4], pv[5], pv[6], pv[7]); // Up
		let r2 = Vector4::new(pv[8], pv[9], pv[10], pv[11]); // Forward
		let r3 = Vector4::new(pv[12], pv[13], pv[14], pv[15]); // Clip space

		[
			Plane::new(Vector3::new(r3.x + r0.x, r3.y + r0.y, r3.z + r0.z), r3.w + r0.w), // Left
			Plane::new(Vector3::new(r3.x - r0.x, r3.y - r0.y, r3.z - r0.z), r3.w - r0.w), // Right
			Plane::new(Vector3::new(r3.x + r1.x, r3.y + r1.y, r3.z + r1.z), r3.w + r1.w), // Bottom
			Plane::new(Vector3::new(r3.x - r1.x, r3.y - r1.y, r3.z - r1.z), r3.w - r1.w), // Top
			Plane::new(Vector3::new(r3.x + r2.x, r3.y + r2.y, r3.z + r2.z), r3.w + r2.w), // Near
			Plane::new(Vector3::new(r3.x - r2.x, r3.y - r2.y, r3.z - r2.z), r3.w - r2.w), // Far
		]
	}
}

#[cfg(test)]
mod tests {
	use math::{assert_float_eq, assert_vec3f_near, VecN as _, Vector3};

	use super::*;

	#[test]
	fn test_view_frustum_corners() {
		let view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());

		let corners = view.get_frustum_corners();

		assert_eq!(corners[0], Vector4::new(-1.0, -1.0, -1.0, 1.0));
		assert_eq!(corners[1], Vector4::new(1.0, -1.0, -1.0, 1.0));
		assert_eq!(corners[2], Vector4::new(-1.0, 1.0, -1.0, 1.0));
		assert_eq!(corners[3], Vector4::new(1.0, 1.0, -1.0, 1.0));
		assert_eq!(corners[4], Vector4::new(-1.0, -1.0, 1.0, 1.0));
		assert_eq!(corners[5], Vector4::new(1.0, -1.0, 1.0, 1.0));
		assert_eq!(corners[6], Vector4::new(-1.0, 1.0, 1.0, 1.0));
		assert_eq!(corners[7], Vector4::new(1.0, 1.0, 1.0, 1.0));
	}

	#[test]
	fn test_orthographic_view_frustum_planes() {
		let view = View::new_orthographic(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());

		let planes = view.get_frustum_planes();

		assert_eq!(planes[0].normal, Vector3::new(1.0, 0.0, 0.0)); // Left
		assert_eq!(planes[1].normal, Vector3::new(-1.0, 0.0, 0.0)); // Right
		assert_eq!(planes[2].normal, Vector3::new(0.0, 1.0, 0.0)); // Bottom
		assert_eq!(planes[3].normal, Vector3::new(0.0, -1.0, 0.0)); // Top
		assert_eq!(planes[4].normal, Vector3::new(0.0, 0.0, -1.0)); // Near
		assert_eq!(planes[5].normal, Vector3::new(0.0, 0.0, 1.0)); // Far
	}

	#[test]
	fn test_perspective_view_frustum_planes() {
		let view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());

		let planes = view.get_frustum_planes();

		assert_vec3f_near!(planes[0].normal, Vector3::new(0.707, 0.0, 0.707)); // Left
		assert_vec3f_near!(planes[1].normal, Vector3::new(-0.707, 0.0, 0.707)); // Right
		assert_vec3f_near!(planes[2].normal, Vector3::new(0.0, 0.707, 0.707)); // Bottom
		assert_vec3f_near!(planes[3].normal, Vector3::new(0.0, -0.707, 0.707)); // Top
		assert_vec3f_near!(planes[4].normal, Vector3::new(0.0, 0.0, 1.0)); // Near
		assert_vec3f_near!(planes[5].normal, Vector3::new(0.0, 0.0, 1.0)); // Far

		assert_float_eq!(planes[0].distance, 0.0, "Left plane distance");
		assert_float_eq!(planes[1].distance, 0.0, "Right plane distance");
		assert_float_eq!(planes[2].distance, 0.0, "Bottom plane distance");
		assert_float_eq!(planes[3].distance, 0.0, "Top plane distance");
		assert_float_eq!(planes[4].distance, 0.1, "Near plane distance");
		assert_float_eq!(planes[5].distance, -0.1, "Far plane distance");
	}
}
