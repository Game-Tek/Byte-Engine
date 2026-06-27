use crate::ui::{style::ConcreteStyle, Transform};

/// The `Text` struct carries styled UI copy that participates in layout and rendering.
pub struct Text {
	pub(crate) content: String,
	pub(crate) settings: TextSettings,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
}

impl Text {
	pub fn new(content: impl Into<String>) -> Self {
		Self {
			content: content.into(),
			settings: TextSettings::default(),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
		}
	}

	pub fn font_size(mut self, font_size: f32) -> Self {
		self.settings.font_size = font_size;
		self
	}

	pub fn style(mut self, style: impl Into<ConcreteStyle>) -> Self {
		self.style = style.into();
		self
	}

	pub fn transform(mut self, transform: impl Into<Transform>) -> Self {
		self.transform = transform.into();
		self
	}

	pub fn set_style(&mut self, style: impl Into<ConcreteStyle>) {
		self.style = style.into();
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn content(&self) -> &str {
		&self.content
	}

	pub fn settings(&self) -> &TextSettings {
		&self.settings
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}

	pub fn transform_ref(&self) -> &Transform {
		&self.transform
	}
}

/// The `TextSettings` struct captures the font choices that keep UI text consistent across layout and rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSettings {
	pub font_size: f32,
}

impl Default for TextSettings {
	fn default() -> Self {
		Self { font_size: 16.0 }
	}
}
