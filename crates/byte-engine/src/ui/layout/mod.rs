pub mod context;
pub mod engine;
#[doc(hidden)]
pub mod query;
mod retained_tree;
#[doc(hidden)]
pub mod snapshot;
mod visual_transform;

use utils::{Box, RGBA};

use super::{
	Primitive,
	element::{self, Element, ElementHandle, Id},
	flow::{self, FlowInput, FlowOutput, Location, Location3, Offset, Size},
	primitive::BasePrimitive,
};
use crate::ui::{
	components::curve::CurveSegment,
	element::ConcreteElement,
	flow::FlowFunction,
	font::TextSystem,
	primitive::{Primitives, Shapes},
	style::{ConcreteStyle, EdgeFeather},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PathSegment {
	pub(crate) name: std::borrow::Cow<'static, str>,
	pub(crate) ordinal: u32,
}

/// The `LayoutElement` struct stores an element positioned and sized for a viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayoutElement {
	pub(crate) id: Id,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) hit_testable: bool,
}

/// The `RenderElement` struct stores an element prepared for rendering.
#[derive(Clone)]
pub(crate) struct RenderElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) clip: Option<Geometry>,
	pub(crate) clip_mask: Option<ClipMask>,
	/// The turn this element draws with; its position, size, clip, and mask are unrotated.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	pub(crate) style: ConcreteStyle,
	pub(crate) opacity: f32,
	pub(crate) backdrop_blur_radius: f32,
	pub(crate) corner_radius: f32,
	pub(crate) corner_exponent: f32,
}

#[derive(Clone)]
pub(crate) struct RenderTextElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) clip: Option<Geometry>,
	pub(crate) clip_mask: Option<ClipMask>,
	/// The turn this element draws with; its position, size, clip, and mask are unrotated.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	pub(crate) color: RGBA,
	pub(crate) opacity: f32,
	pub(crate) font_size: f32,
	/// Inherited visual scale applied to the font size when glyphs are placed.
	pub(crate) scale: f32,
	pub(crate) content: String,
}

#[derive(Clone)]
pub(crate) struct RenderImageElement {
	pub(crate) id: u32,
	pub(crate) image_id: u64,
	pub(crate) version: u64,
	pub(crate) source_width: u32,
	pub(crate) source_height: u32,
	pub(crate) pixels: std::sync::Arc<[u8]>,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) clip: Option<Geometry>,
	pub(crate) clip_mask: Option<ClipMask>,
	/// The turn this element draws with; its position, size, clip, and mask are unrotated.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	pub(crate) opacity: f32,
}

#[derive(Clone)]
pub(crate) struct RenderCurveElement {
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) clip: Option<Geometry>,
	pub(crate) clip_mask: Option<ClipMask>,
	/// The turn this element draws with; its position, size, clip, and mask are unrotated.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	pub(crate) style: ConcreteStyle,
	pub(crate) opacity: f32,
	/// Inherited visual scale applied to segment points and stroke width.
	pub(crate) scale: [f32; 2],
	pub(crate) segments: Vec<CurveSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClipMask {
	pub(crate) geometry: Geometry,
	pub(crate) feather: EdgeFeather,
	pub(crate) corner_radius: f32,
	pub(crate) corner_exponent: f32,
}

fn random_color_from_id(id: u32) -> RGBA {
	let mut state = id.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
	state ^= state >> 16;
	state = state.wrapping_mul(2_246_822_519);
	state ^= state >> 13;

	let r = ((state & 0xFF) as f32) / 255.0;
	let g = (((state >> 8) & 0xFF) as f32) / 255.0;
	let b = (((state >> 16) & 0xFF) as f32) / 255.0;

	RGBA::new(0.25 + r * 0.75, 0.25 + g * 0.75, 0.25 + b * 0.75, 1.0)
}

pub struct IdedElement {
	pub(crate) id: Id,
	pub(crate) element: ConcreteElement,
	pub(crate) path: usize,
	/// Last mutation of this node, used by its measurement cache.
	pub(crate) revision: u64,
}

impl ElementHandle for IdedElement {
	fn id(&self) -> Id {
		self.id
	}
}

