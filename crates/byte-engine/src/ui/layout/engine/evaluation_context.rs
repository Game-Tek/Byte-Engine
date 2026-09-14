//! Evaluation context and component-construction implementations.

use super::*;

/// The `EvaluationContext` struct keeps the context owned by a mounted asynchronous
/// UI task.
pub struct EvaluationContext<C = ()> {
	pub(super) id: Id,
	pub(super) parent: Option<Id>,
	pub(super) path: usize,
	pub(super) ctx: Rc<C>,
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) tree: Rc<RefCell<RetainedTree>>,
	pub(super) task_id: TaskId,
	/// The mounted scope that owns tasks spawned from this context.
	pub(super) owner: ScopeId,
}

impl<C> EvaluationContext<C> {
	pub(super) fn new_root(
		ctx: Rc<C>,
		runtime: Rc<RefCell<Runtime>>,
		tree: Rc<RefCell<RetainedTree>>,
		task_id: TaskId,
	) -> Self {
		Self {
			id: Id::new(1).unwrap(),
			parent: None,
			path: 0,
			ctx,
			runtime,
			tree,
			task_id,
			owner: ScopeId::ROOT,
		}
	}

	fn new_child(
		ctx: Rc<C>,
		runtime: Rc<RefCell<Runtime>>,
		tree: Rc<RefCell<RetainedTree>>,
		task_id: TaskId,
		owner: ScopeId,
		id: Id,
		path: usize,
	) -> Self {
		Self {
			id,
			parent: Some(id),
			path,
			ctx,
			runtime,
			tree,
			task_id,
			owner,
		}
	}

	fn add_element(&mut self, name: &'static str, element: ConcreteElement) -> EvaluationContext<C> {
		let (id, path) = self.tree.borrow_mut().add_element(self.parent, self.path, name, element);
		EvaluationContext::new_child(
			Rc::clone(&self.ctx),
			Rc::clone(&self.runtime),
			Rc::clone(&self.tree),
			self.task_id,
			self.owner,
			id,
			path,
		)
	}

	/// Moves this element under another parent as its last child, keeping its
	/// id, path, and properties. The new parent's flow lays it out from the next
	/// frame. Returns false for an unknown parent or when the parent is this
	/// element or one of its descendants.
	pub fn reparent(&mut self, parent: Id) -> bool {
		self.tree.borrow_mut().reparent(self.id, parent)
	}

	pub fn update_container(&mut self, update: impl FnOnce(&mut Container)) -> bool {
		self.tree.borrow_mut().update_element(self.id, |primitive| {
			let Primitives::Container(value) = primitive else {
				return false;
			};
			update(value);
			true
		})
	}

	pub fn update_text(&mut self, update: impl FnOnce(&mut Text)) -> bool {
		self.tree.borrow_mut().update_element(self.id, |primitive| {
			let Primitives::Text(value) = primitive else { return false };
			update(value);
			true
		})
	}

	pub fn update_text_field(&mut self, update: impl FnOnce(&mut TextField)) -> bool {
		self.tree.borrow_mut().update_element(self.id, |primitive| {
			let Primitives::TextField(value) = primitive else {
				return false;
			};
			update(value);
			true
		})
	}

	pub fn update_shape(&mut self, update: impl FnOnce(&mut Shape)) -> bool {
		self.tree.borrow_mut().update_element(self.id, |primitive| {
			let Primitives::Shape(value) = primitive else { return false };
			update(value);
			true
		})
	}

	pub fn update_image(&mut self, update: impl FnOnce(&mut Image)) -> bool {
		self.tree.borrow_mut().update_element(self.id, |primitive| {
			let Primitives::Image(value) = primitive else { return false };
			update(value);
			true
		})
	}

	pub fn geometry(&self) -> Option<Geometry> {
		self.runtime.borrow().geometry.get(&self.id).copied()
	}

	pub fn pointer(&self) -> PointerState {
		self.runtime.borrow().pointer
	}

	/// Returns the engine's captured drag gesture, if a source is held.
	pub fn drag(&self) -> Option<DragCapture> {
		self.runtime.borrow().drag.capture()
	}
}

impl<C: 'static> Context<C> for EvaluationContext<C> {
	fn id(&self) -> Id {
		self.id
	}

	fn ctx(&self) -> &C {
		self.ctx.as_ref()
	}

	fn element<'a>(&'a mut self, name: &'static str) -> ElementSlot<'a, C> {
		ElementSlot { parent: self, name }
	}

	fn render(&mut self) -> RenderFuture {
		RenderFuture {
			waiter: None,
			runtime: Rc::clone(&self.runtime),
			frame_seen: None,
			complete: false,
		}
	}

	fn geometry(&self) -> Option<Geometry> {
		EvaluationContext::geometry(self)
	}

	fn pointer(&self) -> PointerState {
		EvaluationContext::pointer(self)
	}

	fn drag(&self) -> Option<DragCapture> {
		EvaluationContext::drag(self)
	}

	fn request_focus(&mut self) {
		self.runtime.borrow_mut().request_focus(self.id);
	}

	fn release_focus(&mut self) {
		self.runtime.borrow_mut().release_focus(self.id);
	}
}

impl<C: 'static> ElementContext<C> for ElementSlot<'_, C> {
	fn container(self, element: Container) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::container(element))
	}

	fn text(self, text: Text) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::text(text))
	}

	fn text_field(self, text_field: TextField) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::text_field(text_field))
	}

	fn shape(self, shape: Shape) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::shape(shape))
	}

	fn curve(self, curve: Curve) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::curve(curve))
	}

	fn image(self, image: Image) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::image(image))
	}

	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.parent.runtime);
		let tree = Rc::clone(&self.parent.tree);
		// The task belongs to the enclosing mounted scope and ends when that scope is removed.
		let task_id = runtime.borrow_mut().reserve_task(self.parent.owner);
		let path = tree
			.borrow_mut()
			.scope_path(Some(self.parent.id), self.parent.path, self.name);
		let ctx = EvaluationContext {
			id: self.parent.id,
			parent: Some(self.parent.id),
			path,
			ctx: Rc::clone(&self.parent.ctx),
			runtime: Rc::clone(&runtime),
			tree,
			task_id,
			owner: self.parent.owner,
		};

		// The runtime owns the context through this outer future; the component's
		// borrowed future never escapes the scope in which that context is alive.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await;
		});
		runtime.borrow_mut().start_task(task_id, future);
	}

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
			ctx: Rc::clone(&self.parent.ctx),
			runtime: Rc::clone(&self.parent.runtime),
			tree: Rc::clone(&self.parent.tree),
			parent: self.parent.id,
			parent_path: self.parent.path,
			name: self.name,
			task_id: self.parent.task_id,
			scope: None,
			complete: false,
			output: PhantomData,
		}
	}
}

impl<C: 'static> super::super::context::ContainerContext<C> for EvaluationContext<C> {
	fn on(&mut self, event: Events) -> EventFuture {
		EventFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			kind: event,
			complete: false,
		}
	}

	fn on_key(&mut self, key: Key) -> KeyFuture {
		KeyFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			key,
			complete: false,
		}
	}

	fn on_text_edit(&mut self) -> TextEditFuture {
		TextEditFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			complete: false,
		}
	}
}
