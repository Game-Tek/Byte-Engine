//! Clipping, feather-mask, and visual-transform preparation.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum EffectiveClip {
	Unbounded,
	Empty,
	Rect(Geometry),
}

#[derive(Clone, Copy)]
pub(super) struct ClipInfo {
	pub(super) element: EffectiveClip,
	pub(super) descendants: EffectiveClip,
}

#[derive(Clone, Copy)]
pub(super) struct FeatherInfo {
	pub(super) element: Option<FeatherMask>,
	pub(super) descendants: Option<FeatherMask>,
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

pub(super) fn element_clips<'a>(
	elements: impl IntoIterator<Item = &'a LayoutElement>,
	tree: &RetainedTree,
) -> HashMap<Id, ClipInfo> {
	let mut clips = HashMap::new();

	for element in elements {
		let parent_clip = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| clips.get(&parent).map(|clip: &ClipInfo| clip.descendants))
			.unwrap_or(EffectiveClip::Unbounded);
		let inherited = if element_resets_clip(element.id, tree) {
			EffectiveClip::Unbounded
		} else {
			parent_clip
		};
		let geometry = geometry_from_layout_element(element);
		let descendants = match tree.element(element.id).map(|element| &element.element.primitive) {
			Some(Primitives::Container(container)) if container.clip => inherited.clip_descendants(geometry),
			_ => inherited,
		};

		clips.insert(
			element.id,
			ClipInfo {
				element: inherited,
				descendants,
			},
		);
	}

	clips
}

pub(super) fn element_feather_masks<'a>(
	elements: impl IntoIterator<Item = &'a LayoutElement>,
	tree: &RetainedTree,
) -> HashMap<Id, FeatherInfo> {
	let mut masks = HashMap::new();

	for element in elements {
		let parent_mask = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| masks.get(&parent).and_then(|mask: &FeatherInfo| mask.descendants));
		let inherited = if element_resets_clip(element.id, tree) {
			None
		} else {
			parent_mask
		};
		let descendants = match tree.element(element.id).map(|element| &element.element.primitive) {
			Some(Primitives::Container(container)) if container.clip => first_layer_feather(container.style.layers())
				.map(|feather| FeatherMask {
					geometry: geometry_from_layout_element(element),
					feather,
					corner_radius: container.corner_radius,
					corner_exponent: container.corner_exponent,
				})
				.or(inherited),
			_ => inherited,
		};

		masks.insert(
			element.id,
			FeatherInfo {
				element: inherited,
				descendants,
			},
		);
	}

	masks
}

pub(super) fn element_resets_clip(id: Id, tree: &RetainedTree) -> bool {
	matches!(
		tree.element(id).map(|element| &element.element.primitive),
		Some(Primitives::Container(container)) if matches!(container.depth, Depth::Absolute(_))
	)
}

pub(super) fn first_layer_feather(layers: &[crate::ui::style::ConcreteLayer]) -> Option<EdgeFeather> {
	layers
		.iter()
		.map(crate::ui::style::Layer::feather)
		.find(|feather| !feather.is_none())
}

pub(super) fn clipped_layout_elements<'a>(
	elements: &[LayoutElement],
	tree: &RetainedTree,
	frame_allocator: &'a bumpalo::Bump,
) -> Vec<LayoutElement, &'a bumpalo::Bump> {
	let clips = element_clips(elements, tree);
	let mut clipped = Vec::with_capacity_in(elements.len(), frame_allocator);

	for element in elements {
		let Some(geometry) = clips
			.get(&element.id)
			.map(|clip| clip.element)
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

pub(super) fn apply_visual_transforms(elements: &mut [LayoutElement], tree: &RetainedTree, frame_allocator: &bumpalo::Bump) {
	let mut resolved = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	for _ in 0..tree.elements.len() {
		resolved.push(None);
	}

	for element in elements {
		let parent_transform = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| tree.element_indices.get(&parent).and_then(|index| resolved.get(*index)))
			.and_then(|transform| *transform)
			.unwrap_or_else(Affine2::identity);

		let local_transform = tree
			.element(element.id)
			.map(|retained_element| *retained_element.element.primitive.transform())
			.unwrap_or_default();
		let transform = parent_transform.compose(Affine2::from_transform(local_transform, element));
		let (position, size) = transform.transform_rect(element);

		element.position = position;
		element.size = size;
		if let Some(index) = tree.element_indices.get(&element.id).copied() {
			resolved[index] = Some(transform);
		}
	}
}
