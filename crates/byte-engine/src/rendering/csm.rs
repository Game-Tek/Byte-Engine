//! This module contains logic for rendering cascaded shadow maps.

use maths_rs::{mat::{MatProjection, MatTranslate}, num::Base, Mat4f, Vec3f, Vec4f};

use crate::math::look_down;

use super::view::View;

/// Returns the views for cascaded shadow mapping.
pub fn make_csm_views(camera_view: View, light_direction: Vec3f, num_cascades: usize) -> Vec<View> {
	let near = camera_view.near();
	let far = camera_view.far();
	let range = far - near;
	let ratio = far / near;

    (0..num_cascades).map(|i| {
		let p = (i + 1) as f32 / (num_cascades as f32);
		let log = camera_view.near() * ratio.powf(p);
		let uniform = near + range * p;
		let d = 0.95f32 * (log - uniform) + uniform;
		let factor = (d - near) / range;

		let camera_view = camera_view.from_from_z_planes(near, d);

		let light_view = {
			let camera_frustum_corners = camera_view.get_frustum_corners();
			let center = camera_frustum_corners.iter().fold(Vec4f::zero(), |acc, x| acc + *x) / 8.0;
		
			let radius = camera_frustum_corners.iter().map(|x| maths_rs::length(x - center)).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
		
			let radius = (radius * 16.0).ceil() / 16.0;
		
			let min = Vec3f::new(-radius, -radius, -radius);
			let max = Vec3f::new(radius, radius, radius);
		
			let center = Into::<Vec3f>::into(center);
		
			let from = center - light_direction * min.z;
		
			View::new_orthographic(min[0], max[0], min[1], max[1], 0f32, max[2] - min[2], from, light_direction)
		};

		light_view
	}).collect()
}