//! Animations a component owns and runs on its elements.
//!
//! An [`Animation`] is a value your component moves over time: where it's headed, where it stands, and how each of
//! its numbers moves, as a [`Motion`] such as a [`Easing`], a [`Spring`], or a [`Smoothing`] decides. The engine keeps
//! none of that state, so an animation lives exactly as long as the component that owns it, and a motion you interrupt
//! continues from where it stood the next time you run it.
//!
//! Run one on a container with [`crate::ui::EvaluationContext::animate`], which serves each frame's [`Values`] to
//! callbacks that write whichever properties they drive. To drive something that isn't one element, step it yourself
//! with [`Animation::step`].

use smallvec::SmallVec;
use utils::RGBA;

use crate::{time::MediaTime, ui::flow::Offset, ui::flow::Size};

/// The longest step a spring takes in one frame, in seconds, so a stalled frame can't make it unstable.
const MAX_SPRING_STEP: f32 = 1.0 / 30.0;
/// How close to its target a lane must stand, and how slowly a spring must move, to count as settled.
const SETTLE_EPSILON: f32 = 0.001;

/// The `Animatable` trait lets a value be animated: it splits the value into lanes, numbers that each move on their
/// own, and joins lanes back into a value.
///
/// Implement it for a value of your own, such as a pose with several parts, to animate all of it with one
/// [`Animation`] while a spring keeps a speed for each part.
pub trait Animatable: Copy + PartialEq {
	/// How many numbers the value has.
	const LANES: usize;
	/// Returns the number at `index`, below [`Self::LANES`].
	fn lane(&self, index: usize) -> f32;
	/// Builds a value from its numbers, which `lane` returns by index.
	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self;
}

impl Animatable for f32 {
	const LANES: usize = 1;

	fn lane(&self, _: usize) -> f32 {
		*self
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		lane(0)
	}
}

impl<const N: usize> Animatable for [f32; N] {
	const LANES: usize = N;

	fn lane(&self, index: usize) -> f32 {
		self[index]
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		std::array::from_fn(lane)
	}
}

impl Animatable for Size {
	const LANES: usize = 2;

	fn lane(&self, index: usize) -> f32 {
		[self.x(), self.y()][index]
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		Size::new(lane(0), lane(1))
	}
}

impl Animatable for Offset {
	const LANES: usize = 2;

	fn lane(&self, index: usize) -> f32 {
		[self.x(), self.y()][index]
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		Offset::new(lane(0), lane(1))
	}
}

impl Animatable for RGBA {
	const LANES: usize = 4;

	fn lane(&self, index: usize) -> f32 {
		[self.r, self.g, self.b, self.a][index]
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		RGBA::new(lane(0), lane(1), lane(2), lane(3))
	}
}

/// A pair animates the lanes of its first value, then those of its second.
impl<A: Animatable, B: Animatable> Animatable for (A, B) {
	const LANES: usize = A::LANES + B::LANES;

	fn lane(&self, index: usize) -> f32 {
		if index < A::LANES {
			self.0.lane(index)
		} else {
			self.1.lane(index - A::LANES)
		}
	}

	fn from_lanes(lane: impl Fn(usize) -> f32) -> Self {
		(A::from_lanes(&lane), B::from_lanes(|index| lane(index + A::LANES)))
	}
}

/// The `Motion` trait describes how one lane of an [`Animation`] moves toward its target, such as along a [`Easing`] or
/// as a [`Spring`].
///
/// A motion is only a description; [`Self::State`] holds what one lane needs to move, so one motion serves every lane.
/// Implement it for a motion of your own and use it with [`Animation::new`].
pub trait Motion: Clone {
	/// What one lane keeps between frames, such as its value and speed.
	type State: Copy;
	/// A lane at rest at `value`.
	fn rest(&self, value: f32) -> Self::State;
	/// Where the lane stands.
	fn value(state: &Self::State) -> f32;
	/// Starts the lane toward `target` from where it stands, keeping what carries over, such as a spring's speed.
	fn retarget(&self, state: &mut Self::State, target: f32);
	/// Moves the lane toward `target` by `seconds`.
	fn step(&self, state: &mut Self::State, target: f32, seconds: f32);
	/// Whether the lane has reached `target` and stopped.
	fn is_settled(&self, state: &Self::State, target: f32) -> bool;
}

