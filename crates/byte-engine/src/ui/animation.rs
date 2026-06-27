use std::time::{Duration, Instant};

use crate::ui::layout::{context::Context, engine::EvaluationContext};

const MAX_STEP: f32 = 1.0 / 30.0;
const SETTLE_EPSILON: f32 = 0.001;

pub enum Curves {
	Linear,
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

pub async fn animate<C: 'static, F>(target: &mut EvaluationContext<C>, mut spring: Spring, mut apply: F)
where
	F: FnMut(&mut EvaluationContext<C>, f32),
{
	apply(target, spring.value());

	let mut last_frame = Instant::now();
	while !spring.is_settled() {
		target.render().await;
		let now = Instant::now();
		spring.step(now.duration_since(last_frame));
		last_frame = now;
		apply(target, spring.value());
	}

	apply(target, spring.finish());
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
}
