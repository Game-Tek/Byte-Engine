use std::{borrow::Cow, future::Future, pin::Pin, time::Duration};

use crate::ui::{
	Container, Text,
	components::{curve::Curve, image::Image, path::Path, shape::Shape, text_field::TextField},
	drag::DragCapture,
	element::Id,
	layout::{
		Geometry,
		engine::{
			EvaluationContext, EventFuture, KeyFuture, MountedComponentFuture, PointerState, RenderFuture, TextEditFuture,
		},
	},
	primitive::{Events, Key},
	timer::{WaitFuture, seconds as wait_seconds, wait},
};

pub type UiFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
pub type MountedUiFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Element-construction API available to async UI components.
pub trait Context<C: 'static = ()>: Sized {
	fn id(&self) -> Id;
	fn ctx(&self) -> &C;

	/// Declares a named slot under this context for an element, component, or mount.
	///
	/// A static name identifies a fixed part of a component. An owned `String`
	/// names an element created at runtime, such as one node of a graph, so the
	/// slot is keyed by the string's content rather than its address.
	fn element<'a>(&'a mut self, name: impl Into<Cow<'static, str>>) -> ElementSlot<'a, C>;

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

	fn path(&mut self, path: Path) -> EvaluationContext<C> {
		self.element("path").path(path)
	}

	fn render(&mut self) -> RenderFuture;

	fn geometry(&self) -> Option<Geometry>;

	fn pointer(&self) -> PointerState;

	/// Returns the engine's captured drag gesture, if a source is held.
	fn drag(&self) -> Option<DragCapture>;

	fn request_focus(&mut self);

	fn release_focus(&mut self);

	/// Removes everything declared under this context, including the component
	/// itself when called from one. See [`EvaluationContext::remove`].
	fn remove(&mut self) -> bool;

	fn wait(&mut self, duration: Duration) -> WaitFuture {
		wait(duration)
	}

	fn seconds(&mut self, seconds: u64) -> WaitFuture {
		wait_seconds(seconds)
	}
}

pub struct ElementSlot<'a, C: 'static = ()> {
	pub(crate) parent: &'a mut EvaluationContext<C>,
	pub(crate) name: Cow<'static, str>,
}

pub trait ElementContext<C: 'static = ()> {
	fn container(self, element: Container) -> EvaluationContext<C>;
	fn text(self, text: Text) -> EvaluationContext<C>;
	fn text_field(self, text_field: TextField) -> EvaluationContext<C>;
	fn shape(self, shape: Shape) -> EvaluationContext<C>;
	fn curve(self, curve: Curve) -> EvaluationContext<C>;
	fn image(self, image: Image) -> EvaluationContext<C>;
	fn path(self, path: Path) -> EvaluationContext<C>;
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
