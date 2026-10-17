//! Chooses which shadow maps draw each frame within the update budget, and remembers what each map was drawn with.
//!
//! The shadow map updates of the visibility pipeline's Reference page specify these rules. [`ShadowMapStates`] keeps,
//! for every map, the view it was drawn with and how many frames ago. Each frame, [`ShadowMapStates::update`] matches
//! the maps to this frame's [`ShadowLightSelection`], ranks them with [`schedule_updates`], and records the ones that
//! draw. Next, write the views it keeps into the views buffer and draw the returned [`ShadowDraw`]s with
//! [`super::render_pass::ShadowMaps::prepare`].

use smallvec::SmallVec;

use super::layout::{
	CONE_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_COUNT, MAX_DIRECTIONAL_SHADOW_COUNT, MAX_POINT_SHADOW_COUNT,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT, local_shadow_map_resolution,
};
use super::shader_data::ShaderViewData;
use super::shadow_selection::{
	LightShadow, LocalShadowKind, ShadowLayout, ShadowLightSelection, ShadowSlot, slot_of, sun_cascade_view,
};

/// The cascades of every shadowed sun that draw every frame, counted against the budget first.
pub(crate) const ALWAYS_DRAWN_CASCADES: usize = 2;
/// The texels a frame draws unless the application sets `render.shadow-maps.update-texels`: the four cascades of one
/// sun at 2,048 texels per side.
pub(crate) const DEFAULT_SHADOW_UPDATE_TEXELS: u64 = 16_777_216;
/// The most maps that compete for one frame's budget: two cascades per sun, every cone map, and every point cube face.
pub(crate) const MAX_UPDATE_CANDIDATES: usize = (SHADOW_CASCADE_COUNT - ALWAYS_DRAWN_CASCADES) * MAX_DIRECTIONAL_SHADOW_COUNT
	+ MAX_CONE_SHADOW_COUNT
	+ MAX_POINT_SHADOW_COUNT * POINT_SHADOW_FACE_COUNT;
/// The inline capacity of a frame's candidates and draws: the smallest size `SmallVec` supports that holds every
/// cascade, cone map, and point cube face.
const SHADOW_MAPS_INLINE: usize = 128;
const _: () = assert!(MAX_UPDATE_CANDIDATES + ALWAYS_DRAWN_CASCADES * MAX_DIRECTIONAL_SHADOW_COUNT <= SHADOW_MAPS_INLINE);

/// The `UpdateTier` enum ranks a map's need to draw before its age and importance do. Earlier tiers draw first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UpdateTier {
	/// The map holds nothing yet, so its light has no shadow from it.
	NotDrawn,
	/// The map's view moved since it was drawn.
	ViewChanged,
	Current,
}

/// The `UpdateCandidate` struct is one map competing for this frame's update budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UpdateCandidate {
	pub(crate) tier: UpdateTier,
	/// Frames since the map was drawn, or since it was placed when it was never drawn.
	pub(crate) age: u32,
	/// The most sink pixels the map's light covers in any sink.
	pub(crate) importance: f32,
	/// The texels drawing the map costs.
	pub(crate) texels: u64,
}

/// Returns, one entry per candidate, whether it draws this frame within `budget` texels.
///
/// Candidates are taken by tier, then by age times importance, highest first; equal ones keep their order. Each one
/// draws while its texels fit what's left of the budget, and one that doesn't fit is skipped. When none fits, the first
/// draws anyway, so every map draws eventually.
pub(crate) fn schedule_updates(candidates: &[UpdateCandidate], budget: u64) -> SmallVec<[bool; SHADOW_MAPS_INLINE]> {
	let mut drawn = SmallVec::from_elem(false, candidates.len());
	let priority = |candidate: &UpdateCandidate| candidate.age as f32 * candidate.importance;
	let mut order = (0..candidates.len()).collect::<SmallVec<[usize; SHADOW_MAPS_INLINE]>>();
	// A stable sort keeps equal candidates in their given order.
	order.sort_by(|&a, &b| {
		let (a, b) = (&candidates[a], &candidates[b]);
		a.tier.cmp(&b.tier).then(priority(b).total_cmp(&priority(a)))
	});
	let mut remaining = budget;
	for &index in &order {
		if candidates[index].texels <= remaining {
			remaining -= candidates[index].texels;
			drawn[index] = true;
		}
	}
	if let Some(&first) = order.first()
		&& !drawn.contains(&true)
	{
		drawn[first] = true;
	}
	drawn
}

