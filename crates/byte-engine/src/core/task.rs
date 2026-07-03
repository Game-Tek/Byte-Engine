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
			Interval::Time(duration) => elapsed_time.as_secs_f64() % duration.as_secs_f64() < dt.as_secs_f64(),
			Interval::Frames(frames) => frame.is_multiple_of(*frames as u64),
		}
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
