use std::{future::Future, pin::Pin};

use crate::ui::{
	components::shape::Shape,
	element::Id,
	layout::engine::{EvaluationContext, EventFuture, RenderFuture},
	primitive::Events,
	Container, Text,
};

pub type UiFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

/// Element-construction API available to async UI components.
pub trait Context: Sized {
	fn id(&self) -> Id;

	fn element<'a>(&'a mut self, name: &'static str) -> ElementSlot<'a>;

	fn text(&mut self, text: Text) -> EvaluationContext {
		self.element("text").text(text)
	}

	fn shape(&mut self, shape: Shape) -> EvaluationContext {
		self.element("shape").shape(shape)
	}

	fn render(&mut self) -> RenderFuture;
}

pub struct ElementSlot<'a> {
	pub(crate) parent: &'a mut EvaluationContext,
	pub(crate) name: &'static str,
}

pub trait ElementContext {
	fn container(self, element: Container) -> EvaluationContext;
	fn text(self, text: Text) -> EvaluationContext;
	fn shape(self, shape: Shape) -> EvaluationContext;
	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> UiFuture<'ctx> + 'static;
}

pub trait ContainerContext: Context {
	fn on(&mut self, event: Events) -> EventFuture;
}