/// The `ShadowImage` enum names the image one shadow map lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ShadowImage {
	Directional,
	/// The cone image of a size class.
	Cone(usize),
	/// The point image of a size class.
	Point(usize),
}

/// The `ShadowDraw` struct is one map to draw this frame: the layer of its image and the view to draw it from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShadowDraw {
	pub(crate) image: ShadowImage,
	pub(crate) layer: u32,
	pub(crate) view: u32,
}

/// The maps one frame draws, ordered by image and then layer.
pub(crate) type ShadowDraws = SmallVec<[ShadowDraw; SHADOW_MAPS_INLINE]>;

/// The `ShadowViews` struct holds the view every selected map would be drawn from this frame, in the order of the
/// [`ShadowLightSelection`] it was made for.
#[derive(Default)]
pub(crate) struct ShadowViews {
	/// Each shadowed sun's cascades as the CPU fitted them, by sun slot.
	pub(crate) cascades: SmallVec<[[ShaderViewData; SHADOW_CASCADE_COUNT]; MAX_DIRECTIONAL_SHADOW_COUNT]>,
	pub(crate) cones: SmallVec<[ShaderViewData; MAX_CONE_SHADOW_COUNT]>,
	pub(crate) points: SmallVec<[[ShaderViewData; POINT_SHADOW_FACE_COUNT]; MAX_POINT_SHADOW_COUNT]>,
}

/// Returns `current` with a projection that places every receiver outside the cascade, for a cascade that holds no
/// drawn map. Receivers then use a coarser cascade, or get no shadow from the sun. `current` keeps its split distance.
pub(crate) fn undrawn_cascade_view(current: &ShaderViewData) -> ShaderViewData {
	// Row-major: clip x is twice clip w, and w is one, for every position.
	let mut outside = math::Matrix::zero();
	outside[3] = 2.0;
	outside[15] = 1.0;
	ShaderViewData {
		view_projection: outside.into(),
		..*current
	}
}

/// Returns `drawn`, the view a cascade was drawn from, with `current`'s split distance, which picks the cascade for a
/// receiver this frame.
pub(crate) fn drawn_cascade_view(drawn: &ShaderViewData, current: &ShaderViewData) -> ShaderViewData {
	ShaderViewData {
		far: current.far,
		..*drawn
	}
}

/// The `MapState` struct is what one map was last drawn with.
#[derive(Clone, Copy, Default)]
struct MapState {
	/// The view the map was drawn from, or `None` when it holds nothing.
	drawn: Option<ShaderViewData>,
	/// Frames since the map was drawn, or since it was placed when it was never drawn.
	age: u32,
}

impl MapState {
	/// Returns the map as a candidate for drawing from `current`.
	fn candidate(&self, current: &ShaderViewData, importance: f32, texels: u64) -> UpdateCandidate {
		let tier = match &self.drawn {
			None => UpdateTier::NotDrawn,
			Some(drawn) if bytemuck::bytes_of(drawn) != bytemuck::bytes_of(current) => UpdateTier::ViewChanged,
			Some(_) => UpdateTier::Current,
		};
		UpdateCandidate {
			tier,
			age: self.age,
			importance,
			texels,
		}
	}

	/// Records whether the map draws from `current` this frame.
	fn commit(&mut self, current: &ShaderViewData, drawn: bool) {
		if drawn {
			self.drawn = Some(*current);
			self.age = 0;
		} else {
			self.age = self.age.saturating_add(1);
		}
	}
}

/// The `SunState` struct is what one sun slot's cascades were last drawn with.
#[derive(Clone, Copy, Default)]
struct SunState {
	/// The scene light index of the sun in the slot.
	index: Option<usize>,
	cascades: [MapState; SHADOW_CASCADE_COUNT],
}

/// The `Unit` enum names one map of this frame's selection.
#[derive(Clone, Copy)]
enum Unit {
	Cascade {
		slot: usize,
		cascade: usize,
	},
	/// A cone by its position in the selection.
	Cone(usize),
	/// A point cube face by its light's position in the selection.
	PointFace {
		position: usize,
		face: usize,
	},
}

impl Unit {
	/// Returns the view the map would be drawn from this frame.
	fn view(self, views: &ShadowViews) -> &ShaderViewData {
		match self {
			Unit::Cascade { slot, cascade } => &views.cascades[slot][cascade],
			Unit::Cone(position) => &views.cones[position],
			Unit::PointFace { position, face } => &views.points[position][face],
		}
	}

