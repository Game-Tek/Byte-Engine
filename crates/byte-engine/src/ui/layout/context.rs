use std::{future::Future, pin::Pin, time::Duration};

use crate::ui::{
	components::shape::Shape,
	element::Id,
	layout::engine::{EvaluationContext, EventFuture, KeyFuture, MountedComponentFuture, RenderFuture},
	primitive::{Events, Key},
	timer::{seconds as wait_seconds, wait, WaitFuture},
	Container, Text,
};

pub type UiFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
pub type MountedUiFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Element-construction API available to async UI components.
pub trait Context<C: 'static = ()>: Sized {
	fn id(&self) -> Id;
	fn ctx(&self) -> &C;

	fn element<'a>(&'a mut self, name: &'static str) -> ElementSlot<'a, C>;

	fn text(&mut self, text: Text) -> EvaluationContext<C> {
		self.element("text").text(text)
	}

	fn shape(&mut self, shape: Shape) -> EvaluationContext<C> {
		self.element("shape").shape(shape)
	}

	fn render(&mut self) -> RenderFuture;

	fn request_focus(&mut self);

	fn release_focus(&mut self);

	fn wait(&mut self, duration: Duration) -> WaitFuture {
		wait(duration)
	}

	fn seconds(&mut self, seconds: u64) -> WaitFuture {
		wait_seconds(seconds)
	}
}

pub struct ElementSlot<'a, C: 'static = ()> {
	pub(crate) parent: &'a mut EvaluationContext<C>,
	pub(crate) name: &'static str,
}

pub trait ElementContext<C: 'static = ()> {
	fn container(self, element: Container) -> EvaluationContext<C>;
	fn text(self, text: Text) -> EvaluationContext<C>;
	fn shape(self, shape: Shape) -> EvaluationContext<C>;
	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static;

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static;
}

pub trait ContainerContext<C: 'static = ()>: Context<C> {
	fn on(&mut self, event: Events) -> EventFuture;
	fn on_key(&mut self, key: Key) -> KeyFuture;
}
