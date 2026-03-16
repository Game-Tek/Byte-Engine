use utils::RGBA;

pub enum Color {
	Value(RGBA),
	Sample(String),
}

impl Into<Color> for RGBA {
	fn into(self) -> Color {
		Color::Value(self)
	}
}

pub trait Style {
	fn layers(&self) -> &[&dyn Layer];
}

pub enum MixModes {
	Add,
	Multiply,
	Overlay,
}

pub trait Layer {
	fn fill(&self) -> &Color;
	fn mix_mode(&self) -> MixModes;
}

pub struct ConcreteStyle {
	pub(crate) layers: Vec<ConcreteLayer>,
}

impl Default for ConcreteStyle {
	fn default() -> Self {
		Self {
			layers: vec![ConcreteLayer::default()], // Always has a default layer
		}
	}
}

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

impl Style for ConcreteStyle {
	fn layers(&self) -> &[&dyn Layer] {
		&[]
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

impl Into<ConcreteStyle> for ConcreteLayer {
	fn into(self) -> ConcreteStyle {
		ConcreteStyle { layers: vec![self] }
	}
}

pub struct StyleState {
	pub is_hovered: bool,
}

pub trait Styler = Fn(&StyleState) -> ConcreteStyle;
