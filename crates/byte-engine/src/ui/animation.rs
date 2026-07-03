use std::time::{Duration, Instant};

use crate::ui::layout::{context::Context, engine::EvaluationContext};

const MAX_STEP: f32 = 1.0 / 30.0;
const SETTLE_EPSILON: f32 = 0.001;

pub enum Curves {
	Linear,
}

type EaseFunction = fn(f32) -> f32;

fn capped_frame_duration(dt: Duration) -> Duration {
	Duration::from_secs_f32(dt.as_secs_f32().min(MAX_STEP))
}

fn ease_in_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	t * t
}

fn ease_out_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	1.0 - (1.0 - t) * (1.0 - t)
}

fn ease_out_cubic_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	1.0 - (1.0 - t).powi(3)
}

fn ease_out_quart_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	1.0 - (1.0 - t).powi(4)
}

fn emphasized_out_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	1.0 - (1.0 - t).powi(5)
}

fn ease_in_out_curve(t: f32) -> f32 {
	let t = t.clamp(0.0, 1.0);
	if t < 0.5 {
		2.0 * t * t
	} else {
		1.0 - (-2.0 * t + 2.0).powi(2) * 0.5
	}
}

pub trait AnimationDriver {
	fn value(&self) -> f32;
	fn advance(&mut self, dt: Duration) -> f32;
	fn is_complete(&self) -> bool;
	fn finish(&mut self) -> f32;
}

#[derive(Debug, Clone, Copy)]
pub struct Easing {
	elapsed: f32,
	duration: f32,
	curve: EaseFunction,
}

#[derive(Debug, Clone, Copy)]
pub struct BackOut {
	elapsed: f32,
	duration: f32,
	overshoot: f32,
}

pub fn ease_in(duration: f32) -> Easing {
	Easing::new(duration, ease_in_curve)
}

pub fn ease_out(duration: f32) -> Easing {
	Easing::new(duration, ease_out_curve)
}

pub fn ease_out_cubic(duration: f32) -> Easing {
	Easing::new(duration, ease_out_cubic_curve)
}

pub fn ease_out_quart(duration: f32) -> Easing {
	Easing::new(duration, ease_out_quart_curve)
}

pub fn emphasized_out(duration: f32) -> Easing {
	Easing::new(duration, emphasized_out_curve)
}

pub fn ease_in_out(duration: f32) -> Easing {
	Easing::new(duration, ease_in_out_curve)
}

pub fn back_out(duration: f32, overshoot: f32) -> BackOut {
	BackOut::new(duration, overshoot)
}

impl Easing {
	fn new(duration: f32, curve: EaseFunction) -> Self {
		Self {
			elapsed: 0.0,
			duration: duration.max(0.0),
			curve,
		}
	}

	fn progress(&self) -> f32 {
		if self.duration == 0.0 {
			1.0
		} else {
			(self.elapsed / self.duration).clamp(0.0, 1.0)
		}
	}
}

impl BackOut {
	fn new(duration: f32, overshoot: f32) -> Self {
		Self {
			elapsed: 0.0,
			duration: duration.max(0.0),
			overshoot: overshoot.max(0.0),
		}
	}

	fn progress(&self) -> f32 {
		if self.duration == 0.0 {
			1.0
		} else {
			(self.elapsed / self.duration).clamp(0.0, 1.0)
		}
	}
}

impl AnimationDriver for Easing {
	fn value(&self) -> f32 {
		(self.curve)(self.progress())
	}

	fn advance(&mut self, dt: Duration) -> f32 {
		self.elapsed = (self.elapsed + dt.as_secs_f32()).min(self.duration);
		self.value()
	}

	fn is_complete(&self) -> bool {
		self.elapsed >= self.duration
	}

	fn finish(&mut self) -> f32 {
		self.elapsed = self.duration;
		self.value()
	}
}

impl AnimationDriver for BackOut {
	fn value(&self) -> f32 {
		let t = self.progress() - 1.0;
		1.0 + t * t * ((self.overshoot + 1.0) * t + self.overshoot)
	}

	fn advance(&mut self, dt: Duration) -> f32 {
		self.elapsed = (self.elapsed + dt.as_secs_f32()).min(self.duration);
		self.value()
	}

	fn is_complete(&self) -> bool {
		self.elapsed >= self.duration
	}

	fn finish(&mut self) -> f32 {
		self.elapsed = self.duration;
		self.value()
	}
}

