use crate::ui::{
	components::container::Container,
	primitive::Shapes,
	style::{Styler, StylerFn},
};

pub struct Shape {
	pub(crate) shape: Shapes,
	pub(crate) settings: Container,
	pub(crate) styler: Option<utils::Box<dyn Styler>>,
}

impl Shape {
	pub fn new(settings: Container) -> Self {
		Self {
			shape: Shapes::Box {
				half: (settings.width, settings.height),
				radius: settings.corner_radius,
			},
			settings,
			styler: None,
		}
	}

	pub fn styler<F: Styler + 'static>(mut self, styler: F) -> Self {
		self.styler = Some(utils::Box::new(styler));
		self
	}

	pub fn settings(&self) -> &Container {
		&self.settings
	}
}
