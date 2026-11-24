//! Animations the engine ticks for the component that owns them; see [`EvaluationContext::animation`].

use std::{any::Any, marker::PhantomData, sync::mpsc::Sender};

use super::*;
use crate::{
	accessibility::MotionPreference,
	ui::animation::{Animatable, Animation, Motion, Values},
};

/// The `AnimationFrame` struct lets an animation's every-frame callback edit the elements its value drives, any number
/// of them, by id; get an element's id from [`Context::id`].
pub struct AnimationFrame<'a> {
	tree: &'a mut RetainedTree,
}

impl AnimationFrame<'_> {
	/// Edits the container `target` with `setup`, as [`EvaluationContext::update_container`] does. An element removed
	/// since is skipped, so an animation can outlive what it drove.
	pub fn update_container(&mut self, target: Id, setup: impl Setup<Container>) {
		let updated = self.tree.update_element(target, |element, spares| {
			super::properties::properties::<Container>(element, spares)
				.map(setup)
				.is_some()
		});
		match updated {
			Some(true) => {}
			Some(false) => log::error!(
				"An animation skipped an edit because its target isn't a container. The most likely cause is passing the id of a text or an image to `update_container`. Pass a container's id instead."
			),
			None => log::debug!("An animation skipped an edit because its target was removed."),
		}
	}
}

/// The callback an animation calls on each frame it moves; see [`EvaluationContext::animation`].
type OnFrame<V> = Box<dyn FnMut(&mut AnimationFrame<'_>, Values<V>)>;

/// Identifies an animation in [`Runtime::animations`]. Slots are reused, so a handle whose animation was removed
/// finds nothing instead of its successor.
pub(super) type AnimationId = StableVecHandle;

/// The `Animated` struct is one animation the engine ticks, with the bookkeeping every animation needs whatever its
/// value and motion: the frames it wrote and the tasks waiting for its next one.
pub(super) struct Animated {
	animation: Box<dyn Tick>,
	/// How many frames the engine wrote, which [`Moved`] compares.
	ticks: u64,
	/// The tasks waiting on [`Moved`] or [`Settled`], woken by the next write. A task that stopped waiting is woken
	/// once more for nothing, which costs one poll.
	waiting: SmallVec<[TaskId; 1]>,
}

/// The `Slot` struct is the typed part of an animation the engine owns: the animation and its every-frame callback.
struct Slot<V: Animatable, M: Motion> {
	animation: Animation<V, M>,
	frame: OnFrame<V>,
	/// Whether the component changed the animation since the engine last wrote it, so a snap or a retarget is drawn even
	/// when it leaves the animation at rest.
	changed: bool,
}

/// The `Tick` trait is how the engine reaches every animation it ticks whatever its value and motion.
///
/// It extends [`Any`] so a handle's futures, which know the value and motion, downcast to their [`Slot`].
pub(super) trait Tick: Any {
	/// Whether the animation moves or was changed, so the engine needs a frame for it.
	fn wants_tick(&self) -> bool;
	/// Advances the animation by `elapsed`, or to its target under reduced `motion`, and writes its frame.
	fn tick(&mut self, elapsed: crate::time::MediaTime, motion: MotionPreference, tree: &mut RetainedTree);
}

/// Moves `animation` on by `elapsed`, or straight to its target under reduced `motion`, so neither an engine-ticked
/// animation nor a run plays out its motion for a player who asked for less.
pub(super) fn advance<V: Animatable, M: Motion>(
	animation: &mut Animation<V, M>,
	elapsed: crate::time::MediaTime,
	motion: MotionPreference,
) {
	match motion {
		MotionPreference::Full => _ = animation.step(elapsed),
		MotionPreference::Reduced => animation.snap(animation.target()),
	}
}

impl<V: Animatable + 'static, M: Motion + 'static> Tick for Slot<V, M> {
	fn wants_tick(&self) -> bool {
		self.changed || !self.animation.is_settled()
	}

	fn tick(&mut self, elapsed: crate::time::MediaTime, motion: MotionPreference, tree: &mut RetainedTree) {
		self.changed = false;
		advance(&mut self.animation, elapsed, motion);
		(self.frame)(&mut AnimationFrame { tree }, self.animation.values());
	}
}

/// Returns the slot of the animation `id`, which the engine owns, beside its frame count and waiting tasks.
///
/// # Panics
///
/// Panics when the animation was removed, which only its handle's drop does, so a live handle always finds it.
fn slot<'a, V: Animatable + 'static, M: Motion + 'static>(
	runtime: &'a mut Runtime,
	id: AnimationId,
) -> (&'a mut Slot<V, M>, u64, &'a mut SmallVec<[TaskId; 1]>) {
	let Animated {
		animation,
		ticks,
		waiting,
	} = runtime.animations.get_mut(id).expect(
		"An animation handle outlived its animation. The most likely cause is a handle used with an engine other than the one that made it.",
	);
	let animation: &mut dyn Any = &mut **animation;
	let slot = animation
		.downcast_mut::<Slot<V, M>>()
		.expect("an animation handle's value and motion types match its slot");
	(slot, *ticks, waiting)
}