#[derive(Debug, Clone, Copy)]
pub struct Spring {
	value: f32,
	target: f32,
	velocity: f32,
	mass: f32,
	stiffness: f32,
	damping: f32,
}

pub fn spring(from: f32, to: f32) -> Spring {
	Spring::new(from, to)
}

impl Spring {
	pub fn new(from: f32, to: f32) -> Self {
		Self {
			value: from,
			target: to,
			velocity: 0.0,
			mass: 1.0,
			stiffness: 380.0,
			damping: 16.0,
		}
	}

	pub fn value(&self) -> f32 {
		self.value
	}

	pub fn target(&self) -> f32 {
		self.target
	}

	pub fn velocity(&self) -> f32 {
		self.velocity
	}

	pub fn step(&mut self, dt: Duration) -> f32 {
		let dt = dt.as_secs_f32().min(MAX_STEP);
		if dt <= 0.0 {
			return self.value;
		}

		let displacement = self.value - self.target;
		let spring_force = -self.stiffness * displacement;
		let damping_force = -self.damping * self.velocity;
		let acceleration = (spring_force + damping_force) / self.mass;

		self.velocity += acceleration * dt;
		self.value += self.velocity * dt;
		self.value
	}

	pub fn is_settled(&self) -> bool {
		(self.value - self.target).abs() <= SETTLE_EPSILON && self.velocity.abs() <= SETTLE_EPSILON
	}

	pub fn finish(&mut self) -> f32 {
		self.value = self.target;
		self.velocity = 0.0;
		self.value
	}
}

impl AnimationDriver for Spring {
	fn value(&self) -> f32 {
		Spring::value(self)
	}

	fn advance(&mut self, dt: Duration) -> f32 {
		Spring::step(self, dt)
	}

	fn is_complete(&self) -> bool {
		Spring::is_settled(self)
	}

	fn finish(&mut self) -> f32 {
		Spring::finish(self)
	}
}

pub async fn animate<C: 'static, A, F>(target: &mut EvaluationContext<C>, mut animation: A, mut apply: F)
where
	A: AnimationDriver,
	F: FnMut(&mut EvaluationContext<C>, f32),
{
	apply(target, animation.value());

	let mut last_frame = Instant::now();
	while !animation.is_complete() {
		target.render().await;
		let now = Instant::now();
		animation.advance(capped_frame_duration(now.duration_since(last_frame)));
		last_frame = now;
		apply(target, animation.value());
	}

	apply(target, animation.finish());
}

pub struct Animation<V: Interpolate> {
	keyframes: Vec<(f32, V)>,
}

impl<V: Interpolate> Default for Animation<V> {
	fn default() -> Self {
		Self::new()
	}
}

impl<V: Interpolate> Animation<V> {
	pub fn new() -> Self {
		Self { keyframes: Vec::new() }
	}

	pub fn add_keyframe(&mut self, time: f32, value: V) {
		self.keyframes.push((time, value));
	}
}

pub struct Track<V: Interpolate> {
	animation: Animation<V>,
	duration: f32,
	current_time: f32,
}

impl<V: Interpolate> Track<V> {
	pub fn new(animation: Animation<V>, duration: f32) -> Self {
		Self {
			animation,
			duration,
			current_time: 0.0,
		}
	}

	pub fn update(&mut self, dt: f32) -> V {
		self.current_time += dt;
		if self.current_time > self.duration {
			self.current_time = 0.0;
		}

		let mut keyframes = self.animation.keyframes.iter();
		let mut prev = keyframes.next().unwrap();
		for curr in keyframes {
			if self.current_time < curr.0 {
				return prev.1.interpolate(&curr.1, (self.current_time - prev.0) / (curr.0 - prev.0));
			}
			prev = curr;
		}
		prev.1.interpolate(&prev.1, 0.0)
	}
}

pub trait Interpolate {
	fn interpolate(&self, other: &Self, t: f32) -> Self;
}

