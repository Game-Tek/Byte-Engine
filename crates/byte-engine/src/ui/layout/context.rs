use std::{
	fmt::{Display, Write},
	hash::{Hash, Hasher},
	num::NonZeroU64,
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
			EvaluationContext, EventFuture, KeyFuture, MountedComponentFuture, PointerState, Properties, Read, RenderFuture,
			Setup, TextEditFuture, With,
		},
	},
	primitive::{Events, Key},
	timer::WaitFuture,
};

/// The `ElementKey` struct names one slot under a context: an element, a component, or a mount.
///
/// Pass it to [`Context::element`]. A static name identifies a fixed part of a component, such as `"title"`. A name
/// made at runtime, such as one node of a graph, is keyed by its content: pass `format_args!("node-{id}")`, which is
/// hashed as it is formatted, or an owned `String`, which gives the same key. Siblings declared from one list use a
/// `(name, index)` pair, such as `("row", index)`.
///
/// Keys under one parent must be distinct: declaring the same key twice under the same parent in one frame is an
/// error. A key is hashed when it is made, so it never allocates or keeps the name alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementKey(u64);

impl ElementKey {
	/// Hashes a name and, for one slot of a list, its index. A plain name and the same name with an index differ.
	fn new(name: &str, index: Option<usize>) -> Self {
		let mut hasher = KeyHasher::default();
		let _ = hasher.write_str(name);
		hasher.finish(index)
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

impl From<std::fmt::Arguments<'_>> for ElementKey {
	fn from(name: std::fmt::Arguments<'_>) -> Self {
		let mut hasher = KeyHasher::default();
		// Hashing never fails, so only a failing `Display` implementation in `name` could stop the write early.
		let _ = std::fmt::write(&mut hasher, name);
		hasher.finish(None)
	}
}

/// The `KeyHasher` struct hashes a key's name one byte at a time, so a name formatted in pieces hashes like the same
/// name in one string.
#[derive(Default)]
struct KeyHasher(FxHasher);

impl KeyHasher {
	fn finish(mut self, index: Option<usize>) -> ElementKey {
		// The terminator keeps a name from running into its index.
		self.0.write_u8(0xff);
		index.hash(&mut self.0);
		ElementKey(self.0.finish())
	}
}

impl Write for KeyHasher {
	fn write_str(&mut self, part: &str) -> std::fmt::Result {
		part.bytes().for_each(|byte| self.0.write_u8(byte));
		Ok(())
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
/// Writes such as declaring elements, updates, and removals are futures: await them and they write straight into the
/// engine on their first poll, in the order they were awaited. Reads such as [`Self::geometry`] and [`Self::with`]
/// complete on their first poll the same way.
pub trait Context<C: 'static = ()>: Sized {
	fn id(&self) -> Id;

	/// Reads or changes the engine's application context, which the engine lends mutably while it polls this
	/// component.
	///
	/// Await the result: `let value = ctx.with(|app: &mut App| app.value).await;`. The closure runs to completion
	/// before the future resolves, so the borrow never spans an `.await`. The host reaches the same value
	/// through [`crate::ui::Engine::ctx`] and [`crate::ui::Engine::ctx_mut`].
	fn with<F, T>(&self, read: F) -> With<C, F>
	where
		F: FnOnce(&mut C) -> T;

	/// Declares a keyed slot under this context for an element, component, or mount.
	///
	/// The slot's id is computed from this context's path and `key`, so declaring the same key in a later frame
	/// finds the same element. See [`ElementKey`] for the keys you can pass and the rule for siblings.
	fn element<'a>(&'a mut self, key: impl Into<ElementKey>) -> ElementSlot<'a, C>;

	fn text(&mut self, content: impl Display, setup: impl Setup<Text>) -> impl Future<Output = EvaluationContext<C>> {
		self.element("text").text(content, setup)
	}

	fn text_field(
		&mut self,
		content: impl Display,
		setup: impl Setup<TextField>,
	) -> impl Future<Output = EvaluationContext<C>> {
		self.element("text_field").text_field(content, setup)
	}

	fn shape(&mut self, setup: impl Setup<Shape>) -> impl Future<Output = EvaluationContext<C>> {
		self.element("shape").shape(setup)
	}

	fn curve(&mut self, setup: impl Setup<Curve>) -> impl Future<Output = EvaluationContext<C>> {
		self.element("curve").curve(setup)
	}

