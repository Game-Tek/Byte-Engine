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

pub trait Layer {
	fn fill(&self) -> &Color;
	fn mix_mode(&self) -> MixModes;
	fn kind(&self) -> LayerKind;
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
}

impl ConcreteLayer {
	pub fn new() -> Self {
		Self {
			color: Color::Value(RGBA::white()),
			kind: LayerKind::Fill,
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
}
