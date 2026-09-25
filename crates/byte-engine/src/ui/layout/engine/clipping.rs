//! Clipping, clip-mask, and visual-transform preparation.

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
	pub(super) mask: Option<ClipMask>,
	descendant_mask: Option<ClipMask>,
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
			mask: None,
			descendant_mask: None,
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

/// Resolves clip and mask inheritance together without allocating per-element maps.
pub(super) fn prepare_visual_state(elements: &[LayoutElement], tree: &RetainedTree, states: &mut Vec<VisualState>) {
	states.clear();
	states.resize(tree.elements.len(), VisualState::default());
	for element in elements {
		let Some(index) = tree.index_of(element) else {
			continue;
		};
		states[index] = inherited_visual_state(element, index, tree, states);
	}
}

/// Derives an element's state from its parent's already resolved state and its own geometry.
pub(super) fn inherited_visual_state(
	element: &LayoutElement,
	index: usize,
	tree: &RetainedTree,
	states: &[VisualState],
) -> VisualState {
	let mut inherited = tree.parents[index].map(|parent| states[parent]).unwrap_or_default();
	let primitive = &tree.elements[index].element.primitive;
	// Transforms compose through absolute-depth layers even though clipping restarts there.
	let scale = composed_scale(inherited.scale, primitive.transform());
	if matches!(primitive, Primitives::Container(container) if matches!(container.depth, Depth::Absolute(_))) {
		inherited = VisualState::default();
	}
	let clip = inherited.descendant_clip;
	let mask = inherited.descendant_mask;
	let mut state = VisualState {
		clip,
		descendant_clip: clip,
		mask,
		descendant_mask: mask,
		opacity: None,
		scale,
	};
	if let Primitives::Container(container) = primitive
		&& container.clip
	{
		// Descendants clip to the inside of the border, so they never paint over an inset stroke.
		let border = border_width(container.style.layers());
		let geometry = geometry_from_layout_element(element).expanded(-border);
		let corner_radius = (container.corner_radius - border).max(0.0);
		state.descendant_clip = clip.clip_descendants(geometry);
		// A rounded container masks its descendants even without a feather; the rectangle clip cannot round.
		let own_feather = first_layer_feather(container.style.layers());
		state.descendant_mask = (own_feather.is_some() || corner_radius > 0.0)
			.then(|| ClipMask {
				geometry,
				feather: own_feather.unwrap_or_else(EdgeFeather::none),
				corner_radius,
				corner_exponent: container.corner_exponent,
			})
			.or(mask);
	}
	state
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

/// Widest inset stroke among the layers, in layout units.
fn border_width(layers: &[crate::ui::style::ConcreteLayer]) -> f32 {
	layers
		.iter()
		.filter_map(|layer| match layer.kind() {
			LayerKind::Stroke { width } if width.is_finite() && width > 0.0 => Some(width),
			_ => None,
		})
		.fold(0.0, f32::max)
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

	// Every hit-testable element keeps an entry, empty when clipped away, so a stable
	// topology maps tree indices to the same entries across transform edits.
	for element in elements.iter().filter(|element| element.hit_testable) {
		let index = tree.index_of(element);
		let entry = hit_entry(element, index, tree, states, curves, &mut flattened);
		if let Some(half_width) = entry.half_width {
			hit.curves.push(HitCurve {
				id: element.id.get(),
				half_width,
				first: hit.points.len() as u32,
				count: flattened.len() as u32,
			});
			hit.points.extend_from_slice(&flattened);
		}
		// Placement is reused across sector edits, so the shape is read from the tree as it is now.
		let sector = index.and_then(|index| match &tree.elements[index].element.primitive {
			Primitives::Container(container) => container.sector,
			_ => None,
		});
		hit.elements.push(LayoutElement {
			id: element.id,
			index: element.index,
			position: entry.position,
			size: entry.size,
			hit_testable: element.hit_testable,
			sector,
		});
	}

	hit
}

/// One element's clipped hit bounds; `half_width` is set for a curve, whose polyline
/// is left in `flattened` in layout units.
struct HitEntry {
	position: Location3,
	size: Size,
	half_width: Option<f32>,
}

fn hit_entry(
	element: &LayoutElement,
	index: Option<usize>,
	tree: &RetainedTree,
	states: &[VisualState],
	curves: &mut HashMap<Id, crate::ui::components::curve::FlattenedCurve>,
	flattened: &mut Vec<Location, &bumpalo::Bump>,
) -> HitEntry {
	flattened.clear();
	let clip = index.map(|index| states[index].clip).unwrap_or(EffectiveClip::Unbounded);
	let curve = index.and_then(|index| match &tree.elements[index].element.primitive {
		Primitives::Curve(curve) => curve.hit_width().map(|width| (curve, width, states[index].scale)),
		_ => None,
	});
	// A curve is bounded by its flattened points; the half width is in scaled layout units.
	let (bounds, half_width) = match curve {
		Some((curve, width, scale)) => {
			let origin = (element.position.x(), element.position.y());
			let cached = curves.entry(element.id).or_default();
			cached.update(curve.path().segments(), scale, HIT_CURVE_TOLERANCE);
			flattened.extend(
				cached
					.points
					.iter()
					.map(|point| Location::new(origin.0 + point.x, origin.1 + point.y)),
			);
			let half_width = width * scale[0].min(scale[1]) * 0.5;
			let bounds = polyline_bounds(flattened, half_width).map(|bounds| {
				Geometry::new(
					Location3::new(bounds.0, bounds.1, element.position.z()),
					Size::new(bounds.2, bounds.3),
				)
			});
			(bounds, Some(half_width))
		}
		None => (Some(geometry_from_layout_element(element)), None),
	};
	let geometry = bounds
		.and_then(|bounds| clip.apply(bounds))
		.filter(|geometry| !geometry.is_empty());
	match geometry {
		Some(geometry) => HitEntry {
			position: Location3::new(geometry.x(), geometry.y(), element.position.z()),
			size: geometry.size,
			half_width,
		},
		None => HitEntry {
			position: Location3::new(0.0, 0.0, element.position.z()),
			size: Size::new(0.0, 0.0),
			half_width,
		},
	}
}

/// Refreshes the retained hit entries of moved elements instead of rebuilding the index.
///
/// Returns `false` when the index must be rebuilt: a curve's polyline changed length,
/// an entry moved past the grid, or the index was not retained for this topology.
#[allow(clippy::too_many_arguments)]
pub(super) fn refresh_hit_entries(
	dirty: &[usize],
	elements: &[LayoutElement],
	tree: &RetainedTree,
	states: &[VisualState],
	indices: &[usize],
	hit_offsets: &[u32],
	curves: &mut HashMap<Id, crate::ui::components::curve::FlattenedCurve>,
	acceleration: &mut MouseClickAcceleration,
	frame_allocator: &bumpalo::Bump,
) -> bool {
	let mut flattened = Vec::new_in(frame_allocator);
	for &index in dirty {
		let hit_offset = hit_offsets[index];
		if hit_offset == u32::MAX {
			continue;
		}
		let entry = hit_entry(&elements[indices[index]], Some(index), tree, states, curves, &mut flattened);
		let curve = entry.half_width.map(|half_width| (half_width, flattened.as_slice()));
		if !acceleration.patch(hit_offset as usize, entry.position, entry.size, curve) {
			return false;
		}
	}
	acceleration.commit();
	true
}

/// Bounds of a polyline widened by `half_width`, as (x, y, width, height).
fn polyline_bounds(points: &[Location], half_width: f32) -> Option<(f32, f32, f32, f32)> {
	let mut min = (f32::INFINITY, f32::INFINITY);
	let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
	for point in points {
		min = (min.0.min(point.x()), min.1.min(point.y()));
		max = (max.0.max(point.x()), max.1.max(point.y()));
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
///
/// Every placed element it visits is appended to `dirty`, parents before children,
/// for the stages that refresh appearance, hit geometry, and render entries in place.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_visual_subtree(
	index: usize,
	tree: &RetainedTree,
	placement: &[LayoutElement],
	indices: &[usize],
	resolved: &mut [Affine2],
	elements: &mut [LayoutElement],
	work: &mut Vec<usize>,
	dirty: &mut Vec<usize>,
) {
	work.clear();
	work.push(index);
	while let Some(index) = work.pop() {
		let offset = indices[index];
		if offset == usize::MAX {
			continue;
		}
		dirty.push(index);
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
