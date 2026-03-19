use crate::ui::{
	components::{
		container::{ContainerSettings, OnEventFunction},
		shape::Shape,
	},
	flow::{Offset, Size},
	primitive::{Primitives, Shapes},
	style::Styler,
	Container,
};

use super::{
	flow::FlowFunction,
	primitive::{BasePrimitive, Primitive},
};

pub trait Element {
	fn primitive(&self) -> BasePrimitive;
}

use std::num::NonZeroU32;

pub type Id = NonZeroU32;

pub trait ElementHandle {
	fn id(&self) -> Id;
}

pub struct ConcreteElement {
	pub(crate) primitive: Primitives,
}

impl ConcreteElement {
	pub fn container(container: Container) -> Self {
		let primitive = Primitives::Container(container);

		Self { primitive }
	}

	pub fn shape(shape: Shape) -> Self {
		let primitive = Primitives::Shape(shape);

		Self { primitive }
	}
}
