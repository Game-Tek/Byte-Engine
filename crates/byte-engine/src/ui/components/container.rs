use crate::ui::{
	layout::ConcreteElement,
	style::{ConcreteStyle, Styler},
};

use super::super::{
	element::{Element, ElementHandle, Id},
	flow::{self, FlowFunction},
	layout::{self, Sizing},
	primitive::{BasePrimitive, Primitive, Shapes},
	Component,
};

pub struct BaseContainer {
	settings: ContainerSettings,
	on_click: Box<dyn Fn()>,
	styler: Box<dyn Styler>,
}

impl BaseContainer {
	pub fn new(settings: ContainerSettings) -> Self {
		Self {
			settings,
			on_click: Box::new(|| {}),
			styler: Box::new(|_| ConcreteStyle::default()),
		}
	}

	pub fn on_click(self, callback: impl Fn() + 'static) -> Self {
		Self {
			on_click: Box::new(callback),
			..self
		}
	}

	pub fn styler(self, callback: impl Styler + 'static) -> Self {
		Self {
			styler: Box::new(callback),
			..self
		}
	}

	pub fn settings(&self) -> &ContainerSettings {
		&self.settings
	}
}

impl Element for BaseContainer {
	fn primitive(&self) -> BasePrimitive {
		BasePrimitive::new(Shapes::Box {
			half: (self.settings.width, self.settings.height),
			radius: 0f32,
		})
	}

	fn flow(&self) -> FlowFunction {
		self.settings.flow
	}
}

impl Into<ConcreteElement> for BaseContainer {
	fn into(self) -> ConcreteElement {
		ConcreteElement::new(self.settings.flow, self.primitive().shape)
			.on_click(Some(self.on_click))
			.styler(Some(self.styler))
	}
}

pub struct ContainerSettings {
	min_width: Option<Sizing>,
	min_height: Option<Sizing>,
	pub width: Sizing,
	pub height: Sizing,
	max_width: Option<Sizing>,
	max_height: Option<Sizing>,
	depth: i16,
	flow: FlowFunction,
}

impl ContainerSettings {
	pub fn size(self, sizing: Sizing) -> Self {
		Self {
			width: sizing,
			height: sizing,
			..self
		}
	}

	pub fn width(self, width: Sizing) -> Self {
		Self { width, ..self }
	}

	pub fn height(self, height: Sizing) -> Self {
		Self { height, ..self }
	}

	pub fn min_width(self, min_width: Sizing) -> Self {
		Self {
			min_width: Some(min_width),
			..self
		}
	}

	pub fn min_height(self, min_height: Sizing) -> Self {
		Self {
			min_height: Some(min_height),
			..self
		}
	}

	pub fn max_width(self, max_width: Sizing) -> Self {
		Self {
			max_width: Some(max_width),
			..self
		}
	}

	pub fn max_height(self, max_height: Sizing) -> Self {
		Self {
			max_height: Some(max_height),
			..self
		}
	}

	pub fn depth(self, depth: i16) -> Self {
		Self { depth, ..self }
	}

	pub fn flow(self, flow: FlowFunction) -> Self {
		Self { flow, ..self }
	}
}

impl Default for ContainerSettings {
	fn default() -> Self {
		Self {
			width: Sizing::full(),
			height: Sizing::full(),
			min_width: None,
			min_height: None,
			max_width: None,
			max_height: None,
			depth: 0,
			flow: flow::grid,
		}
	}
}