	fn image(
		&mut self,
		width: u32,
		height: u32,
		pixels: impl AsRef<[u8]>,
		setup: impl Setup<Image>,
	) -> impl Future<Output = EvaluationContext<C>> {
		self.element("image").image(width, height, pixels, setup)
	}

	fn path(&mut self, setup: impl Setup<Path>) -> impl Future<Output = EvaluationContext<C>> {
		self.element("path").path(setup)
	}

	fn render(&mut self) -> RenderFuture<C>;

	/// Reads this element's bounds from the last layout, or `None` before it was laid out.
	fn geometry(&self) -> Read<C, Option<Geometry>>;

	/// Reads the pointer state the engine last synchronized.
	fn pointer(&self) -> Read<C, PointerState>;

	/// Reads the engine's captured drag gesture, if a source is held.
	fn drag(&self) -> Read<C, Option<DragCapture>>;

	/// Puts this element on top of the focus stack, so key and text input reach it. Await it.
	fn request_focus(&mut self) -> impl Future<Output = ()>;

	/// Takes this element off the focus stack, returning focus to the element focused before it. Await it.
	fn release_focus(&mut self) -> impl Future<Output = ()>;

	/// Removes everything declared under this context, including the component
	/// itself when awaited from one. See [`EvaluationContext::remove`].
	fn remove(&mut self) -> impl Future<Output = ()>;

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

/// The `ElementContext` trait declares what fills an [`ElementSlot`]: an element, a component, or a mount.
///
/// Each element declaration takes a [`Setup`] function that sets the new element's properties through [`Properties`],
/// such as `|frame| frame.width(240.into()).clip(false)`, and returns a future. Await it: the engine creates the
/// element in its tree and runs setup on it in place. Setup runs only when the declaration creates the element; an
/// element that already exists keeps its properties. The future resolves to the element's own context, where you
/// declare children and edit the element with an `update_*` method such as [`EvaluationContext::update_container`].
pub trait ElementContext<C: 'static = ()> {
	fn container(self, setup: impl Setup<Container>) -> impl Future<Output = EvaluationContext<C>>;

	/// Declares a text element showing `content`, formatted straight into the new element's storage.
	///
	/// Pass a `&str`, a number, or `format_args!("{name} {score}")`: none of them allocates a string of its own.
	fn text(self, content: impl Display, setup: impl Setup<Text>) -> impl Future<Output = EvaluationContext<C>>;

	/// Declares a text field showing `content`, the current value of an application-owned string.
	fn text_field(self, content: impl Display, setup: impl Setup<TextField>) -> impl Future<Output = EvaluationContext<C>>;

	fn shape(self, setup: impl Setup<Shape>) -> impl Future<Output = EvaluationContext<C>>;

	/// Declares a stroked curve. It starts full size with no segments; add them with [`Properties::line`] and the
	/// other segment setters.
	fn curve(self, setup: impl Setup<Curve>) -> impl Future<Output = EvaluationContext<C>>;

	/// Declares an image of `width` by `height` RGBA pixels, shown at its pixel size until setup sizes it. The pixels
	/// are copied into the engine's storage.
	///
	/// # Panics
	///
	/// Awaiting it panics when `pixels` does not hold exactly `width * height * 4` bytes.
	fn image(
		self,
		width: u32,
		height: u32,
		pixels: impl AsRef<[u8]>,
		setup: impl Setup<Image>,
	) -> impl Future<Output = EvaluationContext<C>>;

	/// Declares a filled path. It starts full size with no contours; give it one with [`Properties::outline`] or the
	/// segment setters.
	fn path(self, setup: impl Setup<Path>) -> impl Future<Output = EvaluationContext<C>>;

	/// Starts `component` as a task of its own, such as a widget that reacts to its own events. Await the start.
	///
	/// Pass an async function or closure, such as `async move |ctx| { ... }`. The component gets a context of its
	/// own that declares under this slot, and it runs until its future ends or the enclosing scope is removed.
	fn component(self, component: impl AsyncFnOnce(&mut EvaluationContext<C>) + 'static) -> impl Future<Output = ()>;

	/// Runs `component` inside the awaiting task and returns its output, such as a dialog that resolves to the
	/// button the player pressed. Its elements and tasks are removed when it returns or the future is dropped. See
	/// [`MountedComponentFuture`].
	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static;
}

pub trait ContainerContext<C: 'static = ()>: Context<C> {
	fn on(&mut self, event: Events) -> EventFuture<C>;
	fn on_key(&mut self, key: Key) -> KeyFuture<C>;
	fn on_text_edit(&mut self) -> TextEditFuture<C>;
}
