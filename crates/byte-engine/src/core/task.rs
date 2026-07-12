use core::task;
use std::{ops::Deref, sync::Arc};

use downcast_rs::Downcast;
use utils::{
	hash::{HashMap, HashMapExt as _},
	sync::Mutex,
};

use super::{Entity, EntityHandle};

pub enum Interval {
	Time(std::time::Duration),
	Frames(u32),
}

impl Interval {
	pub fn is_now(&self, elapsed_time: std::time::Duration, dt: std::time::Duration, frame: u64) -> bool {
		match self {
			Interval::Time(duration) => {
				let period = duration.as_secs_f64();
				period == 0.0 || elapsed_time.as_secs_f64() % period < dt.as_secs_f64()
			}
			Interval::Frames(frames) => *frames == 0 || frame.is_multiple_of(*frames as u64),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::time::Duration;

	use super::{Interval, Task};

	#[test]
	fn frame_intervals_fire_only_on_multiples_including_wrap_origin() {
		let interval = Interval::Frames(3);
		let fired: Vec<_> = (0..10)
			.filter(|frame| interval.is_now(Duration::ZERO, Duration::ZERO, *frame))
			.collect();

		assert_eq!(fired, [0, 3, 6, 9]);
	}

	#[test]
	fn time_intervals_fire_during_the_frame_that_crosses_each_boundary() {
		let interval = Interval::Time(Duration::from_millis(100));
		assert!(interval.is_now(Duration::from_millis(200), Duration::from_millis(16), 0));
		assert!(interval.is_now(Duration::from_millis(305), Duration::from_millis(16), 0));
		assert!(!interval.is_now(Duration::from_millis(350), Duration::from_millis(16), 0));
	}

	#[test]
	fn zero_intervals_are_well_defined_as_every_tick() {
		assert!(Interval::Frames(0).is_now(Duration::ZERO, Duration::ZERO, 17));
		assert!(Interval::Time(Duration::ZERO).is_now(Duration::from_secs(1), Duration::ZERO, 17));
	}

	#[test]
	fn task_constructors_encode_distinct_scheduling_contracts() {
		let tick = Task::tick(|| {});
		assert!(tick.every.is_none() && tick.lifetime.is_none() && tick.delay.is_none());

		let every = Task::every(2u32, || {});
		assert!(matches!(every.every, Some(Interval::Frames(2))));
		assert!(every.lifetime.is_none() && every.delay.is_none());

		let once = Task::once(|| {});
		assert!(matches!(once.lifetime, Some(Interval::Frames(1))));

		let delayed = Task::r#in(Duration::from_millis(10), || {});
		assert!(matches!(delayed.delay, Some(Interval::Time(duration)) if duration == Duration::from_millis(10)));
	}
}

impl From<std::time::Duration> for Interval {
	fn from(val: std::time::Duration) -> Self {
		Interval::Time(val)
	}
}

impl From<u32> for Interval {
	fn from(val: u32) -> Self {
		Interval::Frames(val)
	}
}

/// A task is a unit of work that can be executed by the engine.
pub struct Task {
	f: Box<dyn FnMut()>,
	every: Option<Interval>,
	lifetime: Option<Interval>,
	delay: Option<Interval>,
}

impl Task {
	/// Creates a new task.
	pub fn new(f: impl Fn() + 'static) -> Self {
		Task {
			f: Box::new(f),
			every: None,
			lifetime: None,
			delay: None,
		}
	}

	pub fn tick(f: impl Fn() + 'static) -> Self {
		Self::new(f)
	}

	pub fn every(interval: impl Into<Interval>, f: impl Fn() + 'static) -> Self {
		Self {
			f: Box::new(f),
			every: Some(interval.into()),
			lifetime: None,
			delay: None,
		}
	}

	pub fn once(f: impl Fn() + 'static) -> Self {
		Self {
			f: Box::new(f),
			every: None,
			lifetime: Some(Interval::Frames(1)),
			delay: None,
		}
	}

	pub fn r#in(interval: impl Into<Interval>, f: impl Fn() + 'static) -> Self {
		Self {
			f: Box::new(f),
			every: None,
			lifetime: None,
			delay: Some(interval.into()),
		}
	}
}