/// The `AnimationHandle` struct is an animation the engine moves on every frame until it settles, for a component to
/// head somewhere from any branch of its loop.
///
/// Get one from [`EvaluationContext::animation`]. The engine owns the animation, so the handle is only its id: its
/// methods are futures that reach the animation while the task is polled, and they borrow the handle shared, so a
/// `select!` can wait on [`Self::moved`] or [`Self::settled`] while another branch sets a new target. Dropping the
/// handle stops the animation.
pub struct AnimationHandle<C, V: Animatable, M: Motion> {
	id: AnimationId,
	/// Carries the removal the handle's drop sends, since a destructor can't reach the engine.
	sender: Sender<UiCommand>,
	types: PhantomData<fn() -> (C, V, M)>,
}

impl<C: 'static, V: Animatable + 'static, M: Motion + 'static> AnimationHandle<C, V, M> {
	/// Sets where the animation is headed; see [`Animation::set_target`]. The engine moves it from the next frame on.
	pub fn set_target(&self, target: V) -> impl Future<Output = ()> {
		self.update(move |animation| animation.set_target(target))
	}

	/// Puts the animation at rest at `value`, which the next frame draws; see [`Animation::snap`].
	pub fn snap(&self, value: V) -> impl Future<Output = ()> {
		self.update(move |animation| animation.snap(value))
	}

	/// Changes the animation with `change`, such as to set its motion and target together. A change that moves where
	/// it's headed or where it stands is drawn in the next frame; one that changes neither draws nothing.
	pub fn update<T>(&self, change: impl FnOnce(&mut Animation<V, M>) -> T) -> impl Future<Output = T> {
		let id = self.id;
		direct(move |poll: &mut UiPoll<C>| {
			let (slot, ..) = slot::<V, M>(&mut poll.runtime, id);
			let before = slot.animation.values();
			let result = change(&mut slot.animation);
			slot.changed |= slot.animation.values() != before;
			result
		})
	}

	/// Where the animation is headed.
	pub fn target(&self) -> impl Future<Output = V> {
		let id = self.id;
		direct(move |poll: &mut UiPoll<C>| slot::<V, M>(&mut poll.runtime, id).0.animation.target())
	}

	/// Where the animation stands as of the last frame.
	pub fn current(&self) -> impl Future<Output = V> {
		let id = self.id;
		direct(move |poll: &mut UiPoll<C>| slot::<V, M>(&mut poll.runtime, id).0.animation.current())
	}

	/// Resolves after the next frame that moves or redraws the animation, and never while it stays at rest, so a loop
	/// can follow it, such as to write where it stands into the application context.
	pub fn moved(&self) -> Moved<C, V, M> {
		Moved {
			id: self.id,
			seen: None,
			complete: false,
			types: PhantomData,
		}
	}

	/// Resolves once the animation stands at its target: at once when it already does. Await it to finish a motion
	/// before what follows, such as removing a panel after it leaves.
	pub fn settled(&self) -> Settled<C, V, M> {
		Settled {
			id: self.id,
			complete: false,
			types: PhantomData,
		}
	}
}

impl<C, V: Animatable, M: Motion> Drop for AnimationHandle<C, V, M> {
	fn drop(&mut self) {
		// Nothing is left to stop once the engine was dropped, so a failed send is ignored.
		let _ = self.sender.send(UiCommand::RemoveAnimation(self.id));
	}
}

/// The `Moved` struct waits for the next frame that writes an animation; see [`AnimationHandle::moved`].
pub struct Moved<C, V: Animatable, M: Motion> {
	id: AnimationId,
	/// The frames written when it was first polled.
	seen: Option<u64>,
	complete: bool,
	types: PhantomData<fn() -> (C, V, M)>,
}

impl<C, V: Animatable, M: Motion> Unpin for Moved<C, V, M> {}

impl<C: 'static, V: Animatable + 'static, M: Motion + 'static> Future for Moved<C, V, M> {
	type Output = V;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<V> {
		if self.complete {
			return Poll::Pending;
		}
		let poll = UiPoll::<C>::from_context(cx);
		let (task, _) = poll.current();
		let (slot, ticks, waiting) = slot::<V, M>(&mut poll.runtime, self.id);
		let seen = *self.seen.get_or_insert(ticks);
		if ticks > seen {
			self.complete = true;
			return Poll::Ready(slot.animation.current());
		}
		wait(waiting, task);
		Poll::Pending
	}
}