// Cache keys include the node revision and parent space, so size changes naturally propagate.
type Measurement = Option<(u64, Size, Size)>;

/// Resolves and caches the primitive size used by placement and its parent flow.
fn measure_element(element: &IdedElement, available: Size, text: &mut TextSystem, cached: &mut Measurement) -> Size {
	if let Some((revision, parent_space, size)) = *cached
		&& revision == element.revision
		&& parent_space == available
	{
		return size;
	}
	let size = match &element.element.primitive {
		Primitives::Container(container) => Shapes::Box {
			half: (container.width, container.height),
			radius: container.corner_radius,
			exponent: container.corner_exponent,
		}
		.bbox(available),
		Primitives::Shape(shape) => shape.shape.bbox(available),
		Primitives::Curve(curve) => curve.path().size(available),
		Primitives::Image(image) => Shapes::Box {
			half: (image.width, image.height),
			radius: 0.0,
			exponent: 2.0,
		}
		.bbox(available),
		Primitives::Text(value) => text.measure(value.content(), value.settings().font_size),
		Primitives::TextField(value) => text.measure(value.content(), value.settings().font_size),
	};
	*cached = Some((element.revision, available, size));
	size
}

/// Replays flow placement while remeasuring only changed elements or parent spaces.
fn layout_elements<'a>(
	tree: &retained_tree::RetainedTree,
	available_space: Size,
	text_system: &mut TextSystem,
	measurements: &mut Vec<Measurement>,
	frame_allocator: &'a bumpalo::Bump,
) -> Vec<LayoutElement, &'a bumpalo::Bump> {
	let mut elements = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	measurements.resize(tree.elements.len(), None);
	if tree.elements.is_empty() {
		return elements;
	}

	// Placement uses the size already measured for the parent's flow. Visual transforms
	// remain a later pass and cannot move the flow cursor or change sibling sizing.
	#[allow(clippy::too_many_arguments)]
	fn place(
		tree: &retained_tree::RetainedTree,
		index: usize,
		size: Size,
		root_size: Size,
		offset: Offset,
		anchored: bool,
		text: &mut TextSystem,
		measurements: &mut [Measurement],
		slots: &mut [usize],
		output: &mut Vec<LayoutElement, &bumpalo::Bump>,
	) {
		let element = &tree.elements[index];
		// Flow keeps children inside the viewport; an anchored child may sit partly
		// outside its parent, such as a canvas node panned past the left edge.
		let (x, y) = if anchored {
			(offset.x(), offset.y())
		} else {
			(offset.x().max(0.0), offset.y().max(0.0))
		};
		// The paint walk below assigns the depth once every element is placed.
		let position = Location3::new(x, y, 0);
		let hit_testable = match &element.element.primitive {
			Primitives::Container(container) => container.hit_testable,
			Primitives::TextField(_) => true,
			Primitives::Curve(curve) => curve.hit_width().is_some(),
			_ => false,
		};
		slots[index] = output.len();
		output.push(LayoutElement {
			id: element.id,
			position,
			size,
			hit_testable,
		});
		let Primitives::Container(container) = &element.element.primitive else {
			return;
		};
		let origin: Offset = Into::<Location>::into(position).into();
		let mut cursor = origin;
		// Absolute-depth layers start from the viewport after ordinary flow children.
		for reset_layer in [false, true] {
			for &child_index in &tree.children[index] {
				let child = &tree.elements[child_index];
				let child_container = match &child.element.primitive {
					Primitives::Container(value) => Some(value),
					_ => None,
				};
				let reset = child_container.is_some_and(|value| matches!(value.depth, Depth::Absolute(_)));
				if reset != reset_layer {
					continue;
				}
				let available = if reset { root_size } else { size };
				let child_size = measure_element(child, available, text, &mut measurements[child_index]);
				let anchored = matches!(child_container.map(|value| value.position), Some(Position::Absolute { .. }));
				let flow_output = match child_container.map(|value| value.position) {
					// Absolute positions are offsets from the parent's top-left corner.
					Some(Position::Absolute { x, y }) => FlowOutput::new(Offset::new(origin.x() + x, origin.y() + y), cursor),
					_ if reset => FlowOutput::new(Offset::new(0.0, 0.0), cursor),
					_ => container.flow.call(FlowInput::new(size, cursor, child_size)),
				};
				place(
					tree,
					child_index,
					child_size,
					root_size,
					flow_output.child_offset(),
					anchored,
					text,
					measurements,
					slots,
					output,
				);
				if !reset {
					cursor = flow_output.next_cursor();
				}
			}
		}
	}

	// Depth is an element's rank in the paint order. A container is painted before its children and
	// its children in the order of their relative depths, so a subtree takes one contiguous range of
	// ranks and none of its elements can end up over or under another component's.
	fn rank(
		tree: &retained_tree::RetainedTree,
		index: usize,
		next: &mut u32,
		slots: &[usize],
		siblings: &mut Vec<(i32, usize), &bumpalo::Bump>,
		layers: &mut Vec<(i32, usize), &bumpalo::Bump>,
		output: &mut [LayoutElement],
	) {
		let position = &mut output[slots[index]].position;
		*position = Location3::new(position.x(), position.y(), *next);
		*next += 1;
		if !matches!(tree.elements[index].element.primitive, Primitives::Container(_)) {
			return;
		}
		// Every level sorts its own tail of the shared scratch list.
		let start = siblings.len();
		for &child in &tree.children[index] {
			match &tree.elements[child].element.primitive {
				Primitives::Container(container) => match container.depth {
					Depth::Relative(depth) => siblings.push((depth, child)),
					Depth::Absolute(depth) => layers.push((depth, child)),
				},
				_ => siblings.push((1, child)),
			}
		}
		// The sort is stable, so children at the same depth keep their declaration order.
		siblings[start..].sort_by_key(|&(depth, _)| depth);
		let end = siblings.len();
		for sibling in start..end {
			let child = siblings[sibling].1;
			rank(tree, child, next, slots, siblings, layers, output);
		}
		siblings.truncate(start);
	}

	let root = tree
		.parents
		.iter()
		.position(Option::is_none)
		.expect("Root container not found");
	let root_size = measure_element(&tree.elements[root], available_space, text_system, &mut measurements[root]);
	let mut slots = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	slots.resize(tree.elements.len(), 0);
	place(
		tree,
		root,
		root_size,
		available_space,
		Offset::new(0.0, 0.0),
		false,
		text_system,
		measurements,
		&mut slots,
		&mut elements,
	);

	let mut next = 0;
	let mut siblings = Vec::new_in(frame_allocator);
	let mut layers = Vec::new_in(frame_allocator);
	rank(tree, root, &mut next, &slots, &mut siblings, &mut layers, &mut elements);
	// Absolute-depth layers leave their parent and paint above all ordinary content, lowest first and
	// in tree order among equals. A layer found inside another one is ranked after it, which keeps it
	// above the layer that opened it.
	while let Some(lowest) = (0..layers.len()).min_by_key(|&layer| layers[layer].0) {
		let (_, index) = layers.remove(lowest);
		rank(tree, index, &mut next, &slots, &mut siblings, &mut layers, &mut elements);
	}
	elements
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
	Relative(i32),
	Absolute(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
	Flow,
	Absolute { x: f32, y: f32 },
}

