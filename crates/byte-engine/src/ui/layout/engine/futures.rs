//! Futures used by mounted UI components.

use std::{
	sync::mpsc::{Receiver, Sender},
	time::Instant,
};

use super::*;
use crate::ui::animation::{Animatable, Animation, Ending, Motion, Values};

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
	/// Whether the application context was lent mutably since polled values last read it; see [`poll_reads`].
	pub(super) written: bool,
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

	/// Returns the application context for a polled read, and registers this task to read it again after the next
	/// write. Reading doesn't count as a write.
	pub(crate) fn read_context(&mut self) -> &C {
		let (task, poll) = self.current();
		self.runtime.wait_for_read(task, poll);
		&self.ctx
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

/// The `With` struct reads or changes the engine's application context while its task is polled.
///
/// Get one from [`Context::with`] and await it. It completes on its first poll with what `read` returned.
pub struct With<C, F> {
	pub(super) read: Option<F>,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C, F> Unpin for With<C, F> {}

impl<C: 'static, F: FnOnce(&mut C) -> T, T> Future for With<C, F> {
	type Output = T;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		let read = self.read.take().expect(
			"A UI context read was polled after it completed. The most likely cause is polling a finished future again.",
		);
		let poll = UiPoll::<C>::from_context(cx);
		// The closure may change the context, so polled values read it again.
		poll.written = true;
		Poll::Ready(read(&mut poll.ctx))
	}
}

/// The `RenderFuture` struct resolves on the first frame that begins after it was first polled, with that frame's
/// time: the time since the frame before, from the engine's clock, which a component steps its own animations by.
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
	type Output = crate::time::MediaTime;

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
			return Poll::Ready(runtime.frame_time);
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

/// The `OnStart` trait is a run's start callback: it writes a container's properties once, as the run begins, from
/// the values the animation serves. See [`Animate::on_start`].
pub trait OnStart<V>: for<'s> FnOnce(Properties<'s, Container>, Values<V>) -> Properties<'s, Container> {}

impl<V, F: for<'s> FnOnce(Properties<'s, Container>, Values<V>) -> Properties<'s, Container>> OnStart<V> for F {}

/// The `OnFrame` trait is a run's every-frame callback: it writes a container's properties from each frame's values.
/// See [`Animate::on_every_frame`].
pub trait OnFrame<V>: for<'s> FnMut(Properties<'s, Container>, Values<V>) -> Properties<'s, Container> {}

impl<V, F: for<'s> FnMut(Properties<'s, Container>, Values<V>) -> Properties<'s, Container>> OnFrame<V> for F {}

/// The `OnEnd` trait is a run's end callback: it writes a container's properties once the run settles or is
/// interrupted. It must own what it uses, since an interrupted run hands it to the engine to apply later. See
/// [`Animate::on_end`].
pub trait OnEnd<V>: for<'s> FnOnce(Properties<'s, Container>, Values<V>, Ending) -> Properties<'s, Container> + 'static {}

impl<V, F: for<'s> FnOnce(Properties<'s, Container>, Values<V>, Ending) -> Properties<'s, Container> + 'static> OnEnd<V>
	for F
{
}

/// The type a run's start or every-frame callback has while none was given; it's never called.
pub type NoStart<V> = for<'s> fn(Properties<'s, Container>, Values<V>) -> Properties<'s, Container>;
/// See [`NoStart`].
pub type NoEnd<V> = for<'s> fn(Properties<'s, Container>, Values<V>, Ending) -> Properties<'s, Container>;

/// The `Animate` struct is one run of an [`Animation`] on a container: it serves the animation's values to the
/// callbacks you chain on, one frame at a time, until the animation settles.
///
/// Get one from [`EvaluationContext::animate`], chain [`Self::on_start`], [`Self::on_every_frame`], and
/// [`Self::on_end`], then await it. It holds only the container's id, so it can wait in a `select!` beside other waits
/// on the same element; dropping it interrupts the run, and the animation keeps its state for the next one.
pub struct Animate<'a, C, V, M, S = NoStart<V>, F = NoStart<V>, E = NoEnd<V>>
where
	V: Animatable + 'static,
	M: Motion,
	E: OnEnd<V>,
{
	pub(super) target: Id,
	pub(super) animation: &'a mut Animation<V, M>,
	pub(super) start: Option<S>,
	pub(super) frame: Option<F>,
	/// The frame wait between two frames of the run.
	pub(super) render: Option<RenderFuture<C>>,
	pub(super) started: bool,
	pub(super) complete: bool,
	pub(super) end: EndGuard<V, E>,
}

