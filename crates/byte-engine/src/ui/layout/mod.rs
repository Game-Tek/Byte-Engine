pub mod context;
pub mod engine;
mod retained_tree;
pub mod snapshot;
mod visual_transform;

use utils::RGBA;

use super::{
	element::{self, Id},
	flow::{self, FlowInput, FlowOutput, Location, Location3, Offset, Size},
};
use crate::ui::{
	components::{
		container::{Container, Sector},
		curve::CurveSegment,
	},
	font::TextSystem,
	primitive::Primitives,
	style::{ConcreteStyle, EdgeFeather},
	transform::Transform,
};

/// The `LayoutElement` struct stores an element positioned and sized for a viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayoutElement {
	pub(crate) id: Id,
	/// The element's index in the tree it was laid out from, so walks over a
	/// snapshot need no ID hashing; see [`retained_tree::RetainedTree::index_of`].
	pub(crate) index: usize,
	pub(crate) position: Location3,
	/// The logical size: what the parent's flow placed, what hits test against, and what transform origins follow.
	pub(crate) size: Size,
	/// The size the element is seen at, from the same top-left corner: what paints, clips descendants, and damages.
	/// It equals [`Self::size`] for every element but a container with a display size; see [`Container::display_width`].
	pub(crate) display_size: Size,
	pub(crate) hit_testable: bool,
}

impl LayoutElement {
	/// The box the element is seen as, which painting, descendant clipping, and damage use.
	pub(crate) fn display_geometry(&self) -> Geometry {
		Geometry::new(self.position, self.display_size)
	}

	/// Both of the element's boxes, as a geometry read reports them.
	pub(crate) fn geometry(&self) -> ElementGeometry {
		ElementGeometry {
			logical: Geometry::new(self.position, self.size),
			display: self.display_geometry(),
		}
	}
}

/// The `ElementGeometry` struct reports where an element was laid out in the last evaluated frame, as two boxes that
/// share a top-left corner.
///
/// Read it with [`crate::ui::EvaluationContext::geometry`]. Use [`Self::logical`] for interaction, such as turning a
/// pointer position into a local one or anchoring a popover, since hits test against it and it never animates while
/// a container's display size does. Use [`Self::display`] to follow what is seen. Both include visual transforms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementGeometry {
	/// The box at the element's logical size, which pointer hits test against.
	pub logical: Geometry,
	/// The box at the element's display size, which is painted and clips its descendants.
	pub display: Geometry,
}

/// The `RenderPlacement` struct holds where and how one render list entry draws, whatever its kind.
///
/// Every entry of a [`engine::Render`] carries one, so the render build patches it in place when only the element's
/// geometry or inherited appearance moved, and the render pass converts it the same way for every kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RenderPlacement {
	/// The element's compact render order; see [`IdedElement::serial`].
	pub(crate) id: u32,
	pub(crate) position: Location3,
	pub(crate) size: Size,
	pub(crate) clip: Option<Geometry>,
	pub(crate) clip_mask: Option<ClipMask>,
	/// The turn this element draws with; its position, size, clip, and mask are unrotated.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	/// The inherited opacity, already multiplied through the element's ancestors.
	pub(crate) opacity: f32,
	/// The inherited visual scale, which curve points, stroke widths, glyph sizes, and unboxed paths follow.
	pub(crate) scale: [f32; 2],
	/// One more than the index of this entry's [`RenderMotion`] in the render, or 0 when it didn't move.
	pub(crate) motion: u32,
}

/// The `RenderMotion` struct records where a moving element was drawn in the previous render, so the render pass can
/// blur it along its motion; see [`engine::Render::motion`].
///
/// Every field is in layout units and unrotated, like the current [`RenderPlacement`] it pairs with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RenderMotion {
	/// The previous display box.
	pub(crate) position: [f32; 2],
	pub(crate) size: [f32; 2],
	/// The previous turn.
	pub(crate) rotation: Option<crate::ui::transform::Rotation>,
	/// The previous clip, which the blur moves the current clip back to.
	pub(crate) clip: Option<Geometry>,
	/// The previous mask box, which the blur moves the current mask back to.
	pub(crate) mask: Option<Geometry>,
}

/// The `RenderElement` struct stores a rectangle or sector prepared for rendering.
#[derive(Clone)]
pub(crate) struct RenderElement {
	pub(crate) placement: RenderPlacement,
	pub(crate) style: ConcreteStyle,
	/// The corner radii as the container sets them, top left, top right, bottom right, bottom left; the render pass
	/// fits them to the box.
	pub(crate) corner_radii: [f32; 4],
	pub(crate) corner_exponent: f32,
	pub(crate) sector: Option<Sector>,
}

#[derive(Clone)]
pub(crate) struct RenderTextElement {
	pub(crate) placement: RenderPlacement,
	pub(crate) color: RGBA,
	/// The unscaled font size; glyphs are placed at it times the smaller placement scale.
	pub(crate) font_size: f32,
	/// The name of the font to draw with, or `None` for the default.
	pub(crate) font: Option<&'static str>,
	/// The width lines break to, in multiples of the font size, or `None` for a text that doesn't wrap.
	pub(crate) wrap: Option<f32>,
	pub(crate) content: String,
}

#[derive(Clone)]
pub(crate) struct RenderImageElement {
	pub(crate) placement: RenderPlacement,
	pub(crate) image_id: u64,
	pub(crate) version: u64,
	pub(crate) source_width: u32,
	pub(crate) source_height: u32,
	pub(crate) source: crate::ui::components::image::ImageSource,
}

