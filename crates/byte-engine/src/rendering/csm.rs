//! Cascaded shadow-map calculation and rendering support.

use math::{Matrix, Point, UnitVector, Vector};
use maths_rs::{mat::MatTranslate as _, Vec4f};
use smallvec::SmallVec;

use super::view::View;

/// Returns the camera-space near and far distance for each shadow cascade.
pub(crate) fn make_cascade_split_ranges(camera_view: View, num_cascades: usize) -> impl ExactSizeIterator<Item = (f32, f32)> {
	let near = camera_view.near();
	let far = camera_view.far();
	debug_assert!(
		num_cascades > 0,
		"Cascade count is zero. The most likely cause is creating a shadow pipeline without any cascade layers."
	);
	debug_assert!(
		near.is_finite() && far.is_finite() && near > 0.0 && far > near,
		"Camera depth range is invalid. The most likely cause is a nonpositive near plane or a far plane that does not follow it."
	);
	let range = far - near;
	let ratio = far / near;
	let mut cascade_near = near;

	(0..num_cascades).map(move |index| {
		let p = (index + 1) as f32 / num_cascades as f32;
		let log = near * ratio.powf(p);
		let uniform = near + range * p;
		let cascade_far = 0.95 * (log - uniform) + uniform;
		let cascade_range = (cascade_near, cascade_far);
		cascade_near = cascade_far;
		cascade_range
	})
}

/// Returns the world-space views for cascaded shadow mapping.
pub fn make_csm_views(
	camera_view: View,
	light_direction: UnitVector,
	num_cascades: usize,
	shadow_map_resolution: u32,
) -> impl ExactSizeIterator<Item = View> {
	let camera_far = camera_view.far();

	make_cascade_split_ranges(camera_view, num_cascades).map(move |(cascade_near, cascade_far)| {
		let camera_view = camera_view.from_from_z_planes(cascade_near, cascade_far);
		let camera_frustum_corners = camera_view.get_frustum_corners();
		let center = frustum_center(&camera_frustum_corners);
		let radius = stabilize_cascade_radius(center, &camera_frustum_corners, shadow_map_resolution);

		// Extend behind the bounding sphere so casters between the light and camera remain in the shadow view.
		let back_extension = camera_far;
		let depth = 2.0 * radius + back_extension;
		let light_position = center - light_direction * (radius + back_extension);
		let light_view = View::new_orthographic(-radius, radius, -radius, radius, 0.0, depth, light_position, light_direction);

		snap_shadow_view_to_texels(light_view, center, radius, shadow_map_resolution)
	})
}

/// Returns the arithmetic center of a fixed-size group of world-space frustum corners.
fn frustum_center(corners: &[Point; 8]) -> Point {
	let sum = corners
		.iter()
		.fold(Vector::zero(), |sum, corner| sum + (*corner - Point::origin()));
	Point::origin() + sum / corners.len() as f32
}

/// Expands the cascade sphere to a stable size that changes only in texel-sized steps.
fn stabilize_cascade_radius(center: Point, camera_frustum_corners: &[Point; 8], shadow_map_resolution: u32) -> f32 {
	let base_radius = camera_frustum_corners
		.iter()
		.map(|corner| (*corner - center).length_squared())
		.max_by(|left, right| left.partial_cmp(right).expect("Frustum corner distance must be finite"))
		.expect("A cascade frustum must have corners")
		.sqrt();

	if shadow_map_resolution == 0 {
		return (base_radius * 16.0).ceil() / 16.0;
	}

	let minimum_radius = (base_radius * 16.0).ceil() / 16.0;
	let texel_scale = shadow_map_resolution as f32 / 2.0;
	(minimum_radius * texel_scale).ceil() / texel_scale
}

