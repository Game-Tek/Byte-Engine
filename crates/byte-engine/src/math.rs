use maths_rs::mat::MatNew4;

use crate::Vector3;

/// Calculates the direction to move in a plane from a direction(absolute) vector and a head/camera relative direction vector
pub fn plane_navigation(direction: Vector3, command: Vector3) -> Vector3 {
	Vector3::new(direction.x, 0.0, direction.z) * command.z + Vector3::new(direction.z, 0.0, -direction.x) * command.x
}

pub fn look_at(direction: crate::Vector3) -> maths_rs::Mat4f {
	let x_axis = maths_rs::normalize(maths_rs::cross(crate::Vector3::new(0f32, 1f32, 0f32), maths_rs::normalize(direction)));
	let y_axis = maths_rs::normalize(maths_rs::cross(maths_rs::normalize(direction), x_axis));

	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((x_axis, 0f32)),
		maths_rs::Vec4f::from((y_axis, 0f32)),
		maths_rs::Vec4f::from((direction, 0f32)),
		maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
	))
}

/// Calculates a left handed perspective projection matrix for 0 to 1 depth range
/// 
/// # Arguments
/// 
/// * `fov` - Full vertical field of view in degrees
/// * `aspect_ratio` - Aspect ratio of the screen
/// * `near_plane` - Distance to the near plane
/// * `far_plane` - Distance to the far plane
pub fn projection_matrix(fov: f32, aspect_ratio: f32, near_plane: f32, far_plane: f32) -> maths_rs::Mat4f {
	let h = 1f32 / (fov / 2f32).to_radians().tan();
	let w = h / aspect_ratio;

	let far_minus_near = far_plane - near_plane;

	let a = -near_plane / far_minus_near;
	let b = (near_plane * far_plane) / far_minus_near;

	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((w,		0f32, 		0f32, 		0f32)),
		maths_rs::Vec4f::from((0f32, 	-h,			0f32, 		0f32)),
		maths_rs::Vec4f::from((0f32, 	0f32, 		a, 			b	)),
		maths_rs::Vec4f::from((0f32,	0f32, 		1f32,		0f32)),
	))
}

pub fn orthographic_matrix(width: f32, height: f32, near_plane: f32, far_plane: f32) -> maths_rs::Mat4f {
	let near_minus_far = near_plane - far_plane;
	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((2f32 / width, 	0f32, 			0f32,					0f32					   )),
		maths_rs::Vec4f::from((0f32, 			2f32 / height,	0f32,					0f32					   )),
		maths_rs::Vec4f::from((0f32, 			0f32, 			1f32 / near_minus_far,  near_plane / near_minus_far)),
		maths_rs::Vec4f::from((0f32,			0f32, 			0f32,					1f32					   )),
	))
}

pub fn are_colinear(a: crate::Vector3, b: crate::Vector3) -> bool {
	maths_rs::dot(a, b).abs() > 0.99f32
}

pub fn from_normal(normal: Vector3) -> maths_rs::Mat4f {
	let x_basis;
	let y_basis;
	let z_basis = normal;

	if are_colinear(normal, Vector3::new(0f32, 1f32, 0f32)) {
		x_basis = maths_rs::normalize(maths_rs::cross(maths_rs::normalize(normal), crate::Vector3::new(0f32, 0f32, 1f32)));
		y_basis = maths_rs::normalize(maths_rs::cross(x_basis, maths_rs::normalize(normal)));
	} else {
		x_basis = maths_rs::normalize(maths_rs::cross(Vector3::new(0f32, 1f32, 0f32), maths_rs::normalize(normal)));
		y_basis = maths_rs::normalize(maths_rs::cross(maths_rs::normalize(normal), x_basis));
	}

	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((x_basis, 0f32)),
		maths_rs::Vec4f::from((y_basis, 0f32)),
		maths_rs::Vec4f::from((z_basis, 0f32)),
		maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
	))
}

pub fn from_rotation(axis: Vector3, theta: f32) -> maths_rs::Mat4f {
	let c = theta.cos();
	let s = -theta.sin();
	let one_minus_c = 1.0 - c;
	let x = axis.x;
	let y = axis.y;
	let z = axis.z;

	maths_rs::Mat4f::new(
		c + x * x * one_minus_c,    x * y * one_minus_c - z * s, x * z * one_minus_c + y * s, 0.0,
		y * x * one_minus_c + z * s, c + y * y * one_minus_c,    y * z * one_minus_c - x * s, 0.0,
		z * x * one_minus_c - y * s, z * y * one_minus_c + x * s, c + z * z * one_minus_c,    0.0,
		0.0,                        0.0,                        0.0,                        1.0
	)
}

