//! Animations the engine ticks for the component that owns them; see [`EvaluationContext::animation`].

use std::{
	cell::RefCell,
	rc::{Rc, Weak},
};

use super::*;
use crate::ui::animation::{Animatable, Animation, Motion, Values};

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

/// The `Slot` struct is what a handle and the engine share: the animation, its every-frame callback, and the tasks that
/// wait for it to move or settle.
struct Slot<V: Animatable, M: Motion> {
	animation: Animation<V, M>,
	frame: OnFrame<V>,
	/// Whether the component changed the animation since the engine last wrote it, so a snap or a retarget is drawn even
	/// when it leaves the animation at rest.
	changed: bool,
	/// How many frames the engine wrote, which [`Moved`] compares.
	ticks: u64,
	/// The tasks waiting on [`Moved`] or [`Settled`], woken by the next write.
	waiting: SmallVec<[Waker; 1]>,
}

/// The `Tick` trait is how the engine reaches every animation it ticks whatever its value and motion.
pub(super) trait Tick {
	/// Whether the animation moves or was changed, so the engine needs a frame for it.
	fn wants_tick(&self) -> bool;
	/// Advances the animation by `elapsed`, writes its frame, and wakes the tasks waiting on it.
	fn tick(&mut self, elapsed: crate::time::MediaTime, tree: &mut RetainedTree);
}

impl<V: Animatable, M: Motion> Tick for Slot<V, M> {
	fn wants_tick(&self) -> bool {
		self.changed || !self.animation.is_settled()
	}

	fn tick(&mut self, elapsed: crate::time::MediaTime, tree: &mut RetainedTree) {
		self.changed = false;
		self.animation.step(elapsed);
		(self.frame)(&mut AnimationFrame { tree }, self.animation.values());
		self.ticks += 1;
		for waker in self.waiting.drain(..) {
			waker.wake();
		}
	}
}

/// The `AnimationHandle` struct is an animation the engine moves on every frame until it settles, for a component to
/// head somewhere from any branch of its loop.
///
/// Get one from [`EvaluationContext::animation`]. Its methods borrow it shared, so a `select!` can wait on
/// [`Self::moved`] or [`Self::settled`] while another branch sets a new target. Dropping it stops the animation.
pub struct AnimationHandle<V: Animatable, M: Motion> {
	slot: Rc<RefCell<Slot<V, M>>>,
}

impl<V: Animatable, M: Motion> AnimationHandle<V, M> {
	/// Sets where the animation is headed; see [`Animation::set_target`]. The engine moves it from the next frame on.
	pub fn set_target(&self, target: V) {
		self.update(|animation| animation.set_target(target));
	}

	/// Puts the animation at rest at `value`, which the next frame draws; see [`Animation::snap`].
	pub fn snap(&self, value: V) {
		self.update(|animation| animation.snap(value));
	}

	/// Changes the animation with `change`, such as to set its motion and target together. A change that moves where
	/// it's headed or where it stands is drawn in the next frame; one that changes neither draws nothing.
	pub fn update<T>(&self, change: impl FnOnce(&mut Animation<V, M>) -> T) -> T {
		let mut slot = self.slot.borrow_mut();
		let before = slot.animation.values();
		let result = change(&mut slot.animation);
		slot.changed |= slot.animation.values() != before;
		result
	}

	/// Where the animation is headed.
	pub fn target(&self) -> V {
		self.slot.borrow().animation.target()
	}

	/// Where the animation stands as of the last frame.
	pub fn current(&self) -> V {
		self.slot.borrow().animation.current()
	}

	/// Resolves after the next frame that moves or redraws the animation, and never while it stays at rest, so a loop
	/// can follow it, such as to write where it stands into the application context.
	pub fn moved(&self) -> Moved<'_, V, M> {
		Moved {
			slot: &self.slot,
			seen: None,
			complete: false,
		}
	}

	/// Resolves once the animation stands at its target: at once when it already does. Await it to finish a motion
	/// before what follows, such as removing a panel after it leaves.
	pub fn settled(&self) -> Settled<'_, V, M> {
		Settled {
			slot: &self.slot,
			complete: false,
		}
	}
}