#[derive(Clone)]
pub(crate) struct RenderPathElement {
	/// Its scale applies only when the path has no view box.
	pub(crate) placement: RenderPlacement,
	pub(crate) path_id: u64,
	pub(crate) version: u64,
	pub(crate) fill_rule: crate::ui::components::path::FillRule,
	pub(crate) view_box: Option<[f32; 2]>,
	pub(crate) style: ConcreteStyle,
	/// Unscaled segments in path units, shared with the draw list; the pass packs them once per content key.
	pub(crate) segments: std::sync::Arc<[CurveSegment]>,
}

#[derive(Clone)]
pub(crate) struct RenderCurveElement {
	pub(crate) placement: RenderPlacement,
	pub(crate) style: ConcreteStyle,
	/// Segments in the curve's own units; the placement scale applies to their points and stroke width.
	pub(crate) segments: Vec<CurveSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClipMask {
	pub(crate) geometry: Geometry,
	pub(crate) feather: EdgeFeather,
	/// Each corner's radii (across, down), top left, top right, bottom right, bottom left: elliptical when a border's
	/// sides differ; see [`crate::ui::components::container::inner_corners`].
	pub(crate) corners: [[f32; 2]; 4],
	pub(crate) corner_exponent: f32,
}

/// The `IdedElement` struct is one node of the retained tree: an element's identity, the look every kind shares,
/// and its kind-specific [`Primitives`] state.
///
/// Declarations and edits write the node through [`engine::Properties`]; layout, hit testing, and rendering read it.
pub struct IdedElement {
	pub(crate) id: Id,
	/// The element's compact render order: its place among the elements the tree created, which render data and
	/// render-pass caches use instead of the 64-bit id.
	pub(crate) serial: u32,
	/// Last mutation of this node, used by its measurement cache.
	pub(crate) revision: u64,
	/// The layers the element paints, in order.
	pub(crate) style: ConcreteStyle,
	/// The visual transform, applied after placement; it never moves the flow cursor.
	pub(crate) transform: Transform,
	/// The element's own opacity, which multiplies its ancestors'.
	pub(crate) opacity: f32,
	pub(crate) primitive: Primitives,
}

impl IdedElement {
	/// Makes a node with the default look: no transform, full opacity, and one white fill layer, except for a
	/// container, which paints nothing until it is styled, so a container that only arranges its children needs no
	/// setup.
	pub(crate) fn new(id: Id, serial: u32, revision: u64, primitive: Primitives) -> Self {
		let style = match primitive {
			Primitives::Container(_) => ConcreteStyle::new(),
			_ => ConcreteStyle::default(),
		};
		Self {
			id,
			serial,
			revision,
			style,
			transform: Transform::default(),
			opacity: 1.0,
			primitive,
		}
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
	let size = match &element.primitive {
		Primitives::Container(container) => box_size(container.width, container.height, available),
		Primitives::Curve(curve) => box_size(curve.path.width, curve.path.height, available),
		Primitives::Path(path) => box_size(path.path.width, path.path.height, available),
		Primitives::Image(image) => box_size(image.width, image.height, available),
		Primitives::Text(value) => {
			let settings = value.settings();
			// A wrapping text takes the offered width, and breaks its lines against it in multiples of its font size.
			match text_wrap(value, available.x()) {
				Some(wrap) => Size::new(
					available.x(),
					text.measure(value.content(), settings.font, settings.font_size, Some(wrap))
						.y(),
				),
				None => text.measure(value.content(), settings.font, settings.font_size, None),
			}
		}
	};
	*cached = Some((element.revision, available, size));
	size
}

/// The width `text` wraps to, in multiples of its font size, when it's `width` wide: `None` unless it wraps.
pub(crate) fn text_wrap(text: &crate::ui::Text, width: f32) -> Option<f32> {
	(text.settings().wrap && !text.editable).then(|| width / text.settings().font_size.max(1.0))
}

/// Resolves a box's `width` and `height` sizings against the space its parent offers.
pub(crate) fn box_size(width: Sizing, height: Sizing, available: Size) -> Size {
	Size::new(width.calculate(available.x()), height.calculate(available.y()))
}

/// Returns `size` with the container's fit axes replaced by the matching axes of `fit`.
///
/// It offers a fit container's children the space the container itself was offered on its fit axes, so a relative
/// child never measures against the size it helps decide, and it writes the content extent into the fit axes.
fn on_fit_axes(container: &Container, size: Size, fit: Size) -> Size {
	let pick = |sizing: Sizing, fit: f32, size: f32| if sizing == Sizing::Fit { fit } else { size };
	Size::new(
		pick(container.width, fit.x(), size.x()),
		pick(container.height, fit.y(), size.y()),
	)
}

/// Whether `element` takes part in its parent's flow: everything but containers with an absolute position or depth.
fn in_flow(element: &IdedElement) -> bool {
	match &element.primitive {
		Primitives::Container(container) => {
			!matches!(container.depth, Depth::Absolute(_)) && !matches!(container.position, Position::Absolute { .. })
		}
		_ => true,
	}
}

/// The width and height sizings of an element that has them; text sizes to its content.
fn sizings(primitive: &Primitives) -> Option<(Sizing, Sizing)> {
	match primitive {
		Primitives::Container(container) => Some((container.width, container.height)),
		Primitives::Curve(curve) => Some((curve.path.width, curve.path.height)),
		Primitives::Path(path) => Some((path.path.width, path.path.height)),
		Primitives::Image(image) => Some((image.width, image.height)),
		Primitives::Text(_) => None,
	}
}

/// The space a flow child is offered: `offered`, with each axis the child fills replaced by that axis's share.
fn offer_share(child: &IdedElement, offered: Size, shares: [Option<f32>; 2]) -> Size {
	let Some((width, height)) = sizings(&child.primitive).filter(|_| shares != [None; 2]) else {
		return offered;
	};
	let pick = |sizing: Sizing, share: Option<f32>, offered: f32| match share {
		Some(share) if sizing == Sizing::Fill => share,
		_ => offered,
	};
	Size::new(pick(width, shares[0], offered.x()), pick(height, shares[1], offered.y()))
}

/// Returns `size` less the container's padding on both sides of each axis: the space its flow sees and offers its
/// children.
fn inside_padding(container: &Container, size: Size) -> Size {
	let padding = container.padding;
	Size::new(
		(size.x() - padding.x() * 2.0).max(0.0),
		(size.y() - padding.y() * 2.0).max(0.0),
	)
}

/// The `Measure` struct resolves element sizes for one layout pass.
///
/// Leaf and fixed-size elements hit the per-element cache, which outlives the pass. A fit container's size depends
/// on its descendants, which that cache can't key on, so it is measured once per pass and kept in [`Self::fits`].
struct Measure<'t, 'a> {
	tree: &'t retained_tree::RetainedTree,
	text: &'t mut TextSystem,
	measurements: &'t mut [Measurement],
	/// The space each fit container was offered and the size its content pass resolved, by tree index.
	fits: Vec<Option<(Size, Size)>, &'a bumpalo::Bump>,
}

impl Measure<'_, '_> {
	/// Resolves the size of the element at `index` against the space its parent offers.
	fn size(&mut self, index: usize, available: Size) -> Size {
		let element = &self.tree.elements[index];
		let container = match &element.primitive {
			Primitives::Container(container) if container.width == Sizing::Fit || container.height == Sizing::Fit => container,
			_ => return measure_element(element, available, self.text, &mut self.measurements[index]),
		};
		if let Some((offered, size)) = self.fits[index]
			&& offered == available
		{
			return size;
		}
		let size = self.content(index, container, available);
		self.fits[index] = Some((available, size));
		size
	}

