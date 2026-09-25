//! Fluent element properties written straight into the engine's retained elements.
//!
//! Declaring or editing an element returns a future that completes on its first poll. During that poll the engine
//! lends its tree to the task, so the future creates the element in the tree's own storage and runs the component's
//! [`Setup`] function on it. Every [`Properties`] setter is a plain field write into the element
//! in its final place: no element value or property list is built on the component's side, moved, or sent. The heap
//! buffers of removed elements are kept in [`Spares`], so a new element reuses them instead of allocating.

use std::fmt::Write as _;

use smallvec::SmallVec;

use super::*;
use crate::ui::{
	components::{
		container::Sector,
		curve::{CurvePath, CurvePoint, CurveSegment},
		path::{FillRule, Path},
	},
	flow::{FlowFunction, FlowInput, FlowOutput},
	layout::{Position, Sizing},
	style::ConcreteLayer,
};

/// The `ElementKind` trait names the element kinds a component can declare, such as [`Container`] and [`Text`].
///
/// It lets [`Properties`] and the declaration futures reach the element of that kind inside the engine's tree. The
/// engine implements it for every built-in element; you only use it as a type parameter.
pub trait ElementKind: Sized {
	#[doc(hidden)]
	fn from_primitive(primitive: &mut Primitives) -> Option<&mut Self>;
}

/// Implements [`ElementKind`] and the setters every element has.
macro_rules! element_kinds {
	($($kind:ident),*) => {$(
		impl ElementKind for $kind {
			fn from_primitive(primitive: &mut Primitives) -> Option<&mut Self> {
				match primitive {
					Primitives::$kind(element) => Some(element),
					_ => None,
				}
			}
		}

		impl Properties<'_, $kind> {
			/// Replaces the element's style layers, keeping the element's layer storage.
			///
			/// Pass one [`ConcreteLayer`], an array of layers, or a [`crate::ui::ConcreteStyle`]. Layers paint in order.
			pub fn style(self, layers: impl IntoIterator<Item = ConcreteLayer>) -> Self {
				let layers = layers.into_iter();
				let style = &mut self.target.style.layers;
				style.clear();
				self.spares.fit_layers(style, layers.size_hint().0);
				style.extend(layers);
				self
			}

			/// Adds one layer on top of the element's current style. A new element starts with one white fill layer,
			/// so call [`Self::style`] first to start from an empty style.
			pub fn layer(self, layer: ConcreteLayer) -> Self {
				let style = &mut self.target.style.layers;
				self.spares.fit_layers(style, style.len() + 1);
				style.push(layer);
				self
			}

			pub fn transform(self, transform: impl Into<Transform>) -> Self {
				self.target.transform = transform.into();
				self
			}

			pub fn opacity(self, opacity: f32) -> Self {
				self.target.visual.opacity = opacity;
				self
			}
		}
	)*};
}

element_kinds!(Container, Shape, Curve, Path, Image, Text, TextField);

/// The `Setup` trait is a function that sets the properties of a `K` element, such as
/// `|frame| frame.width(240.into()).clip(false)`.
///
/// Declarations and edits take one. Every function from [`Properties`] to [`Properties`] is one, so a plain function
/// can hold a reusable look; see [`Properties`].
pub trait Setup<K>: for<'s> FnOnce(Properties<'s, K>) -> Properties<'s, K> {}

impl<K, F: for<'s> FnOnce(Properties<'s, K>) -> Properties<'s, K>> Setup<K> for F {}

/// The `Spares` struct keeps the heap buffers of removed elements so the next elements reuse them instead of
/// allocating.
///
/// A screen that closes and opens again, such as a menu or a leaderboard, then builds its elements without
/// allocating. The retained tree fills one as it removes elements, and the render lists keep their own for the
/// entries they drop.
#[derive(Default)]
pub(crate) struct Spares {
	/// Emptied text contents.
	strings: Vec<String>,
	/// Emptied style layer lists that outgrew their inline storage.
	layers: Vec<SmallVec<[ConcreteLayer; 1]>>,
	/// Emptied curve and path segment lists.
	segments: Vec<Vec<CurveSegment>>,
}

impl Spares {
	/// Returns an empty string, reusing spare storage when there is some.
	pub(crate) fn string(&mut self) -> String {
		self.strings.pop().unwrap_or_default()
	}

	/// Formats `content` into a string that reuses spare storage when there is some.
	pub(crate) fn format(&mut self, content: impl std::fmt::Display) -> String {
		let mut string = self.string();
		// Writing to a string fails only when `content`'s own formatting fails, which leaves what it wrote.
		let _ = write!(string, "{content}");
		string
	}

