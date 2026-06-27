use crate::ui::{components::container::Container, primitive::Shapes, style::ConcreteStyle};

pub struct Shape {
	pub(crate) shape: Shapes,
	pub(crate) settings: Container,
	pub(crate) style: ConcreteStyle,
}

impl Shape {
	pub fn new(settings: Container) -> Self {
		Self {
			shape: Shapes::Box {
				half: (settings.width, settings.height),
				radius: settings.corner_radius,
			},
			settings,
			style: ConcreteStyle::default(),
		}
	}

	pub fn style(mut self, style: impl Into<ConcreteStyle>) -> Self {
		self.style = style.into();
		self
	}

	pub fn settings(&self) -> &Container {
		&self.settings
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}
}