impl Position {
	pub fn flow() -> Self {
		Self::Flow
	}

	pub fn absolute(x: impl Into<f64>, y: impl Into<f64>) -> Self {
		Self::Absolute {
			x: x.into() as f32,
			y: y.into() as f32,
		}
	}
}

impl Default for Position {
	fn default() -> Self {
		Self::flow()
	}
}

impl From<(i32, i32)> for Position {
	fn from((x, y): (i32, i32)) -> Self {
		Self::absolute(x, y)
	}
}

impl From<(u32, u32)> for Position {
	fn from((x, y): (u32, u32)) -> Self {
		Self::absolute(x, y)
	}
}

impl From<(f32, f32)> for Position {
	fn from((x, y): (f32, f32)) -> Self {
		Self::absolute(x, y)
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
	pub position: Location3,
	pub size: Size,
}

impl Geometry {
	pub fn new(position: Location3, size: Size) -> Self {
		Self { position, size }
	}

	pub fn x(&self) -> f32 {
		self.position.x()
	}

	pub fn y(&self) -> f32 {
		self.position.y()
	}

	pub fn z(&self) -> u32 {
		self.position.z()
	}

	pub fn width(&self) -> f32 {
		self.size.x()
	}

	pub fn height(&self) -> f32 {
		self.size.y()
	}

