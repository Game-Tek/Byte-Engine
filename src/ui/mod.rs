use crate::core::{orchestrator::{Component, SinkProperty, DerivedProperty}, Entity};

pub mod render_model;

pub trait Text: Component {
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
impl Component for TextComponent {
	// type Parameters<'a> = &'a str;
}