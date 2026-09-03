//! Chooses which scene lights receive shadow views this frame and builds those views.
//!
//! One directional light gets the cascades. Local lights compete for a bounded pool of cone layers and point
//! cube maps, ranked by how many sink pixels their conservative bounds cover.

use maths_rs::{Vec3f, Vec4f};
use smallvec::SmallVec;

use super::layout::{
	CONE_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_LIGHTS, MAX_POINT_SHADOW_POOL_CAPACITY,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_VIEW_OFFSET,
};
use crate::gameplay::Transform;
use crate::rendering::lights::{ConeLight, Lights, PointLight};
use crate::rendering::{Sink, View};
use crate::space::{Orientable as _, Positionable as _};

/// The minimum distance from a local light covered by an automatic shadow view.
pub(crate) const SHADOW_NEAR_M: f32 = 0.1;
/// The linear exposure multiplier used until a camera provides an exposure value.
pub(crate) const SHADOW_DEFAULT_EXPOSURE_SCALE: f32 = 1.0;
/// The exposure-weighted peak illuminance below which a local light stops casting shadows.
pub(crate) const SHADOW_EXPOSURE_THRESHOLD_LUX: f32 = 0.125;

/// The `LightShadow` enum is the shadow assignment encoded into one GPU light record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightShadow {
	None,
	Directional,
	Cone { view_index: u32, layer: u32 },
	Point { view_index: u32, cube_index: u32 },
}

/// The `ShadowLightSelection` struct retains the bounded directional and local-light shadow work for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowLightSelection<'a> {
	pub(crate) directional: Option<(usize, math::UnitVector)>,
	pub(crate) cones: [Option<(usize, &'a ConeLight, &'a Transform)>; MAX_CONE_SHADOW_POOL_CAPACITY],
	pub(crate) eligible_cone_count: usize,
	pub(crate) points: [Option<(usize, &'a PointLight, &'a Transform)>; MAX_POINT_SHADOW_POOL_CAPACITY],
	pub(crate) eligible_point_count: usize,
}

impl ShadowLightSelection<'_> {
	pub(crate) fn cone_count(&self) -> usize {
		self.cones.iter().flatten().count()
	}

	pub(crate) fn point_count(&self) -> usize {
		self.points.iter().flatten().count()
	}

	/// Returns the shadow assignment of the scene light at `light_index`.
	pub(crate) fn shadow_for(&self, light_index: usize) -> LightShadow {
		if self.directional.is_some_and(|(index, _)| index == light_index) {
			return LightShadow::Directional;
		}
		if let Some(layer) = self
			.cones
			.iter()
			.position(|cone| cone.is_some_and(|(index, ..)| index == light_index))
		{
			return LightShadow::Cone {
				view_index: (CONE_SHADOW_VIEW_OFFSET + layer) as u32,
				layer: layer as u32,
			};
		}
		if let Some(cube_index) = self
			.points
			.iter()
			.position(|point| point.is_some_and(|(index, ..)| index == light_index))
		{
			return LightShadow::Point {
				view_index: (POINT_SHADOW_VIEW_OFFSET + cube_index * POINT_SHADOW_FACE_COUNT) as u32,
				cube_index: cube_index as u32,
			};
		}
		LightShadow::None
	}
}

/// The `LocalLight` trait gives cone and point lights one shadow-range and brightness contract.
pub(crate) trait LocalLight {
	fn color(&self) -> Vec3f;
	fn shadow_near_override(&self) -> Option<f32>;
	fn shadow_far_override(&self) -> Option<f32>;
}

impl LocalLight for ConeLight {
	fn color(&self) -> Vec3f {
		self.color
	}
	fn shadow_near_override(&self) -> Option<f32> {
		ConeLight::shadow_near_override(self)
	}
	fn shadow_far_override(&self) -> Option<f32> {
		ConeLight::shadow_far_override(self)
	}
}

impl LocalLight for PointLight {
	fn color(&self) -> Vec3f {
		self.color
	}
	fn shadow_near_override(&self) -> Option<f32> {
		PointLight::shadow_near_override(self)
	}
	fn shadow_far_override(&self) -> Option<f32> {
		PointLight::shadow_far_override(self)
	}
}

