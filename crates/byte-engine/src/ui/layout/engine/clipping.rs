//! Clipping, feather-mask, and visual-transform preparation.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum EffectiveClip {
	Unbounded,
	Empty,
	Rect(Geometry),
}

/// The `VisualState` struct carries inherited appearance in retained element order.
#[derive(Clone, Copy)]
pub(super) struct VisualState {
	pub(super) clip: EffectiveClip,
	descendant_clip: EffectiveClip,
	pub(super) feather: Option<FeatherMask>,
	descendant_feather: Option<FeatherMask>,
	opacity: Option<f32>,
	/// The product of every ancestor's and this element's visual scale, so curve
	/// points and glyph sizes follow a zoomed subtree the way its rectangles do.
	pub(super) scale: [f32; 2],
}

impl Default for VisualState {
	fn default() -> Self {
		Self {
			clip: EffectiveClip::Unbounded,
			descendant_clip: EffectiveClip::Unbounded,
			feather: None,
			descendant_feather: None,
			opacity: None,
			scale: [1.0, 1.0],
		}
	}
}

impl EffectiveClip {
	pub(super) fn apply(self, geometry: Geometry) -> Option<Geometry> {
		match self {
			EffectiveClip::Unbounded => Some(geometry),
			EffectiveClip::Empty => None,
			EffectiveClip::Rect(clip) => geometry.intersect(clip),
		}
	}

	pub(super) fn clip_descendants(self, geometry: Geometry) -> Self {
		match self.apply(geometry) {
			Some(geometry) => EffectiveClip::Rect(geometry),
			None => EffectiveClip::Empty,
		}
	}

	pub(super) fn as_rect(self) -> Option<Geometry> {
		match self {
			EffectiveClip::Rect(geometry) => Some(geometry),
			EffectiveClip::Unbounded | EffectiveClip::Empty => None,
		}
	}
}

pub(super) fn geometry_from_layout_element(element: &LayoutElement) -> Geometry {
	Geometry::new(element.position, element.size)
}

/// Resolves clip and feather inheritance together without allocating per-element maps.
pub(super) fn prepare_visual_state(elements: &[LayoutElement], tree: &RetainedTree, states: &mut Vec<VisualState>) {
	states.clear();
	states.resize(tree.elements.len(), VisualState::default());
	for element in elements {
		let Some(&index) = tree.element_indices.get(&element.id) else {
			continue;
		};
		let mut inherited = tree.parents[index].map(|parent| states[parent]).unwrap_or_default();
		let primitive = &tree.elements[index].element.primitive;
		// Transforms compose through absolute-depth layers even though clipping restarts there.
		let scale = composed_scale(inherited.scale, primitive.transform());
		if matches!(primitive, Primitives::Container(container) if matches!(container.depth, Depth::Absolute(_))) {
			inherited = VisualState::default();
		}
		let clip = inherited.descendant_clip;
		let feather = inherited.descendant_feather;
		let mut state = VisualState {
			clip,
			descendant_clip: clip,
			feather,
			descendant_feather: feather,
			opacity: None,
			scale,
		};
		if let Primitives::Container(container) = primitive
			&& container.clip {
				let geometry = geometry_from_layout_element(element);
				state.descendant_clip = clip.clip_descendants(geometry);
				state.descendant_feather = first_layer_feather(container.style.layers())
					.map(|feather| FeatherMask {
						geometry,
						feather,
						corner_radius: container.corner_radius,
						corner_exponent: container.corner_exponent,
					})
					.or(feather);
			}
		states[index] = state;
	}
}

/// Composes an element's visual scale onto its parent's, matching [`Affine2::from_transform`].
fn composed_scale(parent: [f32; 2], transform: &Transform) -> [f32; 2] {
	let sanitize = |value: f32| if value.is_finite() { value.max(0.0) } else { 1.0 };
	[
		parent[0] * sanitize(transform.scale_x),
		parent[1] * sanitize(transform.scale_y),
	]
}

/// Resolves opacity from the current tree, including parents changed after layout by input callbacks.
pub(super) fn effective_opacity(index: usize, tree: &RetainedTree, states: &mut [VisualState]) -> f32 {
	if let Some(opacity) = states[index].opacity {
		return opacity;
	}
	let parent = tree.parents[index]
		.map(|parent| effective_opacity(parent, tree, states))
		.unwrap_or(1.0);
	let local = sanitize_opacity(tree.elements[index].element.primitive.visual().opacity);
	let opacity = (parent * local).clamp(0.0, 1.0);
	states[index].opacity = Some(opacity);
	opacity
}

pub(super) fn first_layer_feather(layers: &[crate::ui::style::ConcreteLayer]) -> Option<EdgeFeather> {
	layers
		.iter()
		.map(crate::ui::style::Layer::feather)
		.find(|feather| !feather.is_none())
}

/// Hit geometry for one frame: clipped bounds, plus the polylines curves are hit along.
pub(super) struct HitGeometry<'a> {
	pub(super) elements: Vec<LayoutElement, &'a bumpalo::Bump>,
	pub(super) curves: Vec<HitCurve, &'a bumpalo::Bump>,
	pub(super) points: Vec<Location, &'a bumpalo::Bump>,
}

