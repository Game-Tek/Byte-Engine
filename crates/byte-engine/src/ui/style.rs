pub enum Color {
	Value(f32),
	Sample(String),
}

pub trait Style {
	fn layers(&self) -> &[Box<dyn Layer>];
}

pub enum MixModes {
	Add,
	Multiply,
	Overlay,
}

pub trait Layer {
	fn fill(&self) -> &Color;
	fn mix_mode(&self) -> &MixModes;
}
