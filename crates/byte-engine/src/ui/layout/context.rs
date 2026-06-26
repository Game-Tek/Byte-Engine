use std::{future::Future, pin::Pin, time::Duration};

use crate::ui::{
	components::shape::Shape,
	element::Id,
	layout::engine::{EvaluationContext, EventFuture, MountedComponentFuture, RenderFuture},
	primitive::Events,
	timer::{seconds as wait_seconds, wait, WaitFuture},
	Container, Text,
};

pub type UiFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
pub type MountedUiFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

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

	fn wait(&mut self, duration: Duration) -> WaitFuture {
		wait(duration)
	}

	fn seconds(&mut self, seconds: u64) -> WaitFuture {
		wait_seconds(seconds)
	}
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

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, T> + 'static;
}

pub trait ContainerContext: Context {
	fn on(&mut self, event: Events) -> EventFuture;
}
