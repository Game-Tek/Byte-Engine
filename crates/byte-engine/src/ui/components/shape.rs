use crate::ui::{Transform, Visual, components::container::Container, primitive::Shapes, style::ConcreteStyle};

/// The `Shape` struct is the retained state of a painted box that has no children.
///
/// The engine owns every shape. Declare one with [`crate::ui::ElementContext::shape`] and edit it with
/// [`crate::ui::EvaluationContext::update_shape`].
pub struct Shape {
	pub(crate) settings: Container,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Shape {
	/// Creates a full-size box with the default container settings.
	pub(crate) fn new() -> Self {
		Self {
			settings: Container::default(),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
	}

	/// Returns the box drawn for this shape's settings.
	pub(crate) fn outline(&self) -> Shapes {
		Shapes::Box {
			half: (self.settings.width, self.settings.height),
			radius: self.settings.corner_radius,
			exponent: self.settings.corner_exponent,
		}
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