	/// Returns an empty segment list, reusing spare storage when there is some.
	pub(crate) fn segments(&mut self) -> Vec<CurveSegment> {
		self.segments.pop().unwrap_or_default()
	}

	/// Moves `layers` into spare storage when its inline storage cannot hold `count` layers, keeping the layers it
	/// holds, so filling it up to `count` does not allocate.
	pub(crate) fn fit_layers(&mut self, layers: &mut SmallVec<[ConcreteLayer; 1]>, count: usize) {
		if count > layers.inline_size()
			&& !layers.spilled()
			&& let Some(mut spare) = self.layers.pop()
		{
			spare.extend(layers.drain(..));
			*layers = spare;
		}
	}

	pub(crate) fn keep_string(&mut self, mut string: String) {
		if string.capacity() > 0 {
			string.clear();
			self.strings.push(string);
		}
	}

	pub(crate) fn keep_segments(&mut self, mut segments: Vec<CurveSegment>) {
		if segments.capacity() > 0 {
			segments.clear();
			self.segments.push(segments);
		}
	}

	/// Keeps a layer list only when it outgrew its inline storage; an inline list owns no heap buffer.
	pub(crate) fn keep_layers(&mut self, mut layers: SmallVec<[ConcreteLayer; 1]>) {
		if layers.spilled() {
			layers.clear();
			self.layers.push(layers);
		}
	}

	/// Takes the heap buffers of an element the tree is removing.
	pub(crate) fn recycle(&mut self, primitive: &mut Primitives) {
		self.keep_layers(std::mem::take(&mut primitive.style_mut().layers));
		match primitive {
			Primitives::Text(text) => self.keep_string(std::mem::take(&mut text.content)),
			Primitives::TextField(text_field) => self.keep_string(std::mem::take(&mut text_field.content)),
			Primitives::Curve(curve) => self.keep_segments(std::mem::take(&mut curve.path.segments)),
			Primitives::Path(path) => self.keep_segments(std::mem::take(&mut path.path.segments)),
			Primitives::Container(_) | Primitives::Shape(_) | Primitives::Image(_) => {}
		}
	}
}

/// Returns a future that declares the element `slot` names. Its first poll creates the element with `create` when the
/// tree does not hold it yet, then runs `setup` on it in place, and resolves to the element's context.
pub(super) fn declare<C: 'static, K: ElementKind>(
	slot: ElementSlot<'_, C>,
	create: impl FnOnce(u64, &mut Spares) -> Primitives,
	setup: impl Setup<K>,
) -> impl Future<Output = EvaluationContext<C>> {
	let EvaluationContext { parent, path, owner, .. } = *slot.parent;
	let id = crate::ui::layout::context::slot_path(path, slot.key);
	direct(move |poll: &mut UiPoll<C>| {
		if let Some((primitive, spares)) = poll.tree.add_element(parent, path, id, create) {
			let target = K::from_primitive(primitive).expect("a new element has its declared kind");
			let _ = setup(Properties { target, spares });
		}
		EvaluationContext::new(id, Some(id), id.get(), owner)
	})
}

/// Returns a future that edits element `id` with `setup` on its first poll. The tree compares the element before and
/// after the whole edit, so writing values it already has changes nothing.
pub(super) fn update<C: 'static, K: ElementKind>(id: Id, setup: impl Setup<K>) -> impl Future<Output = ()> {
	direct(move |poll: &mut UiPoll<C>| {
		let updated = poll.tree.update_element(id, |primitive, spares| {
			K::from_primitive(primitive)
				.map(|target| setup(Properties { target, spares }))
				.is_some()
		});
		match updated {
			Some(true) => {}
			Some(false) => log::error!(
				"A UI element update was skipped because the element is of another kind. The most likely cause is calling an `update_*` method that does not match the element the context declared."
			),
			// A component awaited from outside a removed element keeps running and may still edit its elements.
			None => log::debug!("A UI element update was skipped because the element was removed."),
		}
	})
}