	/// Runs the content pass of a fit container: places its flow children from a zero cursor and returns the
	/// container's size, with each fit axis set to the children's extent.
	fn content(&mut self, index: usize, container: &Container, available: Size) -> Size {
		// Fit axes resolve to 0 for the flow, so centering flows pack from the cursor instead of spreading over space
		// the container doesn't take yet.
		let resolve = |sizing: Sizing, available: f32| {
			if sizing == Sizing::Fit {
				0.0
			} else {
				sizing.calculate(available)
			}
		};
		let resolved = Size::new(
			resolve(container.width, available.x()),
			resolve(container.height, available.y()),
		);
		let offered = inside_padding(container, on_fit_axes(container, resolved, available));
		let inner = inside_padding(container, resolved);
		// The flow starts inside the padding, and an empty container is as large as its padding on both sides.
		let padding = container.padding;
		let shares = self.fill_shares(index, container, inner, offered);
		let (right, bottom) = self.extent(index, container, inner, offered, shares, padding.x(), padding.y());
		on_fit_axes(container, resolved, Size::new(right + padding.x(), bottom + padding.y()))
	}

	/// Runs the flow of the container at `index` from (`x`, `y`) over its flow children, each offered `offered` with
	/// its share of `shares`, and returns the far edges they reach, at least (`x`, `y`).
	#[allow(clippy::too_many_arguments)]
	fn extent(
		&mut self,
		index: usize,
		container: &Container,
		inner: Size,
		offered: Size,
		shares: [Option<f32>; 2],
		x: f32,
		y: f32,
	) -> (f32, f32) {
		let tree = self.tree;
		let mut cursor = Offset::new(x, y);
		let (mut right, mut bottom) = (x, y);
		for &child_index in &tree.children[index] {
			let child = &tree.elements[child_index];
			// Absolutely positioned and absolute-depth children don't take part in flow, so they don't size it.
			if !in_flow(child) {
				continue;
			}
			let child_size = self.size(child_index, offer_share(child, offered, shares));
			let output = container.flow.call(FlowInput::new(inner, cursor, child_size));
			// The extent ends at the far edge of each child, never at the next cursor, so trailing gaps don't count.
			right = right.max(output.child_offset().x() + child_size.x());
			bottom = bottom.max(output.child_offset().y() + child_size.y());
			cursor = output.next_cursor();
		}
		(right, bottom)
	}

	/// Returns each axis's share of free space for the fill children of the container at `index`, or `None` on an axis
	/// that isn't the flow's main axis or that no flow child fills. `inner` is the parent size the flow sees and
	/// `offered` the space it offers its children. See [Fill](/docs/reference/gui#fill).
	fn fill_shares(&mut self, index: usize, container: &Container, inner: Size, offered: Size) -> [Option<f32>; 2] {
		let tree = self.tree;
		let mut fills = [0u32; 2];
		for &child_index in &tree.children[index] {
			let child = &tree.elements[child_index];
			if let (true, Some((width, height))) = (in_flow(child), sizings(&child.primitive)) {
				fills[0] += u32::from(width == Sizing::Fill);
				fills[1] += u32::from(height == Sizing::Fill);
			}
		}
		if fills == [0, 0] {
			return [None; 2];
		}
		// The flow's main axes are the ones it moves its cursor along past a unit child.
		let past = container
			.flow
			.call(FlowInput::new(inner, Offset::new(0.0, 0.0), Size::new(1.0, 1.0)));
		let main = [past.next_cursor().x() != 0.0, past.next_cursor().y() != 0.0];
		let filled = [main[0] && fills[0] > 0, main[1] && fills[1] > 0];
		if filled == [false, false] {
			return [None; 2];
		}
		// A probe pass places the flow with every fill child at 0 on its filled axes; what's left is the free space.
		let probe = [filled[0].then_some(0.0), filled[1].then_some(0.0)];
		let (right, bottom) = self.extent(index, container, inner, offered, probe, 0.0, 0.0);
		let share =
			|filled: bool, inner: f32, extent: f32, count: u32| filled.then(|| (inner - extent).max(0.0) / count as f32);
		[
			share(filled[0], inner.x(), right, fills[0]),
			share(filled[1], inner.y(), bottom, fills[1]),
		]
	}

