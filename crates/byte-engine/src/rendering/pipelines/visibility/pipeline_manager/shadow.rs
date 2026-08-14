use super::*;

/// The `ShadowLightSelection` struct retains the bounded directional and local-light shadow work for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ShadowLightSelection {
	pub(crate) directional: Option<(usize, UnitVector)>,
	pub(crate) cones: [Option<(usize, ConeLight)>; MAX_CONE_SHADOW_POOL_CAPACITY],
	pub(crate) eligible_cone_count: usize,
	pub(crate) points: [Option<(usize, PointLight)>; MAX_POINT_SHADOW_POOL_CAPACITY],
	pub(crate) eligible_point_count: usize,
}

/// The `ShadowLightCandidate` struct retains one local light eligible for a shadow-view assignment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowLightCandidate<T> {
	index: usize,
	light: T,
}

/// Selects the most important local shadow casters from the light prefix uploaded to material evaluation.
pub(crate) fn select_shadow_lights<'a>(
	lights: impl Iterator<Item = &'a Lights>,
	sinks: &[Sink],
	cone_shadow_map_pool_capacity: usize,
	point_shadow_map_pool_capacity: usize,
) -> ShadowLightSelection {
	let mut selection = ShadowLightSelection::default();
	if sinks.is_empty() {
		return selection;
	}
	// The light table bounds each list, so these inline candidates never spill to the heap.
	let mut cone_candidates = SmallVec::<[ShadowLightCandidate<ConeLight>; MAX_LIGHTS]>::new();
	let mut point_candidates = SmallVec::<[ShadowLightCandidate<PointLight>; MAX_LIGHTS]>::new();

	for (index, light) in lights.take(MAX_LIGHTS).enumerate() {
		match light {
			Lights::Direction(light) if selection.directional.is_none() => {
				selection.directional = Some((index, light.direction));
			}
			Lights::Cone(light) if cone_light_has_brightness(*light) && light.supports_shadow_mapping() => {
				if shadow_light_is_visible_in_any_sink(*light, sinks, cone_shadow_importance) {
					cone_candidates.push(ShadowLightCandidate { index, light: *light });
				}
			}
			Lights::Point(light) if point_light_has_brightness(*light) => {
				if shadow_light_is_visible_in_any_sink(*light, sinks, point_shadow_importance) {
					point_candidates.push(ShadowLightCandidate { index, light: *light });
				}
			}
			Lights::Cone(_) | Lights::Direction(_) | Lights::Point(_) => {}
		}
	}

	selection.eligible_cone_count = cone_candidates.len();
	selection.cones = select_fair_shadow_lights(
		&mut cone_candidates,
		sinks,
		cone_shadow_map_pool_capacity,
		cone_shadow_importance,
	);
	selection.eligible_point_count = point_candidates.len();
	selection.points = select_fair_shadow_lights(
		&mut point_candidates,
		sinks,
		point_shadow_map_pool_capacity,
		point_shadow_importance,
	);

	selection
}

/// Returns whether `light` has a projected coverage score in at least one active sink.
pub(crate) fn shadow_light_is_visible_in_any_sink<T: Copy>(
	light: T,
	sinks: &[Sink],
	importance: impl Fn(T, &Sink) -> Option<f32>,
) -> bool {
	sinks.iter().any(|sink| importance(light, sink).is_some())
}

/// Assigns existing shadow-map slots in sink-priority rounds without cross-sink competition.
pub(crate) fn select_fair_shadow_lights<T: Copy, const N: usize>(
	candidates: &mut [ShadowLightCandidate<T>],
	sinks: &[Sink],
	pool_capacity: usize,
	importance: impl Fn(T, &Sink) -> Option<f32>,
) -> [Option<(usize, T)>; N] {
	let capacity = pool_capacity.min(N);
	let mut selection = [None; N];
	let mut selection_count = 0;

	// Advancing all sinks together prevents a sink's changing coverage from displacing another
	// sink's turn. A partial final round favors earlier sinks.
	for priority in 0..candidates.len() {
		for sink in sinks {
			if selection_count == capacity {
				return selection;
			}
			let Some(candidate) = candidate_for_sink_priority(candidates, sink, priority, &importance) else {
				continue;
			};
			if selection[..selection_count]
				.iter()
				.flatten()
				.any(|(index, _)| *index == candidate.index)
			{
				continue;
			}
			selection[selection_count] = Some((candidate.index, candidate.light));
			selection_count += 1;
		}
	}

	selection
}