	pub fn right(&self) -> f32 {
		self.x() + self.width()
	}

	pub fn bottom(&self) -> f32 {
		self.y() + self.height()
	}

	pub fn is_empty(&self) -> bool {
		self.width() <= 0.0 || self.height() <= 0.0
	}

	/// Returns the smallest rectangle containing both, keeping this depth.
	pub fn union(self, other: Self) -> Self {
		let left = self.x().min(other.x());
		let top = self.y().min(other.y());
		let right = self.right().max(other.right());
		let bottom = self.bottom().max(other.bottom());
		Self::new(Location3::new(left, top, self.z()), Size::new(right - left, bottom - top))
	}

	/// Grows every edge by `outset` layout units.
	pub fn expanded(self, outset: f32) -> Self {
		Self::new(
			Location3::new(self.x() - outset, self.y() - outset, self.z()),
			Size::new(self.width() + outset * 2.0, self.height() + outset * 2.0),
		)
	}

	pub fn intersect(self, other: Self) -> Option<Self> {
		let left = self.x().max(other.x());
		let top = self.y().max(other.y());
		let right = self.right().min(other.right());
		let bottom = self.bottom().min(other.bottom());

		if right <= left || bottom <= top {
			return None;
		}

		Some(Self::new(
			Location3::new(left, top, self.z()),
			Size::new(right - left, bottom - top),
		))
	}
}

impl Depth {
	pub fn relative(depth: i32) -> Self {
		Self::Relative(depth)
	}

	pub fn absolute(depth: i32) -> Self {
		Self::Absolute(depth)
	}
}

impl Default for Depth {
	fn default() -> Self {
		Self::relative(1)
	}
}

impl From<i16> for Depth {
	fn from(value: i16) -> Self {
		Self::relative(value.into())
	}
}

impl From<i32> for Depth {
	fn from(value: i32) -> Self {
		Self::relative(value)
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sizing {
	Relative(u16, u16),
	Absolute(f32),
}

impl Sizing {
	pub fn full() -> Self {
		Self::Relative(1, 1)
	}

	pub fn pixels(value: impl Into<f64>) -> Self {
		Self::Absolute(value.into() as f32)
	}

	pub fn calculate(&self, available: f32) -> f32 {
		match self {
			Sizing::Relative(num, denom) => {
				debug_assert_ne!(
					*denom, 0,
					"Relative sizing denominator is zero. The most likely cause is constructing `Sizing::Relative` directly with an invalid ratio."
				);
				available * *num as f32 / *denom as f32
			}
			Sizing::Absolute(value) => *value,
		}
	}
}

impl Default for Sizing {
	fn default() -> Self {
		Self::full()
	}
}

impl From<u32> for Sizing {
	fn from(val: u32) -> Self {
		Sizing::Absolute(val as f32)
	}
}

impl From<f32> for Sizing {
	fn from(val: f32) -> Self {
		Sizing::Absolute(val)
	}
}

#[cfg(test)]
mod tests {

	use super::super::{
		Element,
		components::container::Container,
		element::{ElementHandle, Id},
		flow::{self, Location, Location3, Size},
		layout::{ConcreteElement, Depth, Position, Sizing},
	};
	use super::LayoutElement;
	use crate::ui::{
		font::TextSystem,
		layout::IdedElement,
		primitive::{Primitives, Shapes},
	};

	/// Supplies retained topology to the layout seam from declarative test fixtures.
	fn layout_elements<'a>(
		elements: Vec<IdedElement>,
		relations: &[(Id, Id)],
		size: Size,
		text: &mut TextSystem,
		allocator: &'a bumpalo::Bump,
	) -> Vec<LayoutElement, &'a bumpalo::Bump> {
		let mut tree = super::retained_tree::RetainedTree::new();
		tree.elements = elements;
		tree.relations.extend_from_slice(relations);
		tree.rebuild_element_indices();
		super::layout_elements(&tree, size, text, &mut Vec::new(), allocator)
	}