/// The `Properties` struct lets a component set the properties of one element with chained calls.
///
/// A declaration's or an edit's setup function receives one, such as `|frame| frame.width(240.into()).clip(false)`.
/// It borrows the element inside the engine's tree, so each setter writes the value straight into its final place.
/// The type parameter is the element kind, so only the setters that kind supports are available. Because the proxy
/// knows nothing about the application context or whether it declares or edits, a plain function can hold a reusable
/// look and apply it in both places:
///
/// ```ignore
/// fn card(card: Properties<'_, Container>) -> Properties<'_, Container> {
///     card.corner_radius(8.0).clip(false)
/// }
///
/// let mut panel = ctx.element("panel").container(|panel| card(panel).width(240.into())).await;
/// panel.update_container(|panel| card(panel).opacity(0.5)).await;
/// ```
pub struct Properties<'a, K> {
	target: &'a mut K,
	/// Storage of removed elements, which setters take instead of allocating.
	spares: &'a mut Spares,
}

/// Setters for elements that fill a box. Each kind names the path from the element to its width and height.
macro_rules! box_setters {
	($($kind:ty => [$($path:ident)*];)*) => {$(
		impl Properties<'_, $kind> {
			pub fn width(self, width: Sizing) -> Self {
				self.target $(.$path)* .width = width;
				self
			}

			pub fn height(self, height: Sizing) -> Self {
				self.target $(.$path)* .height = height;
				self
			}

			/// Sets the width and the height to the same sizing.
			pub fn size(self, sizing: Sizing) -> Self {
				self.width(sizing).height(sizing)
			}
		}
	)*};
}

box_setters! {
	Container => [];
	Shape => [settings];
	Image => [];
	Curve => [path];
	Path => [path];
}

/// Setters for elements drawn as a rounded rectangle: containers and shapes.
macro_rules! corner_setters {
	($($kind:ty => [$($path:ident)*];)*) => {$(
		impl Properties<'_, $kind> {
			pub fn corner_radius(self, corner_radius: f32) -> Self {
				self.target $(.$path)* .corner_radius = corner_radius;
				self
			}

			pub fn corner_exponent(self, corner_exponent: f32) -> Self {
				self.target $(.$path)* .corner_exponent = corner_exponent;
				self
			}
		}
	)*};
}

corner_setters! {
	Container => [];
	Shape => [settings];
}

impl Properties<'_, Container> {
	/// Selects whether this surface participates in pointer hit testing.
	/// Disable this for decorative roots; children retain their own policy.
	pub fn hit_testable(self, enabled: bool) -> Self {
		self.target.hit_testable = enabled;
		self
	}

	/// Shapes this container as an annular sector, or as a rounded rectangle again with `None`. See [`Sector`].
	pub fn sector(self, sector: impl Into<Option<Sector>>) -> Self {
		self.target.sector = sector.into();
		self
	}

	pub fn min_width(self, min_width: Sizing) -> Self {
		self.target.min_width = Some(min_width);
		self
	}

	pub fn min_height(self, min_height: Sizing) -> Self {
		self.target.min_height = Some(min_height);
		self
	}

	pub fn max_width(self, max_width: Sizing) -> Self {
		self.target.max_width = Some(max_width);
		self
	}

	pub fn max_height(self, max_height: Sizing) -> Self {
		self.target.max_height = Some(max_height);
		self
	}

	pub fn depth(self, depth: impl Into<Depth>) -> Self {
		self.target.depth = depth.into();
		self
	}

	pub fn position(self, position: impl Into<Position>) -> Self {
		self.target.position = position.into();
		self
	}

	/// Places this container at an offset from its parent's top-left corner
	/// instead of in the parent's flow. A container with [`Depth::absolute`]
	/// is placed from the viewport's corner instead.
	pub fn absolute_position(self, x: impl Into<f64>, y: impl Into<f64>) -> Self {
		self.position(Position::absolute(x, y))
	}

	pub fn clip(self, enabled: bool) -> Self {
		self.target.clip = enabled;
		self
	}

	/// Lays out this container's children with `flow`, such as [`crate::ui::flow::row_with_gap`].
	pub fn flow(self, flow: impl FlowFunction + 'static) -> Self {
		self.target.flow = utils::InlineCopyFn::<fn(FlowInput) -> FlowOutput>::new(flow);
		self
	}
}

/// Setters for text elements: static labels and text fields.
macro_rules! text_setters {
	($($kind:ty),*) => {$(
		impl Properties<'_, $kind> {
			/// Replaces the text by formatting `content` into the element's own string storage, so a label that fits
			/// allocates nothing. Pass a `&str`, a number, or `format_args!` instead of a formatted `String`.
			pub fn content(self, content: impl std::fmt::Display) -> Self {
				self.target.content.clear();
				// Writing to a string fails only when `content`'s own formatting fails, which leaves what it wrote.
				let _ = write!(self.target.content, "{content}");
				self
			}

			pub fn font_size(self, font_size: f32) -> Self {
				self.target.settings.font_size = font_size;
				self
			}
		}
	)*};
}

