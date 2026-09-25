use smallvec::{SmallVec, smallvec};
use utils::RGBA;

/// A two-stop linear gradient. `from` and `to` are in the element's own units from its origin:
/// path units for a path with a view box, layout units otherwise. Pixels before `from` take
/// `start`, pixels past `to` take `end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGradient {
	pub from: [f32; 2],
	pub to: [f32; 2],
	pub start: RGBA,
	pub end: RGBA,
}

impl LinearGradient {
	pub fn new(from: impl Into<[f32; 2]>, to: impl Into<[f32; 2]>, start: RGBA, end: RGBA) -> Self {
		Self {
			from: from.into(),
			to: to.into(),
			start,
			end,
		}
	}
}

#[derive(Clone, PartialEq)]
pub enum Color {
	Value(RGBA),
	Sample(String),
	Gradient(LinearGradient),
}

impl From<LinearGradient> for Color {
	fn from(gradient: LinearGradient) -> Self {
		Color::Gradient(gradient)
	}
}

impl From<RGBA> for Color {
	fn from(val: RGBA) -> Self {
		Color::Value(val)
	}
}

#[derive(Clone, Copy)]
pub enum MixModes {
	Add,
	Multiply,
	Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerKind {
	Fill,
	Stroke {
		width: f32,
	},
	/// A blurred copy of the element's shape painted in the layer's color. See [`Shadow`].
	Shadow(Shadow),
}

/// Standard deviations a shadow's Gaussian tail reaches before it stops being drawn.
pub(crate) const SHADOW_EXTENT_SIGMAS: f32 = 3.0;

/// A drop or inset shadow of an element's rectangle, in layout units.
///
/// The shape is moved by `offset`, grown by `spread` (negative values shrink it), and blurred
/// with a Gaussian of standard deviation `sigma`, which is half of a CSS `box-shadow` blur radius.
/// An inset shadow darkens the inside of the element around a hole cut out of that shape.
/// Shadows never change layout or hit testing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
	pub offset: [f32; 2],
	pub sigma: f32,
	pub spread: f32,
	pub inset: bool,
}

impl Shadow {
	/// Creates an outer shadow with no spread.
	pub fn new(offset: [f32; 2], sigma: f32) -> Self {
		Self {
			offset: [sanitize_shadow_length(offset[0]), sanitize_shadow_length(offset[1])],
			sigma: sanitize_feather_width(sigma),
			spread: 0.0,
			inset: false,
		}
	}

	pub fn spread(mut self, spread: f32) -> Self {
		self.spread = sanitize_shadow_length(spread);
		self
	}

	pub fn inset(mut self) -> Self {
		self.inset = true;
		self
	}

	/// Layout distance an outer shadow reaches past the element's box on its farthest side. Zero for an inset shadow.
	pub(crate) fn outset(self) -> f32 {
		if self.inset {
			return 0.0;
		}
		let offset = self.offset[0].abs().max(self.offset[1].abs());
		offset + self.spread.max(0.0) + self.sigma * SHADOW_EXTENT_SIGMAS
	}
}

