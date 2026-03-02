use super::{
	flow::FlowFunction,
	primitive::{BasePrimitive, Primitive},
};

pub trait Element {
	fn primitive(&self) -> BasePrimitive;
	fn flow(&self) -> FlowFunction;
}

use std::num::NonZeroU32;

pub type Id = NonZeroU32;

pub trait ElementHandle {
	fn id(&self) -> Id;
}
