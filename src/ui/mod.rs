use crate::core::{Entity, property::{SinkProperty, DerivedProperty}};

pub mod render_model;

pub trait Text {
}

pub struct TextComponent {
	text: SinkProperty<String>,
}

impl TextComponent {
	pub fn new(property: &mut DerivedProperty<usize, String>) -> Self {
		Self {
			text: SinkProperty::from_derived(property),
		}
	}
}

impl Text for TextComponent {
}

impl Entity for TextComponent {}