	fn make_elements(elements: impl IntoIterator<Item = Container>) -> Vec<IdedElement> {
		let mut counter = Id::MIN;

		elements
			.into_iter()
			.map(|e| {
				let id = counter;

				counter = counter.checked_add(1).expect("expected test value");

				IdedElement {
					id,
					element: ConcreteElement {
						primitive: Primitives::Container(e),
					},
					path: 0,
					revision: 0,
				}
			})
			.collect()
	}

	/// Lays out test containers whose relationships refer to their array indexes.
	fn layout(
		containers: impl IntoIterator<Item = Container>,
		relations: &[(usize, usize)],
		viewport: Size,
	) -> std::vec::Vec<LayoutElement> {
		let elements = make_elements(containers);
		let relations = relations
			.iter()
			.map(|&(parent, child)| (elements[parent].id(), elements[child].id()))
			.collect::<std::vec::Vec<_>>();
		let frame_allocator = bumpalo::Bump::new();

		layout_elements(elements, &relations, viewport, &mut TextSystem::new(), &frame_allocator)
			.into_iter()
			.collect()
	}

	fn assert_layout(element: &LayoutElement, size: Size, position: Location3) {
		assert_eq!(element.size, size);
		assert_eq!(element.position, position);
	}

	#[test]
	fn layout_root() {
		let elements = layout([Container::default()], &[], Size::new(1024, 10));

		assert_eq!(elements.len(), 1);
		assert_eq!(elements[0].size, Size::new(1024, 10));
	}

	#[test]
	fn layout_root_half_size() {
		let elements = layout([Container::default().size(Sizing::Relative(1, 2))], &[], Size::new(1024, 10));

		assert_eq!(elements.len(), 1);
		assert_eq!(elements[0].size, Size::new(512, 5));
	}

	#[test]
	fn layout_half_children() {
		let half = || Container::default().size(Sizing::Relative(1, 2));
		let elements = layout(
			[Container::default(), half(), half(), half(), half()],
			&[(0, 1), (1, 2), (2, 3), (3, 4)],
			Size::new(1024, 1024),
		);

		let expected_sizes = [1024, 512, 256, 128, 64];
		for (index, size) in expected_sizes.into_iter().enumerate() {
			assert_layout(&elements[index], Size::new(size, size), Location3::new(0, 0, index as u32));
		}
	}

	#[test]
	fn layout_column() {
		let child = || Container::default().size(Sizing::pixels(64));
		let elements = layout(
			[Container::default().flow(flow::column), child(), child(), child(), child()],
			&[(0, 1), (0, 2), (0, 3), (0, 4)],
			Size::new(1024, 1024),
		);

		assert_layout(&elements[0], Size::new(1024, 1024), Location3::new(0, 0, 0));
		for (index, y) in [0, 64, 128, 192].into_iter().enumerate() {
			assert_layout(
				&elements[index + 1],
				Size::new(64, 64),
				Location3::new(0, y, index as u32 + 1),
			);
		}
	}

	/// Returns each container's depth, in the order the containers were given to `layout`.
	fn depths(containers: impl IntoIterator<Item = Container>, relations: &[(usize, usize)]) -> std::vec::Vec<u32> {
		let elements = make_elements(containers);
		let ids: std::vec::Vec<Id> = elements.iter().map(|element| element.id()).collect();
		let relations: std::vec::Vec<(Id, Id)> = relations.iter().map(|&(parent, child)| (ids[parent], ids[child])).collect();
		let frame_allocator = bumpalo::Bump::new();
		let placed = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);
		ids.iter()
			.map(|id| placed.iter().find(|element| element.id == *id).expect("placed").position.z())
			.collect()
	}

	#[test]
	fn layout_relative_depth_orders_siblings_inside_their_parent() {
		let raised = Container::default().depth(Depth::relative(2));
		// root -> [raised -> raised_child, regular]
		let depths = depths(
			[Container::default(), raised, Container::default(), Container::default()],
			&[(0, 1), (1, 2), (0, 3)],
		);

		// The regular sibling paints first although it is declared last, and the raised child follows its parent.
		assert_eq!(depths, [0, 2, 3, 1]);
	}

