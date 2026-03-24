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
	let camera_far = camera_view.far();

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

				let center: Vector3 = center.into();

				// Extend the depth range behind the bounding sphere to capture
				// shadow casters between the light source and the camera frustum.
				// Without this, nearby cascades may miss tall/distant casters,
				// causing shadows to phase in and out as objects cross cascade boundaries.
				let back_extension = camera_far;
				let depth = 2.0 * radius + back_extension;

				let from = center - light_direction * (radius + back_extension);

				View::new_orthographic(-radius, radius, -radius, radius, 0f32, depth, from, light_direction)
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

	/// Verifies that a surface point inside a cascade's camera frustum projects into valid NDC
	/// range [0,1] for depth and [-1,1] for x/y when transformed by the cascade's light view-projection.
	/// This simulates the full shadow rendering + sampling pipeline on the CPU to catch projection mismatches.
	#[test]
	fn surface_point_projects_into_valid_shadow_ndc() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());
		let light_direction = math::normalize(Vector3::new(0.5, -1.0, 0.3));
		let num_cascades = 4;

		let cascade_views = super::make_csm_views(camera_view, light_direction, num_cascades);
		let cascade_ranges = super::make_cascade_split_ranges(camera_view, num_cascades);

		// Test surface points at various depths inside each cascade.
		for (cascade_idx, ((cascade_near, cascade_far), cascade_view)) in
			cascade_ranges.iter().zip(cascade_views.iter()).enumerate()
		{
			// Pick a surface point at the midpoint depth of this cascade, on the camera's z-axis.
			let mid_depth = (cascade_near + cascade_far) / 2.0;
			let surface_point = math::Vector4::new(0.0, 0.0, mid_depth, 1.0);

			// Project through the cascade's view-projection (simulates the mesh shader).
			let clip = cascade_view.view_projection() * surface_point;
			let ndc = Vector3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);

			assert!(
				ndc.x >= -1.0 && ndc.x <= 1.0,
				"Cascade {}: NDC x ({}) out of range for surface at depth {}",
				cascade_idx,
				ndc.x,
				mid_depth
			);
			assert!(
				ndc.y >= -1.0 && ndc.y <= 1.0,
				"Cascade {}: NDC y ({}) out of range for surface at depth {}",
				cascade_idx,
				ndc.y,
				mid_depth
			);
			assert!(
				ndc.z >= 0.0 && ndc.z <= 1.0,
				"Cascade {}: NDC z ({}) out of [0,1] for surface at depth {}",
				cascade_idx,
				ndc.z,
				mid_depth
			);
		}
	}

	/// Verifies that shadow views for various light directions produce valid orthonormal view matrices.
	#[test]
	fn shadow_view_matrices_are_valid_for_various_light_directions() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Vector3::zero(), Vector3::unit_z());

		let light_directions = [
			Vector3::new(0.0, -1.0, 0.0),  // Straight down
			Vector3::new(0.0, 1.0, 0.0),   // Straight up
			Vector3::new(1.0, 0.0, 0.0),   // Right
			Vector3::new(0.0, 0.0, 1.0),   // Forward
			Vector3::new(0.5, -1.0, 0.3),  // Diagonal
			Vector3::new(-0.2, -0.8, 0.5), // Another diagonal
		];

		for light_dir in &light_directions {
			let cascade_views = super::make_csm_views(camera_view, *light_dir, 4);

			for (i, view) in cascade_views.iter().enumerate() {
				let m = view.view();

				// Check that the 3x3 rotation part is orthonormal.
				// Row vectors should be unit length.
				let r0 = Vector3::new(m[0], m[1], m[2]);
				let r1 = Vector3::new(m[4], m[5], m[6]);
				let r2 = Vector3::new(m[8], m[9], m[10]);

				let len0 = math::length(r0);
				let len1 = math::length(r1);
				let len2 = math::length(r2);

				assert!(
					(len0 - 1.0).abs() < 1e-5,
					"Light {:?}, cascade {}: row 0 length = {}",
					light_dir,
					i,
					len0
				);
				assert!(
					(len1 - 1.0).abs() < 1e-5,
					"Light {:?}, cascade {}: row 1 length = {}",
					light_dir,
					i,
					len1
				);
				assert!(
					(len2 - 1.0).abs() < 1e-5,
					"Light {:?}, cascade {}: row 2 length = {}",
					light_dir,
					i,
					len2
				);

				// Rows should be mutually orthogonal.
				let d01 = math::dot(r0, r1);
				let d02 = math::dot(r0, r2);
				let d12 = math::dot(r1, r2);

				assert!(d01.abs() < 1e-5, "Light {:?}, cascade {}: dot(r0,r1) = {}", light_dir, i, d01);
				assert!(d02.abs() < 1e-5, "Light {:?}, cascade {}: dot(r0,r2) = {}", light_dir, i, d02);
				assert!(d12.abs() < 1e-5, "Light {:?}, cascade {}: dot(r1,r2) = {}", light_dir, i, d12);
			}
		}
	}
}