/// Returns the light at one sink's projected-coverage priority.
pub(crate) fn candidate_for_sink_priority<T: Copy>(
	candidates: &[ShadowLightCandidate<T>],
	sink: &Sink,
	priority: usize,
	importance: &impl Fn(T, &Sink) -> Option<f32>,
) -> Option<ShadowLightCandidate<T>> {
	let mut higher_priority_indices = [None; MAX_LIGHTS];
	for rank in 0..=priority {
		let mut best: Option<(ShadowLightCandidate<T>, f32)> = None;
		for candidate in candidates {
			if higher_priority_indices[..rank].contains(&Some(candidate.index)) {
				continue;
			}
			let Some(candidate_importance) = importance(candidate.light, sink) else {
				continue;
			};
			let is_more_important = match best {
				Some((best_candidate, best_importance)) => {
					shadow_light_is_more_important(candidate.index, candidate_importance, best_candidate.index, best_importance)
				}
				None => true,
			};
			if is_more_important {
				best = Some((*candidate, candidate_importance));
			}
		}

		let (candidate, _) = best?;
		if rank == priority {
			return Some(candidate);
		}
		higher_priority_indices[rank] = Some(candidate.index);
	}

	None
}

/// Returns whether the left light wins an importance comparison, with scene order as a stable tie-breaker.
pub(crate) fn shadow_light_is_more_important(
	left_index: usize,
	left_importance: f32,
	right_index: usize,
	right_importance: f32,
) -> bool {
	match left_importance.total_cmp(&right_importance) {
		std::cmp::Ordering::Greater => true,
		std::cmp::Ordering::Equal => left_index < right_index,
		std::cmp::Ordering::Less => false,
	}
}

/// The minimum distance from a cone light covered by an automatic shadow view.
pub(crate) const CONE_SHADOW_NEAR_M: f32 = 0.1;
/// The linear exposure multiplier used until a camera provides an exposure value.
pub(crate) const CONE_SHADOW_DEFAULT_EXPOSURE_SCALE: f32 = 1.0;
/// The exposure-weighted peak illuminance threshold for automatic cone shadow coverage.
pub(crate) const CONE_SHADOW_EXPOSURE_THRESHOLD_LUX: f32 = 0.125;
/// The minimum distance from a point light covered by an automatic cube shadow map.
pub(crate) const POINT_SHADOW_NEAR_M: f32 = 0.1;
/// The linear exposure multiplier used until a camera provides an exposure value.
pub(crate) const POINT_SHADOW_DEFAULT_EXPOSURE_SCALE: f32 = 1.0;
/// The exposure-weighted peak illuminance threshold for automatic point shadow coverage.
pub(crate) const POINT_SHADOW_EXPOSURE_THRESHOLD_LUX: f32 = 0.125;

/// Resolves the clipping range for one cone-light shadow view.
///
/// The far distance is where the light's exposure-weighted peak illuminance reaches
/// [`CONE_SHADOW_EXPOSURE_THRESHOLD_LUX`]. `exposure_scale` is a linear multiplier, not an EV
/// value. Manual endpoints replace their respective automatic values and are clamped to retain a
/// valid perspective projection.
pub(crate) fn resolve_cone_shadow_range(light: ConeLight, exposure_scale: f32) -> (f32, f32) {
	let peak_candela = cone_light_peak_candela(light);
	let exposure_scale = if exposure_scale.is_finite() {
		exposure_scale
	} else {
		CONE_SHADOW_DEFAULT_EXPOSURE_SCALE
	}
	.max(0.0);
	let automatic_far = (peak_candela * exposure_scale / CONE_SHADOW_EXPOSURE_THRESHOLD_LUX)
		.sqrt()
		.max(CONE_SHADOW_NEAR_M + CONE_SHADOW_NEAR_M);
	let near = light
		.shadow_near_override()
		.filter(|value| value.is_finite())
		.unwrap_or(CONE_SHADOW_NEAR_M)
		.max(CONE_SHADOW_NEAR_M);
	let far = light
		.shadow_far_override()
		.filter(|value| value.is_finite())
		.unwrap_or(automatic_far)
		.max(near + CONE_SHADOW_NEAR_M);

	(near, far)
}

/// Returns whether a cone has finite positive luminance that can cast a visible shadow.
pub(crate) fn cone_light_has_brightness(light: ConeLight) -> bool {
	let peak_candela = cone_light_peak_candela(light);
	peak_candela.is_finite() && peak_candela > 0.0
}

/// Returns the luminance-weighted luminous intensity used for cone shadow coverage.
pub(crate) fn cone_light_peak_candela(light: ConeLight) -> f32 {
	0.2126 * light.color.x + 0.7152 * light.color.y + 0.0722 * light.color.z
}