	/// Returns where the map draws: its image, its layer there, and its view.
	fn draw(self, selection: &ShadowLightSelection<'_>) -> ShadowDraw {
		match self {
			Unit::Cascade { slot, cascade } => ShadowDraw {
				image: ShadowImage::Directional,
				layer: (slot * SHADOW_CASCADE_COUNT + cascade) as u32,
				view: (sun_cascade_view(slot) + cascade) as u32,
			},
			Unit::Cone(position) => {
				let cone = &selection.cones[position];
				ShadowDraw {
					image: ShadowImage::Cone(cone.size_class),
					layer: cone.layer,
					view: (CONE_SHADOW_VIEW_OFFSET + position) as u32,
				}
			}
			Unit::PointFace { position, face } => {
				let point = &selection.points[position];
				ShadowDraw {
					image: ShadowImage::Point(point.size_class),
					layer: point.layer * POINT_SHADOW_FACE_COUNT as u32 + face as u32,
					view: (POINT_SHADOW_VIEW_OFFSET + position * POINT_SHADOW_FACE_COUNT + face) as u32,
				}
			}
		}
	}
}

/// The `ShadowMapStates` struct remembers what every shadow map was drawn with, so a map that doesn't draw this frame
/// keeps its view and its contents.
///
/// The visibility pipeline manager owns one. Each frame it calls [`Self::update`] on a copy and keeps the copy only once
/// the frame records the draws, so a frame that doesn't record leaves every map as it was.
#[derive(Clone, Default)]
pub(crate) struct ShadowMapStates {
	suns: [SunState; MAX_DIRECTIONAL_SHADOW_COUNT],
	/// The cones of the last selection, in its order.
	cones: SmallVec<[(ShadowSlot, MapState); MAX_CONE_SHADOW_COUNT]>,
	/// The points of the last selection, in its order.
	points: SmallVec<[(ShadowSlot, [MapState; POINT_SHADOW_FACE_COUNT]); MAX_POINT_SHADOW_COUNT]>,
	/// The layout of the images the maps were drawn into.
	layout: ShadowLayout,
}

impl ShadowMapStates {
	/// Matches the maps to `selection`, chooses the ones that draw this frame within `budget` texels, and records them.
	/// Returns the maps to draw.
	///
	/// `layout` is the layout the images take this frame; an image whose size changes loses its maps. `views` holds
	/// every map's view this frame. A sun's cascades count the pixels of `sink_pixels`, the largest sink, as their
	/// importance, and cost `cascade_resolution` squared texels each.
	pub(crate) fn update(
		&mut self,
		selection: &ShadowLightSelection<'_>,
		layout: ShadowLayout,
		views: &ShadowViews,
		sink_pixels: f32,
		cascade_resolution: u32,
		budget: u64,
	) -> ShadowDraws {
		self.match_selection(selection, layout);

		let cascade_texels = u64::from(cascade_resolution).pow(2);
		let class_texels = |size_class: usize| u64::from(local_shadow_map_resolution(size_class)).pow(2);
		let cascade = |slot, cascade| Unit::Cascade { slot, cascade };
		let always_drawn = (0..selection.suns.len())
			.flat_map(|slot| (0..ALWAYS_DRAWN_CASCADES).map(move |index| cascade(slot, index)))
			.collect::<SmallVec<[Unit; SHADOW_MAPS_INLINE]>>();
		// Every competing map with its importance and texels.
		let competing = (0..selection.suns.len())
			.flat_map(|slot| (ALWAYS_DRAWN_CASCADES..SHADOW_CASCADE_COUNT).map(move |index| cascade(slot, index)))
			.map(|unit| (unit, sink_pixels, cascade_texels))
			.chain(
				selection
					.cones
					.iter()
					.enumerate()
					.map(|(position, cone)| (Unit::Cone(position), cone.importance, class_texels(cone.size_class))),
			)
			.chain(selection.points.iter().enumerate().flat_map(|(position, point)| {
				let texels = class_texels(point.size_class);
				(0..POINT_SHADOW_FACE_COUNT).map(move |face| (Unit::PointFace { position, face }, point.importance, texels))
			}))
			.collect::<SmallVec<[_; SHADOW_MAPS_INLINE]>>();
		let candidates = competing
			.iter()
			.map(|&(unit, importance, texels)| self.state(unit).candidate(unit.view(views), importance, texels))
			.collect::<SmallVec<[_; SHADOW_MAPS_INLINE]>>();
		let always_drawn_texels = cascade_texels * always_drawn.len() as u64;
		let drawn = schedule_updates(&candidates, budget.saturating_sub(always_drawn_texels));

		let mut draws = ShadowDraws::new();
		let competing = competing.iter().map(|(unit, ..)| *unit).zip(drawn);
		for (unit, drawn) in always_drawn.iter().map(|unit| (*unit, true)).chain(competing) {
			self.state_mut(unit).commit(unit.view(views), drawn);
			if drawn {
				draws.push(unit.draw(selection));
			}
		}
		draws.sort_unstable_by_key(|draw| (draw.image, draw.layer));
		draws
	}