	/// Returns the space the container at `index`, laid out at `size`, offers its flow children, inside its padding.
	fn offered_to_children(&self, index: usize, container: &Container, size: Size) -> Size {
		let space = self.fits[index].map_or(size, |(offered, _)| on_fit_axes(container, size, offered));
		inside_padding(container, space)
	}
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
	let mut measure = Measure {
		tree,
		text: text_system,
		measurements,
		fits: Vec::with_capacity_in(tree.elements.len(), frame_allocator),
	};
	measure.fits.resize(tree.elements.len(), None);

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
		measure: &mut Measure,
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
		slots[index] = output.len();
		output.push(LayoutElement {
			id: element.id,
			index,
			position,
			size,
			// The visual pass resolves the display size, so a display edit never needs placement replayed.
			display_size: size,
			hit_testable: element.primitive.hit_testable(),
		});
		let Primitives::Container(container) = &element.primitive else {
			return;
		};
		let origin: Offset = Into::<Location>::into(position).into();
		// Flow children start inside the padding and see the box within it; absolute positions use the whole box.
		let mut cursor = Offset::new(origin.x() + container.padding.x(), origin.y() + container.padding.y());
		let inner = inside_padding(container, size);
		let offered = measure.offered_to_children(index, container, size);
		let shares = measure.fill_shares(index, container, inner, offered);
		// Absolute-depth layers start from the viewport after ordinary flow children.
		for reset_layer in [false, true] {
			for &child_index in &tree.children[index] {
				let child = &tree.elements[child_index];
				let child_container = match &child.primitive {
					Primitives::Container(value) => Some(value),
					_ => None,
				};
				let reset = child_container.is_some_and(|value| matches!(value.depth, Depth::Absolute(_)));
				if reset != reset_layer {
					continue;
				}
				let anchored = matches!(child_container.map(|value| value.position), Some(Position::Absolute { .. }));
				let available = match (reset, anchored) {
					(true, _) => root_size,
					(false, true) => offered,
					(false, false) => offer_share(child, offered, shares),
				};
				let child_size = measure.size(child_index, available);
				let flow_output = match child_container.map(|value| value.position) {
					// Absolute positions are anchored to the parent's box.
					Some(Position::Absolute { x, y }) => FlowOutput::new(
						Offset::new(
							origin.x() + x.offset(size.x(), child_size.x()),
							origin.y() + y.offset(size.y(), child_size.y()),
						),
						cursor,
					),
					_ if reset => FlowOutput::new(Offset::new(0.0, 0.0), cursor),
					_ => container.flow.call(FlowInput::new(inner, cursor, child_size)),
				};
				place(
					tree,
					child_index,
					child_size,
					root_size,
					flow_output.child_offset(),
					anchored,
					measure,
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
		layers: &mut Layers<'_>,
		output: &mut [LayoutElement],
	) {
		let position = &mut output[slots[index]].position;
		*position = Location3::new(position.x(), position.y(), *next);
		*next += 1;
		if !matches!(tree.elements[index].primitive, Primitives::Container(_)) {
			return;
		}
		// Every level sorts its own tail of the shared scratch list.
		let start = siblings.len();
		for &child in &tree.children[index] {
			match &tree.elements[child].primitive {
				Primitives::Container(container) => match container.depth {
					Depth::Relative(depth) => siblings.push((depth, child)),
					Depth::Absolute(depth) => layers.push(depth, child),
				},
				_ => siblings.push((1, child)),
			}
		}
		// The sort is stable, so children at the same depth keep their declaration order. Siblings usually share one
		// depth, and skipping their sort also skips the scratch buffer a stable sort of a long list allocates.
		let level = &mut siblings[start..];
		if !level.is_sorted_by_key(|&(depth, _)| depth) {
			level.sort_by_key(|&(depth, _)| depth);
		}
		let end = siblings.len();
		for sibling in start..end {
			let child = siblings[sibling].1;
			rank(tree, child, next, slots, siblings, layers, output);
		}
		siblings.truncate(start);
	}

	/// The `Layers` struct queues the absolute-depth layers that wait to be ranked, lowest depth first and in the
	/// order they were found among equals.
	///
	/// It is a binary min-heap in the frame allocator, which `std::collections::BinaryHeap` can't use.
	struct Layers<'a> {
		/// Each layer's depth, the order it was found in, and its element.
		heap: Vec<(i32, usize, usize), &'a bumpalo::Bump>,
		found: usize,
	}

	impl Layers<'_> {
		fn push(&mut self, depth: i32, index: usize) {
			self.heap.push((depth, self.found, index));
			self.found += 1;
			let heap = &mut self.heap;
			let mut child = heap.len() - 1;
			while child > 0 {
				let parent = (child - 1) / 2;
				if heap[parent] <= heap[child] {
					break;
				}
				heap.swap(parent, child);
				child = parent;
			}
		}

		/// Removes the lowest layer and returns its element.
		fn pop(&mut self) -> Option<usize> {
			let heap = &mut self.heap;
			let last = heap.len().checked_sub(1)?;
			heap.swap(0, last);
			let (_, _, lowest) = heap.pop()?;
			let mut parent = 0;
			loop {
				let left = parent * 2 + 1;
				if left >= heap.len() {
					break;
				}
				let right = left + 1;
				let child = if right < heap.len() && heap[right] < heap[left] {
					right
				} else {
					left
				};
				if heap[parent] <= heap[child] {
					break;
				}
				heap.swap(parent, child);
				parent = child;
			}
			Some(lowest)
		}
	}

