//! Futures used by mounted UI components.

use std::{sync::mpsc::Sender, time::Instant};

use super::*;

type BoxedMountedUiFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

/// The `MountedComponentFuture` struct runs a component inside the awaiting task and removes its scope when it ends.
///
/// Get one from [`ElementContext::mount`] and await it. The scope starts on the first poll, and its elements and the
/// components spawned inside it are removed when the component returns or this future is dropped.
pub struct MountedComponentFuture<F, T, C = ()> {
	pub(super) component: Option<F>,
	pub(super) future: Option<BoxedMountedUiFuture<T>>,
	pub(super) commands: Sender<UiCommand>,
	/// The id and attach target of the context the scope was declared from, which the scope's context inherits.
	pub(super) id: Id,
	pub(super) parent: Option<Id>,
	pub(super) parent_path: u64,
	/// The path of the mounted scope, computed from the parent's path and the slot key.
	pub(super) path: u64,
	/// The identity that owns the started scope's tasks.
	pub(super) scope: Option<ScopeId>,
	pub(super) complete: bool,
	pub(super) output: PhantomData<fn() -> (T, C)>,
}

impl<F, T, C> Unpin for MountedComponentFuture<F, T, C> {}

impl<F, T, C> MountedComponentFuture<F, T, C> {
	/// Asks the engine to remove the scope's elements and end the tasks spawned inside it.
	fn cleanup_scope(&mut self) {
		let Some(owner) = self.scope.take() else {
			return;
		};
		// Nothing is left to clean up once the engine was dropped, so a failed send is ignored.
		let _ = self.commands.send(UiCommand::Remove {
			path: self.path,
			owner: Some(owner),
		});
	}
}

impl<F, T, C> MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	/// Starts the scope on the first poll, taking its task-ownership identity from the polling engine.
	fn start(&mut self, poll: &mut UiPoll<C>) {
		let Some(component) = self.component.take() else {
			return;
		};

		let owner = poll.runtime.next_scope();
		let ctx = EvaluationContext::new(self.commands.clone(), self.id, self.parent, self.path, owner);
		ctx.send(UiCommand::DeclareScope {
			path: self.path,
			declared_in: self.parent_path,
			task: None,
		});

		// Keep the context and its borrowing component future in one owned future.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await
		});
		self.scope = Some(owner);
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

		self.start(UiPoll::from_context(cx));

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
			// Drop the component's future first: the scope ends after everything it owns.
			self.future = None;
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

/// The `UiPoll` struct is the engine state a UI future reaches while its task is polled.
///
/// The [`Engine`] owns it as a plain field and lends it by `&mut` to every task poll through [`TaskContext::ext`],
/// so no future or context keeps a handle to the engine. Wait futures such as [`RenderFuture`] and [`EventFuture`]
/// hold only identifiers and register through it; reads such as [`Read`] and [`With`] complete from it on their first
/// poll. Writes do not go through it: contexts send [`UiCommand`]s instead.
///
/// A wait stays registered only while its task keeps polling it: after each poll, the runtime drops the waits that
/// poll did not register again. Dropping a wait future, as a losing `select!` branch is, cancels it that way.
/// Combinators that re-poll every live branch on each poll, such as `select!`, `join!`, and `fuse`, keep their
/// branches registered. Do not hold a pending wait in a combinator that polls only woken branches.
///
/// Next, see [`poll_ready_tasks`], the only place that lends it.
pub(crate) struct UiPoll<C> {
	pub(super) runtime: Runtime,
	/// The application context components read through [`Context::with`].
	pub(super) ctx: C,
	/// The task being polled and the number of its poll; see [`Registration::polled`]. `None` between polls.
	pub(super) current: Option<(TaskId, u64)>,
}

impl<C: 'static> UiPoll<C> {
	/// Returns the poll state the engine lent to `cx`.
	pub(crate) fn from_context<'a>(cx: &'a mut TaskContext<'_>) -> &'a mut Self {
		cx.ext().downcast_mut::<Self>().expect(
			"A UI future was polled outside a UI engine. The most likely cause is polling a UI future from another executor instead of awaiting it inside a mounted component.",
		)
	}

	/// Returns the task being polled and the number of its poll.
	fn current(&self) -> (TaskId, u64) {
		self.current.expect(
			"A UI future was polled between task polls. The most likely cause is polling it outside its engine's task loop.",
		)
	}

	/// Registers a UI timer that wakes this task at `deadline`.
	pub(crate) fn wait_until(&mut self, deadline: Instant) {
		let (task, poll) = self.current();
		self.runtime.wait_until(task, deadline, poll);
	}
}

/// The `Read` struct reads one value from the engine's runtime while its task is polled.
///
/// Get one from [`Context::geometry`], [`Context::pointer`], or [`Context::drag`] and await it. It completes on its
/// first poll, so awaiting it never yields to other tasks.
pub struct Read<C, T> {
	pub(super) target: Id,
	pub(super) read: fn(&Runtime, Id) -> T,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C, T> Unpin for Read<C, T> {}

impl<C: 'static, T> Future for Read<C, T> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		Poll::Ready((self.read)(&UiPoll::<C>::from_context(cx).runtime, self.target))
	}
}

/// The `With` struct reads the engine's application context while its task is polled.
///
/// Get one from [`Context::with`] and await it. It completes on its first poll with what `read` returned.
pub struct With<C, F> {
	pub(super) read: Option<F>,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C, F> Unpin for With<C, F> {}

impl<C: 'static, F: FnOnce(&C) -> T, T> Future for With<C, F> {
	type Output = T;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		let read = self.read.take().expect(
			"A UI context read was polled after it completed. The most likely cause is polling a finished future again.",
		);
		Poll::Ready(read(&UiPoll::<C>::from_context(cx).ctx))
	}
}

