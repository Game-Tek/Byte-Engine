//! Evaluation context and component-construction implementations.

use std::fmt::Display;

use super::{
	properties::{Setup, Spares, assert_rgba_len, declare, update},
	*,
};
use crate::ui::{components::path::Path, layout::context::slot_path};

/// The `EvaluationContext` struct lets a mounted asynchronous UI component declare and edit its part of the tree.
///
/// It holds no engine state, only the identifiers that place its declarations. Every declaration, edit, and change is
/// a future that completes on its first poll, writing straight into the engine the task is polled by; reads such as
/// [`Context::geometry`] and [`Context::with`] complete the same way. Get one from [`Engine::mount`] or from an
/// [`ElementSlot`] of another context.
pub struct EvaluationContext<C = ()> {
	/// The element this context edits, or for a component scope the element it was declared under.
	pub(super) id: Id,
	/// The element declarations from this context attach under.
	pub(super) parent: Option<Id>,
	/// The path slots declared from this context are keyed under; see [`slot_path`].
	pub(super) path: u64,
	/// The mounted scope that owns tasks spawned from this context.
	pub(super) owner: ScopeId,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C: 'static> EvaluationContext<C> {
	pub(super) fn new(id: Id, parent: Option<Id>, path: u64, owner: ScopeId) -> Self {
		Self {
			id,
			parent,
			path,
			owner,
			ctx: PhantomData,
		}
	}

	/// Makes the context of the root component, which declares top-level elements.
	pub(super) fn new_root() -> Self {
		// The root context edits no element; its id only names the root for focus and events.
		Self::new(Id::MIN, None, ROOT_PATH, ScopeId::ROOT)
	}

	/// Returns a future that applies `command` on its first poll, so what the component does next already sees it.
	fn change(command: UiCommand) -> impl Future<Output = ()> {
		direct(move |poll: &mut UiPoll<C>| poll.apply(command))
	}

	/// Moves this element under another parent as its last child, keeping its
	/// id, path, and properties. Await it. The new parent's flow lays it out from
	/// the next frame. An unknown parent, or a parent that is this element or one
	/// of its descendants, is logged and changes nothing.
	pub fn reparent(&mut self, parent: Id) -> impl Future<Output = ()> {
		Self::change(UiCommand::Reparent { child: self.id, parent })
	}

	/// Moves another retained element under this element as its last child, so a
	/// drop target can take in the source it was given. Await it. An unknown
	/// element, or one that is this element or one of its ancestors, is logged and
	/// changes nothing.
	pub fn adopt(&mut self, child: Id) -> impl Future<Output = ()> {
		Self::change(UiCommand::Reparent { child, parent: self.id })
	}

	/// Removes this element, everything declared under it, and the components started
	/// there. Await it.
	///
	/// Awaited in a component's own context, it removes that component's elements
	/// and ends the component, so return right after. The context stays usable only
	/// to declare again: a later declaration of the same key under the same parent
	/// gets the same id. A mounted component awaited from outside the removed element
	/// keeps running until its own future ends; its elements are gone, so its updates
	/// change nothing.
	pub fn remove(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Remove {
			path: self.path,
			owner: None,
		})
	}

	/// Edits this container with `setup`. Await it. The engine compares the container before and after the whole
	/// edit, so writing values it already has changes nothing.
	pub fn update_container(&mut self, setup: impl Setup<Container>) -> impl Future<Output = ()> {
		update::<C, Container>(self.id, setup)
	}

	/// Edits this text element. See [`Self::update_container`].
	pub fn update_text(&mut self, setup: impl Setup<Text>) -> impl Future<Output = ()> {
		update::<C, Text>(self.id, setup)
	}

	/// Edits this text field. See [`Self::update_container`].
	pub fn update_text_field(&mut self, setup: impl Setup<TextField>) -> impl Future<Output = ()> {
		update::<C, TextField>(self.id, setup)
	}

	/// Edits this shape. See [`Self::update_container`].
	pub fn update_shape(&mut self, setup: impl Setup<Shape>) -> impl Future<Output = ()> {
		update::<C, Shape>(self.id, setup)
	}

	/// Edits a retained curve in place, such as re-routing a wire while its ends move.
	///
	/// Changing only the segments keeps the layout placement, and re-routing with
	/// [`Properties::clear_segments`] reuses the segment storage. Changing its size or
	/// transform replays placement like any other element edit.
	pub fn update_curve(&mut self, setup: impl Setup<Curve>) -> impl Future<Output = ()> {
		update::<C, Curve>(self.id, setup)
	}

	/// Edits a retained path in place. Replacing its outline repacks it; a style edit repaints only.
	pub fn update_path(&mut self, setup: impl Setup<Path>) -> impl Future<Output = ()> {
		update::<C, Path>(self.id, setup)
	}

	/// Edits this image. See [`Self::update_container`].
	pub fn update_image(&mut self, setup: impl Setup<Image>) -> impl Future<Output = ()> {
		update::<C, Image>(self.id, setup)
	}

	fn read<T>(&self, read: fn(&Runtime, Id) -> T) -> Read<C, T> {
		Read {
			target: self.id,
			read,
			ctx: PhantomData,
		}
	}
}