impl Interpolate for f32 {
	fn interpolate(&self, other: &Self, t: f32) -> Self {
		self * (1.0 - t) + other * t
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spring_moves_toward_target() {
		let mut spring = spring(0.0, 1.0);

		spring.step(Duration::from_millis(16));

		assert!(spring.value() > 0.0);
		assert!(spring.value() < 1.0);
	}

	#[test]
	fn spring_overshoots_with_default_config() {
		let mut spring = spring(0.0, 1.0);
		let mut peak = 0.0f32;

		for _ in 0..60 {
			spring.step(Duration::from_millis(16));
			peak = peak.max(spring.value());
		}

		assert!(peak > 1.08);
	}

	#[test]
	fn spring_settles_to_exact_target() {
		let mut spring = spring(0.0, 1.0);

		for _ in 0..240 {
			spring.step(Duration::from_millis(16));
			if spring.is_settled() {
				break;
			}
		}

		assert!(spring.is_settled());
		assert_eq!(spring.finish(), 1.0);
		assert_eq!(spring.velocity(), 0.0);
	}

	#[test]
	fn spring_clamps_large_steps() {
		let mut large_step = spring(0.0, 1.0);
		let mut capped_step = spring(0.0, 1.0);

		large_step.step(Duration::from_secs(1));
		capped_step.step(Duration::from_secs_f32(MAX_STEP));

		assert_eq!(large_step.value(), capped_step.value());
	}

	#[test]
	fn animation_frame_duration_is_capped_for_all_drivers() {
		assert!((capped_frame_duration(Duration::from_secs(1)).as_secs_f32() - MAX_STEP).abs() < f32::EPSILON);
		assert!((capped_frame_duration(Duration::from_millis(16)).as_secs_f32() - 0.016).abs() < f32::EPSILON);
	}

	#[test]
	fn easing_drivers_preserve_endpoints_and_handle_zero_duration() {
		let mut ease_in_driver = ease_in(1.0);
		let mut ease_out_driver = ease_out(1.0);
		let mut ease_in_out_driver = ease_in_out(1.0);
		let mut emphasized_out_driver = emphasized_out(1.0);
		let mut back_out_driver = back_out(1.0, 1.70158);

		assert_eq!(ease_in_driver.value(), 0.0);
		assert_eq!(ease_out_driver.value(), 0.0);
		assert_eq!(ease_in_out_driver.value(), 0.0);
		assert_eq!(emphasized_out_driver.value(), 0.0);
		assert_eq!(back_out_driver.value(), 0.0);

		assert_eq!(ease_in_driver.finish(), 1.0);
		assert_eq!(ease_out_driver.finish(), 1.0);
		assert_eq!(ease_in_out_driver.finish(), 1.0);
		assert_eq!(emphasized_out_driver.finish(), 1.0);
		assert_eq!(back_out_driver.finish(), 1.0);

		assert_eq!(ease_in(-1.0).value(), 1.0);
	}

	#[test]
	fn easing_drivers_have_expected_midpoint_shape() {
		let mut ease_in_driver = ease_in(1.0);
		let mut ease_out_driver = ease_out(1.0);
		let mut ease_in_out_driver = ease_in_out(1.0);

		ease_in_driver.advance(Duration::from_millis(500));
		ease_out_driver.advance(Duration::from_millis(500));
		ease_in_out_driver.advance(Duration::from_millis(500));

		assert!(ease_in_driver.value() < 0.5);
		assert!(ease_out_driver.value() > 0.5);
		assert_eq!(ease_in_out_driver.value(), 0.5);

		let mut ease_in_out_first_half = ease_in_out(1.0);
		let mut ease_in_out_second_half = ease_in_out(1.0);
		ease_in_out_first_half.advance(Duration::from_millis(250));
		ease_in_out_second_half.advance(Duration::from_millis(750));

		assert!(ease_in_out_first_half.value() < 0.25);
		assert!(ease_in_out_second_half.value() > 0.75);
	}

	#[test]
	fn emphasized_easing_moves_more_decisively_than_quadratic_ease_out() {
		let mut quadratic = ease_out(1.0);
		let mut cubic = ease_out_cubic(1.0);
		let mut quart = ease_out_quart(1.0);
		let mut emphasized = emphasized_out(1.0);

		quadratic.advance(Duration::from_millis(250));
		cubic.advance(Duration::from_millis(250));
		quart.advance(Duration::from_millis(250));
		emphasized.advance(Duration::from_millis(250));

		assert!(cubic.value() > quadratic.value());
		assert!(quart.value() > cubic.value());
		assert!(emphasized.value() > quart.value());
		assert!(emphasized.value() < 1.0);
	}

	#[test]
	fn back_out_overshoots_before_settling() {
		let mut driver = back_out(1.0, 1.70158);

		driver.advance(Duration::from_millis(600));
		assert!(driver.value() > 1.0);

		assert_eq!(driver.finish(), 1.0);
	}
}