/// The shape of a curve: progress from 0 to 1 in, eased progress out, given the curve's parameter.
type Shape = fn(f32, f32) -> f32;

/// The `Easing` struct moves each lane from where it stood when its target last changed to the target, over a fixed
/// duration, along an easing shape. Get one from [`ease_out`], [`emphasized_out`], or the other easing functions.
#[derive(Debug, Clone, Copy)]
pub struct Easing {
	duration: f32,
	shape: Shape,
	/// A value the shape takes, such as [`back_out`]'s overshoot.
	parameter: f32,
}

/// One lane moving along a [`Easing`].
#[derive(Debug, Clone, Copy)]
pub struct EasingState {
	from: f32,
	value: f32,
	elapsed: f32,
}

impl Easing {
	fn new(duration: f32, shape: Shape, parameter: f32) -> Self {
		Self {
			duration: duration.max(0.0),
			shape,
			parameter,
		}
	}

	fn progress(&self, elapsed: f32) -> f32 {
		if self.duration == 0.0 {
			1.0
		} else {
			(elapsed / self.duration).clamp(0.0, 1.0)
		}
	}
}

impl Motion for Easing {
	type State = EasingState;

	fn rest(&self, value: f32) -> EasingState {
		EasingState {
			from: value,
			value,
			elapsed: f32::INFINITY,
		}
	}

	fn value(state: &EasingState) -> f32 {
		state.value
	}

	fn retarget(&self, state: &mut EasingState, target: f32) {
		// A lane already at its target stays at rest, so a new motion or target doesn't make it wait out a duration.
		*state = if (state.value - target).abs() <= SETTLE_EPSILON {
			self.rest(target)
		} else {
			EasingState {
				from: state.value,
				value: state.value,
				elapsed: 0.0,
			}
		};
	}

	fn step(&self, state: &mut EasingState, target: f32, seconds: f32) {
		state.elapsed += seconds.max(0.0);
		let eased = (self.shape)(self.progress(state.elapsed), self.parameter);
		state.value = state.from + (target - state.from) * eased;
	}

	fn is_settled(&self, state: &EasingState, target: f32) -> bool {
		// A zero duration has passed before its first step, so the lane must also stand at its target.
		state.elapsed >= self.duration && (state.value - target).abs() <= SETTLE_EPSILON
	}
}

fn ease_in_shape(t: f32, _: f32) -> f32 {
	t * t
}

fn ease_out_shape(t: f32, _: f32) -> f32 {
	1.0 - (1.0 - t) * (1.0 - t)
}

fn ease_out_cubic_shape(t: f32, _: f32) -> f32 {
	1.0 - (1.0 - t).powi(3)
}

fn ease_out_quart_shape(t: f32, _: f32) -> f32 {
	1.0 - (1.0 - t).powi(4)
}

fn emphasized_out_shape(t: f32, _: f32) -> f32 {
	1.0 - (1.0 - t).powi(5)
}

fn ease_in_out_shape(t: f32, _: f32) -> f32 {
	if t < 0.5 {
		2.0 * t * t
	} else {
		1.0 - (-2.0 * t + 2.0).powi(2) * 0.5
	}
}

/// The back-out shape: past the end near the finish, by `overshoot`, then back to it.
fn back_out_shape(t: f32, overshoot: f32) -> f32 {
	let t = t - 1.0;
	1.0 + t * t * ((overshoot + 1.0) * t + overshoot)
}

/// A quadratic curve that starts slowly, over `duration` seconds.
pub fn ease_in(duration: f32) -> Easing {
	Easing::new(duration, ease_in_shape, 0.0)
}

/// A quadratic curve that slows into place, over `duration` seconds.
pub fn ease_out(duration: f32) -> Easing {
	Easing::new(duration, ease_out_shape, 0.0)
}

/// A cubic curve that slows into place, over `duration` seconds.
pub fn ease_out_cubic(duration: f32) -> Easing {
	Easing::new(duration, ease_out_cubic_shape, 0.0)
}

/// A quartic curve that slows into place, over `duration` seconds.
pub fn ease_out_quart(duration: f32) -> Easing {
	Easing::new(duration, ease_out_quart_shape, 0.0)
}

/// A quintic curve that moves most of the way at once and lands gently, over `duration` seconds.
pub fn emphasized_out(duration: f32) -> Easing {
	Easing::new(duration, emphasized_out_shape, 0.0)
}

