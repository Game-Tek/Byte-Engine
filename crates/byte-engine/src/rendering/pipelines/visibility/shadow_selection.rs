//! Chooses which scene lights receive shadow views this frame and builds those views.
//!
//! Every shadow-casting light draws its maps from one memory budget, [`ShadowBudget`]. The first directional light
//! claims its cascades first. Cone and point lights then compete in one ranking by how many sink pixels their
//! conservative bounds cover. Other directional lights take what remains. Next, size the shadow maps with
//! [`retain_layout`] and render the views with [`super::render_pass::ShadowMaps::prepare`].

use ghi::Size as _;
use maths_rs::Vec4f;
use smallvec::SmallVec;

use super::layout::{
	CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET, DIRECTIONAL_SHADOW_MAP_FORMAT,
	DIRECTIONAL_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_COUNT, MAX_DIRECTIONAL_SHADOW_COUNT, MAX_LIGHTS, MAX_POINT_SHADOW_COUNT,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_FORMAT, POINT_SHADOW_MAP_RESOLUTION, POINT_SHADOW_VIEW_OFFSET,
	SHADOW_CASCADE_COUNT,
};
use crate::gameplay::Transform;
use crate::rendering::lights::{ConeLight, Lights, LocalEmission, PointLight};
use crate::rendering::{Sink, View};
use crate::space::{Orientable as _, Positionable as _};

/// The minimum distance from a local light covered by an automatic shadow view.
pub(crate) const SHADOW_NEAR_M: f32 = 0.1;
/// The linear exposure multiplier used until a camera provides an exposure value.
pub(crate) const SHADOW_DEFAULT_EXPOSURE_SCALE: f32 = 1.0;
/// The exposure-weighted peak illuminance below which a local light stops casting shadows.
pub(crate) const SHADOW_EXPOSURE_THRESHOLD_LUX: f32 = 0.125;

/// The `ShadowBudget` struct holds the shadow-map memory every light shares and what one light of each kind costs.
///
/// Build it from the pipeline settings with [`Self::new`] and pass it to [`select_shadow_lights`] and [`retain_layout`]
/// every frame. Costs count shadow-map texels only: each sun's depth pyramids, a thirty-second of its cascades, and the
/// padding a driver may add are not counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShadowBudget {
	pub(crate) bytes: u64,
	/// One sun's cascades.
	pub(crate) directional_bytes: u64,
	pub(crate) cone_bytes: u64,
	/// One point light's six cube faces.
	pub(crate) point_bytes: u64,
}

impl ShadowBudget {
	/// Prices each light kind for a budget of `mebibytes` and directional cascades of `cascade_resolution` texels per
	/// side.
	pub(crate) fn new(mebibytes: u32, cascade_resolution: u32) -> Self {
		let map_bytes =
			|format: ghi::Formats, resolution: u32| format.size() as u64 * u64::from(resolution) * u64::from(resolution);
		Self {
			bytes: u64::from(mebibytes) << 20,
			directional_bytes: map_bytes(DIRECTIONAL_SHADOW_MAP_FORMAT, cascade_resolution) * SHADOW_CASCADE_COUNT as u64,
			cone_bytes: map_bytes(CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_MAP_RESOLUTION),
			point_bytes: map_bytes(POINT_SHADOW_MAP_FORMAT, POINT_SHADOW_MAP_RESOLUTION) * POINT_SHADOW_FACE_COUNT as u64,
		}
	}

	/// Returns the bytes the shadow maps of `layout` occupy.
	pub(crate) fn cost(&self, layout: ShadowLayout) -> u64 {
		layout.suns as u64 * self.directional_bytes
			+ layout.cones as u64 * self.cone_bytes
			+ layout.points as u64 * self.point_bytes
	}
}

/// The `ShadowLayout` struct counts the lights of each kind that the shadow-map images hold maps for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShadowLayout {
	pub(crate) suns: usize,
	pub(crate) cones: usize,
	pub(crate) points: usize,
}

