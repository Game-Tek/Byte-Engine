//! Chooses which scene lights receive shadow views this frame and builds those views.
//!
//! Every shadow-casting light draws its maps from one memory budget, [`ShadowBudget`]. The first directional light
//! claims its cascades first. Cone and point lights then compete in one ranking by how many sink pixels their
//! conservative bounds cover. Other directional lights take what remains. Each cone and point light's map takes the
//! power-of-two size class its bound's largest span over every sink needs, see [`size_class_for_span`]. Next, size the
//! shadow maps with [`retain_layout`] and render the views with [`super::render_pass::ShadowMaps::prepare`].

use ghi::Size as _;
use maths_rs::Vec4f;
use smallvec::SmallVec;

use super::layout::{
	CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_VIEW_OFFSET, DIRECTIONAL_SHADOW_MAP_FORMAT, DIRECTIONAL_SHADOW_VIEW_OFFSET,
	LOCAL_SHADOW_SIZE_CLASS_COUNT, MAX_CONE_SHADOW_COUNT, MAX_DIRECTIONAL_SHADOW_COUNT, MAX_LIGHTS, MAX_POINT_SHADOW_COUNT,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_FORMAT, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT,
	local_shadow_map_resolution,
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
	/// One cone map of each size class.
	pub(crate) cone_bytes: [u64; LOCAL_SHADOW_SIZE_CLASS_COUNT],
	/// One point light's six cube faces of each size class.
	pub(crate) point_bytes: [u64; LOCAL_SHADOW_SIZE_CLASS_COUNT],
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
			cone_bytes: std::array::from_fn(|size_class| {
				map_bytes(CONE_SHADOW_MAP_FORMAT, local_shadow_map_resolution(size_class))
			}),
			point_bytes: std::array::from_fn(|size_class| {
				map_bytes(POINT_SHADOW_MAP_FORMAT, local_shadow_map_resolution(size_class)) * POINT_SHADOW_FACE_COUNT as u64
			}),
		}
	}

	/// Returns the bytes the shadow maps of `layout` occupy.
	pub(crate) fn cost(&self, layout: ShadowLayout) -> u64 {
		let classes = |counts: [usize; LOCAL_SHADOW_SIZE_CLASS_COUNT], bytes: [u64; LOCAL_SHADOW_SIZE_CLASS_COUNT]| {
			counts
				.iter()
				.zip(bytes)
				.map(|(count, bytes)| *count as u64 * bytes)
				.sum::<u64>()
		};
		layout.suns as u64 * self.directional_bytes
			+ classes(layout.cones, self.cone_bytes)
			+ classes(layout.points, self.point_bytes)
	}
}

/// The `ShadowLayout` struct counts the lights of each kind, and of each size class for cone and point lights, that the
/// shadow-map images hold maps for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShadowLayout {
	pub(crate) suns: usize,
	pub(crate) cones: [usize; LOCAL_SHADOW_SIZE_CLASS_COUNT],
	pub(crate) points: [usize; LOCAL_SHADOW_SIZE_CLASS_COUNT],
}

/// Returns the size class of a cone map or point cube face whose light spans `span` sink pixels at most, given the
/// class it held last frame.
///
/// A light grows to the smallest class with at least one texel per pixel of its span at once. It shrinks only once its
/// span falls below three quarters of a smaller class, so a span that hovers at a power of two does not resize its map
/// every frame.
pub(crate) fn size_class_for_span(span: f32, previous: Option<usize>) -> usize {
	let needed = |span: f32| {
		(0..LOCAL_SHADOW_SIZE_CLASS_COUNT)
			.find(|&size_class| local_shadow_map_resolution(size_class) as f32 >= span)
			.unwrap_or(LOCAL_SHADOW_SIZE_CLASS_COUNT - 1)
	};
	match previous {
		Some(previous) if previous > needed(span) => needed(span * (4.0 / 3.0)).min(previous),
		_ => needed(span),
	}
}