impl<C: 'static, V: Animatable + 'static, M: Motion + 'static> FusedFuture for Moved<C, V, M> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// The `Settled` struct waits for an animation to stand at its target; see [`AnimationHandle::settled`].
pub struct Settled<C, V: Animatable, M: Motion> {
	id: AnimationId,
	complete: bool,
	types: PhantomData<fn() -> (C, V, M)>,
}

impl<C, V: Animatable, M: Motion> Unpin for Settled<C, V, M> {}

impl<C: 'static, V: Animatable + 'static, M: Motion + 'static> Future for Settled<C, V, M> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
		if self.complete {
			return Poll::Pending;
		}
		let poll = UiPoll::<C>::from_context(cx);
		let (task, _) = poll.current();
		let (slot, _, waiting) = slot::<V, M>(&mut poll.runtime, self.id);
		if !slot.wants_tick() {
			self.complete = true;
			return Poll::Ready(());
		}
		wait(waiting, task);
		Poll::Pending
	}
}

impl<C: 'static, V: Animatable + 'static, M: Motion + 'static> FusedFuture for Settled<C, V, M> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// Adds `task` to `waiting` unless it is already there.
fn wait(waiting: &mut SmallVec<[TaskId; 1]>, task: TaskId) {
	if !waiting.contains(&task) {
		waiting.push(task);
	}
}

/// Returns a future that registers `animation` with the engine and resolves to its handle; see
/// [`EvaluationContext::animation`].
pub(super) fn register<C: 'static, V, M>(
	animation: Animation<V, M>,
	frame: impl FnMut(&mut AnimationFrame<'_>, Values<V>) + 'static,
) -> impl Future<Output = AnimationHandle<C, V, M>>
where
	V: Animatable + 'static,
	M: Motion + 'static,
{
	direct(move |poll: &mut UiPoll<C>| {
		let id = poll.runtime.animations.push(Animated {
			animation: Box::new(Slot {
				animation,
				frame: Box::new(frame),
				changed: false,
			}),
			ticks: 0,
			waiting: SmallVec::new(),
		});
		AnimationHandle {
			id,
			sender: poll.sender.clone(),
			types: PhantomData,
		}
	})
}

/// Moves every animation that moves or changed by this frame's time, writes its frame, and wakes the tasks waiting on
/// it. The engine calls it as each frame begins, before it polls the tasks the frame woke.
pub(super) fn tick_animations<C: 'static>(core: &mut UiPoll<C>) {
	// A removal sent before this frame lands first, so an animation never edits an element its scope already removed,
	// and a dropped handle's animation stops.
	core.apply_commands();
	let runtime = &mut core.runtime;
	let (elapsed, motion) = (runtime.frame_time, runtime.motion);
	for animated in runtime.animations.iter_mut() {
		if !animated.animation.wants_tick() {
			continue;
		}
		animated.animation.tick(elapsed, motion, &mut core.tree);
		animated.ticks += 1;
		for task in animated.waiting.drain(..) {
			// A task that ended since it waited is gone; generational handles find nothing.
			if let Some(task) = runtime.tasks.get(task) {
				task.wake();
			}
		}
	}
}

impl Runtime {
	/// Whether an animation moves or was changed, so the next frame must tick it.
	pub(super) fn animating(&self) -> bool {
		self.animations.iter().any(|animated| animated.animation.wants_tick())
	}
}

#[cfg(test)]
mod tests {
	use std::sync::mpsc::{Receiver, Sender, channel};

	use super::*;
	use crate::{
		time::MediaTime,
		ui::{Watch, ease_in},
		utils::r#async::{FutureExt as _, select},
	};

	/// What the component under test and its callbacks sent, in order. Callbacks reach only the tree, so they send
	/// their entries down a channel the test drains instead of writing into the engine's context.
	struct Log {
		receiver: Receiver<String>,
		seen: std::vec::Vec<String>,
	}

	impl Log {
		/// Returns every entry sent so far.
		fn entries(&mut self) -> &[String] {
			self.seen.extend(self.receiver.try_iter());
			&self.seen
		}
	}

	/// Mounts a panel and runs `component` on it with the sending end of the log. The engine's clock steps by hand from
	/// the first evaluation on.
	fn engine(component: impl AsyncFnOnce(&mut EvaluationContext<()>, Sender<String>) + 'static) -> (Engine, Log) {
		let (sender, receiver) = channel();
		let mut engine = Engine::new();
		engine.advance_clock(MediaTime::ZERO);
		engine.mount(async move |ctx| {
			let mut panel = ctx.element("panel").container(|c| c).await;
			component(&mut panel, sender).await;
		});
		(
			engine,
			Log {
				receiver,
				seen: std::vec::Vec::new(),
			},
		)
	}