/// Returns how many maps of each kind the shadow-map images keep this frame.
///
/// Images keep the maps they already hold while those and this frame's `needed` maps fit the budget together, so a
/// light leaving view does not reallocate an image. When they do not fit, every image shrinks to what this frame
/// needs, which [`select_shadow_lights`] keeps within the budget. Memory therefore moves to another kind of light only
/// when that kind needs it.
pub(crate) fn retain_layout(previous: ShadowLayout, needed: ShadowLayout, budget: &ShadowBudget) -> ShadowLayout {
	let retained = ShadowLayout {
		suns: previous.suns.max(needed.suns),
		cones: previous.cones.max(needed.cones),
		points: previous.points.max(needed.points),
	};
	if budget.cost(retained) <= budget.bytes {
		retained
	} else {
		needed
	}
}

/// The `SunShadow` struct describes one directional light that holds cascades this frame.
///
/// Its position in [`ShadowLightSelection::suns`] is its sun slot, which picks its cascade views, its shadow-map
/// layers, and its sun-visibility layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SunShadow {
	/// The scene light index.
	pub(crate) index: usize,
	/// The world-space direction the light travels.
	pub(crate) direction: math::UnitVector,
	/// The tangent of the light's angular radius, which sizes its penumbrae.
	pub(crate) angular_radius_tangent: f32,
}

/// The `LightShadow` enum is the shadow assignment encoded into one GPU light record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightShadow {
	None,
	Directional { slot: u32 },
	Cone { view_index: u32, layer: u32 },
	Point { view_index: u32, cube_index: u32 },
}

/// The inline capacity of a frame's shadow assignments: the smallest size `SmallVec` supports that holds every sun and
/// every local shadow.
const SHADOW_ASSIGNMENTS_INLINE: usize = 36;
const _: () =
	assert!(MAX_DIRECTIONAL_SHADOW_COUNT + MAX_CONE_SHADOW_COUNT + MAX_POINT_SHADOW_COUNT <= SHADOW_ASSIGNMENTS_INLINE);

/// The `ShadowLightSelection` struct retains the lights that hold shadow maps this frame, within the shared budget.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadowLightSelection<'a> {
	/// Shadowed suns by sun slot.
	pub(crate) suns: SmallVec<[SunShadow; MAX_DIRECTIONAL_SHADOW_COUNT]>,
	/// Shadowed cones by shadow-map layer.
	pub(crate) cones: SmallVec<[(usize, &'a ConeLight, &'a Transform); MAX_CONE_SHADOW_COUNT]>,
	/// Shadowed points by cube index.
	pub(crate) points: SmallVec<[(usize, &'a PointLight, &'a Transform); MAX_POINT_SHADOW_COUNT]>,
	/// Directional lights, and local lights some sink sees, that are lit but hold no shadow map this frame.
	pub(crate) unshadowed_count: usize,
	/// Every selected light's shadow, sorted by light index, so the lighting upload looks each light up once.
	assignments: SmallVec<[(usize, LightShadow); SHADOW_ASSIGNMENTS_INLINE]>,
}

impl ShadowLightSelection<'_> {
	/// Returns how many maps of each kind this selection draws.
	pub(crate) fn layout(&self) -> ShadowLayout {
		ShadowLayout {
			suns: self.suns.len(),
			cones: self.cones.len(),
			points: self.points.len(),
		}
	}

	/// Returns the shadow assignment of the scene light at `light_index`.
	pub(crate) fn shadow_for(&self, light_index: usize) -> LightShadow {
		self.assignments
			.binary_search_by_key(&light_index, |(index, _)| *index)
			.map_or(LightShadow::None, |position| self.assignments[position].1)
	}

	/// Records every selected light's shadow in light order for [`Self::shadow_for`].
	fn index_assignments(&mut self) {
		let suns = self
			.suns
			.iter()
			.enumerate()
			.map(|(slot, sun)| (sun.index, LightShadow::Directional { slot: slot as u32 }));
		let cones = self.cones.iter().enumerate().map(|(layer, (index, ..))| {
			(
				*index,
				LightShadow::Cone {
					view_index: (CONE_SHADOW_VIEW_OFFSET + layer) as u32,
					layer: layer as u32,
				},
			)
		});
		let points = self.points.iter().enumerate().map(|(cube_index, (index, ..))| {
			(
				*index,
				LightShadow::Point {
					view_index: (POINT_SHADOW_VIEW_OFFSET + cube_index * POINT_SHADOW_FACE_COUNT) as u32,
					cube_index: cube_index as u32,
				},
			)
		});
		self.assignments = suns.chain(cones).chain(points).collect();
		self.assignments.sort_unstable_by_key(|(index, _)| *index);
	}
}

