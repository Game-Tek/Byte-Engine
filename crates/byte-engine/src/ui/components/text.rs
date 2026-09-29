/// The `Text` struct is the retained state of styled UI copy that participates in layout and rendering.
///
/// The engine owns every text element. Declare a label with [`crate::ui::ElementContext::text`], or a single-line
/// field whose content the application owns with [`crate::ui::ElementContext::text_field`]. Edit either one with
/// [`crate::ui::EvaluationContext::update_text`].
pub struct Text {
	pub(crate) content: String,
	pub(crate) settings: TextSettings,
	/// Set for a text field: the pointer can hit it and backward deletes are routed to it while it is focused.
	pub(crate) editable: bool,
}

impl Text {
	pub(crate) fn new(content: String, editable: bool) -> Self {
		Self {
			content,
			settings: TextSettings::default(),
			editable,
		}
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