/// The `EndGuard` struct keeps a run's end callback and, once the run has written its first frame, hands it to the
/// engine if the run is dropped before it settles, since a destructor can't edit the tree itself.
pub(super) struct EndGuard<V: 'static, E: OnEnd<V>> {
	target: Id,
	end: Option<E>,
	/// The engine's command channel, taken on the first poll of a run with an end.
	sender: Option<Sender<UiCommand>>,
	/// The values the run wrote last, which an interrupted end receives.
	written: Option<Values<V>>,
}

impl<V: 'static, E: OnEnd<V>> EndGuard<V, E> {
	pub(super) fn new(target: Id, end: Option<E>) -> Self {
		Self {
			target,
			end,
			sender: None,
			written: None,
		}
	}
}

impl<V: 'static, E: OnEnd<V>> Drop for EndGuard<V, E> {
	fn drop(&mut self) {
		let (Some(end), Some(sender), Some(values)) = (self.end.take(), self.sender.take(), self.written.take()) else {
			return;
		};
		// The engine applies it before the next change any component makes. Once the engine is gone, nothing is left
		// to edit, so a failed send is ignored.
		let _ = sender.send(UiCommand::Edit {
			target: self.target,
			edit: Box::new(move |c| end(c, values, Ending::Interrupted)),
		});
	}
}

// The run never pins its fields, so it can move freely and wait in a `select!` without being pinned first.
impl<C, V: Animatable + 'static, M: Motion, S, F, E: OnEnd<V>> Unpin for Animate<'_, C, V, M, S, F, E> {}

impl<'a, C, V: Animatable + 'static, M: Motion, S, F, E: OnEnd<V>> Animate<'a, C, V, M, S, F, E> {
	/// Calls `start` once, as the run begins, in the same edit as its first frame: the place for what the target
	/// decides at once, such as a logical size or a position.
	pub fn on_start<S2: OnStart<V>>(self, start: S2) -> Animate<'a, C, V, M, S2, F, E> {
		Animate {
			target: self.target,
			animation: self.animation,
			start: Some(start),
			frame: self.frame,
			render: self.render,
			started: self.started,
			complete: self.complete,
			end: self.end,
		}
	}

	/// Calls `frame` on the run's first frame and on each one after, until the animation settles: the place for what
	/// the current value drives, such as a display size, a transform, or an opacity.
	pub fn on_every_frame<F2: OnFrame<V>>(self, frame: F2) -> Animate<'a, C, V, M, S, F2, E> {
		Animate {
			target: self.target,
			animation: self.animation,
			start: self.start,
			frame: Some(frame),
			render: self.render,
			started: self.started,
			complete: self.complete,
			end: self.end,
		}
	}

	/// Calls `end` once the run ends: in the edit of its last frame when the animation settles, or, when the run is
	/// dropped first, before the next change any component makes, with the values it wrote last. `Ending` says which.
	pub fn on_end<E2: OnEnd<V>>(self, end: E2) -> Animate<'a, C, V, M, S, F, E2> {
		// The guard being replaced holds no end to send, since the run hasn't started.
		Animate {
			target: self.target,
			animation: self.animation,
			start: self.start,
			frame: self.frame,
			render: self.render,
			started: self.started,
			complete: self.complete,
			end: EndGuard::new(self.target, Some(end)),
		}
	}
}