	// The viewport parents every root: each element declared directly in a mount, and each one whose parent was
	// removed without it. It offers the evaluation size and has no flow, so roots overlap from its corner.
	let mut roots = Vec::new_in(frame_allocator);
	roots.extend((0..tree.elements.len()).filter(|&index| tree.parent(index).is_none()));
	let mut slots = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	slots.resize(tree.elements.len(), 0);
	// Roots are placed apart from `place`'s child loop, which stays as small as the hot recursion needs.
	for &root in &roots {
		let element = &tree.elements[root];
		let size = measure.size(root, available_space);
		let (offset, anchored) = match &element.primitive {
			Primitives::Container(Container {
				position: Position::Absolute { x, y },
				..
			}) => (
				Offset::new(
					x.offset(available_space.x(), size.x()),
					y.offset(available_space.y(), size.y()),
				),
				true,
			),
			_ => (Offset::new(0.0, 0.0), false),
		};
		place(
			tree,
			root,
			size,
			available_space,
			offset,
			anchored,
			&mut measure,
			&mut slots,
			&mut elements,
		);
	}

	let mut next = 0;
	let mut siblings = Vec::new_in(frame_allocator);
	let mut layers = Layers {
		heap: Vec::new_in(frame_allocator),
		found: 0,
	};
	// Roots order among themselves like siblings: by relative depth, then in tree order.
	for &root in &roots {
		match &tree.elements[root].primitive {
			Primitives::Container(container) => match container.depth {
				Depth::Relative(depth) => siblings.push((depth, root)),
				Depth::Absolute(depth) => layers.push(depth, root),
			},
			_ => siblings.push((1, root)),
		}
	}
	if !siblings.is_sorted_by_key(|&(depth, _)| depth) {
		siblings.sort_by_key(|&(depth, _)| depth);
	}
	for sibling in 0..siblings.len() {
		let root = siblings[sibling].1;
		rank(tree, root, &mut next, &slots, &mut siblings, &mut layers, &mut elements);
	}
	// Absolute-depth layers leave their parent and paint above all ordinary content, lowest first and
	// in tree order among equals. A layer found inside another one is ranked after it, which keeps it
	// above the layer that opened it.
	while let Some(index) = layers.pop() {
		rank(tree, index, &mut next, &slots, &mut siblings, &mut layers, &mut elements);
	}
	elements
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
	Relative(i32),
	Absolute(i32),
}

/// The `Position` enum lets a container leave its parent's flow and sit at a fixed spot in the parent's box, such as a
/// badge in a card's corner or a heads-up display pinned to the window's bottom-right corner.
///
/// Set it with [`Properties::position`](crate::ui::Properties::position).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
	Flow,
	Absolute { x: Anchor, y: Anchor },
}

impl Position {
	pub fn flow() -> Self {
		Self::Flow
	}

	/// Places the container at an offset from its parent's top-left corner.
	pub fn absolute(x: impl Into<f64>, y: impl Into<f64>) -> Self {
		Self::anchored(Anchor::Start(x.into() as f32), Anchor::Start(y.into() as f32))
	}

	/// Places the container against its parent's box with one [`Anchor`] per axis, so it follows the parent's size.
	pub fn anchored(x: Anchor, y: Anchor) -> Self {
		Self::Absolute { x, y }
	}
}

/// The `Anchor` enum picks which part of the parent's box an absolutely positioned container measures from on one
/// axis, so it can stay against an edge or centered however the parent resizes.
///
/// Each variant pairs a point of the parent's box with the same point of the container's box and holds a distance in
/// layout units. Use it with [`Position::anchored`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
	/// From the parent's left or top edge to the container's; a positive distance moves right or down.
	Start(f32),
	/// From the parent's center to the container's; a positive distance moves right or down.
	Center(f32),
	/// From the parent's right or bottom edge to the container's; a positive distance moves left or up, into the parent.
	End(f32),
}

