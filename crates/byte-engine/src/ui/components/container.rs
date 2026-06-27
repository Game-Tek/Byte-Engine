use super::super::{
	flow::{self, FlowFunction},
	layout::{Depth, Sizing},
};
use crate::ui::{
	flow::{FlowInput, FlowOutput},
	style::ConcreteStyle,
};

pub struct Container {
	min_width: Option<Sizing>,
	min_height: Option<Sizing>,
	pub width: Sizing,
	pub height: Sizing,
	pub corner_radius: f32,
	max_width: Option<Sizing>,
	max_height: Option<Sizing>,
	pub depth: Depth,
	pub flow: utils::InlineCopyFn<fn(FlowInput) -> FlowOutput>,
	pub(crate) style: ConcreteStyle,
}

impl Container {
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

	pub fn depth(self, depth: impl Into<Depth>) -> Self {
		Self {
			depth: depth.into(),
			..self
		}
	}

	pub fn flow(self, flow: impl FlowFunction + 'static) -> Self {
		Self {
			flow: utils::InlineCopyFn::<fn(FlowInput) -> FlowOutput>::new(flow),
			..self
		}
	}

	pub fn style(self, style: impl Into<ConcreteStyle>) -> Self {
		Self {
			style: style.into(),
			..self
		}
	}

	pub fn set_style(&mut self, style: impl Into<ConcreteStyle>) {
		self.style = style.into();
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}
}

impl Default for Container {
	fn default() -> Self {
		Self {
			width: Sizing::full(),
			height: Sizing::full(),
			corner_radius: 0.0,
			min_width: None,
			min_height: None,
			max_width: None,
			max_height: None,
			depth: Depth::default(),
			flow: utils::InlineCopyFn::<fn(FlowInput) -> FlowOutput>::new(flow::grid),
			style: ConcreteStyle::default(),
		}
	}
}
