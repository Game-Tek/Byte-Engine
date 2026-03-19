use crate::ui::style::Styler;

/// The `Text` struct carries styled UI copy that participates in layout and rendering.
pub struct Text {
	pub(crate) content: String,
	pub(crate) settings: TextSettings,
	pub(crate) styler: Option<utils::Box<dyn Styler>>,
}

impl Text {
	pub fn new(content: impl Into<String>) -> Self {
		Self {
			content: content.into(),
			settings: TextSettings::default(),
			styler: None,
		}
	}

	pub fn font_size(mut self, font_size: f32) -> Self {
		self.settings.font_size = font_size;
		self
	}

	pub fn styler<F: Styler + 'static>(mut self, styler: F) -> Self {
		self.styler = Some(utils::Box::new(styler));
		self
	}

	pub fn content(&self) -> &str {
		&self.content
	}

	pub fn settings(&self) -> &TextSettings {
		&self.settings
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