text_setters!(Text, TextField);

/// Setters for elements drawn from curve segments: stroked curves and filled paths.
macro_rules! segment_setters {
	($($kind:ty),*) => {$(
		impl Properties<'_, $kind> {
			/// Replaces the segments and the size with those of `path`, reusing the element's segment storage.
			pub fn outline(self, path: CurvePath) -> Self {
				let target = &mut self.target.path;
				target.width = path.width;
				target.height = path.height;
				if target.segments.capacity() < path.segments.len() {
					target.segments = path.segments;
				} else {
					target.segments.clear();
					target.segments.extend(path.segments);
				}
				self.target.outline_changed();
				self
			}

			/// Removes every segment while keeping the segment storage, so re-routing allocates nothing.
			pub fn clear_segments(self) -> Self {
				self.target.path.segments.clear();
				self.target.outline_changed();
				self
			}

			/// Replaces the segments, reusing the element's segment storage.
			pub fn segments(self, segments: impl IntoIterator<Item = CurveSegment>) -> Self {
				let properties = self.clear_segments();
				properties.target.path.segments.extend(segments);
				properties
			}

			/// Appends one segment.
			pub fn segment(self, segment: CurveSegment) -> Self {
				self.target.path.segments.push(segment);
				self.target.outline_changed();
				self
			}

			pub fn line(self, from: impl Into<CurvePoint>, to: impl Into<CurvePoint>) -> Self {
				self.segment(CurveSegment::Line {
					from: from.into(),
					to: to.into(),
				})
			}

			pub fn quadratic(
				self,
				from: impl Into<CurvePoint>,
				control: impl Into<CurvePoint>,
				to: impl Into<CurvePoint>,
			) -> Self {
				self.segment(CurveSegment::Quadratic {
					from: from.into(),
					control: control.into(),
					to: to.into(),
				})
			}

			pub fn cubic(
				self,
				from: impl Into<CurvePoint>,
				control0: impl Into<CurvePoint>,
				control1: impl Into<CurvePoint>,
				to: impl Into<CurvePoint>,
			) -> Self {
				self.segment(CurveSegment::Cubic {
					from: from.into(),
					control0: control0.into(),
					control1: control1.into(),
					to: to.into(),
				})
			}
		}
	)*};
}

segment_setters!(Curve, Path);

impl Properties<'_, Curve> {
	/// Lets the pointer find this curve within `width` layout units of its stroke,
	/// so a wire can be clicked, hovered, and dropped on like a container.
	/// `None` makes the curve ignore the pointer again.
	///
	/// The width is measured before any inherited scale. Pass a width wider than
	/// the stroke for a comfortable target.
	pub fn hit_width(self, width: impl Into<Option<f32>>) -> Self {
		self.target.hit_width = width.into().filter(|width| width.is_finite() && *width > 0.0);
		self
	}
}

impl Properties<'_, Path> {
	/// Maps `[width, height]` path units onto the element's box. With `None`, points are layout units.
	pub fn view_box(self, view_box: impl Into<Option<[f32; 2]>>) -> Self {
		self.target.view_box = view_box.into();
		self
	}

	pub fn fill_rule(self, fill_rule: FillRule) -> Self {
		self.target.fill_rule = fill_rule;
		self
	}
}

impl Properties<'_, Image> {
	/// Replaces the RGBA pixels, copying them into the engine's storage. `pixels` must hold exactly
	/// `width * height * 4` bytes.
	#[track_caller]
	pub fn pixels(self, width: u32, height: u32, pixels: impl AsRef<[u8]>) -> Self {
		let pixels = pixels.as_ref();
		assert_rgba_len(width, height, pixels);
		self.target.set_rgba(width, height, pixels);
		self
	}
}

/// Panics at the caller when RGBA data does not match its dimensions.
#[track_caller]
pub(super) fn assert_rgba_len(width: u32, height: u32, pixels: &[u8]) {
	assert_eq!(
		pixels.len(),
		width as usize * height as usize * 4,
		"RGBA image data must contain exactly width * height * 4 bytes"
	);
}

/// Builds a container outside any engine from `setup`'s properties, for tests that lay out elements directly.
#[cfg(test)]
pub(in crate::ui::layout) fn detached_container(setup: impl Setup<Container>) -> Container {
	let mut container = Container::default();
	let _ = setup(Properties {
		target: &mut container,
		spares: &mut Spares::default(),
	});
	container
}
