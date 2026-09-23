use std::{
	future::Future,
	hash::{Hash, Hasher},
	num::NonZeroU64,
	pin::Pin,
	time::Duration,
};

use utils::hash::FxHasher;

use crate::ui::{
	Container, Text,
	components::{curve::Curve, image::Image, path::Path, shape::Shape, text_field::TextField},
	drag::DragCapture,
	element::Id,
	layout::{
		Geometry,
		engine::{
			EvaluationContext, EventFuture, KeyFuture, MountedComponentFuture, PointerState, Read, RenderFuture,
			TextEditFuture, With,
		},
	},
	primitive::{Events, Key},
	timer::WaitFuture,
};

pub type UiFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
pub type MountedUiFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// The `ElementKey` struct names one slot under a context: an element, a component, or a mount.
///
/// Pass it to [`Context::element`]. A static name identifies a fixed part of a component, such as `"title"`. An
/// owned `String` names a slot created at runtime, such as one node of a graph; the key is its content, not its
/// address. Siblings declared from one list use a `(name, index)` pair, such as `("row", index)`.
///
/// Keys under one parent must be distinct: declaring the same key twice under the same parent in one frame is an
/// error. A key is hashed when it is made, so it never allocates or keeps the name alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementKey(u64);

impl ElementKey {
	/// Hashes a name and, for one slot of a list, its index. A plain name and the same name with an index differ.
	fn new(name: &str, index: Option<usize>) -> Self {
		let mut hasher = FxHasher::default();
		(name, index).hash(&mut hasher);
		Self(hasher.finish())
	}
}

impl From<&str> for ElementKey {
	fn from(name: &str) -> Self {
		Self::new(name, None)
	}
}

impl From<String> for ElementKey {
	fn from(name: String) -> Self {
		Self::new(&name, None)
	}
}

impl From<(&str, usize)> for ElementKey {
	fn from((name, index): (&str, usize)) -> Self {
		Self::new(name, Some(index))
	}
}

/// Returns the path, and so the element id, of the slot `key` declared in the context at path `parent`.
///
/// This is a pure function of its inputs: the same key under the same parent always gives the same id, and no tree
/// lookup is needed to compute it. The root context's path is `0`, which no slot can have.
pub(crate) fn slot_path(parent: u64, key: ElementKey) -> NonZeroU64 {
	let mut hasher = FxHasher::default();
	hasher.write_u64(parent);
	hasher.write_u64(key.0);
	// Spread every bit so sibling and nested ids are unrelated; the multiply-rotate hash alone leaves weak high bits.
	let mut id = hasher.finish();
	id ^= id >> 33;
	id = id.wrapping_mul(0xff51_afd7_ed55_8ccd);
	id ^= id >> 33;
	NonZeroU64::new(id).unwrap_or(NonZeroU64::MIN)
}

/// Element-construction API available to async UI components.
///
/// Writes such as declaring elements, updates, and removals are sent to the engine and applied after the current
/// task poll, in the order they were made. Reads such as [`Self::geometry`] and [`Self::with`] are futures that
/// complete on their first poll.
pub trait Context<C: 'static = ()>: Sized {
	fn id(&self) -> Id;

	/// Reads the engine's application context, which the engine lends while it polls this component.
	///
	/// Await the result: `let value = ctx.with(|app: &App| app.value).await;`. The host reaches the same value
	/// through [`crate::ui::Engine::ctx`] and [`crate::ui::Engine::ctx_mut`].
	fn with<F, T>(&self, read: F) -> With<C, F>
	where
		F: FnOnce(&C) -> T;

	/// Declares a keyed slot under this context for an element, component, or mount.
	///
	/// The slot's id is computed from this context's path and `key`, so declaring the same key in a later frame
	/// finds the same element. See [`ElementKey`] for the keys you can pass and the rule for siblings.
	fn element<'a>(&'a mut self, key: impl Into<ElementKey>) -> ElementSlot<'a, C>;

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

	fn render(&mut self) -> RenderFuture<C>;

	/// Reads this element's bounds from the last layout, or `None` before it was laid out.
	fn geometry(&self) -> Read<C, Option<Geometry>>;

	/// Reads the pointer state the engine last synchronized.
	fn pointer(&self) -> Read<C, PointerState>;

	/// Reads the engine's captured drag gesture, if a source is held.
	fn drag(&self) -> Read<C, Option<DragCapture>>;

	fn request_focus(&mut self);

	fn release_focus(&mut self);

	/// Removes everything declared under this context, including the component
	/// itself when called from one. See [`EvaluationContext::remove`].
	fn remove(&mut self);

	/// Returns a future that completes after `duration`. See [`WaitFuture`].
	fn wait(&mut self, duration: Duration) -> WaitFuture<C> {
		WaitFuture::new(duration)
	}

	fn seconds(&mut self, seconds: u64) -> WaitFuture<C> {
		WaitFuture::new(Duration::from_secs(seconds))
	}
}

pub struct ElementSlot<'a, C: 'static = ()> {
	pub(crate) parent: &'a mut EvaluationContext<C>,
	pub(crate) key: ElementKey,
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
	fn on(&mut self, event: Events) -> EventFuture<C>;
	fn on_key(&mut self, key: Key) -> KeyFuture<C>;
	fn on_text_edit(&mut self) -> TextEditFuture<C>;
}