/// Aligns the orthographic shadow view to the shadow-map texel grid.
fn snap_shadow_view_to_texels(light_view: View, center: Point, radius: f32, shadow_map_resolution: u32) -> View {
	if shadow_map_resolution == 0 {
		return light_view;
	}

	let texel_size = (2.0 * radius) / shadow_map_resolution as f32;
	if texel_size <= 0.0 {
		return light_view;
	}

	let center_maths = center.into_maths();
	let light_space_center = light_view.view() * Vec4f::new(center_maths.x, center_maths.y, center_maths.z, 1.0);
	let snap_offset: Vector = Vector::new(
		(light_space_center.x / texel_size).round() * texel_size - light_space_center.x,
		(light_space_center.y / texel_size).round() * texel_size - light_space_center.y,
		0.0,
	);

	// Translation is a matrix boundary, so the world displacement is explicitly unbranded here.
	light_view.from_view(Matrix::from_translation(snap_offset.into_maths()) * light_view.view())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cascade_split_ranges_partition_the_camera_frustum() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let ranges = make_cascade_split_ranges(camera_view, 4).collect::<SmallVec<[(f32, f32); 4]>>();

		assert_eq!(ranges.len(), 4);
		assert!((ranges[0].0 - camera_view.near()).abs() < 0.0001);
		assert!((ranges[3].1 - camera_view.far()).abs() < 0.0001);
		assert!(ranges.windows(2).all(|ranges| (ranges[0].1 - ranges[1].0).abs() < 0.0001));
	}

	#[test]
	fn cascade_views_keep_the_frustum_center_inside_light_depth() {
		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let shadow_view = make_csm_views(camera_view, UnitVector::z_axis(), 1, 2048)
			.next()
			.expect("a shadow cascade view");
		let center = frustum_center(&camera_view.get_frustum_corners());
		let center_maths = center.into_maths();
		let light_space_center = shadow_view.view() * Vec4f::new(center_maths.x, center_maths.y, center_maths.z, 1.0);

		assert!((0.0..=shadow_view.far()).contains(&light_space_center.z));
	}

	#[test]
	fn cascade_radius_is_quantized_to_texel_steps() {
		let view = View::new_perspective(
			75.0,
			16.0 / 9.0,
			0.1,
			100.0,
			Point::new(0.37, -1.12, 2.83),
			UnitVector::z_axis(),
		);
		let corners = view.get_frustum_corners();
		let resolution = 1024;
		let radius = stabilize_cascade_radius(frustum_center(&corners), &corners, resolution);

		assert!(((radius * resolution as f32) / 2.0).fract().abs() < 0.0001);
	}

	#[test]
	fn texel_snapping_aligns_the_cascade_center() {
		let view = View::new_perspective(
			75.0,
			16.0 / 9.0,
			0.1,
			100.0,
			Point::new(0.37, -1.12, 2.83),
			UnitVector::z_axis(),
		);
		let resolution = 1024;
		let shadow_view = make_csm_views(
			view,
			Vector::new(0.5, -1.0, 0.3).normalize().expect("nonzero light direction"),
			1,
			resolution,
		)
		.next()
		.expect("a shadow cascade view");
		let corners = view.get_frustum_corners();
		let center = frustum_center(&corners);
		let radius = stabilize_cascade_radius(center, &corners, resolution);
		let center_maths = center.into_maths();
		let snapped = shadow_view.view() * Vec4f::new(center_maths.x, center_maths.y, center_maths.z, 1.0);
		let texel_size = (2.0 * radius) / resolution as f32;

		assert!((snapped.x / texel_size).fract().abs() < 0.0001);
		assert!((snapped.y / texel_size).fract().abs() < 0.0001);
	}

	#[test]
	fn shadow_view_matrices_are_orthonormal_for_cardinal_and_diagonal_directions() {
		use maths_rs::{dot, length, Vec3f};

		let camera_view = View::new_perspective(90.0, 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		let directions = [
			UnitVector::y_axis(),
			-UnitVector::y_axis(),
			UnitVector::x_axis(),
			UnitVector::z_axis(),
			Vector::new(0.5, -1.0, 0.3).normalize().expect("nonzero light direction"),
		];

		for direction in directions {
			for view in make_csm_views(camera_view, direction, 4, 2048) {
				let matrix = view.view();
				// `View` is a raw-matrix boundary, so inspect its basis as `maths_rs` vectors.
				let x = Vec3f::new(matrix[0], matrix[1], matrix[2]);
				let y = Vec3f::new(matrix[4], matrix[5], matrix[6]);
				let z = Vec3f::new(matrix[8], matrix[9], matrix[10]);

				assert!((length(x) - 1.0).abs() < 1e-5);
				assert!((length(y) - 1.0).abs() < 1e-5);
				assert!((length(z) - 1.0).abs() < 1e-5);
				assert!(dot(x, y).abs() < 1e-5);
				assert!(dot(x, z).abs() < 1e-5);
				assert!(dot(y, z).abs() < 1e-5);
			}
		}
	}

	#[test]
	fn zenith_light_keeps_vertical_casters_on_one_shadow_texel_column() {
		let camera_view = View::new_perspective(75.0, 1.0, 0.1, 100.0, Point::new(0.0, 2.0, 0.0), UnitVector::z_axis());
		let floor = Vec4f::new(1.25, 0.0, 5.0, 1.0);
		let caster = Vec4f::new(1.25, 3.0, 5.0, 1.0);

		for view in make_csm_views(camera_view, -UnitVector::y_axis(), 4, 2048) {
			let floor_clip = view.view_projection() * floor;
			let caster_clip = view.view_projection() * caster;

			assert!((floor_clip.x / floor_clip.w - caster_clip.x / caster_clip.w).abs() < 1e-5);
			assert!((floor_clip.y / floor_clip.w - caster_clip.y / caster_clip.w).abs() < 1e-5);
		}
	}
}
