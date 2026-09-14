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
	pub(super) parent_path: usize,
	pub(super) name: &'static str,
	pub(super) task_id: TaskId,
	/// The started scope's element path and the identity that owns its tasks.
	pub(super) scope: Option<(usize, ScopeId)>,
	pub(super) complete: bool,
	pub(super) output: PhantomData<T>,
}

impl<F, T, C> Unpin for MountedComponentFuture<F, T, C> {}

impl<F, T, C> MountedComponentFuture<F, T, C> {
	/// Removes the scope's elements and ends the tasks spawned inside it.
	fn cleanup_scope(&mut self) {
		let Some((path, owner)) = self.scope.take() else {
			return;
		};

		{
			let mut tree = self.tree.borrow_mut();
			let removed = tree.remove_scope(path);
			if !removed.is_empty() {
				self.runtime.borrow_mut().remove_targets(removed);
			}
		}
		Runtime::end_scope(&self.runtime, owner);
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
			.scope_path(Some(self.parent), self.parent_path, self.name);
		let owner = self.runtime.borrow_mut().next_scope();
		let ctx = EvaluationContext {
			id: self.parent,
			parent: Some(self.parent),
			path: scope,
			ctx: Rc::clone(&self.ctx),
			runtime: Rc::clone(&self.runtime),
			tree: Rc::clone(&self.tree),
			task_id: self.task_id,
			owner,
		};

		// Keep the context and its borrowing component future in one owned future.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await
		});
		self.scope = Some((scope, owner));
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
	pub(super) waiter: Option<StableVecHandle>,
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
		if self.frame_seen.is_some_and(|seen| seen < current) {
			if let Some(waiter) = self.waiter.take() {
				self.runtime.borrow_mut().frame_waiters.remove(waiter);
			}
			self.complete = true;
			return Poll::Ready(());
		}
		self.frame_seen = Some(current);
		// A future keeps one subscription even when another selected branch wakes its task.
		if let Some(waiter) = self.waiter {
			let mut runtime = self.runtime.borrow_mut();
			if let Some(Some(waker)) = runtime.frame_waiters.get_mut(waiter) {
				waker.clone_from(cx.waker());
			}
		} else {
			let waiter = self.runtime.borrow_mut().frame_waiters.push(Some(cx.waker().clone()));
			self.waiter = Some(waiter);
		}
		Poll::Pending
	}
}

impl Drop for RenderFuture {
	fn drop(&mut self) {
		// Dropping a losing select branch cancels its pending frame notification.
		if let Some(waiter) = self.waiter.take() {
			self.runtime.borrow_mut().frame_waiters.remove(waiter);
		}
	}
}

impl FusedFuture for RenderFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

#[cfg(test)]
mod frame_wait_tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::*;

	/// The `WakeCount` struct observes notifications through the future's caller-supplied waker.
	#[derive(Default)]
	struct WakeCount(AtomicUsize);

	impl Wake for WakeCount {
		fn wake(self: Arc<Self>) {
			self.0.fetch_add(1, Ordering::Relaxed);
		}
	}

	/// Creates a frame wait through the same context a mounted component receives.
	fn frame_wait() -> (Engine, RenderFuture) {
		let result = Rc::new(RefCell::new(None));
		let output = Rc::clone(&result);
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			Box::pin(async move {
				ctx.element("root").container(Container::default());
				*output.borrow_mut() = Some(ctx.render());
			})
		});
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		let future = result.borrow_mut().take().unwrap();
		(engine, future)
	}

	#[test]
	fn frame_wait_notifies_the_latest_caller_once() {
		let (mut engine, mut future) = frame_wait();
		let first = Arc::new(WakeCount::default());
		let latest = Arc::new(WakeCount::default());
		let first_waker = Waker::from(Arc::clone(&first));
		let latest_waker = Waker::from(Arc::clone(&latest));
		for _ in 0..8 {
			assert!(
				Pin::new(&mut future)
					.poll(&mut TaskContext::from_waker(&first_waker))
					.is_pending()
			);
		}
		assert!(
			Pin::new(&mut future)
				.poll(&mut TaskContext::from_waker(&latest_waker))
				.is_pending()
		);
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert_eq!(first.0.load(Ordering::Relaxed), 0);
		assert_eq!(latest.0.load(Ordering::Relaxed), 1);
		assert!(
			Pin::new(&mut future)
				.poll(&mut TaskContext::from_waker(&latest_waker))
				.is_ready()
		);
	}

	#[test]
	fn dropping_a_frame_wait_cancels_its_notification() {
		let (mut engine, mut future) = frame_wait();
		let count = Arc::new(WakeCount::default());
		let waker = Waker::from(Arc::clone(&count));
		assert!(Pin::new(&mut future).poll(&mut TaskContext::from_waker(&waker)).is_pending());
		drop(future);
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert_eq!(count.0.load(Ordering::Relaxed), 0);
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
