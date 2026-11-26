use std::borrow::Cow;

use smallvec::{SmallVec, smallvec};
use utils::RGBA;

/// The `LinearGradient` struct paints a layer with a two-stop gradient between `from` and `to`, such as a panel lit
/// from above. Pixels before `from` take `start`, and pixels past `to` take `end`.
///
/// Build one with [`Self::relative`] to place the points as fractions of the element's box, so the gradient follows
/// the element as it resizes, or with [`Self::new`] to place them in the element's own units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGradient {
	pub from: [f32; 2],
	pub to: [f32; 2],
	pub start: RGBA,
	pub end: RGBA,
	/// Whether `from` and `to` are fractions of the element's box instead of its own units.
	pub relative: bool,
}

impl LinearGradient {
	/// A gradient between two points in the element's own units from its origin: path units for a path with a view
	/// box, layout units otherwise.
	pub fn new(from: impl Into<[f32; 2]>, to: impl Into<[f32; 2]>, start: RGBA, end: RGBA) -> Self {
		Self {
			from: from.into(),
			to: to.into(),
			start,
			end,
			relative: false,
		}
	}

	/// A gradient between two points given as fractions of the element's box from its origin: `[0, 0]` is the top left
	/// and `[1, 1]` the bottom right. The box is a container's display box, a curve's own size, and a path's view box, or
	/// its own size without one.
	pub fn relative(from: impl Into<[f32; 2]>, to: impl Into<[f32; 2]>, start: RGBA, end: RGBA) -> Self {
		Self {
			relative: true,
			..Self::new(from, to, start, end)
		}
	}
}

#[derive(Clone, PartialEq)]
pub enum Color {
	Value(RGBA),
	Gradient(LinearGradient),
	/// A UI material, which computes the color of every pixel with a shader of its own. See [`Material`].
	Material(Material),
}

impl From<Material> for Color {
	fn from(material: Material) -> Self {
		Color::Material(material)
	}
}

/// The `Material` struct paints a fill layer with a UI material: a `.bema` material whose `domain` is `UI`, such as the
/// grid behind a node graph, for a look a flat color or a gradient can't give.
///
/// A material draws with its own pipeline and draw call, so keep it to the few elements that need it. Build one with
/// [`Self::new`], set the values of its parameters with [`Self::set`], and pass it to
/// [`ConcreteLayer::color`]. Only a container's fill layer takes a material. The
/// [UI material reference](/docs/reference/bemx#ui-domain) lists what its graph can read.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
	/// The asset ID of the material, such as `byte_engine/editor/graph-grid.bema`.
	pub id: Cow<'static, str>,
	/// The values this element gives the material's parameters, by name. A parameter left out keeps the material's value.
	pub parameters: SmallVec<[(Cow<'static, str>, MaterialValue); 4]>,
}

impl Material {
	/// A material that paints with the asset `id`, a `.bema` material whose `domain` is `UI`, with its own parameter
	/// values. Next, give its parameters this element's values with [`Self::set`].
	pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
		Self {
			id: id.into(),
			parameters: SmallVec::new(),
		}
	}

	/// Gives the parameter `name` the value `value` for this element, replacing a value set before. The value must have
	/// the parameter's type, such as `[f32; 2]` for a `vec2f` parameter.
	pub fn set(mut self, name: impl Into<Cow<'static, str>>, value: impl Into<MaterialValue>) -> Self {
		let name = name.into();
		let value = value.into();
		match self.parameters.iter_mut().find(|(set, _)| *set == name) {
			Some((_, set)) => *set = value,
			None => self.parameters.push((name, value)),
		}
		self
	}
}

/// The `MaterialValue` struct is the value a [`Material`] gives one of its parameters: a `float`, `vec2f`, `vec3f`, or
/// `vec4f`, by how many of its components count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialValue {
	pub components: [f32; 4],
	/// How many of `components` the value holds, from 1 for a `float` to 4 for a `vec4f`.
	pub width: u8,
}

impl From<f32> for MaterialValue {
	fn from(value: f32) -> Self {
		Self {
			components: [value, 0.0, 0.0, 0.0],
			width: 1,
		}
	}
}

impl From<[f32; 2]> for MaterialValue {
	fn from([x, y]: [f32; 2]) -> Self {
		Self {
			components: [x, y, 0.0, 0.0],
			width: 2,
		}
	}
}

impl From<[f32; 3]> for MaterialValue {
	fn from([x, y, z]: [f32; 3]) -> Self {
		Self {
			components: [x, y, z, 0.0],
			width: 3,
		}
	}
}

impl From<[f32; 4]> for MaterialValue {
	fn from(components: [f32; 4]) -> Self {
		Self { components, width: 4 }
	}
}