/// Returns the luminance-weighted luminous intensity used for shadow coverage.
pub(crate) fn peak_candela(light: &impl LocalLight, intensity_scale_candela: f32) -> f32 {
	let color = light.color();
	(0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z) * intensity_scale_candela
}

/// Returns whether a local light has finite positive luminance that can cast a visible shadow.
pub(crate) fn has_brightness(light: &impl LocalLight, intensity_scale_candela: f32) -> bool {
	let peak = peak_candela(light, intensity_scale_candela);
	peak.is_finite() && peak > 0.0
}

/// Resolves the clipping range of one local shadow view.
///
/// The far distance is where the light's exposure-weighted peak illuminance reaches
/// [`SHADOW_EXPOSURE_THRESHOLD_LUX`]. Manual endpoints replace their automatic values and are clamped to
/// retain a valid perspective projection.
pub(crate) fn resolve_shadow_range(light: &impl LocalLight, exposure_scale: f32, intensity_scale_candela: f32) -> (f32, f32) {
	let exposure_scale = if exposure_scale.is_finite() {
		exposure_scale
	} else {
		SHADOW_DEFAULT_EXPOSURE_SCALE
	}
	.max(0.0);
	let automatic_far = (peak_candela(light, intensity_scale_candela) * exposure_scale / SHADOW_EXPOSURE_THRESHOLD_LUX)
		.sqrt()
		.max(SHADOW_NEAR_M + SHADOW_NEAR_M);
	let near = light
		.shadow_near_override()
		.filter(|value| value.is_finite())
		.unwrap_or(SHADOW_NEAR_M)
		.max(SHADOW_NEAR_M);
	let far = light
		.shadow_far_override()
		.filter(|value| value.is_finite())
		.unwrap_or(automatic_far)
		.max(near + SHADOW_NEAR_M);
	(near, far)
}

/// Builds the perspective view used to cull and render one cone-light shadow layer.
pub(crate) fn make_cone_shadow_view(
	light: &ConeLight,
	transform: &Transform,
	exposure_scale: f32,
	intensity_scale_candela: f32,
) -> View {
	let (near, far) = resolve_shadow_range(light, exposure_scale, intensity_scale_candela);
	View::new_perspective(
		(light.outer_angle * 2.0).to_degrees(),
		1.0,
		near,
		far,
		transform.position(),
		math::direction_from_orientation(transform.orientation()),
	)
}

/// Builds one of the six perspective views used to render a point-light cube shadow map.
pub(crate) fn make_point_shadow_view(
	light: &PointLight,
	transform: &Transform,
	face: usize,
	exposure_scale: f32,
	intensity_scale_candela: f32,
) -> View {
	let (near, far) = resolve_shadow_range(light, exposure_scale, intensity_scale_candela);
	let (direction, up) = match face {
		0 => (math::UnitVector::x_axis(), math::UnitVector::y_axis()),
		1 => (-math::UnitVector::x_axis(), math::UnitVector::y_axis()),
		2 => (math::UnitVector::y_axis(), -math::UnitVector::z_axis()),
		3 => (-math::UnitVector::y_axis(), math::UnitVector::z_axis()),
		4 => (math::UnitVector::z_axis(), math::UnitVector::y_axis()),
		5 => (-math::UnitVector::z_axis(), math::UnitVector::y_axis()),
		_ => unreachable!("Point shadow face is invalid. The most likely cause is a cube map dispatch outside its six faces."),
	};
	View::new_perspective_with_up(math::Degrees::new(90.0), 1.0, near, far, transform.position(), direction, up)
}

/// Returns the estimated screen coverage of a cone-shadow candidate in one sink.
pub(crate) fn cone_shadow_importance(
	light: &ConeLight,
	transform: &Transform,
	intensity_scale_candela: f32,
	sink: &Sink,
) -> Option<f32> {
	let (_, far) = resolve_shadow_range(light, SHADOW_DEFAULT_EXPOSURE_SCALE, intensity_scale_candela);
	let cosine = light.outer_angle.cos();
	let enclosing_radius = far / (2.0 * cosine * cosine);
	let bounds = math::Sphere::new(
		transform.position() + math::direction_from_orientation(transform.orientation()) * enclosing_radius,
		enclosing_radius,
	);
	shadow_view_importance(bounds, sink)
}