/// A curve that starts and ends slowly, over `duration` seconds.
pub fn ease_in_out(duration: f32) -> Easing {
	Easing::new(duration, ease_in_out_shape, 0.0)
}

/// A curve that passes its target by `overshoot` and comes back, over `duration` seconds.
pub fn back_out(duration: f32, overshoot: f32) -> Easing {
	Easing::new(duration, back_out_shape, overshoot.max(0.0))
}

/// The `Spring` struct moves each lane as a damped spring, so a lane keeps its speed when its target changes and a
/// motion you interrupt bends toward the new target instead of starting over.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
	mass: f32,
	stiffness: f32,
	damping: f32,
}

/// One lane moving as a [`Spring`].
#[derive(Debug, Clone, Copy)]
pub struct SpringState {
	value: f32,
	velocity: f32,
}

impl Default for Spring {
	fn default() -> Self {
		Self {
			mass: 1.0,
			stiffness: 380.0,
			damping: 16.0,
		}
	}
}

impl Spring {
	pub fn mass(self, mass: f32) -> Self {
		Self { mass, ..self }
	}

	pub fn stiffness(self, stiffness: f32) -> Self {
		Self { stiffness, ..self }
	}

	pub fn damping(self, damping: f32) -> Self {
		Self { damping, ..self }
	}
}

impl Motion for Spring {
	type State = SpringState;

	fn rest(&self, value: f32) -> SpringState {
		SpringState { value, velocity: 0.0 }
	}

	fn value(state: &SpringState) -> f32 {
		state.value
	}

	/// A spring keeps its value and speed, so the motion bends toward the new target.
	fn retarget(&self, _: &mut SpringState, _: f32) {}

	fn step(&self, state: &mut SpringState, target: f32, seconds: f32) {
		let seconds = seconds.min(MAX_SPRING_STEP);
		debug_assert!(
			self.mass.is_finite() && self.mass > 0.0,
			"Spring mass is invalid. The most likely cause is building a spring with `Spring::mass` of 0 or less. Give it a positive mass."
		);
		if seconds <= 0.0 {
			return;
		}
		let force = -self.stiffness * (state.value - target) - self.damping * state.velocity;
		state.velocity += force / self.mass * seconds;
		state.value += state.velocity * seconds;
	}

	fn is_settled(&self, state: &SpringState, target: f32) -> bool {
		(state.value - target).abs() <= SETTLE_EPSILON && state.velocity.abs() <= SETTLE_EPSILON
	}
}

/// The `Smoothing` struct chases a target by closing the same share of the distance left in equal times.
///
/// This is exponential smoothing: a lane moves fast at first, slows as it arrives, and never overshoots. Steps of any
/// size land on the same curve, so it looks the same at every frame rate. Use [`Spring`] when the motion should keep
/// its speed.
#[derive(Debug, Clone, Copy)]
pub struct Smoothing {
	retention: f32,
}

impl Default for Smoothing {
	fn default() -> Self {
		Self { retention: 0.01 }
	}
}

impl Smoothing {
	/// Sets the share of the distance that's still left after one second, between 0 and 1. Lower is snappier.
	pub fn retention(self, retention: f32) -> Self {
		Self {
			retention: retention.clamp(0.0, 1.0),
		}
	}

	/// Sets the time in seconds it takes to close half of the distance left.
	pub fn half_life(self, seconds: f32) -> Self {
		self.retention(0.5f32.powf(1.0 / seconds.max(f32::EPSILON)))
	}
}

impl Motion for Smoothing {
	type State = f32;

	fn rest(&self, value: f32) -> f32 {
		value
	}

	fn value(state: &f32) -> f32 {
		*state
	}

	fn retarget(&self, _: &mut f32, _: f32) {}

	fn step(&self, state: &mut f32, target: f32, seconds: f32) {
		if seconds > 0.0 {
			*state = target + (*state - target) * self.retention.powf(seconds);
		}
	}

	fn is_settled(&self, state: &f32, target: f32) -> bool {
		(*state - target).abs() <= SETTLE_EPSILON
	}
}

/// The `Values` struct is what an animation serves for one frame: where it's headed, for logical properties that
/// place and lay out at once, and where it stands, for display properties that are only seen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values<V> {
	pub target: V,
	pub current: V,
}