	/// Advances the clock by `millis`, then evaluates.
	fn frame(engine: &mut Engine, millis: i64) {
		engine.advance_clock(MediaTime::from_millis(millis));
		engine.evaluate(Size::new(100, 100), &bumpalo::Bump::new());
	}

	fn push(log: &Sender<String>, entry: String) {
		log.send(entry).expect("the test keeps the log's receiver");
	}

	#[test]
	fn an_animation_moves_by_the_clock_until_it_settles_then_asks_for_no_frames() {
		let (mut engine, mut log) = engine(async |panel, log| {
			let id = panel.id();
			let every = log.clone();
			let opacity = panel
				.animation(Animation::new(0.0, ease_in(0.3)), move |frame, v| {
					push(&every, format!("frame {:.2}", v.current));
					frame.update_container(id, |c| c.opacity(v.current));
				})
				.await;
			opacity.set_target(1.0).await;
			opacity.settled().await;
			push(&log, "settled".into());
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		for _ in 0..4 {
			frame(&mut engine, 100);
		}
		assert_eq!(log.entries(), ["frame 0.11", "frame 0.44", "frame 1.00", "settled"]);
		assert_eq!(engine.next_tick(), None, "a settled animation kept asking for frames");
	}

	#[test]
	fn settled_resolves_at_once_for_an_animation_at_rest() {
		let (mut engine, mut log) = engine(async |panel, log| {
			let still = panel.animation(Animation::new(0.5, ease_in(1.0)), |_, _| {}).await;
			still.settled().await;
			push(&log, "settled".into());
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		assert_eq!(log.entries(), ["settled"]);
	}

	#[test]
	fn moved_waits_while_at_rest_so_a_loop_over_it_sleeps() {
		let go = Watch::new(0.0f32);
		let signal = go.clone();
		let (mut engine, mut log) = engine(async move |panel, log| {
			let mut subscriber = signal.subscribe();
			drop(subscriber.read());
			let x = panel.animation(Animation::new(0.0, ease_in(0.2)), |_, _| {}).await;
			loop {
				let changed = select! {
					at = x.moved() => {
						push(&log, format!("moved {at:.2}"));
						false
					},
					_ = subscriber.changed().fuse() => true,
				};
				if changed {
					let target = *subscriber.read();
					x.set_target(target).await;
				}
			}
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		assert!(log.entries().is_empty(), "an animation at rest woke its loop");
		assert_eq!(engine.next_tick(), None, "an animation at rest asked for frames");
		go.set(1.0);
		for _ in 0..4 {
			frame(&mut engine, 100);
		}
		assert_eq!(log.entries(), ["moved 0.25", "moved 1.00"]);
		assert_eq!(engine.next_tick(), None);
	}

	#[test]
	fn a_snap_draws_in_the_next_frame_without_moving() {
		let (mut engine, mut log) = engine(async |panel, log| {
			let every = log.clone();
			let x = panel
				.animation(Animation::new(0.0, ease_in(1.0)), move |_, v| {
					push(&every, format!("frame {}", v.current))
				})
				.await;
			x.snap(3.0).await;
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		frame(&mut engine, 100);
		assert_eq!(log.entries(), ["frame 3"]);
	}

	#[test]
	fn under_reduced_motion_an_animation_draws_its_target_in_its_first_frame_and_settles() {
		let (mut engine, mut log) = engine(async |panel, log| {
			let every = log.clone();
			let x = panel
				.animation(Animation::new(0.0, ease_in(1.0)), move |_, v| {
					push(&every, format!("frame {}", v.current))
				})
				.await;
			x.set_target(1.0).await;
			x.settled().await;
			push(&log, "settled".into());
			std::future::pending::<()>().await;
		});
		engine.set_motion(crate::accessibility::MotionPreference::Reduced);
		frame(&mut engine, 0);
		frame(&mut engine, 10);
		assert_eq!(log.entries(), ["frame 1", "settled"]);
		assert_eq!(engine.next_tick(), None, "a reduced animation kept asking for frames");
	}

	#[test]
	fn dropping_the_handle_stops_the_animation() {
		let (mut engine, mut log) = engine(async |panel, log| {
			let every = log.clone();
			let x = panel
				.animation(Animation::new(0.0, ease_in(1.0)), move |_, v| {
					push(&every, format!("frame {:.1}", v.current))
				})
				.await;
			x.set_target(1.0).await;
			drop(x);
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		assert!(log.entries().is_empty(), "a dropped animation kept drawing");
		assert_eq!(engine.next_tick(), None, "a dropped animation kept asking for frames");
	}
}