impl<C: 'static> Context<C> for EvaluationContext<C> {
	fn id(&self) -> Id {
		self.id
	}

	fn with<F, T>(&self, read: F) -> With<C, F>
	where
		F: FnOnce(&C) -> T,
	{
		With {
			read: Some(read),
			ctx: PhantomData,
		}
	}

	fn element<'a>(&'a mut self, key: impl Into<ElementKey>) -> ElementSlot<'a, C> {
		ElementSlot {
			parent: self,
			key: key.into(),
		}
	}

	fn render(&mut self) -> RenderFuture<C> {
		RenderFuture {
			wait: None,
			complete: false,
			ctx: PhantomData,
		}
	}

	fn geometry(&self) -> Read<C, Option<Geometry>> {
		self.read(|runtime, id| runtime.geometry.get(&id).copied())
	}

	fn pointer(&self) -> Read<C, PointerState> {
		self.read(|runtime, _| runtime.pointer)
	}

	fn drag(&self) -> Read<C, Option<DragCapture>> {
		self.read(|runtime, _| runtime.drag.capture())
	}

	fn request_focus(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Focus {
			target: self.id,
			focused: true,
		})
	}

	fn release_focus(&mut self) -> impl Future<Output = ()> {
		Self::change(UiCommand::Focus {
			target: self.id,
			focused: false,
		})
	}

	fn remove(&mut self) -> impl Future<Output = ()> {
		EvaluationContext::remove(self)
	}
}

impl<C: 'static> ElementContext<C> for ElementSlot<'_, C> {
	fn container(self, setup: impl Setup<Container>) -> impl Future<Output = EvaluationContext<C>> {
		declare(self, |_, _| Primitives::Container(Container::default()), setup)
	}

	fn text(self, content: impl Display, setup: impl Setup<Text>) -> impl Future<Output = EvaluationContext<C>> {
		declare(
			self,
			move |_, spares| Primitives::Text(Text::new(spares.format(content))),
			setup,
		)
	}

	fn text_field(self, content: impl Display, setup: impl Setup<TextField>) -> impl Future<Output = EvaluationContext<C>> {
		declare(
			self,
			move |_, spares| Primitives::TextField(TextField::new(spares.format(content))),
			setup,
		)
	}

	fn shape(self, setup: impl Setup<Shape>) -> impl Future<Output = EvaluationContext<C>> {
		declare(self, |_, _| Primitives::Shape(Shape::new()), setup)
	}

	fn curve(self, setup: impl Setup<Curve>) -> impl Future<Output = EvaluationContext<C>> {
		let create = |_, spares: &mut Spares| {
			let mut curve = Curve::new();
			curve.path.segments = spares.segments();
			Primitives::Curve(curve)
		};
		declare(self, create, setup)
	}

	fn image(
		self,
		width: u32,
		height: u32,
		pixels: impl AsRef<[u8]>,
		setup: impl Setup<Image>,
	) -> impl Future<Output = EvaluationContext<C>> {
		let create = move |content_id, _: &mut Spares| {
			let pixels = pixels.as_ref();
			assert_rgba_len(width, height, pixels);
			Primitives::Image(Image::new(content_id, width, height, pixels))
		};
		declare(self, create, setup)
	}

	fn path(self, setup: impl Setup<Path>) -> impl Future<Output = EvaluationContext<C>> {
		let create = |content_id, spares: &mut Spares| {
			let mut path = Path::new(content_id);
			path.path.segments = spares.segments();
			Primitives::Path(path)
		};
		declare(self, create, setup)
	}

	fn component(self, component: impl AsyncFnOnce(&mut EvaluationContext<C>) + 'static) -> impl Future<Output = ()> {
		let EvaluationContext {
			id,
			parent,
			path: declared_in,
			owner,
			..
		} = *self.parent;
		let path = slot_path(declared_in, self.key).get();
		// The task belongs to the enclosing mounted scope and ends when that scope is removed,
		// or earlier when the element it was declared under is removed.
		direct(move |poll: &mut UiPoll<C>| {
			poll.tree.declare_scope(path, declared_in);
			let mut ctx = EvaluationContext::new(id, parent, path, owner);
			// The runtime owns the context through this future; the component's borrow of it never escapes it.
			poll.runtime
				.spawn(owner, path, Box::pin(async move { component(&mut ctx).await }));
		})
	}

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
			id: self.parent.id,
			parent: self.parent.parent,
			parent_path: self.parent.path,
			path: slot_path(self.parent.path, self.key).get(),
			scope: None,
			complete: false,
			output: PhantomData,
		}
	}
}

impl<C: 'static> super::super::context::ContainerContext<C> for EvaluationContext<C> {
	fn on(&mut self, event: Events) -> EventFuture<C> {
		EventFuture {
			target: self.id,
			kind: event,
			complete: false,
			ctx: PhantomData,
		}
	}

	fn on_key(&mut self, key: Key) -> KeyFuture<C> {
		KeyFuture {
			target: self.id,
			key,
			complete: false,
			ctx: PhantomData,
		}
	}

	fn on_text_edit(&mut self) -> TextEditFuture<C> {
		TextEditFuture {
			target: self.id,
			complete: false,
			ctx: PhantomData,
		}
	}
}