/// The `Ending` enum says how a run ended, for its `on_end` callback; see
/// [`crate::ui::EvaluationContext::animate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
	/// The animation reached its target.
	Settled,
	/// The run was dropped before then, such as by losing a `select!`.
	Interrupted,
}

/// The `Animation` struct is motion your component owns: a value's target, and where each of its lanes stands and how
/// it moves. It keeps that state across runs, so a run you interrupt continues from there in the next one.
///
/// Create one with [`Self::new`], set where it's headed with [`Self::set_target`], and run it on a container with
/// [`crate::ui::EvaluationContext::animate`].
pub struct Animation<V: Animatable, M: Motion = Spring> {
	motion: M,
	target: V,
	/// One state per lane of `V`, inline for values of up to four lanes.
	lanes: SmallVec<[M::State; 4]>,
}

impl<V: Animatable, M: Motion> Animation<V, M> {
	/// An animation at rest at `value`, which moves as `motion` says once it has a new target.
	pub fn new(value: V, motion: M) -> Self {
		let lanes = (0..V::LANES).map(|index| motion.rest(value.lane(index))).collect();
		Self {
			motion,
			target: value,
			lanes,
		}
	}

	/// Sets where the animation is headed. Each lane starts toward it from where it stands, keeping what its motion
	/// carries over; the target it already has changes nothing.
	pub fn set_target(&mut self, target: V) {
		if target == self.target {
			return;
		}
		self.target = target;
		for (index, lane) in self.lanes.iter_mut().enumerate() {
			self.motion.retarget(lane, target.lane(index));
		}
	}

	/// Changes how the animation moves from now on, toward the target it has, from where each lane stands.
	pub fn set_motion(&mut self, motion: M) {
		self.motion = motion;
		for (index, lane) in self.lanes.iter_mut().enumerate() {
			self.motion.retarget(lane, self.target.lane(index));
		}
	}

	/// Puts the animation at rest at `value` at once, as its target and where it stands.
	pub fn snap(&mut self, value: V) {
		self.target = value;
		for (index, lane) in self.lanes.iter_mut().enumerate() {
			*lane = self.motion.rest(value.lane(index));
		}
	}

	pub fn target(&self) -> V {
		self.target
	}

	/// Where the animation stands.
	pub fn current(&self) -> V {
		V::from_lanes(|index| M::value(&self.lanes[index]))
	}

	/// Where it's headed and where it stands, as a run serves them.
	pub fn values(&self) -> Values<V> {
		Values {
			target: self.target,
			current: self.current(),
		}
	}

	/// Whether every lane has reached the target and stopped.
	pub fn is_settled(&self) -> bool {
		self.lanes
			.iter()
			.enumerate()
			.all(|(index, lane)| self.motion.is_settled(lane, self.target.lane(index)))
	}