impl From<RGBA> for MaterialValue {
	fn from(color: RGBA) -> Self {
		[color.r, color.g, color.b, color.a].into()
	}
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerKind {
	Fill,
	/// A line along the element's edge, with a width for each side: top, right, bottom, left. Containers stroke
	/// inward and curves outward, with their largest width.
	Stroke {
		widths: [f32; 4],
	},
	/// A blurred copy of the element's shape painted in the layer's color. See [`Shadow`].
	Shadow(Shadow),
}

impl LayerKind {
	/// A stroke's widths, top, right, bottom, left, each finite and at least 0, or `None` for a fill, a shadow, or a
	/// stroke whose widths are all 0, which draws nothing.
	pub(crate) fn stroke_widths(&self) -> Option<[f32; 4]> {
		let LayerKind::Stroke { widths } = self else {
			return None;
		};
		let widths = widths.map(|width| if width.is_finite() { width.max(0.0) } else { 0.0 });
		widths.iter().any(|width| *width > 0.0).then_some(widths)
	}

	/// The kind as an element under an inherited `scale`, horizontal and vertical, draws it: a stroke's top and bottom
	/// widths follow the vertical scale and its sides the horizontal one, and a shadow scales as [`Shadow::scaled`].
	pub(crate) fn scaled(self, scale: [f32; 2]) -> Self {
		match self {
			LayerKind::Fill => self,
			LayerKind::Stroke {
				widths: [top, right, bottom, left],
			} => LayerKind::Stroke {
				widths: [top * scale[1], right * scale[0], bottom * scale[1], left * scale[0]],
			},
			LayerKind::Shadow(shadow) => LayerKind::Shadow(shadow.scaled(scale)),
		}
	}
}

/// The factor a length with no axis, such as a corner radius or a blur's sigma, follows under an inherited `scale`:
/// the smaller of the two, as a curve's stroke and a glyph's size do.
pub(crate) fn uniform_scale(scale: [f32; 2]) -> f32 {
	scale[0].min(scale[1])
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

	/// The shadow as an element under an inherited `scale` draws it: the offset follows each axis's scale, and the
	/// sigma and spread follow [`uniform_scale`].
	pub(crate) fn scaled(self, scale: [f32; 2]) -> Self {
		let uniform = uniform_scale(scale);
		Self {
			offset: [self.offset[0] * scale[0], self.offset[1] * scale[1]],
			sigma: self.sigma * uniform,
			spread: self.spread * uniform,
			inset: self.inset,
		}
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

	/// The feather as an element under an inherited `scale` draws it: top and bottom follow the vertical scale, and the
	/// sides the horizontal one.
	pub(crate) fn scaled(self, scale: [f32; 2]) -> Self {
		Self {
			top: self.top * scale[1],
			right: self.right * scale[0],
			bottom: self.bottom * scale[1],
			left: self.left * scale[0],
		}
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

	/// Turns this layer into a stroke `width` wide on every side.
	pub fn stroke(self, width: f32) -> Self {
		self.stroke_sides([width; 4])
	}

	/// Turns this layer into a stroke with a width for each side, top, right, bottom, then left, such as a rule under
	/// a header with only a bottom width. A non-finite or negative width counts as 0.
	pub fn stroke_sides(mut self, widths: [f32; 4]) -> Self {
		self.kind = LayerKind::Stroke { widths };
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
}

impl Default for ConcreteLayer {
	fn default() -> Self {
		Self::new()
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
	fn shadow_sanitizes_and_measures_its_outset() {
		let shadow = Shadow::new([f32::NAN, -4.0], -1.0).spread(f32::INFINITY);
		assert_eq!(shadow.offset, [0.0, -4.0]);
		assert_eq!(shadow.sigma, 0.0);
		assert_eq!(shadow.spread, 0.0);
		assert_eq!(Shadow::new([2.0, -4.0], 3.0).spread(1.0).outset(), 4.0 + 1.0 + 9.0);
		assert_eq!(Shadow::new([2.0, -4.0], 3.0).inset().outset(), 0.0);
		let layer = ConcreteLayer::default().drop_shadow([0.0, 2.0], 4.0);
		assert_eq!(layer.kind, LayerKind::Shadow(Shadow::new([0.0, 2.0], 4.0)));
	}

	#[test]
	fn backdrop_blur_radius_sanitizes_invalid_values() {
		assert_eq!(ConcreteLayer::default().backdrop_blur(-1.0).backdrop_blur_radius, 0.0);
		assert_eq!(ConcreteLayer::default().backdrop_blur(f32::NAN).backdrop_blur_radius, 0.0);
		assert_eq!(
			ConcreteLayer::default().backdrop_blur(f32::INFINITY).backdrop_blur_radius,
			0.0
		);
		assert_eq!(ConcreteLayer::default().backdrop_blur(128.0).backdrop_blur_radius, 64.0);
	}
}