	#[test]
	fn layout_relative_depth_cannot_cross_into_another_component() {
		let overlay = Container::default().depth(Depth::relative(8));
		// root -> [first -> overlay -> overlay_child, second -> second_child]
		let depths = depths(
			[
				Container::default(),
				Container::default(),
				overlay,
				Container::default(),
				Container::default(),
				Container::default(),
			],
			&[(0, 1), (1, 2), (2, 3), (0, 4), (4, 5)],
		);

		// Everything in the first component stays under everything in the second one.
		assert_eq!(depths, [0, 1, 2, 3, 4, 5]);
	}

	#[test]
	fn layout_absolute_depth_paints_above_a_deeper_later_component() {
		let modal = Container::default().depth(Depth::absolute(1));
		let overlay = Container::default().depth(Depth::relative(8));
		// root -> [toolbar -> modal -> modal_child, graph -> overlay -> overlay_child]
		let depths = depths(
			[
				Container::default(),
				Container::default(),
				modal,
				Container::default(),
				Container::default(),
				overlay,
				Container::default(),
			],
			&[(0, 1), (1, 2), (2, 3), (0, 4), (4, 5), (5, 6)],
		);

		assert_eq!(depths, [0, 1, 5, 6, 2, 3, 4]);
	}

	#[test]
	fn layout_absolute_depth_layers_stack_by_value_and_above_their_host() {
		let toast = Container::default().depth(Depth::absolute(4));
		let modal = Container::default().depth(Depth::absolute(1));
		let nested_modal = Container::default().depth(Depth::absolute(1));
		let popup = Container::default().depth(Depth::absolute(1));
		// root -> [toast -> popup, modal -> nested_modal]
		let depths = depths(
			[Container::default(), toast, modal, nested_modal, popup],
			&[(0, 1), (0, 2), (2, 3), (1, 4)],
		);

		// The modals come first, the nested one over its host, then the toast and the popup it opened.
		assert_eq!(depths, [0, 3, 1, 2, 4]);
	}

	#[test]
	fn layout_absolute_depth_siblings_stack_in_layout_order() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let first_modal = Container::default().depth(Depth::absolute(1));
		let second_modal = Container::default().depth(Depth::absolute(1));

		let elements = make_elements([root, first_modal, second_modal]);

		let root = &elements[0];
		let first_modal = &elements[1];
		let second_modal = &elements[2];

