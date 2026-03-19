use crate::ui::{
	components::container::ContainerSettings,
	primitive::Shapes,
	style::{Styler, StylerFn},
};

pub struct Shape {
	pub(crate) shape: Shapes,
	pub(crate) settings: ContainerSettings,
	pub(crate) styler: Option<utils::Box<dyn Styler>>,
}

impl Shape {
	pub fn new(settings: ContainerSettings) -> Self {
		Self {
			shape: Shapes::Box {
				half: (settings.width, settings.height),
				radius: 0f32,
			},
			settings,
			styler: None,
		}
	}

	pub fn styler<F: Styler + 'static>(mut self, styler: F) -> Self {
		self.styler = Some(utils::Box::new(styler));
		self
	}

	pub fn settings(&self) -> &ContainerSettings {
		&self.settings
	}
}