/// Returns how many maps of each kind the shadow-map images keep this frame.
///
/// Images keep the maps they already hold while those and this frame's `needed` maps fit the budget together, so a
/// light leaving view does not reallocate an image. When they do not fit, every image shrinks to what this frame
/// needs, which [`select_shadow_lights`] keeps within the budget. Memory therefore moves to another kind of light only
/// when that kind needs it.
pub(crate) fn retain_layout(previous: ShadowLayout, needed: ShadowLayout, budget: &ShadowBudget) -> ShadowLayout {
	let classes = |previous: [usize; LOCAL_SHADOW_SIZE_CLASS_COUNT], needed: [usize; LOCAL_SHADOW_SIZE_CLASS_COUNT]| {
		std::array::from_fn(|size_class| previous[size_class].max(needed[size_class]))
	};
	let retained = ShadowLayout {
		suns: previous.suns.max(needed.suns),
		cones: classes(previous.cones, needed.cones),
		points: classes(previous.points, needed.points),
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
	Directional {
		slot: u32,
	},
	/// `layer` is the light's layer of the cone image of its size class.
	Cone {
		view_index: u32,
		size_class: u32,
		layer: u32,
	},
	/// `cube_index` is the light's cube of the point image of its size class.
	Point {
		view_index: u32,
		size_class: u32,
		cube_index: u32,
	},
}

/// The `LocalShadow` struct is one cone or point light that holds a shadow map this frame, and where that map lives.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalShadow<'a, L> {
	/// The scene light index.
	pub(crate) index: usize,
	pub(crate) light: &'a L,
	pub(crate) transform: &'a Transform,
	pub(crate) size_class: usize,
	/// The light's cone layer or point cube in the image of its size class. It stays the same while the light keeps
	/// its map in that class, so a map that isn't drawn again keeps its contents.
	pub(crate) layer: u32,
	/// The most sink pixels the light's shadow range covers in any sink, which orders its map's updates.
	pub(crate) importance: f32,
}

/// The `LocalShadowKind` enum tells a cone light's map from a point light's cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalShadowKind {
	Cone,
	Point,
}

/// The `ShadowSlot` struct is where one local light's map lived last frame. Pass last frame's slots, from
/// [`ShadowLightSelection::slots`], to [`select_shadow_lights`], which keeps each light in its size class until its
/// span falls well below it, and at its layer while it stays in that class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShadowSlot {
	/// The scene light index.
	pub(crate) index: usize,
	pub(crate) kind: LocalShadowKind,
	pub(crate) size_class: usize,
	pub(crate) layer: u32,
}

/// Every local light's slot, sorted by scene light index.
pub(crate) type ShadowSlots = SmallVec<[ShadowSlot; MAX_CONE_SHADOW_COUNT + MAX_POINT_SHADOW_COUNT]>;

