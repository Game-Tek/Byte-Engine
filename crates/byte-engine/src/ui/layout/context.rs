use std::{future::Future, pin::Pin, time::Duration};

use crate::ui::{
	components::{curve::Curve, image::Image, shape::Shape, text_field::TextField},
	element::Id,
	layout::{
		engine::{
			EvaluationContext, EventFuture, KeyFuture, MountedComponentFuture, PointerState, RenderFuture, TextEditFuture,
		},
		Geometry,
	},
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

	fn text_field(&mut self, text_field: TextField) -> EvaluationContext<C> {
		self.element("text_field").text_field(text_field)
	}

	fn shape(&mut self, shape: Shape) -> EvaluationContext<C> {
		self.element("shape").shape(shape)
	}

	fn curve(&mut self, curve: Curve) -> EvaluationContext<C> {
		self.element("curve").curve(curve)
	}

	fn image(&mut self, image: Image) -> EvaluationContext<C> {
		self.element("image").image(image)
	}

	fn render(&mut self) -> RenderFuture;

	fn geometry(&self) -> Option<Geometry>;

	fn pointer(&self) -> PointerState;

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
	fn text_field(self, text_field: TextField) -> EvaluationContext<C>;
	fn shape(self, shape: Shape) -> EvaluationContext<C>;
	fn curve(self, curve: Curve) -> EvaluationContext<C>;
	fn image(self, image: Image) -> EvaluationContext<C>;
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
	fn on_text_edit(&mut self) -> TextEditFuture;
}