	/// Advances the animation by `elapsed` and returns where it stands. Once every lane settles, it stands exactly at
	/// the target.
	///
	/// A run calls this each frame; call it yourself to drive what isn't one element, such as several elements from
	/// one value.
	pub fn step(&mut self, elapsed: MediaTime) -> V {
		let seconds = elapsed.as_seconds_f32();
		for (index, lane) in self.lanes.iter_mut().enumerate() {
			let target = self.target.lane(index);
			if !self.motion.is_settled(lane, target) {
				self.motion.step(lane, target, seconds);
			}
		}
		if self.is_settled() {
			self.snap(self.target);
		}
		self.current()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Steps `animation` in frames of `frame` seconds until it settles, at most `frames` times, and returns every value.
	fn run<V: Animatable, M: Motion>(animation: &mut Animation<V, M>, frame: f32, frames: usize) -> Vec<V> {
		(0..frames)
			.map_while(|_| (!animation.is_settled()).then(|| animation.step(MediaTime::from_seconds_f32(frame))))
			.collect()
	}

	#[test]
	fn a_new_animation_is_settled_where_it_starts() {
		let animation = Animation::new(3.0, ease_out(1.0));
		assert!(animation.is_settled());
		assert_eq!(animation.values(), Values { target: 3.0, current: 3.0 });
	}

	#[test]
	fn a_curve_reaches_its_target_at_its_duration_and_never_before() {
		let mut animation = Animation::new(0.0, ease_out(0.5));
		animation.set_target(10.0);
		let values = run(&mut animation, 0.1, 100);
		assert_eq!(values.len(), 5);
		assert!(values[..4].iter().all(|value| *value < 10.0));
		assert_eq!(*values.last().unwrap(), 10.0);
		assert!(animation.is_settled());
	}

	#[test]
	fn a_retargeted_curve_starts_over_from_where_it_stands() {
		let mut animation = Animation::new(0.0, ease_in(1.0));
		animation.set_target(10.0);
		let halfway = animation.step(MediaTime::from_seconds_f32(0.5));
		animation.set_target(0.0);
		assert_eq!(animation.current(), halfway, "a new target never jumps");
		let next = animation.step(MediaTime::from_seconds_f32(0.1));
		assert!(next < halfway && next > 0.0);
	}

	#[test]
	fn setting_the_same_target_again_doesnt_restart_a_curve() {
		let mut animation = Animation::new(0.0, ease_out(1.0));
		animation.set_target(10.0);
		animation.step(MediaTime::from_seconds_f32(0.9));
		animation.set_target(10.0);
		assert_eq!(run(&mut animation, 0.1, 100).len(), 1);
	}

	#[test]
	fn a_lane_at_its_target_stays_settled_when_the_motion_changes() {
		let mut animation = Animation::new(5.0, ease_out(1.0));
		animation.set_motion(ease_in(2.0));
		assert!(animation.is_settled());
	}

	#[test]
	fn a_retargeted_spring_keeps_its_speed_and_settles_on_the_new_target() {
		let mut toward = Animation::new(0.0, Spring::default());
		toward.set_target(100.0);
		toward.step(MediaTime::from_millis(16));
		let mut fresh = Animation::new(toward.current(), Spring::default());
		toward.set_target(0.0);
		fresh.set_target(0.0);
		let (kept, rested) = (
			toward.step(MediaTime::from_millis(16)),
			fresh.step(MediaTime::from_millis(16)),
		);
		assert!(kept > rested, "the turned spring still moves outward for a moment");
		run(&mut toward, 0.016, 1000);
		assert_eq!(toward.values(), Values { target: 0.0, current: 0.0 });
	}

	#[test]
	fn a_spring_steps_at_most_a_thirtieth_of_a_second_per_frame() {
		let mut large = Animation::new(0.0, Spring::default());
		let mut capped = Animation::new(0.0, Spring::default());
		large.set_target(1.0);
		capped.set_target(1.0);
		assert_eq!(
			large.step(MediaTime::from_seconds(1)),
			capped.step(MediaTime::from_seconds_f32(MAX_SPRING_STEP))
		);
	}

	#[test]
	fn smoothing_reaches_the_same_value_whatever_the_step_size() {
		let motion = Smoothing::default().half_life(0.1);
		let (mut coarse, mut fine) = (Animation::new(0.0, motion), Animation::new(0.0, motion));
		coarse.set_target(10.0);
		fine.set_target(10.0);
		coarse.step(MediaTime::from_seconds_f32(0.2));
		for _ in 0..20 {
			fine.step(MediaTime::from_seconds_f32(0.01));
		}
		// Two half lives close three quarters of the distance, and never pass the target.
		assert!((coarse.current() - 7.5).abs() < 0.001);
		assert!((fine.current() - coarse.current()).abs() < 0.001);
	}

	#[test]
	fn each_lane_of_a_value_moves_on_its_own() {
		let mut animation = Animation::new((Size::new(0, 0), 1.0), ease_out(1.0));
		animation.set_target((Size::new(10, 0), 1.0));
		let (size, opacity) = animation.step(MediaTime::from_seconds_f32(0.5));
		assert!(size.x() > 0.0 && size.x() < 10.0);
		assert_eq!((size.y(), opacity), (0.0, 1.0), "lanes already at their target stay");
	}

	#[test]
	fn curves_keep_their_endpoints_and_a_zero_duration_arrives_at_once() {
		for curve in [
			ease_in(1.0),
			ease_out(1.0),
			ease_in_out(1.0),
			emphasized_out(1.0),
			back_out(1.0, 1.7),
		] {
			let mut animation = Animation::new(0.0, curve);
			animation.set_target(1.0);
			assert_eq!(animation.step(MediaTime::ZERO), 0.0);
			assert_eq!(animation.step(MediaTime::from_seconds(1)), 1.0);
		}
		let mut instant = Animation::new(0.0, ease_out(0.0));
		instant.set_target(1.0);
		assert_eq!(instant.step(MediaTime::ZERO), 1.0);
	}
}
