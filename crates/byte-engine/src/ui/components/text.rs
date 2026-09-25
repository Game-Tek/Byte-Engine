use crate::ui::{Transform, Visual, style::ConcreteStyle};

/// The `Text` struct is the retained state of styled UI copy that participates in layout and rendering.
///
/// The engine owns every text element. Declare one with [`crate::ui::ElementContext::text`] and edit it with
/// [`crate::ui::EvaluationContext::update_text`].
pub struct Text {
	pub(crate) content: String,
	pub(crate) settings: TextSettings,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Text {
	pub(crate) fn new(content: String) -> Self {
		Self {
			content,
			settings: TextSettings::default(),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
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

	pub fn visual_ref(&self) -> &Visual {
		&self.visual
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