/// The `RenderFuture` struct resolves on the first frame that begins after it was first polled.
///
/// A wait its task stops polling before it resolves, as the losing branch of a selection is, hands the frame it was
/// counting from to the next wait its task first polls in the same poll. A component that selects over events and
/// frames in a loop therefore still sees every frame while events keep winning.
pub struct RenderFuture<C = ()> {
	/// The token and frame of this wait, set on its first poll.
	pub(super) wait: Option<(u64, u64)>,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for RenderFuture<C> {}

impl<C: 'static> Future for RenderFuture<C> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		let runtime = &mut access.runtime;
		let (token, seen) = *self.wait.get_or_insert_with(|| runtime.start_frame_wait(task, poll));
		if seen < runtime.frame {
			runtime.end_frame_wait(task, token);
			self.complete = true;
			return Poll::Ready(());
		}
		runtime.keep_frame_wait(task, token, seen, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for RenderFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct EventFuture<C = ()> {
	pub(super) target: Id,
	pub(super) kind: Events,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for EventFuture<C> {}

impl<C: 'static> Future for EventFuture<C> {
	type Output = UiEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		if let Some(event) = access.runtime.take_event(task, self.target, self.kind) {
			self.complete = true;
			return Poll::Ready(event);
		}
		access.runtime.wait_for_event(task, self.target, self.kind, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for EventFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct KeyFuture<C = ()> {
	pub(super) target: Id,
	pub(super) key: Key,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for KeyFuture<C> {}

impl<C: 'static> Future for KeyFuture<C> {
	type Output = UiKeyEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		if let Some(event) = access.runtime.take_key_event(task, self.target, self.key) {
			self.complete = true;
			return Poll::Ready(event);
		}
		access.runtime.wait_for_key(task, self.target, self.key, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for KeyFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct TextEditFuture<C = ()> {
	pub(super) target: Id,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for TextEditFuture<C> {}

impl<C: 'static> Future for TextEditFuture<C> {
	type Output = UiTextEditEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		if let Some(event) = access.runtime.take_text_edit_event(task, self.target) {
			self.complete = true;
			return Poll::Ready(event);
		}
		access.runtime.wait_for_text_edit(task, self.target, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for TextEditFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

#[cfg(test)]
mod frame_wait_tests {
	use super::*;
	use crate::ui::layout::context::ContainerContext as _;

	/// Mounts a surface that selects over its drag events and frames, events first, and counts each.
	fn selecting_surface() -> Engine<std::cell::Cell<(u32, u32)>> {
		let mut engine = Engine::with_context(std::cell::Cell::new((0, 0)));
		engine.mount(move |ctx| {
			Box::pin(async move {
				let mut surface = ctx.element("surface").container(Container::default());
				loop {
					let dragged = utils::r#async::select_biased! {
						_ = surface.on(Events::Dragged) => true,
						_ = surface.render() => false,
					};
					ctx.with(|counts| {
						let (events, frames) = counts.get();
						counts.set(if dragged { (events + 1, frames) } else { (events, frames + 1) });
					})
					.await;
				}
			})
		});
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		engine
	}

	#[test]
	fn a_frame_wait_that_keeps_losing_a_selection_still_sees_every_frame() {
		let mut engine = selecting_surface();
		assert!(engine.press(UiPoint::new(0.0, 0.0)));
		for step in 1..=20 {
			engine.drag_to(UiPoint::new(step as f32 * 0.04, 0.0));
			engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		}
		let (events, frames) = engine.ctx().get();
		assert!(events >= 15, "the drag did not reach the surface every frame: {events}");
		assert_eq!(frames, 20, "continuous events starved the frame wait");
	}

	#[test]
	fn a_frame_wait_after_idle_frames_still_waits_for_the_next_frame() {
		let mut engine = Engine::with_context(std::cell::Cell::new(0));
		engine.mount(move |ctx| {
			Box::pin(async move {
				let mut surface = ctx.element("surface").container(Container::default());
				loop {
					// The frame wait loses once and is abandoned while the task idles on events alone.
					utils::r#async::select_biased! {
						_ = surface.on(Events::Grabbed) => {},
						_ = surface.render() => {},
					};
					surface.on(Events::DragEnded).await;
					surface.render().await;
					ctx.with(|ticks| ticks.set(ticks.get() + 1)).await;
				}
			})
		});
		let frame = |engine: &mut Engine<std::cell::Cell<u32>>| {
			engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		};
		frame(&mut engine);
		engine.press(UiPoint::new(0.0, 0.0));
		for _ in 0..10 {
			frame(&mut engine);
		}
		engine.release(UiPoint::new(0.0, 0.0));
		frame(&mut engine);
		assert_eq!(engine.ctx().get(), 0, "a stale frame wait resolved without a new frame");
		frame(&mut engine);
		assert_eq!(engine.ctx().get(), 1);
	}

	#[test]
	fn a_frame_wait_its_task_stopped_polling_stops_requesting_frames() {
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut surface = ctx.element("surface").container(Container::default());
				// The frame wait loses the selection, and the task then waits for events alone.
				utils::r#async::select_biased! {
					_ = surface.on(Events::Grabbed) => {},
					_ = surface.render() => {},
				};
				surface.on(Events::DragEnded).await;
			})
		});
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert!(engine.next_tick().is_some(), "a pending frame wait did not request a frame");
		engine.press(UiPoint::new(0.0, 0.0));
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert_eq!(engine.next_tick(), None, "a dropped frame wait still requests frames");
	}
}