/// Layout distance a flattened curve may stray from its true shape for hit testing.
const HIT_CURVE_TOLERANCE: f32 = 0.25;

/// Keeps visible geometry for hit testing while preserving layout depth.
///
/// A hit-testable curve is flattened in layout units, with its inherited scale,
/// and bounded by its points widened by half its hit width, so a wire routed
/// beyond its declared size stays clickable along its whole length.
pub(super) fn clipped_hit_elements<'a>(
	elements: &[LayoutElement],
	tree: &RetainedTree,
	states: &[VisualState],
	curves: &mut HashMap<Id, crate::ui::components::curve::FlattenedCurve>,
	frame_allocator: &'a bumpalo::Bump,
) -> HitGeometry<'a> {
	let mut hit = HitGeometry {
		elements: Vec::new_in(frame_allocator),
		curves: Vec::new_in(frame_allocator),
		points: Vec::new_in(frame_allocator),
	};
	let mut flattened = Vec::new_in(frame_allocator);
	curves.retain(|id, _| tree.element_indices.contains_key(id));

	for element in elements.iter().filter(|element| element.hit_testable) {
		let index = tree.element_indices.get(&element.id).copied();
		let clip = index.map(|index| states[index].clip).unwrap_or(EffectiveClip::Unbounded);
		let curve = index.and_then(|index| match &tree.elements[index].element.primitive {
			Primitives::Curve(curve) => curve.hit_width().map(|width| (curve, width, states[index].scale)),
			_ => None,
		});
		// A curve is bounded by its flattened points; the half width is in scaled layout units.
		let (bounds, half_width) = match curve {
			Some((curve, width, scale)) => {
				flattened.clear();
				let origin = (element.position.x(), element.position.y());
				let cached = curves.entry(element.id).or_default();
				cached.update(curve.path().segments(), scale, HIT_CURVE_TOLERANCE);
				flattened.extend(
					cached
						.points
						.iter()
						.map(|point| CurvePoint::new(origin.0 + point.x, origin.1 + point.y)),
				);
				let half_width = width * scale[0].min(scale[1]) * 0.5;
				let Some(bounds) = polyline_bounds(&flattened, half_width) else {
					continue;
				};
				(
					Geometry::new(
						Location3::new(bounds.0, bounds.1, element.position.z()),
						Size::new(bounds.2, bounds.3),
					),
					Some(half_width),
				)
			}
			None => (geometry_from_layout_element(element), None),
		};
		let Some(geometry) = clip.apply(bounds) else {
			continue;
		};
		if geometry.is_empty() {
			continue;
		}

		if let Some(half_width) = half_width {
			hit.curves.push(HitCurve {
				id: element.id.get(),
				half_width,
				first: hit.points.len() as u32,
				count: flattened.len() as u32,
			});
			hit.points
				.extend(flattened.iter().map(|point| Location::new(point.x, point.y)));
		}
		hit.elements.push(LayoutElement {
			id: element.id,
			position: Location3::new(geometry.x(), geometry.y(), element.position.z()),
			size: geometry.size,
			hit_testable: element.hit_testable,
		});
	}

	hit
}

/// Bounds of a polyline widened by `half_width`, as (x, y, width, height).
fn polyline_bounds(points: &[CurvePoint], half_width: f32) -> Option<(f32, f32, f32, f32)> {
	let mut min = (f32::INFINITY, f32::INFINITY);
	let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
	for point in points {
		min = (min.0.min(point.x), min.1.min(point.y));
		max = (max.0.max(point.x), max.1.max(point.y));
	}
	if points.is_empty() || !half_width.is_finite() {
		return None;
	}
	Some((
		min.0 - half_width,
		min.1 - half_width,
		max.0 - min.0 + half_width * 2.0,
		max.1 - min.1 + half_width * 2.0,
	))
}

/// Updates one visual subtree from its retained, untransformed placement.
/// Parent transforms outside this boundary remain valid after a local edit.
pub(super) fn update_visual_subtree(
	index: usize,
	tree: &RetainedTree,
	placement: &[LayoutElement],
	indices: &[usize],
	resolved: &mut [Affine2],
	elements: &mut [LayoutElement],
	work: &mut Vec<usize>,
) {
	work.clear();
	work.push(index);
	while let Some(index) = work.pop() {
		let offset = indices[index];
		if offset == usize::MAX {
			continue;
		}
		let local = placement[offset];
		let parent = tree.parents[index].map_or_else(Affine2::identity, |parent| resolved[parent]);
		let transform = parent.compose(Affine2::from_transform(
			*tree.elements[index].element.primitive.transform(),
			&local,
		));
		let (position, size) = transform.transform_rect(&local);
		elements[offset].position = position;
		elements[offset].size = size;
		resolved[index] = transform;
		work.extend(tree.children[index].iter().rev().copied());
	}
}
