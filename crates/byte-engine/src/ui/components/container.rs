use utils::{Box, RGBA};

use crate::ui::{
	element::ConcreteElement,
	flow::{Offset, Size},
	primitive::Events,
	style::{ConcreteLayer, ConcreteStyle, Styler, StylerFn},
};

use super::super::{
	element::{Element, ElementHandle, Id},
	flow::{self, FlowFunction},
	layout::{self, Sizing},
	primitive::{BasePrimitive, Primitive, Shapes},
	Component,
};

pub trait OnEvent = Fn(Events) + Copy;
pub type OnEventFunction = fn(Events);

pub struct Container {
	pub(crate) settings: ContainerSettings,
	pub(crate) on_event: Option<utils::InlineCopyFn<OnEventFunction>>,
	pub(crate) styler: Option<utils::Box<dyn Styler>>,
}

impl Container {
	pub fn new(settings: ContainerSettings) -> Self {
		Self {
			settings,
			on_event: None,
			styler: None,
		}
	}

	pub fn on_event<F: OnEvent + 'static>(mut self, on_event: F) -> Self {
		self.on_event = Some(utils::InlineCopyFn::<OnEventFunction>::new(on_event));
		self
	}

	pub fn styler<F: Styler + 'static>(mut self, styler: F) -> Self {
		self.styler = Some(utils::Box::new(styler));
		self
	}

	pub fn settings(&self) -> &ContainerSettings {
		&self.settings
	}
}

pub struct ContainerSettings {
	min_width: Option<Sizing>,
	min_height: Option<Sizing>,
	pub width: Sizing,
	pub height: Sizing,
	pub corner_radius: f32,
	max_width: Option<Sizing>,
	max_height: Option<Sizing>,
	depth: i16,
	pub flow: utils::InlineCopyFn<fn(Offset, Size) -> Offset>,
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

	pub fn corner_radius(self, corner_radius: f32) -> Self {
		Self { corner_radius, ..self }
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

	pub fn flow(self, flow: impl FlowFunction + Copy + 'static) -> Self {
		Self {
			flow: utils::InlineCopyFn::<fn(Offset, Size) -> Offset>::new(flow),
			..self
		}
	}
}

impl Default for ContainerSettings {
	fn default() -> Self {
		Self {
			width: Sizing::full(),
			height: Sizing::full(),
			corner_radius: 0.0,
			min_width: None,
			min_height: None,
			max_width: None,
			max_height: None,
			depth: 0,
			flow: utils::InlineCopyFn::<fn(Offset, Size) -> Offset>::new(flow::grid),
		}
	}
}