/// The `Moved` struct waits for the next frame that writes an animation; see [`AnimationHandle::moved`].
pub struct Moved<'a, V: Animatable, M: Motion> {
	slot: &'a Rc<RefCell<Slot<V, M>>>,
	/// The frames written when it was first polled.
	seen: Option<u64>,
	complete: bool,
}

impl<V: Animatable, M: Motion> Unpin for Moved<'_, V, M> {}

impl<V: Animatable, M: Motion> Future for Moved<'_, V, M> {
	type Output = V;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<V> {
		if self.complete {
			return Poll::Pending;
		}
		let mut slot = self.slot.borrow_mut();
		let seen = *self.seen.get_or_insert(slot.ticks);
		if slot.ticks > seen {
			let current = slot.animation.current();
			drop(slot);
			self.complete = true;
			return Poll::Ready(current);
		}
		wait(&mut slot.waiting, cx.waker());
		Poll::Pending
	}
}

impl<V: Animatable, M: Motion> FusedFuture for Moved<'_, V, M> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// The `Settled` struct waits for an animation to stand at its target; see [`AnimationHandle::settled`].
pub struct Settled<'a, V: Animatable, M: Motion> {
	slot: &'a Rc<RefCell<Slot<V, M>>>,
	complete: bool,
}

impl<V: Animatable, M: Motion> Unpin for Settled<'_, V, M> {}

impl<V: Animatable, M: Motion> Future for Settled<'_, V, M> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
		if self.complete {
			return Poll::Pending;
		}
		let mut slot = self.slot.borrow_mut();
		if !slot.wants_tick() {
			drop(slot);
			self.complete = true;
			return Poll::Ready(());
		}
		wait(&mut slot.waiting, cx.waker());
		Poll::Pending
	}
}

impl<V: Animatable, M: Motion> FusedFuture for Settled<'_, V, M> {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

/// Adds `waker` to `waiting` unless it would wake a task already there.
fn wait(waiting: &mut SmallVec<[Waker; 1]>, waker: &Waker) {
	if !waiting.iter().any(|waiting| waiting.will_wake(waker)) {
		waiting.push(waker.clone());
	}
}

/// Returns a future that registers `animation` with the engine and resolves to its handle; see
/// [`EvaluationContext::animation`].
pub(super) fn register<C: 'static, V, M>(
	animation: Animation<V, M>,
	frame: impl FnMut(&mut AnimationFrame<'_>, Values<V>) + 'static,
) -> impl Future<Output = AnimationHandle<V, M>>
where
	V: Animatable + 'static,
	M: Motion + 'static,
{
	direct(move |poll: &mut UiPoll<C>| {
		let slot = Rc::new(RefCell::new(Slot {
			animation,
			frame: Box::new(frame),
			changed: false,
			ticks: 0,
			waiting: SmallVec::new(),
		}));
		poll.runtime.animations.push(Rc::downgrade(&slot) as Weak<RefCell<dyn Tick>>);
		AnimationHandle { slot }
	})
}

/// Moves every animation that moves or changed by this frame's time and writes its frame, and forgets the ones whose
/// handle was dropped. The engine calls it as each frame begins, before it polls the tasks the frame woke.
pub(super) fn tick_animations<C: 'static>(core: &mut UiPoll<C>) {
	// A removal sent before this frame lands first, so an animation never edits an element its scope already removed.
	core.apply_commands();
	let elapsed = core.runtime.frame_time;
	let tree = &mut core.tree;
	core.runtime.animations.retain(|animation| {
		let Some(animation) = animation.upgrade() else {
			return false;
		};
		let mut animation = animation.borrow_mut();
		if animation.wants_tick() {
			animation.tick(elapsed, tree);
		}
		true
	});
}