impl<C: 'static, V, M, S, F, E> Animate<'_, C, V, M, S, F, E>
where
	V: Animatable + 'static,
	M: Motion,
	S: OnStart<V>,
	F: OnFrame<V>,
	E: OnEnd<V>,
{
	/// Writes one frame of the run in one edit: the start on the first, then the every-frame callback, then the end once
	/// the animation settled. Returns whether it settled.
	fn write(&mut self, poll: &mut UiPoll<C>) -> bool {
		let values = self.animation.values();
		let settled = self.animation.is_settled();
		let start = self.start.take();
		let frame = self.frame.as_mut();
		let end = if settled { self.end.end.take() } else { None };
		let edited = poll.tree.update_element(self.target, move |element, spares| {
			let Some(mut c) = super::properties::properties::<Container>(element, spares) else {
				return false;
			};
			if let Some(start) = start {
				c = start(c, values);
			}
			if let Some(frame) = frame {
				c = frame(c, values);
			}
			if let Some(end) = end {
				c = end(c, values, Ending::Settled);
			}
			let _ = c;
			true
		});
		match edited {
			Some(true) => {}
			Some(false) => log::error!(
				"An animation run skipped a frame because its element isn't a container. The most likely cause is calling `animate` on the context of a text or an image. Run it on a container's context instead."
			),
			// A run on a removed element still plays out, so code that awaits it carries on.
			None => log::debug!("An animation run skipped a frame because its element was removed."),
		}
		self.end.written = Some(values);
		settled
	}
}

impl<C: 'static, V, M, S, F, E> Future for Animate<'_, C, V, M, S, F, E>
where
	V: Animatable + 'static,
	M: Motion,
	S: OnStart<V>,
	F: OnFrame<V>,
	E: OnEnd<V>,
{
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		let this = self.get_mut();
		if this.complete {
			return Poll::Pending;
		}
		if !this.started {
			this.started = true;
			let poll = UiPoll::<C>::from_context(cx);
			// The end of a run interrupted earlier lands before this run's start.
			poll.apply_commands();
			if this.end.end.is_some() {
				this.end.sender = Some(poll.sender.clone());
			}
			if this.write(poll) {
				this.complete = true;
				return Poll::Ready(());
			}
		}
		loop {
			let render = this.render.get_or_insert_with(|| RenderFuture {
				wait: None,
				complete: false,
				ctx: PhantomData,
			});
			let Poll::Ready(frame_time) = Pin::new(render).poll(cx) else {
				return Poll::Pending;
			};
			this.render = None;
			let poll = UiPoll::<C>::from_context(cx);
			poll.apply_commands();
			this.animation.step(frame_time);
			if this.write(poll) {
				this.complete = true;
				return Poll::Ready(());
			}
		}
	}
}