/// Returns the estimated screen coverage of a point-shadow candidate in one sink.
pub(crate) fn point_shadow_importance(
	light: &PointLight,
	transform: &Transform,
	intensity_scale_candela: f32,
	sink: &Sink,
) -> Option<f32> {
	let (_, far) = resolve_shadow_range(light, SHADOW_DEFAULT_EXPOSURE_SCALE, intensity_scale_candela);
	shadow_view_importance(math::Sphere::new(transform.position(), far), sink)
}

/// Returns the estimated number of sink pixels covered by a local light's conservative bound, or `None` when culled.
///
/// This projection is only a ranking proxy for assigning existing shadow views. It does not alter light
/// culling, shadow-map dimensions, or a light's shadow projection.
fn shadow_view_importance(bounds: math::Sphere, sink: &Sink) -> Option<f32> {
	let view = sink.view();
	if !math::collision::sphere_in_frustum(&bounds, &view.get_frustum_planes()) {
		return None;
	}
	let radius = bounds.radius();
	if !radius.is_finite() || radius <= 0.0 {
		return None;
	}
	let center = bounds.center().into_maths();
	let center_in_view = view.view() * Vec4f::new(center.x, center.y, center.z, 1.0);
	let depth = center_in_view.z;
	let pixel_count = sink.extent().width() as f32 * sink.extent().height() as f32;
	if !depth.is_finite() || !pixel_count.is_finite() {
		return None;
	}
	// A bound containing the camera covers every ray in the view, so rank it as a full sink.
	if depth <= radius {
		return Some(pixel_count);
	}
	let projection = view.projection();
	let clip_center = projection * center_in_view;
	if !clip_center.w.is_finite() || clip_center.w <= 0.0 {
		return None;
	}
	let center_x = clip_center.x / clip_center.w;
	let center_y = clip_center.y / clip_center.w;
	let depth_to_nearest_bound = depth - radius;
	let (radius_x, radius_y) = if view.y_fov() > math::Degrees::new(0.0) {
		(
			radius * projection[0].abs() / depth_to_nearest_bound,
			radius * projection[5].abs() / depth_to_nearest_bound,
		)
	} else {
		(radius * projection[0].abs(), radius * projection[5].abs())
	};
	if radius_x.is_infinite() || radius_y.is_infinite() {
		return Some(pixel_count);
	}
	if !center_x.is_finite() || !center_y.is_finite() || !radius_x.is_finite() || !radius_y.is_finite() {
		return None;
	}
	let covered_width = (center_x + radius_x).min(1.0) - (center_x - radius_x).max(-1.0);
	let covered_height = (center_y + radius_y).min(1.0) - (center_y - radius_y).max(-1.0);
	let coverage = (covered_width * 0.5).max(0.0) * (covered_height * 0.5).max(0.0);
	let importance = coverage * pixel_count;
	importance.is_finite().then_some(importance)
}

/// One local light eligible for a shadow-view assignment.
struct Candidate<'a, T> {
	index: usize,
	light: &'a T,
	transform: &'a Transform,
	intensity_scale_candela: f32,
}

// Derived `Copy` would require `T: Copy`, which the light types are not.
impl<T> Copy for Candidate<'_, T> {}
impl<T> Clone for Candidate<'_, T> {
	fn clone(&self) -> Self {
		*self
	}
}