fn sanitize_shadow_length(length: f32) -> f32 {
	if length.is_finite() { length } else { 0.0 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeFeather {
	pub top: f32,
	pub right: f32,
	pub bottom: f32,
	pub left: f32,
}

impl EdgeFeather {
	pub const fn none() -> Self {
		Self {
			top: 0.0,
			right: 0.0,
			bottom: 0.0,
			left: 0.0,
		}
	}

	pub fn all(width: f32) -> Self {
		Self::edges(width, width, width, width)
	}

	pub fn vertical(width: f32) -> Self {
		Self::edges(width, 0.0, width, 0.0)
	}

	pub fn horizontal(width: f32) -> Self {
		Self::edges(0.0, width, 0.0, width)
	}

	pub fn edges(top: f32, right: f32, bottom: f32, left: f32) -> Self {
		Self {
			top: sanitize_feather_width(top),
			right: sanitize_feather_width(right),
			bottom: sanitize_feather_width(bottom),
			left: sanitize_feather_width(left),
		}
	}

	pub fn is_none(self) -> bool {
		self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
	}
}

impl Default for EdgeFeather {
	fn default() -> Self {
		Self::none()
	}
}

fn sanitize_feather_width(width: f32) -> f32 {
	if width.is_finite() { width.max(0.0) } else { 0.0 }
}

/// Maps a backdrop blur radius to the standard deviation of its Gaussian: `sigma = scale * sqrt(radius)`.
pub(crate) const BACKDROP_BLUR_SIGMA_SCALE: f32 = 1.689_394_6;

fn sanitize_backdrop_blur_radius(radius: f32) -> f32 {
	if radius.is_finite() { radius.clamp(0.0, 64.0) } else { 0.0 }
}

pub trait Layer {
	fn fill(&self) -> &Color;
	fn mix_mode(&self) -> MixModes;
	fn kind(&self) -> LayerKind;
	fn feather(&self) -> EdgeFeather;
	fn backdrop_blur_radius(&self) -> f32;
}

#[derive(Clone)]
pub struct ConcreteStyle {
	// A default fill needs no allocation; layered styles can still grow normally.
	pub(crate) layers: SmallVec<[ConcreteLayer; 1]>,
}

impl Default for ConcreteStyle {
	fn default() -> Self {
		Self {
			layers: smallvec![ConcreteLayer::default()],
		}
	}
}

impl ConcreteStyle {
	/// Creates a [`ConcreteStyle`] with no layers.
	///
	/// This style produces invisible elements. Use [`ConcreteStyle::default`] to
	/// create a style with one visible default layer.
	pub fn new() -> Self {
		Self { layers: SmallVec::new() }
	}

	pub fn layer(mut self, layer: impl Into<ConcreteLayer>) -> Self {
		self.layers.push(layer.into());
		self
	}

	pub fn from_layers(layers: impl IntoIterator<Item = ConcreteLayer>) -> Self {
		Self {
			layers: layers.into_iter().collect(),
		}
	}

	pub fn layers(&self) -> &[ConcreteLayer] {
		&self.layers
	}
}

#[derive(Clone, PartialEq)]
pub struct ConcreteLayer {
	pub(crate) color: Color,
	pub(crate) kind: LayerKind,
	pub(crate) feather: EdgeFeather,
	pub(crate) backdrop_blur_radius: f32,
}

impl ConcreteLayer {
	pub fn new() -> Self {
		Self {
			color: Color::Value(RGBA::white()),
			kind: LayerKind::Fill,
			feather: EdgeFeather::none(),
			backdrop_blur_radius: 0.0,
		}
	}

	pub fn color(mut self, color: Color) -> Self {
		self.color = color;
		self
	}

	pub fn fill(mut self) -> Self {
		self.kind = LayerKind::Fill;
		self
	}

	pub fn stroke(mut self, width: f32) -> Self {
		self.kind = LayerKind::Stroke { width };
		self
	}

	/// Turns this layer into a shadow of the element's shape, painted in the layer's color.
	///
	/// Layers paint in order, so add a drop shadow before the fill it sits under.
	pub fn shadow(mut self, shadow: Shadow) -> Self {
		self.kind = LayerKind::Shadow(shadow);
		self
	}

	/// Turns this layer into an outer shadow moved by `offset` and blurred with standard deviation `sigma`.
	pub fn drop_shadow(self, offset: [f32; 2], sigma: f32) -> Self {
		self.shadow(Shadow::new(offset, sigma))
	}

	/// Turns this layer into an inset shadow moved by `offset` and blurred with standard deviation `sigma`.
	pub fn inset_shadow(self, offset: [f32; 2], sigma: f32) -> Self {
		self.shadow(Shadow::new(offset, sigma).inset())
	}

	pub fn feather(mut self, feather: EdgeFeather) -> Self {
		self.feather = feather;
		self
	}

	pub fn feather_edges(self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
		self.feather(EdgeFeather::edges(top, right, bottom, left))
	}

	/// Blurs the pixels rendered behind this layer with an adaptive Gaussian filter.
	///
	/// `radius` preserves the UI renderer's legacy Gaussian-variance scale. It
	/// controls blur variance, not the Gaussian standard deviation or a literal
	/// pixel radius. The renderer derives standard deviation from `sqrt(radius)`,
	/// so use `max_radius * strength * strength` for approximately linear animated
	/// blur strength, where `strength` ranges from `0.0` to `1.0`.
	///
	/// Finite values are clamped to `0.0..=64.0`. Negative and non-finite values
	/// disable the blur. After configuring the layer, add it with
	/// [`ConcreteStyle::layer`].
	pub fn backdrop_blur(mut self, radius: f32) -> Self {
		self.backdrop_blur_radius = sanitize_backdrop_blur_radius(radius);
		self
	}

	/// Blurs the pixels rendered behind this layer with a Gaussian of standard deviation `sigma`,
	/// in layout units. This is the radius of [`ConcreteLayer::backdrop_blur`] expressed the way
	/// an SVG filter or a design tool states it.
	pub fn backdrop_blur_sigma(self, sigma: f32) -> Self {
		let radius = if sigma.is_finite() && sigma > 0.0 {
			(sigma / BACKDROP_BLUR_SIGMA_SCALE).powi(2)
		} else {
			0.0
		};
		self.backdrop_blur(radius)
	}

	/// Blurs the pixels rendered behind this layer using the legacy method name.
	///
	/// This method is an alias for [`ConcreteLayer::backdrop_blur`], including its
	/// Gaussian-variance radius scale and input sanitization. Use
	/// [`ConcreteLayer::backdrop_blur`] in new code to make the effect explicit.
	pub fn blur(self, radius: f32) -> Self {
		self.backdrop_blur(radius)
	}
}

impl Default for ConcreteLayer {
	fn default() -> Self {
		Self::new()
	}
}

impl Layer for ConcreteLayer {
	fn fill(&self) -> &Color {
		&self.color
	}

	fn mix_mode(&self) -> MixModes {
		MixModes::Overlay
	}

	fn kind(&self) -> LayerKind {
		self.kind
	}

	fn feather(&self) -> EdgeFeather {
		self.feather
	}

	fn backdrop_blur_radius(&self) -> f32 {
		self.backdrop_blur_radius
	}
}

impl AsRef<[ConcreteLayer]> for ConcreteStyle {
	fn as_ref(&self) -> &[ConcreteLayer] {
		&self.layers
	}
}

impl AsRef<[ConcreteLayer]> for ConcreteLayer {
	fn as_ref(&self) -> &[ConcreteLayer] {
		std::slice::from_ref(self)
	}
}

/// Lets one layer stand in for a whole style, such as in [`crate::ui::Properties::style`].
impl IntoIterator for ConcreteLayer {
	type Item = ConcreteLayer;
	type IntoIter = std::iter::Once<ConcreteLayer>;

	fn into_iter(self) -> Self::IntoIter {
		std::iter::once(self)
	}
}

impl IntoIterator for ConcreteStyle {
	type Item = ConcreteLayer;
	type IntoIter = smallvec::IntoIter<[ConcreteLayer; 1]>;

	fn into_iter(self) -> Self::IntoIter {
		self.layers.into_iter()
	}
}

impl From<ConcreteLayer> for ConcreteStyle {
	fn from(val: ConcreteLayer) -> Self {
		ConcreteStyle { layers: smallvec![val] }
	}
}

impl<const N: usize> From<[ConcreteLayer; N]> for ConcreteStyle {
	fn from(val: [ConcreteLayer; N]) -> Self {
		ConcreteStyle::from_layers(val)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stroke_layer_stores_width_and_color() {
		let color = RGBA::new(0.2, 0.3, 0.4, 1.0);
		let layer = ConcreteLayer::default().color(color.into()).stroke(2.5);

		assert_eq!(layer.kind(), LayerKind::Stroke { width: 2.5 });
		match Layer::fill(&layer) {
			Color::Value(actual) => assert_eq!(*actual, color),
			_ => panic!("expected value color"),
		}
	}

	#[test]
	fn shadow_sanitizes_and_measures_its_outset() {
		let shadow = Shadow::new([f32::NAN, -4.0], -1.0).spread(f32::INFINITY);
		assert_eq!(shadow.offset, [0.0, -4.0]);
		assert_eq!(shadow.sigma, 0.0);
		assert_eq!(shadow.spread, 0.0);
		assert_eq!(Shadow::new([2.0, -4.0], 3.0).spread(1.0).outset(), 4.0 + 1.0 + 9.0);
		assert_eq!(Shadow::new([2.0, -4.0], 3.0).inset().outset(), 0.0);
		let layer = ConcreteLayer::default().drop_shadow([0.0, 2.0], 4.0);
		assert_eq!(layer.kind(), LayerKind::Shadow(Shadow::new([0.0, 2.0], 4.0)));
	}

	#[test]
	fn edge_feather_sanitizes_invalid_widths() {
		assert_eq!(
			EdgeFeather::edges(-1.0, f32::NAN, f32::INFINITY, 4.0),
			EdgeFeather {
				top: 0.0,
				right: 0.0,
				bottom: 0.0,
				left: 4.0,
			}
		);
	}

	#[test]
	fn backdrop_blur_radius_sanitizes_invalid_values() {
		assert_eq!(ConcreteLayer::default().backdrop_blur(-1.0).backdrop_blur_radius(), 0.0);
		assert_eq!(ConcreteLayer::default().backdrop_blur(f32::NAN).backdrop_blur_radius(), 0.0);
		assert_eq!(
			ConcreteLayer::default().backdrop_blur(f32::INFINITY).backdrop_blur_radius(),
			0.0
		);
		assert_eq!(ConcreteLayer::default().backdrop_blur(128.0).backdrop_blur_radius(), 64.0);
	}
}