impl<C: 'static, V, M, S, F, E> FusedFuture for Animate<'_, C, V, M, S, F, E>
where
	V: Animatable + 'static,
	M: Motion,
	S: OnStart<V>,
	F: OnFrame<V>,
	E: OnEnd<V>,
{
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// The `SizeChangeFuture` struct lets a component follow a size the engine decides, such as a fit container's, by
/// resolving with the element's logical size once it differs from the size the component last knew.
///
/// Get one from [`Context::logical_size_change`] and await it, usually in a `select!` with [`Context::render`] while
/// a driver animates the display size. It checks on every poll and after every layout, so a size that changed before
/// the first poll resolves it at once. A wait on an element that isn't laid out stays pending.
pub struct SizeChangeFuture<C = ()> {
	pub(super) target: Id,
	pub(super) known: Size,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for SizeChangeFuture<C> {}

impl<C: 'static> Future for SizeChangeFuture<C> {
	type Output = Size;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		let runtime = &mut access.runtime;
		if let Some(published) = runtime.geometry.get(&self.target)
			&& published.size != self.known
		{
			self.complete = true;
			return Poll::Ready(published.size);
		}
		runtime.wait_for_size(task, self.target, self.known, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for SizeChangeFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// The `FocusChangeFuture` struct lets a component follow whether its element keeps the focus, such as a text field
/// that ends typing once a press elsewhere takes it, without checking the pointer every frame.
///
/// Get one from [`Context::focus_change`] and await it beside the field's inputs. It checks on every poll and after
/// every focus change, so a change before the first poll resolves it at once.
pub struct FocusChangeFuture<C = ()> {
	pub(super) target: Id,
	pub(super) known: bool,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

impl<C> Unpin for FocusChangeFuture<C> {}

impl<C: 'static> Future for FocusChangeFuture<C> {
	type Output = bool;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		let runtime = &mut access.runtime;
		let focused = runtime.holds_focus(self.target);
		if focused != self.known {
			self.complete = true;
			return Poll::Ready(focused);
		}
		runtime.wait_for_focus(task, poll);
		Poll::Pending
	}
}

impl<C: 'static> FusedFuture for FocusChangeFuture<C> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// The `InputFuture` struct resolves with the next input of one kind that reaches its element, such as a click.
///
/// Get one from [`ContainerContext::on`], [`ContainerContext::on_key`], or [`ContainerContext::on_text_edit`] and
/// await it, usually inside a loop or a `select!`. A wait counts only while its task keeps polling it, so an input
/// that arrives after the task stops awaiting the future is not kept for it.
///
/// [`ContainerContext::on`]: crate::ui::ContainerContext::on
/// [`ContainerContext::on_key`]: crate::ui::ContainerContext::on_key
/// [`ContainerContext::on_text_edit`]: crate::ui::ContainerContext::on_text_edit
pub struct InputFuture<C, E: UiInput> {
	pub(super) target: Id,
	pub(super) filter: E::Filter,
	pub(super) complete: bool,
	pub(super) ctx: PhantomData<fn() -> C>,
}

/// Resolves with the next pointer [`UiEvent`] of one [`Events`] kind.
pub type EventFuture<C = ()> = InputFuture<C, UiEvent>;

/// Resolves with the next press of one [`Key`] while the element is focused.
pub type KeyFuture<C = ()> = InputFuture<C, UiKeyEvent>;

/// Resolves with the next [`TextEdit`] made while the element is focused.
pub type TextEditFuture<C = ()> = InputFuture<C, UiTextEditEvent>;

impl<C, E: UiInput> InputFuture<C, E> {
	pub(super) fn new(target: Id, filter: E::Filter) -> Self {
		Self {
			target,
			filter,
			complete: false,
			ctx: PhantomData,
		}
	}
}

impl<C, E: UiInput> Unpin for InputFuture<C, E> {}

impl<C: 'static, E: Routed> Future for InputFuture<C, E> {
	type Output = E;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}
		let access = UiPoll::<C>::from_context(cx);
		let (task, poll) = access.current();
		if let Some(input) = access.runtime.take_event::<E>(task, self.target, self.filter) {
			self.complete = true;
			return Poll::Ready(input);
		}
		access.runtime.wait_for_event::<E>(task, self.target, self.filter, poll);
		Poll::Pending
	}
}

impl<C: 'static, E: Routed> FusedFuture for InputFuture<C, E> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

#[cfg(test)]
mod frame_wait_tests {
	use super::*;
	use crate::ui::layout::context::ContainerContext as _;