/// Selects the shadow-casting lights for this frame from the light prefix uploaded to material evaluation.
///
/// `intensity_scale_candela` returns each light's calibrated IES peak scale, or `1.0` for analytic lights.
pub(crate) fn select_shadow_lights<'a>(
	lights: impl Iterator<Item = (&'a Lights, &'a Transform)>,
	sinks: &[Sink],
	cone_pool_capacity: usize,
	point_pool_capacity: usize,
	intensity_scale_candela: impl Fn(&Lights) -> f32,
) -> ShadowLightSelection<'a> {
	let mut selection = ShadowLightSelection::default();
	if sinks.is_empty() {
		return selection;
	}
	// The light table bounds each list, so these inline candidates never spill to the heap.
	let mut cone_candidates = SmallVec::<[Candidate<'a, ConeLight>; MAX_LIGHTS]>::new();
	let mut point_candidates = SmallVec::<[Candidate<'a, PointLight>; MAX_LIGHTS]>::new();

	for (index, (light, transform)) in lights.take(MAX_LIGHTS).enumerate() {
		let scale = intensity_scale_candela(light);
		match light {
			Lights::Direction(_) if selection.directional.is_none() => {
				selection.directional = Some((index, math::direction_from_orientation(transform.orientation())));
			}
			Lights::Cone(light)
				if has_brightness(light, scale)
					&& light.supports_shadow_mapping()
					&& sinks
						.iter()
						.any(|sink| cone_shadow_importance(light, transform, scale, sink).is_some()) =>
			{
				cone_candidates.push(Candidate {
					index,
					light,
					transform,
					intensity_scale_candela: scale,
				});
			}
			Lights::Point(light)
				if has_brightness(light, scale)
					&& sinks
						.iter()
						.any(|sink| point_shadow_importance(light, transform, scale, sink).is_some()) =>
			{
				point_candidates.push(Candidate {
					index,
					light,
					transform,
					intensity_scale_candela: scale,
				});
			}
			_ => {}
		}
	}

	selection.eligible_cone_count = cone_candidates.len();
	selection.cones = select_fair(&cone_candidates, sinks, cone_pool_capacity, |candidate, sink| {
		cone_shadow_importance(candidate.light, candidate.transform, candidate.intensity_scale_candela, sink)
	});
	selection.eligible_point_count = point_candidates.len();
	selection.points = select_fair(&point_candidates, sinks, point_pool_capacity, |candidate, sink| {
		point_shadow_importance(candidate.light, candidate.transform, candidate.intensity_scale_candela, sink)
	});
	selection
}

/// Assigns pool slots in sink-priority rounds so no sink can starve another.
///
/// Advancing all sinks together prevents a sink's changing coverage from displacing another sink's turn. A
/// partial final round favors earlier sinks.
fn select_fair<'a, T, const N: usize>(
	candidates: &[Candidate<'a, T>],
	sinks: &[Sink],
	pool_capacity: usize,
	importance: impl Fn(&Candidate<T>, &Sink) -> Option<f32>,
) -> [Option<(usize, &'a T, &'a Transform)>; N] {
	let capacity = pool_capacity.min(N);
	let mut selection = [None; N];
	let mut selected = 0;
	for priority in 0..candidates.len() {
		for sink in sinks {
			if selected == capacity {
				return selection;
			}
			let Some(candidate) = candidate_at_priority(candidates, sink, priority, &importance) else {
				continue;
			};
			if selection[..selected]
				.iter()
				.flatten()
				.any(|(index, ..)| *index == candidate.index)
			{
				continue;
			}
			selection[selected] = Some((candidate.index, candidate.light, candidate.transform));
			selected += 1;
		}
	}
	selection
}

/// Returns the candidate ranked `priority`-th for one sink by projected coverage, with scene order breaking ties.
fn candidate_at_priority<'a, T>(
	candidates: &[Candidate<'a, T>],
	sink: &Sink,
	priority: usize,
	importance: &impl Fn(&Candidate<T>, &Sink) -> Option<f32>,
) -> Option<Candidate<'a, T>> {
	let mut taken = [None; MAX_LIGHTS];
	for rank in 0..=priority {
		let best = candidates
			.iter()
			.filter(|candidate| !taken[..rank].contains(&Some(candidate.index)))
			.filter_map(|candidate| importance(candidate, sink).map(|importance| (*candidate, importance)))
			.max_by(|(left, left_importance), (right, right_importance)| {
				left_importance.total_cmp(right_importance).then(right.index.cmp(&left.index))
			})?
			.0;
		if rank == priority {
			return Some(best);
		}
		taken[rank] = Some(best.index);
	}
	None
}

#[cfg(test)]
mod tests {
	use math::{Point, UnitVector};
	use maths_rs::Vec3f;
	use utils::Extent;

	use super::*;
	use crate::rendering::lights::{DirectionalLight, LightColor, PhotometricIntensity};
	use crate::rendering::pipelines::visibility::layout::{
		DEFAULT_CONE_SHADOW_POOL_CAPACITY, DEFAULT_POINT_SHADOW_POOL_CAPACITY,
	};