	fn state(&self, unit: Unit) -> &MapState {
		match unit {
			Unit::Cascade { slot, cascade } => &self.suns[slot].cascades[cascade],
			Unit::Cone(position) => &self.cones[position].1,
			Unit::PointFace { position, face } => &self.points[position].1[face],
		}
	}

	fn state_mut(&mut self, unit: Unit) -> &mut MapState {
		match unit {
			Unit::Cascade { slot, cascade } => &mut self.suns[slot].cascades[cascade],
			Unit::Cone(position) => &mut self.cones[position].1,
			Unit::PointFace { position, face } => &mut self.points[position].1[face],
		}
	}

	/// Keeps the state of every map that keeps its place, and starts every other map as never drawn: a new map, a map
	/// in another slot or size class, and every map of an image that changes size.
	fn match_selection(&mut self, selection: &ShadowLightSelection<'_>, layout: ShadowLayout) {
		let previous = std::mem::replace(&mut self.layout, layout);
		let directional_kept = previous.suns == layout.suns;
		for (slot, state) in self.suns.iter_mut().enumerate() {
			let index = selection.suns.get(slot).map(|sun| sun.index);
			if index.is_none() || state.index != index || !directional_kept {
				*state = SunState {
					index,
					..SunState::default()
				};
			}
		}
		let cone_kept = |size_class: usize| previous.cones[size_class] == layout.cones[size_class];
		self.cones = selection
			.cones
			.iter()
			.map(|cone| {
				let slot = slot_of(cone, LocalShadowKind::Cone);
				let state = self
					.cones
					.iter()
					.find(|(kept, _)| *kept == slot && cone_kept(slot.size_class))
					.map_or_else(MapState::default, |(_, state)| *state);
				(slot, state)
			})
			.collect();
		let point_kept = |size_class: usize| previous.points[size_class] == layout.points[size_class];
		self.points = selection
			.points
			.iter()
			.map(|point| {
				let slot = slot_of(point, LocalShadowKind::Point);
				let faces = self
					.points
					.iter()
					.find(|(kept, _)| *kept == slot && point_kept(slot.size_class))
					.map_or_else(Default::default, |(_, faces)| *faces);
				(slot, faces)
			})
			.collect();
	}

	/// Returns the view cascade `cascade` of sun slot `slot` was drawn from, or `None` when it holds nothing.
	pub(crate) fn cascade_view(&self, slot: usize, cascade: usize) -> Option<&ShaderViewData> {
		self.suns[slot].cascades[cascade].drawn.as_ref()
	}

	/// Returns the view the cone at `position` of the last selection was drawn from, or `None` when it holds nothing.
	pub(crate) fn cone_view(&self, position: usize) -> Option<&ShaderViewData> {
		self.cones[position].1.drawn.as_ref()
	}

	/// Returns the view face `face` of the point at `position` of the last selection was drawn from, or `None` when it
	/// holds nothing.
	pub(crate) fn point_face_view(&self, position: usize, face: usize) -> Option<&ShaderViewData> {
		self.points[position].1[face].drawn.as_ref()
	}

	/// Returns `selection`'s shadow for the scene light at `index`, or none for a local light whose map holds nothing
	/// yet: a cone's map, or every face of a point's cube.
	pub(crate) fn shadow_for(&self, selection: &ShadowLightSelection<'_>, index: usize) -> LightShadow {
		let shadow = selection.shadow_for(index);
		let drawn = match shadow {
			LightShadow::Cone { view_index, .. } => self.cone_view(view_index as usize - CONE_SHADOW_VIEW_OFFSET).is_some(),
			LightShadow::Point { view_index, .. } => {
				let position = (view_index as usize - POINT_SHADOW_VIEW_OFFSET) / POINT_SHADOW_FACE_COUNT;
				self.points[position].1.iter().all(|face| face.drawn.is_some())
			}
			LightShadow::None | LightShadow::Directional { .. } => true,
		};
		if drawn { shadow } else { LightShadow::None }
	}

