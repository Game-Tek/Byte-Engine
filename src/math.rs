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
	maths_rs::Mat4f::from((
		maths_rs::Vec4f::from((2f32 / width, 	0f32, 			0f32, 													0f32)),
		maths_rs::Vec4f::from((0f32, 			2f32 / height,	0f32, 													0f32)),
		maths_rs::Vec4f::from((0f32, 			0f32, 			2f32 / (far_plane - near_plane), 						0f32)),
		maths_rs::Vec4f::from((0f32,			0f32, 			-((far_plane + near_plane) / (far_plane - near_plane)),	1f32)),
	))
}