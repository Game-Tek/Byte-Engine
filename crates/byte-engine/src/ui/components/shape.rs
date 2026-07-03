use crate::ui::{components::container::Container, primitive::Shapes, style::ConcreteStyle, Transform, Visual};

pub struct Shape {
	pub(crate) shape: Shapes,
	pub(crate) settings: Container,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Shape {
	pub fn new(settings: Container) -> Self {
		let visual = settings.visual;
		Self {
			shape: Shapes::Box {
				half: (settings.width, settings.height),
				radius: settings.corner_radius,
				exponent: settings.corner_exponent,
			},
			settings,
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual,
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

	pub fn opacity(mut self, opacity: f32) -> Self {
		self.visual.opacity = opacity;
		self
	}

	pub fn set_style(&mut self, style: impl Into<ConcreteStyle>) {
		self.style = style.into();
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn set_opacity(&mut self, opacity: f32) {
		self.visual.opacity = opacity;
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

	pub fn visual_ref(&self) -> &Visual {
		&self.visual
	}
}
