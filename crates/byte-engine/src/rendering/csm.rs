//! This module contains logic for rendering cascaded shadow maps.

use math::{length, normalize, Base as _, Vector3, Vector4};

use super::view::View;

/// Returns the camera-space near and far distance for each shadow cascade.
pub(crate) fn make_cascade_split_ranges(camera_view: View, num_cascades: usize) -> Vec<(f32, f32)> {
	let near = camera_view.near();
	let far = camera_view.far();
	let range = far - near;
	let ratio = far / near;
	let mut cascade_near = near;

	(0..num_cascades)
		.map(|index| {
			let p = (index + 1) as f32 / num_cascades as f32;
			let log = near * ratio.powf(p);
			let uniform = near + range * p;
			let cascade_far = 0.95f32 * (log - uniform) + uniform;
			let cascade_range = (cascade_near, cascade_far);
			cascade_near = cascade_far;
			cascade_range
		})
		.collect()
}

/// Returns the views for cascaded shadow mapping.
pub fn make_csm_views(camera_view: View, light_direction: Vector3, num_cascades: usize) -> Vec<View> {
	let light_direction = normalize(light_direction);

	make_cascade_split_ranges(camera_view, num_cascades)
		.into_iter()
		.map(|(cascade_near, cascade_far)| {
			let camera_view = camera_view.from_from_z_planes(cascade_near, cascade_far);

			let light_view = {
				let camera_frustum_corners = camera_view.get_frustum_corners();
				let center = camera_frustum_corners.iter().fold(Vector4::zero(), |acc, x| acc + *x) / 8.0;

				let radius = camera_frustum_corners
					.iter()
					.map(|x| length(x - center))
					.max_by(|a, b| a.partial_cmp(b).unwrap())
					.unwrap();

				let radius = (radius * 16.0).ceil() / 16.0;

				let min = Vector3::new(-radius, -radius, -radius);
				let max = Vector3::new(radius, radius, radius);

				let center: Vector3 = center.into();

				let from = center - light_direction * max.z;

				View::new_orthographic(min[0], max[0], min[1], max[1], 0f32, max[2] - min[2], from, light_direction)
			};

			light_view
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use math::{assert_float_eq, Base as _, VecN as _, Vector3};

	use crate::rendering::view::View;

	#[test]
	fn cascade_split_ranges_partition_the_camera_frustum() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());
		let cascade_ranges = super::make_cascade_split_ranges(camera_view, 4);

		let mut expected_near = camera_view.near();

		for (cascade_near, cascade_far) in cascade_ranges {
			assert_float_eq!(
				cascade_near,
				expected_near,
				"Cascade near plane should continue from the previous split"
			);
			assert!(
				cascade_far > cascade_near,
				"Cascade far plane should lie beyond the cascade near plane"
			);
			expected_near = cascade_far;
		}

		assert_float_eq!(
			expected_near,
			camera_view.far(),
			"Cascade splits should reach the camera far plane"
		);
	}

	#[test]
	fn shadow_view_keeps_cascade_center_in_front_of_the_light() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());
		let shadow_view = super::make_csm_views(camera_view, Vector3::unit_z(), 1)
			.into_iter()
			.next()
			.expect("A shadow cascade view should be generated");

		let corners = camera_view.get_frustum_corners();
		let center = corners
			.iter()
			.copied()
			.fold(math::Vector4::new(0.0, 0.0, 0.0, 0.0), |acc, corner| acc + corner)
			/ corners.len() as f32;
		let light_space_center = shadow_view.view() * center;

		assert!(
			light_space_center.z >= 0.0,
			"Cascade center should be in front of the light view"
		);
		assert!(
			light_space_center.z <= shadow_view.far(),
			"Cascade center should lie inside the shadow depth range"
		);
	}
}