/// Left handed row major 4x4 matrix inverse
pub fn inverse(m: maths_rs::Mat4f) -> maths_rs::Mat4f {
	let mut inv = maths_rs::Mat4f::default();

	inv[0] = m[5]  * m[10] * m[15] - m[5]  * m[11] * m[14] - m[9]  * m[6]  * m[15] + m[9]  * m[7]  * m[14] + m[13] * m[6]  * m[11] - m[13] * m[7]  * m[10];
    inv[4] = -m[4]  * m[10] * m[15] + m[4]  * m[11] * m[14] + m[8]  * m[6]  * m[15] - m[8]  * m[7]  * m[14] - m[12] * m[6]  * m[11] + m[12] * m[7]  * m[10];
    inv[8] = m[4]  * m[9] * m[15] - m[4]  * m[11] * m[13] - m[8]  * m[5] * m[15] + m[8]  * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
    inv[12] = -m[4]  * m[9] * m[14] + m[4]  * m[10] * m[13] + m[8]  * m[5] * m[14] - m[8]  * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
    inv[1] = -m[1]  * m[10] * m[15] + m[1]  * m[11] * m[14] + m[9]  * m[2] * m[15] - m[9]  * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
    inv[5] = m[0]  * m[10] * m[15] - m[0]  * m[11] * m[14] - m[8]  * m[2] * m[15] + m[8]  * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
    inv[9] = -m[0]  * m[9] * m[15] + m[0]  * m[11] * m[13] + m[8]  * m[1] * m[15] - m[8]  * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
    inv[13] = m[0]  * m[9] * m[14] - m[0]  * m[10] * m[13] - m[8]  * m[1] * m[14] + m[8]  * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
    inv[2] = m[1]  * m[6] * m[15] - m[1]  * m[7] * m[14] - m[5]  * m[2] * m[15] + m[5]  * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
    inv[6] = -m[0]  * m[6] * m[15] + m[0]  * m[7] * m[14] + m[4]  * m[2] * m[15] - m[4]  * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
    inv[10] = m[0]  * m[5] * m[15] - m[0]  * m[7] * m[13] - m[4]  * m[1] * m[15] + m[4]  * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
    inv[14] = -m[0]  * m[5] * m[14] + m[0]  * m[6] * m[13] + m[4]  * m[1] * m[14] - m[4]  * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
    inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];

    let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];

    if det == 0f32 {
        panic!("Matrix is not invertible");
	}

    let det = 1.0 / det;

    for i in 0..16 {
        inv[i] = inv[i] * det;
	}

	inv
}

#[cfg(test)]
mod tests {
    use maths_rs::mat::{MatInverse, MatTranspose};

	#[test]
	fn test_from_normal() {
		let value = super::from_normal(crate::Vector3::new(0f32, 0f32, 1f32));
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));

		let value = super::from_normal(crate::Vector3::new(0f32, 1f32, 0f32));
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));

		let value = super::from_normal(crate::Vector3::new(1f32, 0f32, 0f32));
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((0f32, 0f32, -1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));

		let value = super::from_normal(crate::Vector3::new(-1f32, 0f32, 0f32));
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((0f32, 0f32, 1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((-1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));
	}

	#[test]
	fn test_inverse_matrix() {
		let value = maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		));
		let value = super::inverse(value);
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 1f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 1f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));

		let value = maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 2f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 3f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		));
		let value = super::inverse(value);
		assert_eq!(value, maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 0f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0.5f32, 0f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 1f32 / 3f32, 0f32)),
			maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
		)));

		let nearly_equal = |a: f32, b: f32| (a - b).abs() < 0.0001f32;

		let value = maths_rs::Mat4f::from((
			maths_rs::Vec4f::from((1f32, 2f32, 3f32, 4f32)),
			maths_rs::Vec4f::from((5f32, 1f32, 6f32, 7f32)),
			maths_rs::Vec4f::from((8f32, 9f32, 1f32, 10f32)),
			maths_rs::Vec4f::from((11f32, 12f32, 13f32, 1f32)),
		));
		let value = value.inverse();
		
		assert!(nearly_equal(value[0], -212f32/507.0f32));
		assert!(nearly_equal(value[1], 55f32/338f32));
		assert!(nearly_equal(value[2], 157f32/3042f32));
		assert!(nearly_equal(value[3], 53f32/3042f32));
		assert!(nearly_equal(value[4], 103f32/507f32));
		assert!(nearly_equal(value[5], -61f32/338f32));
		assert!(nearly_equal(value[6], 127f32/3042f32));
		assert!(nearly_equal(value[7], 101f32/3042f32));
		assert!(nearly_equal(value[8], 79f32/507f32));
		assert!(nearly_equal(value[9], 9f32/338f32));
		assert!(nearly_equal(value[10], -257f32/3042f32));
		assert!(nearly_equal(value[11], 107f32/3042f32));
		assert!(nearly_equal(value[12], 23f32/169f32));
		assert!(nearly_equal(value[13], 5f32/169f32));
		assert!(nearly_equal(value[14], 5f32/169f32));
		assert!(nearly_equal(value[15], -8f32/169f32));
	}
}