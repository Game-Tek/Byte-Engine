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

pub fn from_normal(normal: crate::Vector3) -> maths_rs::Mat4f {
	let x_basis;
	let y_basis;
	let z_basis = normal;

	if maths_rs::dot(normal, crate::Vector3::new(0f32, 1f32, 0f32)).abs() < 0.99f32 {
		// If not colinear
		x_basis = maths_rs::normalize(maths_rs::cross(crate::Vector3::new(0f32, 1f32, 0f32), maths_rs::normalize(normal)));
		y_basis = maths_rs::normalize(maths_rs::cross(maths_rs::normalize(normal), x_basis));
	} else {
		// If colinear
		x_basis = maths_rs::normalize(maths_rs::cross(maths_rs::normalize(normal), crate::Vector3::new(0f32, 0f32, 1f32)));
		y_basis = maths_rs::normalize(maths_rs::cross(x_basis, maths_rs::normalize(normal)));
	}

	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((x_basis, 0f32)),
		maths_rs::Vec4f::from((y_basis, 0f32)),
		maths_rs::Vec4f::from((z_basis, 0f32)),
		maths_rs::Vec4f::from((0f32, 0f32, 0f32, 1f32)),
	))
}

#[cfg(test)]
mod tests {
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
}