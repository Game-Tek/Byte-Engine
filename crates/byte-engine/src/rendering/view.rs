use maths_rs::{mat::{MatInverse, MatProjection, MatTranslate}, num::Base, Mat4f, Vec3f, Vec4f};

use crate::{gameplay::Transform, math::{self, projection_matrix}};

/// A view represents a viewport into the world. It can be used to render a scene from a specific perspective.
/// It's used to represent cameras, lights, and other objects that can be used to render a scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
	projection: Mat4f,
	view: Mat4f,

	near: f32,
	far: f32,
	y_fov: f32,
	aspect_ratio: f32,
}

impl View {
	/// Creates a new view with the given parameters.
	pub fn new_perspective(fov: f32, aspect_ratio: f32, near: f32, far: f32, position: Vec3f, rotation: Vec3f) -> Self {
		Self {
			projection: projection_matrix(fov, aspect_ratio, near, far),
			view: math::look_down(rotation) * maths_rs::Mat4f::from_translation(-position),
			near,
			far,
			y_fov: fov,
			aspect_ratio,
		}
	}

	pub fn new_orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32, rotation: Vec3f) -> Self {
		Self {
			projection: Mat4f::create_ortho_matrix(left, right, bottom, top, near, far),
			view: math::look_down(rotation),
			near,
			far,
			y_fov: 0.0,
			aspect_ratio: 0.0,
		}
	}

	pub fn from_view(&self, view: Mat4f) -> Self {
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
	pub fn projection(&self) -> Mat4f {
		self.projection
	}

	/// Returns the view matrix of the view.
	pub fn view(&self) -> Mat4f {
		self.view
	}

	/// Returns the PV matrix of the view.
	pub fn projection_view(&self) -> Mat4f {
		self.projection * self.view
	}

	/// Returns the PV matrix of the view.
	pub fn view_projection(&self) -> Mat4f {
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
	pub fn get_frustum_corners(&self) -> [Vec4f; 8] {
		let inv = (self.projection * self.view).inverse();
    
		let mut corners = [Vec4f::zero(); 8];

		for i in 0..2 {
			let x = i as f32;
			for j in 0..2 {
				let y = j as f32;
				for k in 0..2 {
					let z = k as f32;
					let pt = inv * Vec4f::new(2.0f32 * x - 1.0f32, 2.0f32 * y - 1.0f32, 2.0f32 * z - 1.0f32, 1.0f32);
					corners[i + j * 2 + k * 4] = pt / pt.w;
				}
			}
		}
		
		return corners;
	}

	/// Returns the frustum planes of the view, in world space.
	pub fn get_frustum_planes(&self) -> [Vec4f; 6] {
		let corners = self.get_frustum_corners();

		let mut planes = [Vec4f::zero(); 6];

		planes[0] = maths_rs::normalize(Vec4f::new(corners[0].x, corners[1].x, corners[2].x, corners[3].x));
		planes[1] = maths_rs::normalize(Vec4f::new(corners[0].y, corners[1].y, corners[4].y, corners[5].y));
		planes[2] = maths_rs::normalize(Vec4f::new(corners[0].z, corners[2].z, corners[4].z, corners[6].z));
		planes[3] = maths_rs::normalize(Vec4f::new(corners[7].x, corners[6].x, corners[5].x, corners[4].x));
		planes[4] = maths_rs::normalize(Vec4f::new(corners[7].y, corners[3].y, corners[2].y, corners[6].y));
		planes[5] = maths_rs::normalize(Vec4f::new(corners[7].z, corners[5].z, corners[3].z, corners[1].z));

		return planes;
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::{mat::MatNew4, vec::{Vec3, VecN}};

	use super::*;

	#[test]
	fn test_view_frustum_corners() {
		let view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vec3f::zero(), Vec3f::unit_z());

		let corners = view.get_frustum_corners();

		assert_eq!(corners[0], Vec4f::new(-1.0, -1.0, -1.0, 1.0));
		assert_eq!(corners[1], Vec4f::new(1.0, -1.0, -1.0, 1.0));
		assert_eq!(corners[2], Vec4f::new(-1.0, 1.0, -1.0, 1.0));
		assert_eq!(corners[3], Vec4f::new(1.0, 1.0, -1.0, 1.0));
		assert_eq!(corners[4], Vec4f::new(-1.0, -1.0, 1.0, 1.0));
		assert_eq!(corners[5], Vec4f::new(1.0, -1.0, 1.0, 1.0));
		assert_eq!(corners[6], Vec4f::new(-1.0, 1.0, 1.0, 1.0));
		assert_eq!(corners[7], Vec4f::new(1.0, 1.0, 1.0, 1.0));
	}
}