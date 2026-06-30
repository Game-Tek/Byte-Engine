use utils::RGBA;

#[derive(Clone)]
pub enum Color {
	Value(RGBA),
	Sample(String),
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
	Stroke { width: f32 },
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
	if width.is_finite() {
		width.max(0.0)
	} else {
		0.0
	}
}

fn sanitize_backdrop_blur_radius(radius: f32) -> f32 {
	if radius.is_finite() {
		radius.clamp(0.0, 64.0)
	} else {
		0.0
	}
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
	pub(crate) layers: Vec<ConcreteLayer>,
}

impl Default for ConcreteStyle {
	fn default() -> Self {
		Self {
			layers: vec![ConcreteLayer::default()],
		}
	}
}

impl ConcreteStyle {
	/// Creates a new [`ConcreteStyle`] with no layers.
	///
	/// Keep in mind that [`ConcreteStyle::default`] will create a [`ConcreteStyle`] with a single default layer, while this will create an empty [`ConcreteStyle`] with no layers, which produces invisible elements.
	pub fn new() -> Self {
		Self { layers: Vec::new() }
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

#[derive(Clone)]
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

	pub fn feather(mut self, feather: EdgeFeather) -> Self {
		self.feather = feather;
		self
	}

	pub fn feather_edges(self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
		self.feather(EdgeFeather::edges(top, right, bottom, left))
	}

	pub fn backdrop_blur(mut self, radius: f32) -> Self {
		self.backdrop_blur_radius = sanitize_backdrop_blur_radius(radius);
		self
	}

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

impl From<ConcreteLayer> for ConcreteStyle {
	fn from(val: ConcreteLayer) -> Self {
		ConcreteStyle { layers: vec![val] }
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
	fn default_style_contains_one_fill_layer() {
		let style = ConcreteStyle::default();

		assert_eq!(style.layers().len(), 1);
		assert_eq!(style.layers()[0].kind(), LayerKind::Fill);
		assert_eq!(Layer::feather(&style.layers()[0]), EdgeFeather::none());
		assert_eq!(style.layers()[0].backdrop_blur_radius(), 0.0);
	}

	#[test]
	fn stroke_layer_stores_width_and_color() {
		let color = RGBA::new(0.2, 0.3, 0.4, 1.0);
		let layer = ConcreteLayer::default().color(color.into()).stroke(2.5);

		assert_eq!(layer.kind(), LayerKind::Stroke { width: 2.5 });
		match Layer::fill(&layer) {
			Color::Value(actual) => assert_eq!(*actual, color),
			Color::Sample(_) => panic!("expected value color"),
		}
	}

	#[test]
	fn layer_conversion_remains_single_layer_style() {
		let style: ConcreteStyle = ConcreteLayer::default().stroke(1.0).into();

		assert_eq!(style.layers().len(), 1);
		assert_eq!(style.layers()[0].kind(), LayerKind::Stroke { width: 1.0 });
	}

	#[test]
	fn edge_feather_constructors_store_per_edge_widths() {
		assert_eq!(
			EdgeFeather::edges(1.0, 2.0, 3.0, 4.0),
			EdgeFeather {
				top: 1.0,
				right: 2.0,
				bottom: 3.0,
				left: 4.0,
			}
		);
		assert_eq!(EdgeFeather::vertical(6.0), EdgeFeather::edges(6.0, 0.0, 6.0, 0.0));
		assert_eq!(EdgeFeather::horizontal(7.0), EdgeFeather::edges(0.0, 7.0, 0.0, 7.0));
		assert_eq!(EdgeFeather::all(8.0), EdgeFeather::edges(8.0, 8.0, 8.0, 8.0));
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
	fn concrete_layer_stores_feather() {
		let layer = ConcreteLayer::default().feather_edges(1.0, 2.0, 3.0, 4.0);

		assert_eq!(Layer::feather(&layer), EdgeFeather::edges(1.0, 2.0, 3.0, 4.0));
	}

	#[test]
	fn backdrop_blur_radius_is_stored() {
		let layer = ConcreteLayer::default().backdrop_blur(18.0);

		assert_eq!(layer.backdrop_blur_radius(), 18.0);
	}

	#[test]
	fn blur_alias_stores_backdrop_blur_radius() {
		let layer = ConcreteLayer::default().blur(12.0);

		assert_eq!(layer.backdrop_blur_radius(), 12.0);
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
