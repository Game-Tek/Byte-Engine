//! This module contains logic for rendering cascaded shadow maps.

use maths_rs::{mat::{MatProjection, MatTranslate}, num::Base, Mat4f, Vec3f, Vec4f};

use crate::math::look_down;

use super::view::View;

/// Returns the views for cascaded shadow mapping.
pub fn make_csm_views(camera_view: View, light_direction: Vec3f, num_cascades: usize) -> Vec<View> {
    (0..num_cascades).map(|i| {
		let near_distance = 0.01f32.max((i as f32) * 4f32);
		let far_distance = ((i + 1) as f32) * 4f32;

		let camera_view = camera_view.from_from_z_planes(near_distance, far_distance);

		let light_view = make_light_view(camera_view, light_direction);

		light_view
	}).collect()
}

fn make_light_view(camera_view: View, light_direction: Vec3f) -> View {
	let camera_frustum_corners = camera_view.get_frustum_corners();
	let center = camera_frustum_corners.iter().fold(Vec4f::zero(), |acc, x| acc + *x) / 8.0;

	let light_view = look_down(light_direction.into()) * Mat4f::from_translation(-Into::<Vec3f>::into(center));

	let transformed_corners = camera_frustum_corners.iter().map(|x| light_view * x);

	let min = [
		transformed_corners.clone().map(|x| x.x).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
		transformed_corners.clone().map(|x| x.y).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
		transformed_corners.clone().map(|x| x.z).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap()
	];
	let max = [
		transformed_corners.clone().map(|x| x.x).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
		transformed_corners.clone().map(|x| x.y).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
		transformed_corners.clone().map(|x| x.z).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap()
	];

	const zMult: f32 = 10f32;

    // if min[2] < 0f32 {
    //     min[2] *= zMult;
    // } else {
    //     min[2] /= zMult;
    // }

    // if max[2] < 0f32 {
    //     max[2] /= zMult;
    // } else {
    //     max[2] *= zMult;
    // }

	View::new_orthographic(min[0], max[0], min[1], max[1], min[2], max[2], center.into(), light_direction)
}