	/// Mounts a surface that selects over its drag events and frames, events first, and counts each.
	fn selecting_surface() -> Engine<(u32, u32)> {
		let mut engine = Engine::with_context((0, 0));
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c.hit_testable(true)).await;
			loop {
				let dragged = utils::r#async::select_biased! {
					_ = surface.on(Events::Dragged) => true,
					_ = surface.render() => false,
				};
				ctx.with(|(events, frames)| if dragged { *events += 1 } else { *frames += 1 })
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
		let (events, frames) = *engine.ctx();
		assert!(events >= 15, "the drag did not reach the surface every frame: {events}");
		assert_eq!(frames, 20, "continuous events starved the frame wait");
	}

	#[test]
	fn a_frame_wait_after_idle_frames_still_waits_for_the_next_frame() {
		let mut engine = Engine::with_context(0);
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c.hit_testable(true)).await;
			loop {
				// The frame wait loses once and is abandoned while the task idles on events alone.
				utils::r#async::select_biased! {
					_ = surface.on(Events::Grabbed) => {},
					_ = surface.render() => {},
				};
				surface.on(Events::DragEnded).await;
				surface.render().await;
				ctx.with(|ticks| *ticks += 1).await;
			}
		});
		let frame = |engine: &mut Engine<u32>| {
			engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
		};
		frame(&mut engine);
		engine.press(UiPoint::new(0.0, 0.0));
		for _ in 0..10 {
			frame(&mut engine);
		}
		engine.release(UiPoint::new(0.0, 0.0));
		frame(&mut engine);
		assert_eq!(*engine.ctx(), 0, "a stale frame wait resolved without a new frame");
		frame(&mut engine);
		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn a_frame_wait_its_task_stopped_polling_stops_requesting_frames() {
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut surface = ctx.element("surface").container(|c| c.hit_testable(true)).await;
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

#[cfg(test)]
mod size_change_tests {
	use super::*;
	use crate::ui::{Watch, layout::Sizing, transform::Transform};

	/// Mounts a fit panel around a label. Each write to `edits` applies one edit to the panel or its label, then the
	/// component waits for the panel's logical size to change and records each size it sees in the context.
	fn fit_panel(edits: &Watch<u32>, edit: fn(u32) -> Edit) -> Engine<std::vec::Vec<Size>> {
		let mut engine = Engine::with_context(std::vec::Vec::new());
		let edits = edits.clone();
		engine.mount(async move |ctx| {
			let mut subscriber = edits.subscribe();
			let mut panel = ctx.element("panel").container(|c| c.size(Sizing::fit())).await;
			let mut label = panel.element("label").text("ab", |t| t).await;
			let mut known = Size::new(0, 0);
			loop {
				known = panel.logical_size_change(known).await;
				ctx.with(|seen| seen.push(known)).await;
				let step = *subscriber.changed().await;
				match edit(step) {
					Edit::Text(content) => label.update_text(|t| t.content(content)).await,
					Edit::Display(width) => panel.update_container(|c| c.display_width(Sizing::pixels(width))).await,
					Edit::Scale(scale) => {
						panel
							.update_container(|c| c.transform(Transform::default().scale(scale)))
							.await
					}
				}
			}
		});
		engine
	}

	enum Edit {
		Text(&'static str),
		Display(f32),
		Scale(f32),
	}

	fn evaluate(engine: &mut Engine<std::vec::Vec<Size>>) {
		engine.evaluate(Size::new(400, 300), &bumpalo::Bump::new());
	}

	#[test]
	fn a_size_change_completes_in_the_evaluation_whose_layout_changed_the_size() {
		let edits = Watch::new(0);
		let mut engine = fit_panel(&edits, |_| Edit::Text("a much longer label"));
		evaluate(&mut engine);
		assert_eq!(
			engine.ctx().len(),
			1,
			"the first layout did not complete a wait given an empty size"
		);
		let first = engine.ctx()[0];
		assert!(first.x() > 0.0 && first.y() > 0.0);

		edits.set(1);
		evaluate(&mut engine);
		assert_eq!(
			engine.ctx().len(),
			2,
			"the text edit's layout did not complete the wait in the same evaluation"
		);
		assert!(engine.ctx()[1].x() > first.x());
	}

	#[test]
	fn display_and_transform_edits_never_complete_a_size_change() {
		for edit in [|_| Edit::Display(10.0), |_| Edit::Scale(2.0)] as [fn(u32) -> Edit; 2] {
			let edits = Watch::new(0);
			let mut engine = fit_panel(&edits, edit);
			evaluate(&mut engine);
			edits.set(1);
			evaluate(&mut engine);
			evaluate(&mut engine);
			assert_eq!(
				engine.ctx().len(),
				1,
				"an edit that leaves the logical size alone completed the wait"
			);
			assert_eq!(engine.next_tick(), None, "a pending size wait requested frames");
		}
	}
}

#[cfg(test)]
mod animation_run_tests {
	use std::{cell::RefCell, rc::Rc};

	use super::*;
	use crate::{
		time::MediaTime,
		ui::{Watch, ease_in, ease_out},
		utils::r#async::{FutureExt as _, select},
	};

	/// What the runs under test called, in order.
	type Log = Rc<RefCell<std::vec::Vec<String>>>;

	/// Mounts a panel and runs `component` on it, with a log the run's callbacks write to. The engine's clock steps by
	/// hand from the first evaluation on.
	fn engine(component: impl AsyncFnOnce(&mut EvaluationContext<()>, Log) + 'static) -> (Engine, Log) {
		let log = Log::default();
		let mut engine = Engine::new();
		engine.advance_clock(MediaTime::ZERO);
		let shared = log.clone();
		engine.mount(async move |ctx| {
			let mut panel = ctx.element("panel").container(|c| c).await;
			component(&mut panel, shared).await;
		});
		(engine, log)
	}

	/// Advances the clock by `millis`, then evaluates.
	fn frame(engine: &mut Engine, millis: i64) {
		engine.advance_clock(MediaTime::from_millis(millis));
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
	}

	fn push(log: &Log, entry: String) {
		log.borrow_mut().push(entry);
	}

	#[test]
	fn a_run_starts_once_steps_by_the_clock_and_ends_settled_in_its_last_frame() {
		let (mut engine, log) = engine(async |panel, log| {
			let mut opacity = Animation::new(0.0, ease_in(0.3));
			opacity.set_target(1.0);
			let (start, every, end) = (log.clone(), log.clone(), log.clone());
			panel
				.animate(&mut opacity)
				.on_start(move |c, v| {
					push(&start, format!("start {} {}", v.target, v.current));
					c
				})
				.on_every_frame(move |c, v| {
					push(&every, format!("frame {:.2}", v.current));
					c.opacity(v.current)
				})
				.on_end(move |c, v, ending| {
					push(&end, format!("end {ending:?} {}", v.current));
					c
				})
				.await;
			push(&log, "done".into());
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		frame(&mut engine, 100);
		frame(&mut engine, 100);
		frame(&mut engine, 100);
		assert_eq!(
			*log.borrow(),
			[
				"start 1 0",
				"frame 0.00",
				"frame 0.11",
				"frame 0.44",
				"frame 1.00",
				"end Settled 1",
				"done",
			]
		);
	}

	#[test]
	fn a_run_of_a_settled_animation_completes_on_its_first_poll() {
		let (mut engine, log) = engine(async |panel, log| {
			let mut still = Animation::new(0.5, ease_out(1.0));
			let (every, end) = (log.clone(), log.clone());
			panel
				.animate(&mut still)
				.on_every_frame(move |c, v| {
					push(&every, format!("frame {}", v.current));
					c
				})
				.on_end(move |c, _, ending| {
					push(&end, format!("end {ending:?}"));
					c
				})
				.await;
			push(&log, "done".into());
		});
		frame(&mut engine, 0);
		assert_eq!(*log.borrow(), ["frame 0.5", "end Settled", "done"]);
		assert_eq!(engine.next_tick(), None, "a finished run kept asking for frames");
	}

	#[test]
	fn an_interrupted_run_ends_before_the_next_run_starts_from_where_it_stood() {
		let interrupt = Watch::new(0);
		let signal = interrupt.clone();
		let (mut engine, log) = engine(async move |panel, log| {
			let mut subscriber = signal.subscribe();
			// The interrupt is the next write, so the value already there counts as seen.
			drop(subscriber.read());
			let mut x = Animation::new(0.0, ease_in(1.0));
			x.set_target(10.0);
			let end = log.clone();
			let run = panel.animate(&mut x).on_end(move |c, v, ending| {
				push(&end, format!("end {ending:?} {}", v.current));
				c
			});
			select! {
				_ = run.fuse() => push(&log, "settled".into()),
				_ = subscriber.changed().fuse() => {},
			}
			let stood = x.current();
			x.set_target(0.0);
			let start = log.clone();
			panel
				.animate(&mut x)
				.on_start(move |c, v| {
					push(&start, format!("start {}", v.current));
					c
				})
				.await;
			push(&log, format!("back from {stood}"));
		});
		frame(&mut engine, 0);
		frame(&mut engine, 500);
		interrupt.set(1);
		frame(&mut engine, 0);
		let entries = log.borrow().clone();
		assert_eq!(entries, ["end Interrupted 2.5", "start 2.5"], "{entries:?}");
		for _ in 0..10 {
			frame(&mut engine, 100);
		}
		assert_eq!(log.borrow().last().map(String::as_str), Some("back from 2.5"));
	}
}

#[cfg(test)]
mod reactive_tests {
	use super::*;
	use crate::ui::Watch;

	/// The application context of these tests: a value components follow, and what each run of a component saw.
	#[derive(Default)]
	struct App {
		value: u32,
		seen: Vec<u32>,
	}

	fn frame<C: 'static>(engine: &mut Engine<C>) {
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
	}

	/// Mounts a component that records every value its polled read of `App::value` hands out.
	fn follower() -> Engine<App> {
		let mut engine = Engine::with_context(App::default());
		engine.mount(async move |ctx| {
			let mut value = ctx.proactive(|app: &App| app.value);
			loop {
				let value = *value.next().await;
				ctx.with(move |app| app.seen.push(value)).await;
			}
		});
		engine
	}

	#[test]
	fn a_polled_value_resolves_at_mount_with_the_current_value() {
		let mut engine = follower();
		engine.ctx_mut().value = 4;
		frame(&mut engine);
		assert_eq!(engine.ctx().seen, [4]);
	}

	#[test]
	fn a_polled_value_resolves_only_when_the_read_differs() {
		let mut engine = follower();
		frame(&mut engine);
		// A write that leaves the value as it was is not a change.
		engine.ctx_mut().value = 0;
		frame(&mut engine);
		engine.ctx_mut().value = 1;
		engine.ctx_mut().value = 2;
		frame(&mut engine);
		assert_eq!(engine.ctx().seen, [0, 2]);
	}

	#[test]
	fn a_component_waiting_only_on_polled_values_asks_for_no_ticks() {
		let mut engine = follower();
		frame(&mut engine);
		assert_eq!(engine.next_tick(), None);
		frame(&mut engine);
		assert_eq!(engine.ctx().seen, [0], "an evaluation without writes ran the component");
	}

	#[test]
	fn a_write_after_an_evaluation_asks_for_one_tick() {
		let mut engine = follower();
		frame(&mut engine);
		engine.ctx_mut().value = 3;
		assert!(engine.next_tick().is_some(), "an unread write asked for no tick");
		frame(&mut engine);
		assert_eq!(engine.ctx().seen, [0, 3]);
		assert_eq!(engine.next_tick(), None, "a read write still asks for ticks");
	}

	#[test]
	fn a_change_one_component_writes_reaches_another_in_the_same_evaluation() {
		let mut engine = Engine::with_context(App::default());
		engine.mount(async move |ctx| {
			ctx.element("writer")
				.component(async |ctx| {
					let mut seen = ctx.proactive(|app: &App| app.seen.len());
					// Each time the follower records a value, the writer moves the value on, up to 3.
					loop {
						let count = *seen.next().await;
						ctx.with(move |app| app.value = app.value.max(count as u32).min(3)).await;
					}
				})
				.await;
			let mut value = ctx.proactive(|app: &App| app.value);
			loop {
				let value = *value.next().await;
				ctx.with(move |app| app.seen.push(value)).await;
			}
		});
		frame(&mut engine);
		assert_eq!(engine.ctx().seen, [0, 1, 2, 3]);
		assert_eq!(engine.next_tick(), None);
	}

	#[test]
	fn a_watch_resolves_at_mount_and_then_once_for_writes_between_frames() {
		let watch = Watch::new(1u32);
		let mut engine = Engine::with_context(Vec::<u32>::new());
		let subscriber = watch.subscribe();
		engine.mount(async move |ctx| {
			let mut value = ctx.reactive(subscriber);
			loop {
				let value = *value.next().await;
				ctx.with(move |seen: &mut Vec<u32>| seen.push(value)).await;
			}
		});
		frame(&mut engine);
		watch.set(2);
		watch.set(3);
		frame(&mut engine);
		assert_eq!(*engine.ctx(), [1, 3]);
	}
}