/// Marks a map whose layer is not assigned yet.
const UNASSIGNED_LAYER: u32 = u32::MAX;

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
	/// Shadowed cones in view order, ordered by size class and then layer.
	pub(crate) cones: SmallVec<[LocalShadow<'a, ConeLight>; MAX_CONE_SHADOW_COUNT]>,
	/// Shadowed points in view order, ordered by size class and then cube.
	pub(crate) points: SmallVec<[LocalShadow<'a, PointLight>; MAX_POINT_SHADOW_COUNT]>,
	/// Directional lights, and local lights some sink sees, that are lit but hold no shadow map this frame.
	pub(crate) unshadowed_count: usize,
	/// Every selected light's shadow, sorted by light index, so the lighting upload looks each light up once.
	assignments: SmallVec<[(usize, LightShadow); SHADOW_ASSIGNMENTS_INLINE]>,
}

impl ShadowLightSelection<'_> {
	/// Returns how many maps of each kind and size class the images need for this selection: through the highest layer
	/// each class uses.
	pub(crate) fn layout(&self) -> ShadowLayout {
		let mut layout = ShadowLayout {
			suns: self.suns.len(),
			..ShadowLayout::default()
		};
		for cone in &self.cones {
			let layers = &mut layout.cones[cone.size_class];
			*layers = (*layers).max(cone.layer as usize + 1);
		}
		for point in &self.points {
			let cubes = &mut layout.points[point.size_class];
			*cubes = (*cubes).max(point.layer as usize + 1);
		}
		layout
	}

	/// Returns each shadowed local light's slot, for the next frame's [`select_shadow_lights`].
	pub(crate) fn slots(&self) -> ShadowSlots {
		let mut slots: ShadowSlots = self
			.cones
			.iter()
			.map(|cone| slot_of(cone, LocalShadowKind::Cone))
			.chain(self.points.iter().map(|point| slot_of(point, LocalShadowKind::Point)))
			.collect();
		slots.sort_unstable_by_key(|slot| slot.index);
		slots
	}

	/// Returns the shadow assignment of the scene light at `light_index`.
	pub(crate) fn shadow_for(&self, light_index: usize) -> LightShadow {
		self.assignments
			.binary_search_by_key(&light_index, |(index, _)| *index)
			.map_or(LightShadow::None, |position| self.assignments[position].1)
	}

	/// Places every local light's map in its class's image, orders the lights by class and layer, and records every
	/// selected light's shadow in light order for [`Self::shadow_for`].
	///
	/// A light keeps last frame's layer while it stays in the same class, and a new map takes its class's lowest free
	/// layer. When the gaps that leaves would push the images past the memory budget, every class is packed from layer
	/// zero instead.
	fn index_assignments(&mut self, previous_slots: &[ShadowSlot], budget: &ShadowBudget) {
		assign_layers(&mut self.cones, LocalShadowKind::Cone, previous_slots);
		assign_layers(&mut self.points, LocalShadowKind::Point, previous_slots);
		if budget.cost(self.layout()) > budget.bytes {
			pack_layers(&mut self.cones);
			pack_layers(&mut self.points);
		}
		self.cones.sort_unstable_by_key(|cone| (cone.size_class, cone.layer));
		self.points.sort_unstable_by_key(|point| (point.size_class, point.layer));
		let mut assignments: SmallVec<[(usize, LightShadow); SHADOW_ASSIGNMENTS_INLINE]> = self
			.suns
			.iter()
			.enumerate()
			.map(|(slot, sun)| (sun.index, LightShadow::Directional { slot: slot as u32 }))
			.collect();
		for (position, cone) in self.cones.iter().enumerate() {
			let shadow = LightShadow::Cone {
				view_index: (CONE_SHADOW_VIEW_OFFSET + position) as u32,
				size_class: cone.size_class as u32,
				layer: cone.layer,
			};
			assignments.push((cone.index, shadow));
		}
		for (position, point) in self.points.iter().enumerate() {
			let shadow = LightShadow::Point {
				view_index: (POINT_SHADOW_VIEW_OFFSET + position * POINT_SHADOW_FACE_COUNT) as u32,
				size_class: point.size_class as u32,
				cube_index: point.layer,
			};
			assignments.push((point.index, shadow));
		}
		assignments.sort_unstable_by_key(|(index, _)| *index);
		self.assignments = assignments;
	}
}

/// Returns where a local light's map lives.
pub(crate) fn slot_of<L>(map: &LocalShadow<'_, L>, kind: LocalShadowKind) -> ShadowSlot {
	ShadowSlot {
		index: map.index,
		kind,
		size_class: map.size_class,
		layer: map.layer,
	}
}

