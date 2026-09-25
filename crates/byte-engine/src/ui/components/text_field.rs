use crate::ui::{Transform, Visual, style::ConcreteStyle};

/// The `TextField` struct provides a single-line text field whose content remains
/// owned by the application.
///
/// The field stores the current render snapshot of the app-owned string so it
/// can participate in layout, rendering, and backward-delete edit routing.
pub struct TextField {
	pub(crate) content: String,
	pub(crate) settings: TextFieldSettings,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl TextField {
	pub(crate) fn new(content: String) -> Self {
		Self {
			content,
			settings: TextFieldSettings::default(),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
	}

	pub fn content(&self) -> &str {
		&self.content
	}

	pub fn settings(&self) -> &TextFieldSettings {
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

/// The `TextFieldSettings` struct shares text-field settings between layout
/// measurement and rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextFieldSettings {
	pub font_size: f32,
}

impl Default for TextFieldSettings {
	fn default() -> Self {
		Self { font_size: 16.0 }
	}
}