/// Returns the first view of the cascades of sun slot `slot`.
pub(crate) fn sun_cascade_view(slot: usize) -> usize {
	DIRECTIONAL_SHADOW_VIEW_OFFSET + slot * SHADOW_CASCADE_COUNT
}

/// Returns the luminance-weighted luminous intensity used for shadow coverage.
pub(crate) fn peak_candela(emission: &LocalEmission, intensity_scale_candela: f32) -> f32 {
	let color = emission.color;
	utils::color::rec709_luminance(color.x, color.y, color.z) * intensity_scale_candela
}

/// Returns whether a local light has finite positive luminance that can cast a visible shadow.
pub(crate) fn has_brightness(emission: &LocalEmission, intensity_scale_candela: f32) -> bool {
	let peak = peak_candela(emission, intensity_scale_candela);
	peak.is_finite() && peak > 0.0
}

/// Resolves the clipping range of one local shadow view.
///
/// The far distance is where the light's exposure-weighted peak illuminance reaches
/// [`SHADOW_EXPOSURE_THRESHOLD_LUX`]. Manual endpoints replace their automatic values and are clamped to
/// retain a valid perspective projection.
pub(crate) fn resolve_shadow_range(emission: &LocalEmission, exposure_scale: f32, intensity_scale_candela: f32) -> (f32, f32) {
	let exposure_scale = if exposure_scale.is_finite() {
		exposure_scale
	} else {
		SHADOW_DEFAULT_EXPOSURE_SCALE
	}
	.max(0.0);
	let automatic_far = (peak_candela(emission, intensity_scale_candela) * exposure_scale / SHADOW_EXPOSURE_THRESHOLD_LUX)
		.sqrt()
		.max(SHADOW_NEAR_M + SHADOW_NEAR_M);
	let near = emission
		.shadow_near_override()
		.filter(|value| value.is_finite())
		.unwrap_or(SHADOW_NEAR_M)
		.max(SHADOW_NEAR_M);
	let far = emission
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
	let (near, far) = resolve_shadow_range(&light.emission, exposure_scale, intensity_scale_candela);
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
	let (near, far) = resolve_shadow_range(&light.emission, exposure_scale, intensity_scale_candela);
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

/// Returns the estimated screen coverage of a cone-shadow candidate in one sink whose view has `frustum` planes.
pub(crate) fn cone_shadow_importance(
	light: &ConeLight,
	transform: &Transform,
	intensity_scale_candela: f32,
	sink: &Sink,
	frustum: &[math::Plane; 6],
) -> Option<f32> {
	let (_, far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, intensity_scale_candela);
	let cosine = light.outer_angle.cos();
	let enclosing_radius = far / (2.0 * cosine * cosine);
	let bounds = math::Sphere::new(
		transform.position() + math::direction_from_orientation(transform.orientation()) * enclosing_radius,
		enclosing_radius,
	);
	shadow_view_importance(bounds, sink, frustum)
}

/// Returns the estimated screen coverage of a point-shadow candidate in one sink whose view has `frustum` planes.
pub(crate) fn point_shadow_importance(
	light: &PointLight,
	transform: &Transform,
	intensity_scale_candela: f32,
	sink: &Sink,
	frustum: &[math::Plane; 6],
) -> Option<f32> {
	let (_, far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, intensity_scale_candela);
	shadow_view_importance(math::Sphere::new(transform.position(), far), sink, frustum)
}

/// Returns the estimated number of sink pixels covered by a local light's conservative bound, or `None` when culled.
///
/// This projection is only a ranking proxy for assigning existing shadow views. It does not alter light
/// culling, shadow-map dimensions, or a light's shadow projection.
fn shadow_view_importance(bounds: math::Sphere, sink: &Sink, frustum: &[math::Plane; 6]) -> Option<f32> {
	let view = sink.view();
	if !math::collision::sphere_in_frustum(&bounds, frustum) {
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

/// A local light kind that competes for shadow-map memory.
#[derive(Clone, Copy)]
enum LocalLight<'a> {
	Cone(&'a ConeLight),
	Point(&'a PointLight),
}

/// One local light eligible for a shadow-view assignment.
#[derive(Clone, Copy)]
struct Candidate<'a> {
	index: usize,
	light: LocalLight<'a>,
	transform: &'a Transform,
}

/// The most ranks one sink keeps.
///
/// A sink moves past a rank when that rank's light is already selected, when its kind has no map left, or when it
/// costs more memory than remains. At most every local map can be selected, so a sink that keeps one more candidate
/// than that still has a candidate to offer in nearly every round.
const MAX_RANKED_CANDIDATES: usize = MAX_CONE_SHADOW_COUNT + MAX_POINT_SHADOW_COUNT + 1;
/// The inline capacity of one sink's ranking: the smallest size `SmallVec` supports that holds its candidates.
const RANKING_INLINE: usize = 36;
const _: () = assert!(MAX_RANKED_CANDIDATES <= RANKING_INLINE);

/// The `SinkRanking` struct keeps one sink's best local shadow candidates, so selection ranks every light once instead
/// of once per shadow map.
#[derive(Default)]
struct SinkRanking<'a> {
	/// Candidates and their projected coverage, best first.
	candidates: SmallVec<[(f32, Candidate<'a>); RANKING_INLINE]>,
}

impl<'a> SinkRanking<'a> {
	/// Inserts a candidate by projected coverage. An earlier scene light stays ahead of a later one with equal coverage.
	fn insert(&mut self, importance: f32, candidate: Candidate<'a>) {
		let position = self
			.candidates
			.iter()
			.position(|(ranked, _)| importance.total_cmp(ranked).is_gt())
			.unwrap_or(self.candidates.len());
		if position >= MAX_RANKED_CANDIDATES {
			return;
		}
		if self.candidates.len() == MAX_RANKED_CANDIDATES {
			self.candidates.pop();
		}
		self.candidates.insert(position, (importance, candidate));
	}
}

/// Selects the shadow-casting lights for this frame from the light prefix uploaded to material evaluation.
///
/// The first directional light claims its cascades first. Cone and point lights then share one sink-fair ranking, and
/// the other directional lights, in scene order, take the memory that remains. `intensity_scale_candela` returns the
/// calibrated IES peak scale of the light at each index, or `1.0` for analytic lights.
pub(crate) fn select_shadow_lights<'a>(
	lights: impl Iterator<Item = (&'a Lights, &'a Transform)>,
	sinks: &[Sink],
	budget: &ShadowBudget,
	intensity_scale_candela: impl Fn(usize) -> f32,
) -> ShadowLightSelection<'a> {
	let mut selection = ShadowLightSelection::default();
	if sinks.is_empty() {
		return selection;
	}
	// Four sinks stay inline, matching the recorded-sink list of the pipeline manager.
	let mut rankings = sinks
		.iter()
		.map(|_| SinkRanking::default())
		.collect::<SmallVec<[SinkRanking<'a>; 4]>>();
	// Each sink's frustum planes depend only on its view, so the first candidate a sink ranks extracts them for all.
	let mut frusta = sinks
		.iter()
		.map(|_| None)
		.collect::<SmallVec<[Option<[math::Plane; 6]>; 4]>>();
	let mut suns = SmallVec::<[SunShadow; MAX_DIRECTIONAL_SHADOW_COUNT]>::new();
	let mut shadow_casters = 0;

	for (index, (light, transform)) in lights.take(MAX_LIGHTS).enumerate() {
		let scale = intensity_scale_candela(index);
		let candidate = |light| Candidate { index, light, transform };
		let ranked = match light {
			Lights::Direction(light) => {
				shadow_casters += 1;
				if suns.len() < MAX_DIRECTIONAL_SHADOW_COUNT {
					suns.push(SunShadow {
						index,
						direction: math::direction_from_orientation(transform.orientation()),
						angular_radius_tangent: light.angular_radius.value().tan(),
					});
				}
				false
			}
			Lights::Cone(light) if has_brightness(&light.emission, scale) && light.supports_shadow_mapping() => rank(
				&mut rankings,
				sinks,
				&mut frusta,
				candidate(LocalLight::Cone(light)),
				|sink, frustum| cone_shadow_importance(light, transform, scale, sink, frustum),
			),
			Lights::Point(light) if has_brightness(&light.emission, scale) => rank(
				&mut rankings,
				sinks,
				&mut frusta,
				candidate(LocalLight::Point(light)),
				|sink, frustum| point_shadow_importance(light, transform, scale, sink, frustum),
			),
			_ => false,
		};
		shadow_casters += usize::from(ranked);
	}

	// The first sun claims its cascades before local lights, the others after them.
	let mut remaining = budget.bytes;
	let (primary, others) = suns.split_at(suns.len().min(1));
	for sun in primary {
		if claim(&mut remaining, budget.directional_bytes) {
			selection.suns.push(*sun);
		}
	}
	select_fair(&rankings, budget, &mut remaining, &mut selection);
	for sun in others {
		if claim(&mut remaining, budget.directional_bytes) {
			selection.suns.push(*sun);
		}
	}
	selection.unshadowed_count = shadow_casters - selection.suns.len() - selection.cones.len() - selection.points.len();
	selection.index_assignments();
	selection
}

/// Ranks one candidate for every sink that sees it, and returns whether any sink does.
///
/// `frusta` caches each sink's frustum planes, extracted the first time any candidate needs them.
fn rank<'a>(
	rankings: &mut [SinkRanking<'a>],
	sinks: &[Sink],
	frusta: &mut [Option<[math::Plane; 6]>],
	candidate: Candidate<'a>,
	importance: impl Fn(&Sink, &[math::Plane; 6]) -> Option<f32>,
) -> bool {
	let mut visible = false;
	for ((ranking, sink), frustum) in rankings.iter_mut().zip(sinks).zip(frusta) {
		let frustum = frustum.get_or_insert_with(|| sink.view().get_frustum_planes());
		if let Some(importance) = importance(sink, frustum) {
			ranking.insert(importance, candidate);
			visible = true;
		}
	}
	visible
}

/// Assigns local shadow maps in sink-priority rounds so no sink can starve another, while `remaining` bytes last.
///
/// Advancing all sinks together prevents a sink's changing coverage from displacing another sink's turn. A partial
/// final round favors earlier sinks. A light that costs more than what remains, or whose kind has no map left, is
/// passed over, so a cheaper light ranked below it can still use the memory.
fn select_fair<'a>(
	rankings: &[SinkRanking<'a>],
	budget: &ShadowBudget,
	remaining: &mut u64,
	selection: &mut ShadowLightSelection<'a>,
) {
	for priority in 0..MAX_RANKED_CANDIDATES {
		for ranking in rankings {
			let Some((_, candidate)) = ranking.candidates.get(priority) else {
				continue;
			};
			let Candidate { index, light, transform } = *candidate;
			match light {
				LocalLight::Cone(light) => {
					if !holds_map(&selection.cones, index)
						&& selection.cones.len() < MAX_CONE_SHADOW_COUNT
						&& claim(remaining, budget.cone_bytes)
					{
						selection.cones.push((index, light, transform));
					}
				}
				LocalLight::Point(light) => {
					if !holds_map(&selection.points, index)
						&& selection.points.len() < MAX_POINT_SHADOW_COUNT
						&& claim(remaining, budget.point_bytes)
					{
						selection.points.push((index, light, transform));
					}
				}
			}
		}
	}
}

/// Returns whether the scene light at `index` already holds one of `maps`.
fn holds_map<T>(maps: &[(usize, T, &Transform)], index: usize) -> bool {
	maps.iter().any(|(selected, ..)| *selected == index)
}

/// Takes `cost` bytes from `remaining` and returns whether they fit.
fn claim(remaining: &mut u64, cost: u64) -> bool {
	let fits = cost <= *remaining;
	if fits {
		*remaining -= cost;
	}
	fits
}

#[cfg(test)]
mod tests {
	use math::{Point, UnitVector};
	use maths_rs::Vec3f;
	use utils::Extent;

	use super::*;
	use crate::rendering::lights::{DirectionalLight, LightColor, PhotometricIntensity};

	/// Prices a sun at eight units, a cone at one, and a point at six, so tests state budgets in those units.
	fn budget(bytes: u64) -> ShadowBudget {
		ShadowBudget {
			bytes,
			directional_bytes: 8,
			cone_bytes: 1,
			point_bytes: 6,
		}
	}

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

	fn directional() -> Lights {
		Lights::Direction(
			DirectionalLight::new(
				LightColor::Kelvin(6_500.0),
				PhotometricIntensity::Illuminance {
					lux: 100_000.0,
					measurement_distance_m: 1.0,
				},
			)
			.expect("physical directional light"),
		)
	}

	fn sun_transform() -> Transform {
		Transform::from_rotation(math::orientation_from_direction(-UnitVector::<math::WorldSpace>::y_axis()))
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
		budget: ShadowBudget,
	) -> ShadowLightSelection<'a> {
		select_shadow_lights(lights.iter().zip(transforms), sinks, &budget, |_| 1.0)
	}

	fn sun_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.suns.iter().map(|sun| sun.index).collect()
	}

	fn cone_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.cones.iter().map(|(index, ..)| *index).collect()
	}

	fn point_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.points.iter().map(|(index, ..)| *index).collect()
	}

	#[test]
	fn shadow_selection_shares_the_budget_between_the_sun_cones_and_points() {
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
		let lights = [
			Lights::Cone(wide_cone),
			Lights::Cone(cone()),
			Lights::Point(point()),
			directional(),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
			Lights::Cone(cone()),
		];
		let transforms = [
			light_transform(0.0),
			light_transform(0.0),
			light_transform(0.0),
			sun_transform(),
			light_transform(1.0),
			light_transform(2.0),
			light_transform(3.0),
			light_transform(4.0),
		];

		// The sun, one point, and four cones fit.
		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(8 + 6 + 4));

		assert_eq!(sun_indices(&selection), [3]);
		assert_eq!(cone_indices(&selection), [1, 4, 5, 6]);
		assert_eq!(point_indices(&selection), [2]);
		assert_eq!(selection.unshadowed_count, 1);
		assert_eq!(selection.shadow_for(3), LightShadow::Directional { slot: 0 });
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
	fn the_first_sun_claims_the_budget_before_local_lights_that_cover_more_of_the_screen() {
		let lights = [Lights::Cone(cone()), directional()];
		let transforms = [light_transform(0.0), sun_transform()];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(8));

		assert_eq!(sun_indices(&selection), [1]);
		assert!(selection.cones.is_empty());
		assert_eq!(selection.unshadowed_count, 1);
	}

	#[test]
	fn a_cone_ranked_below_a_point_that_does_not_fit_uses_the_remaining_budget() {
		let lights = [Lights::Point(point()), Lights::Cone(cone())];
		let transforms = [light_transform(0.0), light_transform(0.0)];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(5));

		assert!(selection.points.is_empty());
		assert_eq!(cone_indices(&selection), [1]);
		assert_eq!(selection.unshadowed_count, 1);
	}

	#[test]
	fn other_suns_take_only_the_budget_local_lights_leave() {
		let lights = [directional(), directional(), directional(), Lights::Cone(cone())];
		let transforms = [sun_transform(), sun_transform(), sun_transform(), light_transform(0.0)];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(8 + 1 + 8));

		assert_eq!(sun_indices(&selection), [0, 1]);
		assert_eq!(cone_indices(&selection), [3]);
		assert_eq!(selection.unshadowed_count, 1);
		assert_eq!(selection.shadow_for(1), LightShadow::Directional { slot: 1 });
	}

	#[test]
	fn suns_use_the_whole_budget_without_local_lights() {
		let lights = [directional(), directional(), directional()];
		let transforms = [sun_transform(), sun_transform(), sun_transform()];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(3 * 8));

		assert_eq!(sun_indices(&selection), [0, 1, 2]);
		assert_eq!(selection.unshadowed_count, 0);
	}

	#[test]
	fn a_budget_smaller_than_one_sun_leaves_it_to_local_lights() {
		let lights = [directional(), Lights::Cone(cone())];
		let transforms = [sun_transform(), light_transform(0.0)];

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(7));

		assert!(selection.suns.is_empty());
		assert_eq!(cone_indices(&selection), [1]);
		assert_eq!(selection.unshadowed_count, 1);
	}

	#[test]
	fn retained_layout_keeps_maps_while_they_fit_and_shrinks_to_the_frame_when_not() {
		let previous = ShadowLayout {
			suns: 1,
			cones: 4,
			points: 0,
		};
		let fewer_cones = ShadowLayout { cones: 1, ..previous };
		let one_point = ShadowLayout {
			points: 1,
			..fewer_cones
		};

		assert_eq!(retain_layout(previous, fewer_cones, &budget(8 + 4)), previous);
		assert_eq!(retain_layout(previous, one_point, &budget(8 + 1 + 6)), one_point);
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

		assert!(sinks.iter().any(|sink| {
			let frustum = sink.view().get_frustum_planes();
			cone_shadow_importance(&visible_in_second_sink, &transforms[0], 1.0, sink, &frustum).is_some()
		}));
		assert!(sinks.iter().all(|sink| {
			let frustum = sink.view().get_frustum_planes();
			cone_shadow_importance(&outside_all_sinks, &transforms[1], 1.0, sink, &frustum).is_none()
		}));

		let selection = select(&lights, &transforms, &sinks, budget(64));

		assert_eq!(cone_indices(&selection), [0]);
		assert_eq!(selection.unshadowed_count, 0);
	}

	#[test]
	fn cone_shadows_continue_in_sink_order_after_assigning_each_sink_its_top_light() {
		let lights: Vec<_> = (0..6).map(|_| Lights::Cone(cone().with_shadow_far(20.0))).collect();
		let transforms = [0.0, 1.0, 2.0, 3.0, 100.0, 200.0].map(light_transform);
		let sinks = [
			sink(Point::origin()),
			sink(Point::new(100.0, 0.0, 0.0)),
			sink(Point::new(200.0, 0.0, 0.0)),
		];

		let selection = select(&lights, &transforms, &sinks, budget(4));

		assert_eq!(cone_indices(&selection), [0, 4, 5, 1]);
	}

	#[test]
	fn point_shadows_rank_every_light_in_a_large_light_table() {
		// Every light is in view, and coverage falls with distance, so the last light covers the most and the first
		// light the next most.
		let lights: Vec<_> = (0..200).map(|_| Lights::Point(point().with_shadow_far(1.0))).collect();
		let mut transforms: Vec<_> = (0..200)
			.map(|index| Transform::from_position(Point::new(0.0, 0.0, 3.0 + index as f32 * 0.4)))
			.collect();
		transforms[199] = Transform::from_position(Point::new(0.0, 0.0, 2.0));

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(2 * 6));

		assert_eq!(point_indices(&selection), [199, 0]);
		assert_eq!(selection.unshadowed_count, 198);
	}

	#[test]
	fn unlit_cones_yield_shadow_maps_to_visible_lit_cones() {
		let mut unlit = cone();
		unlit.emission.color = Vec3f::new(0.0, 0.0, 0.0);
		let lights = [Lights::Cone(unlit.clone()), Lights::Cone(cone())];
		let transforms = [light_transform(0.0), light_transform(1.0)];

		assert!(!has_brightness(&unlit.emission, 1.0));

		let selection = select(&lights, &transforms, &[sink(Point::origin())], budget(1));

		assert_eq!(cone_indices(&selection), [1]);
		assert_eq!(selection.unshadowed_count, 0);
	}

	/// Verifies a resident profile's dimmed peak intensity drives both local-shadow range and selection.
	#[test]
	fn ies_profile_scale_expands_point_shadow_coverage() {
		let light = PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.5, "lights/office.ies")
			.expect("physical IES point light");
		let lights = [Lights::Point(light.clone())];
		let transforms = [light_transform(20.0)];
		let sinks = [sink(Point::origin())];

		let fallback = select(&lights, &transforms, &sinks, budget(6));
		let resident = select_shadow_lights(lights.iter().zip(&transforms), &sinks, &budget(6), |_| 90.0);
		let (_, fallback_far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let (_, resident_far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 90.0);

		assert!(fallback.points.is_empty());
		assert_eq!(fallback.unshadowed_count, 0);
		assert_eq!(point_indices(&resident), [0]);
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
		let (near, far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let automatic_far = (100.0 / SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert_eq!(near, SHADOW_NEAR_M);
		assert!((far - automatic_far).abs() < 0.0001);

		let light = cone().with_shadow_near(50.0).with_shadow_far(20.0);
		assert_eq!(
			resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0),
			(50.0, 50.1)
		);
	}

	#[test]
	fn cone_shadow_range_scales_with_linear_exposure() {
		let light = cone();
		let (_, neutral_far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let (_, brighter_far) = resolve_shadow_range(&light.emission, 4.0, 1.0);
		let (_, invalid_far) = resolve_shadow_range(&light.emission, f32::NAN, 1.0);

		assert!((brighter_far - neutral_far * 2.0).abs() < 0.0001);
		assert!((invalid_far - neutral_far).abs() < 0.0001);
	}
}
