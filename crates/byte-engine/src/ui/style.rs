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

pub trait Layer {
	fn fill(&self) -> &Color;
	fn mix_mode(&self) -> MixModes;
}

#[derive(Clone)]
pub struct ConcreteStyle {
	pub(crate) layer: ConcreteLayer,
}

impl Default for ConcreteStyle {
	fn default() -> Self {
		Self {
			layer: ConcreteLayer::default(),
		}
	}
}

#[derive(Clone)]
pub struct ConcreteLayer {
	pub(crate) color: Color,
}

impl ConcreteLayer {
	pub fn new() -> Self {
		Self {
			color: Color::Value(RGBA::white()),
		}
	}

	pub fn color(mut self, color: Color) -> Self {
		self.color = color;
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
}

impl From<ConcreteLayer> for ConcreteStyle {
	fn from(val: ConcreteLayer) -> Self {
		ConcreteStyle { layer: val }
	}
}
