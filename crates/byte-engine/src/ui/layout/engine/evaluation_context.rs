//! Evaluation context and component-construction implementations.

use std::sync::mpsc::Sender;

use super::*;
use crate::ui::layout::context::slot_path;

/// The `EvaluationContext` struct lets a mounted asynchronous UI component declare and edit its part of the tree.
///
/// It holds no engine state: writes are sent to the engine through a channel and applied after the current task
/// poll, in the order they were made, and reads such as [`Context::geometry`] and [`Context::with`] are futures that
/// complete on their first poll. Get one from [`Engine::mount`] or from an [`ElementSlot`] of another context.
pub struct EvaluationContext<C = ()> {
	/// The element this context edits, or for a component scope the element it was declared under.
	pub(super) id: Id,
	/// The element declarations from this context attach under.
	pub(super) parent: Option<Id>,
	/// The path slots declared from this context are keyed under; see [`slot_path`].
	pub(super) path: u64,
	pub(super) commands: Sender<UiCommand>,
	/// The mounted scope that owns tasks spawned from this context.
	pub(super) owner: ScopeId,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> EvaluationContext<C> {
	pub(super) fn new(commands: Sender<UiCommand>, id: Id, parent: Option<Id>, path: u64, owner: ScopeId) -> Self {
		Self {
			id,
			parent,
			path,
			commands,
			owner,
			ctx: PhantomData,
		}
	}

	/// Makes the context of the root component, which declares top-level elements.
	pub(super) fn new_root(commands: Sender<UiCommand>) -> Self {
		// The root context edits no element; its id only names the root for focus and events.
		Self::new(commands, Id::MIN, None, ROOT_PATH, ScopeId::ROOT)
	}

	/// Sends a change to the engine. A send fails only once the engine was dropped, when nothing is left to change.
	pub(super) fn send(&self, command: UiCommand) {
		let _ = self.commands.send(command);
	}

	fn add_element(&mut self, key: ElementKey, element: ConcreteElement) -> EvaluationContext<C> {
		let id = slot_path(self.path, key);
		self.send(UiCommand::Create {
			parent: self.parent,
			declared_in: self.path,
			id,
			element: Box::new(element),
		});
		EvaluationContext::new(self.commands.clone(), id, Some(id), id.get(), self.owner)
	}

	/// Moves this element under another parent as its last child, keeping its
	/// id, path, and properties. The new parent's flow lays it out from the next
	/// frame. An unknown parent, or a parent that is this element or one of its
	/// descendants, is logged when the move is applied and changes nothing.
	pub fn reparent(&mut self, parent: Id) {
		self.send(UiCommand::Reparent { child: self.id, parent });
	}

	/// Moves another retained element under this element as its last child, so a
	/// drop target can take in the source it was given. An unknown element, or one
	/// that is this element or one of its ancestors, is logged when the move is
	/// applied and changes nothing.
	pub fn adopt(&mut self, child: Id) {
		self.send(UiCommand::Reparent { child, parent: self.id });
	}

	/// Removes this element, everything declared under it, and the components started
	/// there. The removal is applied after the current poll.
	///
	/// Called from a component's own context, it removes that component's elements
	/// and ends the component, so return right after. The context stays usable only
	/// to declare again: a later declaration of the same key under the same parent
	/// gets the same id. A mounted component awaited from outside the removed element
	/// keeps running until its own future ends; its elements are gone, so its updates
	/// change nothing.
	pub fn remove(&mut self) {
		self.send(UiCommand::Remove {
			path: self.path,
			owner: None,
		});
	}

	/// Sends an edit of this element's primitive. `edit` returns false when the primitive is of another kind.
	fn update(&mut self, edit: impl FnOnce(&mut Primitives) -> bool + 'static) {
		self.send(UiCommand::Update {
			id: self.id,
			update: Box::new(edit),
		});
	}

	/// Edits this container. The edit is applied after the current poll, in the order it was made.
	pub fn update_container(&mut self, update: impl FnOnce(&mut Container) + 'static) {
		self.update(|primitive| {
			let Primitives::Container(value) = primitive else {
				return false;
			};
			update(value);
			true
		});
	}

	pub fn update_text(&mut self, update: impl FnOnce(&mut Text) + 'static) {
		self.update(|primitive| {
			let Primitives::Text(value) = primitive else { return false };
			update(value);
			true
		});
	}

	pub fn update_text_field(&mut self, update: impl FnOnce(&mut TextField) + 'static) {
		self.update(|primitive| {
			let Primitives::TextField(value) = primitive else {
				return false;
			};
			update(value);
			true
		});
	}

	pub fn update_shape(&mut self, update: impl FnOnce(&mut Shape) + 'static) {
		self.update(|primitive| {
			let Primitives::Shape(value) = primitive else { return false };
			update(value);
			true
		});
	}

	/// Edits a retained curve in place, such as re-routing a wire while its ends move.
	///
	/// Changing only the path's segments keeps the layout placement; changing its
	/// size or transform replays placement like any other element edit.
	pub fn update_curve(&mut self, update: impl FnOnce(&mut Curve) + 'static) {
		self.update(|primitive| {
			let Primitives::Curve(value) = primitive else { return false };
			update(value);
			true
		});
	}

	/// Edits a retained path in place. Replacing its outline repacks it; a style edit repaints only.
	pub fn update_path(&mut self, update: impl FnOnce(&mut crate::ui::components::path::Path) + 'static) {
		self.update(|primitive| {
			let Primitives::Path(value) = primitive else { return false };
			update(value);
			true
		});
	}

	pub fn update_image(&mut self, update: impl FnOnce(&mut Image) + 'static) {
		self.update(|primitive| {
			let Primitives::Image(value) = primitive else { return false };
			update(value);
			true
		});
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

	fn request_focus(&mut self) {
		self.send(UiCommand::Focus {
			target: self.id,
			focused: true,
		});
	}

	fn release_focus(&mut self) {
		self.send(UiCommand::Focus {
			target: self.id,
			focused: false,
		});
	}

	fn remove(&mut self) {
		EvaluationContext::remove(self);
	}
}

impl<C: 'static> ElementContext<C> for ElementSlot<'_, C> {
	fn container(self, element: Container) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::container(element))
	}

	fn text(self, text: Text) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::text(text))
	}

	fn text_field(self, text_field: TextField) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::text_field(text_field))
	}

	fn shape(self, shape: Shape) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::shape(shape))
	}

	fn curve(self, curve: Curve) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::curve(curve))
	}

	fn image(self, image: Image) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::image(image))
	}

	fn path(self, path: crate::ui::components::path::Path) -> EvaluationContext<C> {
		self.parent.add_element(self.key, ConcreteElement::path(path))
	}

	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let parent = &*self.parent;
		// The task belongs to the enclosing mounted scope and ends when that scope is removed,
		// or earlier when the element it was declared under is removed.
		let path = slot_path(parent.path, self.key).get();
		let ctx = EvaluationContext::new(parent.commands.clone(), parent.id, parent.parent, path, parent.owner);

		// The runtime owns the context through this outer future; the component's
		// borrowed future never escapes the scope in which that context is alive.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await;
		});
		parent.send(UiCommand::DeclareScope {
			path,
			declared_in: parent.path,
			task: Some((parent.owner, future)),
		});
	}

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
			commands: self.parent.commands.clone(),
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