	fn cone() -> ConeLight {
		ConeLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			math::Degrees::new(15.0).to_radians(),
			math::Degrees::new(30.0).to_radians(),
		)
		.expect("physical cone light")
	}

	fn point() -> PointLight {
		PointLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
		)
		.expect("physical point light")
	}

	fn light_transform(position_x: f32) -> Transform {
		Transform::from_position(Point::new(position_x, 2.0, 3.0))
	}

	fn sink(position: Point) -> Sink {
		Sink::new(
			View::new_perspective(math::Degrees::new(90.0), 1.0, 0.1, 100.0, position, UnitVector::z_axis()),
			Extent::square(1),
			0,
		)
	}

	fn select<'a>(
		lights: &'a [Lights],
		transforms: &'a [Transform],
		sinks: &[Sink],
		cone_capacity: usize,
		point_capacity: usize,
	) -> ShadowLightSelection<'a> {
		select_shadow_lights(lights.iter().zip(transforms), sinks, cone_capacity, point_capacity, |_| 1.0)
	}

	fn cone_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.cones.iter().flatten().map(|(index, ..)| *index).collect()
	}

	fn point_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.points.iter().flatten().map(|(index, ..)| *index).collect()
	}

	#[test]
	fn shadow_selection_keeps_one_directional_light_and_four_highest_priority_cones() {
		let wide_cone = ConeLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			math::Radians::new(0.25),
			math::Radians::new(std::f32::consts::PI),
		)
		.expect("physical cone light");
		let directional = DirectionalLight::new(
			LightColor::Kelvin(6_500.0),
			PhotometricIntensity::Illuminance {
				lux: 100_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light");
		let lights = [
			Lights::Cone(wide_cone),
			Lights::Cone(cone()),
			Lights::Point(point()),
			Lights::Direction(directional),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
		];
		let transforms = [
			light_transform(0.0),
			light_transform(0.0),
			light_transform(0.0),
			Transform::from_rotation(math::orientation_from_direction(-UnitVector::<math::WorldSpace>::y_axis())),
			light_transform(1.0),
			light_transform(2.0),
			light_transform(3.0),
			light_transform(4.0),
		];

		let selection = select(
			&lights,
			&transforms,
			&[sink(Point::origin())],
			DEFAULT_CONE_SHADOW_POOL_CAPACITY,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(selection.directional.map(|(index, _)| index), Some(3));
		assert_eq!(cone_indices(&selection), [1, 4, 5, 6]);
		assert_eq!(selection.eligible_cone_count, 5);
		assert_eq!(point_indices(&selection), [2]);
		assert_eq!(selection.eligible_point_count, 1);
		assert_eq!(selection.shadow_for(3), LightShadow::Directional);
		assert_eq!(
			selection.shadow_for(4),
			LightShadow::Cone {
				view_index: CONE_SHADOW_VIEW_OFFSET as u32 + 1,
				layer: 1
			}
		);
		assert_eq!(
			selection.shadow_for(2),
			LightShadow::Point {
				view_index: POINT_SHADOW_VIEW_OFFSET as u32,
				cube_index: 0
			}
		);
		assert_eq!(selection.shadow_for(0), LightShadow::None);
	}

	#[test]
	fn shadow_selection_keeps_cones_visible_in_any_sink_and_skips_cones_outside_all_sinks() {
		let visible_in_second_sink = cone().with_shadow_far(20.0);
		let outside_all_sinks = cone().with_shadow_far(20.0);
		let lights = [
			Lights::Cone(visible_in_second_sink.clone()),
			Lights::Cone(outside_all_sinks.clone()),
		];
		let transforms = [light_transform(100.0), light_transform(500.0)];
		let sinks = [sink(Point::origin()), sink(Point::new(100.0, 0.0, 0.0))];

		assert!(
			sinks
				.iter()
				.any(|sink| cone_shadow_importance(&visible_in_second_sink, &transforms[0], 1.0, sink).is_some())
		);
		assert!(
			sinks
				.iter()
				.all(|sink| cone_shadow_importance(&outside_all_sinks, &transforms[1], 1.0, sink).is_none())
		);

		let selection = select(
			&lights,
			&transforms,
			&sinks,
			DEFAULT_CONE_SHADOW_POOL_CAPACITY,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(cone_indices(&selection), [0]);
		assert_eq!(selection.eligible_cone_count, 1);
	}

	#[test]
	fn cone_shadow_pool_assigns_its_limited_layers_to_visible_lights() {
		let lights = [Lights::Cone(cone()), Lights::Cone(cone())];
		let transforms = [light_transform(0.0), light_transform(1.0)];
		let sinks = [sink(Point::origin())];

		let selection = select(&lights, &transforms, &sinks, 1, DEFAULT_POINT_SHADOW_POOL_CAPACITY);
		let empty_selection = select(&lights, &transforms, &sinks, 0, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(cone_indices(&selection), [0]);
		assert_eq!(selection.eligible_cone_count, 2);
		assert!(empty_selection.cones.iter().all(Option::is_none));
		assert_eq!(empty_selection.eligible_cone_count, 2);
	}

	#[test]
	fn cone_shadow_pool_orders_lights_by_projected_sink_coverage() {
		let lights = [
			Lights::Cone(cone().with_shadow_far(5.0)),
			Lights::Cone(cone().with_shadow_far(5.0)),
		];
		let transforms = [light_transform(8.0), light_transform(0.0)];

		let selection = select(
			&lights,
			&transforms,
			&[sink(Point::origin())],
			1,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(cone_indices(&selection), [1]);
	}

	#[test]
	fn cone_shadow_pool_continues_in_sink_order_after_assigning_each_sink_its_top_light() {
		let lights: Vec<_> = (0..6).map(|_| Lights::Cone(cone().with_shadow_far(20.0))).collect();
		let transforms = [0.0, 1.0, 2.0, 3.0, 100.0, 200.0].map(light_transform);
		let sinks = [
			sink(Point::origin()),
			sink(Point::new(100.0, 0.0, 0.0)),
			sink(Point::new(200.0, 0.0, 0.0)),
		];

		let selection = select(&lights, &transforms, &sinks, 4, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(cone_indices(&selection), [0, 4, 5, 1]);
	}

	#[test]
	fn unlit_cones_yield_pool_layers_to_visible_lit_cones() {
		let mut unlit = cone();
		unlit.color = Vec3f::new(0.0, 0.0, 0.0);
		let lights = [Lights::Cone(unlit.clone()), Lights::Cone(cone())];
		let transforms = [light_transform(0.0), light_transform(1.0)];

		assert!(!has_brightness(&unlit, 1.0));

		let selection = select(
			&lights,
			&transforms,
			&[sink(Point::origin())],
			1,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(cone_indices(&selection), [1]);
		assert_eq!(selection.eligible_cone_count, 1);
	}

	#[test]
	fn point_shadow_pool_assigns_its_limited_cubes_to_visible_lights() {
		let lights = [Lights::Point(point()), Lights::Point(point()), Lights::Point(point())];
		let transforms = [light_transform(0.0), light_transform(1.0), light_transform(2.0)];
		let sinks = [sink(Point::origin())];

		let selection = select(&lights, &transforms, &sinks, 0, 2);
		let empty_selection = select(&lights, &transforms, &sinks, 0, 0);

		assert_eq!(point_indices(&selection), [0, 1]);
		assert_eq!(selection.eligible_point_count, 3);
		assert!(empty_selection.points.iter().all(Option::is_none));
		assert_eq!(empty_selection.eligible_point_count, 3);
	}

	#[test]
	fn point_shadow_pool_orders_lights_by_projected_sink_coverage() {
		let lights = [
			Lights::Point(point().with_shadow_far(1.0)),
			Lights::Point(point().with_shadow_far(1.0)),
		];
		let transforms = [light_transform(3.5), light_transform(0.0)];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], 0, 1);

		assert_eq!(point_indices(&selection), [1]);
	}

	/// Verifies a resident profile's dimmed peak intensity drives both local-shadow range and selection.
	#[test]
	fn ies_profile_scale_expands_point_shadow_coverage() {
		let light = PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.5, "lights/office.ies")
			.expect("physical IES point light");
		let lights = [Lights::Point(light.clone())];
		let transforms = [light_transform(20.0)];
		let sinks = [sink(Point::origin())];

		let fallback = select(&lights, &transforms, &sinks, 0, 1);
		let resident = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, 1, |_| 90.0);
		let (_, fallback_far) = resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let (_, resident_far) = resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 90.0);

		assert!(fallback.points.iter().all(Option::is_none));
		assert_eq!(fallback.eligible_point_count, 0);
		assert_eq!(resident.points[0].map(|(index, ..)| index), Some(0));
		assert_eq!(resident.eligible_point_count, 1);
		assert!((resident_far / fallback_far - 90.0_f32.sqrt()).abs() < 0.0001);
	}

	#[test]
	fn point_shadow_views_cover_every_cube_direction_and_range() {
		let light = point().with_shadow_range(0.2, 50.0);
		let transform = light_transform(1.0);
		let directions = [
			UnitVector::x_axis(),
			-UnitVector::x_axis(),
			UnitVector::y_axis(),
			-UnitVector::y_axis(),
			UnitVector::z_axis(),
			-UnitVector::z_axis(),
		];

		for (face, direction) in directions.into_iter().enumerate() {
			let view = make_point_shadow_view(&light, &transform, face, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
			let point = (transform.get_position() + direction * 10.0).into_maths();
			let clip = view.view_projection() * Vec4f::new(point.x, point.y, point.z, 1.0);
			let ndc = clip / clip.w;

			assert!((view.y_fov().value() - 90.0).abs() < 0.0001);
			assert_eq!(view.near(), 0.2);
			assert_eq!(view.far(), 50.0);
			assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001);
			assert!((0.0..=1.0).contains(&ndc.z));
		}

		let positive_y_view = make_point_shadow_view(&light, &transform, 2, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let right_of_positive_y_face =
			(transform.get_position() + UnitVector::y_axis() * 10.0 + UnitVector::x_axis()).into_maths();
		let clip = positive_y_view.view_projection()
			* Vec4f::new(
				right_of_positive_y_face.x,
				right_of_positive_y_face.y,
				right_of_positive_y_face.z,
				1.0,
			);

		assert!((clip.x / clip.w) > 0.0);
	}

	#[test]
	fn point_shadow_range_uses_manual_endpoints_and_visibility() {
		let light = point().with_shadow_range(-4.0, f32::NAN);
		let transform = light_transform(500.0);

		let (near, far) = resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let automatic_far = (100.0 / SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert_eq!(near, SHADOW_NEAR_M);
		assert_eq!(far, automatic_far);
		assert!(point_shadow_importance(&light.with_shadow_far(20.0), &transform, 1.0, &sink(Point::origin())).is_none());
		assert!(
			point_shadow_importance(
				&point().with_shadow_far(20.0),
				&light_transform(100.0),
				1.0,
				&sink(Point::new(100.0, 0.0, 0.0)),
			)
			.is_some()
		);

		let mut unlit = point();
		unlit.color = Vec3f::new(0.0, 0.0, 0.0);
		assert!(!has_brightness(&unlit, 1.0));
	}

	#[test]
	fn cone_shadow_view_uses_the_light_projection_and_automatic_clip_range() {
		let light = cone();
		let transform = light_transform(1.0);

		let view = make_cone_shadow_view(&light, &transform, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let point =
			(transform.get_position() + math::direction_from_orientation(transform.get_orientation()) * 10.0).into_maths();
		let clip = view.view_projection() * Vec4f::new(point.x, point.y, point.z, 1.0);
		let ndc = clip / clip.w;
		let automatic_far = (100.0 / SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert!((view.y_fov().value() - 60.0).abs() < 0.0001);
		assert_eq!(view.near(), SHADOW_NEAR_M);
		assert_eq!(SHADOW_EXPOSURE_THRESHOLD_LUX, 0.125);
		assert!((view.far() - automatic_far).abs() < 0.0001);
		assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001);
		assert!((0.0..=1.0).contains(&ndc.z));
	}

	#[test]
	fn cone_shadow_range_uses_manual_endpoints_and_clamps_invalid_values() {
		let light = cone().with_shadow_range(-4.0, f32::NAN);
		let (near, far) = resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let automatic_far = (100.0 / SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert_eq!(near, SHADOW_NEAR_M);
		assert!((far - automatic_far).abs() < 0.0001);

		let light = cone().with_shadow_near(50.0).with_shadow_far(20.0);
		assert_eq!(resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0), (50.0, 50.1));
	}

	#[test]
	fn cone_shadow_range_scales_with_linear_exposure() {
		let light = cone();
		let (_, neutral_far) = resolve_shadow_range(&light, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let (_, brighter_far) = resolve_shadow_range(&light, 4.0, 1.0);
		let (_, invalid_far) = resolve_shadow_range(&light, f32::NAN, 1.0);

		assert!((brighter_far - neutral_far * 2.0).abs() < 0.0001);
		assert!((invalid_far - neutral_far).abs() < 0.0001);
	}
}
