use core::task;
use std::ops::Deref;

use super::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle};

pub enum Interval {
	Time(std::time::Duration),
	Frames(u32),
}

impl Into<Interval> for std::time::Duration {
	fn into(self) -> Interval {
		Interval::Time(self)
	}
}

impl Into<Interval> for u32 {
	fn into(self) -> Interval {
		Interval::Frames(self)
	}
}

/// A task is a unit of work that can be executed by the engine.
pub struct Task {
	f: Box<dyn Fn()>,
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

	pub fn tick(f: impl Fn() + 'static) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(f))
	}

	pub fn every(interval: impl Into<Interval>, f: impl Fn() + 'static) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self { f: Box::new(f), every: Some(interval.into()), lifetime: None, delay: None })
	}

	pub fn once(f: impl Fn() + 'static) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self { f: Box::new(f), every: None, lifetime: Some(Interval::Frames(1)), delay: None })
	}

	pub fn r#in(interval: impl Into<Interval>, f: impl Fn() + 'static) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self { f: Box::new(f), every: None, lifetime: None, delay: Some(interval.into()) })
	}
}

impl Entity for Task {}

pub struct TaskExecutor {
	tasks: Vec<EntityHandle<Task>>,
}

impl TaskExecutor {
	fn new() -> Self {
		TaskExecutor {
			tasks: Vec::new(),
		}
	}

	pub fn create() -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new()).listen_to::<CreateEvent<Task>>()
	}

	fn add_task(&mut self, task: EntityHandle<Task>) {
		self.tasks.push(task);
	}

	pub fn execute(&mut self, elapsed_time: std::time::Duration, dt: std::time::Duration, frame: u64) {
		for task in &self.tasks {
			let task = task.read();
			
			let interval = if let Some(interval) = &task.every {
				match interval {
					Interval::Time(duration) => {
						elapsed_time.as_secs_f64() % duration.as_secs_f64() < dt.as_secs_f64()
					}
					Interval::Frames(frames) => {
						frame % *frames as u64 == 0
					}
				}
			} else {
				true
			};

			let delay = if let Some(delay) = &task.delay {
				match delay {
					Interval::Time(duration) => {
						duration.as_secs_f64() <= dt.as_secs_f64()
					}
					Interval::Frames(frames) => {
						*frames as u64 == 0
					}
				}
			} else {
				true
			};

			let run = interval && delay;

			if run {
				(task.f)();
			}
		}

		for task in &mut self.tasks {
			let mut task = task.write();

			if let Some(lifetime) = &mut task.lifetime {
				match lifetime {
					Interval::Time(duration) => {
						*duration -= dt;
					}
					Interval::Frames(frames) => {
						*frames -= 1;
					}
				}
			}

			if let Some(delay) = &mut task.delay {
				match delay {
					Interval::Time(duration) => {
						if *duration >= dt {
							*duration -= dt;
						} else {
							*duration = std::time::Duration::ZERO;
						}
					}
					Interval::Frames(frames) => {
						*frames -= 1;
					}
				}
			}
		}

		self.tasks.retain(|task| {
			let task = task.read();

			let lifetime = if let Some(lifetime) = &task.lifetime {
				match lifetime {
					Interval::Time(duration) => {
						*duration > std::time::Duration::ZERO
					}
					Interval::Frames(frames) => {
						*frames > 0
					}
				}
			} else {
				true
			};

			let delay = if let Some(delay) = &task.delay {
				match delay {
					Interval::Time(duration) => {
						*duration > std::time::Duration::ZERO
					}
					Interval::Frames(frames) => {
						*frames > 0
					}
				}
			} else {
				true
			};

			lifetime && delay
		});
	}
}

impl Entity for TaskExecutor {}

impl Listener<CreateEvent<Task>> for TaskExecutor {
	fn handle(&mut self, event: &CreateEvent<Task>) {
		let handle = event.handle();
		self.add_task(handle.clone());
	}
}