/// Resolves the clipping range for one point-light cube shadow map.
pub(crate) fn resolve_point_shadow_range(light: PointLight, exposure_scale: f32) -> (f32, f32) {
	let peak_candela = point_light_peak_candela(light);
	let exposure_scale = if exposure_scale.is_finite() {
		exposure_scale
	} else {
		POINT_SHADOW_DEFAULT_EXPOSURE_SCALE
	}
	.max(0.0);
	let automatic_far = (peak_candela * exposure_scale / POINT_SHADOW_EXPOSURE_THRESHOLD_LUX)
		.sqrt()
		.max(POINT_SHADOW_NEAR_M + POINT_SHADOW_NEAR_M);
	let near = light
		.shadow_near_override()
		.filter(|value| value.is_finite())
		.unwrap_or(POINT_SHADOW_NEAR_M)
		.max(POINT_SHADOW_NEAR_M);
	let far = light
		.shadow_far_override()
		.filter(|value| value.is_finite())
		.unwrap_or(automatic_far)
		.max(near + POINT_SHADOW_NEAR_M);

	(near, far)
}

/// Returns whether a point light has finite positive luminance that can cast a visible shadow.
pub(crate) fn point_light_has_brightness(light: PointLight) -> bool {
	let peak_candela = point_light_peak_candela(light);
	peak_candela.is_finite() && peak_candela > 0.0
}

/// Returns the luminance-weighted luminous intensity used for point shadow coverage.
pub(crate) fn point_light_peak_candela(light: PointLight) -> f32 {
	0.2126 * light.color.x + 0.7152 * light.color.y + 0.0722 * light.color.z
}

/// Builds the perspective view used to cull and render one cone-light shadow layer.
pub(crate) fn make_cone_shadow_view(light: ConeLight, exposure_scale: f32) -> View {
	let (near, far) = resolve_cone_shadow_range(light, exposure_scale);
	View::new_perspective(
		(light.outer_angle * 2.0).to_degrees(),
		1.0,
		near,
		far,
		light.position,
		light.direction,
	)
}

/// Builds one of the six perspective views used to render a point-light cube shadow map.
pub(crate) fn make_point_shadow_view(light: PointLight, face: usize, exposure_scale: f32) -> View {
	let (near, far) = resolve_point_shadow_range(light, exposure_scale);
	let (direction, up) = match face {
		0 => (UnitVector::x_axis(), UnitVector::y_axis()),
		1 => (-UnitVector::x_axis(), UnitVector::y_axis()),
		2 => (UnitVector::y_axis(), -UnitVector::z_axis()),
		3 => (-UnitVector::y_axis(), UnitVector::z_axis()),
		4 => (UnitVector::z_axis(), UnitVector::y_axis()),
		5 => (-UnitVector::z_axis(), UnitVector::y_axis()),
		_ => unreachable!("Point shadow face is invalid. The most likely cause is a cube map dispatch outside its six faces."),
	};
	View::new_perspective_with_up(90.0, 1.0, near, far, light.position, direction, up)
}

/// Returns the conservative sphere that bounds cone-shadow coverage.
pub(crate) fn cone_shadow_bounds(light: ConeLight) -> math::Sphere {
	let (_, far) = resolve_cone_shadow_range(light, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE);
	let cosine = light.outer_angle.cos();
	let enclosing_radius = far / (2.0 * cosine * cosine);
	math::Sphere::new(light.position + light.direction * enclosing_radius, enclosing_radius)
}

/// Returns the conservative sphere that bounds point-shadow coverage.
pub(crate) fn point_shadow_bounds(light: PointLight) -> math::Sphere {
	let (_, far) = resolve_point_shadow_range(light, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE);
	math::Sphere::new(light.position, far)
}

/// Returns the estimated screen coverage of a cone-shadow candidate in one sink.
pub(crate) fn cone_shadow_importance(light: ConeLight, sink: &Sink) -> Option<f32> {
	shadow_view_importance(cone_shadow_bounds(light), sink)
}

/// Returns the estimated screen coverage of a point-shadow candidate in one sink.
pub(crate) fn point_shadow_importance(light: PointLight, sink: &Sink) -> Option<f32> {
	shadow_view_importance(point_shadow_bounds(light), sink)
}

/// Returns the estimated number of sink pixels covered by a local light's conservative bound.
///
/// This projection is only a ranking proxy for assigning existing shadow views. It does not alter
/// light culling, shadow-map dimensions, or a light's shadow projection.
pub(crate) fn shadow_view_importance(bounds: math::Sphere, sink: &Sink) -> Option<f32> {
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
	let (radius_x, radius_y) = if view.y_fov() > 0.0 {
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
