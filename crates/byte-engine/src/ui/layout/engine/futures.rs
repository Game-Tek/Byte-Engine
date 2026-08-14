//! Futures used by mounted UI components.

use super::*;

type BoxedMountedUiFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

pub struct MountedComponentFuture<F, T, C = ()> {
	pub(super) component: Option<F>,
	pub(super) future: Option<BoxedMountedUiFuture<T>>,
	pub(super) ctx: Rc<C>,
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) tree: Rc<RefCell<RetainedTree>>,
	pub(super) parent: Id,
	pub(super) parent_path: Vec<PathSegment>,
	pub(super) name: &'static str,
	pub(super) task_id: TaskId,
	pub(super) scope: Option<Vec<PathSegment>>,
	pub(super) complete: bool,
	pub(super) output: PhantomData<T>,
}

impl<F, T, C> Unpin for MountedComponentFuture<F, T, C> {}

impl<F, T, C> MountedComponentFuture<F, T, C> {
	fn cleanup_scope(&mut self) {
		let Some(scope) = self.scope.take() else {
			return;
		};

		let removed = self.tree.borrow_mut().remove_scope(&scope);
		if !removed.is_empty() {
			self.runtime.borrow_mut().remove_targets(&removed);
		}
	}
}

impl<F, T, C> MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	fn start(&mut self) {
		if self.future.is_some() {
			return;
		}

		let Some(component) = self.component.take() else {
			return;
		};

		let scope = self
			.tree
			.borrow_mut()
			.scope_path(Some(self.parent), &self.parent_path, self.name);
		let ctx = EvaluationContext {
			id: self.parent,
			parent: Some(self.parent),
			path: scope.clone(),
			ctx: Rc::clone(&self.ctx),
			runtime: Rc::clone(&self.runtime),
			tree: Rc::clone(&self.tree),
			task_id: self.task_id,
		};

		// Keep the context and its borrowing component future in one owned future.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await
		});
		self.scope = Some(scope);
		self.future = Some(future);
	}
}

impl<F, T, C> Future for MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	type Output = T;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		self.start();

		let Some(future) = self.future.as_mut() else {
			return Poll::Pending;
		};

		match future.as_mut().poll(cx) {
			Poll::Ready(output) => {
				self.complete = true;
				self.future = None;
				self.cleanup_scope();
				Poll::Ready(output)
			}
			Poll::Pending => Poll::Pending,
		}
	}
}

impl<F, T, C> Drop for MountedComponentFuture<F, T, C> {
	fn drop(&mut self) {
		if !self.complete {
			self.cleanup_scope();
		}
	}
}

impl<F, T, C> FusedFuture for MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct RenderFuture {
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) frame_seen: Option<u64>,
	pub(super) complete: bool,
}

impl Future for RenderFuture {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let current = self.runtime.borrow().frame;

		match self.frame_seen {
			None => {
				self.frame_seen = Some(current);
				self.runtime.borrow_mut().frame_waiters.push(cx.waker().clone());
				Poll::Pending
			}
			Some(seen) if seen < current => {
				self.complete = true;
				Poll::Ready(())
			}
			Some(_) => {
				self.runtime.borrow_mut().frame_waiters.push(cx.waker().clone());
				Poll::Pending
			}
		}
	}
}

impl FusedFuture for RenderFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct EventFuture {
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) kind: Events,
	pub(super) complete: bool,
}

impl Future for EventFuture {
	type Output = UiEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_event(self.task_id, self.target, self.kind);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_event(self.task_id, self.target, self.kind, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for EventFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct KeyFuture {
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) key: Key,
	pub(super) complete: bool,
}

impl Future for KeyFuture {
	type Output = UiKeyEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_key_event(self.task_id, self.target, self.key);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_key(self.task_id, self.target, self.key, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for KeyFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct TextEditFuture {
	pub(super) runtime: Rc<RefCell<Runtime>>,
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) complete: bool,
}

impl Future for TextEditFuture {
	type Output = UiTextEditEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_text_edit_event(self.task_id, self.target);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_text_edit(self.task_id, self.target, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for TextEditFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}