	/// Returns one bit per cascade of `sun_count` suns, bit `4s + c` for cascade `c` of sun slot `s`, set for the
	/// cascades that hold a drawn map.
	pub(crate) fn drawn_cascades(&self, sun_count: usize) -> u32 {
		self.suns[..sun_count]
			.iter()
			.flat_map(|sun| &sun.cascades)
			.enumerate()
			.filter(|(_, state)| state.drawn.is_some())
			.fold(0, |mask, (bit, _)| mask | 1 << bit)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn candidate(tier: UpdateTier, age: u32, importance: f32, texels: u64) -> UpdateCandidate {
		UpdateCandidate {
			tier,
			age,
			importance,
			texels,
		}
	}

	fn schedule(candidates: &[UpdateCandidate], budget: u64) -> Vec<bool> {
		schedule_updates(candidates, budget).to_vec()
	}

	use crate::gameplay::Transform;
	use crate::rendering::lights::{ConeLight, LightColor, PhotometricIntensity};
	use crate::rendering::pipelines::visibility::shadow_selection::{LocalShadow, SunShadow};

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

	/// Returns a view that differs from every other view of a test by `id`.
	fn view(id: f32) -> ShaderViewData {
		ShaderViewData {
			far: id,
			..bytemuck::Zeroable::zeroed()
		}
	}

	/// Returns a selection of `cones` cones of size class `size_class`, one per layer, and `suns` suns.
	fn selection<'a>(
		light: &'a ConeLight,
		transform: &'a Transform,
		cones: usize,
		size_class: usize,
		suns: usize,
	) -> ShadowLightSelection<'a> {
		let mut selection = ShadowLightSelection::default();
		for index in 0..cones {
			selection.cones.push(LocalShadow {
				index,
				light,
				transform,
				size_class,
				layer: index as u32,
				importance: 100.0,
			});
		}
		for index in 0..suns {
			selection.suns.push(SunShadow {
				index: cones + index,
				direction: math::UnitVector::y_axis(),
				angular_radius_tangent: 0.0,
			});
		}
		selection
	}

	fn layout_of(selection: &ShadowLightSelection<'_>) -> ShadowLayout {
		let mut layout = ShadowLayout {
			suns: selection.suns.len(),
			..ShadowLayout::default()
		};
		for cone in &selection.cones {
			layout.cones[cone.size_class] += 1;
		}
		layout
	}

	fn views(cones: &[f32], suns: usize) -> ShadowViews {
		ShadowViews {
			cascades: (0..suns)
				.map(|_| std::array::from_fn(|cascade| view(100.0 + cascade as f32)))
				.collect(),
			cones: cones.iter().map(|id| view(*id)).collect(),
			points: SmallVec::new(),
		}
	}

	fn cone_draws(draws: &ShadowDraws) -> Vec<u32> {
		draws
			.iter()
			.filter(|draw| matches!(draw.image, ShadowImage::Cone(_)))
			.map(|draw| draw.layer)
			.collect()
	}

	/// The texels of one map of the smallest size class.
	const SMALLEST: u64 = 128 * 128;

	#[test]
	fn a_budget_of_one_map_per_frame_draws_every_map_in_turn_and_keeps_their_views() {
		let (light, transform) = (cone(), Transform::default());
		let selection = selection(&light, &transform, 3, 0, 0);
		let mut states = ShadowMapStates::default();

		let mut drawn = Vec::new();
		for _ in 0..3 {
			drawn.extend(cone_draws(&states.update(
				&selection,
				layout_of(&selection),
				&views(&[1.0, 2.0, 3.0], 0),
				1.0,
				2048,
				SMALLEST,
			)));
		}
		drawn.sort_unstable();
		assert_eq!(drawn, [0, 1, 2], "Every map should draw once before any draws again.");
		assert!((0..3).all(|position| states.cone_view(position).is_some()));

		// A moved light ranks above the others and draws next; the rest keep the views they were drawn with.
		let draws = states.update(
			&selection,
			layout_of(&selection),
			&views(&[1.0, 9.0, 3.0], 0),
			1.0,
			2048,
			SMALLEST,
		);
		assert_eq!(cone_draws(&draws), [1]);
		assert_eq!(states.cone_view(1).map(|view| view.far), Some(9.0));
		assert_eq!(states.cone_view(0).map(|view| view.far), Some(1.0));
	}

	#[test]
	fn maps_that_were_never_drawn_hold_no_view_until_they_draw() {
		let (light, transform) = (cone(), Transform::default());
		let selection = selection(&light, &transform, 2, 0, 0);
		let mut states = ShadowMapStates::default();

		states.update(&selection, layout_of(&selection), &views(&[1.0, 2.0], 0), 1.0, 2048, SMALLEST);

		assert_eq!(
			[states.cone_view(0).is_some(), states.cone_view(1).is_some()],
			[true, false],
			"Only one map fits the budget, so the other has nothing to sample yet."
		);
	}

	/// Draws two maps of size class `size_class`, then updates with `next` and no budget, so only its first candidate
	/// draws. Returns how many of `next`'s maps hold a drawn view.
	fn drawn_after(next: &ShadowLightSelection<'_>, size_class: usize) -> usize {
		let (light, transform) = (cone(), Transform::default());
		let first = selection(&light, &transform, 2, size_class, 0);
		let mut states = ShadowMapStates::default();
		states.update(&first, layout_of(&first), &views(&[1.0, 2.0], 0), 1.0, 2048, 2 * SMALLEST);
		let ids = (0..next.cones.len()).map(|id| id as f32 + 1.0).collect::<Vec<_>>();
		states.update(next, layout_of(next), &views(&ids, 0), 1.0, 2048, 0);
		(0..next.cones.len())
			.filter(|position| states.cone_view(*position).is_some())
			.count()
	}

	#[test]
	fn maps_keep_their_contents_until_their_class_or_image_size_changes() {
		let (light, transform) = (cone(), Transform::default());

		assert_eq!(
			drawn_after(&selection(&light, &transform, 2, 0, 0), 0),
			2,
			"Unchanged maps keep their contents."
		);
		assert_eq!(
			drawn_after(&selection(&light, &transform, 2, 1, 0), 0),
			1,
			"Maps in a new size class start over, so only the one that drew holds a view."
		);
		assert_eq!(
			drawn_after(&selection(&light, &transform, 3, 0, 0), 0),
			1,
			"The image grew to three layers, so every map in it starts over and only the one that drew holds a view."
		);
	}

	#[test]
	fn the_first_two_cascades_draw_every_frame_and_count_first() {
		let (light, transform) = (cone(), Transform::default());
		let selection = selection(&light, &transform, 1, 0, 1);
		let mut states = ShadowMapStates::default();

		// The budget holds exactly the two always-drawn cascades of 64 by 64 texels.
		let draws = states.update(&selection, layout_of(&selection), &views(&[1.0], 1), 1.0, 64, 2 * 64 * 64);
		let cascades = draws
			.iter()
			.filter(|draw| draw.image == ShadowImage::Directional)
			.map(|draw| draw.layer)
			.collect::<Vec<_>>();

		assert_eq!(cascades[..2], [0, 1]);
		// Nothing is left, so only the first candidate draws past the budget: one cascade or the cone.
		assert_eq!(draws.len(), 3);
	}

	#[test]
	fn earlier_tiers_draw_before_older_or_more_important_maps() {
		let candidates = [
			candidate(UpdateTier::Current, 100, 1000.0, 4),
			candidate(UpdateTier::ViewChanged, 1, 1.0, 4),
			candidate(UpdateTier::NotDrawn, 0, 1.0, 4),
		];

		assert_eq!(schedule(&candidates, 8), [false, true, true]);
	}

	#[test]
	fn within_a_tier_age_times_importance_orders_the_maps() {
		let candidates = [
			candidate(UpdateTier::Current, 2, 10.0, 4),
			candidate(UpdateTier::Current, 30, 1.0, 4),
			candidate(UpdateTier::Current, 1, 25.0, 4),
		];

		assert_eq!(schedule(&candidates, 4), [false, true, false]);
		assert_eq!(schedule(&candidates, 8), [false, true, true]);
	}

	#[test]
	fn a_map_that_does_not_fit_is_skipped_and_smaller_ones_after_it_still_draw() {
		let candidates = [
			candidate(UpdateTier::NotDrawn, 0, 1.0, 16),
			candidate(UpdateTier::Current, 5, 1.0, 4),
		];

		assert_eq!(schedule(&candidates, 8), [false, true]);
	}

	#[test]
	fn the_first_map_draws_when_nothing_fits() {
		let candidates = [
			candidate(UpdateTier::Current, 1, 1.0, 16),
			candidate(UpdateTier::NotDrawn, 0, 1.0, 32),
		];

		assert_eq!(schedule(&candidates, 0), [false, true]);
		assert!(schedule(&[], 0).is_empty());
	}
}