impl Anchor {
	/// Resolves this anchor to the container's offset from the parent's start edge on one axis.
	#[inline]
	pub(crate) fn offset(self, parent: f32, child: f32) -> f32 {
		let finite = |distance: f32| if distance.is_finite() { distance } else { 0.0 };
		match self {
			Self::Start(distance) => finite(distance),
			Self::Center(distance) => (parent - child) * 0.5 + finite(distance),
			Self::End(distance) => parent - child - finite(distance),
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

/// The `Sizing` enum says how one axis of an element's box is resolved during layout.
///
/// Pass it to [`Properties::width`](crate::ui::Properties::width) and
/// [`Properties::height`](crate::ui::Properties::height), or to a container's display size through
/// [`Properties::display_width`](crate::ui::Properties::display_width).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sizing {
	/// A share of the space the parent offers: the numerator over the denominator.
	Relative(u16, u16),
	/// A fixed number of layout units.
	Absolute(f32),
	/// The extent of a container's flow children, for a panel or menu that should hug its content.
	///
	/// Only a container's logical width and height accept it. To animate a fit container's resize, pin its display
	/// size and follow [`Context::logical_size_change`](crate::ui::Context::logical_size_change).
	Fit,
	/// An equal share of the space the parent flow's other children leave on its main axis, like a flexbox item that
	/// grows, such as a list between a header and a footer. Off the main axis, it counts as full.
	Fill,
}

impl Sizing {
	pub fn full() -> Self {
		Self::Relative(1, 1)
	}

	pub fn pixels(value: impl Into<f64>) -> Self {
		Self::Absolute(value.into() as f32)
	}

	/// Sizes a container's axis to its flow children; see [`Sizing::Fit`].
	pub fn fit() -> Self {
		Self::Fit
	}

	/// Sizes an axis to what the parent flow's other children leave; see [`Sizing::Fill`].
	pub fn fill() -> Self {
		Self::Fill
	}

	/// Resolves a relative or absolute sizing against `available` layout units.
	///
	/// Layout resolves [`Sizing::Fit`] from the children instead. Anywhere else, it counts as full. A parent offers a
	/// [`Sizing::Fill`] child its share as the available space, so fill resolves as full.
	pub fn calculate(&self, available: f32) -> f32 {
		match self {
			Sizing::Fill => available,
			Sizing::Fit => {
				debug_assert!(
					false,
					"Fit sizing was used where it isn't accepted. The most likely cause is setting `Sizing::fit` on an image, curve, path, or display size, which have no children to fit. Use `Sizing::full` or a pixel size there instead."
				);
				available
			}
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
		components::container::Container,
		element::Id,
		flow::{self, Location3, Size},
		layout::{Depth, Position, Sizing},
	};
	use super::LayoutElement;
	use super::engine::properties::detached_container;
	use crate::ui::{font::TextSystem, layout::IdedElement, primitive::Primitives};

	/// Supplies retained topology to the layout seam from declarative test fixtures.
	fn layout_elements<'a>(
		elements: Vec<IdedElement>,
		relations: &[(Id, Id)],
		size: Size,
		text: &mut TextSystem,
		allocator: &'a bumpalo::Bump,
	) -> Vec<LayoutElement, &'a bumpalo::Bump> {
		let mut tree = super::retained_tree::RetainedTree::new();
		tree.parents.resize(elements.len(), usize::MAX);
		tree.children.resize_with(elements.len(), Vec::new);
		let index = |id: Id| {
			elements
				.iter()
				.position(|element| element.id == id)
				.expect("a fixture element")
		};
		for &(parent, child) in relations {
			let (parent, child) = (index(parent), index(child));
			tree.parents[child] = parent;
			tree.children[parent].push(child);
		}
		tree.elements = elements;
		super::layout_elements(&tree, size, text, &mut Vec::new(), allocator)
	}

	fn make_elements(elements: impl IntoIterator<Item = Container>) -> Vec<IdedElement> {
		let mut counter = Id::MIN;

		elements
			.into_iter()
			.map(|e| {
				let id = counter;

				counter = counter.checked_add(1).expect("expected test value");

				IdedElement::new(id, id.get() as u32, 0, Primitives::Container(e))
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
			.map(|&(parent, child)| (elements[parent].id, elements[child].id))
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
	fn layout_half_children() {
		let half = || detached_container(|c| c.size(Sizing::Relative(1, 2)));
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
		let child = || detached_container(|c| c.size(Sizing::pixels(64)));
		let root = detached_container(|c| c.flow(flow::column));
		let elements = layout(
			[root, child(), child(), child(), child()],
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

	/// Returns the size of the first container in a `[root, fit, children...]` fixture laid out in a 400 by 300 viewport,
	/// with every child under the fit container.
	fn fit_size(fit: Container, children: impl IntoIterator<Item = Container>) -> Size {
		let children: std::vec::Vec<Container> = children.into_iter().collect();
		let relations: std::vec::Vec<(usize, usize)> = std::iter::once((0, 1))
			.chain((0..children.len()).map(|child| (1, child + 2)))
			.collect();
		let containers = [Container::default(), fit].into_iter().chain(children);
		layout(containers, &relations, Size::new(400, 300))[1].size
	}

	fn pixels(width: u32, height: u32) -> Container {
		detached_container(|c| c.width(Sizing::pixels(width)).height(Sizing::pixels(height)))
	}

	#[test]
	fn a_fill_child_takes_what_its_siblings_and_gaps_leave_on_the_flows_main_axis() {
		let root = detached_container(|c| c.flow(flow::column_with_gap(10)).padding(5.0, 5.0));
		let list = detached_container(|c| c.height(Sizing::fill()).width(Sizing::fill()));
		let elements = layout(
			[root, pixels(100, 40), list, pixels(100, 30)],
			&[(0, 1), (0, 2), (0, 3)],
			Size::new(400, 300),
		);
		// 290 inside the padding, less 40, 30, and two gaps of 10. Across, off the main axis, fill is full.
		assert_eq!(elements[2].size, Size::new(390, 200));
		assert_eq!(elements[2].position.y(), 55.0);
		assert_eq!(elements[3].position.y(), 265.0);
	}

	#[test]
	fn fill_children_share_the_free_space_equally_and_only_on_a_main_axis() {
		let row = detached_container(|c| c.flow(flow::row));
		let fill = || detached_container(|c| c.width(Sizing::fill()).height(Sizing::pixels(10)));
		let elements = layout(
			[row, fill(), pixels(100, 10), fill()],
			&[(0, 1), (0, 2), (0, 3)],
			Size::new(400, 300),
		);
		assert_eq!(elements[1].size.x(), 150.0);
		assert_eq!(elements[3].size.x(), 150.0);
		assert_eq!(elements[3].position.x(), 250.0);

		// A centering flow never moves its cursor, so it has no main axis and fill is full.
		let center = detached_container(|c| c.flow(flow::center));
		let elements = layout([center, fill(), pixels(100, 10)], &[(0, 1), (0, 2)], Size::new(400, 300));
		assert_eq!(elements[1].size.x(), 400.0);
	}

	#[test]
	fn a_fill_child_of_a_fit_axis_gets_nothing_on_it() {
		let fit = detached_container(|c| c.width(Sizing::fit()).height(Sizing::pixels(20)).flow(flow::row));
		let fill = detached_container(|c| c.width(Sizing::fill()).height(Sizing::pixels(10)));
		assert_eq!(fit_size(fit, [pixels(30, 10), fill]), Size::new(30, 20));
	}

	#[test]
	fn a_fit_row_is_as_wide_as_its_children_and_as_tall_as_the_tallest() {
		let fit = detached_container(|c| c.size(Sizing::fit()).flow(flow::row));
		assert_eq!(fit_size(fit, [pixels(20, 10), pixels(30, 25)]), Size::new(50, 25));
	}

	#[test]
	fn a_fit_container_leaves_out_the_gap_after_its_last_child() {
		let fit = detached_container(|c| c.size(Sizing::fit()).flow(flow::row_with_gap(8)));
		assert_eq!(fit_size(fit, [pixels(20, 10), pixels(20, 10)]), Size::new(48, 10));
	}

	#[test]
	fn a_centered_flow_packs_its_children_on_a_fit_axis() {
		let fit = detached_container(|c| c.size(Sizing::fit()).flow(flow::centered_row));
		assert_eq!(fit_size(fit, [pixels(20, 10), pixels(20, 40)]), Size::new(40, 40));
	}

	#[test]
	fn a_fit_axis_only_follows_the_children_on_that_axis() {
		let fit = detached_container(|c| c.width(Sizing::fit()).height(Sizing::Relative(1, 2)).flow(flow::column));
		assert_eq!(fit_size(fit, [pixels(20, 10), pixels(35, 10)]), Size::new(35, 150));
	}

	#[test]
	fn absolutely_placed_children_dont_size_a_fit_container() {
		let fit = detached_container(|c| c.size(Sizing::fit()).flow(flow::row));
		let anchored = detached_container(|c| c.size(Sizing::pixels(90)).absolute_position(100, 100));
		let layer = detached_container(|c| c.size(Sizing::pixels(90)).depth(Depth::absolute(1)));
		assert_eq!(fit_size(fit, [pixels(20, 10), anchored, layer]), Size::new(20, 10));
	}

	#[test]
	fn an_empty_fit_container_measures_zero() {
		assert_eq!(fit_size(detached_container(|c| c.size(Sizing::fit())), []), Size::new(0, 0));
	}

	#[test]
	fn a_relative_child_on_a_fit_axis_is_a_share_of_the_space_the_fit_container_was_offered() {
		let fit = detached_container(|c| c.size(Sizing::fit()));
		let half = detached_container(|c| c.size(Sizing::Relative(1, 2)));
		let elements = layout([Container::default(), fit, half], &[(0, 1), (1, 2)], Size::new(400, 300));
		assert_eq!(elements[1].size, Size::new(200, 150));
		// Placement offers the child the same space, so it keeps the size the content pass measured.
		assert_eq!(elements[2].size, Size::new(200, 150));
	}

	#[test]
	fn nested_fit_containers_hug_their_innermost_children() {
		let outer = detached_container(|c| c.size(Sizing::fit()).flow(flow::column));
		let inner = || detached_container(|c| c.size(Sizing::fit()).flow(flow::row));
		let elements = layout(
			[
				Container::default(),
				outer,
				inner(),
				pixels(10, 5),
				pixels(15, 5),
				inner(),
				pixels(40, 8),
			],
			&[(0, 1), (1, 2), (2, 3), (2, 4), (1, 5), (5, 6)],
			Size::new(400, 300),
		);
		assert_eq!(elements[2].size, Size::new(25, 5));
		assert_eq!(elements[5].size, Size::new(40, 8));
		assert_eq!(elements[1].size, Size::new(40, 13));
		assert_eq!(elements[5].position, Location3::new(0, 5, elements[5].position.z()));
	}

	#[test]
	fn a_fit_container_counts_its_padding_on_both_sides() {
		let fit = detached_container(|c| c.size(Sizing::fit()).padding(6.0, 4.0).flow(flow::row_with_gap(8)));
		assert_eq!(fit_size(fit, [pixels(20, 10), pixels(20, 10)]), Size::new(60, 18));
		let empty = detached_container(|c| c.size(Sizing::fit()).padding(6.0, 4.0));
		assert_eq!(fit_size(empty, []), Size::new(12, 8));
	}

	#[test]
	fn padding_moves_flow_children_in_and_shrinks_their_space_but_not_an_absolute_childs() {
		let padded = detached_container(|c| c.size(Sizing::pixels(100)).padding(10.0, 20.0).flow(flow::center));
		let full = Container::default();
		let centered = pixels(20, 20);
		let anchored = detached_container(|c| c.size(Sizing::pixels(10)).absolute_position(0, 0));
		let elements = layout(
			[Container::default(), padded, full, centered, anchored],
			&[(0, 1), (1, 2), (1, 3), (1, 4)],
			Size::new(400, 300),
		);
		assert_eq!(
			elements[2].size,
			Size::new(80, 60),
			"a full child fills the box inside the padding"
		);
		assert_eq!(elements[2].position, Location3::new(10, 20, elements[2].position.z()));
		// A centering flow centers in the box inside the padding, which shares the box's center.
		assert_eq!(elements[3].position, Location3::new(40, 40, elements[3].position.z()));
		assert_eq!(elements[4].position, Location3::new(0, 0, elements[4].position.z()));
	}

	/// Returns each container's depth, in the order the containers were given to `layout`.
	fn depths(containers: impl IntoIterator<Item = Container>, relations: &[(usize, usize)]) -> std::vec::Vec<u32> {
		let elements = make_elements(containers);
		let ids: std::vec::Vec<Id> = elements.iter().map(|element| element.id).collect();
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
		let raised = detached_container(|c| c.depth(Depth::relative(2)));
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
		let overlay = detached_container(|c| c.depth(Depth::relative(8)));
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
		let modal = detached_container(|c| c.depth(Depth::absolute(1)));
		let overlay = detached_container(|c| c.depth(Depth::relative(8)));
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
		let toast = detached_container(|c| c.depth(Depth::absolute(4)));
		let modal = detached_container(|c| c.depth(Depth::absolute(1)));
		let nested_modal = detached_container(|c| c.depth(Depth::absolute(1)));
		let popup = detached_container(|c| c.depth(Depth::absolute(1)));
		// root -> [toast -> popup, modal -> nested_modal]
		let depths = depths(
			[Container::default(), toast, modal, nested_modal, popup],
			&[(0, 1), (0, 2), (2, 3), (1, 4)],
		);

		// The modals come first, the nested one over its host, then the toast and the popup it opened.
		assert_eq!(depths, [0, 3, 1, 2, 4]);
	}

	#[test]
	fn layout_absolute_position_places_child_without_advancing_flow() {
		let frame_allocator = bumpalo::Bump::new();
		let root = detached_container(|c| c.flow(flow::row));
		let first = detached_container(|c| c.size(Sizing::pixels(20)));
		let positioned = detached_container(|c| c.size(Sizing::pixels(30)).position(Position::absolute(70, 12)));
		let second = detached_container(|c| c.size(Sizing::pixels(20)));

		let elements = make_elements([root, first, positioned, second]);

		let root = &elements[0];
		let first = &elements[1];
		let positioned = &elements[2];
		let second = &elements[3];

		let relations = [(root.id, first.id), (root.id, positioned.id), (root.id, second.id)];

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
		let root = detached_container(|c| c.flow(flow::row));
		let first = detached_container(|c| c.size(Sizing::pixels(20)));
		let dropdown = detached_container(|c| c.size(Sizing::pixels(30)).depth(Depth::absolute(1)).absolute_position(24, 32));
		let child = detached_container(|c| c.size(Sizing::pixels(10)));

		let elements = make_elements([root, first, dropdown, child]);

		let root = &elements[0];
		let first = &elements[1];
		let dropdown = &elements[2];
		let child = &elements[3];

		let relations = [(root.id, first.id), (root.id, dropdown.id), (dropdown.id, child.id)];

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
	fn layout_absolute_depth_does_not_advance_parent_flow_cursor() {
		let frame_allocator = bumpalo::Bump::new();
		let root = detached_container(|c| c.flow(flow::row_with_gap(10)));
		let first = detached_container(|c| c.size(Sizing::pixels(20)));
		let modal = detached_container(|c| c.size(Sizing::pixels(30)).depth(Depth::absolute(1)));
		let second = detached_container(|c| c.size(Sizing::pixels(20)));

		let elements = make_elements([root, first, modal, second]);

		let root = &elements[0];
		let first = &elements[1];
		let modal_id = elements[2].id;
		let modal = &elements[2];
		let second = &elements[3];

		let relations = [(root.id, first.id), (root.id, modal.id), (root.id, second.id)];

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
	fn layout_centered_column() {
		let frame_allocator = bumpalo::Bump::new();
		let root = detached_container(|c| c.flow(flow::centered_column));
		let a = detached_container(|c| c.width(Sizing::pixels(64)).height(Sizing::pixels(32)));
		let b = detached_container(|c| c.width(Sizing::pixels(20)).height(Sizing::pixels(16)));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id, a.id), (root.id, b.id)];

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
		let root = detached_container(|c| c.flow(flow::centered_row));
		let a = detached_container(|c| c.width(Sizing::pixels(20)).height(Sizing::pixels(10)));
		let b = detached_container(|c| c.width(Sizing::pixels(20)).height(Sizing::pixels(10)));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id, a.id), (root.id, b.id)];

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
		let root = detached_container(|c| c.flow(flow::center));
		let a = detached_container(|c| c.width(Sizing::pixels(20)).height(Sizing::pixels(10)));
		let b = detached_container(|c| c.width(Sizing::pixels(40)).height(Sizing::pixels(20)));

		let elements = make_elements([root, a, b]);

		let root = &elements[0];
		let a = &elements[1];
		let b = &elements[2];

		let relations = [(root.id, a.id), (root.id, b.id)];

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