/// Gives every map of `kind` a layer in its class's image: last frame's layer when the light held a map of the same
/// kind and class there, otherwise the class's lowest free layer.
fn assign_layers<L>(maps: &mut [LocalShadow<'_, L>], kind: LocalShadowKind, previous_slots: &[ShadowSlot]) {
	// One bit per layer; each kind holds at most 16 maps, so every layer fits in 32 bits.
	const _: () = assert!(MAX_CONE_SHADOW_COUNT <= 32 && MAX_POINT_SHADOW_COUNT <= 32);
	let mut taken = [0u32; LOCAL_SHADOW_SIZE_CLASS_COUNT];
	for map in maps.iter_mut() {
		map.layer = UNASSIGNED_LAYER;
		let kept = previous_slots
			.binary_search_by_key(&map.index, |slot| slot.index)
			.ok()
			.map(|position| previous_slots[position])
			.filter(|slot| slot.kind == kind && slot.size_class == map.size_class)
			.filter(|slot| taken[map.size_class] & (1 << slot.layer) == 0);
		if let Some(slot) = kept {
			taken[map.size_class] |= 1 << slot.layer;
			map.layer = slot.layer;
		}
	}
	for map in maps.iter_mut().filter(|map| map.layer == UNASSIGNED_LAYER) {
		let layer = taken[map.size_class].trailing_ones();
		taken[map.size_class] |= 1 << layer;
		map.layer = layer;
	}
}

/// Renumbers every class's maps from layer zero, keeping their order within the class.
fn pack_layers<L>(maps: &mut [LocalShadow<'_, L>]) {
	maps.sort_unstable_by_key(|map| (map.size_class, map.layer));
	let mut next = [0u32; LOCAL_SHADOW_SIZE_CLASS_COUNT];
	for map in maps {
		map.layer = next[map.size_class];
		next[map.size_class] += 1;
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

/// The `ShadowCoverage` struct is how much of one sink a local light's conservative bound covers, which ranks the light
/// for a shadow map and sizes that map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ShadowCoverage {
	/// The estimated number of sink pixels the bound covers, which ranks the light.
	pub(crate) pixels: f32,
	/// The texels per side the light's map needs to give the bound about one texel per sink pixel. See
	/// [`size_class_for_span`].
	pub(crate) span: f32,
}

/// Returns the estimated screen coverage of a cone-shadow candidate in one sink whose view has `frustum` planes.
pub(crate) fn cone_shadow_importance(
	light: &ConeLight,
	transform: &Transform,
	intensity_scale_candela: f32,
	sink: &Sink,
	frustum: &[math::Plane; 6],
) -> Option<ShadowCoverage> {
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
) -> Option<ShadowCoverage> {
	let (_, far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, intensity_scale_candela);
	let coverage = shadow_view_importance(math::Sphere::new(transform.position(), far), sink, frustum)?;
	// Each cube face spans 90 degrees, so about two faces lie across the bound's diameter.
	Some(ShadowCoverage {
		span: coverage.span * 0.5,
		..coverage
	})
}

/// Returns how much of a sink a local light's conservative bound covers, or `None` when culled.
///
/// The covered pixels rank the light for a shadow map. The span is the bound's projected diameter in sink pixels,
/// including any part off screen, so a light partly in view keeps the texel density of its visible part. A bound that
/// contains the camera covers the whole sink. This projection does not alter light culling or a light's shadow
/// projection.
fn shadow_view_importance(bounds: math::Sphere, sink: &Sink, frustum: &[math::Plane; 6]) -> Option<ShadowCoverage> {
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
	let (width, height) = (sink.extent().width() as f32, sink.extent().height() as f32);
	let pixel_count = width * height;
	if !depth.is_finite() || !pixel_count.is_finite() {
		return None;
	}
	let full_sink = ShadowCoverage {
		pixels: pixel_count,
		span: width.max(height),
	};
	// A bound containing the camera covers every ray in the view, so rank it as a full sink.
	if depth <= radius {
		return Some(full_sink);
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
		return Some(full_sink);
	}
	if !center_x.is_finite() || !center_y.is_finite() || !radius_x.is_finite() || !radius_y.is_finite() {
		return None;
	}
	let covered_width = (center_x + radius_x).min(1.0) - (center_x - radius_x).max(-1.0);
	let covered_height = (center_y + radius_y).min(1.0) - (center_y - radius_y).max(-1.0);
	let coverage = (covered_width * 0.5).max(0.0) * (covered_height * 0.5).max(0.0);
	let importance = coverage * pixel_count;
	// Normalized device coordinates span two units across the sink, so a radius of `r` units is `r` sink widths across.
	let span = (radius_x * width).max(radius_y * height);
	importance.is_finite().then_some(ShadowCoverage {
		pixels: importance,
		span: if span.is_finite() { span } else { full_sink.span },
	})
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
	/// The size class of the light's map, from its largest span over every sink that sees it.
	size_class: usize,
	/// The most pixels the light covers in any sink.
	importance: f32,
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
///
/// Each cone and point light's map takes the size class its largest span over every sink needs, and costs the budget
/// that class's bytes. `previous_slots` is last frame's [`ShadowLightSelection::slots`], which keeps a shrinking light in
/// its class until its span falls well below it, and each light at its layer while it stays in its class.
pub(crate) fn select_shadow_lights<'a>(
	lights: impl Iterator<Item = (&'a Lights, &'a Transform)>,
	sinks: &[Sink],
	budget: &ShadowBudget,
	intensity_scale_candela: impl Fn(usize) -> f32,
	previous_slots: &[ShadowSlot],
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
		let previous_size_class = previous_slots
			.binary_search_by_key(&index, |slot| slot.index)
			.ok()
			.map(|position| previous_slots[position].size_class);
		let candidate = |light, span, importance| Candidate {
			index,
			light,
			transform,
			size_class: size_class_for_span(span, previous_size_class),
			importance,
		};
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
				|span, importance| candidate(LocalLight::Cone(light), span, importance),
				|sink, frustum| cone_shadow_importance(light, transform, scale, sink, frustum),
			),
			Lights::Point(light) if has_brightness(&light.emission, scale) => rank(
				&mut rankings,
				sinks,
				&mut frusta,
				|span, importance| candidate(LocalLight::Point(light), span, importance),
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
	selection.index_assignments(previous_slots, budget);
	selection
}

/// Ranks one candidate for every sink that sees it, and returns whether any sink does.
///
/// `candidate` builds the candidate from its largest span over every sink, which sizes its map, and the most pixels it
/// covers in any sink. `frusta` caches each
/// sink's frustum planes, extracted the first time any candidate needs them.
fn rank<'a>(
	rankings: &mut [SinkRanking<'a>],
	sinks: &[Sink],
	frusta: &mut [Option<[math::Plane; 6]>],
	candidate: impl FnOnce(f32, f32) -> Candidate<'a>,
	importance: impl Fn(&Sink, &[math::Plane; 6]) -> Option<ShadowCoverage>,
) -> bool {
	// Four sinks stay inline, matching the rankings.
	let coverages = sinks
		.iter()
		.zip(frusta)
		.map(|(sink, frustum)| importance(sink, frustum.get_or_insert_with(|| sink.view().get_frustum_planes())))
		.collect::<SmallVec<[Option<ShadowCoverage>; 4]>>();
	let Some(span) = coverages.iter().flatten().map(|coverage| coverage.span).reduce(f32::max) else {
		return false;
	};
	let importance = coverages.iter().flatten().map(|coverage| coverage.pixels).fold(0.0, f32::max);
	let candidate = candidate(span, importance);
	for (ranking, coverage) in rankings.iter_mut().zip(coverages) {
		if let Some(coverage) = coverage {
			ranking.insert(coverage.pixels, candidate);
		}
	}
	true
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
			let Candidate {
				index,
				light,
				transform,
				size_class,
				importance,
			} = *candidate;
			match light {
				LocalLight::Cone(light) => {
					if !holds_map(&selection.cones, index)
						&& selection.cones.len() < MAX_CONE_SHADOW_COUNT
						&& claim(remaining, budget.cone_bytes[size_class])
					{
						selection.cones.push(LocalShadow {
							index,
							light,
							transform,
							size_class,
							layer: UNASSIGNED_LAYER,
							importance,
						});
					}
				}
				LocalLight::Point(light) => {
					if !holds_map(&selection.points, index)
						&& selection.points.len() < MAX_POINT_SHADOW_COUNT
						&& claim(remaining, budget.point_bytes[size_class])
					{
						selection.points.push(LocalShadow {
							index,
							light,
							transform,
							size_class,
							layer: UNASSIGNED_LAYER,
							importance,
						});
					}
				}
			}
		}
	}
}

/// Returns whether the scene light at `index` already holds one of `maps`.
fn holds_map<L>(maps: &[LocalShadow<'_, L>], index: usize) -> bool {
	maps.iter().any(|map| map.index == index)
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

	/// Prices a sun at eight units, a cone at one, and a point at six in every size class, so tests state budgets in
	/// those units.
	fn budget(bytes: u64) -> ShadowBudget {
		ShadowBudget {
			bytes,
			directional_bytes: 8,
			cone_bytes: [1; LOCAL_SHADOW_SIZE_CLASS_COUNT],
			point_bytes: [6; LOCAL_SHADOW_SIZE_CLASS_COUNT],
		}
	}

	/// Returns a layout of `suns` suns and `cones` cones and `points` points of the smallest size class.
	fn smallest_class_layout(suns: usize, cones: usize, points: usize) -> ShadowLayout {
		let mut layout = ShadowLayout {
			suns,
			..ShadowLayout::default()
		};
		layout.cones[0] = cones;
		layout.points[0] = points;
		layout
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
		sized_sink(position, 1)
	}

	fn sized_sink(position: Point, pixels: u32) -> Sink {
		Sink::new(
			View::new_perspective(math::Degrees::new(90.0), 1.0, 0.1, 100.0, position, UnitVector::z_axis()),
			Extent::square(pixels),
			0,
		)
	}

	fn select<'a>(
		lights: &'a [Lights],
		transforms: &'a [Transform],
		sinks: &[Sink],
		budget: ShadowBudget,
	) -> ShadowLightSelection<'a> {
		select_shadow_lights(lights.iter().zip(transforms), sinks, &budget, |_| 1.0, &[])
	}

	fn sun_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.suns.iter().map(|sun| sun.index).collect()
	}

	fn cone_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.cones.iter().map(|cone| cone.index).collect()
	}

	fn point_indices(selection: &ShadowLightSelection<'_>) -> Vec<usize> {
		selection.points.iter().map(|point| point.index).collect()
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
				size_class: 0,
				layer: 1
			}
		);
		assert_eq!(
			selection.shadow_for(2),
			LightShadow::Point {
				view_index: POINT_SHADOW_VIEW_OFFSET as u32,
				size_class: 0,
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
		let previous = smallest_class_layout(1, 4, 0);
		let fewer_cones = smallest_class_layout(1, 1, 0);
		let one_point = smallest_class_layout(1, 1, 1);

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
		let resident = select_shadow_lights(lights.iter().zip(&transforms), &sinks, &budget(6), |_| 90.0, &[]);
		let (_, fallback_far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);
		let (_, resident_far) = resolve_shadow_range(&light.emission, SHADOW_DEFAULT_EXPOSURE_SCALE, 90.0);

		assert!(fallback.points.is_empty());
		assert_eq!(fallback.unshadowed_count, 0);
		assert_eq!(point_indices(&resident), [0]);
		assert!((resident_far / fallback_far - 90.0_f32.sqrt()).abs() < 0.0001);
	}

	#[test]
	fn size_classes_snap_spans_up_to_powers_of_two_within_the_class_range() {
		assert_eq!(size_class_for_span(0.0, None), 0);
		assert_eq!(size_class_for_span(128.0, None), 0);
		assert_eq!(size_class_for_span(129.0, None), 1);
		assert_eq!(size_class_for_span(1000.0, None), 3);
		assert_eq!(size_class_for_span(100_000.0, None), LOCAL_SHADOW_SIZE_CLASS_COUNT - 1);
	}

	#[test]
	fn size_classes_grow_at_once_and_shrink_only_well_below_a_smaller_class() {
		// 1024 texels held: a span just under 512 still keeps them, and one under three quarters of 512 drops to 512.
		assert_eq!(size_class_for_span(500.0, Some(3)), 3);
		assert_eq!(size_class_for_span(383.0, Some(3)), 2);
		// A span far below drops straight to the class it needs.
		assert_eq!(size_class_for_span(10.0, Some(3)), 0);
		// Growth ignores the previous class.
		assert_eq!(size_class_for_span(1025.0, Some(1)), 4);
	}

	#[test]
	fn a_light_takes_the_size_class_of_its_largest_span_over_every_sink_and_pays_for_it() {
		// The cone sits two meters in front of the second sink's camera and far from the first's.
		let lights = [Lights::Cone(cone().with_shadow_far(1.0))];
		let transforms = [Transform::from_position(Point::new(0.0, 0.0, 30.0))];
		let near_sink = sized_sink(Point::new(0.0, 0.0, 28.0), 1024);
		let far_sink = sized_sink(Point::origin(), 1024);
		let mut priced = budget(64);
		priced.cone_bytes = [1, 2, 4, 8, 16];

		let far_only = select(&lights, &transforms, &[far_sink], priced);
		let both = select(&lights, &transforms, &[far_sink, near_sink], priced);

		let far_class = far_only.cones[0].size_class;
		let near_class = both.cones[0].size_class;
		assert!(near_class > far_class, "{near_class} should exceed {far_class}");
		assert_eq!(both.layout().cones[near_class], 1);
		assert_eq!(priced.cost(both.layout()), priced.cone_bytes[near_class]);

		// A budget too small for the larger class leaves the light unshadowed.
		priced.bytes = priced.cone_bytes[near_class] - 1;
		assert!(select(&lights, &transforms, &[near_sink], priced).cones.is_empty());
	}

	#[test]
	fn local_shadows_are_grouped_by_size_class_with_cubes_counted_within_each_class() {
		// Two points near the camera need a larger class than the far one, which ranks between them in scene order.
		let lights: Vec<_> = (0..3).map(|_| Lights::Point(point().with_shadow_far(1.0))).collect();
		let transforms = [3.0, 40.0, 3.5].map(|z| Transform::from_position(Point::new(0.0, 0.0, z)));

		let selection = select(&lights, &transforms, &[sized_sink(Point::origin(), 1024)], budget(64));

		let shadow = |index| match selection.shadow_for(index) {
			LightShadow::Point {
				view_index,
				size_class,
				cube_index,
			} => (view_index - POINT_SHADOW_VIEW_OFFSET as u32, size_class, cube_index),
			other => panic!("Expected a point shadow, got {other:?}"),
		};
		let (far_view, far_class, far_cube) = shadow(1);
		let (first_near_view, near_class, first_near_cube) = shadow(0);
		let (second_near_view, second_near_class, second_near_cube) = shadow(2);
		let faces = POINT_SHADOW_FACE_COUNT as u32;
		assert!(far_class < near_class && near_class == second_near_class);
		// The smaller class's views come first, and cubes count from zero within each class.
		assert_eq!((far_view, far_cube), (0, 0));
		assert_eq!((first_near_view, first_near_cube), (faces, 0));
		assert_eq!((second_near_view, second_near_cube), (2 * faces, 1));
		assert_eq!(selection.layout().points[near_class as usize], 2);
	}

	fn cone_slot(index: usize, layer: u32) -> ShadowSlot {
		ShadowSlot {
			index,
			kind: LocalShadowKind::Cone,
			size_class: 0,
			layer,
		}
	}

	#[test]
	fn lights_keep_their_layers_and_new_lights_take_the_lowest_free_layer() {
		let lights: Vec<_> = (0..3).map(|_| Lights::Cone(cone())).collect();
		let transforms = [0.0, 1.0, 2.0].map(light_transform);
		let previous = [cone_slot(0, 2), cone_slot(1, 0)];

		let selection = select_shadow_lights(
			lights.iter().zip(&transforms),
			&[sink(Point::origin())],
			&budget(64),
			|_| 1.0,
			&previous,
		);

		let cone_shadow = |view, layer| LightShadow::Cone {
			view_index: CONE_SHADOW_VIEW_OFFSET as u32 + view,
			size_class: 0,
			layer,
		};
		// Views follow the layers, so a run of layers is a run of views.
		assert_eq!(selection.shadow_for(1), cone_shadow(0, 0));
		assert_eq!(selection.shadow_for(2), cone_shadow(1, 1));
		assert_eq!(selection.shadow_for(0), cone_shadow(2, 2));
		assert_eq!(
			selection.slots().as_slice(),
			[cone_slot(0, 2), cone_slot(1, 0), cone_slot(2, 1)]
		);
	}

	#[test]
	fn layers_pack_from_zero_when_their_gaps_would_exceed_the_memory_budget() {
		let lights = [Lights::Cone(cone())];
		let transforms = [light_transform(0.0)];

		let kept = select_shadow_lights(
			lights.iter().zip(&transforms),
			&[sink(Point::origin())],
			&budget(6),
			|_| 1.0,
			&[cone_slot(0, 5)],
		);
		let packed = select_shadow_lights(
			lights.iter().zip(&transforms),
			&[sink(Point::origin())],
			&budget(5),
			|_| 1.0,
			&[cone_slot(0, 5)],
		);

		assert_eq!(kept.cones[0].layer, 5);
		assert_eq!(packed.cones[0].layer, 0);
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