impl Runtime {
	/// Whether an animation moves or was changed, so the next frame must tick it.
	pub(super) fn animating(&self) -> bool {
		self.animations
			.iter()
			.any(|animation| animation.upgrade().is_some_and(|animation| animation.borrow().wants_tick()))
	}
}

#[cfg(test)]
mod tests {
	use std::{cell::RefCell, rc::Rc};

	use super::*;
	use crate::{
		time::MediaTime,
		ui::{Watch, ease_in},
		utils::r#async::{FutureExt as _, select},
	};

	/// What the component under test saw, in order.
	type Log = Rc<RefCell<std::vec::Vec<String>>>;

	/// Mounts a panel and runs `component` on it with a log. The engine's clock steps by hand from the first
	/// evaluation on.
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
	fn an_animation_moves_by_the_clock_until_it_settles_then_asks_for_no_frames() {
		let (mut engine, log) = engine(async |panel, log| {
			let id = panel.id();
			let every = log.clone();
			let opacity = panel
				.animation(Animation::new(0.0, ease_in(0.3)), move |frame, v| {
					push(&every, format!("frame {:.2}", v.current));
					frame.update_container(id, |c| c.opacity(v.current));
				})
				.await;
			opacity.set_target(1.0);
			opacity.settled().await;
			push(&log, "settled".into());
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		for _ in 0..4 {
			frame(&mut engine, 100);
		}
		assert_eq!(*log.borrow(), ["frame 0.11", "frame 0.44", "frame 1.00", "settled"]);
		assert_eq!(engine.next_tick(), None, "a settled animation kept asking for frames");
	}

	#[test]
	fn settled_resolves_at_once_for_an_animation_at_rest() {
		let (mut engine, log) = engine(async |panel, log| {
			let still = panel.animation(Animation::new(0.5, ease_in(1.0)), |_, _| {}).await;
			still.settled().await;
			push(&log, "settled".into());
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		assert_eq!(*log.borrow(), ["settled"]);
	}

	#[test]
	fn moved_waits_while_at_rest_so_a_loop_over_it_sleeps() {
		let go = Watch::new(0.0f32);
		let signal = go.clone();
		let (mut engine, log) = engine(async move |panel, log| {
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
					x.set_target(*subscriber.read());
				}
			}
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		assert!(log.borrow().is_empty(), "an animation at rest woke its loop");
		assert_eq!(engine.next_tick(), None, "an animation at rest asked for frames");
		go.set(1.0);
		for _ in 0..4 {
			frame(&mut engine, 100);
		}
		assert_eq!(*log.borrow(), ["moved 0.25", "moved 1.00"]);
		assert_eq!(engine.next_tick(), None);
	}

	#[test]
	fn a_snap_draws_in_the_next_frame_without_moving() {
		let (mut engine, log) = engine(async |panel, log| {
			let every = log.clone();
			let x = panel
				.animation(Animation::new(0.0, ease_in(1.0)), move |_, v| {
					push(&every, format!("frame {}", v.current))
				})
				.await;
			x.snap(3.0);
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		frame(&mut engine, 100);
		assert_eq!(*log.borrow(), ["frame 3"]);
	}

	#[test]
	fn dropping_the_handle_stops_the_animation() {
		let (mut engine, log) = engine(async |panel, log| {
			let every = log.clone();
			let x = panel
				.animation(Animation::new(0.0, ease_in(1.0)), move |_, v| {
					push(&every, format!("frame {:.1}", v.current))
				})
				.await;
			x.set_target(1.0);
			drop(x);
			std::future::pending::<()>().await;
		});
		frame(&mut engine, 0);
		frame(&mut engine, 100);
		assert!(log.borrow().is_empty(), "a dropped animation kept drawing");
		assert_eq!(engine.next_tick(), None, "a dropped animation kept asking for frames");
	}
}
