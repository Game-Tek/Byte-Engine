use core::task;
use std::{ops::Deref, sync::Arc};

use downcast_rs::Downcast;
use utils::{
	hash::{HashMap, HashMapExt as _},
	sync::Mutex,
};

use super::{Entity, EntityHandle};
use crate::time::MediaTime;

pub enum Interval {
	Time(MediaTime),
	Frames(u32),
}

impl Interval {
	pub fn is_now(&self, elapsed_time: MediaTime, dt: MediaTime, frame: u64) -> bool {
		match self {
			Interval::Time(duration) => {
				let period = duration.as_ticks();
				period == 0 || elapsed_time.as_ticks().rem_euclid(period) < dt.as_ticks()
			}
			Interval::Frames(frames) => *frames == 0 || frame.is_multiple_of(*frames as u64),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Interval, Task};
	use crate::time::MediaTime;

	#[test]
	fn frame_intervals_fire_only_on_multiples_including_wrap_origin() {
		let interval = Interval::Frames(3);
		let fired: Vec<_> = (0..10)
			.filter(|frame| interval.is_now(MediaTime::ZERO, MediaTime::ZERO, *frame))
			.collect();

		assert_eq!(fired, [0, 3, 6, 9]);
	}

	#[test]
	fn time_intervals_fire_during_the_frame_that_crosses_each_boundary() {
		let interval = Interval::Time(MediaTime::from_millis(100));
		assert!(interval.is_now(MediaTime::from_millis(200), MediaTime::from_millis(16), 0));
		assert!(interval.is_now(MediaTime::from_millis(305), MediaTime::from_millis(16), 0));
		assert!(!interval.is_now(MediaTime::from_millis(350), MediaTime::from_millis(16), 0));
	}

	#[test]
	fn zero_intervals_are_well_defined_as_every_tick() {
		assert!(Interval::Frames(0).is_now(MediaTime::ZERO, MediaTime::ZERO, 17));
		assert!(Interval::Time(MediaTime::ZERO).is_now(MediaTime::from_seconds(1), MediaTime::ZERO, 17));
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

		let delayed = Task::r#in(MediaTime::from_millis(10), || {});
		assert!(matches!(delayed.delay, Some(Interval::Time(duration)) if duration == MediaTime::from_millis(10)));
	}
}

impl From<MediaTime> for Interval {
	fn from(val: MediaTime) -> Self {
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
