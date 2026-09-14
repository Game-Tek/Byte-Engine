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
}

impl Default for VisualState {
	fn default() -> Self {
		Self {
			clip: EffectiveClip::Unbounded,
			descendant_clip: EffectiveClip::Unbounded,
			feather: None,
			descendant_feather: None,
			opacity: None,
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
		};
		if let Primitives::Container(container) = primitive {
			if container.clip {
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
		}
		states[index] = state;
	}
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

/// Keeps visible geometry for hit testing while preserving layout depth.
pub(super) fn clipped_hit_elements<'a>(
	elements: &[LayoutElement],
	tree: &RetainedTree,
	states: &[VisualState],
	frame_allocator: &'a bumpalo::Bump,
) -> Vec<LayoutElement, &'a bumpalo::Bump> {
	let mut clipped = Vec::new_in(frame_allocator);

	for element in elements.iter().filter(|element| element.hit_testable) {
		let Some(geometry) = tree
			.element_indices
			.get(&element.id)
			.map(|&index| states[index].clip)
			.unwrap_or(EffectiveClip::Unbounded)
			.apply(geometry_from_layout_element(element))
		else {
			continue;
		};

		if geometry.is_empty() {
			continue;
		}

		clipped.push(LayoutElement {
			id: element.id,
			position: Location3::new(geometry.x(), geometry.y(), element.position.z()),
			size: geometry.size,
			hit_testable: element.hit_testable,
		});
	}

	clipped
}

/// Applies inherited visual transforms in layout order without changing flow placement.
pub(super) fn apply_visual_transforms(elements: &mut [LayoutElement], tree: &RetainedTree, frame_allocator: &bumpalo::Bump) {
	let mut resolved = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	for _ in 0..tree.elements.len() {
		resolved.push(None);
	}

	for element in elements {
		let Some(&index) = tree.element_indices.get(&element.id) else {
			continue;
		};
		let parent_transform = tree.parents[index]
			.and_then(|parent| resolved[parent])
			.unwrap_or_else(Affine2::identity);
		let local_transform = *tree.elements[index].element.primitive.transform();
		let transform = parent_transform.compose(Affine2::from_transform(local_transform, element));
		let (position, size) = transform.transform_rect(element);

		element.position = position;
		element.size = size;
		resolved[index] = Some(transform);
	}
}