		let relations = [(root.id(), first_modal.id()), (root.id(), second_modal.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[0].position, Location3::new(0, 0, 0));
		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position.z(), 2);
	}

	#[test]
	fn layout_absolute_depth_resets_position_to_root_origin() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row_with_gap(10));
		let menu_item = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));
		let modal = Container::default()
			.width(Sizing::pixels(30))
			.height(Sizing::pixels(30))
			.depth(Depth::absolute(1));

		let elements = make_elements([root, menu_item, modal]);

		let root = &elements[0];
		let menu_item = &elements[1];
		let modal = &elements[2];

		let relations = [(root.id(), menu_item.id()), (root.id(), modal.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(0, 0, 2));
	}

	#[test]
	fn layout_absolute_position_places_child_without_advancing_flow() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row);
		let first = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));
		let positioned = Container::default()
			.width(Sizing::pixels(30))
			.height(Sizing::pixels(30))
			.position(Position::absolute(70, 12));
		let second = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));

		let elements = make_elements([root, first, positioned, second]);

		let root = &elements[0];
		let first = &elements[1];
		let positioned = &elements[2];
		let second = &elements[3];

		let relations = [
			(root.id(), first.id()),
			(root.id(), positioned.id()),
			(root.id(), second.id()),
		];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(70, 12, 2));
		assert_eq!(elements[3].position, Location3::new(20, 0, 3));
	}

	#[test]
	fn layout_absolute_depth_uses_absolute_position_in_root_space() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row);
		let first = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));
		let dropdown = Container::default()
			.width(Sizing::pixels(30))
			.height(Sizing::pixels(30))
			.depth(Depth::absolute(1))
			.absolute_position(24, 32);
		let child = Container::default().width(Sizing::pixels(10)).height(Sizing::pixels(10));

		let elements = make_elements([root, first, dropdown, child]);

		let root = &elements[0];
		let first = &elements[1];
		let dropdown = &elements[2];
		let child = &elements[3];

		let relations = [
			(root.id(), first.id()),
			(root.id(), dropdown.id()),
			(dropdown.id(), child.id()),
		];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(24, 32, 2));
		assert_eq!(elements[3].position, Location3::new(24, 32, 3));
	}

	#[test]
	fn layout_absolute_position_keeps_negative_coordinates() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let child = Container::default()
			.width(Sizing::pixels(30))
			.height(Sizing::pixels(30))
			.position(Position::absolute(-10, -20));

		let elements = make_elements([root, child]);

		let root = &elements[0];
		let child = &elements[1];

		let relations = [(root.id(), child.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		// An anchored child may start outside its parent, such as a canvas node panned past the edge.
		assert_eq!(elements[1].position, Location3::new(-10, -20, 1));
		assert_eq!(elements[1].size, Size::new(30, 30));
	}

	#[test]
	fn layout_absolute_depth_does_not_advance_parent_flow_cursor() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::row_with_gap(10));
		let first = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));
		let modal = Container::default()
			.width(Sizing::pixels(30))
			.height(Sizing::pixels(30))
			.depth(Depth::absolute(1));
		let second = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(20));

		let elements = make_elements([root, first, modal, second]);

		let root = &elements[0];
		let first = &elements[1];
		let modal_id = elements[2].id();
		let modal = &elements[2];
		let second = &elements[3];

		let relations = [(root.id(), first.id()), (root.id(), modal.id()), (root.id(), second.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].position, Location3::new(0, 0, 1));
		assert_eq!(elements[2].position, Location3::new(30, 0, 2));
		assert_eq!(elements[3].id, modal_id);
		assert_eq!(elements[3].position, Location3::new(0, 0, 3));
	}

	#[test]
	fn layout_absolute_depth_resolves_after_relative_siblings_even_when_declared_first() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default();
		let modal = Container::default().depth(Depth::absolute(1));
		let background = Container::default();

		let elements = make_elements([root, modal, background]);

		let root = &elements[0];
		let modal_id = elements[1].id();
		let background_id = elements[2].id();
		let modal = &elements[1];
		let background = &elements[2];

		let relations = [(root.id(), modal.id()), (root.id(), background.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements[1].id, background_id);
		assert_eq!(elements[1].position.z(), 1);
		assert_eq!(elements[2].id, modal_id);
		assert_eq!(elements[2].position.z(), 2);
	}

	#[test]
	fn layout_centered_column() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::centered_column);
		let a = Container::default().width(Sizing::pixels(64)).height(Sizing::pixels(32));
		let b = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(16));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 100),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 3);

		let element = &elements[1];

		assert_eq!(element.position, Location3::new(18, 0, 1));
		assert_eq!(element.size, Size::new(64, 32));

		let element = &elements[2];

		assert_eq!(element.position, Location3::new(40, 32, 2));
		assert_eq!(element.size, Size::new(20, 16));
	}

	#[test]
	fn layout_centered_row_keeps_siblings_on_same_baseline() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::centered_row);
		let a = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(10));
		let b = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(10));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 80),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 3);
		assert_eq!(elements[1].position, Location3::new(0, 35, 1));
		assert_eq!(elements[2].position, Location3::new(20, 35, 2));
	}

	#[test]
	fn layout_center() {
		let frame_allocator = bumpalo::Bump::new();
		let root = Container::default().flow(flow::center);
		let a = Container::default().width(Sizing::pixels(20)).height(Sizing::pixels(10));
		let b = Container::default().width(Sizing::pixels(40)).height(Sizing::pixels(20));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id(), a.id()), (root.id(), b.id())];

		let elements = layout_elements(
			elements,
			&relations,
			Size::new(100, 80),
			&mut TextSystem::new(),
			&frame_allocator,
		);

		assert_eq!(elements.len(), 3);

		let element = &elements[1];

		assert_eq!(element.position, Location3::new(40, 35, 1));
		assert_eq!(element.size, Size::new(20, 10));

		let element = &elements[2];

		assert_eq!(element.position, Location3::new(30, 30, 2));
		assert_eq!(element.size, Size::new(40, 20));
	}
}
