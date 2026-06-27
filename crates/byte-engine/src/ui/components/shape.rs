use crate::ui::{components::container::Container, primitive::Shapes, style::ConcreteStyle, Transform};

pub struct Shape {
	pub(crate) shape: Shapes,
	pub(crate) settings: Container,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
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
			transform: Transform::default(),
		}
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

	pub fn settings(&self) -> &Container {
		&self.settings
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}

	pub fn transform_ref(&self) -> &Transform {
		&self.transform
	}
}
