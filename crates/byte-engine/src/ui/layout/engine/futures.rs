//! Futures used by mounted UI components.

use std::{
	sync::mpsc::{Receiver, Sender},
	time::Instant,
};

use super::*;

type BoxedMountedUiFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

/// The `MountedComponentFuture` struct runs a component inside the awaiting task and removes its scope when it ends.
///
/// Get one from [`ElementContext::mount`] and await it. The scope starts on the first poll, and its elements and the
/// components spawned inside it are removed when the component returns or this future is dropped.
pub struct MountedComponentFuture<F, T, C = ()> {
	pub(super) component: Option<F>,
	pub(super) future: Option<BoxedMountedUiFuture<T>>,
	/// The id and attach target of the context the scope was declared from, which the scope's context inherits.
	pub(super) id: Id,
	pub(super) parent: Option<Id>,
	pub(super) parent_path: u64,
	/// The path of the mounted scope, computed from the parent's path and the slot key.
	pub(super) path: u64,
	/// The started scope's task owner, and the channel its removal goes through when this future is dropped before
	/// the component ends; a destructor cannot reach the engine any other way.
	pub(super) scope: Option<(ScopeId, Sender<UiCommand>)>,
	pub(super) complete: bool,
	pub(super) output: PhantomData<fn() -> (T, C)>,
}

impl<F, T, C> Unpin for MountedComponentFuture<F, T, C> {}

impl<F, T, C> MountedComponentFuture<F, T, C>
where
	C: 'static,
	T: 'static,
	F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static,
{
	/// Starts the scope on the first poll: declares it in the tree and takes its task-ownership identity.
	fn start(&mut self, poll: &mut UiPoll<C>) {
		let Some(component) = self.component.take() else {
			return;
		};
		poll.apply_commands();
		let owner = poll.runtime.next_scope();
		poll.tree.declare_scope(self.path, self.parent_path);
		let ctx = EvaluationContext::new(self.id, self.parent, self.path, owner);
		// Keep the context and its borrowing component future in one owned future.
		self.future = Some(Box::pin(async move {
			let mut ctx = ctx;
			component(&mut ctx).await
		}));
		self.scope = Some((owner, poll.sender.clone()));
	}
}

impl<F, T, C> Future for MountedComponentFuture<F, T, C>
where
	C: 'static,
	T: 'static,
	F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static,
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
				if let Some((owner, _)) = self.scope.take() {
					let path = self.path;
					UiPoll::<C>::from_context(cx).apply(UiCommand::Remove {
						path,
						owner: Some(owner),
					});
				}
				Poll::Ready(output)
			}
			Poll::Pending => Poll::Pending,
		}
	}
}

impl<F, T, C> Drop for MountedComponentFuture<F, T, C> {
	fn drop(&mut self) {
		if self.complete {
			return;
		}
		// Drop the component's future first: the scope ends after everything it owns.
		self.future = None;
		if let Some((owner, sender)) = self.scope.take() {
			// Nothing is left to clean up once the engine was dropped, so a failed send is ignored.
			let _ = sender.send(UiCommand::Remove {
				path: self.path,
				owner: Some(owner),
			});
		}
	}
}

impl<F, T, C> FusedFuture for MountedComponentFuture<F, T, C>
where
	C: 'static,
	T: 'static,
	F: AsyncFnOnce(&mut EvaluationContext<C>) -> T + 'static,
{
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// Returns a future that runs `write` on its first poll with the engine state lent to the polling task.
///
/// Declarations, edits, and structural changes are these futures, so they write straight into [`UiPoll::tree`] and
/// [`UiPoll::runtime`]. The removals of mounts dropped earlier in the poll land first, as they happened first.
pub(super) fn direct<C: 'static, T>(write: impl FnOnce(&mut UiPoll<C>) -> T) -> impl Future<Output = T> {
	let mut write = Some(write);
	std::future::poll_fn(move |cx| {
		let write = write
			.take()
			.expect("A UI write was polled after it completed. The most likely cause is polling a finished future again.");
		let poll = UiPoll::<C>::from_context(cx);
		poll.apply_commands();
		Poll::Ready(write(poll))
	})
}

/// The `UiPoll` struct is the engine state a UI future reaches while its task is polled.
///
/// The [`Engine`] owns it as a plain field and lends it by `&mut` to every task poll through [`TaskContext::ext`],
/// so no future or context keeps a handle to the engine. Wait futures such as [`RenderFuture`] and [`EventFuture`]
/// hold only identifiers and register through it; reads such as [`Read`] and [`With`] complete from it on their first
/// poll. Element declarations, edits, and structural changes write through it into [`Self::tree`] and
/// [`Self::runtime`] directly; see [`direct`].
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
	/// The live elements, which declarations and edits write into while their task is polled.
	pub(super) tree: RetainedTree,
	/// Receives the removals that dropped mounts send; see [`MountedComponentFuture`].
	pub(super) commands: Receiver<UiCommand>,
	/// Handed to every mount as it starts, so its destructor can send its removal.
	pub(super) sender: Sender<UiCommand>,
}

impl<C: 'static> UiPoll<C> {
	/// Returns the poll state the engine lent to `cx`.
	pub(crate) fn from_context<'a>(cx: &'a mut TaskContext<'_>) -> &'a mut Self {
		cx.ext().downcast_mut::<Self>().expect(
			"A UI future was polled outside a UI engine. The most likely cause is polling a UI future from another executor instead of awaiting it inside a mounted component.",
		)
	}

	/// Applies the removals dropped mounts sent so far, including the ones that applying them sends, so a direct
	/// write lands after the changes that happened before it.
	pub(super) fn apply_commands(&mut self) {
		while let Ok(command) = self.commands.try_recv() {
			apply(command, &mut self.runtime, &mut self.tree);
		}
	}

	/// Applies one structural change in order with the removals of dropped mounts.
	pub(super) fn apply(&mut self, command: UiCommand) {
		self.apply_commands();
		apply(command, &mut self.runtime, &mut self.tree);
		// A removal drops the futures of the tasks it ends, and dropped mounts among them send their own removal.
		self.apply_commands();
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
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c).await;
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
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c).await;
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
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c).await;
			// The frame wait loses the selection, and the task then waits for events alone.
			utils::r#async::select_biased! {
				_ = surface.on(Events::Grabbed) => {},
				_ = surface.render() => {},
			};
			surface.on(Events::DragEnded).await;
		});
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert!(engine.next_tick().is_some(), "a pending frame wait did not request a frame");
		engine.press(UiPoint::new(0.0, 0.0));
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		assert_eq!(engine.next_tick(), None, "a dropped frame wait still requests frames");
	}
}
